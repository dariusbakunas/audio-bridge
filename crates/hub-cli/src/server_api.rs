use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::library::{LibraryItem, Track};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LibraryEntry {
    Dir { path: String, name: String },
    Track {
        path: String,
        file_name: String,
        duration_ms: Option<u64>,
        sample_rate: Option<u32>,
        album: Option<String>,
        artist: Option<String>,
        format: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LibraryResponse {
    dir: String,
    entries: Vec<LibraryEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StatusResponse {
    now_playing: Option<String>,
    paused: bool,
    elapsed_ms: Option<u64>,
    duration_ms: Option<u64>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
    output_sample_rate: Option<u32>,
    output_device: Option<String>,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    format: Option<String>,
    output_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OutputsResponse {
    active_id: String,
    outputs: Vec<OutputInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OutputInfo {
    id: String,
    kind: String,
    name: String,
    state: String,
    bridge_id: Option<String>,
    bridge_name: Option<String>,
    supported_rates: Option<SupportedRates>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SupportedRates {
    min_hz: u32,
    max_hz: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum QueueItem {
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
struct QueueResponse {
    items: Vec<QueueItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct QueueAddRequest {
    paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct QueueRemoveRequest {
    path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum QueueMode {
    Keep,
    Replace,
    Append,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PlayRequest {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue_mode: Option<QueueMode>,
}

pub(crate) fn list_entries(server: &str, dir: &Path) -> Result<Vec<LibraryItem>> {
    let dir_str = dir.to_string_lossy();
    let url = format!(
        "{}/library?dir={}",
        server.trim_end_matches('/'),
        urlencoding::encode(&dir_str)
    );
    let resp: LibraryResponse = ureq::get(&url)
        .call()
        .context("request /library")?
        .into_json()
        .context("decode /library response")?;

    let mut out = Vec::with_capacity(resp.entries.len());
    for entry in resp.entries {
        match entry {
            LibraryEntry::Dir { path, name } => {
                out.push(LibraryItem::Dir { path: PathBuf::from(path), name });
            }
            LibraryEntry::Track { path, file_name, duration_ms, sample_rate, album, artist, format } => {
                out.push(LibraryItem::Track(Track {
                    path: PathBuf::from(path),
                    file_name,
                    duration_ms,
                    sample_rate,
                    album,
                    artist,
                    format,
                }));
            }
        }
    }
    Ok(out)
}

pub(crate) fn rescan(server: &str) -> Result<()> {
    let url = format!("{}/library/rescan", server.trim_end_matches('/'));
    let resp = ureq::post(&url).call().context("request /library/rescan")?;
    if resp.status() / 100 != 2 {
        return Err(anyhow::anyhow!("rescan failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn pause_toggle(server: &str) -> Result<()> {
    let url = format!("{}/pause", server.trim_end_matches('/'));
    let resp = ureq::post(&url).call().context("request /pause")?;
    if resp.status() / 100 != 2 {
        return Err(anyhow::anyhow!("pause failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) struct RemoteStatus {
    pub(crate) now_playing: Option<String>,
    pub(crate) elapsed_ms: Option<u64>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) paused: bool,
    pub(crate) sample_rate: Option<u32>,
    pub(crate) channels: Option<u16>,
    pub(crate) output_sample_rate: Option<u32>,
    pub(crate) output_device: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) artist: Option<String>,
    pub(crate) album: Option<String>,
    pub(crate) format: Option<String>,
    pub(crate) output_id: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteOutput {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) state: String,
    pub(crate) kind: String,
    pub(crate) bridge_id: Option<String>,
    pub(crate) bridge_name: Option<String>,
    pub(crate) supported_rates: Option<(u32, u32)>,
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteOutputs {
    pub(crate) active_id: String,
    pub(crate) outputs: Vec<RemoteOutput>,
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteQueueItem {
    pub(crate) path: PathBuf,
    pub(crate) meta: Option<crate::library::TrackMeta>,
}

pub(crate) struct RemoteQueue {
    pub(crate) items: Vec<RemoteQueueItem>,
}

pub(crate) fn status(server: &str) -> Result<RemoteStatus> {
    let url = format!("{}/status", server.trim_end_matches('/'));
    let resp: StatusResponse = ureq::get(&url)
        .call()
        .context("request /status")?
        .into_json()
        .context("decode /status response")?;
    Ok(RemoteStatus {
        now_playing: resp.now_playing,
        elapsed_ms: resp.elapsed_ms,
        duration_ms: resp.duration_ms,
        paused: resp.paused,
        sample_rate: resp.sample_rate,
        channels: resp.channels,
        output_sample_rate: resp.output_sample_rate,
        output_device: resp.output_device,
        title: resp.title,
        artist: resp.artist,
        album: resp.album,
        format: resp.format,
        output_id: resp.output_id,
    })
}

pub(crate) fn outputs(server: &str) -> Result<RemoteOutputs> {
    let url = format!("{}/outputs", server.trim_end_matches('/'));
    let resp: OutputsResponse = ureq::get(&url)
        .call()
        .context("request /outputs")?
        .into_json()
        .context("decode /outputs response")?;
    let outputs = resp
        .outputs
        .into_iter()
        .map(|o| RemoteOutput {
            id: o.id,
            name: o.name,
            state: o.state,
            kind: o.kind,
            bridge_id: o.bridge_id,
            bridge_name: o.bridge_name,
            supported_rates: o.supported_rates.map(|r| (r.min_hz, r.max_hz)),
        })
        .collect();
    Ok(RemoteOutputs {
        active_id: resp.active_id,
        outputs,
    })
}

pub(crate) fn outputs_select(server: &str, id: &str) -> Result<()> {
    let url = format!("{}/outputs/select", server.trim_end_matches('/'));
    let body = serde_json::json!({ "id": id });
    let resp = ureq::post(&url)
        .send_json(body)
        .context("request /outputs/select")?;
    if resp.status() / 100 != 2 {
        return Err(anyhow::anyhow!("outputs select failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn queue_list(server: &str) -> Result<RemoteQueue> {
    let url = format!("{}/queue", server.trim_end_matches('/'));
    let resp: QueueResponse = ureq::get(&url)
        .call()
        .context("request /queue")?
        .into_json()
        .context("decode /queue response")?;
    let items = resp
        .items
        .into_iter()
        .map(|item| match item {
            QueueItem::Track {
                path,
                duration_ms,
                sample_rate,
                album,
                artist,
                format,
                ..
            } => RemoteQueueItem {
                path: PathBuf::from(path),
                meta: Some(crate::library::TrackMeta {
                    duration_ms,
                    sample_rate,
                    album,
                    artist,
                    format: Some(format),
                }),
            },
            QueueItem::Missing { path } => RemoteQueueItem {
                path: PathBuf::from(path),
                meta: None,
            },
        })
        .collect();
    Ok(RemoteQueue {
        items,
    })
}

pub(crate) fn queue_add(server: &str, paths: &[PathBuf]) -> Result<()> {
    let url = format!("{}/queue", server.trim_end_matches('/'));
    let body = QueueAddRequest {
        paths: paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
    };
    let resp = ureq::post(&url)
        .send_json(body)
        .context("request /queue")?;
    if resp.status() / 100 != 2 {
        return Err(anyhow::anyhow!("queue add failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn queue_remove(server: &str, path: &Path) -> Result<()> {
    let url = format!("{}/queue/remove", server.trim_end_matches('/'));
    let body = QueueRemoveRequest {
        path: path.to_string_lossy().to_string(),
    };
    let resp = ureq::post(&url)
        .send_json(body)
        .context("request /queue/remove")?;
    if resp.status() / 100 != 2 {
        return Err(anyhow::anyhow!("queue remove failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn queue_next(server: &str) -> Result<bool> {
    let url = format!("{}/queue/next", server.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .call()
        .context("request /queue/next")?;
    Ok(resp.status() / 100 == 2)
}

pub(crate) fn play_replace(server: &str, path: &Path) -> Result<()> {
    let url = format!("{}/play", server.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .send_json(PlayRequest {
            path: path.to_string_lossy().to_string(),
            queue_mode: Some(QueueMode::Replace),
        })
        .context("request /play (replace)")?;
    if resp.status() / 100 != 2 {
        return Err(anyhow::anyhow!("play replace failed with {}", resp.status()));
    }
    Ok(())
}
