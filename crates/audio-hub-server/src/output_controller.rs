use actix_web::HttpResponse;
use std::path::PathBuf;

use crate::bridge_manager::{merge_bridges, parse_output_id};
use crate::bridge::{http_list_devices, http_set_device, http_status};
use crate::models::{OutputsResponse, OutputInfo, OutputCapabilities, SupportedRates, StatusResponse, ProviderInfo, ProvidersResponse};
use crate::state::AppState;

pub(crate) async fn ensure_active_bridge_connected(state: &AppState) -> Result<(), HttpResponse> {
    if state.bridge_online.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }

    let (bridge_id, addr) = {
        let bridges_state = state.bridges.lock().unwrap();
        let discovered = state.discovered_bridges.lock().unwrap();
        let merged = merge_bridges(&bridges_state.bridges, &discovered);
        let Some(active_bridge_id) = bridges_state.active_bridge_id.as_ref() else {
            return Err(HttpResponse::ServiceUnavailable().body("no active output selected"));
        };
        let Some(bridge) = merged.iter().find(|b| b.id == *active_bridge_id) else {
            return Err(HttpResponse::ServiceUnavailable().body("active bridge not found"));
        };
        (bridge.id.clone(), bridge.http_addr)
    };

    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
    {
        let mut player = state.player.lock().unwrap();
        let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
        player.cmd_tx = cmd_tx.clone();
    }
    crate::bridge::spawn_bridge_worker(
        bridge_id,
        addr,
        cmd_rx,
        cmd_tx,
        state.status.clone(),
        state.queue.clone(),
        state.bridge_online.clone(),
        state.bridges.clone(),
        state.public_base_url.clone(),
    );

    let mut waited = 0u64;
    while waited < 2000
        && !state
            .bridge_online
            .load(std::sync::atomic::Ordering::Relaxed)
    {
        actix_web::rt::time::sleep(std::time::Duration::from_millis(100)).await;
        waited += 100;
    }
    if !state.bridge_online.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(HttpResponse::ServiceUnavailable().body("bridge offline"));
    }
    Ok(())
}

