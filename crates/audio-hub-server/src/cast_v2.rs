//! Minimal Cast V2 client for Default Media Receiver playback.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use audio_bridge_types::{BridgeStatus, PlaybackEndReason};
use crossbeam_channel::{Receiver, Sender, TryRecvError};
use prost::Message;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, SignatureScheme, StreamOwned};
use serde_json::{json, Value};

use crate::bridge::BridgeCommand;
use crate::events::EventBus;
use crate::metadata_db::MetadataDb;
use crate::playback_transport::ChannelTransport;
use crate::queue_service::QueueService;
use crate::state::QueueState;
use crate::status_store::StatusStore;
use crate::stream_url::build_stream_url_for;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/cast_channel.rs"));
}

const NAMESPACE_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
const NAMESPACE_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
const NAMESPACE_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
const NAMESPACE_MEDIA: &str = "urn:x-cast:com.google.cast.media";
const DMR_APP_ID: &str = "CC1AD845";
const SENDER_ID: &str = "sender-0";
const RECEIVER_ID: &str = "receiver-0";

#[derive(Debug, Clone)]
pub struct CastDeviceDescriptor {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
}

struct CastSession {
    transport_id: String,
    session_id: String,
    media_session_id: Option<i64>,
}

struct CastConnection {
    stream: StreamOwned<ClientConnection, TcpStream>,
}

