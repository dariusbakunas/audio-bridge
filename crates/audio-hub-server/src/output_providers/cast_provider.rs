//! Chromecast (Google Cast) output provider.
//!
//! Supports discovery, selection, and basic Default Media Receiver playback.

use async_trait::async_trait;
use crossbeam_channel::Sender;

use crate::cast_v2::{spawn_cast_worker, CastDeviceDescriptor};
use crate::bridge::BridgeCommand;
use crate::models::{OutputCapabilities, OutputInfo, OutputsResponse, ProviderInfo, StatusResponse};
use crate::output_providers::registry::{OutputProvider, ProviderError};
use crate::state::AppState;

pub(crate) struct CastProvider;

impl CastProvider {
    fn provider_id() -> &'static str {
        "cast"
    }

    fn output_id(device_id: &str) -> String {
        format!("cast:{device_id}")
    }

    pub(crate) fn parse_output_id(output_id: &str) -> Option<String> {
        let mut parts = output_id.splitn(2, ':');
        let kind = parts.next().unwrap_or("");
        let id = parts.next().unwrap_or("");
        if kind != "cast" || id.is_empty() {
            return None;
        }
        Some(id.to_string())
    }

    pub(crate) fn ensure_worker_for_output(
        state: &AppState,
        output_id: &str,
    ) -> Result<Sender<BridgeCommand>, ProviderError> {
        let Some(device_id) = Self::parse_output_id(output_id) else {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        };
        if let Some(existing) = state
            .providers
            .cast
            .workers
            .lock()
            .ok()
            .and_then(|map| map.get(output_id).cloned())
        {
            return Ok(existing);
        }
        let found = state
            .providers
            .cast
            .discovered
            .lock()
            .ok()
            .and_then(|map| map.get(&device_id).cloned());
        let Some(found) = found else {
            return Err(ProviderError::Unavailable("cast device offline".to_string()));
        };
        let host = found
            .host
            .clone()
            .ok_or_else(|| ProviderError::Unavailable("cast device missing host".to_string()))?;
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        spawn_cast_worker(
            output_id.to_string(),
            CastDeviceDescriptor {
                id: found.id,
                name: found.name.clone(),
                host,
                port: found.port,
            },
            cmd_rx,
            cmd_tx.clone(),
            state.playback.manager.status().clone(),
            state.playback.manager.queue_service().queue().clone(),
            state.events.clone(),
            state.providers.bridge.public_base_url.clone(),
            Some(state.metadata.db.clone()),
            state.providers.bridge.bridges.clone(),
            state.providers.cast.workers.clone(),
            state.providers.cast.status_by_output.clone(),
        );
        if let Ok(mut workers) = state.providers.cast.workers.lock() {
            workers.insert(output_id.to_string(), cmd_tx.clone());
        }
        Ok(cmd_tx)
    }

    fn active_output_id(state: &AppState) -> Option<String> {
        state.providers.bridge.bridges.lock().unwrap().active_output_id.clone()
    }

    fn device_output_info(device: &crate::state::DiscoveredCast, active_id: &Option<String>) -> OutputInfo {
        let id = Self::output_id(&device.id);
        let state = if active_id.as_deref() == Some(&id) {
            "active"
        } else {
            "online"
        };
        let name = if let Some(host) = device.host.as_deref() {
            format!("{} ({})", device.name, host)
        } else {
            device.name.clone()
        };
        OutputInfo {
            id,
            kind: "cast".to_string(),
            name,
            state: state.to_string(),
            provider_id: Some(Self::provider_id().to_string()),
            provider_name: Some("Chromecast".to_string()),
            supported_rates: None,
            capabilities: OutputCapabilities {
                device_select: false,
                volume: false,
            },
        }
    }

    fn idle_status(output_id: &str, device_name: Option<String>, bridge_online: bool) -> StatusResponse {
        StatusResponse {
            now_playing: None,
            paused: true,
            bridge_online,
            elapsed_ms: None,
            duration_ms: None,
            source_codec: None,
            source_bit_depth: None,
            container: None,
            output_sample_format: None,
            resampling: None,
            resample_from_hz: None,
            resample_to_hz: None,
            sample_rate: None,
            channels: None,
            output_sample_rate: None,
            output_device: device_name,
            title: None,
            artist: None,
            album: None,
            format: None,
            output_id: Some(output_id.to_string()),
            bitrate_kbps: None,
            underrun_frames: None,
            underrun_events: None,
            buffer_size_frames: None,
            buffered_frames: None,
            buffer_capacity_frames: None,
            has_previous: None,
        }
    }
}

