//! Session management API handlers.

use actix_web::{get, post, web, HttpResponse, Responder};

use crate::models::{
    SessionCreateRequest,
    SessionCreateResponse,
    SessionHeartbeatRequest,
    SessionSummary,
    SessionsListResponse,
};

#[utoipa::path(
    post,
    path = "/sessions",
    request_body = SessionCreateRequest,
    responses(
        (status = 200, description = "Session created or refreshed", body = SessionCreateResponse),
        (status = 400, description = "Invalid request")
    )
)]
#[post("/sessions")]
/// Create or refresh a session by `(mode, client_id)`.
pub async fn sessions_create(body: web::Json<SessionCreateRequest>) -> impl Responder {
    let req = body.into_inner();
    let name = req.name.trim().to_string();
    let client_id = req.client_id.trim().to_string();
    let app_version = req.app_version.trim().to_string();
    if name.is_empty() || client_id.is_empty() || app_version.is_empty() {
        return HttpResponse::BadRequest().body("name, client_id, and app_version are required");
    }
    let (session_id, lease_ttl_sec) = crate::session_registry::create_or_refresh(
        name,
        req.mode,
        client_id,
        app_version,
        req.owner,
        req.lease_ttl_sec,
    );
    HttpResponse::Ok().json(SessionCreateResponse {
        session_id,
        lease_ttl_sec,
    })
}

#[utoipa::path(
    get,
    path = "/sessions",
    responses(
        (status = 200, description = "Known sessions", body = SessionsListResponse)
    )
)]
#[get("/sessions")]
/// List known sessions.
pub async fn sessions_list() -> impl Responder {
    let sessions = crate::session_registry::list_sessions()
        .into_iter()
        .map(|s| SessionSummary {
            id: s.id,
            name: s.name,
            mode: s.mode,
            client_id: s.client_id,
            app_version: s.app_version,
            owner: s.owner,
            active_output_id: s.active_output_id,
            queue_len: s.queue_len,
            created_age_ms: s.created_at.elapsed().as_millis() as u64,
            last_seen_age_ms: s.last_seen.elapsed().as_millis() as u64,
        })
        .collect();
    HttpResponse::Ok().json(SessionsListResponse { sessions })
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/heartbeat",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    request_body = SessionHeartbeatRequest,
    responses(
        (status = 200, description = "Heartbeat accepted"),
        (status = 404, description = "Session not found")
    )
)]
#[post("/sessions/{id}/heartbeat")]
/// Update session heartbeat metadata.
pub async fn sessions_heartbeat(
    id: web::Path<String>,
    body: web::Json<SessionHeartbeatRequest>,
) -> impl Responder {
    let session_id = id.into_inner();
    let req = body.into_inner();
    let state = req.state.trim().to_string();
    if state.is_empty() {
        return HttpResponse::BadRequest().body("state is required");
    }
    match crate::session_registry::heartbeat(&session_id, state, req.battery) {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(()) => HttpResponse::NotFound().body("session not found"),
    }
}
