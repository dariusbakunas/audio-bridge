//! HTTP client for bridge control + status polling.
//!
//! Wraps the bridge HTTP API with timeouts and JSON parsing.

use std::net::SocketAddr;
use std::io::BufRead;
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

/// Async HTTP transport client for bridge control and status.
#[derive(Clone)]
pub struct BridgeTransportClient {
    http_addr: SocketAddr,
    client: Client,
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
        }
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
    pub async fn set_device(&self, name: &str) -> Result<()> {
        let url = format!("http://{}/devices/select", self.http_addr);
        let payload = serde_json::json!({ "name": name });
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
}

/// Blocking HTTP transport client for bridge control and status.
#[derive(Clone)]
pub struct BridgeTransportClientBlocking {
    http_addr: SocketAddr,
    public_base_url: String,
    metadata: Option<MetadataDb>,
}

impl BridgeTransportClientBlocking {
    /// Create a new blocking client for a bridge HTTP address.
    pub fn new(http_addr: SocketAddr, public_base_url: String, metadata: Option<MetadataDb>) -> Self {
        Self {
            http_addr,
            public_base_url,
            metadata,
        }
    }

    /// Fetch the list of devices from the bridge.
    pub fn list_devices(&self) -> Result<Vec<HttpDeviceInfo>> {
        let url = format!("http://{}/devices", self.http_addr);
        let mut resp = ureq::get(&url)
            .config()
            .timeout_per_call(Some(Duration::from_secs(2)))
            .build()
            .call()
            .map_err(|e| anyhow::anyhow!("http devices request failed: {e}"))?;
        let resp: HttpDevicesResponse = resp
            .body_mut()
            .read_json()
            .map_err(|e| anyhow::anyhow!("http devices decode failed: {e}"))?;
        Ok(resp.devices)
    }

    /// Select an output device by name on the bridge.
    pub fn set_device(&self, name: &str) -> Result<()> {
        let url = format!("http://{}/devices/select", self.http_addr);
        let payload = serde_json::json!({ "name": name });
        ureq::post(&url)
            .config()
            .timeout_per_call(Some(Duration::from_secs(2)))
            .build()
            .send_json(payload)
            .map_err(|e| anyhow::anyhow!("http set device failed: {e}"))?;
        Ok(())
    }

    /// Listen for bridge device updates via server-sent events.
    pub fn listen_devices_stream<F>(&self, mut on_snapshot: F) -> Result<()>
    where
        F: FnMut(HttpDevicesSnapshot),
    {
        let url = format!("http://{}/devices/stream", self.http_addr);
        let resp = ureq::get(&url)
            .header("Accept", "text/event-stream")
            .config()
            .timeout_per_call(None)
            .build()
            .call()
            .map_err(|e| anyhow::anyhow!("http devices stream failed: {e}"))?;

        let mut reader = std::io::BufReader::new(resp.into_body().into_reader());
        let mut event = String::new();
        let mut data_lines: Vec<String> = Vec::new();
        loop {
            let mut line = String::new();
            let bytes = reader
                .read_line(&mut line)
                .map_err(|e| anyhow::anyhow!("http devices stream read failed: {e}"))?;
            if bytes == 0 {
                return Err(anyhow::anyhow!("http devices stream ended"));
            }
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

    /// Listen for bridge status updates via server-sent events.
    pub fn listen_status_stream<F>(&self, mut on_snapshot: F) -> Result<()>
    where
        F: FnMut(HttpStatusResponse),
    {
        let url = format!("http://{}/status/stream", self.http_addr);
        let resp = ureq::get(&url)
            .header("Accept", "text/event-stream")
            .config()
            .timeout_per_call(None)
            .build()
            .call()
            .map_err(|e| anyhow::anyhow!("http status stream failed: {e}"))?;

        let mut reader = std::io::BufReader::new(resp.into_body().into_reader());
        let mut event = String::new();
        let mut data_lines: Vec<String> = Vec::new();
        loop {
            let mut line = String::new();
            let bytes = reader
                .read_line(&mut line)
                .map_err(|e| anyhow::anyhow!("http status stream read failed: {e}"))?;
            if bytes == 0 {
                return Err(anyhow::anyhow!("http status stream ended"));
            }
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

    /// Ask the bridge to play the specified path via the hub stream URL.
    pub fn play_path(
        &self,
        path: &PathBuf,
        ext_hint: Option<&str>,
        title: Option<&str>,
        seek_ms: Option<u64>,
        start_paused: bool,
    ) -> Result<()> {
        let url = self.build_stream_url(path);
        let endpoint = format!("http://{}/play", self.http_addr);
        let payload = HttpPlayRequest {
            url: &url,
            ext_hint,
            title,
            seek_ms,
        };
        ureq::post(&endpoint)
            .config()
            .timeout_per_call(Some(Duration::from_secs(3)))
            .build()
            .send_json(payload)
            .map_err(|e| anyhow::anyhow!("http play failed: {e}"))?;
        if start_paused {
            self.pause_toggle()?;
        }
        Ok(())
    }

    /// Toggle pause/resume on the bridge.
    pub fn pause_toggle(&self) -> Result<()> {
        let endpoint = format!("http://{}/pause", self.http_addr);
        ureq::post(&endpoint)
            .config()
            .timeout_per_call(Some(Duration::from_secs(2)))
            .build()
            .send_json(serde_json::json!({}))
            .map_err(|e| anyhow::anyhow!("http pause failed: {e}"))?;
        Ok(())
    }

    /// Stop playback on the bridge.
    pub fn stop(&self) -> Result<()> {
        let endpoint = format!("http://{}/stop", self.http_addr);
        ureq::post(&endpoint)
            .config()
            .timeout_per_call(Some(Duration::from_secs(2)))
            .build()
            .send_json(serde_json::json!({}))
            .map_err(|e| anyhow::anyhow!("http stop failed: {e}"))?;
        Ok(())
    }

    /// Seek to the specified position in milliseconds.
    pub fn seek(&self, ms: u64) -> Result<()> {
        let endpoint = format!("http://{}/seek", self.http_addr);
        let payload = HttpSeekRequest { ms };
        ureq::post(&endpoint)
            .config()
            .timeout_per_call(Some(Duration::from_secs(2)))
            .build()
            .send_json(payload)
            .map_err(|e| anyhow::anyhow!("http seek failed: {e}"))?;
        Ok(())
    }

    /// Build a fully-qualified stream URL for the given path.
    fn build_stream_url(&self, path: &PathBuf) -> String {
        if let Some(track_id) = self.track_id_for_path(path) {
            build_stream_url_for_id(track_id, &self.public_base_url)
        } else {
            build_stream_url_for(path, &self.public_base_url)
        }
    }

    fn track_id_for_path(&self, path: &PathBuf) -> Option<i64> {
        self.metadata
            .as_ref()
            .and_then(|meta| meta.track_id_for_path(&path.to_string_lossy()).ok())
            .flatten()
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