#[async_trait]
impl OutputProvider for CastProvider {
    fn list_providers(&self, _state: &AppState) -> Vec<ProviderInfo> {
        vec![ProviderInfo {
            id: Self::provider_id().to_string(),
            kind: "cast".to_string(),
            name: "Chromecast".to_string(),
            state: "available".to_string(),
            capabilities: OutputCapabilities {
                device_select: false,
                volume: false,
            },
        }]
    }

    async fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, ProviderError> {
        if provider_id != Self::provider_id() {
            return Err(ProviderError::BadRequest("unknown provider id".to_string()));
        }
        let outputs = self.list_outputs(state).await;
        let active_id = Self::active_output_id(state).filter(|id| id.starts_with("cast:"));
        Ok(OutputsResponse { active_id, outputs })
    }

    async fn list_outputs(&self, state: &AppState) -> Vec<OutputInfo> {
        let active_id = Self::active_output_id(state);
        let snapshot = state.providers.cast.discovered.lock().ok();
        snapshot
            .map(|map| {
                map.values()
                    .map(|device| Self::device_output_info(device, &active_id))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn can_handle_output_id(&self, output_id: &str) -> bool {
        output_id.starts_with("cast:")
    }

    fn can_handle_provider_id(&self, _state: &AppState, provider_id: &str) -> bool {
        provider_id == Self::provider_id()
    }

    fn inject_active_output_if_missing(
        &self,
        _state: &AppState,
        _outputs: &mut Vec<OutputInfo>,
        _active_output_id: &str,
    ) {
    }

    async fn ensure_active_connected(&self, state: &AppState) -> Result<(), ProviderError> {
        let active_id = Self::active_output_id(state)
            .ok_or_else(|| ProviderError::Unavailable("no active output selected".to_string()))?;
        let Some(device_id) = Self::parse_output_id(&active_id) else {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        };
        let found = state
            .providers
            .cast
            .discovered
            .lock()
            .ok()
            .and_then(|map| map.get(&device_id).cloned());
        if found.is_some() {
            Ok(())
        } else {
            Err(ProviderError::Unavailable("cast device offline".to_string()))
        }
    }

    async fn select_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError> {
        let cmd_tx = Self::ensure_worker_for_output(state, output_id)?;

        {
            let player = state.providers.bridge.player.lock().unwrap();
            let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
        }
        let resume_info = {
            let status = state.playback.manager.status().inner().lock().unwrap();
            (status.now_playing.clone(), status.elapsed_ms, status.paused)
        };
        {
            let mut player = state.providers.bridge.player.lock().unwrap();
            player.cmd_tx = cmd_tx.clone();
        }
        {
            let mut bridges = state.providers.bridge.bridges.lock().unwrap();
            bridges.active_output_id = Some(output_id.to_string());
            bridges.active_bridge_id = None;
        }

        if let (Some(path), Some(elapsed_ms)) = (resume_info.0, resume_info.1) {
            let ext_hint = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let start_paused = resume_info.2;
            let _ = state.providers.bridge.player.lock().unwrap().cmd_tx.send(
                crate::bridge::BridgeCommand::Play {
                    path,
                    ext_hint,
                    seek_ms: Some(elapsed_ms),
                    start_paused,
                },
            );
        }
        Ok(())
    }

    async fn status_for_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<StatusResponse, ProviderError> {
        let Some(device_id) = Self::parse_output_id(output_id) else {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        };
        let found = state
            .providers
            .cast
            .discovered
            .lock()
            .ok()
            .and_then(|map| map.get(&device_id).cloned());
        let Some(found) = found else {
            return Ok(Self::idle_status(output_id, None, false));
        };
        let device_name = Some(found.name.clone());
        if let Some(remote) = state
            .providers
            .cast
            .status_by_output
            .lock()
            .ok()
            .and_then(|map| map.get(output_id).cloned())
        {
            return Ok(status_from_remote(state, output_id, remote));
        }
        Ok(Self::idle_status(output_id, device_name, true))
    }

    async fn stop_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError> {
        if Self::parse_output_id(output_id).is_none() {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        }
        if let Some(tx) = state
            .providers
            .cast
            .workers
            .lock()
            .ok()
            .and_then(|map| map.get(output_id).cloned())
        {
            let _ = tx.send(crate::bridge::BridgeCommand::Stop);
            return Ok(());
        }
        if let Ok(player) = state.providers.bridge.player.lock() {
            let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Stop);
        }
        Ok(())
    }
}

fn estimate_bitrate_kbps(path: &std::path::PathBuf, duration_ms: u64) -> Option<u32> {
    if duration_ms == 0 {
        return None;
    }
    let size = std::fs::metadata(path).ok()?.len();
    if size == 0 {
        return None;
    }
    let bits = size.saturating_mul(8);
    let kbps = bits
        .saturating_mul(1000)
        .saturating_div(duration_ms)
        .saturating_div(1000);
    u32::try_from(kbps).ok()
}

fn container_from_path(path: &std::path::PathBuf) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "flac" => Some("FLAC"),
        "mp3" => Some("MP3"),
        "aac" => Some("AAC"),
        "m4a" => Some("MP4"),
        "ogg" => Some("OGG"),
        "opus" => Some("OPUS"),
        "wav" => Some("WAV"),
        _ => None,
    }
}

