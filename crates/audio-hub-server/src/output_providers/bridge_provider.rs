use std::path::PathBuf;
use async_trait::async_trait;

use crate::bridge_transport::BridgeTransportClient;
use crate::bridge_manager::{merge_bridges, parse_output_id, parse_provider_id};
use crate::models::{
    OutputCapabilities,
    OutputInfo,
    OutputsResponse,
    ProviderInfo,
    StatusResponse,
    SupportedRates,
};
use crate::output_providers::registry::{OutputProvider, ProviderError};
use crate::state::AppState;

pub(crate) struct BridgeProvider;

impl BridgeProvider {
    async fn ensure_active_connected(state: &AppState) -> Result<(), ProviderError> {
        if state
            .bridge
            .bridge_online
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Ok(());
        }

        let (bridge_id, addr) = {
            let bridges_state = state.bridge.bridges.lock().unwrap();
            let discovered = state.bridge.discovered_bridges.lock().unwrap();
            let merged = merge_bridges(&bridges_state.bridges, &discovered);
            let Some(active_bridge_id) = bridges_state.active_bridge_id.as_ref() else {
                return Err(ProviderError::Unavailable("no active output selected".to_string()));
            };
            let Some(bridge) = merged.iter().find(|b| b.id == *active_bridge_id) else {
                return Err(ProviderError::Unavailable("active bridge not found".to_string()));
            };
            (bridge.id.clone(), bridge.http_addr)
        };

        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        {
            let mut player = state.bridge.player.lock().unwrap();
            let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
            player.cmd_tx = cmd_tx.clone();
        }
        crate::bridge::spawn_bridge_worker(
            bridge_id,
            addr,
            cmd_rx,
            cmd_tx,
            state.playback_manager.status().clone(),
            state.playback_manager.queue_service().queue().clone(),
            state.bridge.bridge_online.clone(),
            state.bridge.bridges.clone(),
            state.bridge.public_base_url.clone(),
        );

        let mut waited = 0u64;
        while waited < 2000
            && !state
                .bridge
                .bridge_online
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            actix_web::rt::time::sleep(std::time::Duration::from_millis(100)).await;
            waited += 100;
        }
        if !state
            .bridge
            .bridge_online
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(ProviderError::Unavailable("bridge offline".to_string()));
        }
        Ok(())
    }

    fn list_outputs_internal(state: &AppState) -> Vec<OutputInfo> {
        let bridges_state = state.bridge.bridges.lock().unwrap();
        let discovered = state.bridge.discovered_bridges.lock().unwrap();
        let merged = merge_bridges(&bridges_state.bridges, &discovered);
        let (outputs, failed) = build_outputs_from_bridges_with_failures(&merged);
        drop(bridges_state);
        drop(discovered);
        if !failed.is_empty() {
            if let Ok(mut map) = state.bridge.discovered_bridges.lock() {
                let configured_ids: std::collections::HashSet<String> = state
                    .bridge
                    .bridges
                    .lock()
                    .unwrap()
                    .bridges
                    .iter()
                    .map(|b| b.id.clone())
                    .collect();
                for id in failed {
                    if !configured_ids.contains(&id) {
                        map.remove(&id);
                        tracing::info!(
                            bridge_id = %id,
                            "outputs: removed discovered bridge after device list failure"
                        );
                    }
                }
            }
        }
        outputs
    }
}

#[async_trait]
impl OutputProvider for BridgeProvider {
    fn list_providers(&self, state: &AppState) -> Vec<ProviderInfo> {
        let bridges_state = state.bridge.bridges.lock().unwrap();
        let discovered = state.bridge.discovered_bridges.lock().unwrap();
        let active_online = state
            .bridge
            .bridge_online
            .load(std::sync::atomic::Ordering::Relaxed);
        let merged = merge_bridges(&bridges_state.bridges, &discovered);
        merged
            .iter()
            .map(|b| ProviderInfo {
                id: format!("bridge:{}", b.id),
                kind: "bridge".to_string(),
                name: b.name.clone(),
                state: if bridges_state.active_bridge_id.as_deref() == Some(b.id.as_str()) {
                    if active_online {
                        "connected".to_string()
                    } else {
                        "idle".to_string()
                    }
                } else if bridges_state.bridges.iter().any(|c| c.id == b.id) {
                    "configured".to_string()
                } else {
                    "discovered".to_string()
                },
                capabilities: OutputCapabilities {
                    device_select: true,
                    volume: false,
                },
            })
            .collect()
    }

