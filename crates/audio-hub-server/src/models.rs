use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LibraryEntry {
    Dir {
        path: String,
        name: String,
    },
    Track {
        path: String,
        file_name: String,
        ext_hint: String,
        duration_ms: Option<u64>,
        sample_rate: Option<u32>,
        album: Option<String>,
        artist: Option<String>,
        format: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LibraryResponse {
    pub dir: String,
    pub entries: Vec<LibraryEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct PlayRequest {
    pub path: String,
    #[serde(default)]
    pub queue_mode: Option<QueueMode>,
    #[serde(default)]
    pub output_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueueMode {
    Keep,
    Replace,
    Append,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct StatusResponse {
    pub now_playing: Option<String>,
    pub paused: bool,
    pub elapsed_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub output_sample_rate: Option<u32>,
    pub output_device: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub format: Option<String>,
    pub output_id: String,
    pub underrun_frames: Option<u64>,
    pub underrun_events: Option<u64>,
    pub buffer_size_frames: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueItem {
    Track {
        path: String,
        file_name: String,
        duration_ms: Option<u64>,
        sample_rate: Option<u32>,
        album: Option<String>,
        artist: Option<String>,
        format: String,
    },
    Missing { path: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct QueueResponse {
    pub items: Vec<QueueItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct QueueAddRequest {
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct QueueRemoveRequest {
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct BridgeInfo {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct BridgesResponse {
    pub bridges: Vec<BridgeInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OutputsResponse {
    pub active_id: String,
    pub outputs: Vec<OutputInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OutputInfo {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub state: String,
    pub bridge_id: Option<String>,
    pub bridge_name: Option<String>,
    pub supported_rates: Option<SupportedRates>,
    pub capabilities: OutputCapabilities,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SupportedRates {
    pub min_hz: u32,
    pub max_hz: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OutputCapabilities {
    pub device_select: bool,
    pub volume: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OutputSelectRequest {
    pub id: String,
}
