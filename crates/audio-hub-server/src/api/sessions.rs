//! Session management API handlers.

use actix_web::{get, post, web, HttpResponse, Responder};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::models::{
    OutputInUseError,
    QueueAddRequest,
    QueueClearRequest,
    QueuePlayFromRequest,
    QueueRemoveRequest,
    QueueResponse,
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

fn require_session(session_id: &str) -> Result<(), HttpResponse> {
    if crate::session_registry::touch_session(session_id) {
        Ok(())
    } else {
        Err(HttpResponse::NotFound().body("session not found"))
    }
}

#[utoipa::path(
    get,
    path = "/sessions/{id}/queue",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Queue contents", body = QueueResponse),
        (status = 404, description = "Session not found")
    )
)]
#[get("/sessions/{id}/queue")]
/// Return queue for a session.
pub async fn sessions_queue_list(state: web::Data<AppState>, id: web::Path<String>) -> impl Responder {
    let session_id = id.into_inner();
    if let Err(resp) = require_session(&session_id) {
        return resp;
    }
    let snapshot = match crate::session_registry::queue_snapshot(&session_id) {
        Ok(snapshot) => snapshot,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    };
    HttpResponse::Ok().json(build_queue_response(&state, snapshot))
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/queue",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    request_body = QueueAddRequest,
    responses(
        (status = 200, description = "Queue updated"),
        (status = 404, description = "Session not found")
    )
)]
#[post("/sessions/{id}/queue")]
/// Add paths to a session queue.
pub async fn sessions_queue_add(
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<QueueAddRequest>,
) -> impl Responder {
    let session_id = id.into_inner();
    if let Err(resp) = require_session(&session_id) {
        return resp;
    }
    let mut resolved = Vec::new();
    for path_str in &body.paths {
        let candidate = PathBuf::from(path_str);
        let path = match state.output.controller.canonicalize_under_root(&state, &candidate) {
            Ok(path) => path,
            Err(_) => continue,
        };
        resolved.push(path);
    }
    let added = match crate::session_registry::queue_add_paths(&session_id, resolved) {
        Ok(added) => added,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    };
    HttpResponse::Ok().body(format!("added {added}"))
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/queue/next/add",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    request_body = QueueAddRequest,
    responses(
        (status = 200, description = "Queue updated"),
        (status = 404, description = "Session not found")
    )
)]
#[post("/sessions/{id}/queue/next/add")]
/// Insert paths at the front of a session queue.
pub async fn sessions_queue_add_next(
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<QueueAddRequest>,
) -> impl Responder {
    let session_id = id.into_inner();
    if let Err(resp) = require_session(&session_id) {
        return resp;
    }
    let mut resolved = Vec::new();
    for path_str in &body.paths {
        let candidate = PathBuf::from(path_str);
        let path = match state.output.controller.canonicalize_under_root(&state, &candidate) {
            Ok(path) => path,
            Err(_) => continue,
        };
        resolved.push(path);
    }
    let added = match crate::session_registry::queue_add_next_paths(&session_id, resolved) {
        Ok(added) => added,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    };
    HttpResponse::Ok().body(format!("added {added}"))
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/queue/remove",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    request_body = QueueRemoveRequest,
    responses(
        (status = 200, description = "Queue updated"),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Session not found")
    )
)]
#[post("/sessions/{id}/queue/remove")]
/// Remove an item from a session queue.
pub async fn sessions_queue_remove(
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<QueueRemoveRequest>,
) -> impl Responder {
    let session_id = id.into_inner();
    if let Err(resp) = require_session(&session_id) {
        return resp;
    }
    let candidate = PathBuf::from(&body.path);
    let path = match state.output.controller.canonicalize_under_root(&state, &candidate) {
        Ok(path) => path,
        Err(err) => return err.into_response(),
    };
    match crate::session_registry::queue_remove_path(&session_id, &path) {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(()) => HttpResponse::NotFound().body("session not found"),
    }
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/queue/play_from",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    request_body = QueuePlayFromRequest,
    responses(
        (status = 200, description = "Playback started"),
        (status = 404, description = "Session or queue item not found"),
        (status = 500, description = "Player offline")
    )
)]
#[post("/sessions/{id}/queue/play_from")]
/// Play from a queued item in a session.
pub async fn sessions_queue_play_from(
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<QueuePlayFromRequest>,
) -> impl Responder {
    let session_id = id.into_inner();
    if let Err(resp) = require_session(&session_id) {
        return resp;
    }

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

    let canonical = {
        let candidate = PathBuf::from(&path);
        match state.output.controller.canonicalize_under_root(&state, &candidate) {
            Ok(path) => path,
            Err(err) => return err.into_response(),
        }
    };

    let found = match crate::session_registry::queue_play_from(&session_id, &canonical) {
        Ok(found) => found,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    };
    if !found {
        return HttpResponse::NotFound().finish();
    }

    match state
        .output
        .session_playback
        .play_path(&state, &session_id, canonical)
        .await
    {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/queue/clear",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    request_body = QueueClearRequest,
    responses(
        (status = 200, description = "Queue cleared"),
        (status = 404, description = "Session not found")
    )
)]
#[post("/sessions/{id}/queue/clear")]
/// Clear a session queue.
pub async fn sessions_queue_clear(
    _state: web::Data<AppState>,
    id: web::Path<String>,
    body: Option<web::Json<QueueClearRequest>>,
) -> impl Responder {
    let session_id = id.into_inner();
    if let Err(resp) = require_session(&session_id) {
        return resp;
    }
    let clear_history = body.as_ref().map(|req| req.clear_history).unwrap_or(false);
    let clear_queue = body.as_ref().map(|req| req.clear_queue).unwrap_or(true);
    match crate::session_registry::queue_clear(&session_id, clear_queue, clear_history) {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(()) => HttpResponse::NotFound().body("session not found"),
    }
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/queue/next",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Advanced to next"),
        (status = 204, description = "End of queue"),
        (status = 404, description = "Session not found")
    )
)]
#[post("/sessions/{id}/queue/next")]
/// Skip to the next track in a session queue.
pub async fn sessions_queue_next(state: web::Data<AppState>, id: web::Path<String>) -> impl Responder {
    let session_id = id.into_inner();
    if let Err(resp) = require_session(&session_id) {
        return resp;
    }
    let Some(next_path) = (match crate::session_registry::queue_next_path(&session_id) {
        Ok(path) => path,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    }) else {
        return HttpResponse::NoContent().finish();
    };
    match state
        .output
        .session_playback
        .play_path(&state, &session_id, next_path)
        .await
    {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/queue/previous",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Playback started"),
        (status = 204, description = "No previous track"),
        (status = 404, description = "Session not found")
    )
)]
#[post("/sessions/{id}/queue/previous")]
/// Skip to previous track in a session queue.
pub async fn sessions_queue_previous(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let session_id = id.into_inner();
    if let Err(resp) = require_session(&session_id) {
        return resp;
    }
    let Some(prev_path) = (match crate::session_registry::queue_previous_path(&session_id) {
        Ok(path) => path,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    }) else {
        return HttpResponse::NoContent().finish();
    };
    match state
        .output
        .session_playback
        .play_path(&state, &session_id, prev_path)
        .await
    {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

fn build_queue_response(
    state: &AppState,
    snapshot: crate::session_registry::SessionQueueSnapshot,
) -> QueueResponse {
    let mut items: Vec<crate::models::QueueItem> = snapshot
        .queue_items
        .iter()
        .map(|path| build_queue_item(state, path, false, false))
        .collect();

    if let Some(current_path) = snapshot.now_playing.as_ref() {
        let current_str = current_path.to_string_lossy();
        let index = items.iter().position(|item| match item {
            crate::models::QueueItem::Track { path, .. } => path == current_str.as_ref(),
            crate::models::QueueItem::Missing { path } => path == current_str.as_ref(),
        });
        if let Some(index) = index {
            if index != 0 {
                let current = items.remove(index);
                items.insert(0, current);
            }
            if let Some(crate::models::QueueItem::Track { now_playing, .. }) = items.get_mut(0) {
                *now_playing = true;
            }
        } else {
            items.insert(0, build_queue_item(state, current_path, true, false));
        }
    }

    let mut played_paths = Vec::new();
    for path in snapshot.history.iter().rev() {
        if snapshot.now_playing.as_deref() == Some(path.as_path()) {
            continue;
        }
        played_paths.push(path.clone());
        if played_paths.len() >= 10 {
            break;
        }
    }

    if !played_paths.is_empty() {
        played_paths.reverse();
        let mut seen = HashSet::new();
        for item in &items {
            match item {
                crate::models::QueueItem::Track { path, .. } => {
                    seen.insert(path.clone());
                }
                crate::models::QueueItem::Missing { path } => {
                    seen.insert(path.clone());
                }
            }
        }

        let mut played_items = Vec::new();
        for path in played_paths {
            let path_str = path.to_string_lossy().to_string();
            if seen.contains(&path_str) {
                continue;
            }
            played_items.push(build_queue_item(state, &path, false, true));
        }

        if !played_items.is_empty() {
            played_items.append(&mut items);
            items = played_items;
        }
    }

    QueueResponse { items }
}

fn build_queue_item(
    state: &AppState,
    path: &PathBuf,
    now_playing: bool,
    played: bool,
) -> crate::models::QueueItem {
    let lib = state.library.read().unwrap();
    if let Some(crate::models::LibraryEntry::Track {
        file_name,
        sample_rate,
        album,
        artist,
        format,
        ..
    }) = lib.find_track_by_path(path)
    {
        let path_str = path.to_string_lossy().to_string();
        let track_id = state.metadata.db.track_id_for_path(&path_str).ok().flatten();
        let title = state
            .metadata
            .db
            .track_record_by_path(&path_str)
            .ok()
            .flatten()
            .and_then(|record| record.title);
        let duration_ms = state
            .metadata
            .db
            .track_record_by_path(&path_str)
            .ok()
            .flatten()
            .and_then(|record| record.duration_ms);
        crate::models::QueueItem::Track {
            id: track_id,
            path: path_str,
            file_name,
            title,
            duration_ms,
            sample_rate,
            album,
            artist,
            format,
            now_playing,
            played,
        }
    } else {
        crate::models::QueueItem::Missing {
            path: path.to_string_lossy().to_string(),
        }
    }
}