    async fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, ProviderError> {
        let bridge_id = parse_provider_id(provider_id)
            .map_err(|e| ProviderError::BadRequest(e))?;
        let (bridge, active_output_id) = {
            let bridges_state = state.bridge.bridges.lock().unwrap();
            let discovered = state.bridge.discovered_bridges.lock().unwrap();
            let merged = merge_bridges(&bridges_state.bridges, &discovered);
            let Some(bridge) = merged.iter().find(|b| b.id == bridge_id) else {
                return Err(ProviderError::BadRequest("unknown provider id".to_string()));
            };
            (bridge.clone(), bridges_state.active_output_id.clone())
        };

        let mut outputs = build_outputs_for_bridge(&bridge)
            .map_err(|e| ProviderError::Internal(format!("{e:#}")))?;
        inject_active_output_for_bridge(&mut outputs, active_output_id.as_deref(), &bridge);

        Ok(OutputsResponse {
            active_id: active_output_id,
            outputs,
        })
    }

    fn list_outputs(&self, state: &AppState) -> Vec<OutputInfo> {
        Self::list_outputs_internal(state)
    }

    fn can_handle_output_id(&self, output_id: &str) -> bool {
        parse_output_id(output_id).is_ok()
    }

    fn can_handle_provider_id(&self, state: &AppState, provider_id: &str) -> bool {
        let bridges_state = state.bridge.bridges.lock().unwrap();
        let discovered = state.bridge.discovered_bridges.lock().unwrap();
        let merged = merge_bridges(&bridges_state.bridges, &discovered);
        parse_provider_id(provider_id)
            .ok()
            .map(|id| merged.iter().any(|b| b.id == id))
            .unwrap_or(false)
    }

    fn inject_active_output_if_missing(
        &self,
        state: &AppState,
        outputs: &mut Vec<OutputInfo>,
        active_output_id: &str,
    ) {
        let Ok((bridge_id, _)) = parse_output_id(active_output_id) else {
            return;
        };
        let bridges_state = state.bridge.bridges.lock().unwrap();
        let discovered = state.bridge.discovered_bridges.lock().unwrap();
        let merged = merge_bridges(&bridges_state.bridges, &discovered);
        if let Some(bridge) = merged.iter().find(|b| b.id == bridge_id) {
            inject_active_output_for_bridge(outputs, Some(active_output_id), bridge);
        }
    }

    async fn ensure_active_connected(&self, state: &AppState) -> Result<(), ProviderError> {
        Self::ensure_active_connected(state).await
    }

