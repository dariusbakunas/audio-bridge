//! Bridge device SSE stream watchers.
//!
//! Spawns background listeners that trigger outputs refreshes when bridge devices change.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use actix_web::web;

use crate::bridge_transport::{BridgeTransportClient, HttpDevicesSnapshot, HttpStatusResponse};
use crate::bridge::update_online_and_should_emit;
use crate::playback_transport::ChannelTransport;
use crate::queue_service::QueueService;
use crate::state::AppState;

const MAX_DISCOVERED_FAILURES: usize = 5;
const RETRY_BASE_DELAY: Duration = Duration::from_secs(2);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(60);

pub(crate) fn spawn_bridge_device_streams_for_config(state: web::Data<AppState>) {
    let bridges = state.providers.bridge.bridges.lock().unwrap().bridges.clone();
    for bridge in bridges {
        spawn_bridge_device_stream(state.clone(), bridge.id);
    }
}

pub(crate) fn spawn_bridge_device_stream_for_discovered(
    state: web::Data<AppState>,
    bridge_id: String,
) {
    spawn_bridge_device_stream(state, bridge_id);
}

pub(crate) fn spawn_bridge_status_streams_for_config(state: web::Data<AppState>) {
    let bridges = state.providers.bridge.bridges.lock().unwrap().bridges.clone();
    for bridge in bridges {
        spawn_bridge_status_stream(state.clone(), bridge.id);
    }
}

pub(crate) fn spawn_bridge_status_stream_for_discovered(
    state: web::Data<AppState>,
    bridge_id: String,
) {
    spawn_bridge_status_stream(state, bridge_id);
}

fn spawn_bridge_device_stream(state: web::Data<AppState>, bridge_id: String) {
    if let Ok(mut active) = state.providers.bridge.device_streams.lock() {
        if !active.insert(bridge_id.clone()) {
            return;
        }
    }

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("bridge device stream runtime");
        runtime.block_on(async move {
        let mut last_snapshot: Option<HttpDevicesSnapshot> = None;
        let mut failures = 0usize;
        loop {
            let Some(http_addr) = resolve_bridge_addr(&state, &bridge_id) else {
                break;
            };
            let is_configured = is_configured_bridge(&state, &bridge_id);
            let events = state.events.clone();
            let client = BridgeTransportClient::new_with_base(
                http_addr,
                String::new(),
                Some(state.metadata.db.clone()),
            );
            let seen_event = AtomicBool::new(false);
            let result = client.listen_devices_stream(|snapshot| {
                if let Ok(mut cache) = state.providers.bridge.device_cache.lock() {
                    cache.insert(bridge_id.clone(), snapshot.devices.clone());
                }
                seen_event.store(true, Ordering::Relaxed);
                if last_snapshot.as_ref() != Some(&snapshot) {
                    last_snapshot = Some(snapshot);
                    events.outputs_changed();
                }
            }).await;
            if let Err(e) = result {
                if seen_event.load(Ordering::Relaxed) {
                    failures = 0;
                } else {
                    failures = failures.saturating_add(1);
                }
                tracing::warn!(
                    bridge_id = %bridge_id,
                    failures,
                    error = %e,
                    "bridge devices stream failed; reconnecting"
                );
            }
            if !is_configured && failures >= MAX_DISCOVERED_FAILURES {
                if let Ok(mut map) = state.providers.bridge.discovered_bridges.lock() {
                    map.remove(&bridge_id);
                }
                if let Ok(mut cache) = state.providers.bridge.device_cache.lock() {
                    cache.remove(&bridge_id);
                }
                if let Ok(mut cache) = state.providers.bridge.status_cache.lock() {
                    cache.remove(&bridge_id);
                }
                state.events.outputs_changed();
                tracing::info!(
                    bridge_id = %bridge_id,
                    "bridge devices stream failed repeatedly; removing discovered bridge"
                );
                break;
            }
            if resolve_bridge_addr(&state, &bridge_id).is_none() {
                break;
            }
            let delay_secs = RETRY_BASE_DELAY
                .as_secs()
                .saturating_mul(failures.max(1) as u64);
            let delay = Duration::from_secs(delay_secs).min(RETRY_MAX_DELAY);
            tokio::time::sleep(delay).await;
        }

        if let Ok(mut active) = state.providers.bridge.device_streams.lock() {
            active.remove(&bridge_id);
        }
        if let Ok(mut cache) = state.providers.bridge.device_cache.lock() {
            cache.remove(&bridge_id);
        }
        });
    });
}

