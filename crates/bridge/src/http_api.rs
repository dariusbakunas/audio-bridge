//! Bridge HTTP API server.
//!
//! Exposes device listing, playback control, and status endpoints.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use actix_web::http::header;
use actix_web::{App, http::StatusCode, middleware::Logger, web, Error, HttpResponse, HttpServer};
use actix_web::web::Bytes;
use futures_util::{Stream, stream::unfold};
use crossbeam_channel::Sender;

use audio_player::device;
use crate::player::PlayerCommand;
use crate::status::{BridgeStatusState, StatusSnapshot};

/// Health check response payload.
#[derive(serde::Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

/// Device listing response payload.
#[derive(serde::Serialize, Clone, PartialEq, Eq)]
struct DevicesResponse {
    devices: Vec<DeviceInfo>,
    selected: Option<String>,
    selected_id: Option<String>,
}

/// Device metadata sent to clients.
#[derive(serde::Serialize, Clone, PartialEq, Eq)]
struct DeviceInfo {
    id: String,
    name: String,
    min_rate: u32,
    max_rate: u32,
}

/// Request body for selecting a device.
#[derive(serde::Deserialize)]
struct DeviceSelectRequest {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

/// Request body for playback.
#[derive(serde::Deserialize)]
struct PlayRequest {
    url: String,
    #[serde(default)]
    ext_hint: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    seek_ms: Option<u64>,
}

/// Request body for seeking.
#[derive(serde::Deserialize)]
struct SeekRequest {
    ms: u64,
}

const DEVICES_STREAM_INTERVAL: Duration = Duration::from_secs(2);
const STATUS_STREAM_INTERVAL: Duration = Duration::from_secs(1);
const PING_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone)]
struct AppState {
    status: Arc<Mutex<BridgeStatusState>>,
    device_selected: Arc<Mutex<Option<String>>>,
    player_tx: Sender<PlayerCommand>,
}

/// Spawn the HTTP API server on the given bind address.
pub(crate) fn spawn_http_server(
    bind: SocketAddr,
    status: Arc<Mutex<BridgeStatusState>>,
    device_selected: Arc<Mutex<Option<String>>>,
    player_tx: Sender<PlayerCommand>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let state = AppState {
            status,
            device_selected,
            player_tx,
        };
        let runner = match HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(state.clone()))
                .wrap(
                    Logger::new("http request method=%m path=%U status=%s")
                        .exclude("/health")
                )
                .route("/health", web::get().to(health))
                .route("/devices", web::get().to(list_devices))
                .route("/devices/stream", web::get().to(devices_stream))
                .route("/devices/select", web::post().to(select_device))
                .route("/status", web::get().to(status_snapshot))
                .route("/status/stream", web::get().to(status_stream))
                .route("/play", web::post().to(play))
                .route("/pause", web::post().to(pause))
                .route("/resume", web::post().to(resume))
                .route("/stop", web::post().to(stop))
                .route("/seek", web::post().to(seek))
        })
        .bind(bind)
        {
            Ok(server) => server.run(),
            Err(e) => {
                tracing::error!(error = %e, "http server bind failed");
                return;
            }
        };

        tracing::info!(bind = %bind, "http api listening");
        let _ = actix_web::rt::System::new().block_on(runner);
    })
}

