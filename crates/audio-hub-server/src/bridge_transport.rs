//! HTTP client for bridge control + status polling.
//!
//! Wraps the bridge HTTP API with timeouts and JSON parsing.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use reqwest::Client;

use audio_bridge_types::BridgeStatus;
use crate::metadata_db::MetadataDb;

/// HTTP response payload for the bridge device list.
#[derive(Debug, serde::Deserialize)]
pub struct HttpDevicesResponse {
    /// Devices reported by the bridge.
    pub devices: Vec<HttpDeviceInfo>,
}

/// Device info returned by the bridge HTTP API.
#[derive(Debug, serde::Deserialize, Clone, PartialEq, Eq)]
pub struct HttpDeviceInfo {
    /// Device identifier reported by the bridge.
    pub id: String,
    /// Human-friendly device name.
    pub name: String,
    /// Minimum supported sample rate (Hz).
    pub min_rate: u32,
    /// Maximum supported sample rate (Hz).
    pub max_rate: u32,
}

pub type HttpStatusResponse = BridgeStatus;

/// HTTP response payload for the bridge device stream.
#[derive(Debug, serde::Deserialize, Clone, PartialEq, Eq)]
pub struct HttpDevicesSnapshot {
    /// Devices reported by the bridge.
    pub devices: Vec<HttpDeviceInfo>,
    /// Selected device name (if any).
    pub selected: Option<String>,
    /// Selected device id (if any).
    pub selected_id: Option<String>,
}

/// JSON payload for starting playback on the bridge.
#[derive(Debug, serde::Serialize)]
struct HttpPlayRequest<'a> {
    url: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    ext_hint: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seek_ms: Option<u64>,
}

/// JSON payload for bridge seek requests.
#[derive(Debug, serde::Serialize)]
struct HttpSeekRequest {
    ms: u64,
}

/// JSON payload for bridge volume set requests.
#[derive(Debug, serde::Serialize)]
struct HttpVolumeSetRequest {
    value: u8,
}

/// JSON payload for bridge mute requests.
#[derive(Debug, serde::Serialize)]
struct HttpMuteRequest {
    muted: bool,
}

/// JSON payload for bridge volume snapshot.
#[derive(Debug, serde::Deserialize, Clone, Copy)]
pub struct HttpVolumeResponse {
    pub value: u8,
    pub muted: bool,
}

/// Async HTTP transport client for bridge control and status.
#[derive(Clone)]
pub struct BridgeTransportClient {
    http_addr: SocketAddr,
    client: Client,
    public_base_url: Option<String>,
    metadata: Option<MetadataDb>,
}

impl BridgeTransportClient {
    /// Create a new async client for a bridge HTTP address.
    pub fn new(http_addr: SocketAddr) -> Self {
        let client = Client::builder()
            .build()
            .expect("build reqwest client");
        Self {
            http_addr,
            client,
            public_base_url: None,
            metadata: None,
        }
    }

    /// Create a new async client configured for playback requests.
    pub fn new_with_base(
        http_addr: SocketAddr,
        public_base_url: String,
        metadata: Option<MetadataDb>,
    ) -> Self {
        let mut client = Self::new(http_addr);
        client.public_base_url = Some(public_base_url);
        client.metadata = metadata;
        client
    }

