//! Session management API handlers.

use actix_web::{get, post, web, Error, HttpRequest, HttpResponse, Responder};
use actix_web::http::header;
use actix_web::web::Bytes;
use futures_util::{Stream, stream::unfold};
use serde::Deserialize;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{Duration, Interval, MissedTickBehavior};
use utoipa::ToSchema;

use crate::events::HubEvent;
use crate::models::{
    LocalPlaybackPlayResponse,
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
    SessionLockInfo,
    SessionLocksResponse,
    SessionReleaseOutputResponse,
    SessionSelectOutputRequest,
    SessionSelectOutputResponse,
    SessionSummary,
    SessionsListResponse,
    StatusResponse,
};
use crate::state::AppState;

const PROTECTED_SESSION_NAMES: [&str; 2] = ["default", "local"];

#[derive(serde::Deserialize)]
pub struct SessionViewerQuery {
    #[serde(default)]
    pub client_id: Option<String>,
}

/// Session seek request payload (milliseconds).
#[derive(Deserialize, ToSchema)]
pub struct SessionSeekBody {
    /// Absolute seek position in milliseconds.
    pub ms: u64,
}

const SESSION_STATUS_PING_INTERVAL: Duration = Duration::from_secs(15);

struct SessionStatusStreamState {
    state: web::Data<AppState>,
    session_id: String,
    receiver: broadcast::Receiver<HubEvent>,
    interval: Interval,
    pending: VecDeque<Bytes>,
    last_status: Option<String>,
    last_ping: Instant,
}

struct SessionQueueStreamState {
    state: web::Data<AppState>,
    session_id: String,
    receiver: broadcast::Receiver<HubEvent>,
    interval: Interval,
    pending: VecDeque<Bytes>,
    last_queue: Option<String>,
    last_ping: Instant,
}

enum SessionStreamSignal {
    Tick,
    Event(Result<HubEvent, RecvError>),
}

fn session_sse_event(event: &str, data: &str) -> Bytes {
    let mut payload = String::new();
    payload.push_str("event: ");
    payload.push_str(event);
    payload.push('\n');
    for line in data.lines() {
        payload.push_str("data: ");
        payload.push_str(line);
        payload.push('\n');
    }
    payload.push('\n');
    Bytes::from(payload)
}

fn push_session_ping_if_needed(pending: &mut VecDeque<Bytes>, last_ping: &mut Instant) {
    if pending.is_empty() && last_ping.elapsed() >= SESSION_STATUS_PING_INTERVAL {
        *last_ping = Instant::now();
        pending.push_back(Bytes::from(": ping\n\n"));
    }
}

async fn recv_session_signal(
    receiver: &mut broadcast::Receiver<HubEvent>,
    interval: &mut Interval,
) -> SessionStreamSignal {
    tokio::select! {
        _ = interval.tick() => SessionStreamSignal::Tick,
        result = receiver.recv() => SessionStreamSignal::Event(result),
    }
}

fn session_sse_response<S>(stream: S) -> HttpResponse
where
    S: Stream<Item = Result<Bytes, Error>> + 'static,
{
    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/event-stream"))
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::CONNECTION, "keep-alive"))
        .streaming(stream)
}

fn cache_session_status(state: &AppState, session_id: &str, status: &StatusResponse) {
    if let Ok(mut cache) = state.output.session_status_cache.lock() {
        cache.insert(session_id.to_string(), status.clone());
    }
}

fn cached_session_status(state: &AppState, session_id: &str) -> Option<StatusResponse> {
    state
        .output
        .session_status_cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(session_id).cloned())
}

