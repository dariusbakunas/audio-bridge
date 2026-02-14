//! Configuration loading and parsing.
//!
//! Defines the server config schema and resolves defaults.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use std::net::SocketAddr;

/// Top-level server configuration loaded from TOML.
#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// Bind address (host:port).
    pub bind: Option<String>,
    /// Media library root directory.
    pub media_dir: Option<String>,
    /// Public base URL used to construct stream URLs.
    pub public_base_url: Option<String>,
    /// Bridge definitions.
    pub bridges: Option<Vec<BridgeConfig>>,
    /// Optional active output id on startup.
    pub active_output: Option<String>,
    /// Enable local outputs.
    pub local_outputs: Option<bool>,
    /// Local provider id.
    pub local_id: Option<String>,
    /// Local provider display name.
    pub local_name: Option<String>,
    /// Optional local output device override.
    pub local_device: Option<String>,
    /// MusicBrainz enrichment settings.
    pub musicbrainz: Option<MusicBrainzConfig>,
    /// Optional TLS certificate path (PEM).
    pub tls_cert: Option<String>,
    /// Optional TLS private key path (PEM).
    pub tls_key: Option<String>,
}

/// Bridge config from TOML.
#[derive(Debug, Deserialize)]
pub struct BridgeConfig {
    /// Stable bridge id used in output ids.
    pub id: String,
    /// Display name (defaults to id).
    pub name: Option<String>,
    /// Bridge HTTP address (host:port).
    pub http_addr: String,
}

/// MusicBrainz configuration.
#[derive(Debug, Deserialize)]
pub struct MusicBrainzConfig {
    /// Enable MusicBrainz lookups during scans.
    pub enabled: Option<bool>,
    /// User-Agent string required by MusicBrainz (include contact info).
    pub user_agent: Option<String>,
    /// Optional base URL override (defaults to https://musicbrainz.org/ws/2).
    pub base_url: Option<String>,
    /// Minimum delay between requests in milliseconds (default: 1000).
    pub rate_limit_ms: Option<u64>,
}

/// Resolved bridge config with parsed socket address.
#[derive(Debug, Clone)]
pub struct BridgeConfigResolved {
    /// Bridge id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Parsed HTTP address.
    pub http_addr: SocketAddr,
}

impl ServerConfig {
    /// Load configuration from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read config {:?}", path))?;
        let cfg = toml::from_str::<ServerConfig>(&raw)
            .with_context(|| format!("parse config {:?}", path))?;
        Ok(cfg)
    }
}

/// Resolve bridge configs and parse their addresses.
pub fn bridges_from_config(cfg: &ServerConfig) -> Result<Vec<BridgeConfigResolved>> {
    let mut bridges = Vec::new();
    if let Some(cfg_bridges) = cfg.bridges.as_ref() {
        for bridge in cfg_bridges {
            let name = bridge.name.clone().unwrap_or_else(|| bridge.id.clone());
            let http_addr: SocketAddr = bridge
                .http_addr
                .parse()
                .with_context(|| format!("parse bridge http_addr {}", bridge.http_addr))?;
            bridges.push(BridgeConfigResolved {
                id: bridge.id.clone(),
                name,
                http_addr,
            });
        }
    }

    Ok(bridges)
}

/// Extract the media directory from config.
pub fn media_dir_from_config(cfg: &ServerConfig) -> Result<std::path::PathBuf> {
    let dir = cfg
        .media_dir
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("media_dir is required in config"))?;
    Ok(std::path::PathBuf::from(dir))
}

/// Parse an optional bind address from config.
pub fn bind_from_config(cfg: &ServerConfig) -> Result<Option<std::net::SocketAddr>> {
    let Some(bind) = cfg.bind.as_deref() else {
        return Ok(None);
    };
    let addr = bind
        .parse()
        .with_context(|| format!("parse bind {bind}"))?;
    Ok(Some(addr))
}

pub fn public_base_url_from_config(
    cfg: &ServerConfig,
    bind: std::net::SocketAddr,
    tls_enabled: bool,
) -> Result<String> {
    if let Some(url) = cfg.public_base_url.as_ref() {
        return Ok(url.trim_end_matches('/').to_string());
    }

    if bind.ip().is_unspecified() {
        return Err(anyhow::anyhow!(
            "public_base_url is required when bind is 0.0.0.0"
        ));
    }

    let scheme = if tls_enabled { "https" } else { "http" };
    Ok(format!("{}://{}", scheme, bind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_base_url_uses_config_when_present() {
        let cfg = ServerConfig {
            bind: None,
            media_dir: None,
            public_base_url: Some("http://example.com/".to_string()),
            bridges: None,
            active_output: None,
            local_outputs: None,
            local_id: None,
            local_name: None,
            local_device: None,
            musicbrainz: None,
            tls_cert: None,
            tls_key: None,
        };
        let bind: std::net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let url = public_base_url_from_config(&cfg, bind, false).unwrap();
        assert_eq!(url, "http://example.com");
    }

    #[test]
    fn public_base_url_requires_explicit_when_unspecified_bind() {
        let cfg = ServerConfig {
            bind: None,
            media_dir: None,
            public_base_url: None,
            bridges: None,
            active_output: None,
            local_outputs: None,
            local_id: None,
            local_name: None,
            local_device: None,
            musicbrainz: None,
            tls_cert: None,
            tls_key: None,
        };
        let bind: std::net::SocketAddr = "0.0.0.0:8080".parse().unwrap();
        assert!(public_base_url_from_config(&cfg, bind, false).is_err());
    }

    #[test]
    fn bind_from_config_parses_when_present() {
        let cfg = ServerConfig {
            bind: Some("127.0.0.1:9000".to_string()),
            media_dir: None,
            public_base_url: None,
            bridges: None,
            active_output: None,
            local_outputs: None,
            local_id: None,
            local_name: None,
            local_device: None,
            musicbrainz: None,
            tls_cert: None,
            tls_key: None,
        };
        let addr = bind_from_config(&cfg).unwrap().unwrap();
        assert_eq!(addr, "127.0.0.1:9000".parse().unwrap());
    }
}
