//! Playback-related API handlers.

use std::path::PathBuf;

use actix_web::{get, post, web, HttpResponse, Responder};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::models::{PlayRequest, QueueMode, StatusResponse};
use crate::state::AppState;

/// Seek request payload (milliseconds).
#[derive(Deserialize, ToSchema)]
pub struct SeekBody {
    /// Absolute seek position in milliseconds.
    pub ms: u64,
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
/// Start playback for the requested track.
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
/// Toggle pause/resume.
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
/// Stop playback.
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
/// Seek to an absolute position (milliseconds).
pub async fn seek(state: web::Data<AppState>, body: web::Json<SeekBody>) -> impl Responder {
    let ms = body.ms;
    match state.output_controller.seek(&state, ms).await {
        Ok(()) => HttpResponse::Ok().finish(),
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
/// Return playback status for a specific output.
pub async fn status_for_output(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let output_id = id.into_inner();
    tracing::debug!(output_id = %output_id, "status for output request");
    match state.output_controller.status_for_output(&state, &output_id).await {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(err) => err.into_response(),
    }
}
