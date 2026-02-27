//! Local playback session endpoints.
//!
//! These endpoints are intentionally decoupled from output selection and
//! hub playback state. They only manage session identity and stream URL
//! resolution for client-side playback.

use std::path::PathBuf;

use actix_web::{get, post, web, HttpResponse, Responder};

use crate::models::{
    LocalPlaybackPlayRequest,
    LocalPlaybackPlayResponse,
    LocalPlaybackRegisterRequest,
    LocalPlaybackRegisterResponse,
    LocalPlaybackSessionInfo,
    LocalPlaybackSessionsResponse,
};
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/local-playback/register",
    request_body = LocalPlaybackRegisterRequest,
    responses(
        (status = 200, description = "Session registered", body = LocalPlaybackRegisterResponse),
        (status = 400, description = "Invalid request")
    )
)]
#[post("/local-playback/register")]
/// Register or refresh a local playback session.
pub async fn local_playback_register(body: web::Json<LocalPlaybackRegisterRequest>) -> impl Responder {
    let req = body.into_inner();
    let kind = req.kind.trim().to_ascii_lowercase();
    let name = req.name.trim().to_string();
    let client_id = req.client_id.trim().to_string();
    let app_version = req.app_version.trim().to_string();

    if kind.is_empty() || name.is_empty() || client_id.is_empty() || app_version.is_empty() {
        return HttpResponse::BadRequest().body("kind, name, client_id, and app_version are required");
    }
    let has_valid_kind_char = kind
        .chars()
        .any(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !has_valid_kind_char {
        return HttpResponse::BadRequest().body("kind contains no valid characters");
    }

    let session_id = crate::local_playback_sessions::register_session(kind, name, client_id, app_version);
    HttpResponse::Ok().json(LocalPlaybackRegisterResponse {
        play_url: format!("/local-playback/{session_id}/play"),
        session_id,
    })
}

#[utoipa::path(
    post,
    path = "/local-playback/{session_id}/play",
    params(
        ("session_id" = String, Path, description = "Local playback session id")
    ),
    request_body = LocalPlaybackPlayRequest,
    responses(
        (status = 200, description = "Resolved stream URL", body = LocalPlaybackPlayResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session or track not found")
    )
)]
#[post("/local-playback/{session_id}/play")]
/// Resolve a stream URL for a local playback session.
pub async fn local_playback_play(
    state: web::Data<AppState>,
    session_id: web::Path<String>,
    body: web::Json<LocalPlaybackPlayRequest>,
    req: actix_web::HttpRequest,
) -> impl Responder {
    let session_id = session_id.into_inner();
    if !crate::local_playback_sessions::has_session(&session_id) {
        tracing::warn!(session_id = %session_id, reason = "session_not_found", "local playback play failed");
        return HttpResponse::NotFound().body("session not found");
    }
    let _ = crate::local_playback_sessions::touch_session(&session_id);
    let payload = body.into_inner();

    let _resolved_path = match state.metadata.db.track_path_for_id(payload.track_id) {
        Ok(Some(path)) => {
            let candidate = PathBuf::from(path);
            match state.output.controller.canonicalize_under_root(&state, &candidate) {
                Ok(path) => path,
                Err(err) => return err.into_response(),
            }
        }
        Ok(None) => {
            tracing::warn!(session_id = %session_id, track_id = payload.track_id, reason = "track_id_not_found", "local playback play failed");
            return HttpResponse::NotFound().body("track not found");
        }
        Err(err) => {
            tracing::warn!(session_id = %session_id, track_id = payload.track_id, error = %err, reason = "track_lookup_error", "local playback play failed");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let conn = req.connection_info();
    let base_url = format!("{}://{}", conn.scheme(), conn.host());
    let url = format!(
        "{}/stream/track/{}",
        base_url.trim_end_matches('/'),
        payload.track_id
    );

    HttpResponse::Ok().json(LocalPlaybackPlayResponse {
        url,
        track_id: payload.track_id,
    })
}

#[utoipa::path(
    get,
    path = "/local-playback/sessions",
    responses(
        (status = 200, description = "Registered local playback sessions", body = LocalPlaybackSessionsResponse)
    )
)]
#[get("/local-playback/sessions")]
/// List registered local playback sessions with age metadata.
pub async fn local_playback_sessions() -> impl Responder {
    let sessions = crate::local_playback_sessions::list_sessions()
        .into_iter()
        .map(|s| LocalPlaybackSessionInfo {
            session_id: s.session_id,
            kind: s.kind,
            name: s.name,
            app_version: s.app_version,
            created_age_ms: s.created_at.elapsed().as_millis() as u64,
            last_seen_age_ms: s.last_seen.elapsed().as_millis() as u64,
        })
        .collect();
    HttpResponse::Ok().json(LocalPlaybackSessionsResponse { sessions })
}
