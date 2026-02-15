//! Local output provider implementation.
//!
//! Exposes local audio devices via the shared `audio-player` device helpers.

use async_trait::async_trait;

use audio_player::device;

use crate::models::{OutputCapabilities, OutputInfo, OutputsResponse, ProviderInfo, StatusResponse, SupportedRates};
use crate::output_providers::registry::{OutputProvider, ProviderError};
use crate::state::AppState;

pub(crate) struct LocalProvider;

impl LocalProvider {
    /// Build the provider id for local outputs.
    fn provider_id(state: &AppState) -> String {
        format!("local:{}", state.providers.local.id)
    }

    /// Build an output id for a local device.
    fn output_id(state: &AppState, device_id: &str) -> String {
        format!("local:{}:{}", state.providers.local.id, device_id)
    }

    /// Return true if local outputs are enabled.
    fn is_enabled(state: &AppState) -> bool {
        state.providers.local.enabled
    }

    fn parse_output_id(output_id: &str) -> Option<String> {
        let mut parts = output_id.splitn(3, ':');
        let kind = parts.next().unwrap_or("");
        let local_id = parts.next().unwrap_or("");
        let device_id = parts.next().unwrap_or("");
        if kind != "local" || local_id.is_empty() || device_id.is_empty() {
            return None;
        }
        Some(device_id.to_string())
    }

    fn short_device_id(id: &str) -> String {
        const MAX_LEN: usize = 48;
        if id.len() <= MAX_LEN {
            return id.to_string();
        }
        let head = &id[..32];
        let tail = &id[id.len().saturating_sub(12)..];
        format!("{head}...{tail}")
    }
}

#[async_trait]
impl OutputProvider for LocalProvider {
    fn list_providers(&self, state: &AppState) -> Vec<ProviderInfo> {
        if !Self::is_enabled(state) {
            return Vec::new();
        }
        vec![ProviderInfo {
            id: Self::provider_id(state),
            kind: "local".to_string(),
            name: state.providers.local.name.clone(),
            state: "available".to_string(),
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        }]
    }

    /// List outputs exposed by the local provider.
    async fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, ProviderError> {
        if !Self::is_enabled(state) {
            return Err(ProviderError::BadRequest("local outputs disabled".to_string()));
        }
        if provider_id != Self::provider_id(state) {
            return Err(ProviderError::BadRequest("unknown provider id".to_string()));
        }
        let mut outputs = self.list_outputs(state).await;
        if let Some(active_id) = state.providers.bridge.bridges.lock().unwrap().active_output_id.clone() {
            if !outputs.iter().any(|o| o.id == active_id) {
                self.inject_active_output_if_missing(state, &mut outputs, &active_id);
            }
        }
        Ok(OutputsResponse {
            active_id: state.providers.bridge.bridges.lock().unwrap().active_output_id.clone(),
            outputs,
        })
    }

    async fn list_outputs(&self, state: &AppState) -> Vec<OutputInfo> {
        if !Self::is_enabled(state) {
            return Vec::new();
        }
        let host = cpal::default_host();
        let devices = match device::list_device_infos(&host) {
            Ok(list) => list,
            Err(_) => return Vec::new(),
        };
        let mut name_counts = std::collections::HashMap::<String, usize>::new();
        for dev in &devices {
            *name_counts.entry(dev.name.clone()).or_insert(0) += 1;
        }
        devices
            .into_iter()
            .filter_map(|dev| {
                let supported_rates = normalize_supported_rates(dev.min_rate, dev.max_rate)?;
                let mut name = dev.name;
                if name_counts.get(&name).copied().unwrap_or(0) > 1 {
                    let suffix = Self::short_device_id(&dev.id);
                    name = format!("{name} ({suffix})");
                }
                Some(OutputInfo {
                    id: Self::output_id(state, &dev.id),
                    kind: "local".to_string(),
                    name,
                    state: "online".to_string(),
                    provider_id: Some(Self::provider_id(state)),
                    provider_name: Some(state.providers.local.name.clone()),
                    supported_rates: Some(supported_rates),
                    capabilities: OutputCapabilities {
                        device_select: true,
                        volume: false,
                    },
                })
            })
            .collect()
    }

