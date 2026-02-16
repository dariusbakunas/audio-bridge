//! Queue-related API handlers.

use actix_web::{get, post, web, HttpResponse, Responder};

use crate::models::{QueueAddRequest, QueueClearRequest, QueuePlayFromRequest, QueueRemoveRequest, QueueResponse};
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/queue",
    responses(
        (status = 200, description = "Queue contents", body = QueueResponse)
    )
)]
#[get("/queue")]
/// Return the current queue.
pub async fn queue_list(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok().json(state.output.controller.queue_list(&state))
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
/// Add paths to the queue.
pub async fn queue_add(state: web::Data<AppState>, body: web::Json<QueueAddRequest>) -> impl Responder {
    let added = state.output.controller
        .queue_add_paths(&state, body.paths.clone());
    HttpResponse::Ok().body(format!("added {added}"))
}

#[utoipa::path(
    post,
    path = "/queue/next/add",
    request_body = QueueAddRequest,
    responses(
        (status = 200, description = "Queue updated")
    )
)]
#[post("/queue/next/add")]
/// Insert paths at the front of the queue.
pub async fn queue_add_next(
    state: web::Data<AppState>,
    body: web::Json<QueueAddRequest>,
) -> impl Responder {
    let added = state.output.controller
        .queue_add_next_paths(&state, body.paths.clone());
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
/// Remove a path from the queue.
pub async fn queue_remove(state: web::Data<AppState>, body: web::Json<QueueRemoveRequest>) -> impl Responder {
    match state.output.controller
        .queue_remove_path(&state, &body.path)
    {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/queue/play_from",
    request_body = QueuePlayFromRequest,
    responses(
        (status = 200, description = "Playback started"),
        (status = 404, description = "Item not found"),
        (status = 500, description = "Player offline")
    )
)]
#[post("/queue/play_from")]
/// Play a queued item and drop items ahead of it.
pub async fn queue_play_from(
    state: web::Data<AppState>,
    body: web::Json<QueuePlayFromRequest>,
) -> impl Responder {
    let path = if let Some(track_id) = body.track_id {
        match state.metadata.db.track_path_for_id(track_id) {
            Ok(Some(path)) => path,
            Ok(None) => return HttpResponse::NotFound().finish(),
            Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
        }
    } else if let Some(path) = body.path.as_ref() {
        path.clone()
    } else {
        return HttpResponse::BadRequest().body("path or track_id is required");
    };

    match state.output.controller
        .queue_play_from(&state, &path)
        .await
    {
        Ok(true) => HttpResponse::Ok().finish(),
        Ok(false) => HttpResponse::NotFound().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/queue/clear",
    request_body = QueueClearRequest,
    responses(
        (status = 200, description = "Queue cleared")
    )
)]
#[post("/queue/clear")]
/// Clear the queue.
pub async fn queue_clear(
    state: web::Data<AppState>,
    body: Option<web::Json<QueueClearRequest>>,
) -> impl Responder {
    let clear_history = body
        .as_ref()
        .map(|req| req.clear_history)
        .unwrap_or(false);
    let clear_queue = body
        .as_ref()
        .map(|req| req.clear_queue)
        .unwrap_or(true);
    state.output.controller.queue_clear(&state, clear_queue, clear_history);
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
/// Skip to the next queued track.
pub async fn queue_next(state: web::Data<AppState>) -> impl Responder {
    tracing::debug!("queue next request");
    match state.output.controller.queue_next(&state).await {
        Ok(true) => HttpResponse::Ok().finish(),
        Ok(false) => HttpResponse::NoContent().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/queue/previous",
    responses(
        (status = 200, description = "Playback started"),
        (status = 204, description = "No previous track")
    )
)]
#[post("/queue/previous")]
/// Skip to the previously played track.
pub async fn queue_previous(state: web::Data<AppState>) -> impl Responder {
    tracing::debug!("queue previous request");
    match state.output.controller.queue_previous(&state).await {
        Ok(true) => HttpResponse::Ok().finish(),
        Ok(false) => HttpResponse::NoContent().finish(),
        Err(err) => err.into_response(),
    }
}
