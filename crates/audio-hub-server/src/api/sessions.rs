//! Session management API handlers.

use actix_web::{get, post, web, HttpResponse, Responder};

use crate::models::{
    OutputInUseError,
    SessionCreateRequest,
    SessionCreateResponse,
    SessionDetailResponse,
    SessionDeleteResponse,
    SessionHeartbeatRequest,
    SessionReleaseOutputResponse,
    SessionSelectOutputRequest,
    SessionSelectOutputResponse,
    SessionSummary,
    SessionsListResponse,
};
use crate::state::AppState;

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
    get,
    path = "/sessions/{id}",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Session detail", body = SessionDetailResponse),
        (status = 404, description = "Session not found")
    )
)]
#[get("/sessions/{id}")]
/// Return detailed session information.
pub async fn sessions_get(id: web::Path<String>) -> impl Responder {
    let session_id = id.into_inner();
    let Some(s) = crate::session_registry::get_session(&session_id) else {
        return HttpResponse::NotFound().body("session not found");
    };
    HttpResponse::Ok().json(SessionDetailResponse {
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
        lease_ttl_sec: s.lease_ttl.as_secs(),
        heartbeat_state: s.heartbeat_state,
        battery: s.battery,
    })
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

#[utoipa::path(
    post,
    path = "/sessions/{id}/select-output",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    request_body = SessionSelectOutputRequest,
    responses(
        (status = 200, description = "Output bound to session", body = SessionSelectOutputResponse),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Output already in use", body = OutputInUseError)
    )
)]
#[post("/sessions/{id}/select-output")]
/// Bind an output to a session (with output-level lock).
pub async fn sessions_select_output(
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<SessionSelectOutputRequest>,
) -> impl Responder {
    let session_id = id.into_inner();
    let payload = body.into_inner();
    let output_id = payload.output_id.trim().to_string();
    if output_id.is_empty() {
        return HttpResponse::BadRequest().body("output_id is required");
    }

    let transition = match crate::session_registry::bind_output(&session_id, &output_id, payload.force) {
        Ok(transition) => transition,
        Err(crate::session_registry::BindError::SessionNotFound) => {
            return HttpResponse::NotFound().body("session not found");
        }
        Err(crate::session_registry::BindError::OutputInUse {
            output_id,
            held_by_session_id,
        }) => {
            return HttpResponse::Conflict().json(OutputInUseError {
                error: "output_in_use".to_string(),
                output_id,
                held_by_session_id,
            });
        }
    };

    if let Err(err) = state.output.controller.select_output(&state, &output_id).await {
        crate::session_registry::rollback_bind(&session_id, &output_id, transition);
        return err.into_response();
    }

    state.events.outputs_changed();
    HttpResponse::Ok().json(SessionSelectOutputResponse {
        session_id,
        output_id,
    })
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/release-output",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Output released", body = SessionReleaseOutputResponse),
        (status = 404, description = "Session not found")
    )
)]
#[post("/sessions/{id}/release-output")]
/// Release the currently bound output (if any) from a session.
pub async fn sessions_release_output(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let session_id = id.into_inner();
    let released_output_id = match crate::session_registry::release_output(&session_id) {
        Ok(released) => released,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    };

    if let Some(output_id) = released_output_id.as_deref() {
        if let Ok(mut bridges) = state.providers.bridge.bridges.lock() {
            if bridges.active_output_id.as_deref() == Some(output_id) {
                bridges.active_output_id = None;
                bridges.active_bridge_id = None;
            }
        }
    }
    state.events.outputs_changed();
    HttpResponse::Ok().json(SessionReleaseOutputResponse {
        session_id,
        released_output_id,
    })
}

#[utoipa::path(
    delete,
    path = "/sessions/{id}",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Session deleted", body = SessionDeleteResponse),
        (status = 404, description = "Session not found")
    )
)]
#[actix_web::delete("/sessions/{id}")]
/// Delete a session and release any held output lock.
pub async fn sessions_delete(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let session_id = id.into_inner();
    let released_output_id = match crate::session_registry::delete_session(&session_id) {
        Ok(released) => released,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    };
    if let Some(output_id) = released_output_id.as_deref() {
        if let Ok(mut bridges) = state.providers.bridge.bridges.lock() {
            if bridges.active_output_id.as_deref() == Some(output_id) {
                bridges.active_output_id = None;
                bridges.active_bridge_id = None;
            }
        }
    }
    state.events.outputs_changed();
    HttpResponse::Ok().json(SessionDeleteResponse {
        session_id,
        released_output_id,
    })
}