fn clear_cached_session_status(state: &AppState, session_id: &str) {
    if let Ok(mut cache) = state.output.session_status_cache.lock() {
        cache.remove(session_id);
    }
}

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
/// Create or refresh a session:
/// - remote mode by `(mode, name)`
/// - local mode by `(mode, client_id)`.
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
pub async fn sessions_list(query: web::Query<SessionViewerQuery>) -> impl Responder {
    let viewer_client_id = query.client_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let sessions = crate::session_registry::list_sessions_visible(viewer_client_id)
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
    path = "/sessions/locks",
    responses(
        (status = 200, description = "Active output/bridge locks", body = SessionLocksResponse)
    )
)]
#[get("/sessions/locks")]
/// Return active output and bridge lock ownership.
pub async fn sessions_locks() -> impl Responder {
    let (output_locks, bridge_locks) = crate::session_registry::lock_snapshot();
    HttpResponse::Ok().json(SessionLocksResponse {
        output_locks: output_locks
            .into_iter()
            .map(|(key, session_id)| SessionLockInfo { key, session_id })
            .collect(),
        bridge_locks: bridge_locks
            .into_iter()
            .map(|(key, session_id)| SessionLockInfo { key, session_id })
            .collect(),
    })
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
pub async fn sessions_get(
    id: web::Path<String>,
    query: web::Query<SessionViewerQuery>,
) -> impl Responder {
    let session_id = id.into_inner();
    let viewer_client_id = query.client_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let Some(s) = crate::session_registry::get_session_visible(&session_id, viewer_client_id) else {
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
    if output_id.starts_with("browser:") {
        let Some(session) = crate::session_registry::get_session(&session_id) else {
            return HttpResponse::NotFound().body("session not found");
        };
        if !matches!(session.mode, crate::models::SessionMode::Local) {
            return HttpResponse::BadRequest().body("browser outputs can only be selected by local sessions");
        }
    }
    let previous_output_id = crate::session_registry::get_session(&session_id)
        .and_then(|session| session.active_output_id);
    let pre_switch_status = state.output.session_playback.status(&state, &session_id).await.ok();
    let resume_path = pre_switch_status
        .as_ref()
        .and_then(|status| status.now_playing.as_ref())
        .map(PathBuf::from)
        .or_else(|| {
            crate::session_registry::queue_snapshot(&session_id)
                .ok()
                .and_then(|snapshot| snapshot.now_playing)
        });
    let resume_elapsed_ms = pre_switch_status.as_ref().and_then(|status| status.elapsed_ms);
    let resume_paused = pre_switch_status
        .as_ref()
        .map(|status| status.paused)
        .unwrap_or(false);
    if previous_output_id.as_deref() != Some(output_id.as_str()) && resume_path.is_some() {
        if let Err(err) = state.output.session_playback.stop(&state, &session_id).await {
            tracing::warn!(
                session_id = %session_id,
                previous_output_id = ?previous_output_id,
                error = ?err,
                "session output switch pre-stop failed"
            );
        }
    }

    match crate::session_registry::bind_output(&session_id, &output_id, payload.force) {
        Ok(_) => {}
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
        Err(crate::session_registry::BindError::BridgeInUse {
            bridge_id,
            held_by_session_id,
        }) => {
            return HttpResponse::Conflict().body(format!(
                "bridge_in_use bridge_id={bridge_id} held_by_session_id={held_by_session_id}"
            ));
        }
    }

    if let Some(path) = resume_path {
        if let Err(err) = state
            .output
            .session_playback
            .play_path(&state, &session_id, path)
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                output_id = %output_id,
                error = ?err,
                "session output switch playback migration failed"
            );
        } else {
            if let Some(ms) = resume_elapsed_ms.filter(|ms| *ms > 0) {
                if let Err(err) = state.output.session_playback.seek(&state, &session_id, ms).await
                {
                    tracing::warn!(
                        session_id = %session_id,
                        output_id = %output_id,
                        elapsed_ms = ms,
                        error = ?err,
                        "session output switch seek restore failed"
                    );
                }
            }
            if resume_paused {
                if let Err(err) = state.output.session_playback.pause_toggle(&state, &session_id).await
                {
                    tracing::warn!(
                        session_id = %session_id,
                        output_id = %output_id,
                        error = ?err,
                        "session output switch pause restore failed"
                    );
                }
            }
        }
    }

    state.events.status_changed();
    state.events.queue_changed();
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
    clear_cached_session_status(&state, &session_id);
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
        (status = 403, description = "Default session cannot be deleted"),
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
    if let Some(session) = crate::session_registry::get_session(&session_id) {
        if PROTECTED_SESSION_NAMES
            .iter()
            .any(|name| session.name.trim().eq_ignore_ascii_case(name))
        {
            return HttpResponse::Forbidden().body("default session cannot be deleted");
        }
        if session.active_output_id.is_some() {
            if let Err(err) = state.output.session_playback.stop(&state, &session_id).await {
                return err.into_response();
            }
        }
    }
    let released_output_id = match crate::session_registry::delete_session(&session_id) {
        Ok(released) => released,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    };
    clear_cached_session_status(&state, &session_id);
    state.events.outputs_changed();
    HttpResponse::Ok().json(SessionDeleteResponse {
        session_id,
        released_output_id,
    })
}

