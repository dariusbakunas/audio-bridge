use std::io::Read;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tiny_http::{Header, Method, Response, Server, StatusCode};

use crate::device;
use crate::status::{BridgeStatus, StatusSnapshot};

#[derive(serde::Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[derive(serde::Serialize)]
struct DevicesResponse {
    devices: Vec<DeviceInfo>,
    selected: Option<String>,
}

#[derive(serde::Serialize)]
struct DeviceInfo {
    name: String,
    min_rate: u32,
    max_rate: u32,
}

#[derive(serde::Deserialize)]
struct DeviceSelectRequest {
    name: String,
}

pub(crate) fn spawn_http_server(
    bind: SocketAddr,
    status: Arc<Mutex<BridgeStatus>>,
    device_selected: Arc<Mutex<Option<String>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let server = match Server::http(bind) {
            Ok(server) => server,
            Err(e) => {
                tracing::error!(error = %e, "http server bind failed");
                return;
            }
        };
        tracing::info!(bind = %bind, "http api listening");

        for mut request in server.incoming_requests() {
            let method = request.method().clone();
            let url = request.url().split('?').next().unwrap_or("").to_string();
            let (status, response) = match (method, url.as_str()) {
                (Method::Get, "/health") => {
                    let body = HealthResponse {
                        status: "ok",
                        version: env!("CARGO_PKG_VERSION"),
                    };
                    json_response(200, &body)
                }
                (Method::Get, "/devices") => {
                    let host = cpal::default_host();
                    match device::list_device_infos(&host) {
                        Ok(devices) => {
                            let mut seen = std::collections::HashSet::new();
                            let mut deduped = Vec::new();
                            for dev in devices {
                                if seen.insert(dev.name.clone()) {
                                    deduped.push(DeviceInfo {
                                        name: dev.name,
                                        min_rate: dev.min_rate,
                                        max_rate: dev.max_rate,
                                    });
                                }
                            }
                            let selected = device_selected.lock().ok().and_then(|g| g.clone());
                            let body = DevicesResponse { devices: deduped, selected };
                            json_response(200, &body)
                        }
                        Err(e) => error_response(500, &format!("{e:#}")),
                    }
                }
                (Method::Post, "/devices/select") => {
                    let mut body = String::new();
                    if let Err(e) = request.as_reader().read_to_string(&mut body) {
                        error_response(400, &format!("read body failed: {e}"))
                    } else {
                        match serde_json::from_str::<DeviceSelectRequest>(&body) {
                            Ok(req) => {
                                if let Ok(mut g) = device_selected.lock() {
                                    if req.name.trim().is_empty() {
                                        *g = None;
                                    } else {
                                        *g = Some(req.name);
                                    }
                                }
                                (204, Response::from_data(Vec::new()).with_status_code(StatusCode(204)))
                            }
                            Err(e) => error_response(400, &format!("invalid json: {e}")),
                        }
                    }
                }
                (Method::Get, "/status") => {
                    let snapshot = status
                        .lock()
                        .map(|s| s.snapshot())
                        .unwrap_or_else(|_| StatusSnapshot {
                            now_playing: None,
                            paused: false,
                            elapsed_ms: None,
                            duration_ms: None,
                            sample_rate: None,
                            channels: None,
                            device: None,
                            underrun_frames: None,
                            underrun_events: None,
                            buffer_size_frames: None,
                        });
                    json_response(200, &snapshot)
                }
                _ => error_response(404, "not found"),
            };

            let response = response.with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
            if should_log_path(&url) {
                tracing::info!(method = %request.method(), path = %url, status = status, "http request");
            }
            let _ = request.respond(response);
        }
    })
}

fn json_response<T: serde::Serialize>(status: u16, body: &T) -> (u16, Response<std::io::Cursor<Vec<u8>>>) {
    match serde_json::to_vec(body) {
        Ok(json) => (status, Response::from_data(json).with_status_code(StatusCode(status))),
        Err(e) => (500, Response::from_string(format!("json encode error: {e}")).with_status_code(StatusCode(500))),
    }
}

fn error_response(status: u16, message: &str) -> (u16, Response<std::io::Cursor<Vec<u8>>>) {
    let body = serde_json::json!({ "error": message });
    (status, Response::from_data(body.to_string()).with_status_code(StatusCode(status)))
}

fn should_log_path(path: &str) -> bool {
    !matches!(path, "/status" | "/health")
}
