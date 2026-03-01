//! mDNS discovery for bridge instances.
//!
//! Runs a background task that updates the bridge registry from mDNS events.

use actix_web::web;
use mdns_sd::{ServiceDaemon, ServiceEvent};

use crate::bridge_device_streams::{
    spawn_bridge_device_stream_for_discovered, spawn_bridge_status_stream_for_discovered,
};
use crate::state::{AppState, DiscoveredCast};

/// Spawn mDNS discovery loop for bridge devices.
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
        let mut fullname_to_id: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for event in receiver {
            match event {
                ServiceEvent::ServiceFound(_ty, fullname) => {
                    tracing::info!(fullname = %fullname, "mdns: service found");
                    if let Some(id) = fullname_to_id.get(&fullname).cloned() {
                        if let Ok(mut map) = state.providers.bridge.discovered_bridges.lock() {
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
                    let id = property_value(&info, "id")
                        .unwrap_or_else(|| info.get_fullname().to_string());
                    let name = property_value(&info, "name").unwrap_or_else(|| id.clone());
                    let version = property_value(&info, "version");
                    if !is_bridge_version_compatible(version.as_deref()) {
                        tracing::warn!(
                            bridge_id = %id,
                            bridge_name = %name,
                            bridge_version = %version.unwrap_or_else(|| "unknown".to_string()),
                            server_version = env!("CARGO_PKG_VERSION"),
                            "mdns: skipping bridge with incompatible version"
                        );
                        continue;
                    }
                    let addr = first_ipv4_addr(&info);
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
                    if let Ok(mut map) = state.providers.bridge.discovered_bridges.lock() {
                        let now = std::time::Instant::now();
                        map.insert(
                            id.clone(),
                            crate::state::DiscoveredBridge {
                                bridge,
                                last_seen: now,
                            },
                        );
                    }
                    spawn_bridge_device_stream_for_discovered(state.clone(), id.clone());
                    spawn_bridge_status_stream_for_discovered(state.clone(), id.clone());
                    state.events.outputs_changed();
                    tracing::info!(
                        bridge_id = %id,
                        http_addr = %http,
                        "mdns: discovered bridge"
                    );
                    fullname_to_id.insert(info.get_fullname().to_string(), id);
                }
                ServiceEvent::ServiceRemoved(name, _) => {
                    if let Some(id) = fullname_to_id.remove(&name) {
                        if let Ok(mut map) = state.providers.bridge.discovered_bridges.lock() {
                            map.remove(&id);
                        }
                        if let Ok(mut cache) = state.providers.bridge.device_cache.lock() {
                            cache.remove(&id);
                        }
                        if let Ok(mut cache) = state.providers.bridge.status_cache.lock() {
                            cache.remove(&id);
                        }
                        state.events.outputs_changed();
                        tracing::info!(bridge_id = %id, "mdns: bridge removed");
                    }
                }
                _ => {}
            }
        }
    });
}

/// Spawn mDNS discovery loop for Google Cast devices.
pub(crate) fn spawn_cast_mdns_discovery(state: web::Data<AppState>) {
    std::thread::spawn(move || {
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "mdns: failed to start cast daemon");
                return;
            }
        };
        let receiver = match daemon.browse("_googlecast._tcp.local.") {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "mdns: cast browse failed");
                return;
            }
        };
        tracing::info!("mdns: browsing for _googlecast._tcp.local.");
        let mut fullname_to_id: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for event in receiver {
            match event {
                ServiceEvent::ServiceFound(_ty, fullname) => {
                    if let Some(id) = fullname_to_id.get(&fullname).cloned() {
                        if let Ok(mut map) = state.providers.cast.discovered.lock() {
                            if let Some(entry) = map.get_mut(&id) {
                                entry.last_seen = std::time::Instant::now();
                            }
                        }
                    }
                }
                ServiceEvent::ServiceResolved(info) => {
                    let id = property_value(&info, "id")
                        .unwrap_or_else(|| info.get_fullname().to_string());
                    let name = property_value(&info, "fn").unwrap_or_else(|| id.clone());
                    let host = first_ipv4_addr(&info).map(|ip| ip.to_string()).or_else(|| {
                        info.get_hostname()
                            .to_string()
                            .strip_suffix('.')
                            .map(|s| s.to_string())
                    });
                    let port = info.get_port();
                    if let Ok(mut map) = state.providers.cast.discovered.lock() {
                        let now = std::time::Instant::now();
                        map.insert(
                            id.clone(),
                            DiscoveredCast {
                                id: id.clone(),
                                name,
                                host,
                                port,
                                last_seen: now,
                            },
                        );
                    }
                    state.events.outputs_changed();
                    fullname_to_id.insert(info.get_fullname().to_string(), id);
                }
                ServiceEvent::ServiceRemoved(name, _) => {
                    if let Some(id) = fullname_to_id.remove(&name) {
                        if let Ok(mut map) = state.providers.cast.discovered.lock() {
                            map.remove(&id);
                        }
                        state.events.outputs_changed();
                        tracing::info!(cast_id = %id, "mdns: cast removed");
                    }
                }
                _ => {}
            }
        }
    });
}

