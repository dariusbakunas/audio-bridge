//! API models and OpenAPI schemas.
//!
//! Defines request/response structures for the hub server API.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use audio_bridge_types::PlaybackStatus;
use crate::metadata_db::{AlbumSummary, ArtistSummary, TrackSummary};

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

/// Playback request payload for the `/play/album` endpoint.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct PlayAlbumRequest {
    /// Album id to play.
    pub album_id: i64,
    /// Queue handling mode for the album tracks.
    #[serde(default)]
    pub queue_mode: Option<AlbumQueueMode>,
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

/// Defines how album playback interacts with the existing queue.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlbumQueueMode {
    /// Replace the current queue with the album tracks.
    Replace,
    /// Append the album tracks to the current queue.
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
        /// Track id if known.
        id: Option<i64>,
        /// Absolute path to the track.
        path: String,
        /// Filename for display.
        file_name: String,
        /// Track title if available.
        #[serde(default)]
        title: Option<String>,
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
        /// True when this is the currently playing track.
        #[serde(default)]
        now_playing: bool,
        /// True when this track has already played.
        #[serde(default)]
        played: bool,
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

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MusicBrainzMatchKind {
    Track,
    Album,
}

/// Payload to search MusicBrainz for a manual match.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct MusicBrainzMatchSearchRequest {
    /// Track title (for track search) or album title (for album search).
    pub title: String,
    /// Artist name used in the query.
    pub artist: String,
    /// Optional album name to refine track searches.
    #[serde(default)]
    pub album: Option<String>,
    /// Search kind (track or album).
    pub kind: MusicBrainzMatchKind,
    /// Optional max number of results.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Single MusicBrainz candidate returned from search.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct MusicBrainzMatchCandidate {
    pub recording_mbid: Option<String>,
    pub release_mbid: Option<String>,
    pub artist_mbid: Option<String>,
    pub title: String,
    pub artist: String,
    pub release_title: Option<String>,
    pub score: Option<i32>,
    pub year: Option<i32>,
}

/// Response payload for MusicBrainz search results.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct MusicBrainzMatchSearchResponse {
    pub items: Vec<MusicBrainzMatchCandidate>,
}

/// Response for resolving a track path to album metadata.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct TrackResolveResponse {
    pub album_id: Option<i64>,
}

/// Current metadata fields for a track path.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct TrackMetadataResponse {
    /// Absolute track path.
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
}

/// Update request for writing tag metadata to a track file.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct TrackMetadataUpdateRequest {
    /// Optional track id from the metadata DB.
    #[serde(default)]
    pub track_id: Option<i64>,
    /// Absolute track path.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub album_artist: Option<String>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub disc_number: Option<u32>,
}

/// Supported metadata fields for a track file.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct TrackMetadataFieldsResponse {
    /// Tag type detected for the file, if known.
    pub tag_type: Option<String>,
    /// Supported field keys for editing.
    pub fields: Vec<String>,
}

/// Request payload for on-demand track analysis.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct TrackAnalysisRequest {
    /// Optional track id from the metadata DB.
    #[serde(default)]
    pub track_id: Option<i64>,
    /// Absolute track path.
    #[serde(default)]
    pub path: Option<String>,
    /// Max seconds to analyze (defaults to 30).
    #[serde(default)]
    pub max_seconds: Option<f32>,
    /// Spectrogram width (columns).
    #[serde(default)]
    pub width: Option<usize>,
    /// Spectrogram height (rows).
    #[serde(default)]
    pub height: Option<usize>,
    /// FFT window size (samples).
    #[serde(default)]
    pub window_size: Option<usize>,
    /// High-frequency cutoff override (Hz) for ultrasonic ratio.
    #[serde(default)]
    pub high_cutoff_hz: Option<f32>,
}

/// Heuristic analysis data for a track.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct TrackAnalysisHeuristics {
    #[serde(default)]
    pub rolloff_hz: Option<f32>,
    #[serde(default)]
    pub ultrasonic_ratio: Option<f32>,
    #[serde(default)]
    pub upper_audible_ratio: Option<f32>,
    #[serde(default)]
    pub dynamic_range_db: Option<f32>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// On-demand track analysis response.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct TrackAnalysisResponse {
    pub width: usize,
    pub height: usize,
    pub sample_rate: u32,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Base64-encoded spectrogram intensity data (row-major, 0..255).
    pub data_base64: String,
    pub heuristics: TrackAnalysisHeuristics,
}

