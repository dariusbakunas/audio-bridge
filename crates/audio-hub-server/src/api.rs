//! HTTP API handlers.
//!
//! Defines the Actix routes for library, playback, queue, and output control.

use std::path::PathBuf;

use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use actix_web::http::{header, StatusCode};
use actix_web::body::SizedStream;
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
    QueueRemoveRequest,
    QueueResponse,
    StatusResponse,
    OutputsResponse,
    OutputSelectRequest,
    ProvidersResponse,
};
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

    let dir = match state.output_controller.canonicalize_under_root(&state, &dir) {
        Ok(dir) => dir,
        Err(err) => return err.into_response(),
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
    let path = match state.output_controller.canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(err) => return err.into_response(),
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
    let path = match state.output_controller.canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(err) => return err.into_response(),
    };

    let mode = body.queue_mode.clone().unwrap_or(QueueMode::Keep);
    tracing::info!(path = %path.display(), "play request");
    let output_id = match state
        .output_controller
        .play_request(&state, path.clone(), mode, body.output_id.as_deref())
        .await
    {
        Ok(id) => id,
        Err(err) => return err.into_response(),
    };
    tracing::info!(output_id = %output_id, "play dispatched");
    HttpResponse::Ok().finish()
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
    match state.output_controller.pause_toggle(&state).await {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/stop",
    responses(
        (status = 200, description = "Playback stopped"),
        (status = 500, description = "Player offline")
    )
)]
#[post("/stop")]
pub async fn stop(state: web::Data<AppState>) -> impl Responder {
    tracing::info!("stop request");
    match state.output_controller.stop(&state).await {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
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
    match state.output_controller.seek(&state, ms).await {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
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
    HttpResponse::Ok().json(state.output_controller.queue_list(&state))
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
    let added = state
        .output_controller
        .queue_add_paths(&state, body.paths.clone());
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
    match state
        .output_controller
        .queue_remove_path(&state, &body.path)
    {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
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
    state.output_controller.queue_clear(&state);
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
    match state.output_controller.queue_next(&state).await {
        Ok(true) => HttpResponse::Ok().finish(),
        Ok(false) => HttpResponse::NoContent().finish(),
        Err(err) => err.into_response(),
    }
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
    match state.output_controller.status_for_output(&state, &output_id).await {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(err) => err.into_response(),
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
    HttpResponse::Ok().json(state.output_controller.list_providers(&state))
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
    match state
        .output_controller
        .outputs_for_provider(&state, id.as_str())
        .await
    {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(err) => err.into_response(),
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
    HttpResponse::Ok().json(state.output_controller.list_outputs(&state))
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
    match state.output_controller.select_output(&state, &body.id).await {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
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
