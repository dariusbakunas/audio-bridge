use std::path::{Path, PathBuf};

use actix_web::{get, post, web, HttpResponse, Responder};
use anyhow::Result;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::library::scan_library;
use crate::models::{
    LibraryResponse,
    PlayRequest,
    QueueMode,
    QueueAddRequest,
    QueueItem,
    QueueRemoveRequest,
    QueueResponse,
    StatusResponse,
    BridgeInfo,
    BridgesResponse,
    OutputsResponse,
    OutputSelectRequest,
    OutputInfo,
    OutputCapabilities,
};
use crate::state::AppState;

#[derive(Deserialize, ToSchema)]
pub struct LibraryQuery {
    pub dir: Option<String>,
}

#[utoipa::path(
    get,
    path = "/library",
    params(
        ("dir" = Option<String>, Query, description = "Directory to list")
    ),
    responses(
        (status = 200, description = "Library entries", body = LibraryResponse)
    )
)]
#[get("/library")]
pub async fn list_library(state: web::Data<AppState>, query: web::Query<LibraryQuery>) -> impl Responder {
    let dir = query
        .dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| state.library.read().unwrap().root().to_path_buf());

    let dir = match canonicalize_under_root(&state, &dir) {
        Ok(dir) => dir,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };

    let library = state.library.read().unwrap();
    let entries = match library.list_dir(&dir) {
        Some(entries) => entries.to_vec(),
        None => Vec::new(),
    };
    let resp = LibraryResponse {
        dir: dir.to_string_lossy().to_string(),
        entries,
    };
    HttpResponse::Ok().json(resp)
}

#[utoipa::path(
    post,
    path = "/library/rescan",
    responses(
        (status = 200, description = "Rescan started"),
        (status = 500, description = "Rescan failed")
    )
)]
#[post("/library/rescan")]
pub async fn rescan_library(state: web::Data<AppState>) -> impl Responder {
    let root = state.library.read().unwrap().root().to_path_buf();
    tracing::info!(root = %root.display(), "rescan requested");
    match scan_library(&root) {
        Ok(new_index) => {
            *state.library.write().unwrap() = new_index;
            HttpResponse::Ok().finish()
        }
        Err(e) => HttpResponse::InternalServerError().body(format!("scan failed: {e:#}")),
    }
}

#[utoipa::path(
    post,
    path = "/play",
    request_body = PlayRequest,
    responses(
        (status = 200, description = "Playback started"),
        (status = 400, description = "Bad request"),
        (status = 500, description = "Player offline")
    )
)]
#[post("/play")]
pub async fn play_track(state: web::Data<AppState>, body: web::Json<PlayRequest>) -> impl Responder {
    let path = PathBuf::from(&body.path);
    let path = match canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };

    let mode = body.queue_mode.clone().unwrap_or(QueueMode::Keep);
    let output_id = match &body.output_id {
        Some(id) => id.clone(),
        None => state.bridges.lock().unwrap().active_output_id.clone(),
    };
    if is_pending_output(&output_id) {
        tracing::warn!(output_id = %output_id, "play rejected: output pending");
        return HttpResponse::ServiceUnavailable().body("active output pending");
    }
    if let Err(resp) = ensure_bridge_connected(&state).await {
        tracing::warn!(output_id = %output_id, "play rejected: bridge offline");
        return resp;
    }
    {
        let bridges = state.bridges.lock().unwrap();
        if output_id != bridges.active_output_id {
            tracing::warn!(
                output_id = %output_id,
                active_output_id = %bridges.active_output_id,
                "play rejected: unsupported output id"
            );
            return HttpResponse::BadRequest().body("unsupported output id");
        }
    }
    match mode {
        QueueMode::Keep => {
            let mut queue = state.queue.lock().unwrap();
            if let Some(pos) = queue.items.iter().position(|p| p == &path) {
                queue.items.remove(pos);
            }
        }
        QueueMode::Replace => {
            let mut queue = state.queue.lock().unwrap();
            queue.items.clear();
        }
        QueueMode::Append => {
            let mut queue = state.queue.lock().unwrap();
            if !queue.items.iter().any(|p| p == &path) {
                queue.items.push(path.clone());
            }
        }
    }

    let ext_hint = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    tracing::info!(path = %path.display(), "play request");
    {
        let mut queue = state.queue.lock().unwrap();
        if let Some(pos) = queue.items.iter().position(|p| p == &path) {
            queue.items.remove(pos);
        }
    }
    if state
        .player
        .lock()
        .unwrap()
        .cmd_tx
        .send(crate::bridge::BridgeCommand::Play { path: path.clone(), ext_hint })
        .is_ok()
    {
        tracing::info!(output_id = %output_id, "play dispatched");
        if let Ok(mut s) = state.status.lock() {
            s.now_playing = Some(path);
            s.paused = false;
            s.user_paused = false;
        }
        HttpResponse::Ok().finish()
    } else {
        tracing::warn!(output_id = %output_id, "play failed: command channel closed");
        HttpResponse::InternalServerError().body("player offline")
    }
}

