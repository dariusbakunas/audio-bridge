//! Configuration loading and parsing.
//!
//! Defines the server config schema and resolves defaults.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use std::net::SocketAddr;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub bind: Option<String>,
    pub media_dir: Option<String>,
    pub public_base_url: Option<String>,
    pub bridges: Option<Vec<BridgeConfig>>,
    pub active_output: Option<String>,
    pub local_outputs: Option<bool>,
    pub local_id: Option<String>,
    pub local_name: Option<String>,
    pub local_device: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BridgeConfig {
    pub id: String,
    pub name: Option<String>,
    pub http_addr: String,
}

#[derive(Debug, Clone)]
pub struct BridgeConfigResolved {
    pub id: String,
    pub name: String,
    pub http_addr: SocketAddr,
}

impl ServerConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read config {:?}", path))?;
        let cfg = toml::from_str::<ServerConfig>(&raw)
            .with_context(|| format!("parse config {:?}", path))?;
        Ok(cfg)
    }
}

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

pub fn media_dir_from_config(cfg: &ServerConfig) -> Result<std::path::PathBuf> {
    let dir = cfg
        .media_dir
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("media_dir is required in config"))?;
    Ok(std::path::PathBuf::from(dir))
}

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
) -> Result<String> {
    if let Some(url) = cfg.public_base_url.as_ref() {
        return Ok(url.trim_end_matches('/').to_string());
    }

    if bind.ip().is_unspecified() {
        return Err(anyhow::anyhow!(
            "public_base_url is required when bind is 0.0.0.0"
        ));
    }

    Ok(format!("http://{}", bind))
}
