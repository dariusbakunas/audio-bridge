use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, TryRecvError};

use crate::state::PlayerStatus;

#[derive(Debug, Clone)]
pub enum BridgeCommand {
    Play { path: PathBuf, ext_hint: String },
    PauseToggle,
    Stop,
    Quit,
}

#[derive(Clone)]
pub struct BridgePlayer {
    pub(crate) cmd_tx: Sender<BridgeCommand>,
}

#[derive(Debug, serde::Deserialize)]
pub struct HttpDevicesResponse {
    pub devices: Vec<HttpDeviceInfo>,
}

#[derive(Debug, serde::Deserialize, Clone)]
pub struct HttpDeviceInfo {
    pub name: String,
    pub min_rate: u32,
    pub max_rate: u32,
}

#[derive(Debug, serde::Deserialize)]
pub struct HttpStatusResponse {
    pub paused: bool,
    pub elapsed_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub device: Option<String>,
    pub underrun_frames: Option<u64>,
    pub underrun_events: Option<u64>,
    pub buffer_size_frames: Option<u32>,
}

pub fn http_list_devices(addr: SocketAddr) -> Result<Vec<HttpDeviceInfo>> {
    let url = format!("http://{addr}/devices");
    let mut resp = ureq::get(&url)
        .config()
        .timeout_per_call(Some(Duration::from_secs(2)))
        .build()
        .call()
        .map_err(|e| anyhow::anyhow!("http devices request failed: {e}"))?;
    let resp: HttpDevicesResponse = resp
        .body_mut()
        .read_json()
        .map_err(|e| anyhow::anyhow!("http devices decode failed: {e}"))?;
    Ok(resp.devices)
}

pub fn http_set_device(addr: SocketAddr, name: &str) -> Result<()> {
    let url = format!("http://{addr}/devices/select");
    let payload = serde_json::json!({ "name": name });
    ureq::post(&url)
        .config()
        .timeout_per_call(Some(Duration::from_secs(2)))
        .build()
        .send_json(payload)
        .map_err(|e| anyhow::anyhow!("http set device failed: {e}"))?;
    Ok(())
}

pub fn http_status(addr: SocketAddr) -> Result<HttpStatusResponse> {
    let url = format!("http://{addr}/status");
    let mut resp = ureq::get(&url)
        .config()
        .timeout_per_call(Some(Duration::from_secs(2)))
        .build()
        .call()
        .map_err(|e| anyhow::anyhow!("http status request failed: {e}"))?;
    let resp: HttpStatusResponse = resp
        .body_mut()
        .read_json()
        .map_err(|e| anyhow::anyhow!("http status decode failed: {e}"))?;
    Ok(resp)
}