#[utoipa::path(
    post,
    path = "/pause",
    responses(
        (status = 200, description = "Pause toggled"),
        (status = 500, description = "Player offline")
    )
)]
#[post("/pause")]
pub async fn pause_toggle(state: web::Data<AppState>) -> impl Responder {
    tracing::info!("pause toggle request");
    if state
        .player
        .lock()
        .unwrap()
        .cmd_tx
        .send(crate::bridge::BridgeCommand::PauseToggle)
        .is_ok()
    {
        if let Ok(mut s) = state.status.lock() {
            s.paused = !s.paused;
            s.user_paused = s.paused;
        }
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::InternalServerError().body("player offline")
    }
}

#[utoipa::path(
    get,
    path = "/queue",
    responses(
        (status = 200, description = "Queue contents", body = QueueResponse)
    )
)]
#[get("/queue")]
pub async fn queue_list(state: web::Data<AppState>) -> impl Responder {
    let queue = state.queue.lock().unwrap();
    let library = state.library.read().unwrap();
    let items = queue
        .items
        .iter()
        .map(|path| match library.find_track_by_path(path) {
            Some(crate::models::LibraryEntry::Track {
                path,
                file_name,
                duration_ms,
                sample_rate,
                album,
                artist,
                format,
                ..
            }) => QueueItem::Track {
                path,
                file_name,
                duration_ms,
                sample_rate,
                album,
                artist,
                format,
            },
            _ => QueueItem::Missing {
                path: path.to_string_lossy().to_string(),
            },
        })
        .collect();
    HttpResponse::Ok().json(QueueResponse { items })
}

#[utoipa::path(
    post,
    path = "/queue",
    request_body = QueueAddRequest,
    responses(
        (status = 200, description = "Queue updated")
    )
)]
#[post("/queue")]
pub async fn queue_add(state: web::Data<AppState>, body: web::Json<QueueAddRequest>) -> impl Responder {
    let mut added = 0usize;
    {
        let mut queue = state.queue.lock().unwrap();
        for path_str in &body.paths {
            let path = PathBuf::from(path_str);
            let path = match canonicalize_under_root(&state, &path) {
                Ok(dir) => dir,
                Err(_) => continue,
            };
            queue.items.push(path);
            added += 1;
        }
    }
    HttpResponse::Ok().body(format!("added {added}"))
}

#[utoipa::path(
    post,
    path = "/queue/remove",
    request_body = QueueRemoveRequest,
    responses(
        (status = 200, description = "Queue updated"),
        (status = 400, description = "Bad request")
    )
)]
#[post("/queue/remove")]
pub async fn queue_remove(state: web::Data<AppState>, body: web::Json<QueueRemoveRequest>) -> impl Responder {
    let path = PathBuf::from(&body.path);
    let path = match canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let mut queue = state.queue.lock().unwrap();
    if let Some(pos) = queue.items.iter().position(|p| p == &path) {
        queue.items.remove(pos);
    }
    HttpResponse::Ok().finish()
}

#[utoipa::path(
    post,
    path = "/queue/clear",
    responses(
        (status = 200, description = "Queue cleared")
    )
)]
#[post("/queue/clear")]
pub async fn queue_clear(state: web::Data<AppState>) -> impl Responder {
    let mut queue = state.queue.lock().unwrap();
    queue.items.clear();
    HttpResponse::Ok().finish()
}