    async fn select_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError> {
        let (bridge_id, device_id) = parse_output_id(output_id)
            .map_err(|e| ProviderError::BadRequest(e))?;
        let http_addr = {
            let bridges_state = state.bridge.bridges.lock().unwrap();
            let discovered = state.bridge.discovered_bridges.lock().unwrap();
            let merged = merge_bridges(&bridges_state.bridges, &discovered);
            let Some(bridge) = merged.iter().find(|b| b.id == bridge_id) else {
                return Err(ProviderError::BadRequest("unknown bridge id".to_string()));
            };
            bridge.http_addr
        };

        let device_name = match BridgeTransportClient::new(http_addr, String::new()).list_devices() {
            Ok(devices) => {
                if let Some(device) = devices.iter().find(|d| d.id == device_id) {
                    device.name.clone()
                } else if let Some(device) = devices.iter().find(|d| d.name == device_id) {
                    device.name.clone()
                } else {
                    tracing::warn!(
                        bridge_id = %bridge_id,
                        device_id = %device_id,
                        "output select rejected: unknown device"
                    );
                    return Err(ProviderError::BadRequest("unknown device".to_string()));
                }
            }
            Err(e) => {
                tracing::warn!(
                    bridge_id = %bridge_id,
                    error = %e,
                    "output select failed: device list"
                );
                return Err(ProviderError::Internal(format!("{e:#}")));
            }
        };

        let resume_info = {
                let status = state.playback_manager.status().inner().lock().unwrap();
            (status.now_playing.clone(), status.elapsed_ms, status.paused)
        };

        {
            let cmd_tx = state.bridge.player.lock().unwrap().cmd_tx.clone();
            let _ = cmd_tx.send(crate::bridge::BridgeCommand::Stop);
        }

        if let Err(e) = switch_active_bridge(state, &bridge_id, http_addr) {
            tracing::warn!(
                bridge_id = %bridge_id,
                error = %e,
                "output select failed: switch bridge"
            );
            return Err(ProviderError::Internal(format!("{e:#}")));
        }
        if let Err(e) = BridgeTransportClient::new(http_addr, String::new()).set_device(&device_name) {
            tracing::warn!(
                bridge_id = %bridge_id,
                device = %device_name,
                error = %e,
                "output select failed: set device"
            );
            return Err(ProviderError::Internal(format!("{e:#}")));
        }

        {
            let mut bridges = state.bridge.bridges.lock().unwrap();
            bridges.active_bridge_id = Some(bridge_id.clone());
            bridges.active_output_id = Some(output_id.to_string());
            tracing::info!(
                output_id = ?bridges.active_output_id,
                bridge_id = ?bridges.active_bridge_id,
                "output selected"
            );
        }
        if let Ok(mut sel) = state.device_selection.bridge.lock() {
            sel.insert(bridge_id.clone(), device_id.clone());
        }

        Self::ensure_active_connected(state).await?;

        if let (Some(path), Some(elapsed_ms)) = (resume_info.0, resume_info.1) {
            let ext_hint = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let start_paused = resume_info.2;
            let _ = state.bridge.player.lock().unwrap().cmd_tx.send(
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
        if parse_output_id(output_id).is_err() {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        }
        let (active_output_id, http_addr) = {
            let bridges = state.bridge.bridges.lock().unwrap();
            let http_addr = bridges.active_bridge_id.as_ref().and_then(|active_id| {
                bridges
                    .bridges
                    .iter()
                    .find(|b| b.id == *active_id)
                    .map(|b| b.http_addr)
            });
            (bridges.active_output_id.clone(), http_addr)
        };
        if active_output_id.as_deref() != Some(output_id) {
            return Err(ProviderError::BadRequest("output is not active".to_string()));
        }
        Self::ensure_active_connected(state).await?;

        let status = state.playback_manager.status().inner().lock().unwrap();
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
                            let bitrate_kbps = estimate_bitrate_kbps(path, status.duration_ms);
                            (Some(file_name), artist, album, Some(format), sample_rate, bitrate_kbps)
                        }
                        _ => (None, None, None, None, None, None),
                    }
                }
                None => (None, None, None, None, None, None),
            };
        let bridge_online = state
            .bridge
            .bridge_online
            .load(std::sync::atomic::Ordering::Relaxed);
        let mut resp = StatusResponse {
            now_playing: status.now_playing.as_ref().map(|p| p.to_string_lossy().to_string()),
            paused: status.paused,
            bridge_online,
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
        if let Some(http_addr) = http_addr {
            match BridgeTransportClient::new(http_addr, String::new()).status() {
                Ok(remote) => {
                    resp.paused = remote.paused;
                    resp.elapsed_ms = remote.elapsed_ms;
                    resp.duration_ms = remote.duration_ms;
                    resp.source_codec = remote.source_codec;
                    resp.source_bit_depth = remote.source_bit_depth;
                    resp.container = remote.container;
                    resp.output_sample_format = remote.output_sample_format;
                    resp.resampling = remote.resampling;
                    resp.resample_from_hz = remote.resample_from_hz;
                    resp.resample_to_hz = remote.resample_to_hz;
                    resp.channels = remote.channels;
                    resp.output_sample_rate = remote.sample_rate;
                    resp.output_device = remote.device;
                    resp.underrun_frames = remote.underrun_frames;
                    resp.underrun_events = remote.underrun_events;
                    resp.buffer_size_frames = remote.buffer_size_frames;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "bridge status poll failed");
                }
            }
        }
        Ok(resp)
    }
}

fn build_outputs_from_bridges_with_failures(
    bridges: &[crate::config::BridgeConfigResolved],
) -> (Vec<OutputInfo>, Vec<String>) {
    let mut outputs = Vec::new();
    let mut name_counts = std::collections::HashMap::<String, usize>::new();
    let mut by_bridge = Vec::new();
    let mut failed = Vec::new();

    for bridge in bridges {
        let devices = match BridgeTransportClient::new(bridge.http_addr, String::new()).list_devices() {
            Ok(list) => {
                tracing::info!(
                    bridge_id = %bridge.id,
                    bridge_name = %bridge.name,
                    count = list.len(),
                    "outputs: devices listed"
                );
                list
            }
            Err(e) => {
                tracing::warn!(
                    bridge_id = %bridge.id,
                    bridge_name = %bridge.name,
                    error = %e,
                    "outputs: device list failed"
                );
                failed.push(bridge.id.clone());
                Vec::new()
            }
        };
        for device in devices {
            *name_counts.entry(device.name.clone()).or_insert(0) += 1;
            by_bridge.push((bridge, device));
        }
    }

    for (bridge, device) in by_bridge {
        let mut display_name = device.name.clone();
        if name_counts.get(&device.name).copied().unwrap_or(0) > 1 {
            let suffix = short_device_id(&device.id);
            display_name = format!("{display_name} [{}] ({suffix})", bridge.name);
        }
        let supported_rates = normalize_supported_rates(device.min_rate, device.max_rate);
        if supported_rates.is_none() {
            continue;
        }
        outputs.push(OutputInfo {
            id: format!("bridge:{}:{}", bridge.id, device.id),
            kind: "bridge".to_string(),
            name: display_name,
            state: "online".to_string(),
            provider_id: Some(format!("bridge:{}", bridge.id)),
            provider_name: Some(bridge.name.clone()),
            supported_rates,
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        });
    }

    (outputs, failed)
}

