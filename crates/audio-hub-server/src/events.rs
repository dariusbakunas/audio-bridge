//! In-process event bus for server-side updates.
//!
//! Provides a lightweight broadcast channel for UI subscriptions.

use tokio::sync::broadcast;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MetadataEvent {
    LibraryScanAlbumStart { path: String },
    LibraryScanAlbumFinish { path: String, tracks: usize },
    MusicBrainzBatch { count: usize },
    MusicBrainzLookupStart {
        path: String,
        title: String,
        artist: String,
        album: Option<String>,
    },
    MusicBrainzLookupSuccess {
        path: String,
        recording_mbid: Option<String>,
        artist_mbid: Option<String>,
        album_mbid: Option<String>,
    },
    MusicBrainzLookupNoMatch {
        path: String,
        title: String,
        artist: String,
        album: Option<String>,
        query: String,
        top_score: Option<i32>,
        best_recording_id: Option<String>,
        best_recording_title: Option<String>,
    },
    MusicBrainzLookupFailure { path: String, error: String },
    CoverArtBatch { count: usize },
    CoverArtFetchStart { album_id: i64, mbid: String },
    CoverArtFetchSuccess { album_id: i64, cover_path: String },
    CoverArtFetchFailure {
        album_id: i64,
        mbid: String,
        error: String,
        attempts: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LogEvent {
    pub level: String,
    pub target: String,
    pub message: String,
    pub timestamp_ms: i64,
}

/// Server event payloads published by core services.
#[derive(Debug, Clone)]
pub enum HubEvent {
    QueueChanged,
    StatusChanged,
    OutputsChanged,
    LibraryChanged,
    Metadata(MetadataEvent),
}

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<HubEvent>,
}

impl EventBus {
    /// Create a new event bus with a bounded broadcast channel.
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(64);
        Self { sender }
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.sender.subscribe()
    }

    /// Notify subscribers that the queue has changed.
    pub fn queue_changed(&self) {
        let _ = self.sender.send(HubEvent::QueueChanged);
    }

    /// Notify subscribers that playback status has changed.
    pub fn status_changed(&self) {
        let _ = self.sender.send(HubEvent::StatusChanged);
    }

    /// Notify subscribers that outputs or selection have changed.
    pub fn outputs_changed(&self) {
        let _ = self.sender.send(HubEvent::OutputsChanged);
    }

    /// Notify subscribers that the library index changed.
    pub fn library_changed(&self) {
        let _ = self.sender.send(HubEvent::LibraryChanged);
    }

    /// Notify subscribers about metadata/background jobs.
    pub fn metadata_event(&self, event: MetadataEvent) {
        let _ = self.sender.send(HubEvent::Metadata(event));
    }
}

pub struct LogBus {
    sender: broadcast::Sender<LogEvent>,
    buffer: Arc<Mutex<VecDeque<LogEvent>>>,
    capacity: usize,
}

impl LogBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(16));
        Self {
            sender,
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogEvent> {
        self.sender.subscribe()
    }

    pub fn snapshot(&self) -> Vec<LogEvent> {
        self.buffer
            .lock()
            .map(|buf| buf.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn publish(&self, event: LogEvent) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.push_back(event.clone());
            while buffer.len() > self.capacity {
                buffer.pop_front();
            }
        }
        let _ = self.sender.send(event);
    }

    pub fn clear(&self) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.clear();
        }
    }
}

pub struct LogLayer {
    log_bus: Arc<LogBus>,
}

impl LogLayer {
    pub fn new(log_bus: Arc<LogBus>) -> Self {
        Self { log_bus }
    }
}

impl<S> Layer<S> for LogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = LogVisitor::default();
        event.record(&mut visitor);
        let mut message = visitor.message.unwrap_or_else(|| "log event".to_string());
        if !visitor.fields.is_empty() {
            message = format!("{message} {}", visitor.fields.join(" "));
        }
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let log_event = LogEvent {
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            message,
            timestamp_ms,
        };
        self.log_bus.publish(log_event);
    }
}

#[derive(Default)]
struct LogVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl Visit for LogVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let formatted = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(formatted.trim_matches('"').to_string());
        } else {
            self.fields.push(format!("{}={}", field.name(), formatted));
        }
    }
}
