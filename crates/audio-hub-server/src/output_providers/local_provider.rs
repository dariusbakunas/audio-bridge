use actix_web::HttpResponse;

use audio_player::device;

use crate::models::{OutputCapabilities, OutputInfo, OutputsResponse, ProviderInfo, StatusResponse, SupportedRates};
use crate::output_providers::registry::OutputProvider;
use crate::state::AppState;

pub(crate) struct LocalProvider;

impl LocalProvider {
    fn provider_id(state: &AppState) -> String {
        format!("local:{}", state.local.id)
    }

    fn output_id(state: &AppState, device_id: &str) -> String {
        format!("local:{}:{}", state.local.id, device_id)
    }

    fn is_enabled(state: &AppState) -> bool {
        state.local.enabled
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

impl OutputProvider for LocalProvider {
    fn list_providers(&self, state: &AppState) -> Vec<ProviderInfo> {
        if !Self::is_enabled(state) {
            return Vec::new();
        }
        vec![ProviderInfo {
            id: Self::provider_id(state),
            kind: "local".to_string(),
            name: state.local.name.clone(),
            state: "available".to_string(),
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        }]
    }

    fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, HttpResponse> {
        if !Self::is_enabled(state) {
            return Err(HttpResponse::BadRequest().body("local outputs disabled"));
        }
        if provider_id != Self::provider_id(state) {
            return Err(HttpResponse::BadRequest().body("unknown provider id"));
        }
        let mut outputs = self.list_outputs(state);
        if let Some(active_id) = state.bridge.bridges.lock().unwrap().active_output_id.clone() {
            if !outputs.iter().any(|o| o.id == active_id) {
                self.inject_active_output_if_missing(state, &mut outputs, &active_id);
            }
        }
        Ok(OutputsResponse {
            active_id: state.bridge.bridges.lock().unwrap().active_output_id.clone(),
            outputs,
        })
    }

    fn list_outputs(&self, state: &AppState) -> Vec<OutputInfo> {
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
                    provider_name: Some(state.local.name.clone()),
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
        let status = state.playback.status.inner().lock().ok();
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
            provider_name: Some(state.local.name.clone()),
            supported_rates,
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        });
    }

    fn ensure_active_connected<'a>(
        &'a self,
        state: &'a AppState,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), HttpResponse>> + Send + 'a>,
    > {
        Box::pin(async move { ensure_local_player(state).await })
    }

    fn select_output<'a>(
        &'a self,
        state: &'a AppState,
        output_id: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), HttpResponse>> + Send + 'a>,
    > {
        Box::pin(async move {
            if !Self::is_enabled(state) {
                return Err(HttpResponse::ServiceUnavailable().body("local outputs disabled"));
            }
            let Some(device_id) = Self::parse_output_id(output_id) else {
                return Err(HttpResponse::BadRequest().body("invalid output id"));
            };
            {
                let player = state.bridge.player.lock().unwrap();
                let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
            }
            let host = cpal::default_host();
            let devices = device::list_device_infos(&host)
                .map_err(|e| HttpResponse::InternalServerError().body(format!("{e:#}")))?;
            let device_name = devices
                .iter()
                .find(|d| d.id == device_id)
                .map(|d| d.name.clone())
                .ok_or_else(|| HttpResponse::BadRequest().body("unknown device"))?;
            if let Ok(mut g) = state.local.device_selected.lock() {
                *g = Some(device_name);
            }
            {
                let mut bridges = state.bridge.bridges.lock().unwrap();
                bridges.active_output_id = Some(output_id.to_string());
                bridges.active_bridge_id = None;
            }
            ensure_local_player(state).await?;
            Ok(())
        })
    }

    fn status_for_output<'a>(
        &'a self,
        state: &'a AppState,
        output_id: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<StatusResponse, HttpResponse>> + Send + 'a>,
    > {
        Box::pin(async move {
            if !Self::is_enabled(state) {
                return Err(HttpResponse::ServiceUnavailable().body("local outputs disabled"));
            }
            if Self::parse_output_id(output_id).is_none() {
                return Err(HttpResponse::BadRequest().body("invalid output id"));
            }
            let active_output_id = state.bridge.bridges.lock().unwrap().active_output_id.clone();
            if active_output_id.as_deref() != Some(output_id) {
                return Err(HttpResponse::BadRequest().body("output is not active"));
            }
            ensure_local_player(state).await?;

            let status = state.playback.status.inner().lock().unwrap();
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
                buffer_size_frames: None,
            };
            drop(status);
            Ok(resp)
        })
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

async fn ensure_local_player(state: &AppState) -> Result<(), HttpResponse> {
    if !LocalProvider::is_enabled(state) {
        return Err(HttpResponse::ServiceUnavailable().body("local outputs disabled"));
    }
    if !state
        .local
        .running
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        let handle = crate::local_player::spawn_local_player(
            state.local.device_selected.clone(),
            state.playback.status.clone(),
            audio_player::config::PlaybackConfig::default(),
        );
        state.bridge.player.lock().unwrap().cmd_tx = handle.cmd_tx.clone();
        state.local.player.lock().unwrap().cmd_tx = handle.cmd_tx;
        state
            .local
            .running
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}
