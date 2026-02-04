use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use audio_bridge_types::PlaybackStatus;

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

pub type StatusResponse = PlaybackStatus;

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
pub struct OutputsResponse {
    pub active_id: Option<String>,
    pub outputs: Vec<OutputInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OutputInfo {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub state: String,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
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

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ProviderInfo {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub state: String,
    pub capabilities: OutputCapabilities,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ProvidersResponse {
    pub providers: Vec<ProviderInfo>,
}
