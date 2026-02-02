use actix_web::web;
use crate::bridge::{http_list_devices, http_set_device};
use crate::state::AppState;

pub(crate) fn parse_output_id(id: &str) -> Result<(String, String), String> {
    let mut parts = id.splitn(3, ':');
    let kind = parts.next().unwrap_or("");
    let bridge_id = parts.next().unwrap_or("");
    let device = parts.next().unwrap_or("");
    if kind != "bridge" || bridge_id.is_empty() || device.is_empty() {
        return Err("invalid output id".to_string());
    }
    Ok((bridge_id.to_string(), device.to_string()))
}

pub(crate) fn merge_bridges(
    configured: &[crate::config::BridgeConfigResolved],
    discovered: &std::collections::HashMap<String, crate::state::DiscoveredBridge>,
) -> Vec<crate::config::BridgeConfigResolved> {
    let mut merged = Vec::with_capacity(configured.len() + discovered.len());
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_addrs: std::collections::HashSet<std::net::SocketAddr> =
        std::collections::HashSet::new();
    for b in configured {
        seen.insert(b.id.clone());
        seen_addrs.insert(b.addr);
        merged.push(b.clone());
    }
    for (id, b) in discovered {
        if seen.contains(id) {
            continue;
        }
        if seen_addrs.contains(&b.bridge.addr) {
            tracing::info!(
                bridge_id = %b.bridge.id,
                bridge_name = %b.bridge.name,
                addr = %b.bridge.addr,
                "merge: skipping discovered bridge with configured addr"
            );
            continue;
        }
        seen_addrs.insert(b.bridge.addr);
        merged.push(b.bridge.clone());
    }
    merged
}

pub(crate) fn spawn_pending_output_watcher(state: web::Data<AppState>) {
    std::thread::spawn(move || {
        let mut backoff_ms = 500u64;
        loop {
            let (pending, active_output_id, bridges) = {
                let bridges = state.bridges.lock().unwrap();
                let discovered = state.discovered_bridges.lock().unwrap();
                let merged = merge_bridges(&bridges.bridges, &discovered);
                (
                    is_pending_output(&bridges.active_output_id),
                    bridges.active_output_id.clone(),
                    merged,
                )
            };

            if !pending {
                backoff_ms = 500;
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }

            let mut resolved = false;
            for bridge in &bridges {
                match http_list_devices(bridge.http_addr) {
                    Ok(devices) if !devices.is_empty() => {
                        let device = devices[0].clone();
                        let output_id = format!("bridge:{}:{}", bridge.id, device.name);
                        match http_set_device(bridge.http_addr, &device.name) {
                            Ok(()) => {
                                {
                                    let mut bridges_state = state.bridges.lock().unwrap();
                                    if bridges_state.active_output_id == active_output_id {
                                        bridges_state.active_output_id = output_id.clone();
                                        bridges_state.active_bridge_id = bridge.id.clone();
                                    }
                                }
                                tracing::info!(
                                    bridge_id = %bridge.id,
                                    bridge_name = %bridge.name,
                                    device = %device.name,
                                    output_id = %output_id,
                                    "active output resolved from pending"
                                );
                                state
                                    .bridge_online
                                    .store(false, std::sync::atomic::Ordering::Relaxed);
                                resolved = true;
                                backoff_ms = 500;
                                break;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    bridge_id = %bridge.id,
                                    bridge_name = %bridge.name,
                                    error = %e,
                                    "device available but bridge command failed; retrying"
                                );
                            }
                        }
                    }
                    Ok(_) => {
                        tracing::debug!(
                            bridge_id = %bridge.id,
                            bridge_name = %bridge.name,
                            "bridge returned no outputs while pending"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            bridge_id = %bridge.id,
                            bridge_name = %bridge.name,
                            error = %e,
                            "bridge unavailable while pending; retrying"
                        );
                    }
                }
            }

            if !resolved {
                std::thread::sleep(std::time::Duration::from_millis(backoff_ms));
                backoff_ms = (backoff_ms * 2).min(30_000);
            }
        }
    });
}

fn is_pending_output(id: &str) -> bool {
    id.ends_with(":pending")
}