fn status_from_remote(
    state: &AppState,
    output_id: &str,
    remote: audio_bridge_types::BridgeStatus,
) -> StatusResponse {
    let now_playing_path = remote.now_playing.as_deref().map(std::path::PathBuf::from);
    let (title, artist, album, format, sample_rate) = match now_playing_path.as_ref() {
        Some(path) => {
            let lib = state.library.read().unwrap();
            match lib.find_track_by_path(path) {
                Some(crate::models::LibraryEntry::Track {
                    file_name,
                    sample_rate,
                    artist,
                    album,
                    format,
                    ..
                }) => {
                    let path_str = path.to_string_lossy();
                    let title = state
                        .metadata
                        .db
                        .track_record_by_path(path_str.as_ref())
                        .ok()
                        .flatten()
                        .and_then(|record| record.title)
                        .or_else(|| Some(file_name));
                    (title, artist, album, Some(format), sample_rate)
                }
                _ => (None, None, None, None, None),
            }
        }
        None => (None, None, None, None, None),
    };
    let container = remote
        .container
        .clone()
        .or_else(|| now_playing_path.as_ref().and_then(container_from_path).map(|s| s.to_string()));
    let source_codec = remote.source_codec.clone().or_else(|| format.clone());
    let bitrate_kbps = remote
        .duration_ms
        .and_then(|duration_ms| now_playing_path.as_ref().and_then(|p| estimate_bitrate_kbps(p, duration_ms)));
    StatusResponse {
        now_playing: remote.now_playing,
        paused: remote.paused,
        bridge_online: true,
        elapsed_ms: remote.elapsed_ms,
        duration_ms: remote.duration_ms,
        source_codec,
        source_bit_depth: remote.source_bit_depth,
        container,
        output_sample_format: remote.output_sample_format,
        resampling: remote.resampling,
        resample_from_hz: remote.resample_from_hz,
        resample_to_hz: remote.resample_to_hz,
        sample_rate,
        channels: remote.channels,
        output_sample_rate: remote.sample_rate,
        output_device: remote.device,
        title,
        artist,
        album,
        format,
        output_id: Some(output_id.to_string()),
        bitrate_kbps,
        underrun_frames: remote.underrun_frames,
        underrun_events: remote.underrun_events,
        buffer_size_frames: remote.buffer_size_frames,
        buffered_frames: remote.buffered_frames,
        buffer_capacity_frames: remote.buffer_capacity_frames,
        has_previous: None,
    }
}
