//! Server-sent event streams.

use std::collections::VecDeque;
use std::time::Instant;

use actix_web::{get, web, Error, HttpResponse, Responder};
use actix_web::http::header;
use actix_web::web::Bytes;
use futures_util::{Stream, stream::unfold};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{Duration, Interval, MissedTickBehavior};

use crate::events::{HubEvent, LogEvent};
use crate::state::AppState;

use super::outputs::normalize_outputs_response;

const PING_INTERVAL: Duration = Duration::from_secs(15);

struct StatusStreamState {
    state: web::Data<AppState>,
    output_id: String,
    receiver: broadcast::Receiver<HubEvent>,
    interval: Interval,
    pending: VecDeque<Bytes>,
    last_status: Option<String>,
    last_ping: Instant,
}

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

fn push_ping_if_needed(pending: &mut VecDeque<Bytes>, last_ping: &mut Instant) {
    if pending.is_empty() && last_ping.elapsed() >= PING_INTERVAL {
        *last_ping = Instant::now();
        pending.push_back(Bytes::from(": ping\n\n"));
    }
}

enum StreamSignal<E> {
    Tick,
    Event(Result<E, RecvError>),
}

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

struct QueueStreamState {
    state: web::Data<AppState>,
    receiver: broadcast::Receiver<HubEvent>,
    interval: Interval,
    pending: VecDeque<Bytes>,
    last_queue: Option<String>,
    last_ping: Instant,
}

struct OutputsStreamState {
    state: web::Data<AppState>,
    receiver: broadcast::Receiver<HubEvent>,
    interval: Interval,
    pending: VecDeque<Bytes>,
    last_outputs: Option<String>,
    last_ping: Instant,
}

struct MetadataStreamState {
    receiver: broadcast::Receiver<HubEvent>,
    pending: VecDeque<Bytes>,
    last_ping: Instant,
}

struct LogsStreamState {
    receiver: broadcast::Receiver<LogEvent>,
    pending: VecDeque<Bytes>,
    last_ping: Instant,
}

struct AlbumsStreamState {
    receiver: broadcast::Receiver<HubEvent>,
    pending: VecDeque<Bytes>,
    last_ping: Instant,
}

#[utoipa::path(
    get,
    path = "/outputs/{id}/status/stream",
    params(
        ("id" = String, Path, description = "Output id")
    ),
    responses(
        (status = 200, description = "Status event stream")
    )
)]
#[get("/outputs/{id}/status/stream")]
/// Stream status updates via server-sent events.
pub async fn status_stream(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let output_id = id.into_inner();
    let initial = match state.output.controller.status_for_output(&state, &output_id).await {
        Ok(resp) => resp,
        Err(err) => return err.into_response(),
    };
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "null".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(sse_event("status", &initial_json));

    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let receiver = state.events.subscribe();

    let stream = unfold(
        StatusStreamState {
            state: state.clone(),
            output_id,
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
                match recv_signal(&mut ctx.receiver, Some(&mut ctx.interval)).await {
                    StreamSignal::Tick => {}
                    StreamSignal::Event(result) => match result {
                        Ok(HubEvent::StatusChanged) => refresh = true,
                        Ok(HubEvent::QueueChanged) => {}
                        Ok(HubEvent::OutputsChanged) => {}
                        Ok(HubEvent::Metadata(_)) => {}
                        Ok(HubEvent::LibraryChanged) => {}
                        Err(RecvError::Lagged(_)) => refresh = true,
                        Err(RecvError::Closed) => return None,
                    },
                }

                if refresh {
                    if let Ok(status) = ctx
                        .state
                        .output.controller
                        .status_for_output(&ctx.state, &ctx.output_id)
                        .await
                    {
                        let json = serde_json::to_string(&status)
                            .unwrap_or_else(|_| "null".to_string());
                        if ctx.last_status.as_deref() != Some(json.as_str()) {
                            ctx.last_status = Some(json.clone());
                            ctx.pending.push_back(sse_event("status", &json));
                        }
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
    path = "/queue/stream",
    responses(
        (status = 200, description = "Queue event stream")
    )
)]
#[get("/queue/stream")]
/// Stream queue updates via server-sent events.
pub async fn queue_stream(state: web::Data<AppState>) -> impl Responder {
    let initial = state.output.controller.queue_list(&state);
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "null".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(sse_event("queue", &initial_json));

    let mut interval = tokio::time::interval(Duration::from_secs(15));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let receiver = state.events.subscribe();

    let stream = unfold(
        QueueStreamState {
            state: state.clone(),
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

                match recv_signal(&mut ctx.receiver, Some(&mut ctx.interval)).await {
                    StreamSignal::Tick => {}
                    StreamSignal::Event(result) => match result {
                        Ok(HubEvent::QueueChanged) => {
                            let queue = ctx.state.output.controller.queue_list(&ctx.state);
                            let json = serde_json::to_string(&queue)
                                .unwrap_or_else(|_| "null".to_string());
                            if ctx.last_queue.as_deref() != Some(json.as_str()) {
                                ctx.last_queue = Some(json.clone());
                                ctx.pending.push_back(sse_event("queue", &json));
                            }
                        }
                        Ok(HubEvent::StatusChanged) => {
                            let queue = ctx.state.output.controller.queue_list(&ctx.state);
                            let json = serde_json::to_string(&queue)
                                .unwrap_or_else(|_| "null".to_string());
                            if ctx.last_queue.as_deref() != Some(json.as_str()) {
                                ctx.last_queue = Some(json.clone());
                                ctx.pending.push_back(sse_event("queue", &json));
                            }
                        }
                        Ok(HubEvent::OutputsChanged) => {}
                        Ok(HubEvent::Metadata(_)) => {}
                        Ok(HubEvent::LibraryChanged) => {}
                        Err(RecvError::Lagged(_)) => {
                            let queue = ctx.state.output.controller.queue_list(&ctx.state);
                            let json = serde_json::to_string(&queue)
                                .unwrap_or_else(|_| "null".to_string());
                            ctx.last_queue = Some(json.clone());
                            ctx.pending.push_back(sse_event("queue", &json));
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
                match recv_signal(&mut ctx.receiver, Some(&mut ctx.interval)).await {
                    StreamSignal::Tick => {}
                    StreamSignal::Event(result) => match result {
                        Ok(HubEvent::OutputsChanged) => refresh = true,
                        Ok(HubEvent::StatusChanged) => {}
                        Ok(HubEvent::QueueChanged) => {}
                        Ok(HubEvent::Metadata(_)) => {}
                        Ok(HubEvent::LibraryChanged) => {}
                        Err(RecvError::Lagged(_)) => refresh = true,
                        Err(RecvError::Closed) => return None,
                    },
                }

                if refresh {
                    let outputs = normalize_outputs_response(
                        ctx.state.output.controller.list_outputs(&ctx.state).await,
                    );
                    let json = serde_json::to_string(&outputs)
                        .unwrap_or_else(|_| "null".to_string());
                    if ctx.last_outputs.as_deref() != Some(json.as_str()) {
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