pub fn spawn_bridge_worker(
    bridge_id: String,
    addr: SocketAddr,
    cmd_rx: Receiver<BridgeCommand>,
    cmd_tx: Sender<BridgeCommand>,
    status: Arc<Mutex<PlayerStatus>>,
    queue: Arc<Mutex<crate::state::QueueState>>,
    bridge_online: Arc<AtomicBool>,
    bridges_state: Arc<Mutex<crate::state::BridgeState>>,
) {
    std::thread::spawn(move || {
        tracing::info!(bridge_id = %bridge_id, addr = %addr, "bridge worker start");
        let mut stream = connect_loop(
            bridge_id.clone(),
            addr,
            status.clone(),
            queue.clone(),
            cmd_tx.clone(),
            bridge_online.clone(),
            bridges_state.clone(),
        );
        let mut paused = false;

        loop {
            let cmd = match cmd_rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            };

            match cmd {
                BridgeCommand::Quit => {
                    tracing::info!("bridge cmd: quit");
                    let _ = audio_bridge_proto::write_frame(
                        &mut stream,
                        audio_bridge_proto::FrameKind::Next,
                        &[],
                    );
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    tracing::info!("bridge cmd: quit sent (shutdown)");
                    break;
                }
                BridgeCommand::PauseToggle => {
                    tracing::info!("bridge cmd: pause toggle");
                    paused = !paused;
                    let kind = if paused {
                        audio_bridge_proto::FrameKind::Pause
                    } else {
                        audio_bridge_proto::FrameKind::Resume
                    };
                    if audio_bridge_proto::write_frame(&mut stream, kind, &[]).is_ok() {
                        if let Ok(mut s) = status.lock() {
                            s.paused = paused;
                        }
                    }
                }
                BridgeCommand::Stop => {
                    tracing::info!("bridge cmd: stop");
                    let _ = audio_bridge_proto::write_frame(
                        &mut stream,
                        audio_bridge_proto::FrameKind::Next,
                        &[],
                    );
                    if let Ok(mut s) = status.lock() {
                        s.now_playing = None;
                        s.paused = false;
                        s.user_paused = false;
                        s.elapsed_ms = None;
                        s.duration_ms = None;
                        s.sample_rate = None;
                        s.channels = None;
                        s.auto_advance_in_flight = false;
                    }
                }
                BridgeCommand::Play { path, ext_hint } => {
                    tracing::info!(path = %path.display(), ext_hint = %ext_hint, "bridge cmd: play");
                    let mut next = Some((path, ext_hint));
                    while let Some((path, ext_hint)) = next.take() {
                        tracing::info!(path = %path.display(), "bridge play");
                        if let Ok(mut s) = status.lock() {
                            s.now_playing = Some(path.clone());
                            s.paused = false;
                            s.elapsed_ms = Some(0);
                            s.sample_rate = None;
                            s.channels = None;
                            s.auto_advance_in_flight = false;
                        }
                        match send_one_track_over_existing_connection(
                            &mut stream,
                            &cmd_rx,
                            &mut paused,
                            path,
                            ext_hint,
                        ) {
                            Ok(Flow::Continue) => break,
                            Ok(Flow::SwitchTo { path, ext_hint }) => {
                                next = Some((path, ext_hint));
                            }
                            Ok(Flow::Quit) => return,
                            Err(e) => {
                                if is_network_error(&e) {
                                    tracing::warn!("bridge connection lost; reconnecting");
                                    stream = connect_loop(
                                        bridge_id.clone(),
                                        addr,
                                        status.clone(),
                                        queue.clone(),
                                        cmd_tx.clone(),
                                        bridge_online.clone(),
                                        bridges_state.clone(),
                                    );
                                    continue;
                                }
                                tracing::error!("playback failed: {e:#}");
                                break;
                            }
                        }
                    }
                }
            }
        }
    });
}

#[derive(Debug)]
enum Flow {
    Continue,
    SwitchTo { path: PathBuf, ext_hint: String },
    Quit,
}

fn connect_loop(
    bridge_id: String,
    addr: SocketAddr,
    status: Arc<Mutex<PlayerStatus>>,
    queue: Arc<Mutex<crate::state::QueueState>>,
    cmd_tx: Sender<BridgeCommand>,
    bridge_online: Arc<AtomicBool>,
    bridges_state: Arc<Mutex<crate::state::BridgeState>>,
) -> TcpStream {
    let mut delay = Duration::from_millis(250);
    loop {
        match connect_and_handshake(
            bridge_id.clone(),
            addr,
            status.clone(),
            queue.clone(),
            cmd_tx.clone(),
            bridge_online.clone(),
            bridges_state.clone(),
        ) {
            Ok(stream) => {
                if bridges_state
                    .lock()
                    .map(|s| s.active_bridge_id.as_deref() == Some(bridge_id.as_str()))
                    .unwrap_or(false)
                {
                    bridge_online.store(true, Ordering::Relaxed);
                }
                tracing::info!(bridge_id = %bridge_id, addr = %addr, "bridge connected");
                return stream;
            }
            Err(e) => {
                if bridges_state
                    .lock()
                    .map(|s| s.active_bridge_id.as_deref() == Some(bridge_id.as_str()))
                    .unwrap_or(false)
                {
                    bridge_online.store(false, Ordering::Relaxed);
                }
                tracing::warn!(
                    bridge_id = %bridge_id,
                    addr = %addr,
                    delay_ms = delay.as_millis(),
                    error = %e,
                    "bridge connect failed"
                );
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_secs(5));
            }
        }
    }
}