async fn health() -> HttpResponse {
    HttpResponse::Ok().json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn list_devices(state: web::Data<AppState>) -> HttpResponse {
    match build_devices_response(&state) {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn devices_stream(state: web::Data<AppState>) -> HttpResponse {
    let mut pending = VecDeque::new();
    let mut last_devices = None;
    if let Ok(resp) = build_devices_response(&state) {
        if let Ok(json) = serde_json::to_string(&resp) {
            pending.push_back(sse_event("devices", &json));
            last_devices = Some(resp);
        }
    }

    let stream = unfold(
        DevicesStreamState {
            state,
            interval: actix_web::rt::time::interval(DEVICES_STREAM_INTERVAL),
            pending,
            last_devices,
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(chunk) = ctx.pending.pop_front() {
                    return Some((Ok(chunk), ctx));
                }

                ctx.interval.tick().await;
                push_ping_if_needed(&mut ctx.pending, &mut ctx.last_ping);
                match build_devices_response(&ctx.state) {
                    Ok(resp) => {
                        if ctx.last_devices.as_ref() != Some(&resp) {
                            ctx.last_devices = Some(resp.clone());
                            if let Ok(json) = serde_json::to_string(&resp) {
                                ctx.pending.push_back(sse_event("devices", &json));
                            }
                        }
                    }
                    Err(e) => {
                        let payload = serde_json::json!({ "error": e }).to_string();
                        ctx.pending.push_back(sse_event("error", &payload));
                    }
                }
            }
        },
    );

    sse_response(stream)
}

async fn select_device(state: web::Data<AppState>, body: web::Bytes) -> HttpResponse {
    let req: DeviceSelectRequest = match parse_json(&body) {
        Ok(req) => req,
        Err(resp) => return resp,
    };

    let mut error: Option<HttpResponse> = None;
    let selected_name = if let Some(id) = req.id {
        let host = cpal::default_host();
        match device::list_device_infos(&host) {
            Ok(devices) => devices
                .into_iter()
                .find(|dev| dev.id == id)
                .map(|dev| dev.name),
            Err(e) => {
                error = Some(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("{e:#}"),
                ));
                None
            }
        }
    } else {
        req.name
    };

    if let Some(resp) = error {
        resp
    } else if let Some(selected_name) = selected_name {
        if let Ok(mut g) = state.device_selected.lock() {
            if selected_name.trim().is_empty() {
                *g = None;
            } else {
                *g = Some(selected_name);
            }
        }
        HttpResponse::NoContent().finish()
    } else {
        error_response(StatusCode::BAD_REQUEST, "unknown device")
    }
}

async fn status_snapshot(state: web::Data<AppState>) -> HttpResponse {
    HttpResponse::Ok().json(build_status_snapshot(&state))
}

async fn status_stream(state: web::Data<AppState>) -> HttpResponse {
    let initial = build_status_snapshot(&state);
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "null".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(sse_event("status", &initial_json));

    let stream = unfold(
        StatusStreamState {
            state,
            interval: actix_web::rt::time::interval(STATUS_STREAM_INTERVAL),
            pending,
            last_status: Some(initial_json),
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(chunk) = ctx.pending.pop_front() {
                    return Some((Ok(chunk), ctx));
                }

                ctx.interval.tick().await;
                push_ping_if_needed(&mut ctx.pending, &mut ctx.last_ping);

                let status = build_status_snapshot(&ctx.state);
                let json = serde_json::to_string(&status).unwrap_or_else(|_| "null".to_string());
                if ctx.last_status.as_deref() != Some(json.as_str()) {
                    ctx.last_status = Some(json.clone());
                    ctx.pending.push_back(sse_event("status", &json));
                }
            }
        },
    );

    sse_response(stream)
}

async fn play(state: web::Data<AppState>, body: web::Bytes) -> HttpResponse {
    let req: PlayRequest = match parse_json(&body) {
        Ok(req) => req,
        Err(resp) => return resp,
    };

    if req.url.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "url is required");
    }

    if state
        .player_tx
        .send(PlayerCommand::Play {
            url: req.url,
            ext_hint: req.ext_hint,
            title: req.title,
            seek_ms: req.seek_ms,
        })
        .is_err()
    {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "player offline")
    } else {
        HttpResponse::NoContent().finish()
    }
}

async fn pause(state: web::Data<AppState>) -> HttpResponse {
    if state.player_tx.send(PlayerCommand::PauseToggle).is_err() {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "player offline")
    } else {
        HttpResponse::NoContent().finish()
    }
}

async fn resume(state: web::Data<AppState>) -> HttpResponse {
    if state.player_tx.send(PlayerCommand::Resume).is_err() {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "player offline")
    } else {
        HttpResponse::NoContent().finish()
    }
}

async fn stop(state: web::Data<AppState>) -> HttpResponse {
    if state.player_tx.send(PlayerCommand::Stop).is_err() {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "player offline")
    } else {
        HttpResponse::NoContent().finish()
    }
}

async fn seek(state: web::Data<AppState>, body: web::Bytes) -> HttpResponse {
    let req: SeekRequest = match parse_json(&body) {
        Ok(req) => req,
        Err(resp) => return resp,
    };

    if state
        .player_tx
        .send(PlayerCommand::Seek { ms: req.ms })
        .is_err()
    {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "player offline")
    } else {
        HttpResponse::NoContent().finish()
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(body: &web::Bytes) -> Result<T, HttpResponse> {
    serde_json::from_slice(body)
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, &format!("invalid json: {e}")))
}

