//! In-process event bus for server-side updates.
//!
//! Provides a lightweight broadcast channel for UI subscriptions.

use tokio::sync::broadcast;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// Metadata/background-work events exposed to API/SSE clients.
pub enum MetadataEvent {
    LibraryScanAlbumStart {
        album: String,
    },
    LibraryScanAlbumFinish {
        album: String,
        tracks: usize,
    },
    MusicBrainzBatch {
        count: usize,
    },
    MusicBrainzLookupStart {
        track_id: Option<i64>,
        title: String,
        artist: String,
        album: Option<String>,
    },
    MusicBrainzLookupSuccess {
        track_id: Option<i64>,
        recording_mbid: Option<String>,
        artist_mbid: Option<String>,
        album_mbid: Option<String>,
    },
    MusicBrainzLookupNoMatch {
        track_id: Option<i64>,
        title: String,
        artist: String,
        album: Option<String>,
        query: String,
        top_score: Option<i32>,
        best_recording_id: Option<String>,
        best_recording_title: Option<String>,
    },
    MusicBrainzLookupFailure {
        track_id: Option<i64>,
        error: String,
    },
    CoverArtBatch {
        count: usize,
    },
    CoverArtFetchStart {
        album_id: i64,
        mbid: String,
    },
    CoverArtFetchSuccess {
        album_id: i64,
    },
    CoverArtFetchFailure {
        album_id: i64,
        mbid: String,
        error: String,
        attempts: i64,
    },
    AlbumNormalization {
        track_id: Option<i64>,
        original_album: String,
        normalized_album: String,
        disc_number: Option<u32>,
        source: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
/// Buffered server log event payload.
pub struct LogEvent {
    /// Log level string.
    pub level: String,
    /// Tracing target/module.
    pub target: String,
    /// Formatted message + selected fields.
    pub message: String,
    /// Event timestamp (unix millis).
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
/// Broadcast bus for high-level server events.
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

/// In-memory rolling log bus plus broadcast fanout for UI log streaming.
pub struct LogBus {
    sender: broadcast::Sender<LogEvent>,
    buffer: Arc<Mutex<VecDeque<LogEvent>>>,
    capacity: usize,
}

impl LogBus {
    /// Create log bus with fixed in-memory ring capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(16));
        Self {
            sender,
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    /// Subscribe to live log stream.
    pub fn subscribe(&self) -> broadcast::Receiver<LogEvent> {
        self.sender.subscribe()
    }

    /// Snapshot buffered log history.
    pub fn snapshot(&self) -> Vec<LogEvent> {
        self.buffer
            .lock()
            .map(|buf| buf.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Publish one log event to buffer and subscribers.
    pub fn publish(&self, event: LogEvent) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.push_back(event.clone());
            while buffer.len() > self.capacity {
                buffer.pop_front();
            }
        }
        let _ = self.sender.send(event);
    }

    /// Clear in-memory buffered log history.
    pub fn clear(&self) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.clear();
        }
    }
}

/// Tracing layer that forwards events into [`LogBus`].
pub struct LogLayer {
    log_bus: Arc<LogBus>,
}

impl LogLayer {
    /// Create tracing layer backed by a shared log bus.
    pub fn new(log_bus: Arc<LogBus>) -> Self {
        Self { log_bus }
    }
}

impl<S> Layer<S> for LogLayer
where
    S: Subscriber,
{
    /// Convert tracing event into [`LogEvent`] and publish it.
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
/// Visitor collecting primary message and key/value fields from tracing events.
struct LogVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl Visit for LogVisitor {
    /// Record string fields from tracing events.
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }

    /// Record debug-formatted fields from tracing events.
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let formatted = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(formatted.trim_matches('"').to_string());
        } else {
            self.fields.push(format!("{}={}", field.name(), formatted));
        }
    }
}
