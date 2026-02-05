//! HTTP client for bridge control + status polling.
//!
//! Wraps the bridge HTTP API with timeouts and JSON parsing.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

use audio_bridge_types::BridgeStatus;

/// HTTP response payload for the bridge device list.
#[derive(Debug, serde::Deserialize)]
pub struct HttpDevicesResponse {
    /// Devices reported by the bridge.
    pub devices: Vec<HttpDeviceInfo>,
}

/// Device info returned by the bridge HTTP API.
#[derive(Debug, serde::Deserialize, Clone)]
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

/// HTTP transport client for bridge control and status.
#[derive(Clone)]
pub struct BridgeTransportClient {
    http_addr: SocketAddr,
    public_base_url: String,
}

impl BridgeTransportClient {
    /// Create a new client for a bridge HTTP address.
    pub fn new(http_addr: SocketAddr, public_base_url: String) -> Self {
        Self {
            http_addr,
            public_base_url,
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

    /// Fetch the current bridge status snapshot.
    pub fn status(&self) -> Result<HttpStatusResponse> {
        let url = format!("http://{}/status", self.http_addr);
        let mut resp = ureq::get(&url)
            .config()
            .timeout_per_call(Some(Duration::from_secs(2)))
            .build()
            .call()
            .map_err(|e| anyhow::anyhow!("http status request failed: {e}"))?;
        let resp: HttpStatusResponse = resp
            .body_mut()
            .read_json()
            .map_err(|e| anyhow::anyhow!("http status decode failed: {e}"))?;
        Ok(resp)
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
        let path_str = path.to_string_lossy();
        let encoded = urlencoding::encode(&path_str);
        format!(
            "{}/stream?path={encoded}",
            self.public_base_url.trim_end_matches('/')
        )
    }
}
