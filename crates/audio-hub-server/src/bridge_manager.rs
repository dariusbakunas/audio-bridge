pub(crate) fn parse_output_id(id: &str) -> Result<(String, String), String> {
    let mut parts = id.splitn(3, ':');
    let kind = parts.next().unwrap_or("");
    let bridge_id = parts.next().unwrap_or("");
    let device_id = parts.next().unwrap_or("");
    if kind != "bridge" || bridge_id.is_empty() || device_id.is_empty() {
        return Err("invalid output id".to_string());
    }
    Ok((bridge_id.to_string(), device_id.to_string()))
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
        seen_addrs.insert(b.http_addr);
        merged.push(b.clone());
    }
    for (id, b) in discovered {
        if seen.contains(id) {
            continue;
        }
        if seen_addrs.contains(&b.bridge.http_addr) {
            tracing::info!(
                bridge_id = %b.bridge.id,
                bridge_name = %b.bridge.name,
                http_addr = %b.bridge.http_addr,
                "merge: skipping discovered bridge with configured addr"
            );
            continue;
        }
        seen_addrs.insert(b.bridge.http_addr);
        merged.push(b.bridge.clone());
    }
    merged
}
