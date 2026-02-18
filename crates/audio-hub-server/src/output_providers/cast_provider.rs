//! Chromecast (Google Cast) output provider.
//!
//! Currently supports discovery and listing only. Playback control is not implemented yet.

use async_trait::async_trait;

use crate::models::{OutputCapabilities, OutputInfo, OutputsResponse, ProviderInfo, StatusResponse};
use crate::output_providers::registry::{OutputProvider, ProviderError};
use crate::state::AppState;

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
        let snapshot = state.providers.cast.discovered.lock().ok();
        snapshot
            .map(|map| {
                map.values()
                    .map(|device| CastDevice {
                        id: device.id.clone(),
                        name: device.name.clone(),
                        host: device.host.clone(),
                        port: device.port,
                    })
                    .map(|device| Self::device_output_info(&device, &active_id))
                    .collect()
            })
            .unwrap_or_default()
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

    async fn ensure_active_connected(&self, state: &AppState) -> Result<(), ProviderError> {
        let active_id = Self::active_output_id(state)
            .ok_or_else(|| ProviderError::Unavailable("no active output selected".to_string()))?;
        let Some(device_id) = Self::parse_output_id(&active_id) else {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        };
        let found = state
            .providers
            .cast
            .discovered
            .lock()
            .ok()
            .and_then(|map| map.get(&device_id).cloned());
        if found.is_some() {
            Ok(())
        } else {
            Err(ProviderError::Unavailable("cast device offline".to_string()))
        }
    }

    async fn select_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError> {
        let Some(device_id) = Self::parse_output_id(output_id) else {
            return Err(ProviderError::BadRequest("invalid output id".to_string()));
        };
        let found = state
            .providers
            .cast
            .discovered
            .lock()
            .ok()
            .and_then(|map| map.get(&device_id).cloned());
        if found.is_none() {
            return Err(ProviderError::Unavailable("cast device offline".to_string()));
        }

        {
            let player = state.providers.bridge.player.lock().unwrap();
            let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
        }
        {
            let mut bridges = state.providers.bridge.bridges.lock().unwrap();
            bridges.active_output_id = Some(output_id.to_string());
            bridges.active_bridge_id = None;
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