/// Current metadata fields for an album.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AlbumMetadataResponse {
    /// Album id from the metadata DB.
    pub album_id: i64,
    pub title: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<i32>,
}

/// Update request for writing album metadata to all tracks.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AlbumMetadataUpdateRequest {
    /// Album id from the metadata DB.
    pub album_id: i64,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub album_artist: Option<String>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub track_artist: Option<String>,
}

/// Response for album metadata updates.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AlbumMetadataUpdateResponse {
    /// Album id after update (may differ if merged).
    pub album_id: i64,
}

/// Text metadata for an artist or album.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct TextMetadata {
    pub text: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub updated_at_ms: Option<i64>,
}

/// Media asset metadata exposed to the UI.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct MediaAssetInfo {
    pub id: i64,
    pub url: String,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub updated_at_ms: Option<i64>,
}

/// Response payload for artist profile metadata.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ArtistProfileResponse {
    pub artist_id: i64,
    pub lang: String,
    pub bio: Option<TextMetadata>,
    pub image: Option<MediaAssetInfo>,
}

/// Response payload for album profile metadata.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AlbumProfileResponse {
    pub album_id: i64,
    pub lang: String,
    pub notes: Option<TextMetadata>,
    #[serde(default)]
    pub original_year: Option<i32>,
    #[serde(default)]
    pub edition_year: Option<i32>,
    #[serde(default)]
    pub edition_label: Option<String>,
    pub image: Option<MediaAssetInfo>,
}

/// Update request for artist profile metadata.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ArtistProfileUpdateRequest {
    pub artist_id: i64,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub bio_locked: Option<bool>,
    #[serde(default)]
    pub source: Option<String>,
}

/// Update request for album profile metadata.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AlbumProfileUpdateRequest {
    pub album_id: i64,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub notes_locked: Option<bool>,
    #[serde(default)]
    pub original_year: Option<i32>,
    #[serde(default)]
    pub edition_year: Option<i32>,
    #[serde(default)]
    pub edition_label: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

/// Request to set an artist image from a URL.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ArtistImageSetRequest {
    pub artist_id: i64,
    pub url: String,
}

/// Request to clear an artist image.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ArtistImageClearRequest {
    pub artist_id: i64,
}

/// Request to set an album image from a URL.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AlbumImageSetRequest {
    pub album_id: i64,
    pub url: String,
}

/// Request to clear an album image.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AlbumImageClearRequest {
    pub album_id: i64,
}

/// Payload to apply a MusicBrainz match to a track or album.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MusicBrainzMatchApplyRequest {
    Track {
        /// Absolute track path.
        path: String,
        /// Recording MBID to apply.
        recording_mbid: String,
        /// Optional artist MBID to apply.
        #[serde(default)]
        artist_mbid: Option<String>,
        /// Optional album/release MBID to apply.
        #[serde(default)]
        album_mbid: Option<String>,
        /// Optional release year.
        #[serde(default)]
        release_year: Option<i32>,
        /// Whether to overwrite existing MBIDs.
        #[serde(default)]
        override_existing: Option<bool>,
    },
    Album {
        /// Album id from the metadata DB.
        album_id: i64,
        /// Release MBID to apply.
        album_mbid: String,
        /// Optional artist MBID to apply.
        #[serde(default)]
        artist_mbid: Option<String>,
        /// Optional release year.
        #[serde(default)]
        release_year: Option<i32>,
        /// Whether to overwrite existing MBIDs.
        #[serde(default)]
        override_existing: Option<bool>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ArtistListResponse {
    pub items: Vec<ArtistSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AlbumListResponse {
    pub items: Vec<AlbumSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct TrackListResponse {
    pub items: Vec<TrackSummary>,
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

/// Payload to play a queued item and drop preceding items.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct QueuePlayFromRequest {
    /// Path of the queued item to play.
    #[serde(default)]
    pub path: Option<String>,
    /// Track id of the queued item to play.
    #[serde(default)]
    pub track_id: Option<i64>,
}

/// Payload to clear the queue, with an optional history reset.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct QueueClearRequest {
    /// True to also clear the recently played history.
    #[serde(default)]
    pub clear_history: bool,
    /// True to clear the queued items.
    #[serde(default = "default_queue_clear")]
    pub clear_queue: bool,
}

fn default_queue_clear() -> bool {
    true
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

/// Request payload for starting or refreshing a local playback session.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LocalPlaybackRegisterRequest {
    pub kind: String,
    pub name: String,
    pub client_id: String,
    pub app_version: String,
}

/// Response payload for local playback session registration.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LocalPlaybackRegisterResponse {
    pub session_id: String,
    pub play_url: String,
}

/// Resolve-stream request for local playback.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LocalPlaybackPlayRequest {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub track_id: Option<i64>,
}