fn spawn_bridge_status_stream(state: web::Data<AppState>, bridge_id: String) {
    if let Ok(mut active) = state.providers.bridge.status_streams.lock() {
        if !active.insert(bridge_id.clone()) {
            return;
        }
    }

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("bridge status stream runtime");
        runtime.block_on(async move {
        let mut last_snapshot: Option<HttpStatusResponse> = None;
        let mut last_duration_ms: Option<u64> = None;
        let mut failures = 0usize;
        loop {
            let Some(http_addr) = resolve_bridge_addr(&state, &bridge_id) else {
                break;
            };
            let events = state.events.clone();
            let client = BridgeTransportClient::new_with_base(
                http_addr,
                String::new(),
                Some(state.metadata.db.clone()),
            );
            let result = client.listen_status_stream(|snapshot| {
                if let Ok(mut cache) = state.providers.bridge.status_cache.lock() {
                    cache.insert(bridge_id.clone(), snapshot.clone());
                }
                if last_snapshot.as_ref() != Some(&snapshot) {
                    apply_remote_status(&state, &bridge_id, &snapshot, &mut last_duration_ms);
                    last_snapshot = Some(snapshot);
                    events.status_changed();
                }
            }).await;
            if let Err(e) = result {
                failures = failures.saturating_add(1);
                if let Ok(mut cache) = state.providers.bridge.status_cache.lock() {
                    cache.remove(&bridge_id);
                }
                tracing::warn!(
                    bridge_id = %bridge_id,
                    failures,
                    error = %e,
                    "bridge status stream failed; reconnecting"
                );
            } else {
                failures = 0;
            }
            if resolve_bridge_addr(&state, &bridge_id).is_none() {
                break;
            }
            let delay_secs = RETRY_BASE_DELAY
                .as_secs()
                .saturating_mul(failures.max(1) as u64);
            let delay = Duration::from_secs(delay_secs).min(RETRY_MAX_DELAY);
            tokio::time::sleep(delay).await;
        }

        if let Ok(mut active) = state.providers.bridge.status_streams.lock() {
            active.remove(&bridge_id);
        }
        if let Ok(mut cache) = state.providers.bridge.status_cache.lock() {
            cache.remove(&bridge_id);
        }
        });
    });
}

fn apply_remote_status(
    state: &AppState,
    bridge_id: &str,
    remote: &HttpStatusResponse,
    last_duration_ms: &mut Option<u64>,
) {
    let is_active = state
        .providers
        .bridge
        .bridges
        .lock()
        .map(|s| s.active_bridge_id.as_deref() == Some(bridge_id))
        .unwrap_or(false);
    if !is_active {
        return;
    }
    if update_online_and_should_emit(&state.providers.bridge.bridge_online, true) {
        state.events.outputs_changed();
    }
    let now = std::time::Instant::now();
    let mut until_guard = state
        .providers
        .bridge
        .output_switch_until
        .lock()
        .ok();
    let mut suppress_auto_advance = false;
    if let Some(ref mut guard) = until_guard {
        if let Some(until) = guard.as_ref() {
            if now < *until {
                suppress_auto_advance = true;
            } else {
                **guard = None;
                state.providers.bridge
                    .output_switch_in_flight
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    if suppress_auto_advance {
        state.playback.manager.set_manual_advance_in_flight(true);
    }

    let (inputs, changed) = state
        .playback
        .manager
        .status()
        .reduce_remote_and_inputs(remote, *last_duration_ms);
    state.playback.manager.status().emit_if_changed(changed);
    state.playback.manager.update_has_previous();
    let transport = ChannelTransport::new(
        state.providers.bridge.player.lock().unwrap().cmd_tx.clone(),
    );
    if !suppress_auto_advance {
        let dispatched = state
            .playback
            .manager
            .queue_service()
            .clone()
            .maybe_auto_advance(&transport, inputs);
        if dispatched {
            *last_duration_ms = remote.duration_ms;
            return;
        }
    }
    *last_duration_ms = remote.duration_ms;
}

fn resolve_bridge_addr(state: &AppState, bridge_id: &str) -> Option<SocketAddr> {
    if let Ok(bridges) = state.providers.bridge.bridges.lock() {
        if let Some(bridge) = bridges.bridges.iter().find(|b| b.id == bridge_id) {
            return Some(bridge.http_addr);
        }
    }
    if let Ok(map) = state.providers.bridge.discovered_bridges.lock() {
        if let Some(entry) = map.get(bridge_id) {
            return Some(entry.bridge.http_addr);
        }
    }
    None
}

fn is_configured_bridge(state: &AppState, bridge_id: &str) -> bool {
    state
        .providers
        .bridge
        .bridges
        .lock()
        .map(|bridges| bridges.bridges.iter().any(|b| b.id == bridge_id))
        .unwrap_or(false)
}
