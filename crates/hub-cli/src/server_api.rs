use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use audio_bridge_types::PlaybackStatus;

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

type StatusResponse = PlaybackStatus;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OutputsResponse {
    active_id: Option<String>,
    outputs: Vec<OutputInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OutputInfo {
    id: String,
    kind: String,
    name: String,
    state: String,
    provider_id: Option<String>,
    provider_name: Option<String>,
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
struct SeekRequest {
    ms: u64,
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
    let resp: LibraryResponse = read_json(
        ureq::get(&url)
            .call()
            .context("request /library")?,
        "library",
    )?;

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
    let resp = ureq::post(&url)
        .send_empty()
        .context("request /library/rescan")?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("rescan failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn pause_toggle(server: &str) -> Result<()> {
    let url = format!("{}/pause", server.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .send_empty()
        .context("request /pause")?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("pause failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn stop(server: &str) -> Result<()> {
    let url = format!("{}/stop", server.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .send_empty()
        .context("request /stop")?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("stop failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn seek(server: &str, ms: u64) -> Result<()> {
    let url = format!("{}/seek", server.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .send_json(SeekRequest { ms })
        .context("request /seek")?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("seek failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) type RemoteStatus = PlaybackStatus;

#[derive(Clone, Debug)]
pub(crate) struct RemoteOutput {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) provider_id: Option<String>,
    pub(crate) provider_name: Option<String>,
    pub(crate) supported_rates: Option<(u32, u32)>,
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteOutputs {
    pub(crate) active_id: Option<String>,
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
    let base = server.trim_end_matches('/');
    let outputs = outputs(base)?;
    let Some(active_id) = outputs.active_id else {
        return Err(anyhow::anyhow!("no active output selected"));
    };
    status_for_output(base, &active_id)
}

pub(crate) fn status_for_output(server: &str, output_id: &str) -> Result<RemoteStatus> {
    let base = server.trim_end_matches('/');
    let url = format!(
        "{}/outputs/{}/status",
        base,
        urlencoding::encode(output_id)
    );
    let resp: StatusResponse = read_json(
        ureq::get(&url)
            .call()
            .context("request /outputs/{id}/status")?,
        "status",
    )?;
    Ok(resp)
}

pub(crate) fn outputs(server: &str) -> Result<RemoteOutputs> {
    let url = format!("{}/outputs", server.trim_end_matches('/'));
    let resp: OutputsResponse = read_json(
        ureq::get(&url)
            .call()
            .context("request /outputs")?,
        "outputs",
    )?;
    let outputs = resp
        .outputs
        .into_iter()
        .map(|o| RemoteOutput {
            id: o.id,
            name: o.name,
            provider_id: o.provider_id,
            provider_name: o.provider_name,
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
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("outputs select failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn queue_list(server: &str) -> Result<RemoteQueue> {
    let url = format!("{}/queue", server.trim_end_matches('/'));
    let resp: QueueResponse = read_json(
        ureq::get(&url)
            .call()
            .context("request /queue")?,
        "queue",
    )?;
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
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("queue add failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn queue_add_next(server: &str, paths: &[PathBuf]) -> Result<()> {
    let url = format!("{}/queue/next/add", server.trim_end_matches('/'));
    let body = QueueAddRequest {
        paths: paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
    };
    let resp = ureq::post(&url)
        .send_json(body)
        .context("request /queue/next/add")?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("queue add-next failed with {}", resp.status()));
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
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("queue remove failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn queue_next(server: &str) -> Result<bool> {
    let url = format!("{}/queue/next", server.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .send_empty()
        .context("request /queue/next")?;
    Ok(resp.status().is_success())
}

pub(crate) fn queue_clear(server: &str) -> Result<()> {
    let url = format!("{}/queue/clear", server.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .send_empty()
        .context("request /queue/clear")?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("queue clear failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn play_replace(server: &str, path: &Path) -> Result<()> {
    let url = format!("{}/play", server.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .send_json(PlayRequest {
            path: path.to_string_lossy().to_string(),
            queue_mode: Some(QueueMode::Replace),
        })
        .context("request /play (replace)")?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("play replace failed with {}", resp.status()));
    }
    Ok(())
}

pub(crate) fn play_keep(server: &str, path: &Path) -> Result<()> {
    let url = format!("{}/play", server.trim_end_matches('/'));
    let resp = ureq::post(&url)
        .send_json(PlayRequest {
            path: path.to_string_lossy().to_string(),
            queue_mode: Some(QueueMode::Keep),
        })
        .context("request /play (keep)")?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("play failed with {}", resp.status()));
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(
    mut resp: ureq::http::Response<ureq::Body>,
    label: &str,
) -> Result<T> {
    let body = resp
        .body_mut()
        .read_to_string()
        .with_context(|| format!("read /{label} response body"))?;
    serde_json::from_str(&body).with_context(|| format!("decode /{label} response"))
}
