use std::path::{Path, PathBuf};

use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use actix_web::http::{header, StatusCode};
use actix_web::body::SizedStream;
use anyhow::Result;
use serde::Deserialize;
use utoipa::ToSchema;
use tokio_util::io::ReaderStream;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

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
    SupportedRates,
};
use crate::bridge_manager::{merge_bridges, parse_output_id};
use crate::state::AppState;

#[derive(Deserialize, ToSchema)]
pub struct LibraryQuery {
    pub dir: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct StreamQuery {
    pub path: String,
}

#[derive(Deserialize, ToSchema)]
pub struct SeekBody {
    pub ms: u64,
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
    get,
    path = "/stream",
    params(
        ("path" = String, Query, description = "Track path under the library root")
    ),
    responses(
        (status = 200, description = "Full file stream"),
        (status = 206, description = "Partial content"),
        (status = 404, description = "Not found"),
        (status = 416, description = "Invalid range")
    )
)]
#[get("/stream")]
pub async fn stream_track(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<StreamQuery>,
) -> impl Responder {
    let path = PathBuf::from(&query.path);
    let path = match canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };

    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return HttpResponse::NotFound().finish(),
    };
    let meta = match file.metadata().await {
        Ok(m) => m,
        Err(_) => return HttpResponse::NotFound().finish(),
    };
    let total_len = meta.len();

    let range_header = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok());
    let range = match range_header.and_then(|h| parse_single_range(h, total_len)) {
        Some(r) => Some(r),
        None if range_header.is_some() => {
            return HttpResponse::RangeNotSatisfiable()
                .insert_header((header::ACCEPT_RANGES, "bytes"))
                .finish();
        }
        None => None,
    };

    let (start, len, status_code) = if let Some((start, end)) = range {
        let len = end.saturating_sub(start).saturating_add(1);
        (start, len, StatusCode::PARTIAL_CONTENT)
    } else {
        (0, total_len, StatusCode::OK)
    };

    if start > 0 {
        if let Err(_) = file.seek(std::io::SeekFrom::Start(start)).await {
            return HttpResponse::InternalServerError().finish();
        }
    }

    let stream = ReaderStream::new(file.take(len));
    let body = SizedStream::new(len, stream);

    let mut resp = HttpResponse::build(status_code);
    resp.insert_header((header::ACCEPT_RANGES, "bytes"));
    if let Some((start, end)) = range {
        resp.insert_header((
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{total_len}"),
        ));
    }
    resp.insert_header((header::CONTENT_LENGTH, len.to_string()));
    resp.body(body)
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
    let output_id = body
        .output_id
        .clone()
        .or_else(|| state.bridges.lock().unwrap().active_output_id.clone());
    let Some(output_id) = output_id else {
        tracing::warn!("play rejected: no active output selected");
        return HttpResponse::ServiceUnavailable().body("no active output selected");
    };
    if let Err(resp) = ensure_bridge_connected(&state).await {
        tracing::warn!(output_id = %output_id, "play rejected: bridge offline");
        return resp;
    }
    {
        let bridges = state.bridges.lock().unwrap();
        if bridges.active_output_id.as_deref() != Some(output_id.as_str()) {
            tracing::warn!(
                output_id = %output_id,
                active_output_id = ?bridges.active_output_id,
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
        .send(crate::bridge::BridgeCommand::Play {
            path: path.clone(),
            ext_hint,
            seek_ms: None,
            start_paused: false,
        })
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
    post,
    path = "/seek",
    request_body = SeekBody,
    responses(
        (status = 200, description = "Seek requested"),
        (status = 500, description = "Player offline")
    )
)]
#[post("/seek")]
pub async fn seek(state: web::Data<AppState>, body: web::Json<SeekBody>) -> impl Responder {
    let ms = body.ms;
    if state
        .player
        .lock()
        .unwrap()
        .cmd_tx
        .send(crate::bridge::BridgeCommand::Seek { ms })
        .is_ok()
    {
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
    if state.bridges.lock().unwrap().active_output_id.is_none() {
        tracing::warn!("queue next rejected: no active output selected");
        return HttpResponse::ServiceUnavailable().body("no active output selected");
    }
    if let Err(resp) = ensure_bridge_connected(&state).await {
        return resp;
    }
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
    path = "/outputs/{id}/status",
    params(
        ("id" = String, Path, description = "Output id")
    ),
    responses(
        (status = 200, description = "Playback status for output", body = StatusResponse),
        (status = 400, description = "Unknown or inactive output")
    )
)]
#[get("/outputs/{id}/status")]
pub async fn status_for_output(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let output_id = id.into_inner();
    if parse_output_id(&output_id).is_err() {
        return HttpResponse::BadRequest().body("invalid output id");
    }
    let (active_output_id, http_addr) = {
        let bridges = state.bridges.lock().unwrap();
        let http_addr = bridges.active_bridge_id.as_ref().and_then(|active_id| {
            bridges
                .bridges
                .iter()
                .find(|b| b.id == *active_id)
                .map(|b| b.http_addr)
        });
        (bridges.active_output_id.clone(), http_addr)
    };
    if active_output_id.as_deref() != Some(output_id.as_str()) {
        return HttpResponse::BadRequest().body("output is not active");
    }
    if let Err(resp) = ensure_bridge_connected(&state).await {
        return resp;
    }

    let status = state.status.lock().unwrap();
    let (title, artist, album, format, sample_rate, bitrate_kbps) = match status.now_playing.as_ref() {
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
                }) => {
                    let bitrate_kbps = estimate_bitrate_kbps(path, status.duration_ms);
                    (Some(file_name), artist, album, Some(format), sample_rate, bitrate_kbps)
                }
                _ => (None, None, None, None, None, None),
            }
        }
        None => (None, None, None, None, None, None),
    };
    let bridge_online = state.bridge_online.load(std::sync::atomic::Ordering::Relaxed);
    let mut resp = StatusResponse {
        now_playing: status.now_playing.as_ref().map(|p| p.to_string_lossy().to_string()),
        paused: status.paused,
        bridge_online,
        elapsed_ms: status.elapsed_ms,
        duration_ms: status.duration_ms,
        source_codec: status.source_codec.clone(),
        source_bit_depth: status.source_bit_depth,
        container: status.container.clone(),
        output_sample_format: status.output_sample_format.clone(),
        resampling: status.resampling,
        resample_from_hz: status.resample_from_hz,
        resample_to_hz: status.resample_to_hz,
        sample_rate,
        channels: status.channels,
        output_sample_rate: status.sample_rate,
        output_device: status.output_device.clone(),
        title,
        artist,
        album,
        format,
        output_id: Some(output_id.clone()),
        bitrate_kbps,
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
                resp.source_codec = remote.source_codec;
                resp.source_bit_depth = remote.source_bit_depth;
                resp.container = remote.container;
                resp.output_sample_format = remote.output_sample_format;
                resp.resampling = remote.resampling;
                resp.resample_from_hz = remote.resample_from_hz;
                resp.resample_to_hz = remote.resample_to_hz;
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
    let bridges_state = state.bridges.lock().unwrap();
    let discovered = state.discovered_bridges.lock().unwrap();
    let active_online = state.bridge_online.load(std::sync::atomic::Ordering::Relaxed);
    let merged = merge_bridges(&bridges_state.bridges, &discovered);
    let items = merged
        .iter()
        .map(|b| BridgeInfo {
            id: b.id.clone(),
            name: b.name.clone(),
            addr: b.http_addr.to_string(),
            state: if bridges_state.active_bridge_id.as_deref() == Some(b.id.as_str()) {
                if active_online {
                    "connected".to_string()
                } else {
                    "idle".to_string()
                }
            } else if bridges_state.bridges.iter().any(|c| c.id == b.id) {
                "configured".to_string()
            } else {
                "discovered".to_string()
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
        let bridges_state = state.bridges.lock().unwrap();
        let discovered = state.discovered_bridges.lock().unwrap();
        let merged = merge_bridges(&bridges_state.bridges, &discovered);
        let Some(bridge) = merged.iter().find(|b| b.id == id.as_str()) else {
            return HttpResponse::BadRequest().body("unknown bridge id");
        };
        (bridge.clone(), bridges_state.active_output_id.clone())
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
    let bridges_state = state.bridges.lock().unwrap();
    let discovered = state.discovered_bridges.lock().unwrap();
    let _active_online = state.bridge_online.load(std::sync::atomic::Ordering::Relaxed);
    tracing::info!(
        count = bridges_state.bridges.len(),
        ids = ?bridges_state.bridges.iter().map(|b| b.id.clone()).collect::<Vec<_>>(),
        active_bridge_id = ?bridges_state.active_bridge_id,
        "outputs: bridge inventory"
    );
    tracing::info!(
        count = discovered.len(),
        ids = ?discovered.keys().cloned().collect::<Vec<_>>(),
        "outputs: discovered bridges"
    );
    let active_id = bridges_state.active_output_id.clone();
    let merged = merge_bridges(&bridges_state.bridges, &discovered);
    let (outputs, failed) = build_outputs_from_bridges_with_failures(&merged);
    if !failed.is_empty() {
        let configured_ids: std::collections::HashSet<String> =
            bridges_state.bridges.iter().map(|b| b.id.clone()).collect();
        drop(bridges_state);
        drop(discovered);
        if let Ok(mut map) = state.discovered_bridges.lock() {
            for id in failed {
                if !configured_ids.contains(&id) {
                    map.remove(&id);
                    tracing::info!(bridge_id = %id, "outputs: removed discovered bridge after device list failure");
                }
            }
        }
    }
    HttpResponse::Ok().json(OutputsResponse {
        active_id,
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
    let http_addr = {
        let bridges_state = state.bridges.lock().unwrap();
        let discovered = state.discovered_bridges.lock().unwrap();
        let merged = merge_bridges(&bridges_state.bridges, &discovered);
        let Some(bridge) = merged.iter().find(|b| b.id == bridge_id) else {
            return HttpResponse::BadRequest().body("unknown bridge id");
        };
        bridge.http_addr
    };

    match crate::bridge::http_list_devices(http_addr) {
        Ok(devices) => {
            if !devices.iter().any(|d| d.name == device_name) {
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

    let resume_info = {
        let status = state.status.lock().unwrap();
        (
            status.now_playing.clone(),
            status.elapsed_ms,
            status.paused,
        )
    };

    // Stop current playback before switching outputs.
    {
        let cmd_tx = state.player.lock().unwrap().cmd_tx.clone();
        let _ = cmd_tx.send(crate::bridge::BridgeCommand::Stop);
    }

    if let Err(e) = switch_active_bridge(&state, &bridge_id, http_addr) {
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

    {
        let mut bridges = state.bridges.lock().unwrap();
        bridges.active_bridge_id = Some(bridge_id);
        bridges.active_output_id = Some(body.id.clone());
        tracing::info!(
            output_id = ?bridges.active_output_id,
            bridge_id = ?bridges.active_bridge_id,
            "output selected"
        );
    }

    if let Err(resp) = ensure_bridge_connected(&state).await {
        return resp;
    }

    if let (Some(path), Some(elapsed_ms)) = (resume_info.0, resume_info.1) {
        let ext_hint = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let start_paused = resume_info.2;
        let _ = state.player.lock().unwrap().cmd_tx.send(
            crate::bridge::BridgeCommand::Play {
                path,
                ext_hint,
                seek_ms: Some(elapsed_ms),
                start_paused,
            },
        );
    }
    HttpResponse::Ok().finish()
}

fn build_outputs_from_bridges_with_failures(
    bridges: &[crate::config::BridgeConfigResolved],
) -> (Vec<OutputInfo>, Vec<String>) {
    let mut outputs = Vec::new();
    let mut name_counts = std::collections::HashMap::<String, usize>::new();
    let mut by_bridge = Vec::new();
    let mut failed = Vec::new();

    for bridge in bridges {
        let devices = match crate::bridge::http_list_devices(bridge.http_addr) {
            Ok(list) => {
                tracing::info!(
                    bridge_id = %bridge.id,
                    bridge_name = %bridge.name,
                    count = list.len(),
                    "outputs: devices listed"
                );
                list
            }
            Err(e) => {
                tracing::warn!(
                    bridge_id = %bridge.id,
                    bridge_name = %bridge.name,
                    error = %e,
                    "outputs: device list failed"
                );
                failed.push(bridge.id.clone());
                Vec::new()
            }
        };
        for device in devices {
            *name_counts.entry(device.name.clone()).or_insert(0) += 1;
            by_bridge.push((bridge, device));
        }
    }

    for (bridge, device) in by_bridge {
        let mut display_name = device.name.clone();
        if name_counts.get(&device.name).copied().unwrap_or(0) > 1 {
            display_name = format!("{display_name} [{}]", bridge.name);
        }
        let supported_rates = normalize_supported_rates(device.min_rate, device.max_rate);
        outputs.push(OutputInfo {
            id: format!("bridge:{}:{}", bridge.id, device.name),
            kind: "bridge".to_string(),
            name: display_name,
            state: "online".to_string(),
            bridge_id: Some(bridge.id.clone()),
            bridge_name: Some(bridge.name.clone()),
            supported_rates,
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        });
    }

    (outputs, failed)
}

fn build_outputs_for_bridge(
    bridge: &crate::config::BridgeConfigResolved,
) -> Result<Vec<OutputInfo>> {
    let devices = crate::bridge::http_list_devices(bridge.http_addr)?;
    let mut outputs = Vec::new();
    for device in devices {
        let supported_rates = normalize_supported_rates(device.min_rate, device.max_rate);
        outputs.push(OutputInfo {
            id: format!("bridge:{}:{}", bridge.id, device.name),
            kind: "bridge".to_string(),
            name: device.name,
            state: "online".to_string(),
            bridge_id: Some(bridge.id.clone()),
            bridge_name: Some(bridge.name.clone()),
            supported_rates,
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        });
    }
    Ok(outputs)
}

fn normalize_supported_rates(min_hz: u32, max_hz: u32) -> Option<SupportedRates> {
    if min_hz == 0 || max_hz == 0 || max_hz < min_hz || max_hz == u32::MAX {
        return None;
    }
    Some(SupportedRates { min_hz, max_hz })
}

fn switch_active_bridge(
    state: &AppState,
    bridge_id: &str,
    http_addr: std::net::SocketAddr,
) -> Result<()> {
    let mut bridges = state.bridges.lock().unwrap();
    if bridges.active_bridge_id.as_deref() == Some(bridge_id) {
        return Ok(());
    }
    tracing::info!(
        from_bridge_id = ?bridges.active_bridge_id,
        to_bridge_id = %bridge_id,
        http_addr = %http_addr,
        "switch active bridge"
    );
    bridges.active_bridge_id = Some(bridge_id.to_string());
    drop(bridges);

    state
        .bridge_online
        .store(false, std::sync::atomic::Ordering::Relaxed);
    {
        let player = state.player.lock().unwrap();
        let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
    }
    Ok(())
}

async fn ensure_bridge_connected(state: &AppState) -> Result<(), HttpResponse> {
    if state.bridge_online.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }

    let (bridge_id, addr) = {
        let bridges_state = state.bridges.lock().unwrap();
        let discovered = state.discovered_bridges.lock().unwrap();
        let merged = merge_bridges(&bridges_state.bridges, &discovered);
        let Some(active_bridge_id) = bridges_state.active_bridge_id.as_ref() else {
            return Err(HttpResponse::ServiceUnavailable().body("no active output selected"));
        };
        let Some(bridge) = merged.iter().find(|b| b.id == *active_bridge_id) else {
            return Err(HttpResponse::ServiceUnavailable().body("active bridge not found"));
        };
        (bridge.id.clone(), bridge.http_addr)
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
        state.public_base_url.clone(),
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

fn estimate_bitrate_kbps(path: &PathBuf, duration_ms: Option<u64>) -> Option<u32> {
    let duration_ms = duration_ms?;
    if duration_ms == 0 {
        return None;
    }
    let size = std::fs::metadata(path).ok()?.len();
    if size == 0 {
        return None;
    }
    let bits = size.saturating_mul(8);
    let kbps = bits
        .saturating_mul(1000)
        .saturating_div(duration_ms)
        .saturating_div(1000);
    u32::try_from(kbps).ok()
}

fn parse_single_range(header: &str, total_len: u64) -> Option<(u64, u64)> {
    let header = header.trim();
    if !header.starts_with("bytes=") {
        return None;
    }
    let range = header.trim_start_matches("bytes=");
    let first = range.split(',').next()?;
    let (start_s, end_s) = first.split_once('-')?;
    if start_s.is_empty() {
        return None;
    }
    let start = start_s.parse::<u64>().ok()?;
    let end = if end_s.is_empty() {
        total_len.saturating_sub(1)
    } else {
        end_s.parse::<u64>().ok()?
    };
    if start >= total_len || end < start {
        return None;
    }
    Some((start, end.min(total_len.saturating_sub(1))))
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
            seek_ms: None,
            start_paused: false,
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