pub(crate) async fn select_output(state: &AppState, output_id: &str) -> Result<(), HttpResponse> {
    let (bridge_id, device_id) = parse_output_id(output_id)
        .map_err(|e| HttpResponse::BadRequest().body(e))?;
    let http_addr = {
        let bridges_state = state.bridges.lock().unwrap();
        let discovered = state.discovered_bridges.lock().unwrap();
        let merged = merge_bridges(&bridges_state.bridges, &discovered);
        let Some(bridge) = merged.iter().find(|b| b.id == bridge_id) else {
            return Err(HttpResponse::BadRequest().body("unknown bridge id"));
        };
        bridge.http_addr
    };

    let device_name = match http_list_devices(http_addr) {
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
                return Err(HttpResponse::BadRequest().body("unknown device"));
            }
        }
        Err(e) => {
            tracing::warn!(
                bridge_id = %bridge_id,
                error = %e,
                "output select failed: device list"
            );
            return Err(HttpResponse::InternalServerError().body(format!("{e:#}")));
        }
    };

    let resume_info = {
        let status = state.status.lock().unwrap();
        (
            status.now_playing.clone(),
            status.elapsed_ms,
            status.paused,
        )
    };

    // Stop current playback before switching outputs.
    {
        let cmd_tx = state.player.lock().unwrap().cmd_tx.clone();
        let _ = cmd_tx.send(crate::bridge::BridgeCommand::Stop);
    }

    if let Err(e) = switch_active_bridge(state, &bridge_id, http_addr) {
        tracing::warn!(
            bridge_id = %bridge_id,
            error = %e,
            "output select failed: switch bridge"
        );
        return Err(HttpResponse::InternalServerError().body(format!("{e:#}")));
    }
    if let Err(e) = http_set_device(http_addr, &device_name) {
        tracing::warn!(
            bridge_id = %bridge_id,
            device = %device_name,
            error = %e,
            "output select failed: set device"
        );
        return Err(HttpResponse::InternalServerError().body(format!("{e:#}")));
    }

    {
        let mut bridges = state.bridges.lock().unwrap();
        bridges.active_bridge_id = Some(bridge_id);
        bridges.active_output_id = Some(output_id.to_string());
        tracing::info!(
            output_id = ?bridges.active_output_id,
            bridge_id = ?bridges.active_bridge_id,
            "output selected"
        );
    }

    ensure_active_bridge_connected(state).await?;

    if let (Some(path), Some(elapsed_ms)) = (resume_info.0, resume_info.1) {
        let ext_hint = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let start_paused = resume_info.2;
        let _ = state.player.lock().unwrap().cmd_tx.send(
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

pub(crate) async fn status_for_output(
    state: &AppState,
    output_id: &str,
) -> Result<StatusResponse, HttpResponse> {
    if parse_output_id(output_id).is_err() {
        return Err(HttpResponse::BadRequest().body("invalid output id"));
    }
    let (active_output_id, http_addr) = {
        let bridges = state.bridges.lock().unwrap();
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
        return Err(HttpResponse::BadRequest().body("output is not active"));
    }
    ensure_active_bridge_connected(state).await?;

    let status = state.status.lock().unwrap();
    let (title, artist, album, format, sample_rate, bitrate_kbps) = match status.now_playing.as_ref() {
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
    let bridge_online = state.bridge_online.load(std::sync::atomic::Ordering::Relaxed);
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
        match crate::bridge::http_status(http_addr) {
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

pub(crate) fn outputs_for_bridge(
    state: &AppState,
    bridge_id: &str,
) -> Result<OutputsResponse, HttpResponse> {
    let (bridge, active_output_id) = {
        let bridges_state = state.bridges.lock().unwrap();
        let discovered = state.discovered_bridges.lock().unwrap();
        let merged = merge_bridges(&bridges_state.bridges, &discovered);
        let Some(bridge) = merged.iter().find(|b| b.id == bridge_id) else {
            return Err(HttpResponse::BadRequest().body("unknown bridge id"));
        };
        (bridge.clone(), bridges_state.active_output_id.clone())
    };

    let mut outputs = build_outputs_for_bridge(&bridge)
        .map_err(|e| HttpResponse::InternalServerError().body(format!("{e:#}")))?;
    inject_active_output_if_missing(&mut outputs, active_output_id.as_deref(), &bridge);

    Ok(OutputsResponse {
        active_id: active_output_id,
        outputs,
    })
}

pub(crate) fn outputs_for_provider(
    state: &AppState,
    provider_id: &str,
) -> Result<OutputsResponse, HttpResponse> {
    outputs_for_bridge(state, provider_id)
}

pub(crate) fn list_outputs(state: &AppState) -> OutputsResponse {
    let bridges_state = state.bridges.lock().unwrap();
    let discovered = state.discovered_bridges.lock().unwrap();
    let _active_online = state.bridge_online.load(std::sync::atomic::Ordering::Relaxed);
    tracing::info!(
        count = bridges_state.bridges.len(),
        ids = ?bridges_state.bridges.iter().map(|b| b.id.clone()).collect::<Vec<_>>(),
        active_bridge_id = ?bridges_state.active_bridge_id,
        "outputs: bridge inventory"
    );
    tracing::info!(
        count = discovered.len(),
        ids = ?discovered.keys().cloned().collect::<Vec<_>>(),
        "outputs: discovered bridges"
    );
    let active_id = bridges_state.active_output_id.clone();
    let merged = merge_bridges(&bridges_state.bridges, &discovered);
    let (mut outputs, failed) = build_outputs_from_bridges_with_failures(&merged);
    if !failed.is_empty() {
        let configured_ids: std::collections::HashSet<String> =
            bridges_state.bridges.iter().map(|b| b.id.clone()).collect();
        drop(bridges_state);
        drop(discovered);
        if let Ok(mut map) = state.discovered_bridges.lock() {
            for id in failed {
                if !configured_ids.contains(&id) {
                    map.remove(&id);
                    tracing::info!(bridge_id = %id, "outputs: removed discovered bridge after device list failure");
                }
            }
        }
    }
    if let Some(active_id) = active_id.as_deref() {
        if !outputs.iter().any(|o| o.id == active_id) {
            if let Ok((bridge_id, _)) = parse_output_id(active_id) {
                if let Some(bridge) = merged.iter().find(|b| b.id == bridge_id) {
                    inject_active_output_if_missing(&mut outputs, Some(active_id), bridge);
                }
            }
        }
    }
    OutputsResponse {
        active_id,
        outputs,
    }
}

pub(crate) fn list_providers(state: &AppState) -> ProvidersResponse {
    let bridges_state = state.bridges.lock().unwrap();
    let discovered = state.discovered_bridges.lock().unwrap();
    let active_online = state.bridge_online.load(std::sync::atomic::Ordering::Relaxed);
    let merged = merge_bridges(&bridges_state.bridges, &discovered);
    let providers = merged
        .iter()
        .map(|b| ProviderInfo {
            id: b.id.clone(),
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
        .collect();
    ProvidersResponse { providers }
}

fn build_outputs_from_bridges_with_failures(
    bridges: &[crate::config::BridgeConfigResolved],
) -> (Vec<OutputInfo>, Vec<String>) {
    let mut outputs = Vec::new();
    let mut name_counts = std::collections::HashMap::<String, usize>::new();
    let mut by_bridge = Vec::new();
    let mut failed = Vec::new();

    for bridge in bridges {
        let devices = match http_list_devices(bridge.http_addr) {
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
            provider_id: Some(bridge.id.clone()),
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
    let devices = http_list_devices(bridge.http_addr)?;
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
            provider_id: Some(bridge.id.clone()),
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

fn inject_active_output_if_missing(
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
    let status = match http_status(bridge.http_addr) {
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
        provider_id: Some(bridge.id.clone()),
        provider_name: Some(bridge.name.clone()),
        supported_rates,
        capabilities: OutputCapabilities {
            device_select: true,
            volume: false,
        },
    });
}

fn switch_active_bridge(
    state: &AppState,
    bridge_id: &str,
    http_addr: std::net::SocketAddr,
) -> Result<(), anyhow::Error> {
    let mut bridges = state.bridges.lock().unwrap();
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
        .bridge_online
        .store(false, std::sync::atomic::Ordering::Relaxed);
    {
        let player = state.player.lock().unwrap();
        let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
    }
    Ok(())
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
