//! Bridge output provider implementation.
//!
//! Maps output provider operations to bridge discovery + HTTP transport calls.

use std::path::PathBuf;
use std::time::Duration;
use async_trait::async_trait;

use crate::bridge_transport::{BridgeTransportClient, HttpDeviceInfo};
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
    /// Ensure the currently active bridge is reachable before serving requests.
    async fn ensure_active_connected(state: &AppState) -> Result<(), ProviderError> {
        tracing::debug!(
            bridge_online = state.providers.bridge
                .bridge_online
                .load(std::sync::atomic::Ordering::Relaxed),
            "ensure_active_connected called"
        );
        if state.providers.bridge
            .bridge_online
            .load(std::sync::atomic::Ordering::Relaxed)
            && state.providers.bridge
                .worker_running
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Ok(());
        }

        let (bridge_id, addr) = {
            let bridges_state = state.providers.bridge.bridges.lock().unwrap();
            let discovered = state.providers.bridge.discovered_bridges.lock().unwrap();
            let merged = merge_bridges(&bridges_state.bridges, &discovered);
            let Some(active_bridge_id) = bridges_state.active_bridge_id.as_ref() else {
                return Err(ProviderError::Unavailable("no active output selected".to_string()));
            };
            let Some(bridge) = merged.iter().find(|b| b.id == *active_bridge_id) else {
                return Err(ProviderError::Unavailable("active bridge not found".to_string()));
            };
            (bridge.id.clone(), bridge.http_addr)
        };
        if let Ok(status) = BridgeTransportClient::new(addr)
            .status()
            .await
        {
            if let Ok(mut cache) = state.providers.bridge.status_cache.lock() {
                cache.insert(bridge_id.clone(), status);
            }
            if crate::bridge::update_online_and_should_emit(
                &state.providers.bridge.bridge_online,
                true,
            ) {
                state.events.outputs_changed();
            }
            state.events.status_changed();
            if state.providers.bridge
                .worker_running
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Ok(());
            }
        }
        if state
            .providers
            .bridge
            .status_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&bridge_id).cloned())
            .is_some()
        {
            state.providers.bridge
                .bridge_online
                .store(true, std::sync::atomic::Ordering::Relaxed);
            if state.providers.bridge
                .worker_running
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Ok(());
            }
        }

        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        {
            let mut player = state.providers.bridge.player.lock().unwrap();
            let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
            player.cmd_tx = cmd_tx.clone();
        }
        crate::bridge::spawn_bridge_worker(
            bridge_id,
            addr,
            cmd_rx,
            state.playback.manager.status().clone(),
            state.providers.bridge.worker_running.clone(),
            state.providers.bridge.public_base_url.clone(),
            Some(state.metadata.db.clone()),
        );

        let mut waited = 0u64;
        while waited < 2000
            && !state.providers.bridge
                .bridge_online
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            actix_web::rt::time::sleep(std::time::Duration::from_millis(100)).await;
            waited += 100;
        }
        if !state.providers.bridge
            .bridge_online
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(ProviderError::Unavailable("bridge offline".to_string()));
        }
        Ok(())
    }

    async fn list_outputs_internal(state: &AppState) -> Vec<OutputInfo> {
        let (merged, active_bridge_id) = {
            let bridges_state = state.providers.bridge.bridges.lock().unwrap();
            let discovered = state.providers.bridge.discovered_bridges.lock().unwrap();
            let merged = merge_bridges(&bridges_state.bridges, &discovered);
            (merged, bridges_state.active_bridge_id.clone())
        };
        let (outputs, failed) = build_outputs_from_bridges_with_failures(state, &merged).await;
        for id in failed {
            let is_active = active_bridge_id.as_deref() == Some(id.as_str());
            tracing::warn!(
                bridge_id = %id,
                active = is_active,
                "outputs: device list failed; keeping discovered bridge for retry"
            );
        }
        outputs
    }
}