/// Spawn periodic health checker for discovered bridges.
pub(crate) fn spawn_discovered_health_watcher(state: web::Data<AppState>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(15));
            let snapshot = match state.providers.bridge.discovered_bridges.lock() {
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
                    if let Ok(mut map) = state.providers.bridge.discovered_bridges.lock() {
                        if let Some(entry) = map.get_mut(&id) {
                            entry.last_seen = now;
                        }
                    }
                } else if now.duration_since(last_seen) > std::time::Duration::from_secs(60) {
                    let active_bridge_id = state
                        .providers
                        .bridge
                        .bridges
                        .lock()
                        .ok()
                        .and_then(|s| s.active_bridge_id.clone());
                    if active_bridge_id.as_deref() == Some(&id) {
                        continue;
                    }
                    if let Ok(mut map) = state.providers.bridge.discovered_bridges.lock() {
                        map.remove(&id);
                    }
                    if let Ok(mut cache) = state.providers.bridge.device_cache.lock() {
                        cache.remove(&id);
                    }
                    if let Ok(mut cache) = state.providers.bridge.status_cache.lock() {
                        cache.remove(&id);
                    }
                    state.events.outputs_changed();
                    tracing::info!(bridge_id = %id, "mdns: bridge removed (health check)");
                }
            }
        }
    });
}

/// Return true when `/health` endpoint responds with success.
fn ping_bridge(http_addr: std::net::SocketAddr) -> bool {
    let url = format!("http://{http_addr}/health");
    let resp = ureq::get(&url)
        .config()
        .timeout_per_call(Some(std::time::Duration::from_secs(2)))
        .build()
        .call();
    resp.map(|r| r.status().is_success()).unwrap_or(false)
}

/// Read TXT property value and strip optional `key=` prefix.
fn property_value(info: &mdns_sd::ResolvedService, key: &str) -> Option<String> {
    info.get_property(key)
        .map(|p| p.val_str().to_string())
        .map(|s| s.strip_prefix(&format!("{key}=")).unwrap_or(&s).to_string())
}

/// Return first resolved IPv4 address from mDNS service info.
fn first_ipv4_addr(info: &mdns_sd::ResolvedService) -> Option<std::net::Ipv4Addr> {
    info.get_addresses().iter().find_map(|ip| match ip {
        mdns_sd::ScopedIp::V4(v4) => Some(*v4.addr()),
        _ => None,
    })
}

/// Require matching major version between discovered bridge and server.
fn is_bridge_version_compatible(version: Option<&str>) -> bool {
    let Some(version) = version else {
        return false;
    };
    let Some(bridge_major) = parse_major(version) else {
        return false;
    };
    let Some(server_major) = parse_major(env!("CARGO_PKG_VERSION")) else {
        return false;
    };
    bridge_major == server_major
}

/// Parse major semver component from version string.
fn parse_major(version: &str) -> Option<u64> {
    version.split('.').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_bridge_returns_false_on_unreachable() {
        let addr: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
        assert!(!ping_bridge(addr));
    }

    #[test]
    fn property_value_strips_key_prefix() {
        let mut props = std::collections::HashMap::new();
        props.insert("id".to_string(), "id=bridge-1".to_string());
        let info = mdns_sd::ServiceInfo::new(
            "_audio-bridge._tcp.local.",
            "bridge-1",
            "bridge.local.",
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            5556,
            props,
        )
        .unwrap()
        .as_resolved_service();
        assert_eq!(property_value(&info, "id"), Some("bridge-1".to_string()));
    }

    #[test]
    fn first_ipv4_addr_returns_address() {
        let info = mdns_sd::ServiceInfo::new(
            "_audio-bridge._tcp.local.",
            "bridge-1",
            "bridge.local.",
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 2)),
            5556,
            std::collections::HashMap::new(),
        )
        .unwrap()
        .as_resolved_service();
        assert_eq!(
            first_ipv4_addr(&info),
            Some(std::net::Ipv4Addr::new(127, 0, 0, 2))
        );
    }
}
