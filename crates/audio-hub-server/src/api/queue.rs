//! Queue-related API handlers.

use actix_web::{get, post, web, HttpResponse, Responder};

use crate::models::{QueueAddRequest, QueuePlayFromRequest, QueueRemoveRequest, QueueResponse};
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
/// Add paths to the queue.
pub async fn queue_add(state: web::Data<AppState>, body: web::Json<QueueAddRequest>) -> impl Responder {
    let added = state
        .output_controller
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
    let added = state
        .output_controller
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
    match state
        .output_controller
        .queue_play_from(&state, &body.path)
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
    responses(
        (status = 200, description = "Queue cleared")
    )
)]
#[post("/queue/clear")]
/// Clear the queue.
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
/// Skip to the next queued track.
pub async fn queue_next(state: web::Data<AppState>) -> impl Responder {
    tracing::debug!("queue next request");
    match state.output_controller.queue_next(&state).await {
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
    match state.output_controller.queue_previous(&state).await {
        Ok(true) => HttpResponse::Ok().finish(),
        Ok(false) => HttpResponse::NoContent().finish(),
        Err(err) => err.into_response(),
    }
}