#[utoipa::path(
    post,
    path = "/queue/next",
    responses(
        (status = 200, description = "Advanced to next"),
        (status = 204, description = "End of queue")
    )
)]
#[post("/queue/next")]
pub async fn queue_next(state: web::Data<AppState>) -> impl Responder {
    let path = {
        let mut queue = state.queue.lock().unwrap();
        if queue.items.is_empty() {
            None
        } else {
            Some(queue.items.remove(0))
        }
    };
    if let Some(path) = path {
        return start_path(&state, path);
    }
    HttpResponse::NoContent().finish()
}

#[utoipa::path(
    get,
    path = "/status",
    responses(
        (status = 200, description = "Playback status", body = StatusResponse)
    )
)]
#[get("/status")]
pub async fn status(state: web::Data<AppState>) -> impl Responder {
    let status = state.status.lock().unwrap();
    let (title, artist, album, format, sample_rate) = match status.now_playing.as_ref() {
        Some(path) => {
            let lib = state.library.read().unwrap();
            match lib.find_track_by_path(path) {
                Some(crate::models::LibraryEntry::Track {
                    file_name,
                    sample_rate,
                    artist,
                    album,
                    format,
                    ..
                }) => (Some(file_name), artist, album, Some(format), sample_rate),
                _ => (None, None, None, None, None),
            }
        }
        None => (None, None, None, None, None),
    };
    let (output_id, http_addr) = {
        let bridges = state.bridges.lock().unwrap();
        let http_addr = bridges
            .bridges
            .iter()
            .find(|b| b.id == bridges.active_bridge_id)
            .map(|b| b.http_addr);
        (bridges.active_output_id.clone(), http_addr)
    };
    let mut resp = StatusResponse {
        now_playing: status.now_playing.as_ref().map(|p| p.to_string_lossy().to_string()),
        paused: status.paused,
        elapsed_ms: status.elapsed_ms,
        duration_ms: status.duration_ms,
        sample_rate,
        channels: status.channels,
        output_sample_rate: status.sample_rate,
        output_device: status.output_device.clone(),
        title,
        artist,
        album,
        format,
        output_id,
        underrun_frames: None,
        underrun_events: None,
        buffer_size_frames: None,
    };
    drop(status);
    if let Some(http_addr) = http_addr {
        match crate::bridge::http_status(http_addr) {
            Ok(remote) => {
                resp.paused = remote.paused;
                resp.elapsed_ms = remote.elapsed_ms;
                resp.duration_ms = remote.duration_ms;
                resp.channels = remote.channels;
                resp.output_sample_rate = remote.sample_rate;
                resp.output_device = remote.device;
                resp.underrun_frames = remote.underrun_frames;
                resp.underrun_events = remote.underrun_events;
                resp.buffer_size_frames = remote.buffer_size_frames;
            }
            Err(e) => {
                tracing::warn!(error = %e, "bridge status poll failed");
            }
        }
    }
    HttpResponse::Ok().json(resp)
}

#[utoipa::path(
    get,
    path = "/bridges",
    responses(
        (status = 200, description = "Configured bridges", body = BridgesResponse)
    )
)]
#[get("/bridges")]
pub async fn bridges_list(state: web::Data<AppState>) -> impl Responder {
    let bridges = state.bridges.lock().unwrap();
    let active_online = state.bridge_online.load(std::sync::atomic::Ordering::Relaxed);
    let items = bridges
        .bridges
        .iter()
        .map(|b| BridgeInfo {
            id: b.id.clone(),
            name: b.name.clone(),
            addr: b.addr.to_string(),
            state: if b.id == bridges.active_bridge_id {
                if active_online {
                    "online".to_string()
                } else {
                    "offline".to_string()
                }
            } else {
                "configured".to_string()
            },
        })
        .collect();
    HttpResponse::Ok().json(BridgesResponse { bridges: items })
}

#[utoipa::path(
    get,
    path = "/bridges/{id}/outputs",
    responses(
        (status = 200, description = "Bridge outputs", body = OutputsResponse),
        (status = 400, description = "Unknown bridge"),
        (status = 500, description = "Bridge unavailable")
    )
)]
#[get("/bridges/{id}/outputs")]
pub async fn bridge_outputs_list(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let (bridge, active_output_id) = {
        let bridges = state.bridges.lock().unwrap();
        let Some(bridge) = bridges.bridges.iter().find(|b| b.id == id.as_str()) else {
            return HttpResponse::BadRequest().body("unknown bridge id");
        };
        (bridge.clone(), bridges.active_output_id.clone())
    };

    let outputs = match build_outputs_for_bridge(&bridge) {
        Ok(outputs) => outputs,
        Err(e) => return HttpResponse::InternalServerError().body(format!("{e:#}")),
    };

    HttpResponse::Ok().json(OutputsResponse {
        active_id: active_output_id,
        outputs,
    })
}

