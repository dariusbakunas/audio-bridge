use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use std::net::SocketAddr;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub bind: Option<String>,
    pub media_dir: Option<String>,
    pub bridges: Option<Vec<BridgeConfig>>,
    pub active_output: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BridgeConfig {
    pub id: String,
    pub name: Option<String>,
    pub addr: String,
    pub api_port: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct BridgeConfigResolved {
    pub id: String,
    pub name: String,
    pub addr: SocketAddr,
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
            let addr: SocketAddr = bridge
                .addr
                .parse()
                .with_context(|| format!("parse bridge addr {}", bridge.addr))?;
            let http_addr = match bridge.api_port {
                Some(port) => SocketAddr::new(addr.ip(), port),
                None => default_http_addr(addr)?,
            };
            bridges.push(BridgeConfigResolved {
                id: bridge.id.clone(),
                name,
                addr,
                http_addr,
            });
        }
    }

    if bridges.is_empty() {
        return Err(anyhow::anyhow!("config must define at least one bridge"));
    }

    Ok(bridges)
}

fn default_http_addr(addr: SocketAddr) -> Result<SocketAddr> {
    let port = addr.port();
    let http_port = port
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("cannot default http port for {addr}"))?;
    Ok(SocketAddr::new(addr.ip(), http_port))
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