#[async_trait]
impl OutputProvider for BridgeProvider {
    fn list_providers(&self, state: &AppState) -> Vec<ProviderInfo> {
        let bridges_state = state.providers.bridge.bridges.lock().unwrap();
        let discovered = state.providers.bridge.discovered_bridges.lock().unwrap();
        let active_online = state.providers.bridge
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

    /// List outputs exposed by a specific bridge provider.
    async fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, ProviderError> {
        let bridge_id = parse_provider_id(provider_id)
            .map_err(|e| ProviderError::BadRequest(e))?;
        let (bridge, active_output_id) = {
            let bridges_state = state.providers.bridge.bridges.lock().unwrap();
            let discovered = state.providers.bridge.discovered_bridges.lock().unwrap();
            let merged = merge_bridges(&bridges_state.bridges, &discovered);
            let Some(bridge) = merged.iter().find(|b| b.id == bridge_id) else {
                return Err(ProviderError::BadRequest("unknown provider id".to_string()));
            };
            (bridge.clone(), bridges_state.active_output_id.clone())
        };

        let mut outputs = build_outputs_for_bridge(state, &bridge)
            .await
            .map_err(|e| ProviderError::Internal(format!("{e:#}")))?;
        inject_active_output_for_bridge(state, &mut outputs, active_output_id.as_deref(), &bridge);

        Ok(OutputsResponse {
            active_id: active_output_id,
            outputs,
        })
    }

    async fn list_outputs(&self, state: &AppState) -> Vec<OutputInfo> {
        Self::list_outputs_internal(state).await
    }

    fn can_handle_output_id(&self, output_id: &str) -> bool {
        parse_output_id(output_id).is_ok()
    }

    fn can_handle_provider_id(&self, state: &AppState, provider_id: &str) -> bool {
        let bridges_state = state.providers.bridge.bridges.lock().unwrap();
        let discovered = state.providers.bridge.discovered_bridges.lock().unwrap();
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
        let bridges_state = state.providers.bridge.bridges.lock().unwrap();
        let discovered = state.providers.bridge.discovered_bridges.lock().unwrap();
        let merged = merge_bridges(&bridges_state.bridges, &discovered);
        if let Some(bridge) = merged.iter().find(|b| b.id == bridge_id) {
            inject_active_output_for_bridge(state, outputs, Some(active_output_id), bridge);
        }
    }

    /// Ensure the provider has a reachable, active bridge.
    async fn ensure_active_connected(&self, state: &AppState) -> Result<(), ProviderError> {
        Self::ensure_active_connected(state).await
    }

    /// Select the active output for this provider and apply device selection on the bridge.
    async fn select_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError> {
        state.providers.bridge
            .output_switch_in_flight
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut until) = state.providers.bridge.output_switch_until.lock() {
            *until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
        }
        state.playback.manager.set_manual_advance_in_flight(true);
        let prior_active_output_id = state.providers.bridge.bridges.lock().unwrap().active_output_id.clone();
        let (bridge_id, device_id) = parse_output_id(output_id)
            .map_err(|e| ProviderError::BadRequest(e))?;
        let http_addr = {
            let bridges_state = state.providers.bridge.bridges.lock().unwrap();
            let discovered = state.providers.bridge.discovered_bridges.lock().unwrap();
            let merged = merge_bridges(&bridges_state.bridges, &discovered);
            let Some(bridge) = merged.iter().find(|b| b.id == bridge_id) else {
                state.providers.bridge
                    .output_switch_in_flight
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut until) = state.providers.bridge.output_switch_until.lock() {
                    *until = None;
                }
                return Err(ProviderError::BadRequest("unknown bridge id".to_string()));
            };
            bridge.http_addr
        };