#[utoipa::path(
    get,
    path = "/outputs",
    responses(
        (status = 200, description = "Available outputs", body = OutputsResponse)
    )
)]
#[get("/outputs")]
pub async fn outputs_list(state: web::Data<AppState>) -> impl Responder {
    let bridges = state.bridges.lock().unwrap();
    let active_online = state.bridge_online.load(std::sync::atomic::Ordering::Relaxed);
    let outputs = build_outputs_from_bridges(&bridges.bridges, &bridges.active_bridge_id, active_online);
    HttpResponse::Ok().json(OutputsResponse {
        active_id: bridges.active_output_id.clone(),
        outputs,
    })
}

#[utoipa::path(
    post,
    path = "/outputs/select",
    request_body = OutputSelectRequest,
    responses(
        (status = 200, description = "Active output set"),
        (status = 400, description = "Unknown output")
    )
)]
#[post("/outputs/select")]
pub async fn outputs_select(
    state: web::Data<AppState>,
    body: web::Json<OutputSelectRequest>,
) -> impl Responder {
    let (bridge_id, device_name) = match parse_output_id(&body.id) {
        Ok(x) => x,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let (addr, http_addr) = {
        let bridges = state.bridges.lock().unwrap();
        let Some(bridge) = bridges.bridges.iter().find(|b| b.id == bridge_id) else {
            return HttpResponse::BadRequest().body("unknown bridge id");
        };
        (bridge.addr, bridge.http_addr)
    };

    match crate::bridge::http_list_devices(http_addr) {
        Ok(devices) => {
            if !devices.iter().any(|d| d == &device_name) {
                tracing::warn!(
                    bridge_id = %bridge_id,
                    device = %device_name,
                    "output select rejected: unknown device"
                );
                return HttpResponse::BadRequest().body("unknown device name");
            }
        }
        Err(e) => {
            tracing::warn!(
                bridge_id = %bridge_id,
                error = %e,
                "output select failed: device list"
            );
            return HttpResponse::InternalServerError().body(format!("{e:#}"));
        }
    }

    // Stop current playback before switching outputs.
    {
        let cmd_tx = state.player.lock().unwrap().cmd_tx.clone();
        let _ = cmd_tx.send(crate::bridge::BridgeCommand::Stop);
    }

    if let Err(e) = switch_active_bridge(&state, &bridge_id, addr) {
        tracing::warn!(
            bridge_id = %bridge_id,
            error = %e,
            "output select failed: switch bridge"
        );
        return HttpResponse::InternalServerError().body(format!("{e:#}"));
    }
    if let Err(e) = crate::bridge::http_set_device(http_addr, &device_name) {
        tracing::warn!(
            bridge_id = %bridge_id,
            device = %device_name,
            error = %e,
            "output select failed: set device"
        );
        return HttpResponse::InternalServerError().body(format!("{e:#}"));
    }

    let mut bridges = state.bridges.lock().unwrap();
    bridges.active_bridge_id = bridge_id;
    bridges.active_output_id = body.id.clone();
    tracing::info!(
        output_id = %bridges.active_output_id,
        bridge_id = %bridges.active_bridge_id,
        "output selected"
    );
    HttpResponse::Ok().finish()
}

fn build_outputs_from_bridges(
    bridges: &[crate::config::BridgeConfigResolved],
    active_bridge_id: &str,
    active_online: bool,
) -> Vec<OutputInfo> {
    let mut outputs = Vec::new();
    let mut name_counts = std::collections::HashMap::<String, usize>::new();
    let mut by_bridge = Vec::new();

    for bridge in bridges {
        if bridge.id == active_bridge_id && !active_online {
            continue;
        }
        let devices = crate::bridge::http_list_devices(bridge.http_addr).unwrap_or_default();
        for device in devices {
            *name_counts.entry(device.clone()).or_insert(0) += 1;
            by_bridge.push((bridge, device));
        }
    }

    for (bridge, device) in by_bridge {
        let mut display_name = device.clone();
        if name_counts.get(&device).copied().unwrap_or(0) > 1 {
            display_name = format!("{display_name} [{}]", bridge.name);
        }
        outputs.push(OutputInfo {
            id: format!("bridge:{}:{}", bridge.id, device),
            kind: "bridge".to_string(),
            name: display_name,
            state: "online".to_string(),
            bridge_id: Some(bridge.id.clone()),
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        });
    }

    outputs
}

fn build_outputs_for_bridge(
    bridge: &crate::config::BridgeConfigResolved,
) -> Result<Vec<OutputInfo>> {
    let devices = crate::bridge::http_list_devices(bridge.http_addr)?;
    let mut outputs = Vec::new();
    for device in devices {
        outputs.push(OutputInfo {
            id: format!("bridge:{}:{}", bridge.id, device),
            kind: "bridge".to_string(),
            name: device,
            state: "online".to_string(),
            bridge_id: Some(bridge.id.clone()),
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        });
    }
    Ok(outputs)
}

fn parse_output_id(id: &str) -> Result<(String, String), String> {
    let mut parts = id.splitn(3, ':');
    let kind = parts.next().unwrap_or("");
    let bridge_id = parts.next().unwrap_or("");
    let device = parts.next().unwrap_or("");
    if kind != "bridge" || bridge_id.is_empty() || device.is_empty() {
        return Err("invalid output id".to_string());
    }
    Ok((bridge_id.to_string(), device.to_string()))
}

fn is_pending_output(id: &str) -> bool {
    id.ends_with(":pending")
}

fn switch_active_bridge(
    state: &AppState,
    bridge_id: &str,
    addr: std::net::SocketAddr,
) -> Result<()> {
    let mut bridges = state.bridges.lock().unwrap();
    if bridges.active_bridge_id == bridge_id {
        return Ok(());
    }
    tracing::info!(
        from_bridge_id = %bridges.active_bridge_id,
        to_bridge_id = %bridge_id,
        addr = %addr,
        "switch active bridge"
    );
    bridges.active_bridge_id = bridge_id.to_string();
    drop(bridges);

    state
        .bridge_online
        .store(false, std::sync::atomic::Ordering::Relaxed);
    {
        let mut player = state.player.lock().unwrap();
        let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
    }
    Ok(())
}

async fn ensure_bridge_connected(state: &AppState) -> Result<(), HttpResponse> {
    if state.bridge_online.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }

    let (bridge_id, addr) = {
        let bridges = state.bridges.lock().unwrap();
        let Some(bridge) = bridges.bridges.iter().find(|b| b.id == bridges.active_bridge_id) else {
            return Err(HttpResponse::InternalServerError().body("active bridge not found"));
        };
        (bridge.id.clone(), bridge.addr)
    };

    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
    {
        let mut player = state.player.lock().unwrap();
        let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
        player.cmd_tx = cmd_tx.clone();
    }
    crate::bridge::spawn_bridge_worker(
        bridge_id,
        addr,
        cmd_rx,
        cmd_tx,
        state.status.clone(),
        state.queue.clone(),
        state.bridge_online.clone(),
        state.bridges.clone(),
    );

    let mut waited = 0u64;
    while waited < 2000
        && !state
            .bridge_online
            .load(std::sync::atomic::Ordering::Relaxed)
    {
        actix_web::rt::time::sleep(std::time::Duration::from_millis(100)).await;
        waited += 100;
    }
    if !state.bridge_online.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(HttpResponse::ServiceUnavailable().body("bridge offline"));
    }
    Ok(())
}

fn canonicalize_under_root(state: &AppState, path: &Path) -> Result<PathBuf, String> {
    let root = state.library.read().unwrap().root().to_path_buf();
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let canon = candidate
        .canonicalize()
        .map_err(|_| format!("path does not exist: {:?}", path))?;
    if !canon.starts_with(&root) {
        return Err(format!("path outside library root: {:?}", path));
    }
    Ok(canon)
}

fn start_path(state: &web::Data<AppState>, path: PathBuf) -> HttpResponse {
    let ext_hint = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if state
        .player
        .lock()
        .unwrap()
        .cmd_tx
        .send(crate::bridge::BridgeCommand::Play {
            path: path.clone(),
            ext_hint,
        })
        .is_ok()
    {
        if let Ok(mut s) = state.status.lock() {
            s.now_playing = Some(path);
            s.paused = false;
        }
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::InternalServerError().body("player offline")
    }
}
