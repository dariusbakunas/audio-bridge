//! Bridge id parsing + merge helpers.
//!
//! Provides helpers for validating provider/output ids and merging discovery results.

/// Parse a bridge output id into `(bridge_id, device_id)`.
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

/// Parse a bridge provider id into `bridge_id`.
pub(crate) fn parse_provider_id(id: &str) -> Result<String, String> {
    let mut parts = id.splitn(2, ':');
    let kind = parts.next().unwrap_or("");
    let bridge_id = parts.next().unwrap_or("");
    if kind != "bridge" || bridge_id.is_empty() {
        return Err("invalid provider id".to_string());
    }
    Ok(bridge_id.to_string())
}

/// Merge configured and discovered bridges, preferring configured entries.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge(id: &str, addr: &str) -> crate::config::BridgeConfigResolved {
        crate::config::BridgeConfigResolved {
            id: id.to_string(),
            name: id.to_string(),
            http_addr: addr.parse().unwrap(),
        }
    }

    #[test]
    fn parse_output_id_accepts_valid() {
        let (bridge_id, device_id) = parse_output_id("bridge:one:device").unwrap();
        assert_eq!(bridge_id, "one");
        assert_eq!(device_id, "device");
    }

    #[test]
    fn parse_output_id_rejects_invalid() {
        assert!(parse_output_id("bridge::device").is_err());
        assert!(parse_output_id("local:one:device").is_err());
        assert!(parse_output_id("bridge:one").is_err());
    }

    #[test]
    fn parse_provider_id_accepts_valid() {
        let bridge_id = parse_provider_id("bridge:one").unwrap();
        assert_eq!(bridge_id, "one");
    }

    #[test]
    fn parse_provider_id_rejects_invalid() {
        assert!(parse_provider_id("bridge:").is_err());
        assert!(parse_provider_id("local:one").is_err());
    }

    #[test]
    fn merge_bridges_prefers_configured_and_skips_duplicate_addr() {
        let configured = vec![bridge("a", "127.0.0.1:5556")];
        let mut discovered = std::collections::HashMap::new();
        discovered.insert(
            "a".to_string(),
            crate::state::DiscoveredBridge {
                bridge: bridge("a", "127.0.0.1:5556"),
                last_seen: std::time::Instant::now(),
            },
        );
        discovered.insert(
            "b".to_string(),
            crate::state::DiscoveredBridge {
                bridge: bridge("b", "127.0.0.1:5556"),
                last_seen: std::time::Instant::now(),
            },
        );
        discovered.insert(
            "c".to_string(),
            crate::state::DiscoveredBridge {
                bridge: bridge("c", "127.0.0.1:5557"),
                last_seen: std::time::Instant::now(),
            },
        );

        let merged = merge_bridges(&configured, &discovered);
        let ids = merged.iter().map(|b| b.id.as_str()).collect::<Vec<_>>();

        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"c"));
        assert_eq!(merged.len(), 2);
    }
}
