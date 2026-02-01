use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::models::{OutputCapabilities, OutputInfo};

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub bind: Option<String>,
    pub media_dir: Option<String>,
    pub outputs: Option<Vec<OutputConfig>>,
    pub active_output: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OutputConfig {
    pub id: String,
    pub kind: String,
    pub name: Option<String>,
    pub bridge_addr: Option<String>,
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

pub fn outputs_from_config(cfg: ServerConfig) -> Result<(Vec<OutputInfo>, String, std::net::SocketAddr)> {
    let mut outputs = Vec::new();
    let mut active_id = cfg.active_output.unwrap_or_else(|| "bridge:default".to_string());
    let mut bridge_addr: Option<std::net::SocketAddr> = None;

    if let Some(outs) = cfg.outputs {
        for out in outs {
            let name = out.name.clone().unwrap_or_else(|| out.id.clone());
            if out.kind == "bridge" {
                if let Some(addr) = out.bridge_addr.as_deref() {
                    if out.id == active_id {
                        bridge_addr = Some(addr.parse().with_context(|| format!("parse bridge_addr {addr}"))?);
                    }
                }
            }
            outputs.push(OutputInfo {
                id: out.id,
                kind: out.kind,
                name,
                state: "online".to_string(),
                capabilities: OutputCapabilities {
                    device_select: true,
                    volume: false,
                },
            });
        }
    }

    if outputs.is_empty() {
        return Err(anyhow::anyhow!("config must define at least one output"));
    }

    if !outputs.iter().any(|o| o.id == active_id) {
        active_id = outputs[0].id.clone();
    }

    let Some(addr) = bridge_addr else {
        return Err(anyhow::anyhow!(
            "active output must be a bridge with bridge_addr set"
        ));
    };

    Ok((outputs, active_id, addr))
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