/// Resolved stream URL for local playback.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LocalPlaybackPlayResponse {
    pub url: String,
    pub path: String,
    #[serde(default)]
    pub track_id: Option<i64>,
}

/// Session summary for local playback sessions.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LocalPlaybackSessionInfo {
    pub session_id: String,
    pub kind: String,
    pub name: String,
    pub app_version: String,
    pub created_age_ms: u64,
    pub last_seen_age_ms: u64,
}

/// Local playback session list.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LocalPlaybackSessionsResponse {
    pub sessions: Vec<LocalPlaybackSessionInfo>,
}

/// Output settings (disabled outputs and renames).
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, Default)]
pub struct OutputSettings {
    /// Disabled output ids (hidden from selection).
    #[serde(default)]
    pub disabled: Vec<String>,
    /// Output id -> display name overrides.
    #[serde(default)]
    pub renames: HashMap<String, String>,
    /// Output ids that should use exclusive mode (bridge-only).
    #[serde(default)]
    pub exclusive: Vec<String>,
}

/// Provider outputs bundled with provider info.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ProviderOutputs {
    /// Provider summary.
    pub provider: ProviderInfo,
    /// Optional provider address (bridge HTTP addr).
    pub address: Option<String>,
    /// Outputs for the provider.
    pub outputs: Vec<OutputInfo>,
}

/// Response payload for output settings.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OutputSettingsResponse {
    /// Current output settings.
    pub settings: OutputSettings,
    /// Providers and their outputs.
    pub providers: Vec<ProviderOutputs>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_entry_roundtrip() {
        let entry = LibraryEntry::Track {
            path: "/music/a.flac".to_string(),
            file_name: "a.flac".to_string(),
            ext_hint: "flac".to_string(),
            duration_ms: Some(1000),
            sample_rate: Some(48_000),
            album: Some("Album".to_string()),
            artist: Some("Artist".to_string()),
            format: "FLAC".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let de: LibraryEntry = serde_json::from_str(&json).unwrap();
        match de {
            LibraryEntry::Track { path, file_name, .. } => {
                assert_eq!(path, "/music/a.flac");
                assert_eq!(file_name, "a.flac");
            }
            _ => panic!("expected track"),
        }
    }

    #[test]
    fn queue_mode_roundtrip() {
        let json = serde_json::to_string(&QueueMode::Append).unwrap();
        assert_eq!(json, "\"append\"");
        let de: QueueMode = serde_json::from_str(&json).unwrap();
        assert!(matches!(de, QueueMode::Append));
    }

    #[test]
    fn output_info_roundtrip() {
        let info = OutputInfo {
            id: "bridge:one:device".to_string(),
            kind: "bridge".to_string(),
            name: "Device".to_string(),
            state: "online".to_string(),
            provider_id: Some("bridge:one".to_string()),
            provider_name: Some("Bridge".to_string()),
            supported_rates: Some(SupportedRates { min_hz: 44_100, max_hz: 192_000 }),
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        };
        let json = serde_json::to_string(&info).unwrap();
        let de: OutputInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(de.id, "bridge:one:device");
        assert_eq!(de.supported_rates.unwrap().max_hz, 192_000);
    }
}
