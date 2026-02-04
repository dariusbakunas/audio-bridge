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
    OutputsResponse,
    OutputSelectRequest,
    ProvidersResponse,
};
use crate::output_controller;
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
    if let Err(resp) = output_controller::ensure_active_bridge_connected(&state).await {
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
    if let Err(resp) = output_controller::ensure_active_bridge_connected(&state).await {
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
    match output_controller::status_for_output(&state, &output_id).await {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(resp) => resp,
    }
}

#[utoipa::path(
    get,
    path = "/providers",
    responses(
        (status = 200, description = "Available output providers", body = ProvidersResponse)
    )
)]
#[get("/providers")]
pub async fn providers_list(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok().json(output_controller::list_providers(&state))
}

#[utoipa::path(
    get,
    path = "/providers/{id}/outputs",
    responses(
        (status = 200, description = "Provider outputs", body = OutputsResponse),
        (status = 400, description = "Unknown provider"),
        (status = 500, description = "Provider unavailable")
    )
)]
#[get("/providers/{id}/outputs")]
pub async fn provider_outputs_list(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    match output_controller::outputs_for_provider(&state, id.as_str()) {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(resp) => resp,
    }
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
    HttpResponse::Ok().json(output_controller::list_outputs(&state))
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
    match output_controller::select_output(&state, &body.id).await {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(resp) => resp,
    }
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