fn build_devices_response(state: &AppState) -> Result<DevicesResponse, String> {
    let host = cpal::default_host();
    let devices = device::list_device_infos(&host)
        .map_err(|e| format!("{e:#}"))?;
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();
    for dev in devices {
        if seen.insert(dev.id.clone()) {
            deduped.push(DeviceInfo {
                id: dev.id,
                name: dev.name,
                min_rate: dev.min_rate,
                max_rate: dev.max_rate,
            });
        }
    }
    deduped.sort_by(|a, b| a.name.cmp(&b.name));
    let selected = state.device_selected.lock().ok().and_then(|g| g.clone());
    let selected_id = selected.as_ref().and_then(|name| {
        deduped
            .iter()
            .find(|dev| dev.name == *name)
            .map(|dev| dev.id.clone())
    });
    Ok(DevicesResponse {
        devices: deduped,
        selected,
        selected_id,
    })
}

fn build_status_snapshot(state: &AppState) -> StatusSnapshot {
    state
        .status
        .lock()
        .map(|s| s.snapshot())
        .unwrap_or_else(|_| StatusSnapshot {
            now_playing: None,
            paused: false,
            elapsed_ms: None,
            duration_ms: None,
            source_codec: None,
            source_bit_depth: None,
            container: None,
            output_sample_format: None,
            resampling: None,
            resample_from_hz: None,
            resample_to_hz: None,
            sample_rate: None,
            channels: None,
            device: None,
            underrun_frames: None,
            underrun_events: None,
            buffer_size_frames: None,
            buffered_frames: None,
            buffer_capacity_frames: None,
            end_reason: None,
        })
}

/// Emit a JSON error response.
fn error_response(status: StatusCode, message: &str) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({ "error": message }))
}

fn sse_event(event: &str, data: &str) -> Bytes {
    let mut payload = String::new();
    payload.push_str("event: ");
    payload.push_str(event);
    payload.push('\n');
    for line in data.lines() {
        payload.push_str("data: ");
        payload.push_str(line);
        payload.push('\n');
    }
    payload.push('\n');
    Bytes::from(payload)
}

fn push_ping_if_needed(pending: &mut VecDeque<Bytes>, last_ping: &mut Instant) {
    if pending.is_empty() && last_ping.elapsed() >= PING_INTERVAL {
        *last_ping = Instant::now();
        pending.push_back(Bytes::from(": ping\n\n"));
    }
}

fn sse_response<S>(stream: S) -> HttpResponse
where
    S: Stream<Item = Result<Bytes, Error>> + 'static,
{
    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/event-stream"))
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::CONNECTION, "keep-alive"))
        .streaming(stream)
}

struct DevicesStreamState {
    state: web::Data<AppState>,
    interval: actix_web::rt::time::Interval,
    pending: VecDeque<Bytes>,
    last_devices: Option<DevicesResponse>,
    last_ping: Instant,
}

struct StatusStreamState {
    state: web::Data<AppState>,
    interval: actix_web::rt::time::Interval,
    pending: VecDeque<Bytes>,
    last_status: Option<String>,
    last_ping: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::body;

    #[actix_web::test]
    async fn error_response_encodes_message() {
        let resp = error_response(StatusCode::NOT_FOUND, "missing");
        let body = body::to_bytes(resp.into_body()).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["error"], "missing");
    }

    #[test]
    fn device_select_request_defaults_to_none() {
        let req: DeviceSelectRequest = serde_json::from_str("{}").unwrap();
        assert!(req.id.is_none());
        assert!(req.name.is_none());
    }

    #[test]
    fn play_request_accepts_optional_fields() {
        let req: PlayRequest = serde_json::from_str(r#"{"url":"http://host/track.flac"}"#).unwrap();
        assert_eq!(req.url, "http://host/track.flac");
        assert!(req.ext_hint.is_none());
        assert!(req.title.is_none());
        assert!(req.seek_ms.is_none());
    }

    #[test]
    fn seek_request_parses_ms() {
        let req: SeekRequest = serde_json::from_str(r#"{"ms":1234}"#).unwrap();
        assert_eq!(req.ms, 1234);
    }
}