    fn can_handle_output_id(&self, output_id: &str) -> bool {
        output_id.starts_with("local:")
    }

    fn can_handle_provider_id(&self, state: &AppState, provider_id: &str) -> bool {
        Self::is_enabled(state) && provider_id == Self::provider_id(state)
    }

    fn inject_active_output_if_missing(
        &self,
        state: &AppState,
        outputs: &mut Vec<OutputInfo>,
        active_output_id: &str,
    ) {
        if !Self::is_enabled(state) {
            return;
        }
        let Some(device_id) = Self::parse_output_id(active_output_id) else {
            return;
        };
        if outputs.iter().any(|o| o.id == active_output_id) {
            return;
        }
        let status = state.playback.manager.status().inner().lock().ok();
        let device_name = status
            .as_ref()
            .and_then(|s| s.output_device.clone())
            .unwrap_or_else(|| format!("active device ({device_id})"));
        let suffix = Self::short_device_id(&device_id);
        let name = format!("{device_name} ({suffix})");
        let supported_rates = status
            .as_ref()
            .and_then(|s| s.sample_rate)
            .map(|sr| SupportedRates { min_hz: sr, max_hz: sr });
        outputs.push(OutputInfo {
            id: active_output_id.to_string(),
            kind: "local".to_string(),
            name,
            state: "active".to_string(),
            provider_id: Some(Self::provider_id(state)),
            provider_name: Some(state.providers.local.name.clone()),
            supported_rates,
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        });
    }

    /// Ensure the local player is running before serving requests.
    async fn ensure_active_connected(&self, state: &AppState) -> Result<(), ProviderError> {
        ensure_local_player(state).await
    }

    /// Select a local output device and resume playback if needed.
    async fn select_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError> {
        if !Self::is_enabled(state) {
            return Err(ProviderError::Unavailable("local outputs disabled".to_string()));
        }
        let Some(device_id) = Self::parse_output_id(output_id) else {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        };
        {
            let player = state.providers.bridge.player.lock().unwrap();
            let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
        }
        let host = cpal::default_host();
        let devices = device::list_device_infos(&host)
            .map_err(|e| ProviderError::Internal(format!("{e:#}")))?;
        let device_name = devices
            .iter()
            .find(|d| d.id == device_id)
            .map(|d| d.name.clone())
            .ok_or_else(|| ProviderError::BadRequest("unknown device".to_string()))?;
        if let Ok(mut g) = state.playback.device_selection.local.lock() {
            *g = Some(device_name);
        }
        {
            let mut bridges = state.providers.bridge.bridges.lock().unwrap();
            bridges.active_output_id = Some(output_id.to_string());
            bridges.active_bridge_id = None;
        }
        ensure_local_player(state).await?;
        Ok(())
    }

