//! Session management API handlers.

use actix_web::http::header;
use actix_web::web::Bytes;
use actix_web::{Error, HttpRequest, HttpResponse, Responder, get, post, web};
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
    LocalPlaybackPlayResponse, OutputInUseError, QueueAddRequest, QueueClearRequest,
    QueuePlayFromRequest, QueueRemoveRequest, QueueResponse, SessionCreateRequest,
    SessionCreateResponse, SessionDeleteResponse, SessionDetailResponse, SessionHeartbeatRequest,
    SessionLockInfo, SessionLocksResponse, SessionMuteRequest, SessionReleaseOutputResponse,
    SessionSelectOutputRequest, SessionSelectOutputResponse, SessionSummary, SessionVolumeResponse,
    SessionVolumeSetRequest, SessionsListResponse, StatusResponse,
};
use crate::session_playback_manager::SessionPlaybackError;
use crate::state::AppState;

const PROTECTED_SESSION_NAMES: [&str; 2] = ["default", "local"];

#[derive(serde::Deserialize)]
/// Optional viewer context for filtering session visibility.
pub struct SessionViewerQuery {
    /// Viewer client id; required to see local sessions owned by that client.
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
const SESSION_STATUS_CAST_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// Per-connection state for `/sessions/{id}/status/stream` SSE.
struct SessionStatusStreamState {
    state: web::Data<AppState>,
    session_id: String,
    receiver: broadcast::Receiver<HubEvent>,
    interval: Interval,
    pending: VecDeque<Bytes>,
    last_status: Option<String>,
    last_ping: Instant,
}

/// Per-connection state for `/sessions/{id}/queue/stream` SSE.
struct SessionQueueStreamState {
    state: web::Data<AppState>,
    session_id: String,
    receiver: broadcast::Receiver<HubEvent>,
    interval: Interval,
    pending: VecDeque<Bytes>,
    last_queue: Option<String>,
    last_ping: Instant,
}

/// Internal signal source for SSE loop coordination.
enum SessionStreamSignal {
    Tick,
    Event(Result<HubEvent, RecvError>),
}

/// Encode one SSE event payload chunk.
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

/// Enqueue a keepalive ping frame when stream is idle.
fn push_session_ping_if_needed(pending: &mut VecDeque<Bytes>, last_ping: &mut Instant) {
    if pending.is_empty() && last_ping.elapsed() >= SESSION_STATUS_PING_INTERVAL {
        *last_ping = Instant::now();
        pending.push_back(Bytes::from(": ping\n\n"));
    }
}

/// Wait for either timer tick or hub event bus message.
async fn recv_session_signal(
    receiver: &mut broadcast::Receiver<HubEvent>,
    interval: &mut Interval,
) -> SessionStreamSignal {
    tokio::select! {
        _ = interval.tick() => SessionStreamSignal::Tick,
        result = receiver.recv() => SessionStreamSignal::Event(result),
    }
}

/// Build common SSE response headers for a byte stream.
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

/// Update session status cache with latest snapshot.
fn cache_session_status(state: &AppState, session_id: &str, status: &StatusResponse) {
    if let Ok(mut cache) = state.output.session_status_cache.lock() {
        cache.insert(session_id.to_string(), status.clone());
    }
}

/// Read cached session status snapshot if present.
fn cached_session_status(state: &AppState, session_id: &str) -> Option<StatusResponse> {
    state
        .output
        .session_status_cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(session_id).cloned())
}