        let devices = match list_devices_cached_or_fetch(
            state,
            &bridge_id,
            &bridge_id,
            http_addr,
        )
        .await
        {
            Ok(devices) => devices,
            Err(e) => {
                state.providers.bridge
                    .output_switch_in_flight
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut until) = state.providers.bridge.output_switch_until.lock() {
                    *until = None;
                }
                tracing::warn!(
                    bridge_id = %bridge_id,
                    error = %e,
                    "output select failed: device list"
                );
                return Err(ProviderError::Internal(format!("{e:#}")));
            }
        };
        let device_name = match resolve_device_name(&devices, &device_id) {
            Some(name) => name,
            None => {
                state.providers.bridge
                    .output_switch_in_flight
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut until) = state.providers.bridge.output_switch_until.lock() {
                    *until = None;
                }
                tracing::warn!(
                    bridge_id = %bridge_id,
                    device_id = %device_id,
                    "output select rejected: unknown device"
                );
                return Err(ProviderError::BadRequest("unknown device".to_string()));
            }
        };

        let resume_info = {
            let status = state.playback.manager.status().inner().lock().unwrap();
            (
                status.now_playing.clone(),
                status.elapsed_ms,
                status.paused,
                status.user_paused,
            )
        };

        {
            let cmd_tx = state.providers.bridge.player.lock().unwrap().cmd_tx.clone();
            let _ = cmd_tx.send(crate::bridge::BridgeCommand::Stop);
        }

        if let Err(e) = switch_active_bridge(state, &bridge_id, http_addr) {
            state.providers.bridge
                .output_switch_in_flight
                .store(false, std::sync::atomic::Ordering::Relaxed);
            if let Ok(mut until) = state.providers.bridge.output_switch_until.lock() {
                *until = None;
            }
            tracing::warn!(
                bridge_id = %bridge_id,
                error = %e,
                "output select failed: switch bridge"
            );
            return Err(ProviderError::Internal(format!("{e:#}")));
        }
        if let Err(e) = BridgeTransportClient::new(http_addr)
            .set_device(&device_name)
            .await
        {
            state.providers.bridge
                .output_switch_in_flight
                .store(false, std::sync::atomic::Ordering::Relaxed);
            if let Ok(mut until) = state.providers.bridge.output_switch_until.lock() {
                *until = None;
            }
            tracing::warn!(
                bridge_id = %bridge_id,
                device = %device_name,
                error = %e,
                "output select failed: set device"
            );
            return Err(ProviderError::Internal(format!("{e:#}")));
        }

        {
            let mut bridges = state.providers.bridge.bridges.lock().unwrap();
            bridges.active_bridge_id = Some(bridge_id.clone());
            bridges.active_output_id = Some(output_id.to_string());
            tracing::info!(
                output_id = ?bridges.active_output_id,
                bridge_id = ?bridges.active_bridge_id,
                "output selected"
            );
        }
        if let Ok(mut sel) = state.playback.device_selection.bridge.lock() {
            sel.insert(bridge_id.clone(), device_id.clone());
        }

        Self::ensure_active_connected(state).await?;

        if let Some(path) = resume_info.0 {
            let ext_hint = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let start_paused = start_paused_for_resume(
                prior_active_output_id.as_deref(),
                resume_info.2,
                resume_info.3,
            );
            let _ = state.providers.bridge.player.lock().unwrap().cmd_tx.send(
                crate::bridge::BridgeCommand::Play {
                    path,
                    ext_hint,
                    seek_ms: resume_info.1,
                    start_paused,
                },
            );
        }
        Ok(())
    }

    /// Return playback status for the active bridge output.
    async fn status_for_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<StatusResponse, ProviderError> {
        let (bridge_id, _) =
            parse_output_id(output_id).map_err(|_| ProviderError::BadRequest("invalid output id".to_string()))?;
        let (active_output_id, http_addr) = {
            let bridges = state.providers.bridge.bridges.lock().unwrap();
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
                            let bitrate_kbps = estimate_bitrate_kbps(path, status.duration_ms);
                            let title = state
                                .metadata
                                .db
                                .track_record_by_path(&path.to_string_lossy())
                                .ok()
                                .flatten()
                                .and_then(|record| record.title)
                                .or_else(|| Some(file_name));
                            (title, artist, album, Some(format), sample_rate, bitrate_kbps)
                        }
                        _ => (None, None, None, None, None, None),
                    }
                }
                None => (None, None, None, None, None, None),
            };
        let bridge_online = state.providers.bridge
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
            buffer_size_frames: status.buffer_size_frames,
            buffered_frames: status.buffered_frames,
            buffer_capacity_frames: status.buffer_capacity_frames,
            has_previous: status.has_previous,
        };
        drop(status);
        if http_addr.is_some() {
            if let Some(remote) = get_cached_status(state, &bridge_id) {
                apply_remote_status(&mut resp, remote);
            }
        }
        Ok(resp)
    }

    /// Stop playback on a specific bridge output id.
    async fn stop_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError> {
        let (bridge_id, _device_id) =
            parse_output_id(output_id).map_err(|e| ProviderError::BadRequest(e))?;
        let http_addr = {
            let bridges_state = state.providers.bridge.bridges.lock().unwrap();
            let discovered = state.providers.bridge.discovered_bridges.lock().unwrap();
            let merged = merge_bridges(&bridges_state.bridges, &discovered);
            let Some(bridge) = merged.iter().find(|b| b.id == bridge_id) else {
                return Err(ProviderError::BadRequest("unknown bridge id".to_string()));
            };
            bridge.http_addr
        };
        BridgeTransportClient::new(http_addr)
            .stop()
            .await
            .map_err(|e| ProviderError::Internal(format!("{e:#}")))
    }

    async fn refresh_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<(), ProviderError> {
        let bridge_id = parse_provider_id(provider_id)
            .map_err(|e| ProviderError::BadRequest(e))?;
        if let Ok(mut cache) = state.providers.bridge.device_cache.lock() {
            cache.remove(&bridge_id);
        }
        if let Ok(mut cache) = state.providers.bridge.status_cache.lock() {
            cache.remove(&bridge_id);
        }
        state.events.outputs_changed();
        Ok(())
    }
}