#[utoipa::path(
    get,
    path = "/sessions/{id}/status",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Playback status for session output", body = StatusResponse),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session output is in use by another session"),
        (status = 503, description = "Session has no output selected or output is unavailable")
    )
)]
#[get("/sessions/{id}/status")]
/// Return playback status for the output bound to this session.
pub async fn sessions_status(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let session_id = id.into_inner();
    match state.output.session_playback.status(&state, &session_id).await {
        Ok(resp) => {
            cache_session_status(&state, &session_id, &resp);
            HttpResponse::Ok().json(resp)
        }
        Err(err) => match cached_session_status(&state, &session_id) {
            Some(cached) => HttpResponse::Ok().json(cached),
            None => err.into_response(),
        },
    }
}

#[utoipa::path(
    get,
    path = "/sessions/{id}/status/stream",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Session status event stream"),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session output is in use by another session"),
        (status = 503, description = "Session has no output selected or output is unavailable")
    )
)]
#[get("/sessions/{id}/status/stream")]
/// Stream status updates for a specific session via server-sent events.
pub async fn sessions_status_stream(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let session_id = id.into_inner();
    let initial = match state.output.session_playback.status(&state, &session_id).await {
        Ok(resp) => {
            cache_session_status(&state, &session_id, &resp);
            resp
        }
        Err(err) => match cached_session_status(&state, &session_id) {
            Some(cached) => cached,
            None => return err.into_response(),
        },
    };
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "null".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(session_sse_event("status", &initial_json));

    let mut interval = tokio::time::interval(SESSION_STATUS_PING_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let receiver = state.events.subscribe();

    let stream = unfold(
        SessionStatusStreamState {
            state: state.clone(),
            session_id,
            receiver,
            interval,
            pending,
            last_status: Some(initial_json),
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }

                let mut refresh = false;
                match recv_session_signal(&mut ctx.receiver, &mut ctx.interval).await {
                    SessionStreamSignal::Tick => {}
                    SessionStreamSignal::Event(result) => match result {
                        Ok(HubEvent::StatusChanged) => refresh = true,
                        Ok(HubEvent::OutputsChanged) => refresh = true,
                        Ok(HubEvent::QueueChanged) => {}
                        Ok(HubEvent::Metadata(_)) => {}
                        Ok(HubEvent::LibraryChanged) => {}
                        Err(RecvError::Lagged(_)) => refresh = true,
                        Err(RecvError::Closed) => return None,
                    },
                }

                if refresh {
                    if let Ok(status) = ctx
                        .state
                        .output
                        .session_playback
                        .status(&ctx.state, &ctx.session_id)
                        .await
                    {
                        cache_session_status(&ctx.state, &ctx.session_id, &status);
                        let json = serde_json::to_string(&status)
                            .unwrap_or_else(|_| "null".to_string());
                        if ctx.last_status.as_deref() != Some(json.as_str()) {
                            ctx.last_status = Some(json.clone());
                            ctx.pending.push_back(session_sse_event("status", &json));
                        }
                    } else if let Some(status) = cached_session_status(&ctx.state, &ctx.session_id) {
                        let json = serde_json::to_string(&status)
                            .unwrap_or_else(|_| "null".to_string());
                        if ctx.last_status.as_deref() != Some(json.as_str()) {
                            ctx.last_status = Some(json.clone());
                            ctx.pending.push_back(session_sse_event("status", &json));
                        }
                    }
                }

                push_session_ping_if_needed(&mut ctx.pending, &mut ctx.last_ping);
            }
        },
    );

    session_sse_response(stream)
}

