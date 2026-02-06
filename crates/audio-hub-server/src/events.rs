//! In-process event bus for server-side updates.
//!
//! Provides a lightweight broadcast channel for UI subscriptions.

use tokio::sync::broadcast;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MetadataEvent {
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

/// Server event payloads published by core services.
#[derive(Debug, Clone)]
pub enum HubEvent {
    QueueChanged,
    StatusChanged,
    OutputsChanged,
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

    /// Notify subscribers about metadata/background jobs.
    pub fn metadata_event(&self, event: MetadataEvent) {
        let _ = self.sender.send(HubEvent::Metadata(event));
    }
}
