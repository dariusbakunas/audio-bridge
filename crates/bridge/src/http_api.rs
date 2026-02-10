//! Bridge HTTP API server.
//!
//! Exposes device listing, playback control, and status endpoints.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use actix_web::{http::StatusCode, middleware::Logger, web, App, HttpResponse, HttpServer};
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
#[derive(serde::Serialize)]
struct DevicesResponse {
    devices: Vec<DeviceInfo>,
    selected: Option<String>,
    selected_id: Option<String>,
}

/// Device metadata sent to clients.
#[derive(serde::Serialize)]
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
                .wrap(Logger::new("http request method=%m path=%U status=%s").exclude("/status").exclude("/health"))
                .route("/health", web::get().to(health))
                .route("/devices", web::get().to(list_devices))
                .route("/devices/select", web::post().to(select_device))
                .route("/status", web::get().to(status_snapshot))
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
    let host = cpal::default_host();
    match device::list_device_infos(&host) {
        Ok(devices) => {
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
            HttpResponse::Ok().json(DevicesResponse {
                devices: deduped,
                selected,
                selected_id,
            })
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}")),
    }
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
    let snapshot = state
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
        });
    HttpResponse::Ok().json(snapshot)
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

/// Emit a JSON error response.
fn error_response(status: StatusCode, message: &str) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({ "error": message }))
}

/// Filter noisy paths from logging output.
fn should_log_path(path: &str) -> bool {
    !matches!(path, "/status" | "/health")
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::body;

    #[test]
    fn should_log_path_filters_health_and_status() {
        assert!(!should_log_path("/status"));
        assert!(!should_log_path("/health"));
        assert!(should_log_path("/devices"));
    }

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
