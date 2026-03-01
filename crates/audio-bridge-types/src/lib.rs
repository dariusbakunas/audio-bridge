use serde::{Deserialize, Serialize};

/// Reason why playback ended on the receiver side.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PlaybackEndReason {
    /// Natural end of stream/file.
    Eof,
    /// Decoder, transport, or output error interrupted playback.
    Error,
    /// Playback was explicitly stopped by a command.
    Stopped,
}

/// Low-level playback status reported by a bridge/receiver instance.
///
/// This payload is focused on transport and renderer details and does not include
/// library metadata identifiers (album/track ids).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BridgeStatus {
    /// Current file/path being played, if available.
    pub now_playing: Option<String>,
    /// `true` when playback is paused or idle.
    pub paused: bool,
    /// Elapsed playback time in milliseconds.
    pub elapsed_ms: Option<u64>,
    /// Total media duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Source codec (for example `flac`, `mp3`).
    pub source_codec: Option<String>,
    /// Source bit depth, if known.
    pub source_bit_depth: Option<u16>,
    /// Source container format, if known.
    pub container: Option<String>,
    /// Output sample format selected by the renderer.
    pub output_sample_format: Option<String>,
    /// Whether active playback is currently resampled.
    pub resampling: Option<bool>,
    /// Input sample rate before resampling (Hz).
    pub resample_from_hz: Option<u32>,
    /// Output sample rate after resampling (Hz).
    pub resample_to_hz: Option<u32>,
    /// Source sample rate (Hz).
    pub sample_rate: Option<u32>,
    /// Channel count.
    pub channels: Option<u16>,
    /// Active output device name, if known.
    pub device: Option<String>,
    /// Count of underrun frames observed by the output pipeline.
    pub underrun_frames: Option<u64>,
    /// Count of underrun events observed by the output pipeline.
    pub underrun_events: Option<u64>,
    /// Output buffer size in frames.
    pub buffer_size_frames: Option<u32>,
    /// Current buffered frames.
    pub buffered_frames: Option<u64>,
    /// Buffer capacity in frames.
    pub buffer_capacity_frames: Option<u64>,
    /// End reason when playback transitions to idle.
    pub end_reason: Option<PlaybackEndReason>,
}

/// Session-level playback status exposed by the hub API.
///
/// This extends bridge status with queue/library metadata and selected output id.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PlaybackStatus {
    /// Currently playing track id from metadata DB.
    pub now_playing_track_id: Option<i64>,
    /// `true` when session playback is paused or idle.
    pub paused: bool,
    /// `true` when the selected output backend is reachable.
    pub bridge_online: bool,
    /// Elapsed playback time in milliseconds.
    pub elapsed_ms: Option<u64>,
    /// Total media duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Source codec (for example `flac`, `mp3`).
    pub source_codec: Option<String>,
    /// Source bit depth.
    pub source_bit_depth: Option<u16>,
    /// Source container format.
    pub container: Option<String>,
    /// Renderer output sample format.
    pub output_sample_format: Option<String>,
    /// Whether resampling is currently active.
    pub resampling: Option<bool>,
    /// Input sample rate before resampling (Hz).
    pub resample_from_hz: Option<u32>,
    /// Output sample rate after resampling (Hz).
    pub resample_to_hz: Option<u32>,
    /// Source sample rate (Hz).
    pub sample_rate: Option<u32>,
    /// Channel count.
    pub channels: Option<u16>,
    /// Renderer output sample rate (Hz).
    pub output_sample_rate: Option<u32>,
    /// Selected output device display name.
    pub output_device: Option<String>,
    /// Current track title.
    pub title: Option<String>,
    /// Current track artist.
    pub artist: Option<String>,
    /// Current track album.
    pub album: Option<String>,
    /// File format label.
    pub format: Option<String>,
    /// Hub output id selected by this session.
    pub output_id: Option<String>,
    /// Approximate stream bitrate in kbps.
    pub bitrate_kbps: Option<u32>,
    /// Count of underrun frames observed by the output pipeline.
    pub underrun_frames: Option<u64>,
    /// Count of underrun events observed by the output pipeline.
    pub underrun_events: Option<u64>,
    /// Output buffer size in frames.
    pub buffer_size_frames: Option<u32>,
    /// Current buffered frames.
    pub buffered_frames: Option<u64>,
    /// Buffer capacity in frames.
    pub buffer_capacity_frames: Option<u64>,
    /// Whether a previous track is available in session history.
    pub has_previous: Option<bool>,
}
