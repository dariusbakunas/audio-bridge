//! API models and OpenAPI schemas.
//!
//! Defines request/response structures for the hub server API.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use audio_bridge_types::PlaybackStatus;

/// A library entry returned by directory listings.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LibraryEntry {
    /// Directory entry with a path and display name.
    Dir {
        /// Absolute path for this directory.
        path: String,
        /// Display name derived from the directory name.
        name: String,
    },
    /// Track entry with metadata.
    Track {
        /// Absolute path to the media file.
        path: String,
        /// Filename for display.
        file_name: String,
        /// Extension hint used by the player.
        ext_hint: String,
        /// Track duration in milliseconds.
        duration_ms: Option<u64>,
        /// Sample rate reported by the decoder.
        sample_rate: Option<u32>,
        /// Album name if available.
        album: Option<String>,
        /// Artist name if available.
        artist: Option<String>,
        /// Format label for the UI.
        format: String,
    },
}

/// Directory listing response from the library endpoint.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LibraryResponse {
    /// Absolute path of the requested directory.
    pub dir: String,
    /// Entries within the directory.
    pub entries: Vec<LibraryEntry>,
}

/// Playback request payload for the `/play` endpoint.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct PlayRequest {
    /// Absolute path to the track within the library root.
    pub path: String,
    /// Queue handling mode for the new track.
    #[serde(default)]
    pub queue_mode: Option<QueueMode>,
    /// Optional output id to target.
    #[serde(default)]
    pub output_id: Option<String>,
}

/// Defines how a play request interacts with the existing queue.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueueMode {
    /// Keep the existing queue unchanged.
    Keep,
    /// Replace the queue with only the new track.
    Replace,
    /// Append the new track to the queue.
    Append,
}

/// Playback status response payload.
pub type StatusResponse = PlaybackStatus;

/// A single queued item entry.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueItem {
    /// Queued track with metadata.
    Track {
        /// Absolute path to the track.
        path: String,
        /// Filename for display.
        file_name: String,
        /// Duration in milliseconds.
        duration_ms: Option<u64>,
        /// Sample rate reported for the track.
        sample_rate: Option<u32>,
        /// Album name if available.
        album: Option<String>,
        /// Artist name if available.
        artist: Option<String>,
        /// Format label for the UI.
        format: String,
    },
    /// Queue entry that no longer exists on disk.
    Missing { path: String },
}

/// Response payload for the queue listing.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct QueueResponse {
    /// Ordered queue items.
    pub items: Vec<QueueItem>,
}

/// Payload to add items to the queue.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct QueueAddRequest {
    /// Paths to enqueue.
    pub paths: Vec<String>,
}

/// Payload to remove a single item from the queue.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct QueueRemoveRequest {
    /// Path of the item to remove.
    pub path: String,
}

/// Response for listing outputs.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OutputsResponse {
    /// Active output id, if any.
    pub active_id: Option<String>,
    /// Available outputs.
    pub outputs: Vec<OutputInfo>,
}

/// Output device information returned by providers.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OutputInfo {
    /// Unique output id.
    pub id: String,
    /// Output kind (bridge/local/etc).
    pub kind: String,
    /// Display name.
    pub name: String,
    /// Reported state (online/offline).
    pub state: String,
    /// Optional provider id.
    pub provider_id: Option<String>,
    /// Optional provider name.
    pub provider_name: Option<String>,
    /// Supported sample rates if known.
    pub supported_rates: Option<SupportedRates>,
    /// Capabilities advertised by the output.
    pub capabilities: OutputCapabilities,
}

/// Minimum/maximum sample rate range for a device.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SupportedRates {
    /// Minimum supported sample rate (Hz).
    pub min_hz: u32,
    /// Maximum supported sample rate (Hz).
    pub max_hz: u32,
}

/// Capabilities reported by an output.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OutputCapabilities {
    /// Whether the output supports selecting a device.
    pub device_select: bool,
    /// Whether the output supports volume control.
    pub volume: bool,
}

/// Request to select the active output.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OutputSelectRequest {
    /// Output id to activate.
    pub id: String,
}

/// Provider summary for output listings.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ProviderInfo {
    /// Provider id.
    pub id: String,
    /// Provider kind (bridge/local/etc).
    pub kind: String,
    /// Display name.
    pub name: String,
    /// Provider state.
    pub state: String,
    /// Provider-level capabilities.
    pub capabilities: OutputCapabilities,
}

/// Response payload for provider listings.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ProvidersResponse {
    /// Available output providers.
    pub providers: Vec<ProviderInfo>,
}