fn build_outputs_for_bridge(
    bridge: &crate::config::BridgeConfigResolved,
) -> Result<Vec<OutputInfo>, anyhow::Error> {
    let devices = BridgeTransportClient::new(bridge.http_addr, String::new()).list_devices()?;
    let mut outputs = Vec::new();
    let mut name_counts = std::collections::HashMap::<String, usize>::new();
    for device in &devices {
        *name_counts.entry(device.name.clone()).or_insert(0) += 1;
    }
    for device in devices {
        let mut name = device.name;
        if name_counts.get(&name).copied().unwrap_or(0) > 1 {
            let suffix = short_device_id(&device.id);
            name = format!("{name} ({suffix})");
        }
        let supported_rates = normalize_supported_rates(device.min_rate, device.max_rate);
        if supported_rates.is_none() {
            continue;
        }
        outputs.push(OutputInfo {
            id: format!("bridge:{}:{}", bridge.id, device.id),
            kind: "bridge".to_string(),
            name,
            state: "online".to_string(),
            provider_id: Some(format!("bridge:{}", bridge.id)),
            provider_name: Some(bridge.name.clone()),
            supported_rates,
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        });
    }
    Ok(outputs)
}

fn normalize_supported_rates(min_hz: u32, max_hz: u32) -> Option<SupportedRates> {
    if min_hz == 0 || max_hz == 0 || max_hz < min_hz || max_hz == u32::MAX {
        return None;
    }
    Some(SupportedRates { min_hz, max_hz })
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

fn inject_active_output_for_bridge(
    outputs: &mut Vec<OutputInfo>,
    active_output_id: Option<&str>,
    bridge: &crate::config::BridgeConfigResolved,
) {
    let Some(active_output_id) = active_output_id else { return };
    if outputs.iter().any(|o| o.id == active_output_id) {
        return;
    }
    let Ok((bridge_id, device_id)) = parse_output_id(active_output_id) else {
        return;
    };
    if bridge_id != bridge.id {
        return;
    }
    let status = match BridgeTransportClient::new(bridge.http_addr, String::new()).status() {
        Ok(status) => status,
        Err(_) => return,
    };
    let device_name = status
        .device
        .unwrap_or_else(|| format!("active device ({device_id})"));
    let suffix = short_device_id(&device_id);
    let name = format!("{device_name} ({suffix})");
    let supported_rates = status
        .sample_rate
        .map(|sr| SupportedRates { min_hz: sr, max_hz: sr });
    outputs.push(OutputInfo {
        id: active_output_id.to_string(),
        kind: "bridge".to_string(),
        name,
        state: "active".to_string(),
        provider_id: Some(format!("bridge:{}", bridge.id)),
        provider_name: Some(bridge.name.clone()),
        supported_rates,
        capabilities: OutputCapabilities {
            device_select: true,
            volume: false,
        },
    });
}

fn estimate_bitrate_kbps(path: &PathBuf, duration_ms: Option<u64>) -> Option<u32> {
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

fn switch_active_bridge(
    state: &AppState,
    bridge_id: &str,
    http_addr: std::net::SocketAddr,
) -> Result<(), anyhow::Error> {
    let mut bridges = state.bridge.bridges.lock().unwrap();
    if bridges.active_bridge_id.as_deref() == Some(bridge_id) {
        return Ok(());
    }
    tracing::info!(
        from_bridge_id = ?bridges.active_bridge_id,
        to_bridge_id = %bridge_id,
        http_addr = %http_addr,
        "switch active bridge"
    );
    bridges.active_bridge_id = Some(bridge_id.to_string());
    drop(bridges);

    state
        .bridge
        .bridge_online
        .store(false, std::sync::atomic::Ordering::Relaxed);
    {
        let player = state.bridge.player.lock().unwrap();
        let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
    }
    Ok(())
}
