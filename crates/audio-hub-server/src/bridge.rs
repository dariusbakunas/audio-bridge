use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};

use crate::state::PlayerStatus;

#[derive(Debug, Clone)]
pub enum BridgeCommand {
    Play {
        path: PathBuf,
        ext_hint: String,
        seek_ms: Option<u64>,
        start_paused: bool,
    },
    PauseToggle,
    Stop,
    Seek { ms: u64 },
    Quit,
}

#[derive(Clone)]
pub struct BridgePlayer {
    pub(crate) cmd_tx: Sender<BridgeCommand>,
}

#[derive(Debug, serde::Deserialize)]
pub struct HttpDevicesResponse {
    pub devices: Vec<HttpDeviceInfo>,
}

#[derive(Debug, serde::Deserialize, Clone)]
pub struct HttpDeviceInfo {
    pub name: String,
    pub min_rate: u32,
    pub max_rate: u32,
}

#[derive(Debug, serde::Deserialize)]
pub struct HttpStatusResponse {
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
}

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

#[derive(Debug, serde::Serialize)]
struct HttpSeekRequest {
    ms: u64,
}

pub fn http_list_devices(addr: SocketAddr) -> Result<Vec<HttpDeviceInfo>> {
    let url = format!("http://{addr}/devices");
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

pub fn http_set_device(addr: SocketAddr, name: &str) -> Result<()> {
    let url = format!("http://{addr}/devices/select");
    let payload = serde_json::json!({ "name": name });
    ureq::post(&url)
        .config()
        .timeout_per_call(Some(Duration::from_secs(2)))
        .build()
        .send_json(payload)
        .map_err(|e| anyhow::anyhow!("http set device failed: {e}"))?;
    Ok(())
}

pub fn http_status(addr: SocketAddr) -> Result<HttpStatusResponse> {
    let url = format!("http://{addr}/status");
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

pub fn spawn_bridge_worker(
    bridge_id: String,
    http_addr: SocketAddr,
    cmd_rx: Receiver<BridgeCommand>,
    cmd_tx: Sender<BridgeCommand>,
    status: Arc<Mutex<PlayerStatus>>,
    queue: Arc<Mutex<crate::state::QueueState>>,
    bridge_online: Arc<AtomicBool>,
    bridges_state: Arc<Mutex<crate::state::BridgeState>>,
    public_base_url: String,
) {
    std::thread::spawn(move || {
        tracing::info!(bridge_id = %bridge_id, http_addr = %http_addr, "bridge worker start");
        let mut next_poll = Instant::now();

        loop {
            let now = Instant::now();
            let timeout = next_poll.saturating_duration_since(now).min(Duration::from_millis(250));
            if let Ok(cmd) = cmd_rx.recv_timeout(timeout) {
                match cmd {
                    BridgeCommand::Quit => break,
                    BridgeCommand::PauseToggle => {
                        let _ = http_pause_toggle(http_addr);
                        if let Ok(mut s) = status.lock() {
                            s.paused = !s.paused;
                            s.user_paused = s.paused;
                        }
                    }
                    BridgeCommand::Stop => {
                        let _ = http_stop(http_addr);
                        if let Ok(mut s) = status.lock() {
                            s.now_playing = None;
                            s.paused = false;
                            s.user_paused = false;
                            s.elapsed_ms = None;
                            s.duration_ms = None;
                            s.sample_rate = None;
                            s.channels = None;
                            s.source_codec = None;
                            s.source_bit_depth = None;
                            s.container = None;
                            s.output_sample_format = None;
                            s.resampling = None;
                            s.resample_from_hz = None;
                            s.resample_to_hz = None;
                            s.auto_advance_in_flight = false;
                        }
                    }
                    BridgeCommand::Seek { ms } => {
                        let _ = http_seek(http_addr, ms);
                    }
                    BridgeCommand::Play { path, ext_hint, seek_ms, start_paused } => {
                        let url = build_stream_url(&public_base_url, &path);
                        let title = path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_string());
                        let _ = http_play(
                            http_addr,
                            &url,
                            if ext_hint.is_empty() { None } else { Some(ext_hint.as_str()) },
                            title.as_deref(),
                            seek_ms,
                        );
                        if start_paused {
                            let _ = http_pause_toggle(http_addr);
                        }

                        if let Ok(mut s) = status.lock() {
                            s.now_playing = Some(path.clone());
                            s.paused = false;
                            s.user_paused = false;
                            s.elapsed_ms = Some(0);
                            s.sample_rate = None;
                            s.channels = None;
                            s.source_codec = None;
                            s.source_bit_depth = None;
                            s.container = None;
                            s.output_sample_format = None;
                            s.resampling = None;
                            s.resample_from_hz = None;
                            s.resample_to_hz = None;
                            s.auto_advance_in_flight = false;
                        }
                    }
                }
            }

            if Instant::now() < next_poll {
                continue;
            }
            next_poll = Instant::now() + Duration::from_millis(500);

        if let Ok(remote) = http_status(http_addr) {
            bridge_online.store(true, Ordering::Relaxed);
                if let Ok(mut s) = status.lock() {
                    s.paused = remote.paused;
                    s.elapsed_ms = remote.elapsed_ms;
                    s.duration_ms = remote.duration_ms;
                    s.sample_rate = remote.sample_rate;
                    s.channels = remote.channels;
                    s.output_device = remote.device;
                    s.source_codec = remote.source_codec;
                    s.source_bit_depth = remote.source_bit_depth;
                    s.container = remote.container;
                    s.output_sample_format = remote.output_sample_format;
                    s.resampling = remote.resampling;
                    s.resample_from_hz = remote.resample_from_hz;
                    s.resample_to_hz = remote.resample_to_hz;

                    if !s.auto_advance_in_flight {
                        if let (Some(elapsed), Some(duration)) = (s.elapsed_ms, s.duration_ms) {
                            if elapsed + 50 >= duration && !s.user_paused {
                                drop(s);
                                if let Some(path) = pop_next_from_queue(&queue) {
                                    let ext_hint = path
                                        .extension()
                                        .and_then(|ext| ext.to_str())
                                        .unwrap_or("")
                                        .to_ascii_lowercase();
                                        let _ = cmd_tx.send(BridgeCommand::Play {
                                            path,
                                            ext_hint,
                                            seek_ms: None,
                                            start_paused: false,
                                        });
                                    if let Ok(mut s2) = status.lock() {
                                        s2.auto_advance_in_flight = true;
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                if bridges_state
                    .lock()
                    .map(|s| s.active_bridge_id.as_deref() == Some(bridge_id.as_str()))
                    .unwrap_or(false)
                {
                    bridge_online.store(false, Ordering::Relaxed);
                }
            }
        }
    });
}

fn http_play(
    addr: SocketAddr,
    url: &str,
    ext_hint: Option<&str>,
    title: Option<&str>,
    seek_ms: Option<u64>,
) -> Result<()> {
    let endpoint = format!("http://{addr}/play");
    let payload = HttpPlayRequest {
        url,
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
    Ok(())
}

fn http_pause_toggle(addr: SocketAddr) -> Result<()> {
    let endpoint = format!("http://{addr}/pause");
    ureq::post(&endpoint)
        .config()
        .timeout_per_call(Some(Duration::from_secs(2)))
        .build()
        .send_json(serde_json::json!({}))
        .map_err(|e| anyhow::anyhow!("http pause failed: {e}"))?;
    Ok(())
}

fn http_stop(addr: SocketAddr) -> Result<()> {
    let endpoint = format!("http://{addr}/stop");
    ureq::post(&endpoint)
        .config()
        .timeout_per_call(Some(Duration::from_secs(2)))
        .build()
        .send_json(serde_json::json!({}))
        .map_err(|e| anyhow::anyhow!("http stop failed: {e}"))?;
    Ok(())
}

fn http_seek(addr: SocketAddr, ms: u64) -> Result<()> {
    let endpoint = format!("http://{addr}/seek");
    let payload = HttpSeekRequest { ms };
    ureq::post(&endpoint)
        .config()
        .timeout_per_call(Some(Duration::from_secs(2)))
        .build()
        .send_json(payload)
        .map_err(|e| anyhow::anyhow!("http seek failed: {e}"))?;
    Ok(())
}

fn build_stream_url(base: &str, path: &PathBuf) -> String {
    let path_str = path.to_string_lossy();
    let encoded = urlencoding::encode(&path_str);
    format!("{}/stream?path={encoded}", base.trim_end_matches('/'))
}

fn pop_next_from_queue(queue: &Arc<Mutex<crate::state::QueueState>>) -> Option<PathBuf> {
    let mut q = queue.lock().ok()?;
    if q.items.is_empty() {
        None
    } else {
        Some(q.items.remove(0))
    }
}
