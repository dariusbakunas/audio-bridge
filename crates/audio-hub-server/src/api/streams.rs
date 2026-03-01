//! Server-sent event streams.

use std::collections::VecDeque;
use std::time::Instant;

use actix_web::http::header;
use actix_web::web::Bytes;
use actix_web::{Error, HttpResponse, Responder, get, web};
use futures_util::{Stream, stream::unfold};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{Duration, Interval, MissedTickBehavior};

use crate::events::{HubEvent, LogEvent};
use crate::state::AppState;

use super::outputs::normalize_outputs_response;

const PING_INTERVAL: Duration = Duration::from_secs(15);

/// SSE loop state for outputs stream.
struct OutputsStreamState {
    state: web::Data<AppState>,
    receiver: broadcast::Receiver<HubEvent>,
    interval: Interval,
    pending: VecDeque<Bytes>,
    last_outputs: Option<String>,
    last_ping: Instant,
}

/// SSE loop state for metadata stream.
struct MetadataStreamState {
    receiver: broadcast::Receiver<HubEvent>,
    pending: VecDeque<Bytes>,
    last_ping: Instant,
}

/// SSE loop state for logs stream.
struct LogsStreamState {
    receiver: broadcast::Receiver<LogEvent>,
    pending: VecDeque<Bytes>,
    last_ping: Instant,
}

/// SSE loop state for albums stream.
struct AlbumsStreamState {
    receiver: broadcast::Receiver<HubEvent>,
    pending: VecDeque<Bytes>,
    last_ping: Instant,
}

/// Internal signal emitted by stream poll loop.
enum StreamSignal<E> {
    Tick,
    Event(Result<E, RecvError>),
}

/// Encode one SSE event frame.
fn sse_event(event: &str, data: &str) -> Bytes {
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

/// Emit periodic SSE ping comments to keep idle connections alive.
fn push_ping_if_needed(pending: &mut VecDeque<Bytes>, last_ping: &mut Instant) {
    if pending.is_empty() && last_ping.elapsed() >= PING_INTERVAL {
        *last_ping = Instant::now();
        pending.push_back(Bytes::from(": ping\n\n"));
    }
}

/// Wait for either broadcast event or interval tick.
async fn recv_signal<E: Clone>(
    receiver: &mut broadcast::Receiver<E>,
    interval: Option<&mut Interval>,
) -> StreamSignal<E> {
    match interval {
        Some(interval) => {
            tokio::select! {
                _ = interval.tick() => StreamSignal::Tick,
                result = receiver.recv() => StreamSignal::Event(result),
            }
        }
        None => StreamSignal::Event(receiver.recv().await),
    }
}

/// Build HTTP response configured for SSE streaming.
fn sse_response<S>(stream: S) -> HttpResponse
where
    S: Stream<Item = Result<Bytes, Error>> + 'static,
{
    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/event-stream"))
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::CONNECTION, "keep-alive"))
        .streaming(stream)
}

#[utoipa::path(
    get,
    path = "/outputs/stream",
    responses(
        (status = 200, description = "Outputs event stream")
    )
)]
#[get("/outputs/stream")]
/// Stream output updates via server-sent events.
pub async fn outputs_stream(state: web::Data<AppState>) -> impl Responder {
    let initial = normalize_outputs_response(state.output.controller.list_outputs(&state).await);
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "null".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(sse_event("outputs", &initial_json));

    let mut interval = tokio::time::interval(Duration::from_millis(2000));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let receiver = state.events.subscribe();

    let stream = unfold(
        OutputsStreamState {
            state: state.clone(),
            receiver,
            interval,
            pending,
            last_outputs: Some(initial_json),
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }

                let mut refresh = false;
                let mut emit_unchanged = false;
                match recv_signal(&mut ctx.receiver, Some(&mut ctx.interval)).await {
                    StreamSignal::Tick => {}
                    StreamSignal::Event(result) => match result {
                        Ok(HubEvent::OutputsChanged) => {
                            refresh = true;
                            emit_unchanged = true;
                        }
                        Ok(HubEvent::StatusChanged) => {}
                        Ok(HubEvent::QueueChanged) => {}
                        Ok(HubEvent::Metadata(_)) => {}
                        Ok(HubEvent::LibraryChanged) => {}
                        Err(RecvError::Lagged(_)) => {
                            refresh = true;
                            emit_unchanged = true;
                        }
                        Err(RecvError::Closed) => return None,
                    },
                }

                if refresh {
                    let outputs = normalize_outputs_response(
                        ctx.state.output.controller.list_outputs(&ctx.state).await,
                    );
                    let json =
                        serde_json::to_string(&outputs).unwrap_or_else(|_| "null".to_string());
                    if emit_unchanged || ctx.last_outputs.as_deref() != Some(json.as_str()) {
                        ctx.last_outputs = Some(json.clone());
                        ctx.pending.push_back(sse_event("outputs", &json));
                    }
                }

                push_ping_if_needed(&mut ctx.pending, &mut ctx.last_ping);
            }
        },
    );

    sse_response(stream)
}