#[utoipa::path(
    get,
    path = "/sessions/{id}/queue/stream",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Session queue event stream"),
        (status = 404, description = "Session not found")
    )
)]
#[get("/sessions/{id}/queue/stream")]
/// Stream queue updates for a specific session via server-sent events.
pub async fn sessions_queue_stream(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let session_id = id.into_inner();
    if let Err(resp) = require_session(&session_id) {
        return resp;
    }

    let initial_snapshot = match crate::session_registry::queue_snapshot(&session_id) {
        Ok(snapshot) => snapshot,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    };
    let initial = build_queue_response(&state, initial_snapshot);
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "null".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(session_sse_event("queue", &initial_json));

    let mut interval = tokio::time::interval(Duration::from_secs(15));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let receiver = state.events.subscribe();

    let stream = unfold(
        SessionQueueStreamState {
            state: state.clone(),
            session_id,
            receiver,
            interval,
            pending,
            last_queue: Some(initial_json),
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }

                let mut refresh = false;
                match recv_session_signal(&mut ctx.receiver, &mut ctx.interval).await {
                    SessionStreamSignal::Tick => {}
                    SessionStreamSignal::Event(result) => match result {
                        Ok(HubEvent::QueueChanged) => refresh = true,
                        Ok(HubEvent::StatusChanged) => refresh = true,
                        Ok(HubEvent::OutputsChanged) => {}
                        Ok(HubEvent::Metadata(_)) => {}
                        Ok(HubEvent::LibraryChanged) => {}
                        Err(RecvError::Lagged(_)) => refresh = true,
                        Err(RecvError::Closed) => return None,
                    },
                }

                if refresh {
                    if let Ok(snapshot) = crate::session_registry::queue_snapshot(&ctx.session_id) {
                        let queue = build_queue_response(&ctx.state, snapshot);
                        let json = serde_json::to_string(&queue)
                            .unwrap_or_else(|_| "null".to_string());
                        if ctx.last_queue.as_deref() != Some(json.as_str()) {
                            ctx.last_queue = Some(json.clone());
                            ctx.pending.push_back(session_sse_event("queue", &json));
                        }
                    }
                }

                push_session_ping_if_needed(&mut ctx.pending, &mut ctx.last_ping);
            }
        },
    );

    session_sse_response(stream)
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/pause",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Pause toggled"),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session output is in use by another session"),
        (status = 503, description = "Session has no output selected or output is unavailable")
    )
)]
#[post("/sessions/{id}/pause")]
/// Toggle pause/resume for the session output.
pub async fn sessions_pause(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let session_id = id.into_inner();
    match state.output.session_playback.pause_toggle(&state, &session_id).await {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/seek",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    request_body = SessionSeekBody,
    responses(
        (status = 200, description = "Seek requested"),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session output is in use by another session"),
        (status = 503, description = "Session has no output selected or output is unavailable")
    )
)]
#[post("/sessions/{id}/seek")]
/// Seek the session output to an absolute position (milliseconds).
pub async fn sessions_seek(
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<SessionSeekBody>,
) -> impl Responder {
    let session_id = id.into_inner();
    match state
        .output
        .session_playback
        .seek(&state, &session_id, body.ms)
        .await
    {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/stop",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Playback stopped"),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session output is in use by another session"),
        (status = 503, description = "Session has no output selected or output is unavailable")
    )
)]
#[post("/sessions/{id}/stop")]
/// Stop playback for the session output.
pub async fn sessions_stop(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let session_id = id.into_inner();
    match state.output.session_playback.stop(&state, &session_id).await {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

fn require_session(session_id: &str) -> Result<(), HttpResponse> {
    if crate::session_registry::touch_session(session_id) {
        Ok(())
    } else {
        Err(HttpResponse::NotFound().body("session not found"))
    }
}

fn is_local_session(session_id: &str) -> bool {
    matches!(
        crate::session_registry::get_session(session_id).map(|s| s.mode),
        Some(crate::models::SessionMode::Local)
    )
}

fn build_local_playback_response(
    state: &AppState,
    req: &HttpRequest,
    path: PathBuf,
) -> LocalPlaybackPlayResponse {
    let conn = req.connection_info();
    let base_url = format!("{}://{}", conn.scheme(), conn.host());
    let url = crate::stream_url::build_stream_url_for(&path, &base_url, Some(&state.metadata.db));
    let path_str = path.to_string_lossy().to_string();
    let track_id = state
        .metadata
        .db
        .track_id_for_path(&path_str)
        .ok()
        .flatten();
    LocalPlaybackPlayResponse {
        url,
        path: path_str,
        track_id,
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
    if added > 0 {
        state.events.queue_changed();
    }
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
    if added > 0 {
        state.events.queue_changed();
    }
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
        Ok(removed) => {
            if removed {
                state.events.queue_changed();
            }
            HttpResponse::Ok().finish()
        }
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
    req: HttpRequest,
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
    state.events.queue_changed();
    state.events.status_changed();

    if is_local_session(&session_id) {
        let payload = build_local_playback_response(&state, &req, canonical);
        return HttpResponse::Ok().json(payload);
    }

    match state.output.session_playback.play_path(&state, &session_id, canonical).await {
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
    state: web::Data<AppState>,
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
        Ok(()) => {
            state.events.queue_changed();
            HttpResponse::Ok().finish()
        }
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
pub async fn sessions_queue_next(
    state: web::Data<AppState>,
    id: web::Path<String>,
    req: HttpRequest,
) -> impl Responder {
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
    state.events.queue_changed();
    state.events.status_changed();
    if is_local_session(&session_id) {
        let payload = build_local_playback_response(&state, &req, next_path);
        return HttpResponse::Ok().json(payload);
    }

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
    req: HttpRequest,
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
    state.events.queue_changed();
    state.events.status_changed();
    if is_local_session(&session_id) {
        let payload = build_local_playback_response(&state, &req, prev_path);
        return HttpResponse::Ok().json(payload);
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use crate::bridge::BridgeCommand;
    use crate::events::{EventBus, LogBus};
    use crate::models::{SessionMode, SessionSelectOutputRequest};
    use crate::state::{
        BridgeProviderState, BridgeState, CastProviderState, DeviceSelectionState, LocalProviderState,
        MetadataWake, PlayerStatus, QueueState,
    };

    fn make_state() -> web::Data<AppState> {
        let root = std::env::temp_dir().join(format!(
            "audio-hub-server-sessions-switch-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let library = crate::library::scan_library(&root).expect("scan library");
        let metadata_db = crate::metadata_db::MetadataDb::new(&root).expect("metadata db");

        let (bridge_cmd_tx, _bridge_cmd_rx) = crossbeam_channel::unbounded();
        let bridges_state = Arc::new(Mutex::new(BridgeState {
            bridges: Vec::new(),
            active_bridge_id: None,
            active_output_id: None,
        }));
        let bridge_state = Arc::new(BridgeProviderState::new(
            bridge_cmd_tx,
            bridges_state,
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new(HashMap::new())),
            "http://localhost".to_string(),
        ));

        let (local_cmd_tx, _local_cmd_rx) = crossbeam_channel::unbounded();
        let local_state = Arc::new(LocalProviderState {
            enabled: false,
            id: "local".to_string(),
            name: "Local Host".to_string(),
            player: Arc::new(Mutex::new(crate::bridge::BridgePlayer { cmd_tx: local_cmd_tx })),
            running: Arc::new(AtomicBool::new(false)),
        });

        let status = Arc::new(Mutex::new(PlayerStatus::default()));
        let events = EventBus::new();
        let status_store = crate::status_store::StatusStore::new(status, events.clone());
        let queue = Arc::new(Mutex::new(QueueState::default()));
        let queue_service = crate::queue_service::QueueService::new(queue, status_store.clone(), events.clone());
        let playback_manager = crate::playback_manager::PlaybackManager::new(
            bridge_state.player.clone(),
            status_store,
            queue_service,
        );
        let device_selection = DeviceSelectionState {
            local: Arc::new(Mutex::new(None)),
            bridge: Arc::new(Mutex::new(HashMap::new())),
        };
        let cast_state = Arc::new(CastProviderState::new());

        web::Data::new(AppState::new(
            library,
            metadata_db,
            None,
            MetadataWake::new(),
            bridge_state,
            local_state,
            cast_state,
            playback_manager,
            device_selection,
            events,
            Arc::new(LogBus::new(64)),
            Arc::new(Mutex::new(crate::state::OutputSettingsState::default())),
            None,
        ))
    }

    #[actix_web::test]
    async fn select_output_while_playing_stops_previous_output_and_starts_new_output() {
        let state = make_state();
        let unique = uuid::Uuid::new_v4().to_string();
        let old_device_id = format!("old-{unique}");
        let new_device_id = format!("new-{unique}");
        let old_output_id = format!("cast:{old_device_id}");
        let new_output_id = format!("cast:{new_device_id}");

        let (old_tx, old_rx) = crossbeam_channel::unbounded::<BridgeCommand>();
        let (new_tx, new_rx) = crossbeam_channel::unbounded::<BridgeCommand>();
        {
            let mut workers = state
                .providers
                .cast
                .workers
                .lock()
                .expect("cast workers lock");
            workers.insert(old_output_id.clone(), old_tx);
            workers.insert(new_output_id.clone(), new_tx);
        }
        {
            let mut discovered = state
                .providers
                .cast
                .discovered
                .lock()
                .expect("cast discovered lock");
            discovered.insert(
                old_device_id.clone(),
                crate::state::DiscoveredCast {
                    id: old_device_id.clone(),
                    name: "Old Cast".to_string(),
                    host: Some("127.0.0.1".to_string()),
                    port: 8009,
                    last_seen: std::time::Instant::now(),
                },
            );
            discovered.insert(
                new_device_id.clone(),
                crate::state::DiscoveredCast {
                    id: new_device_id.clone(),
                    name: "New Cast".to_string(),
                    host: Some("127.0.0.1".to_string()),
                    port: 8009,
                    last_seen: std::time::Instant::now(),
                },
            );
        }
        {
            let mut status_by_output = state
                .providers
                .cast
                .status_by_output
                .lock()
                .expect("cast status lock");
            status_by_output.insert(
                old_output_id.clone(),
                audio_bridge_types::BridgeStatus {
                    now_playing: Some("/tmp/test-track.flac".to_string()),
                    paused: false,
                    elapsed_ms: Some(12_345),
                    duration_ms: Some(180_000),
                    ..Default::default()
                },
            );
        }

        let session_name = format!("switch-test-{unique}");
        let client_id = format!("client-{unique}");
        let (session_id, _) = crate::session_registry::create_or_refresh(
            session_name,
            SessionMode::Remote,
            client_id,
            "test".to_string(),
            Some("test".to_string()),
            Some(30),
        );
        crate::session_registry::bind_output(&session_id, &old_output_id, false)
            .expect("bind old output");

        let app = actix_web::test::init_service(
            actix_web::App::new()
                .app_data(state.clone())
                .service(crate::api::sessions_select_output),
        )
        .await;
        let req = actix_web::test::TestRequest::post()
            .uri(&format!("/sessions/{}/select-output", urlencoding::encode(&session_id)))
            .set_json(SessionSelectOutputRequest {
                output_id: new_output_id.clone(),
                force: false,
            })
            .to_request();
        let response = actix_web::test::call_service(&app, req).await;
        assert_eq!(response.status(), actix_web::http::StatusCode::OK);

        let stopped_old = old_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("old output should receive stop");
        assert!(matches!(stopped_old, BridgeCommand::Stop));

        let mut saw_new_play = false;
        for _ in 0..3 {
            let cmd = new_rx
                .recv_timeout(Duration::from_millis(500))
                .expect("new output should receive command");
            if matches!(cmd, BridgeCommand::Play { .. }) {
                saw_new_play = true;
                break;
            }
        }
        assert!(saw_new_play, "new output did not receive play command");
    }
}
