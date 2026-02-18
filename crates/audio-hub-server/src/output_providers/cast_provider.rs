//! Chromecast (Google Cast) output provider.
//!
//! Currently supports discovery and listing only. Playback control is not implemented yet.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use mdns_sd::{ServiceDaemon, ServiceEvent};

use crate::models::{OutputCapabilities, OutputInfo, OutputsResponse, ProviderInfo, StatusResponse};
use crate::output_providers::registry::{OutputProvider, ProviderError};
use crate::state::AppState;

const CAST_SERVICE: &str = "_googlecast._tcp.local.";

#[derive(Debug, Clone)]
struct CastDevice {
    id: String,
    name: String,
    host: Option<String>,
    port: u16,
}

pub(crate) struct CastProvider;

impl CastProvider {
    fn provider_id() -> &'static str {
        "cast"
    }

    fn output_id(device_id: &str) -> String {
        format!("cast:{device_id}")
    }

    fn parse_output_id(output_id: &str) -> Option<String> {
        let mut parts = output_id.splitn(2, ':');
        let kind = parts.next().unwrap_or("");
        let id = parts.next().unwrap_or("");
        if kind != "cast" || id.is_empty() {
            return None;
        }
        Some(id.to_string())
    }

    fn active_output_id(state: &AppState) -> Option<String> {
        state.providers.bridge.bridges.lock().unwrap().active_output_id.clone()
    }

    fn discover_devices(timeout: Duration) -> Vec<CastDevice> {
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };
        let receiver = match daemon.browse(CAST_SERVICE) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        let deadline = Instant::now() + timeout;
        let mut devices = std::collections::HashMap::new();
        while Instant::now() < deadline {
            for event in receiver.try_iter() {
                if let ServiceEvent::ServiceResolved(info) = event {
                    let id = property_value(&info, "id")
                        .unwrap_or_else(|| info.get_fullname().to_string());
                    let name = property_value(&info, "fn").unwrap_or_else(|| id.clone());
                    let host = first_ipv4_addr(&info)
                        .map(|ip| ip.to_string())
                        .or_else(|| info.get_hostname().to_string().strip_suffix('.').map(|s| s.to_string()));
                    let port = info.get_port();
                    devices.insert(
                        id.clone(),
                        CastDevice {
                            id,
                            name,
                            host,
                            port,
                        },
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        devices.into_values().collect()
    }

    fn device_output_info(device: &CastDevice, active_id: &Option<String>) -> OutputInfo {
        let id = Self::output_id(&device.id);
        let state = if active_id.as_deref() == Some(&id) {
            "active"
        } else {
            "online"
        };
        let name = if let Some(host) = device.host.as_deref() {
            format!("{} ({})", device.name, host)
        } else {
            device.name.clone()
        };
        OutputInfo {
            id,
            kind: "cast".to_string(),
            name,
            state: state.to_string(),
            provider_id: Some(Self::provider_id().to_string()),
            provider_name: Some("Chromecast".to_string()),
            supported_rates: None,
            capabilities: OutputCapabilities {
                device_select: false,
                volume: false,
            },
        }
    }
}

#[async_trait]
impl OutputProvider for CastProvider {
    fn list_providers(&self, _state: &AppState) -> Vec<ProviderInfo> {
        vec![ProviderInfo {
            id: Self::provider_id().to_string(),
            kind: "cast".to_string(),
            name: "Chromecast".to_string(),
            state: "available".to_string(),
            capabilities: OutputCapabilities {
                device_select: false,
                volume: false,
            },
        }]
    }

    async fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, ProviderError> {
        if provider_id != Self::provider_id() {
            return Err(ProviderError::BadRequest("unknown provider id".to_string()));
        }
        let outputs = self.list_outputs(state).await;
        let active_id = Self::active_output_id(state).filter(|id| id.starts_with("cast:"));
        Ok(OutputsResponse { active_id, outputs })
    }

    async fn list_outputs(&self, state: &AppState) -> Vec<OutputInfo> {
        let active_id = Self::active_output_id(state);
        let devices = tokio::task::spawn_blocking(|| Self::discover_devices(Duration::from_millis(250)))
            .await
            .unwrap_or_default();
        devices
            .iter()
            .map(|device| Self::device_output_info(device, &active_id))
            .collect()
    }

    fn can_handle_output_id(&self, output_id: &str) -> bool {
        output_id.starts_with("cast:")
    }

    fn can_handle_provider_id(&self, _state: &AppState, provider_id: &str) -> bool {
        provider_id == Self::provider_id()
    }

    fn inject_active_output_if_missing(
        &self,
        _state: &AppState,
        _outputs: &mut Vec<OutputInfo>,
        _active_output_id: &str,
    ) {
    }

    async fn ensure_active_connected(&self, _state: &AppState) -> Result<(), ProviderError> {
        Err(ProviderError::Unavailable(
            "cast playback not implemented".to_string(),
        ))
    }

    async fn select_output(
        &self,
        _state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError> {
        if Self::parse_output_id(output_id).is_none() {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        }
        Err(ProviderError::Unavailable(
            "cast playback not implemented".to_string(),
        ))
    }

    async fn status_for_output(
        &self,
        _state: &AppState,
        output_id: &str,
    ) -> Result<StatusResponse, ProviderError> {
        if Self::parse_output_id(output_id).is_none() {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        }
        Err(ProviderError::Unavailable(
            "cast playback not implemented".to_string(),
        ))
    }

    async fn stop_output(
        &self,
        _state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError> {
        if Self::parse_output_id(output_id).is_none() {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        }
        Err(ProviderError::Unavailable(
            "cast playback not implemented".to_string(),
        ))
    }
}

fn property_value(info: &mdns_sd::ResolvedService, key: &str) -> Option<String> {
    info.get_property(key)
        .map(|p| p.val_str().to_string())
        .map(|s| s.strip_prefix(&format!("{key}=")).unwrap_or(&s).to_string())
}

fn first_ipv4_addr(info: &mdns_sd::ResolvedService) -> Option<std::net::Ipv4Addr> {
    info.get_addresses()
        .iter()
        .find_map(|ip| match ip {
            mdns_sd::ScopedIp::V4(v4) => Some(*v4.addr()),
            _ => None,
        })
}