#[utoipa::path(
    get,
    path = "/metadata/stream",
    responses(
        (status = 200, description = "Metadata event stream")
    )
)]
#[get("/metadata/stream")]
/// Stream metadata job updates via server-sent events.
pub async fn metadata_stream(state: web::Data<AppState>) -> impl Responder {
    let receiver = state.events.subscribe();
    let pending = VecDeque::new();

    let stream = unfold(
        MetadataStreamState {
            receiver,
            pending,
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }

                match recv_signal(&mut ctx.receiver, None).await {
                    StreamSignal::Tick => {}
                    StreamSignal::Event(result) => match result {
                        Ok(HubEvent::Metadata(event)) => {
                            let json = serde_json::to_string(&event)
                                .unwrap_or_else(|_| "null".to_string());
                            ctx.pending.push_back(sse_event("metadata", &json));
                        }
                        Ok(_) => {}
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => return None,
                    },
                }

                push_ping_if_needed(&mut ctx.pending, &mut ctx.last_ping);
            }
        },
    );

    sse_response(stream)
}

#[utoipa::path(
    get,
    path = "/albums/stream",
    responses(
        (status = 200, description = "Album change event stream")
    )
)]
#[get("/albums/stream")]
/// Stream album change notifications via server-sent events.
pub async fn albums_stream(state: web::Data<AppState>) -> impl Responder {
    let receiver = state.events.subscribe();
    let pending = VecDeque::new();

    let stream = unfold(
        AlbumsStreamState {
            receiver,
            pending,
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }

                match recv_signal(&mut ctx.receiver, None).await {
                    StreamSignal::Tick => {}
                    StreamSignal::Event(result) => match result {
                        Ok(HubEvent::LibraryChanged) => {
                            ctx.pending.push_back(sse_event("albums", "{}"));
                        }
                        Ok(_) => {}
                        Err(RecvError::Lagged(_)) => {
                            ctx.pending.push_back(sse_event("albums", "{}"));
                        }
                        Err(RecvError::Closed) => return None,
                    },
                }

                push_ping_if_needed(&mut ctx.pending, &mut ctx.last_ping);
            }
        },
    );

    sse_response(stream)
}

#[utoipa::path(
    get,
    path = "/logs/stream",
    responses(
        (status = 200, description = "Server log event stream")
    )
)]
#[get("/logs/stream")]
/// Stream server logs via server-sent events.
pub async fn logs_stream(state: web::Data<AppState>) -> impl Responder {
    let initial = state.log_bus.snapshot();
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "[]".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(sse_event("logs", &initial_json));

    let receiver = state.log_bus.subscribe();
    let stream = unfold(
        LogsStreamState {
            receiver,
            pending,
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }

                match recv_signal(&mut ctx.receiver, None).await {
                    StreamSignal::Tick => {}
                    StreamSignal::Event(result) => match result {
                        Ok(event) => {
                            let json = serde_json::to_string(&event)
                                .unwrap_or_else(|_| "null".to_string());
                            ctx.pending.push_back(sse_event("log", &json));
                        }
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => return None,
                    },
                }

                push_ping_if_needed(&mut ctx.pending, &mut ctx.last_ping);
            }
        },
    );

    sse_response(stream)
}
