//! mDNS discovery for bridge instances.
//!
//! Runs a background task that updates the bridge registry from mDNS events.

use actix_web::web;
use mdns_sd::{ServiceDaemon, ServiceEvent};

use crate::state::AppState;

pub(crate) fn spawn_mdns_discovery(state: web::Data<AppState>) {
    std::thread::spawn(move || {
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "mdns: failed to start daemon");
                return;
            }
        };
        let receiver = match daemon.browse("_audio-bridge._tcp.local.") {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "mdns: browse failed");
                return;
            }
        };
        tracing::info!("mdns: browsing for _audio-bridge._tcp.local.");
        let mut fullname_to_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for event in receiver {
            match event {
                ServiceEvent::ServiceFound(_ty, fullname) => {
                    tracing::info!(fullname = %fullname, "mdns: service found");
                    if let Some(id) = fullname_to_id.get(&fullname).cloned() {
                        if let Ok(mut map) = state.bridge.discovered_bridges.lock() {
                            if let Some(entry) = map.get_mut(&id) {
                                entry.last_seen = std::time::Instant::now();
                            }
                        }
                    }
                }
                ServiceEvent::ServiceResolved(info) => {
                    tracing::info!(
                        fullname = %info.get_fullname(),
                        host = %info.get_hostname(),
                        port = info.get_port(),
                        "mdns: service resolved"
                    );
                    let id = info
                        .get_property("id")
                        .map(|p| p.val_str().to_string())
                        .map(|s| s.strip_prefix("id=").unwrap_or(&s).to_string())
                        .unwrap_or_else(|| info.get_fullname().to_string());
                    let name = info
                        .get_property("name")
                        .map(|p| p.val_str().to_string())
                        .map(|s| s.strip_prefix("name=").unwrap_or(&s).to_string())
                        .unwrap_or_else(|| id.clone());
                    let addr = info
                        .get_addresses()
                        .iter()
                        .find_map(|ip| match ip {
                            mdns_sd::ScopedIp::V4(v4) => Some(*v4.addr()),
                            _ => None,
                        });
                    let Some(ip) = addr else {
                        tracing::warn!(fullname = %info.get_fullname(), "mdns: resolved without IPv4");
                        continue;
                    };
                    let http_port = info.get_port();
                    let http = std::net::SocketAddr::new(std::net::IpAddr::V4(ip), http_port);
                    let bridge = crate::config::BridgeConfigResolved {
                        id: id.clone(),
                        name,
                        http_addr: http,
                    };
                    if let Ok(mut map) = state.bridge.discovered_bridges.lock() {
                        let now = std::time::Instant::now();
                        map.insert(
                            id.clone(),
                            crate::state::DiscoveredBridge {
                                bridge,
                                last_seen: now,
                            },
                        );
                    }
                    tracing::info!(
                        bridge_id = %id,
                        http_addr = %http,
                        "mdns: discovered bridge"
                    );
                    fullname_to_id.insert(info.get_fullname().to_string(), id);
                }
                ServiceEvent::ServiceRemoved(name, _) => {
                    if let Some(id) = fullname_to_id.remove(&name) {
                        if let Ok(mut map) = state.bridge.discovered_bridges.lock() {
                            map.remove(&id);
                        }
                        tracing::info!(bridge_id = %id, "mdns: bridge removed");
                    }
                }
                _ => {}
            }
        }
    });
}

pub(crate) fn spawn_discovered_health_watcher(state: web::Data<AppState>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(15));
        let snapshot = match state.bridge.discovered_bridges.lock() {
            Ok(map) => map
                .iter()
                .map(|(id, entry)| (id.clone(), entry.bridge.http_addr, entry.last_seen))
                .collect::<Vec<_>>(),
            Err(_) => continue,
        };

        let now = std::time::Instant::now();
        for (id, http_addr, last_seen) in snapshot {
            let ok = ping_bridge(http_addr);
            if ok {
                if let Ok(mut map) = state.bridge.discovered_bridges.lock() {
                    if let Some(entry) = map.get_mut(&id) {
                        entry.last_seen = now;
                    }
                }
            } else if now.duration_since(last_seen) > std::time::Duration::from_secs(60) {
                if let Ok(mut map) = state.bridge.discovered_bridges.lock() {
                    map.remove(&id);
                }
                tracing::info!(bridge_id = %id, "mdns: bridge removed (health check)");
            }
        }
    });
}

fn ping_bridge(http_addr: std::net::SocketAddr) -> bool {
    let url = format!("http://{http_addr}/health");
    let resp = ureq::get(&url)
        .config()
        .timeout_per_call(Some(std::time::Duration::from_secs(2)))
        .build()
        .call();
    resp.map(|r| r.status().is_success()).unwrap_or(false)
}