    /// Fetch the list of devices from the bridge.
    pub async fn list_devices(&self) -> Result<Vec<HttpDeviceInfo>> {
        let url = format!("http://{}/devices", self.http_addr);
        let resp = self.client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http devices request failed: {e}"))?;
        let resp = resp
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("http devices request failed: {e}"))?;
        let payload: HttpDevicesResponse = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("http devices decode failed: {e}"))?;
        Ok(payload.devices)
    }

    /// Select an output device by name on the bridge.
    pub async fn set_device(&self, name: &str, exclusive: Option<bool>) -> Result<()> {
        let url = format!("http://{}/devices/select", self.http_addr);
        let mut payload = serde_json::json!({ "name": name });
        if let Some(exclusive) = exclusive {
            payload["exclusive"] = serde_json::json!(exclusive);
        }
        self.client
            .post(&url)
            .timeout(Duration::from_secs(2))
            .json(&payload)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http set device failed: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("http set device failed: {e}"))?;
        Ok(())
    }

    /// Fetch the current bridge status snapshot.
    pub async fn status(&self) -> Result<HttpStatusResponse> {
        let url = format!("http://{}/status", self.http_addr);
        let resp = self.client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http status request failed: {e}"))?;
        let resp = resp
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("http status request failed: {e}"))?;
        let payload: HttpStatusResponse = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("http status decode failed: {e}"))?;
        Ok(payload)
    }

    /// Stop playback on the bridge.
    pub async fn stop(&self) -> Result<()> {
        let endpoint = format!("http://{}/stop", self.http_addr);
        self.client
            .post(&endpoint)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http stop failed: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("http stop failed: {e}"))?;
        Ok(())
    }

    /// Toggle pause/resume on the bridge.
    pub async fn pause_toggle(&self) -> Result<()> {
        let endpoint = format!("http://{}/pause", self.http_addr);
        self.client
            .post(&endpoint)
            .timeout(Duration::from_secs(2))
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http pause failed: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("http pause failed: {e}"))?;
        Ok(())
    }

    /// Seek to the specified position in milliseconds.
    pub async fn seek(&self, ms: u64) -> Result<()> {
        let endpoint = format!("http://{}/seek", self.http_addr);
        let payload = HttpSeekRequest { ms };
        self.client
            .post(&endpoint)
            .timeout(Duration::from_secs(2))
            .json(&payload)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http seek failed: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("http seek failed: {e}"))?;
        Ok(())
    }

    /// Fetch current bridge volume snapshot.
    pub async fn volume(&self) -> Result<HttpVolumeResponse> {
        let endpoint = format!("http://{}/volume", self.http_addr);
        let resp = self.client
            .get(&endpoint)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http volume request failed: {e}"))?;
        let resp = resp
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("http volume request failed: {e}"))?;
        resp.json::<HttpVolumeResponse>()
            .await
            .map_err(|e| anyhow::anyhow!("http volume decode failed: {e}"))
    }

    /// Set bridge volume percent (0..100).
    pub async fn set_volume(&self, value: u8) -> Result<HttpVolumeResponse> {
        let endpoint = format!("http://{}/volume", self.http_addr);
        self.client
            .post(&endpoint)
            .timeout(Duration::from_secs(2))
            .json(&HttpVolumeSetRequest {
                value: value.min(100),
            })
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http set volume failed: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("http set volume failed: {e}"))?
            .json::<HttpVolumeResponse>()
            .await
            .map_err(|e| anyhow::anyhow!("http set volume decode failed: {e}"))
    }

    /// Set bridge mute state.
    pub async fn set_mute(&self, muted: bool) -> Result<HttpVolumeResponse> {
        let endpoint = format!("http://{}/mute", self.http_addr);
        self.client
            .post(&endpoint)
            .timeout(Duration::from_secs(2))
            .json(&HttpMuteRequest { muted })
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http set mute failed: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("http set mute failed: {e}"))?
            .json::<HttpVolumeResponse>()
            .await
            .map_err(|e| anyhow::anyhow!("http set mute decode failed: {e}"))
    }

    /// Ask the bridge to play the specified path via the hub stream URL.
    pub async fn play_path(
        &self,
        path: &PathBuf,
        ext_hint: Option<&str>,
        title: Option<&str>,
        seek_ms: Option<u64>,
        start_paused: bool,
    ) -> Result<()> {
        let base_url = self
            .public_base_url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("public base url not configured"))?;
        let url = if let Some(track_id) = self.track_id_for_path(path) {
            build_stream_url_for_id(track_id, base_url)
        } else {
            build_stream_url_for(path, base_url)
        };
        let endpoint = format!("http://{}/play", self.http_addr);
        let payload = HttpPlayRequest {
            url: &url,
            ext_hint,
            title,
            seek_ms,
        };
        self.client
            .post(&endpoint)
            .timeout(Duration::from_secs(3))
            .json(&payload)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http play failed: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("http play failed: {e}"))?;
        if start_paused {
            self.pause_toggle().await?;
        }
        Ok(())
    }

    fn track_id_for_path(&self, path: &PathBuf) -> Option<i64> {
        self.metadata
            .as_ref()
            .and_then(|meta| meta.track_id_for_path(&path.to_string_lossy()).ok())
            .flatten()
    }

    /// Listen for bridge device updates via server-sent events.
    pub async fn listen_devices_stream<F>(&self, mut on_snapshot: F) -> Result<()>
    where
        F: FnMut(HttpDevicesSnapshot) + Send,
    {
        let url = format!("http://{}/devices/stream", self.http_addr);
        let resp = self.client
            .get(&url)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http devices stream failed: {e}"))?;

        let mut stream = resp.bytes_stream();
        let mut buffer = Vec::new();
        let mut event = String::new();
        let mut data_lines: Vec<String> = Vec::new();

        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => return Err(anyhow::anyhow!("http devices stream read failed: {e}")),
            };
            buffer.extend_from_slice(&chunk);
            while let Some(pos) = buffer.windows(1).position(|w| w[0] == b'\n') {
                let line = buffer.drain(..=pos).collect::<Vec<_>>();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim_end_matches(&['\r', '\n'][..]);
                if line.is_empty() {
                    if !data_lines.is_empty() {
                        let payload = data_lines.join("\n");
                        if event == "devices" {
                            match serde_json::from_str::<HttpDevicesSnapshot>(&payload) {
                                Ok(snapshot) => on_snapshot(snapshot),
                                Err(e) => {
                                    tracing::warn!(error = %e, "http devices stream decode failed");
                                }
                            }
                        }
                    }
                    event.clear();
                    data_lines.clear();
                    continue;
                }
                if let Some(rest) = line.strip_prefix("event:") {
                    event = rest.trim().to_string();
                } else if let Some(rest) = line.strip_prefix("data:") {
                    data_lines.push(rest.trim_start().to_string());
                }
            }
        }
        Err(anyhow::anyhow!("http devices stream ended"))
    }

    /// Listen for bridge status updates via server-sent events.
    pub async fn listen_status_stream<F>(&self, mut on_snapshot: F) -> Result<()>
    where
        F: FnMut(HttpStatusResponse) + Send,
    {
        let url = format!("http://{}/status/stream", self.http_addr);
        let resp = self.client
            .get(&url)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("http status stream failed: {e}"))?;

        let mut stream = resp.bytes_stream();
        let mut buffer = Vec::new();
        let mut event = String::new();
        let mut data_lines: Vec<String> = Vec::new();

        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => return Err(anyhow::anyhow!("http status stream read failed: {e}")),
            };
            buffer.extend_from_slice(&chunk);
            while let Some(pos) = buffer.windows(1).position(|w| w[0] == b'\n') {
                let line = buffer.drain(..=pos).collect::<Vec<_>>();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim_end_matches(&['\r', '\n'][..]);
                if line.is_empty() {
                    if !data_lines.is_empty() {
                        let payload = data_lines.join("\n");
                        if event == "status" {
                            match serde_json::from_str::<HttpStatusResponse>(&payload) {
                                Ok(snapshot) => on_snapshot(snapshot),
                                Err(e) => {
                                    tracing::warn!(error = %e, "http status stream decode failed");
                                }
                            }
                        }
                    }
                    event.clear();
                    data_lines.clear();
                    continue;
                }
                if let Some(rest) = line.strip_prefix("event:") {
                    event = rest.trim().to_string();
                } else if let Some(rest) = line.strip_prefix("data:") {
                    data_lines.push(rest.trim_start().to_string());
                }
            }
        }
        Err(anyhow::anyhow!("http status stream ended"))
    }
}

fn build_stream_url_for(path: &PathBuf, public_base_url: &str) -> String {
    let path_str = path.to_string_lossy();
    let encoded = urlencoding::encode(&path_str);
    format!(
        "{}/stream?path={encoded}",
        public_base_url.trim_end_matches('/')
    )
}

fn build_stream_url_for_id(track_id: i64, public_base_url: &str) -> String {
    format!(
        "{}/stream/track/{track_id}",
        public_base_url.trim_end_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_stream_url_for_encodes_and_trims() {
        let path = PathBuf::from("/music/My Song.flac");
        let url = build_stream_url_for(&path, "http://host/");
        assert_eq!(
            url,
            "http://host/stream?path=%2Fmusic%2FMy%20Song.flac"
        );
    }
}
