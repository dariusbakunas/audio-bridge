use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PlaybackEndReason {
    Eof,
    Error,
    Stopped,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BridgeStatus {
    pub now_playing: Option<String>,
    pub paused: bool,
    pub elapsed_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub source_codec: Option<String>,
    pub source_bit_depth: Option<u16>,
    pub container: Option<String>,
    pub output_sample_format: Option<String>,
    pub resampling: Option<bool>,
    pub resample_from_hz: Option<u32>,
    pub resample_to_hz: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub device: Option<String>,
    pub underrun_frames: Option<u64>,
    pub underrun_events: Option<u64>,
    pub buffer_size_frames: Option<u32>,
    pub buffered_frames: Option<u64>,
    pub buffer_capacity_frames: Option<u64>,
    pub end_reason: Option<PlaybackEndReason>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PlaybackStatus {
    pub now_playing: Option<String>,
    pub paused: bool,
    pub bridge_online: bool,
    pub elapsed_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub source_codec: Option<String>,
    pub source_bit_depth: Option<u16>,
    pub container: Option<String>,
    pub output_sample_format: Option<String>,
    pub resampling: Option<bool>,
    pub resample_from_hz: Option<u32>,
    pub resample_to_hz: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub output_sample_rate: Option<u32>,
    pub output_device: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub format: Option<String>,
    pub output_id: Option<String>,
    pub bitrate_kbps: Option<u32>,
    pub underrun_frames: Option<u64>,
    pub underrun_events: Option<u64>,
    pub buffer_size_frames: Option<u32>,
    pub buffered_frames: Option<u64>,
    pub buffer_capacity_frames: Option<u64>,
    pub has_previous: Option<bool>,
}