fn connect_and_handshake(
    bridge_id: String,
    addr: SocketAddr,
    status: Arc<Mutex<PlayerStatus>>,
    queue: Arc<Mutex<crate::state::QueueState>>,
    cmd_tx: Sender<BridgeCommand>,
    bridge_online: Arc<AtomicBool>,
    bridges_state: Arc<Mutex<crate::state::BridgeState>>,
) -> Result<TcpStream> {
    tracing::info!(bridge_id = %bridge_id, addr = %addr, "bridge connect attempt");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .with_context(|| format!("connect {addr}"))?;
    stream.set_nodelay(true).ok();
    stream.set_write_timeout(Some(Duration::from_millis(200))).ok();

    audio_bridge_proto::write_prelude(&mut stream).context("write prelude")?;
    let mut stream_rx = stream.try_clone().context("try_clone stream for rx")?;
    audio_bridge_proto::read_prelude(&mut stream_rx).context("read prelude")?;
    spawn_bridge_reader(
        stream_rx,
        status,
        queue,
        cmd_tx,
        bridge_online,
        bridges_state,
        bridge_id,
    );
    Ok(stream)
}

fn spawn_bridge_reader(
    mut stream_rx: TcpStream,
    status: Arc<Mutex<PlayerStatus>>,
    queue: Arc<Mutex<crate::state::QueueState>>,
    cmd_tx: Sender<BridgeCommand>,
    bridge_online: Arc<AtomicBool>,
    bridges_state: Arc<Mutex<crate::state::BridgeState>>,
    bridge_id: String,
) {
    std::thread::spawn(move || loop {
        let (kind, len) = match audio_bridge_proto::read_frame_header(&mut stream_rx) {
            Ok(x) => x,
            Err(_) => {
                if bridges_state
                    .lock()
                    .map(|s| s.active_bridge_id.as_deref() == Some(bridge_id.as_str()))
                    .unwrap_or(false)
                {
                    bridge_online.store(false, Ordering::Relaxed);
                }
                if let Ok(mut s) = status.lock() {
                    s.now_playing = None;
                    s.paused = false;
                    s.user_paused = false;
                    s.elapsed_ms = None;
                    s.duration_ms = None;
                    s.sample_rate = None;
                    s.channels = None;
                    s.auto_advance_in_flight = false;
                }
                tracing::warn!("bridge reader disconnected");
                break;
            }
        };
        let mut payload = vec![0u8; len as usize];
        if stream_rx.read_exact(&mut payload).is_err() {
            break;
        }
        match kind {
            audio_bridge_proto::FrameKind::TrackInfo => {
                if let Ok((sr, _ch, dur)) = audio_bridge_proto::decode_track_info(&payload) {
                    if let Ok(mut s) = status.lock() {
                        s.sample_rate = Some(sr);
                        s.channels = Some(_ch);
                        if dur.is_some() {
                            s.duration_ms = dur;
                        }
                    }
                }
            }
            audio_bridge_proto::FrameKind::PlaybackPos => {
                if let Ok((frames, paused)) = audio_bridge_proto::decode_playback_pos(&payload) {
                    if let Ok(mut s) = status.lock() {
                        s.paused = paused;
                        if let Some(sr) = s.sample_rate {
                            if sr > 0 {
                                let elapsed = frames.saturating_mul(1000) / sr as u64;
                                s.elapsed_ms = Some(elapsed);
                            }
                        }
                        if let (Some(elapsed), Some(duration)) = (s.elapsed_ms, s.duration_ms) {
                            if elapsed > duration {
                                s.elapsed_ms = Some(duration);
                            }
                        }
                        if !s.auto_advance_in_flight {
                            if let (Some(elapsed), Some(duration)) = (s.elapsed_ms, s.duration_ms) {
                                if elapsed + 50 >= duration && !s.user_paused {
                                    drop(s);
                                    if let Some(path) = pop_next_from_queue(&queue) {
                                        let ext_hint = path
                                            .extension()
                                            .and_then(|ext| ext.to_str())
                                            .unwrap_or("")
                                            .to_ascii_lowercase();
                                        let _ = cmd_tx.send(BridgeCommand::Play { path, ext_hint });
                                        if let Ok(mut s2) = status.lock() {
                                            s2.auto_advance_in_flight = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            audio_bridge_proto::FrameKind::Error => {
                let msg = String::from_utf8_lossy(&payload).to_string();
                tracing::warn!("bridge error: {msg}");
            }
            _ => {}
        }
    });
}

fn pop_next_from_queue(queue: &Arc<Mutex<crate::state::QueueState>>) -> Option<PathBuf> {
    let mut q = queue.lock().ok()?;
    if q.items.is_empty() {
        None
    } else {
        Some(q.items.remove(0))
    }
}

fn write_all_interruptible(
    stream: &mut TcpStream,
    buf: &[u8],
    cmd_rx: &Receiver<BridgeCommand>,
    paused: &mut bool,
) -> Result<Flow> {
    let mut off = 0usize;

    while off < buf.len() {
        match cmd_rx.try_recv() {
            Ok(BridgeCommand::Quit) => return Ok(Flow::Quit),
            Ok(BridgeCommand::Play { path, ext_hint }) => return Ok(Flow::SwitchTo { path, ext_hint }),
            Ok(BridgeCommand::PauseToggle) => {
                *paused = !*paused;
                let kind = if *paused {
                    audio_bridge_proto::FrameKind::Pause
                } else {
                    audio_bridge_proto::FrameKind::Resume
                };
                audio_bridge_proto::write_frame(&mut *stream, kind, &[])
                    .with_context(|| format!("write {kind:?}"))?;
            }
            Ok(BridgeCommand::Stop) => {
                let _ = audio_bridge_proto::write_frame(
                    &mut *stream,
                    audio_bridge_proto::FrameKind::Next,
                    &[],
                );
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return Ok(Flow::Quit),
        }

        match stream.write(&buf[off..]) {
            Ok(0) => return Err(anyhow::anyhow!("socket closed while writing")).map_err(Into::into),
            Ok(n) => off += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(e) => return Err(e).context("write to socket").map_err(Into::into),
        }
    }

    Ok(Flow::Continue)
}

fn is_network_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        if let Some(ioe) = cause.downcast_ref::<io::Error>() {
            matches!(
                ioe.kind(),
                io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::TimedOut
                    | io::ErrorKind::WouldBlock
            )
        } else {
            false
        }
    })
}

fn write_frame_interruptible(
    stream: &mut TcpStream,
    kind: audio_bridge_proto::FrameKind,
    payload: &[u8],
    cmd_rx: &Receiver<BridgeCommand>,
    paused: &mut bool,
) -> Result<Flow> {
    let frame = audio_bridge_proto::encode_frame(kind, payload)
        .context("encode frame")?;
    write_all_interruptible(stream, &frame, cmd_rx, paused)
}

fn send_one_track_over_existing_connection(
    stream: &mut TcpStream,
    cmd_rx: &Receiver<BridgeCommand>,
    paused: &mut bool,
    path: PathBuf,
    ext_hint: String,
) -> Result<Flow> {
    tracing::debug!("bridge stream: next");
    let _ = audio_bridge_proto::write_frame(&mut *stream, audio_bridge_proto::FrameKind::Next, &[]);

    let begin = audio_bridge_proto::encode_begin_file_payload(&ext_hint)
        .context("encode BEGIN_FILE")?;
    tracing::debug!("bridge stream: begin_file");
    match write_frame_interruptible(
        stream,
        audio_bridge_proto::FrameKind::BeginFile,
        &begin,
        cmd_rx,
        paused,
    )? {
        Flow::Continue => {}
        other => return Ok(other),
    }

    if *paused {
        tracing::debug!("bridge stream: pause");
        audio_bridge_proto::write_frame(&mut *stream, audio_bridge_proto::FrameKind::Pause, &[])
            .context("write PAUSE")?;
    }

    let mut file = File::open(&path).with_context(|| format!("open {:?}", path))?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).context("read file")?;
        if n == 0 {
            break;
        }
        match write_frame_interruptible(
            stream,
            audio_bridge_proto::FrameKind::FileChunk,
            &buf[..n],
            cmd_rx,
            paused,
        )? {
            Flow::Continue => {}
            other => return Ok(other),
        }
    }

    tracing::debug!("bridge stream: end_file");
    match write_frame_interruptible(
        stream,
        audio_bridge_proto::FrameKind::EndFile,
        &[],
        cmd_rx,
        paused,
    )? {
        Flow::Continue => {}
        other => return Ok(other),
    }

    Ok(Flow::Continue)
}
