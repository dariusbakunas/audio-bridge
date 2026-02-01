use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryResponse {
    pub dir: String,
    pub entries: Vec<LibraryEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayRequest {
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub now_playing: Option<String>,
    pub paused: bool,
    pub elapsed_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub format: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueueResponse {
    pub items: Vec<QueueItem>,
    pub index: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueueAddRequest {
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueueRemoveRequest {
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueueReplacePlayRequest {
    pub path: String,
}