/// Remove cached session status snapshot.
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
    let viewer_client_id = query
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
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
    let viewer_client_id = query
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(s) = crate::session_registry::get_session_visible(&session_id, viewer_client_id)
    else {
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
            tracing::warn!(session_id = %session_id, output_id = %output_id, reason = "session_not_found", "select output failed");
            return HttpResponse::NotFound().body("session not found");
        };
        if !matches!(session.mode, crate::models::SessionMode::Local) {
            return HttpResponse::BadRequest()
                .body("browser outputs can only be selected by local sessions");
        }
    }
    let previous_output_id = crate::session_registry::get_session(&session_id)
        .and_then(|session| session.active_output_id);
    let pre_switch_status = state
        .output
        .session_playback
        .status(&state, &session_id)
        .await
        .ok();
    let resume_path = pre_switch_status
        .as_ref()
        .and_then(|status| status.now_playing_track_id)
        .and_then(|track_id| state.metadata.db.track_path_for_id(track_id).ok().flatten())
        .map(PathBuf::from)
        .or_else(|| {
            crate::session_registry::queue_snapshot(&session_id)
                .ok()
                .and_then(|snapshot| snapshot.now_playing)
                .and_then(|track_id| state.metadata.db.track_path_for_id(track_id).ok().flatten())
                .map(PathBuf::from)
        });
    let resume_elapsed_ms = pre_switch_status
        .as_ref()
        .and_then(|status| status.elapsed_ms);
    let resume_paused = pre_switch_status
        .as_ref()
        .map(|status| status.paused)
        .unwrap_or(false);
    if previous_output_id.as_deref() != Some(output_id.as_str()) && resume_path.is_some() {
        if let Err(err) = state
            .output
            .session_playback
            .stop(&state, &session_id)
            .await
        {
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
            tracing::warn!(session_id = %session_id, output_id = %output_id, reason = "session_not_found", "select output failed");
            return HttpResponse::NotFound().body("session not found");
        }
        Err(crate::session_registry::BindError::OutputInUse {
            output_id,
            held_by_session_id,
        }) => {
            tracing::warn!(session_id = %session_id, output_id = %output_id, held_by_session_id = %held_by_session_id, reason = "output_in_use", "select output conflict");
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
            tracing::warn!(session_id = %session_id, output_id = %output_id, bridge_id = %bridge_id, held_by_session_id = %held_by_session_id, reason = "bridge_in_use", "select output conflict");
            return HttpResponse::Conflict().body(format!(
                "bridge_in_use bridge_id={bridge_id} held_by_session_id={held_by_session_id}"
            ));
        }
    }

    if let Some(path) = resume_path {
        let resume_seek_ms = resume_elapsed_ms.filter(|ms| *ms > 0);
        if let Err(err) = state
            .output
            .session_playback
            .play_path_with_options(&state, &session_id, path, resume_seek_ms, resume_paused)
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                output_id = %output_id,
                error = ?err,
                "session output switch playback migration failed"
            );
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
        Err(()) => {
            tracing::warn!(session_id = %session_id, reason = "session_not_found", "release output failed");
            return HttpResponse::NotFound().body("session not found");
        }
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
pub async fn sessions_delete(state: web::Data<AppState>, id: web::Path<String>) -> impl Responder {
    let session_id = id.into_inner();
    if let Some(session) = crate::session_registry::get_session(&session_id) {
        if PROTECTED_SESSION_NAMES
            .iter()
            .any(|name| session.name.trim().eq_ignore_ascii_case(name))
        {
            return HttpResponse::Forbidden().body("default session cannot be deleted");
        }
        if session.active_output_id.is_some() {
            if let Err(err) = state
                .output
                .session_playback
                .stop(&state, &session_id)
                .await
            {
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
pub async fn sessions_status(state: web::Data<AppState>, id: web::Path<String>) -> impl Responder {
    let session_id = id.into_inner();
    match state
        .output
        .session_playback
        .status(&state, &session_id)
        .await
    {
        Ok(resp) => {
            cache_session_status(&state, &session_id, &resp);
            HttpResponse::Ok().json(resp)
        }
        Err(err) => {
            let cached = cached_session_status(&state, &session_id);
            let has_cached = cached.is_some();
            log_session_status_error(&session_id, "status", &err, has_cached);
            match cached {
                Some(cached) => HttpResponse::Ok().json(cached),
                None => err.into_response(),
            }
        }
    }
}

#[utoipa::path(
    get,
    path = "/sessions/{id}/volume",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    responses(
        (status = 200, description = "Volume state for session output", body = SessionVolumeResponse),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session output is in use by another session"),
        (status = 503, description = "Session has no output selected or output is unavailable")
    )
)]
#[get("/sessions/{id}/volume")]
/// Return volume state for the output bound to this session.
pub async fn sessions_volume(state: web::Data<AppState>, id: web::Path<String>) -> impl Responder {
    let session_id = id.into_inner();
    match state
        .output
        .session_playback
        .volume(&state, &session_id)
        .await
    {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/volume",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    request_body = SessionVolumeSetRequest,
    responses(
        (status = 200, description = "Volume set", body = SessionVolumeResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session output is in use by another session"),
        (status = 503, description = "Session has no output selected or output is unavailable")
    )
)]
#[post("/sessions/{id}/volume")]
/// Set volume for the output bound to this session.
pub async fn sessions_volume_set(
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<SessionVolumeSetRequest>,
) -> impl Responder {
    let session_id = id.into_inner();
    let value = body.into_inner().value.min(100);
    match state
        .output
        .session_playback
        .set_volume(&state, &session_id, value)
        .await
    {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/mute",
    params(
        ("id" = String, Path, description = "Session id")
    ),
    request_body = SessionMuteRequest,
    responses(
        (status = 200, description = "Mute state set", body = SessionVolumeResponse),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session output is in use by another session"),
        (status = 503, description = "Session has no output selected or output is unavailable")
    )
)]
#[post("/sessions/{id}/mute")]
/// Set mute state for the output bound to this session.
pub async fn sessions_mute_set(
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<SessionMuteRequest>,
) -> impl Responder {
    let session_id = id.into_inner();
    let muted = body.into_inner().muted;
    match state
        .output
        .session_playback
        .set_mute(&state, &session_id, muted)
        .await
    {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(err) => err.into_response(),
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
    let initial = match state
        .output
        .session_playback
        .status(&state, &session_id)
        .await
    {
        Ok(resp) => {
            cache_session_status(&state, &session_id, &resp);
            resp
        }
        Err(err) => {
            let cached = cached_session_status(&state, &session_id);
            let has_cached = cached.is_some();
            log_session_status_error(&session_id, "status_stream_initial", &err, has_cached);
            match cached {
                Some(cached) => cached,
                None => return err.into_response(),
            }
        }
    };
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "null".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(session_sse_event("status", &initial_json));

    let mut interval = tokio::time::interval(SESSION_STATUS_CAST_REFRESH_INTERVAL);
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
                    SessionStreamSignal::Tick => {
                        refresh = session_should_periodic_refresh(&ctx.session_id);
                    }
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
                        let json =
                            serde_json::to_string(&status).unwrap_or_else(|_| "null".to_string());
                        if ctx.last_status.as_deref() != Some(json.as_str()) {
                            ctx.last_status = Some(json.clone());
                            ctx.pending.push_back(session_sse_event("status", &json));
                        }
                    } else if let Some(status) = cached_session_status(&ctx.state, &ctx.session_id)
                    {
                        let json =
                            serde_json::to_string(&status).unwrap_or_else(|_| "null".to_string());
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
                        let json =
                            serde_json::to_string(&queue).unwrap_or_else(|_| "null".to_string());
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
pub async fn sessions_pause(state: web::Data<AppState>, id: web::Path<String>) -> impl Responder {
    let session_id = id.into_inner();
    match state
        .output
        .session_playback
        .pause_toggle(&state, &session_id)
        .await
    {
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
pub async fn sessions_stop(state: web::Data<AppState>, id: web::Path<String>) -> impl Responder {
    let session_id = id.into_inner();
    match state
        .output
        .session_playback
        .stop(&state, &session_id)
        .await
    {
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

fn log_session_status_error(
    session_id: &str,
    endpoint: &str,
    err: &SessionPlaybackError,
    has_cached_status: bool,
) {
    let active_output_id = crate::session_registry::get_session(session_id)
        .and_then(|s| s.active_output_id)
        .unwrap_or_else(|| "<none>".to_string());
    match err {
        SessionPlaybackError::SessionNotFound => {
            tracing::warn!(
                endpoint,
                session_id,
                active_output_id,
                has_cached_status,
                reason = "session_not_found",
                "session status request failed"
            );
        }
        SessionPlaybackError::NoOutputSelected { .. } => {
            tracing::warn!(
                endpoint,
                session_id,
                active_output_id,
                has_cached_status,
                reason = "no_output_selected",
                "session status request failed"
            );
        }
        SessionPlaybackError::OutputLockMissing { output_id, .. } => {
            tracing::warn!(
                endpoint,
                session_id,
                active_output_id,
                output_id,
                has_cached_status,
                reason = "output_lock_missing",
                "session status request failed"
            );
        }
        SessionPlaybackError::OutputInUse {
            output_id,
            held_by_session_id,
            ..
        } => {
            tracing::warn!(
                endpoint,
                session_id,
                active_output_id,
                output_id,
                held_by_session_id,
                has_cached_status,
                reason = "output_in_use",
                "session status request failed"
            );
        }
        SessionPlaybackError::SelectFailed {
            output_id, reason, ..
        } => {
            tracing::warn!(
                endpoint,
                session_id,
                active_output_id,
                output_id,
                status_error = %reason,
                has_cached_status,
                reason = "select_failed",
                "session status request failed"
            );
        }
        SessionPlaybackError::DispatchFailed {
            output_id, reason, ..
        } => {
            tracing::warn!(
                endpoint,
                session_id,
                active_output_id,
                output_id,
                status_error = %reason,
                has_cached_status,
                reason = "dispatch_failed",
                "session status request failed"
            );
        }
        SessionPlaybackError::StatusFailed {
            output_id, reason, ..
        } => {
            tracing::warn!(
                endpoint,
                session_id,
                active_output_id,
                output_id,
                status_error = %reason,
                has_cached_status,
                reason = "status_failed",
                "session status request failed"
            );
        }
        SessionPlaybackError::CommandFailed {
            output_id, reason, ..
        } => {
            tracing::warn!(
                endpoint,
                session_id,
                active_output_id,
                output_id,
                status_error = %reason,
                has_cached_status,
                reason = "command_failed",
                "session status request failed"
            );
        }
    }
}

fn session_should_periodic_refresh(session_id: &str) -> bool {
    crate::session_registry::get_session(session_id)
        .and_then(|s| s.active_output_id)
        .map(|id| id.starts_with("cast:"))
        .unwrap_or(false)
}

fn is_local_session(session_id: &str) -> bool {
    matches!(
        crate::session_registry::get_session(session_id).map(|s| s.mode),
        Some(crate::models::SessionMode::Local)
    )
}

fn canonical_track_path_by_id(state: &web::Data<AppState>, track_id: i64) -> Option<PathBuf> {
    let raw_path = match state.metadata.db.track_path_for_id(track_id) {
        Ok(Some(path)) => path,
        Ok(None) => {
            tracing::warn!(
                track_id,
                reason = "track_id_not_found",
                "queue track path lookup failed"
            );
            return None;
        }
        Err(err) => {
            tracing::warn!(track_id, error = %err, reason = "track_lookup_error", "queue track path lookup failed");
            return None;
        }
    };
    let candidate = PathBuf::from(raw_path);
    match state
        .output
        .controller
        .canonicalize_under_root(state, &candidate)
    {
        Ok(path) => Some(path),
        Err(err) => {
            tracing::warn!(track_id, candidate = %candidate.display(), reason = "path_canonicalize_failed", error = ?err, "queue track path canonicalization failed");
            None
        }
    }
}

fn resolve_queue_add_track_ids(state: &web::Data<AppState>, body: &QueueAddRequest) -> Vec<i64> {
    let mut resolved = Vec::new();
    for track_id in &body.track_ids {
        if canonical_track_path_by_id(state, *track_id).is_some() {
            resolved.push(*track_id);
        } else {
            tracing::warn!(
                track_id,
                reason = "track_path_missing",
                "queue add dropped unknown track id"
            );
        }
    }
    resolved
}

fn build_local_playback_response(
    req: &HttpRequest,
    track_id: i64,
) -> Result<LocalPlaybackPlayResponse, HttpResponse> {
    let conn = req.connection_info();
    let base_url = format!("{}://{}", conn.scheme(), conn.host());
    let url = format!(
        "{}/stream/track/{}",
        base_url.trim_end_matches('/'),
        track_id
    );
    Ok(LocalPlaybackPlayResponse { url, track_id })
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
pub async fn sessions_queue_list(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
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
/// Add tracks to a session queue.
pub async fn sessions_queue_add(
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<QueueAddRequest>,
) -> impl Responder {
    let session_id = id.into_inner();
    if let Err(resp) = require_session(&session_id) {
        return resp;
    }
    let resolved = resolve_queue_add_track_ids(&state, &body);
    let added = match crate::session_registry::queue_add_track_ids(&session_id, resolved) {
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
/// Insert tracks at the front of a session queue.
pub async fn sessions_queue_add_next(
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<QueueAddRequest>,
) -> impl Responder {
    let session_id = id.into_inner();
    if let Err(resp) = require_session(&session_id) {
        return resp;
    }
    let resolved = resolve_queue_add_track_ids(&state, &body);
    let added = match crate::session_registry::queue_add_next_track_ids(&session_id, resolved) {
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
    let track_id = match canonical_track_path_by_id(&state, body.track_id) {
        Some(_) => body.track_id,
        None => return HttpResponse::NotFound().body("track not found"),
    };
    match crate::session_registry::queue_remove_track_id(&session_id, track_id) {
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

    let path = match state.metadata.db.track_path_for_id(body.track_id) {
        Ok(Some(path)) => path,
        Ok(None) => {
            tracing::warn!(session_id = %session_id, track_id = body.track_id, reason = "track_id_not_found", "queue play_from failed");
            return HttpResponse::NotFound().finish();
        }
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };

    let canonical = {
        let candidate = PathBuf::from(&path);
        match state
            .output
            .controller
            .canonicalize_under_root(&state, &candidate)
        {
            Ok(path) => path,
            Err(err) => return err.into_response(),
        }
    };

    let found = match crate::session_registry::queue_play_from(&session_id, body.track_id) {
        Ok(found) => found,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    };
    if !found {
        tracing::warn!(session_id = %session_id, track_id = body.track_id, reason = "track_not_in_queue", "queue play_from failed");
        return HttpResponse::NotFound().finish();
    }
    state.events.queue_changed();
    state.events.status_changed();

    if is_local_session(&session_id) {
        let payload = match build_local_playback_response(&req, body.track_id) {
            Ok(payload) => payload,
            Err(resp) => return resp,
        };
        return HttpResponse::Ok().json(payload);
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
    let Some(next_track_id) = (match crate::session_registry::queue_next_track_id(&session_id) {
        Ok(track_id) => track_id,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    }) else {
        return HttpResponse::NoContent().finish();
    };
    let Some(next_path) = canonical_track_path_by_id(&state, next_track_id) else {
        tracing::warn!(session_id = %session_id, track_id = next_track_id, reason = "next_track_path_missing", "queue next failed");
        return HttpResponse::NotFound().body("track not found");
    };
    state.events.queue_changed();
    state.events.status_changed();
    if is_local_session(&session_id) {
        let payload = match build_local_playback_response(&req, next_track_id) {
            Ok(payload) => payload,
            Err(resp) => return resp,
        };
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
    let Some(prev_track_id) = (match crate::session_registry::queue_previous_track_id(&session_id) {
        Ok(track_id) => track_id,
        Err(()) => return HttpResponse::NotFound().body("session not found"),
    }) else {
        return HttpResponse::NoContent().finish();
    };
    let Some(prev_path) = canonical_track_path_by_id(&state, prev_track_id) else {
        tracing::warn!(session_id = %session_id, track_id = prev_track_id, reason = "previous_track_path_missing", "queue previous failed");
        return HttpResponse::NotFound().body("track not found");
    };
    state.events.queue_changed();
    state.events.status_changed();
    if is_local_session(&session_id) {
        let payload = match build_local_playback_response(&req, prev_track_id) {
            Ok(payload) => payload,
            Err(resp) => return resp,
        };
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
        .map(|track_id| build_queue_item(state, *track_id, false, false))
        .collect();

    if let Some(current_track_id) = snapshot.now_playing {
        let index = items.iter().position(|item| match item {
            crate::models::QueueItem::Track { id, .. } => current_track_id == *id,
            crate::models::QueueItem::Missing { id } => *id == Some(current_track_id),
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
            items.insert(0, build_queue_item(state, current_track_id, true, false));
        }
    }

    let mut played_track_ids = Vec::new();
    for track_id in snapshot.history.iter().rev() {
        if snapshot.now_playing == Some(*track_id) {
            continue;
        }
        played_track_ids.push(*track_id);
        if played_track_ids.len() >= 10 {
            break;
        }
    }

    if !played_track_ids.is_empty() {
        played_track_ids.reverse();
        let mut seen = HashSet::new();
        for item in &items {
            match item {
                crate::models::QueueItem::Track { id, .. } => {
                    seen.insert(Some(*id));
                }
                crate::models::QueueItem::Missing { id } => {
                    seen.insert(*id);
                }
            }
        }

        let mut played_items = Vec::new();
        for track_id in played_track_ids {
            if seen.contains(&Some(track_id)) {
                continue;
            }
            played_items.push(build_queue_item(state, track_id, false, true));
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
    track_id: i64,
    now_playing: bool,
    played: bool,
) -> crate::models::QueueItem {
    let record = match state
        .metadata
        .db
        .track_record_by_id(track_id)
        .ok()
        .flatten()
    {
        Some(record) => record,
        None => return crate::models::QueueItem::Missing { id: Some(track_id) },
    };
    crate::models::QueueItem::Track {
        id: track_id,
        file_name: record.file_name,
        title: record.title,
        duration_ms: record.duration_ms,
        sample_rate: record.sample_rate,
        album: record.album,
        artist: record.artist,
        format: record.format.unwrap_or_else(|| "unknown".to_string()),
        now_playing,
        played,
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
        BridgeProviderState, BridgeState, CastProviderState, DeviceSelectionState,
        LocalProviderState, MetadataWake, PlayerStatus, QueueState,
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
            player: Arc::new(Mutex::new(crate::bridge::BridgePlayer {
                cmd_tx: local_cmd_tx,
            })),
            running: Arc::new(AtomicBool::new(false)),
        });

        let status = Arc::new(Mutex::new(PlayerStatus::default()));
        let events = EventBus::new();
        let status_store = crate::status_store::StatusStore::new(status, events.clone());
        let queue = Arc::new(Mutex::new(QueueState::default()));
        let queue_service =
            crate::queue_service::QueueService::new(queue, status_store.clone(), events.clone());
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
        let _guard = crate::session_registry::test_lock();
        crate::session_registry::reset_for_tests();
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
            let library_root = state
                .library
                .read()
                .expect("library lock")
                .root()
                .to_path_buf();
            let track_path = library_root.join("test-track.flac");
            std::fs::write(&track_path, b"fixture").expect("write track fixture");
            let fs_meta = std::fs::metadata(&track_path).expect("track fixture metadata");
            let mtime_ms = fs_meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            state
                .metadata
                .db
                .upsert_track(&crate::metadata_db::TrackRecord {
                    path: track_path.to_string_lossy().to_string(),
                    file_name: "test-track.flac".to_string(),
                    title: Some("Test Track".to_string()),
                    artist: Some("Test Artist".to_string()),
                    album_artist: Some("Test Artist".to_string()),
                    album: Some("Test Album".to_string()),
                    album_uuid: None,
                    track_number: Some(1),
                    disc_number: Some(1),
                    year: Some(2024),
                    duration_ms: Some(180_000),
                    sample_rate: Some(44_100),
                    bit_depth: Some(16),
                    format: Some("flac".to_string()),
                    mtime_ms,
                    size_bytes: fs_meta.len() as i64,
                })
                .expect("upsert fixture track");
        }
        let track_id = state
            .metadata
            .db
            .track_id_for_path(
                &state
                    .library
                    .read()
                    .expect("library lock")
                    .root()
                    .join("test-track.flac")
                    .to_string_lossy(),
            )
            .expect("lookup track id")
            .expect("track id");
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
                    now_playing: Some(format!("http://localhost/stream/track/{track_id}")),
                    paused: false,
                    elapsed_ms: Some(12_345),
                    duration_ms: Some(180_000),
                    ..Default::default()
                },
            );
        }

        let app = actix_web::test::init_service(
            actix_web::App::new()
                .app_data(state.clone())
                .service(crate::api::sessions_select_output),
        )
        .await;
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
        crate::session_registry::queue_add_track_ids(&session_id, vec![track_id])
            .expect("queue add");
        let found = crate::session_registry::queue_play_from(&session_id, track_id)
            .expect("queue play_from");
        assert!(found, "queued track should be found");

        let req = actix_web::test::TestRequest::post()
            .uri(&format!(
                "/sessions/{}/select-output",
                urlencoding::encode(&session_id)
            ))
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
