//! mDNS advertisement for bridge discovery.
//!
//! Publishes the bridge API address with id/name metadata.

use mdns_sd::{ServiceDaemon, ServiceInfo};

/// Handle for an active mDNS advertisement.
pub(crate) struct MdnsAdvertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

/// Start advertising the bridge via mDNS.
pub(crate) fn spawn_mdns_advertiser(
    http_bind: std::net::SocketAddr,
) -> Option<MdnsAdvertiser> {
    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "mdns: daemon start failed");
            return None;
        }
    };
    let service_type = "_audio-bridge._tcp.local.";
    let host_base = resolve_host_base();
    let host = format_host(&host_base);
    let id = resolve_bridge_id(&host_base);
    let name = resolve_bridge_name(&host_base);
    let instance = format!("{id}");
    let properties: std::collections::HashMap<String, String> = [
        ("id".to_string(), id.clone()),
        ("name".to_string(), name.clone()),
        ("api_port".to_string(), http_bind.port().to_string()),
    ]
    .into_iter()
    .collect();
    let ip = if http_bind.ip().is_unspecified() {
        local_ip().unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
    } else {
        http_bind.ip()
    };
    let info = ServiceInfo::new(
        service_type,
        &instance,
        &host,
        ip,
        http_bind.port(),
        properties,
    )
    .ok()?;
    let fullname = info.get_fullname().to_string();
    if let Err(e) = daemon.register(info) {
        tracing::warn!(error = %e, "mdns: register failed");
        return None;
    }
    tracing::info!(
        bridge_id = %id,
        bridge_name = %name,
        http_addr = %std::net::SocketAddr::new(ip, http_bind.port()),
        "mdns: advertised bridge"
    );
    Some(MdnsAdvertiser { daemon, fullname })
}

impl MdnsAdvertiser {
    /// Unregister and shutdown the mDNS daemon.
    pub(crate) fn shutdown(&self) {
        if let Ok(rx) = self.daemon.unregister(&self.fullname) {
            let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
        }
        if let Ok(rx) = self.daemon.shutdown() {
            let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
        }
    }
}

fn resolve_host_base() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| gethostname::gethostname().to_string_lossy().to_string())
}

fn format_host(host_base: &str) -> String {
    if host_base.ends_with(".local.") {
        host_base.to_string()
    } else {
        format!("{host_base}.local.")
    }
}

fn resolve_bridge_id(host_base: &str) -> String {
    std::env::var("BRIDGE_ID").unwrap_or_else(|_| host_base.to_string())
}

fn resolve_bridge_name(host_base: &str) -> String {
    std::env::var("BRIDGE_NAME").unwrap_or_else(|_| host_base.to_string())
}

/// Determine a best-effort local IP for advertisement.
fn local_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    if socket.connect("8.8.8.8:80").is_err() && socket.connect("1.1.1.1:80").is_err() {
        return None;
    }
    socket.local_addr().ok().map(|addr| addr.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_host_appends_local_suffix() {
        assert_eq!(format_host("bridge"), "bridge.local.");
        assert_eq!(format_host("bridge.local."), "bridge.local.");
    }
}