    /// Return playback status for the active local output.
    async fn status_for_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<StatusResponse, ProviderError> {
        if !Self::is_enabled(state) {
            return Err(ProviderError::Unavailable("local outputs disabled".to_string()));
        }
        if Self::parse_output_id(output_id).is_none() {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        }
        let active_output_id = state.providers.bridge.bridges.lock().unwrap().active_output_id.clone();
        if active_output_id.as_deref() != Some(output_id) {
            return Err(ProviderError::BadRequest("output is not active".to_string()));
        }
        ensure_local_player(state).await?;

            let status = state.playback.manager.status().inner().lock().unwrap();
            let (title, artist, album, format, sample_rate, bitrate_kbps) =
                match status.now_playing.as_ref() {
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
                                let bitrate_kbps =
                                    estimate_bitrate_kbps(path, status.duration_ms);
                                (Some(file_name), artist, album, Some(format), sample_rate, bitrate_kbps)
                            }
                            _ => (None, None, None, None, None, None),
                        }
                    }
                    None => (None, None, None, None, None, None),
                };
            let resp = StatusResponse {
                now_playing: status.now_playing.as_ref().map(|p| p.to_string_lossy().to_string()),
                paused: status.paused,
                bridge_online: true,
                elapsed_ms: status.elapsed_ms,
                duration_ms: status.duration_ms,
                source_codec: status.source_codec.clone(),
                source_bit_depth: status.source_bit_depth,
                container: status.container.clone(),
                output_sample_format: status.output_sample_format.clone(),
                resampling: status.resampling,
                resample_from_hz: status.resample_from_hz,
                resample_to_hz: status.resample_to_hz,
                sample_rate,
                channels: status.channels,
                output_sample_rate: status.sample_rate,
                output_device: status.output_device.clone(),
                title,
                artist,
                album,
                format,
                output_id: Some(output_id.to_string()),
                bitrate_kbps,
                underrun_frames: None,
                underrun_events: None,
                buffer_size_frames: status.buffer_size_frames,
                buffered_frames: status.buffered_frames,
                buffer_capacity_frames: status.buffer_capacity_frames,
                has_previous: status.has_previous,
            };
            drop(status);
        Ok(resp)
    }

    /// Stop playback on a local output (best-effort).
    async fn stop_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError> {
        if !Self::is_enabled(state) {
            return Ok(());
        }
        if Self::parse_output_id(output_id).is_none() {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        }
        if let Ok(player) = state.providers.local.player.lock() {
            let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Stop);
        }
        Ok(())
    }
}

fn normalize_supported_rates(min_hz: u32, max_hz: u32) -> Option<SupportedRates> {
    if min_hz == 0 || max_hz == 0 || max_hz < min_hz || max_hz == u32::MAX {
        return None;
    }
    Some(SupportedRates { min_hz, max_hz })
}