impl CastConnection {
    fn connect(addr: SocketAddr, server_name: ServerName<'static>) -> std::io::Result<Self> {
        let root_store = rustls::RootCertStore::empty();
        let mut config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        config
            .dangerous()
            .set_certificate_verifier(Arc::new(NoCertificateVerification));
        let conn = ClientConnection::new(Arc::new(config), server_name)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(Some(Duration::from_millis(200)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        Ok(Self {
            stream: StreamOwned::new(conn, stream),
        })
    }

    fn send_json(
        &mut self,
        destination_id: &str,
        namespace: &str,
        payload: &Value,
    ) -> std::io::Result<()> {
        let payload = serde_json::to_string(payload)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let msg = proto::CastMessage {
            protocol_version: proto::cast_message::ProtocolVersion::Castv210 as i32,
            source_id: SENDER_ID.to_string(),
            destination_id: destination_id.to_string(),
            namespace: namespace.to_string(),
            payload_type: proto::cast_message::PayloadType::String as i32,
            payload_utf8: Some(payload),
            payload_binary: None,
        };
        let mut buf = Vec::new();
        msg.encode(&mut buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let len = buf.len() as u32;
        self.stream.write_all(&len.to_be_bytes())?;
        self.stream.write_all(&buf)?;
        self.stream.flush()?;
        Ok(())
    }

    fn read_message(&mut self) -> std::io::Result<Option<proto::CastMessage>> {
        let mut len_buf = [0u8; 4];
        match self.stream.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(err) if is_timeout(&err) => return Ok(None),
            Err(err) => return Err(err),
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf)?;
        let msg = proto::CastMessage::decode(&buf[..])
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Some(msg))
    }
}

pub fn spawn_cast_worker(
    output_id: String,
    device: CastDeviceDescriptor,
    cmd_rx: Receiver<BridgeCommand>,
    cmd_tx: Sender<BridgeCommand>,
    status: StatusStore,
    queue: Arc<Mutex<QueueState>>,
    events: EventBus,
    public_base_url: String,
    metadata: Option<MetadataDb>,
    bridge_state: Arc<Mutex<crate::state::BridgeState>>,
    cast_workers: Arc<Mutex<std::collections::HashMap<String, Sender<BridgeCommand>>>>,
    cast_statuses: Arc<Mutex<std::collections::HashMap<String, BridgeStatus>>>,
    cast_status_updated_at: Arc<Mutex<std::collections::HashMap<String, Instant>>>,
) {
    std::thread::spawn(move || {
        let addr = resolve_device_addr(&device.host, device.port);
        let addr = match addr {
            Ok(addr) => addr,
            Err(err) => {
                tracing::warn!(error = %err, cast_id = %device.id, "cast: resolve failed");
                return;
            }
        };
        let server_name = server_name_for(&device.host);
        let mut conn = match CastConnection::connect(addr, server_name) {
            Ok(conn) => conn,
            Err(err) => {
                tracing::warn!(error = %err, cast_id = %device.id, "cast: connect failed");
                return;
            }
        };

        let queue_service = QueueService::new(queue, status.clone(), events);
        let mut session: Option<CastSession> = None;
        let mut pending_play: Option<(PathBuf, String, Option<u64>, bool)> = None;
        let mut current_path: Option<PathBuf> = None;
        let mut request_id: i64 = 1;
        let mut last_ping = Instant::now();
        let mut last_status_poll = Instant::now();
        let mut last_duration_ms: Option<u64> = None;
        let mut session_auto_advance_in_flight = false;

        let _ = conn.send_json(RECEIVER_ID, NAMESPACE_CONNECTION, &json!({ "type": "CONNECT" }));

        loop {
            match cmd_rx.try_recv() {
                Ok(cmd) => match cmd {
                    BridgeCommand::Quit => break,
                    BridgeCommand::PauseToggle => {
                        if let Some(session) = session.as_ref() {
                            let paused = status.inner().lock().ok().map(|s| s.paused).unwrap_or(false);
                            let cmd_type = if paused { "PLAY" } else { "PAUSE" };
                            if let Some(media_session_id) = session.media_session_id {
                                let _ = conn.send_json(
                                    &session.transport_id,
                                    NAMESPACE_MEDIA,
                                    &json!({
                                        "type": cmd_type,
                                        "requestId": next_request_id(&mut request_id),
                                        "mediaSessionId": media_session_id,
                                    }),
                                );
                            }
                            status.on_pause_toggle();
                        }
                    }
                    BridgeCommand::Stop => {
                        if let Some(session) = session.as_ref() {
                            if let Some(media_session_id) = session.media_session_id {
                                let _ = conn.send_json(
                                    &session.transport_id,
                                    NAMESPACE_MEDIA,
                                    &json!({
                                        "type": "STOP",
                                        "requestId": next_request_id(&mut request_id),
                                        "mediaSessionId": media_session_id,
                                    }),
                                );
                            }
                        }
                        current_path = None;
                    }
                    BridgeCommand::StopSilent => {
                        if let Some(session) = session.as_ref() {
                            if let Some(media_session_id) = session.media_session_id {
                                let _ = conn.send_json(
                                    &session.transport_id,
                                    NAMESPACE_MEDIA,
                                    &json!({
                                        "type": "STOP",
                                        "requestId": next_request_id(&mut request_id),
                                        "mediaSessionId": media_session_id,
                                    }),
                                );
                            }
                        }
                        current_path = None;
                    }
                    BridgeCommand::Seek { ms } => {
                        if let Some(session) = session.as_ref() {
                            if let Some(media_session_id) = session.media_session_id {
                                let _ = conn.send_json(
                                    &session.transport_id,
                                    NAMESPACE_MEDIA,
                                    &json!({
                                        "type": "SEEK",
                                        "requestId": next_request_id(&mut request_id),
                                        "mediaSessionId": media_session_id,
                                        "currentTime": (ms as f64) / 1000.0,
                                    }),
                                );
                                status.mark_seek_in_flight();
                            }
                        }
                    }
                    BridgeCommand::Play { path, ext_hint, seek_ms, start_paused } => {
                        current_path = Some(path.clone());
                        pending_play = Some((path, ext_hint, seek_ms, start_paused));
                        ensure_session(&mut conn, &mut session, &device, &mut request_id);
                    }
                },
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => break,
            }

            if let Some((path, ext_hint, seek_ms, start_paused)) = pending_play.take() {
                if let Some(session) = session.as_ref() {
                    let url = build_stream_url_for(&path, &public_base_url, metadata.as_ref());
                    let content_type = content_type_for_ext(&ext_hint);
                    let meta = track_metadata(&path, metadata.as_ref());
                    let payload = load_payload(
                        &url,
                        content_type,
                        meta,
                        seek_ms,
                        start_paused,
                        &session.session_id,
                        next_request_id(&mut request_id),
                    );
                    let _ = conn.send_json(&session.transport_id, NAMESPACE_MEDIA, &payload);
                    status.on_play(path, start_paused);
                } else {
                    pending_play = Some((path, ext_hint, seek_ms, start_paused));
                }
            }

            if last_ping.elapsed() > Duration::from_secs(5) {
                let _ = conn.send_json(RECEIVER_ID, NAMESPACE_HEARTBEAT, &json!({ "type": "PING" }));
                last_ping = Instant::now();
            }
            if last_status_poll.elapsed() > Duration::from_millis(1000) {
                if let Some(session) = session.as_ref() {
                    let _ = conn.send_json(
                        &session.transport_id,
                        NAMESPACE_MEDIA,
                        &json!({
                            "type": "GET_STATUS",
                            "requestId": next_request_id(&mut request_id),
                        }),
                    );
                }
                last_status_poll = Instant::now();
            }

            match conn.read_message() {
                Ok(Some(msg)) => {
                    handle_message(
                        &mut conn,
                        msg,
                        &device,
                        &mut session,
                        &mut last_duration_ms,
                        &status,
                        &queue_service,
                        &cmd_tx,
                        &output_id,
                        &mut current_path,
                        &cast_statuses,
                        &cast_status_updated_at,
                        &mut session_auto_advance_in_flight,
                        &mut request_id,
                        &bridge_state,
                    );
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(error = %err, cast_id = %device.id, "cast: connection lost");
                    break;
                }
            }
        }
        if let Ok(mut statuses) = cast_statuses.lock() {
            statuses.remove(&output_id);
        }
        if let Ok(mut updates) = cast_status_updated_at.lock() {
            updates.remove(&output_id);
        }
        if let Ok(mut workers) = cast_workers.lock() {
            workers.remove(&output_id);
        }
        tracing::info!(cast_id = %device.id, "cast worker stopped");
    });
}

fn ensure_session(
    conn: &mut CastConnection,
    session: &mut Option<CastSession>,
    device: &CastDeviceDescriptor,
    request_id: &mut i64,
) {
    if session.is_some() {
        return;
    }
    let _ = conn.send_json(
        RECEIVER_ID,
        NAMESPACE_RECEIVER,
        &json!({
            "type": "LAUNCH",
            "requestId": next_request_id(request_id),
            "appId": DMR_APP_ID,
        }),
    );
    let _ = conn.send_json(
        RECEIVER_ID,
        NAMESPACE_RECEIVER,
        &json!({
            "type": "GET_STATUS",
            "requestId": next_request_id(request_id),
        }),
    );
    tracing::info!(cast_id = %device.id, "cast: launching DMR");
}

fn handle_message(
    conn: &mut CastConnection,
    msg: proto::CastMessage,
    device: &CastDeviceDescriptor,
    session: &mut Option<CastSession>,
    last_duration_ms: &mut Option<u64>,
    status: &StatusStore,
    queue_service: &QueueService,
    cmd_tx: &Sender<BridgeCommand>,
    output_id: &str,
    current_path: &mut Option<PathBuf>,
    cast_statuses: &Arc<Mutex<std::collections::HashMap<String, BridgeStatus>>>,
    cast_status_updated_at: &Arc<Mutex<std::collections::HashMap<String, Instant>>>,
    session_auto_advance_in_flight: &mut bool,
    request_id: &mut i64,
    bridge_state: &Arc<Mutex<crate::state::BridgeState>>,
) {
    let is_active = is_active_cast_output(bridge_state, &device.id);
    if msg.payload_type != proto::cast_message::PayloadType::String as i32 {
        return;
    }
    let payload = msg.payload_utf8.unwrap_or_default();
    let Ok(value) = serde_json::from_str::<Value>(&payload) else { return };
    let msg_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match msg.namespace.as_str() {
        NAMESPACE_HEARTBEAT => {
            if msg_type == "PING" {
                let _ = conn.send_json(
                    msg.source_id.as_str(),
                    NAMESPACE_HEARTBEAT,
                    &json!({ "type": "PONG" }),
                );
            }
        }
        NAMESPACE_RECEIVER => {
            if msg_type == "RECEIVER_STATUS" {
                if let Some((transport_id, session_id)) = parse_receiver_status(&value) {
                    if session.as_ref().map(|s| s.transport_id.as_str()) != Some(&transport_id) {
                        *session = Some(CastSession {
                            transport_id: transport_id.clone(),
                            session_id,
                            media_session_id: None,
                        });
                        tracing::info!(cast_id = %device.id, "cast: DMR ready");
                        // Connect to app transport
                        let _ = conn.send_json(
                            &transport_id,
                            NAMESPACE_CONNECTION,
                            &json!({ "type": "CONNECT" }),
                        );
                        let _ = conn.send_json(
                            &transport_id,
                            NAMESPACE_MEDIA,
                            &json!({
                                "type": "GET_STATUS",
                                "requestId": next_request_id(request_id),
                            }),
                        );
                    }
                }
            }
        }
        NAMESPACE_MEDIA => {
            if msg_type == "MEDIA_STATUS" {
                if let Some(status_info) = parse_media_status(&value) {
                    if let Some(sess) = session.as_mut() {
                        sess.media_session_id = status_info.media_session_id.or(sess.media_session_id);
                    }
                    apply_media_status(
                        device,
                        status_info,
                        last_duration_ms,
                        status,
                        queue_service,
                        cmd_tx,
                        output_id,
                        is_active,
                        current_path,
                        cast_statuses,
                        cast_status_updated_at,
                        session_auto_advance_in_flight,
                    );
                }
            }
        }
        _ => {}
    }
}

fn is_active_cast_output(
    bridge_state: &Arc<Mutex<crate::state::BridgeState>>,
    device_id: &str,
) -> bool {
    let Ok(guard) = bridge_state.lock() else { return false };
    let expected = format!("cast:{device_id}");
    guard.active_output_id.as_deref() == Some(expected.as_str())
}

fn apply_media_status(
    device: &CastDeviceDescriptor,
    info: MediaStatus,
    last_duration_ms: &mut Option<u64>,
    status: &StatusStore,
    queue_service: &QueueService,
    cmd_tx: &Sender<BridgeCommand>,
    output_id: &str,
    is_active_output: bool,
    current_path: &mut Option<PathBuf>,
    cast_statuses: &Arc<Mutex<std::collections::HashMap<String, BridgeStatus>>>,
    cast_status_updated_at: &Arc<Mutex<std::collections::HashMap<String, Instant>>>,
    session_auto_advance_in_flight: &mut bool,
) {
    let had_current_path = current_path.is_some();
    let is_idle = matches!(info.player_state.as_deref(), Some("IDLE"));
    let should_clear = is_idle && info.idle_reason.is_some();
    let (elapsed_ms, duration_ms) = if should_clear {
        (None, None)
    } else {
        (
            info.current_time_s.map(|s| (s * 1000.0) as u64),
            info.duration_s.map(|s| (s * 1000.0) as u64),
        )
    };

    let end_reason = match info.idle_reason.as_deref() {
        Some("FINISHED") => Some(PlaybackEndReason::Eof),
        Some("CANCELLED") => Some(PlaybackEndReason::Stopped),
        Some("ERROR") => Some(PlaybackEndReason::Error),
        Some("STOPPED") => Some(PlaybackEndReason::Stopped),
        _ => None,
    };

    let mut remote = BridgeStatus::default();
    if should_clear {
        *current_path = None;
    }
    remote.now_playing = current_path.as_ref().map(|path| path.to_string_lossy().to_string());
    remote.paused = matches!(info.player_state.as_deref(), Some("PAUSED"));
    remote.elapsed_ms = elapsed_ms;
    remote.duration_ms = duration_ms;
    remote.device = Some(device.name.clone());
    remote.end_reason = end_reason;
    if let Ok(mut statuses) = cast_statuses.lock() {
        statuses.insert(output_id.to_string(), remote.clone());
    }
    if let Ok(mut updates) = cast_status_updated_at.lock() {
        updates.insert(output_id.to_string(), Instant::now());
    }

    let bound_session_id = crate::session_registry::output_lock_owner(output_id);
    if !is_idle {
        *session_auto_advance_in_flight = false;
    }
    let idle_without_reason = is_idle && end_reason.is_none();
    let should_session_advance = end_reason == Some(PlaybackEndReason::Eof)
        || (idle_without_reason && had_current_path);
    if should_session_advance && !*session_auto_advance_in_flight {
        if let Some(session_id) = bound_session_id.as_deref() {
            if let Ok(Some(next_path)) = crate::session_registry::queue_next_path(session_id) {
                let ext_hint = next_path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let _ = cmd_tx.send(BridgeCommand::Play {
                    path: next_path.clone(),
                    ext_hint,
                    seek_ms: None,
                    start_paused: false,
                });
                *current_path = Some(next_path);
                *session_auto_advance_in_flight = true;
                if let Ok(mut statuses) = cast_statuses.lock() {
                    let mut next_remote = remote.clone();
                    next_remote.now_playing =
                        current_path.as_ref().map(|path| path.to_string_lossy().to_string());
                    next_remote.paused = false;
                    next_remote.elapsed_ms = None;
                    next_remote.end_reason = None;
                    statuses.insert(output_id.to_string(), next_remote);
                }
                if let Ok(mut updates) = cast_status_updated_at.lock() {
                    updates.insert(output_id.to_string(), Instant::now());
                }
            }
        }
    }
    if !is_active_output {
        return;
    }

    let (inputs, changed) = status.reduce_remote_and_inputs(&remote, *last_duration_ms);
    status.emit_if_changed(changed);
    *last_duration_ms = remote.duration_ms;
    if bound_session_id.is_some() {
        return;
    }

    let transport = ChannelTransport::new(cmd_tx.clone());
    let _ = queue_service.maybe_auto_advance(&transport, inputs);
    let current = status
        .inner()
        .lock()
        .ok()
        .and_then(|s| s.now_playing.clone());
    let has_previous = queue_service.has_previous(current.as_deref());
    status.set_has_previous(has_previous);
}

fn parse_receiver_status(payload: &Value) -> Option<(String, String)> {
    let apps = payload.get("status")?.get("applications")?.as_array()?;
    for app in apps {
        let app_id = app.get("appId").and_then(|v| v.as_str()).unwrap_or("");
        if app_id != DMR_APP_ID {
            continue;
        }
        let transport_id = app.get("transportId")?.as_str()?.to_string();
        let session_id = app.get("sessionId")?.as_str()?.to_string();
        return Some((transport_id, session_id));
    }
    None
}

#[derive(Default)]
struct MediaStatus {
    media_session_id: Option<i64>,
    player_state: Option<String>,
    current_time_s: Option<f64>,
    duration_s: Option<f64>,
    idle_reason: Option<String>,
}

fn parse_media_status(payload: &Value) -> Option<MediaStatus> {
    let statuses = payload.get("status")?.as_array()?;
    let status = statuses.first()?;
    let media_session_id = status.get("mediaSessionId").and_then(|v| v.as_i64());
    let player_state = status.get("playerState").and_then(|v| v.as_str()).map(|s| s.to_string());
    let idle_reason = status.get("idleReason").and_then(|v| v.as_str()).map(|s| s.to_string());
    let current_time_s = status.get("currentTime").and_then(|v| v.as_f64());
    let duration_s = status
        .get("media")
        .and_then(|m| m.get("duration"))
        .and_then(|v| v.as_f64());
    Some(MediaStatus {
        media_session_id,
        player_state,
        current_time_s,
        duration_s,
        idle_reason,
    })
}

fn load_payload(
    url: &str,
    content_type: &str,
    meta: TrackMetadata,
    seek_ms: Option<u64>,
    start_paused: bool,
    session_id: &str,
    request_id: i64,
) -> Value {
    let mut media = json!({
        "contentId": url,
        "contentType": content_type,
        "streamType": "BUFFERED",
        "metadata": {
            "metadataType": 3,
        },
        "customData": {
            "path": meta.path,
        }
    });
    if let Some(title) = meta.title {
        media["metadata"]["title"] = title.into();
    }
    if let Some(artist) = meta.artist {
        media["metadata"]["artist"] = artist.into();
    }
    if let Some(album) = meta.album {
        media["metadata"]["albumName"] = album.into();
    }

    let mut payload = json!({
        "type": "LOAD",
        "requestId": request_id,
        "sessionId": session_id,
        "media": media,
        "autoplay": !start_paused,
    });
    if let Some(seek_ms) = seek_ms {
        payload["currentTime"] = ((seek_ms as f64) / 1000.0).into();
    }
    payload
}

struct TrackMetadata {
    path: String,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
}

fn track_metadata(path: &PathBuf, metadata: Option<&MetadataDb>) -> TrackMetadata {
    let path_str = path.to_string_lossy().to_string();
    let record = metadata
        .and_then(|db| db.track_record_by_path(&path_str).ok().flatten());
    TrackMetadata {
        path: path_str,
        title: record.as_ref().and_then(|r| r.title.clone()),
        artist: record.as_ref().and_then(|r| r.artist.clone()),
        album: record.as_ref().and_then(|r| r.album.clone()),
    }
}

fn content_type_for_ext(ext_hint: &str) -> &'static str {
    match ext_hint.to_ascii_lowercase().as_str() {
        "flac" => "audio/flac",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "aac" => "audio/mp4",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "opus" => "audio/ogg",
        _ => "audio/mpeg",
    }
}

fn next_request_id(value: &mut i64) -> i64 {
    let current = *value;
    *value = value.saturating_add(1);
    current
}

fn resolve_device_addr(host: &str, port: u16) -> std::io::Result<SocketAddr> {
    let mut addrs = format!("{host}:{port}").to_socket_addrs()?;
    addrs
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no address"))
}

fn server_name_for(host: &str) -> ServerName<'static> {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        ServerName::IpAddress(ip.into())
    } else {
        ServerName::try_from(host.to_string())
            .unwrap_or_else(|_| ServerName::try_from("localhost".to_string()).unwrap())
    }
}

fn is_timeout(err: &std::io::Error) -> bool {
    matches!(err.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
}

#[derive(Debug)]
struct NoCertificateVerification;

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}
