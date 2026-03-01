//! Bridge device SSE stream watchers.
//!
//! Spawns background listeners that trigger outputs refreshes when bridge devices change.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use actix_web::web;

use crate::bridge::update_online_and_should_emit;
use crate::bridge_manager::parse_output_id;
use crate::bridge_transport::{BridgeTransportClient, HttpDevicesSnapshot, HttpStatusResponse};
use crate::playback_transport::ChannelTransport;
use crate::state::AppState;

const MAX_DISCOVERED_FAILURES: usize = 5;
const RETRY_BASE_DELAY: Duration = Duration::from_secs(2);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(60);

pub(crate) fn spawn_bridge_device_streams_for_config(state: web::Data<AppState>) {
    let bridges = state
        .providers
        .bridge
        .bridges
        .lock()
        .unwrap()
        .bridges
        .clone();
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
    let bridges = state
        .providers
        .bridge
        .bridges
        .lock()
        .unwrap()
        .bridges
        .clone();
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
                let result = client
                    .listen_devices_stream(|snapshot| {
                        if let Ok(mut cache) = state.providers.bridge.device_cache.lock() {
                            cache.insert(bridge_id.clone(), snapshot.devices.clone());
                        }
                        seen_event.store(true, Ordering::Relaxed);
                        if last_snapshot.as_ref() != Some(&snapshot) {
                            last_snapshot = Some(snapshot);
                            events.outputs_changed();
                        }
                    })
                    .await;
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
            let mut session_auto_advance_in_flight = false;
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
                let should_stop_on_join = state
                    .providers
                    .bridge
                    .stop_on_join_done
                    .lock()
                    .map(|done| !done.contains(&bridge_id))
                    .unwrap_or(false);
                if should_stop_on_join {
                    match client.stop().await {
                        Ok(()) => {
                            if let Ok(mut done) = state.providers.bridge.stop_on_join_done.lock() {
                                done.insert(bridge_id.clone());
                            }
                            tracing::info!(bridge_id = %bridge_id, "bridge reset on join");
                        }
                        Err(err) => {
                            tracing::warn!(
                                bridge_id = %bridge_id,
                                error = %err,
                                "bridge stop-on-join failed; will retry"
                            );
                            tokio::time::sleep(RETRY_BASE_DELAY).await;
                            continue;
                        }
                    }
                }
                let result = client
                    .listen_status_stream(|snapshot| {
                        if let Ok(mut cache) = state.providers.bridge.status_cache.lock() {
                            cache.insert(bridge_id.clone(), snapshot.clone());
                        }
                        if last_snapshot.as_ref() != Some(&snapshot) {
                            apply_remote_status(
                                &state,
                                &bridge_id,
                                &snapshot,
                                &mut last_duration_ms,
                                &mut session_auto_advance_in_flight,
                            );
                            last_snapshot = Some(snapshot);
                            events.status_changed();
                        }
                    })
                    .await;
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
    state: &web::Data<AppState>,
    bridge_id: &str,
    remote: &HttpStatusResponse,
    last_duration_ms: &mut Option<u64>,
    session_auto_advance_in_flight: &mut bool,
) {
    let session_bound = session_for_bridge(bridge_id)
        .and_then(|session_id| {
            crate::session_registry::get_session(&session_id).and_then(|record| {
                record
                    .active_output_id
                    .map(|output_id| (session_id, output_id))
            })
        })
        .filter(|(_, output_id)| output_id.starts_with(&format!("bridge:{bridge_id}:")));
    let is_active = state
        .providers
        .bridge
        .bridges
        .lock()
        .map(|s| s.active_bridge_id.as_deref() == Some(bridge_id))
        .unwrap_or(false);
    if !is_active && session_bound.is_none() {
        return;
    }
    let session_eof = remote.end_reason == Some(audio_bridge_types::PlaybackEndReason::Eof);
    if !session_eof {
        *session_auto_advance_in_flight = false;
    }
    if session_eof && !*session_auto_advance_in_flight {
        if let Some((session_id, output_id)) = session_bound.clone() {
            if let Ok(Some(next_track_id)) =
                crate::session_registry::queue_next_track_id(&session_id)
            {
                let Some(next_path) = state
                    .metadata
                    .db
                    .track_path_for_id(next_track_id)
                    .ok()
                    .flatten()
                    .map(PathBuf::from)
                    .and_then(|candidate| {
                        state
                            .output
                            .controller
                            .canonicalize_under_root(state, &candidate)
                            .ok()
                    })
                else {
                    tracing::warn!(
                        session_id = %session_id,
                        track_id = next_track_id,
                        "session bridge auto-advance track not found"
                    );
                    return;
                };
                let Some(http_addr) = resolve_bridge_addr(state, bridge_id) else {
                    return;
                };
                let device_id = parse_output_id(&output_id)
                    .ok()
                    .map(|(_, device_id)| device_id);
                let Some(device_id) = device_id else {
                    return;
                };
                let state_cloned = state.clone();
                let output_id_cloned = output_id.clone();
                let session_id_cloned = session_id.clone();
                let bridge_id_cloned = bridge_id.to_string();
                tokio::spawn(async move {
                    let client = BridgeTransportClient::new_with_base(
                        http_addr,
                        state_cloned.providers.bridge.public_base_url.clone(),
                        Some(state_cloned.metadata.db.clone()),
                    );
                    if let Ok(devices) = client.list_devices().await {
                        if let Some(device_name) = devices
                            .iter()
                            .find(|d| d.id == device_id)
                            .map(|d| d.name.clone())
                        {
                            let _ = client.set_device(&device_name, None).await;
                            let ext_hint = next_path
                                .extension()
                                .and_then(|ext| ext.to_str())
                                .unwrap_or("")
                                .to_ascii_lowercase();
                            let title = Some(next_path.to_string_lossy().to_string());
                            let _ = client
                                .play_path(
                                    &next_path,
                                    if ext_hint.is_empty() {
                                        None
                                    } else {
                                        Some(ext_hint.as_str())
                                    },
                                    title.as_deref(),
                                    None,
                                    false,
                                )
                                .await;
                            tracing::info!(
                                bridge_id = %bridge_id_cloned,
                                session_id = %session_id_cloned,
                                output_id = %output_id_cloned,
                                path = %next_path.to_string_lossy(),
                                "session bridge auto-advance dispatched"
                            );
                        }
                    }
                });
                *session_auto_advance_in_flight = true;
            }
        }
    }
    if !is_active {
        return;
    }
    if update_online_and_should_emit(&state.providers.bridge.bridge_online, true) {
        state.events.outputs_changed();
    }
    let now = std::time::Instant::now();
    let mut until_guard = state.providers.bridge.output_switch_until.lock().ok();
    let mut suppress_auto_advance = false;
    if let Some(ref mut guard) = until_guard {
        if let Some(until) = guard.as_ref() {
            if now < *until {
                suppress_auto_advance = true;
            } else {
                **guard = None;
                state
                    .providers
                    .bridge
                    .output_switch_in_flight
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    if suppress_auto_advance {
        state.playback.manager.set_manual_advance_in_flight(true);
    }

    tracing::debug!(
        bridge_id = %bridge_id,
        now_playing = %remote.now_playing.as_deref().unwrap_or("<none>"),
        elapsed_ms = ?remote.elapsed_ms,
        duration_ms = ?remote.duration_ms,
        paused = remote.paused,
        "bridge status snapshot received"
    );

    let (inputs, changed) = state
        .playback
        .manager
        .status()
        .reduce_remote_and_inputs(remote, *last_duration_ms);
    state.playback.manager.status().emit_if_changed(changed);
    state.playback.manager.update_has_previous();
    let transport =
        ChannelTransport::new(state.providers.bridge.player.lock().unwrap().cmd_tx.clone());
    if !suppress_auto_advance && session_bound.is_none() {
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

fn session_for_bridge(bridge_id: &str) -> Option<String> {
    let (_, bridge_locks) = crate::session_registry::lock_snapshot();
    bridge_locks
        .into_iter()
        .find_map(|(locked_bridge_id, session_id)| {
            (locked_bridge_id == bridge_id).then_some(session_id)
        })
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