fn estimate_bitrate_kbps(path: &std::path::PathBuf, duration_ms: Option<u64>) -> Option<u32> {
    let duration_ms = duration_ms?;
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

/// Ensure the local playback worker has been spawned.
async fn ensure_local_player(state: &AppState) -> Result<(), ProviderError> {
    if !LocalProvider::is_enabled(state) {
        return Err(ProviderError::Unavailable("local outputs disabled".to_string()));
    }
    if !state.providers.local
        .running
        .load(std::sync::atomic::Ordering::Relaxed)
    {
            let handle = crate::local_player::spawn_local_player(
                state.playback.device_selection.local.clone(),
                state.playback.manager.status().clone(),
                audio_player::config::PlaybackConfig::default(),
            );
        state.providers.bridge.player.lock().unwrap().cmd_tx = handle.cmd_tx.clone();
        state.providers.local.player.lock().unwrap().cmd_tx = handle.cmd_tx;
        state.providers.local
            .running
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn parse_output_id_accepts_valid() {
        let id = LocalProvider::parse_output_id("local:host:device").unwrap();
        assert_eq!(id, "device");
    }

    #[test]
    fn parse_output_id_rejects_invalid() {
        assert!(LocalProvider::parse_output_id("bridge:host:device").is_none());
        assert!(LocalProvider::parse_output_id("local::device").is_none());
        assert!(LocalProvider::parse_output_id("local:host").is_none());
    }

    #[test]
    fn short_device_id_truncates_long_ids() {
        let long = "b".repeat(80);
        let shortened = LocalProvider::short_device_id(&long);
        assert!(shortened.len() < long.len());
        assert!(shortened.contains("..."));
    }

    #[test]
    fn normalize_supported_rates_rejects_invalid() {
        assert!(normalize_supported_rates(0, 48_000).is_none());
        assert!(normalize_supported_rates(48_000, 0).is_none());
        assert!(normalize_supported_rates(48_000, 44_100).is_none());
        assert!(normalize_supported_rates(1, u32::MAX).is_none());
    }

    #[test]
    fn normalize_supported_rates_accepts_valid() {
        let rates = normalize_supported_rates(44_100, 96_000).unwrap();
        assert_eq!(rates.min_hz, 44_100);
        assert_eq!(rates.max_hz, 96_000);
    }

    #[test]
    fn estimate_bitrate_kbps_returns_none_for_zero_duration() {
        let root = std::env::temp_dir().join(format!(
            "audio-hub-local-bitrate-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let file = root.join("track.flac");
        let _ = std::fs::write(&file, vec![0u8; 1000]);
        assert!(estimate_bitrate_kbps(&file, Some(0)).is_none());
    }

    fn make_state(active_output_id: Option<String>, enabled: bool) -> AppState {
        let root = std::env::temp_dir().join(format!(
            "audio-hub-local-state-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let library = crate::library::scan_library(&root).expect("scan library");
        let (cmd_tx, _cmd_rx) = crossbeam_channel::unbounded();
        let bridges_state = Arc::new(Mutex::new(crate::state::BridgeState {
            bridges: Vec::new(),
            active_bridge_id: None,
            active_output_id,
        }));
        let bridge_state = Arc::new(crate::state::BridgeProviderState::new(
            cmd_tx,
            bridges_state,
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new(std::collections::HashMap::new())),
            "http://localhost".to_string(),
        ));
        let (local_cmd_tx, _local_cmd_rx) = crossbeam_channel::unbounded();
        let local_state = Arc::new(crate::state::LocalProviderState {
            enabled,
            id: "local".to_string(),
            name: "Local Host".to_string(),
            player: Arc::new(Mutex::new(crate::bridge::BridgePlayer {
                cmd_tx: local_cmd_tx,
            })),
            running: Arc::new(AtomicBool::new(false)),
        });
        let status = crate::status_store::StatusStore::new(
            Arc::new(Mutex::new(crate::state::PlayerStatus::default())),
            crate::events::EventBus::new(),
        );
        let queue = Arc::new(Mutex::new(crate::state::QueueState::default()));
        let queue_service = crate::queue_service::QueueService::new(
            queue,
            status.clone(),
            crate::events::EventBus::new(),
        );
        let playback_manager = crate::playback_manager::PlaybackManager::new(
            bridge_state.player.clone(),
            status,
            queue_service,
        );
        let device_selection = crate::state::DeviceSelectionState {
            local: Arc::new(Mutex::new(None)),
            bridge: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        let metadata_db = crate::metadata_db::MetadataDb::new(library.root()).unwrap();
        let browser_state = Arc::new(crate::browser::BrowserProviderState::new());
        AppState::new(
            library,
            metadata_db,
            None,
            crate::state::MetadataWake::new(),
            bridge_state,
            local_state,
            browser_state,
            playback_manager,
            device_selection,
            crate::events::EventBus::new(),
            Arc::new(crate::events::LogBus::new(64)),
        )
    }

    #[test]
    fn inject_active_output_if_missing_adds_placeholder() {
        let active_id = "local:local:device-1".to_string();
        let state = make_state(Some(active_id.clone()), true);
        if let Ok(mut status) = state.playback.manager.status().inner().lock() {
            status.output_device = Some("USB DAC".to_string());
            status.sample_rate = Some(96_000);
        }
        let provider = LocalProvider;
        let mut outputs = Vec::new();

        provider.inject_active_output_if_missing(&state, &mut outputs, &active_id);

        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].id, active_id);
        assert!(outputs[0].name.contains("USB DAC"));
        assert_eq!(outputs[0].supported_rates.as_ref().unwrap().max_hz, 96_000);
    }

    #[test]
    fn inject_active_output_if_missing_skips_when_present() {
        let active_id = "local:local:device-1".to_string();
        let state = make_state(Some(active_id.clone()), true);
        let provider = LocalProvider;
        let mut outputs = vec![OutputInfo {
            id: active_id.clone(),
            kind: "local".to_string(),
            name: "Device".to_string(),
            state: "online".to_string(),
            provider_id: Some("local:local".to_string()),
            provider_name: Some("Local Host".to_string()),
            supported_rates: None,
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        }];

        provider.inject_active_output_if_missing(&state, &mut outputs, &active_id);

        assert_eq!(outputs.len(), 1);
    }
}