/// Build output entries from bridges, tracking per-bridge failures.
async fn build_outputs_from_bridges_with_failures(
    state: &AppState,
    bridges: &[crate::config::BridgeConfigResolved],
) -> (Vec<OutputInfo>, Vec<String>) {
    let mut outputs = Vec::new();
    let mut name_counts = std::collections::HashMap::<String, usize>::new();
    let mut by_bridge = Vec::new();
    let mut failed = Vec::new();

    for bridge in bridges {
        let devices = match list_devices_cached_or_fetch(
            state,
            &bridge.id,
            &bridge.name,
            bridge.http_addr,
        )
        .await
        {
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
                    "outputs: device list failed after retries"
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

/// Query a single bridge for device outputs.
async fn build_outputs_for_bridge(
    state: &AppState,
    bridge: &crate::config::BridgeConfigResolved,
) -> Result<Vec<OutputInfo>, anyhow::Error> {
    let devices =
        list_devices_cached_or_fetch(state, &bridge.id, &bridge.name, bridge.http_addr).await?;
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

async fn list_devices_cached_or_fetch(
    state: &AppState,
    bridge_id: &str,
    bridge_name: &str,
    http_addr: std::net::SocketAddr,
) -> Result<Vec<HttpDeviceInfo>, anyhow::Error> {
    if let Ok(cache) = state.providers.bridge.device_cache.lock() {
        if let Some(devices) = cache.get(bridge_id) {
            return Ok(devices.clone());
        }
    }
    let bridge = crate::config::BridgeConfigResolved {
        id: bridge_id.to_string(),
        name: bridge_name.to_string(),
        http_addr,
    };
    let devices = list_devices_with_retry(&bridge, 3).await?;
    if let Ok(mut cache) = state.providers.bridge.device_cache.lock() {
        cache.insert(bridge_id.to_string(), devices.clone());
    }
    Ok(devices)
}

fn resolve_device_name(devices: &[HttpDeviceInfo], device_id: &str) -> Option<String> {
    devices
        .iter()
        .find(|d| d.id == device_id)
        .or_else(|| devices.iter().find(|d| d.name == device_id))
        .map(|d| d.name.clone())
}

fn get_cached_status(
    state: &AppState,
    bridge_id: &str,
) -> Option<crate::bridge_transport::HttpStatusResponse> {
    state
        .providers
        .bridge
        .status_cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(bridge_id).cloned())
}

fn apply_remote_status(
    resp: &mut StatusResponse,
    remote: crate::bridge_transport::HttpStatusResponse,
) {
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
    resp.buffered_frames = remote.buffered_frames;
    resp.buffer_capacity_frames = remote.buffer_capacity_frames;
}

async fn list_devices_with_retry(
    bridge: &crate::config::BridgeConfigResolved,
    attempts: usize,
) -> Result<Vec<HttpDeviceInfo>, anyhow::Error> {
    list_devices_with_retry_fn(bridge, attempts, || async {
        BridgeTransportClient::new(bridge.http_addr)
            .list_devices()
            .await
    })
    .await
}

async fn list_devices_with_retry_fn<F, Fut>(
    bridge: &crate::config::BridgeConfigResolved,
    attempts: usize,
    mut list_fn: F,
) -> Result<Vec<HttpDeviceInfo>, anyhow::Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<HttpDeviceInfo>, anyhow::Error>>,
{
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=attempts {
        match list_fn().await {
            Ok(list) => {
                if attempt > 1 {
                    tracing::info!(
                        bridge_id = %bridge.id,
                        bridge_name = %bridge.name,
                        attempt,
                        "outputs: device list recovered"
                    );
                }
                return Ok(list);
            }
            Err(err) => {
                last_err = Some(err);
                if attempt < attempts {
                    let backoff = Duration::from_millis(200 * attempt as u64);
                    tracing::warn!(
                        bridge_id = %bridge.id,
                        bridge_name = %bridge.name,
                        attempt,
                        "outputs: device list failed; retrying"
                    );
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("device list failed")))
}

#[cfg(test)]
mod tests_retry {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_bridge() -> crate::config::BridgeConfigResolved {
        crate::config::BridgeConfigResolved {
            id: "bridge-1".to_string(),
            name: "Bridge 1".to_string(),
            http_addr: "127.0.0.1:1".parse().unwrap(),
        }
    }

    #[test]
    fn list_devices_with_retry_fn_succeeds_after_retry() {
        let bridge = make_bridge();
        let calls = AtomicUsize::new(0);
        let devices = actix_web::rt::System::new().block_on(async {
            list_devices_with_retry_fn(&bridge, 3, || async {
                let attempt = calls.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    Err(anyhow::anyhow!("timeout"))
                } else {
                    Ok(vec![HttpDeviceInfo {
                        id: "dev1".to_string(),
                        name: "Device 1".to_string(),
                        min_rate: 0,
                        max_rate: 0,
                    }])
                }
            })
            .await
        })
        .expect("retry should succeed");

        assert_eq!(devices.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn list_devices_with_retry_fn_returns_last_error() {
        let bridge = make_bridge();
        let calls = AtomicUsize::new(0);
        let result = actix_web::rt::System::new().block_on(async {
            list_devices_with_retry_fn(&bridge, 2, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(anyhow::anyhow!("timeout"))
            })
            .await
        });

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

/// Normalize a reported min/max rate into a valid range.
fn normalize_supported_rates(min_hz: u32, max_hz: u32) -> Option<SupportedRates> {
    if min_hz == 0 || max_hz == 0 || max_hz < min_hz || max_hz == u32::MAX {
        return None;
    }
    Some(SupportedRates { min_hz, max_hz })
}

/// Shorten long device ids for display.
fn short_device_id(id: &str) -> String {
    const MAX_LEN: usize = 48;
    if id.len() <= MAX_LEN {
        return id.to_string();
    }
    let head = &id[..32];
    let tail = &id[id.len().saturating_sub(12)..];
    format!("{head}...{tail}")
}

/// Add a placeholder output when the active output is missing from discovery.
fn inject_active_output_for_bridge(
    state: &AppState,
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
    let status = match get_cached_status(state, &bridge.id) {
        Some(status) => status,
        None => return,
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

fn start_paused_for_resume(
    prior_active_output_id: Option<&str>,
    paused: bool,
    user_paused: bool,
) -> bool {
    if prior_active_output_id
        .map(|id| id.starts_with("browser:"))
        .unwrap_or(false)
        && !user_paused
    {
        return false;
    }
    paused
}

/// Estimate bitrate from file size and duration.
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use super::*;
    use std::sync::atomic::AtomicBool;

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
    fn short_device_id_truncates_long_ids() {
        let long = "a".repeat(80);
        let shortened = short_device_id(&long);
        assert!(shortened.len() < long.len());
        assert!(shortened.contains("..."));
    }

    #[test]
    fn estimate_bitrate_kbps_returns_none_for_zero_duration() {
        let root = std::env::temp_dir().join(format!(
            "audio-hub-bridge-bitrate-{}",
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

    #[test]
    fn start_paused_for_resume_respects_user_pause_on_browser_switch() {
        assert!(start_paused_for_resume(Some("browser:abc"), true, true));
        assert!(!start_paused_for_resume(Some("browser:abc"), false, true));
    }

    #[test]
    fn start_paused_for_resume_forces_play_when_not_user_paused() {
        assert!(!start_paused_for_resume(Some("browser:abc"), true, false));
        assert!(!start_paused_for_resume(Some("browser:abc"), false, false));
    }

    #[test]
    fn start_paused_for_resume_preserves_pause_for_non_browser() {
        assert!(start_paused_for_resume(Some("bridge:one:dev"), true, false));
        assert!(!start_paused_for_resume(Some("bridge:one:dev"), false, true));
        assert!(!start_paused_for_resume(None, false, true));
    }

    #[test]
    fn inject_active_output_for_bridge_skips_when_present() {
        let bridge = crate::config::BridgeConfigResolved {
            id: "bridge-1".to_string(),
            name: "Bridge".to_string(),
            http_addr: "127.0.0.1:5556".parse().unwrap(),
        };
        let active_id = "bridge:bridge-1:device-1";
        let mut outputs = vec![OutputInfo {
            id: active_id.to_string(),
            kind: "bridge".to_string(),
            name: "Device".to_string(),
            state: "online".to_string(),
            provider_id: Some("bridge:bridge-1".to_string()),
            provider_name: Some("Bridge".to_string()),
            supported_rates: None,
            capabilities: OutputCapabilities {
                device_select: true,
                volume: false,
            },
        }];

        let state = make_state(bridge.clone());
        inject_active_output_for_bridge(&state, &mut outputs, Some(active_id), &bridge);

        assert_eq!(outputs.len(), 1);
    }

    fn make_state(bridge: crate::config::BridgeConfigResolved) -> AppState {
        let tmp = std::env::temp_dir().join(format!(
            "audio-hub-bridge-provider-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let library = crate::library::scan_library(&tmp).expect("scan library");
        let (cmd_tx, _cmd_rx) = crossbeam_channel::unbounded();
        let bridges_state = Arc::new(Mutex::new(crate::state::BridgeState {
            bridges: vec![bridge],
            active_bridge_id: None,
            active_output_id: None,
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
            enabled: false,
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
        let cast_state = Arc::new(crate::state::CastProviderState::new());
        AppState::new(
            library,
            metadata_db,
            None,
            crate::state::MetadataWake::new(),
            bridge_state,
            local_state,
            browser_state,
            cast_state,
            playback_manager,
            device_selection,
            crate::events::EventBus::new(),
            Arc::new(crate::events::LogBus::new(64)),
            Arc::new(Mutex::new(crate::state::OutputSettingsState::default())),
            None,
        )
    }
}
/// Switch the active bridge id and stop the current bridge worker.
fn switch_active_bridge(
    state: &AppState,
    bridge_id: &str,
    http_addr: std::net::SocketAddr,
) -> Result<(), anyhow::Error> {
    let mut bridges = state.providers.bridge.bridges.lock().unwrap();
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

    state.providers.bridge
        .bridge_online
        .store(false, std::sync::atomic::Ordering::Relaxed);
    {
        let player = state.providers.bridge.player.lock().unwrap();
        let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
    }
    Ok(())
}
