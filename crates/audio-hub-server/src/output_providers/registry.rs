//! Output provider registry and adapter glue.
//!
//! Routes requests to the active provider and normalizes provider errors.

use actix_web::HttpResponse;
use async_trait::async_trait;

use crate::models::{
    OutputInfo, OutputsResponse, ProvidersResponse, SessionVolumeResponse, StatusResponse,
};
use crate::output_providers::bridge_provider::BridgeProvider;
use crate::output_providers::cast_provider::CastProvider;
use crate::output_providers::local_provider::LocalProvider;
use crate::state::AppState;
use tracing::warn;

#[derive(Debug)]
/// Normalized error type produced by output providers.
pub(crate) enum ProviderError {
    /// The request is invalid or references an unknown id.
    BadRequest(String),
    /// The provider is offline or unavailable.
    Unavailable(String),
    /// An unexpected internal error.
    Internal(String),
}

impl ProviderError {
    /// Convert a provider error into an HTTP response.
    pub(crate) fn into_response(self) -> HttpResponse {
        match self {
            ProviderError::BadRequest(msg) => HttpResponse::BadRequest().body(msg),
            ProviderError::Unavailable(msg) => HttpResponse::ServiceUnavailable().body(msg),
            ProviderError::Internal(msg) => HttpResponse::InternalServerError().body(msg),
        }
    }
}

#[async_trait]
/// Adapter contract implemented by each output provider backend.
pub(crate) trait OutputProvider: Send + Sync {
    /// List providers exposed by this implementation.
    fn list_providers(&self, state: &AppState) -> Vec<crate::models::ProviderInfo>;
    /// List outputs for a specific provider id.
    async fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, ProviderError>;
    /// List all outputs exposed by this provider.
    async fn list_outputs(&self, state: &AppState) -> Vec<OutputInfo>;
    /// Return true if this provider can handle the output id.
    fn can_handle_output_id(&self, output_id: &str) -> bool;
    /// Return true if this provider can handle the provider id.
    fn can_handle_provider_id(&self, state: &AppState, provider_id: &str) -> bool;
    /// Ensure the active output is present even if missing from discovery.
    fn inject_active_output_if_missing(
        &self,
        state: &AppState,
        outputs: &mut Vec<OutputInfo>,
        active_output_id: &str,
    );
    /// Ensure the active output is connected.
    async fn ensure_active_connected(&self, state: &AppState) -> Result<(), ProviderError>;
    /// Select the active output for this provider.
    async fn select_output(&self, state: &AppState, output_id: &str) -> Result<(), ProviderError>;
    /// Return status for the requested output id.
    async fn status_for_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<StatusResponse, ProviderError>;
    /// Stop playback on a specific output id (best-effort).
    async fn stop_output(&self, state: &AppState, output_id: &str) -> Result<(), ProviderError>;
    /// Refresh provider devices (best-effort).
    async fn refresh_provider(
        &self,
        _state: &AppState,
        _provider_id: &str,
    ) -> Result<(), ProviderError> {
        Ok(())
    }
    /// Return volume for the requested output id.
    async fn volume_for_output(
        &self,
        _state: &AppState,
        _output_id: &str,
    ) -> Result<SessionVolumeResponse, ProviderError> {
        Err(ProviderError::Unavailable(
            "volume control unavailable for this output".to_string(),
        ))
    }
    /// Set volume for the requested output id.
    async fn set_volume_for_output(
        &self,
        _state: &AppState,
        _output_id: &str,
        _value: u8,
    ) -> Result<SessionVolumeResponse, ProviderError> {
        Err(ProviderError::Unavailable(
            "volume control unavailable for this output".to_string(),
        ))
    }
    /// Set mute state for the requested output id.
    async fn set_mute_for_output(
        &self,
        _state: &AppState,
        _output_id: &str,
        _muted: bool,
    ) -> Result<SessionVolumeResponse, ProviderError> {
        Err(ProviderError::Unavailable(
            "volume control unavailable for this output".to_string(),
        ))
    }
}

/// Provider multiplexer that routes operations by provider/output id.
pub(crate) struct OutputRegistry {
    providers: Vec<Box<dyn OutputProvider>>,
}

impl OutputRegistry {
    /// Create a registry from an explicit provider list.
    pub(crate) fn new(providers: Vec<Box<dyn OutputProvider>>) -> Self {
        Self { providers }
    }

    /// Create a registry with the default providers.
    pub(crate) fn default() -> Self {
        Self::new(vec![
            Box::new(BridgeProvider),
            Box::new(LocalProvider),
            Box::new(CastProvider),
        ])
    }

    /// List providers across all implementations.
    pub(crate) fn list_providers(&self, state: &AppState) -> ProvidersResponse {
        let mut providers = Vec::new();
        for provider in &self.providers {
            providers.extend(provider.list_providers(state));
        }
        ProvidersResponse { providers }
    }

    /// List outputs for a specific provider id.
    pub(crate) async fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, ProviderError> {
        let mut resp = self.outputs_for_provider_raw(state, provider_id).await?;
        apply_output_settings(&mut resp, state);
        Ok(resp)
    }

    /// List outputs for a specific provider without applying settings.
    pub(crate) async fn outputs_for_provider_raw(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, ProviderError> {
        for provider in &self.providers {
            if provider.can_handle_provider_id(state, provider_id) {
                return provider.outputs_for_provider(state, provider_id).await;
            }
        }
        Err(ProviderError::BadRequest("unknown provider id".to_string()))
    }

    /// List all outputs across providers and ensure active output is present.
    pub(crate) async fn list_outputs(&self, state: &AppState) -> OutputsResponse {
        let mut resp = self.list_outputs_raw(state).await;
        apply_output_settings(&mut resp, state);
        resp
    }

    /// List all outputs across providers without applying settings.
    pub(crate) async fn list_outputs_raw(&self, state: &AppState) -> OutputsResponse {
        let mut outputs = Vec::new();
        for provider in &self.providers {
            outputs.extend(provider.list_outputs(state).await);
        }
        let active_id = state
            .providers
            .bridge
            .bridges
            .lock()
            .unwrap()
            .active_output_id
            .clone();
        if let Some(active_id) = active_id.as_deref() {
            if !outputs.iter().any(|o| o.id == active_id) {
                for provider in &self.providers {
                    if provider.can_handle_output_id(active_id) {
                        provider.inject_active_output_if_missing(state, &mut outputs, active_id);
                        break;
                    }
                }
            }
        }
        OutputsResponse { active_id, outputs }
    }

    /// Select the active output across providers.
    pub(crate) async fn select_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError> {
        if is_output_disabled(state, output_id) {
            return Err(ProviderError::BadRequest("output is disabled".to_string()));
        }
        let previous = state
            .providers
            .bridge
            .bridges
            .lock()
            .unwrap()
            .active_output_id
            .clone();
        tracing::info!(
            from_output = ?previous,
            to_output = %output_id,
            "outputs: select output"
        );
        if previous.as_deref() == Some(output_id) {
            return self.ensure_active_connected(state).await;
        }
        if previous.as_deref() != Some(output_id) {
            if let Some(prev_id) = previous.as_deref() {
                tracing::info!(output_id = %prev_id, "outputs: stopping previous output");
                for provider in &self.providers {
                    if provider.can_handle_output_id(prev_id) {
                        if let Err(err) = provider.stop_output(state, prev_id).await {
                            warn!(
                                output_id = %prev_id,
                                error = ?err,
                                "failed to stop previous output"
                            );
                        }
                        break;
                    }
                }
            }
        }
        for provider in &self.providers {
            if provider.can_handle_output_id(output_id) {
                return provider.select_output(state, output_id).await;
            }
        }
        Err(ProviderError::BadRequest("invalid output id".to_string()))
    }

    /// Return status for the requested output id.
    pub(crate) async fn status_for_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<StatusResponse, ProviderError> {
        for provider in &self.providers {
            if provider.can_handle_output_id(output_id) {
                return provider.status_for_output(state, output_id).await;
            }
        }
        Err(ProviderError::BadRequest("invalid output id".to_string()))
    }

    /// Return volume for the requested output id.
    pub(crate) async fn volume_for_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<SessionVolumeResponse, ProviderError> {
        for provider in &self.providers {
            if provider.can_handle_output_id(output_id) {
                return provider.volume_for_output(state, output_id).await;
            }
        }
        Err(ProviderError::BadRequest("invalid output id".to_string()))
    }

    /// Set volume for the requested output id.
    pub(crate) async fn set_volume_for_output(
        &self,
        state: &AppState,
        output_id: &str,
        value: u8,
    ) -> Result<SessionVolumeResponse, ProviderError> {
        for provider in &self.providers {
            if provider.can_handle_output_id(output_id) {
                return provider
                    .set_volume_for_output(state, output_id, value)
                    .await;
            }
        }
        Err(ProviderError::BadRequest("invalid output id".to_string()))
    }

    /// Set mute for the requested output id.
    pub(crate) async fn set_mute_for_output(
        &self,
        state: &AppState,
        output_id: &str,
        muted: bool,
    ) -> Result<SessionVolumeResponse, ProviderError> {
        for provider in &self.providers {
            if provider.can_handle_output_id(output_id) {
                return provider.set_mute_for_output(state, output_id, muted).await;
            }
        }
        Err(ProviderError::BadRequest("invalid output id".to_string()))
    }

    /// Ensure the active output is connected and reachable.
    pub(crate) async fn ensure_active_connected(
        &self,
        state: &AppState,
    ) -> Result<(), ProviderError> {
        let active_id = state
            .providers
            .bridge
            .bridges
            .lock()
            .unwrap()
            .active_output_id
            .clone();
        let Some(active_id) = active_id else {
            return Err(ProviderError::Unavailable(
                "no active output selected".to_string(),
            ));
        };
        for provider in &self.providers {
            if provider.can_handle_output_id(&active_id) {
                return provider.ensure_active_connected(state).await;
            }
        }
        Err(ProviderError::BadRequest("invalid output id".to_string()))
    }

    /// Refresh devices for a provider (best-effort).
    pub(crate) async fn refresh_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<(), ProviderError> {
        for provider in &self.providers {
            if provider.can_handle_provider_id(state, provider_id) {
                return provider.refresh_provider(state, provider_id).await;
            }
        }
        Err(ProviderError::BadRequest("unknown provider id".to_string()))
    }
}

fn apply_output_settings(resp: &mut OutputsResponse, state: &AppState) {
    let settings = state
        .output_settings
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let disabled = &settings.disabled;
    let renames = &settings.renames;
    resp.outputs.retain(|o| !disabled.contains(&o.id));
    for output in &mut resp.outputs {
        if let Some(name) = renames.get(&output.id) {
            output.name = name.clone();
        }
    }
    if let Some(active_id) = resp.active_id.as_ref() {
        if disabled.contains(active_id) {
            resp.active_id = None;
        }
    }
}

fn is_output_disabled(state: &AppState, output_id: &str) -> bool {
    state
        .output_settings
        .lock()
        .map(|s| s.disabled.contains(output_id))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    struct MockProvider {
        output_id: String,
        provider_id: String,
        should_connect: bool,
        inject_called: Arc<Mutex<bool>>,
    }

    impl MockProvider {
        fn new(output_id: &str, provider_id: &str, should_connect: bool) -> Self {
            Self {
                output_id: output_id.to_string(),
                provider_id: provider_id.to_string(),
                should_connect,
                inject_called: Arc::new(Mutex::new(false)),
            }
        }
    }

    #[async_trait]
    impl OutputProvider for MockProvider {
        fn list_providers(&self, _state: &AppState) -> Vec<crate::models::ProviderInfo> {
            Vec::new()
        }

        async fn outputs_for_provider(
            &self,
            _state: &AppState,
            _provider_id: &str,
        ) -> Result<OutputsResponse, ProviderError> {
            Ok(OutputsResponse {
                active_id: None,
                outputs: Vec::new(),
            })
        }

        async fn list_outputs(&self, _state: &AppState) -> Vec<OutputInfo> {
            Vec::new()
        }

        fn can_handle_output_id(&self, output_id: &str) -> bool {
            output_id == self.output_id
        }

        fn can_handle_provider_id(&self, _state: &AppState, provider_id: &str) -> bool {
            provider_id == self.provider_id
        }

        fn inject_active_output_if_missing(
            &self,
            _state: &AppState,
            _outputs: &mut Vec<OutputInfo>,
            _active_output_id: &str,
        ) {
            if let Ok(mut flag) = self.inject_called.lock() {
                *flag = true;
            }
        }

        async fn ensure_active_connected(&self, _state: &AppState) -> Result<(), ProviderError> {
            if self.should_connect {
                Ok(())
            } else {
                Err(ProviderError::Unavailable("offline".to_string()))
            }
        }

        async fn select_output(
            &self,
            _state: &AppState,
            _output_id: &str,
        ) -> Result<(), ProviderError> {
            Ok(())
        }

        async fn status_for_output(
            &self,
            _state: &AppState,
            _output_id: &str,
        ) -> Result<StatusResponse, ProviderError> {
            Err(ProviderError::Unavailable("offline".to_string()))
        }

        async fn stop_output(
            &self,
            _state: &AppState,
            _output_id: &str,
        ) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    fn make_state(active_output_id: Option<String>) -> AppState {
        let tmp = std::env::temp_dir().join(format!(
            "audio-hub-provider-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let library = crate::library::scan_library(&tmp).expect("scan library");
        let (cmd_tx, _cmd_rx) = crossbeam_channel::unbounded();
        let bridges_state = Arc::new(Mutex::new(crate::state::BridgeState {
            bridges: Vec::new(),
            active_bridge_id: None,
            active_output_id,
        }));
        let bridge_state = Arc::new(crate::state::BridgeProviderState::new(
            cmd_tx,
            bridges_state,
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new(std::collections::HashMap::new())),
            "http://localhost".to_string(),
        ));
        let (local_cmd_tx, _local_cmd_rx) = crossbeam_channel::unbounded();
        let local_state = Arc::new(crate::state::LocalProviderState {
            enabled: false,
            id: "local".to_string(),
            name: "Local Host".to_string(),
            player: Arc::new(Mutex::new(crate::bridge::BridgePlayer {
                cmd_tx: local_cmd_tx,
            })),
            running: Arc::new(AtomicBool::new(false)),
        });
        let status = crate::status_store::StatusStore::new(
            Arc::new(Mutex::new(crate::state::PlayerStatus::default())),
            crate::events::EventBus::new(),
        );
        let queue = Arc::new(Mutex::new(crate::state::QueueState::default()));
        let queue_service = crate::queue_service::QueueService::new(
            queue,
            status.clone(),
            crate::events::EventBus::new(),
        );
        let playback_manager = crate::playback_manager::PlaybackManager::new(
            bridge_state.player.clone(),
            status,
            queue_service,
        );
        let device_selection = crate::state::DeviceSelectionState {
            local: Arc::new(Mutex::new(None)),
            bridge: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        let metadata_db = crate::metadata_db::MetadataDb::new(library.root()).unwrap();
        let cast_state = Arc::new(crate::state::CastProviderState::new());
        AppState::new(
            library,
            metadata_db,
            None,
            crate::state::MetadataWake::new(),
            bridge_state,
            local_state,
            cast_state,
            playback_manager,
            device_selection,
            crate::events::EventBus::new(),
            Arc::new(crate::events::LogBus::new(64)),
            Arc::new(Mutex::new(crate::state::OutputSettingsState::default())),
            None,
        )
    }

    #[test]
    fn list_outputs_injects_active_output_when_missing() {
        let active = "bridge:test:device".to_string();
        let state = make_state(Some(active.clone()));
        let provider = MockProvider::new(&active, "bridge", true);
        let inject_flag = provider.inject_called.clone();
        let registry = OutputRegistry::new(vec![Box::new(provider)]);

        let _ =
            actix_web::rt::System::new().block_on(async { registry.list_outputs(&state).await });

        assert!(*inject_flag.lock().unwrap());
    }

    #[test]
    fn ensure_active_connected_fails_without_active() {
        let state = make_state(None);
        let registry = OutputRegistry::new(Vec::new());
        let result = actix_web::rt::System::new()
            .block_on(async { registry.ensure_active_connected(&state).await });
        assert!(matches!(result, Err(ProviderError::Unavailable(_))));
    }

    #[test]
    fn ensure_active_connected_delegates_to_provider() {
        let active = "bridge:test:device".to_string();
        let state = make_state(Some(active.clone()));
        let provider = MockProvider::new(&active, "bridge", true);
        let registry = OutputRegistry::new(vec![Box::new(provider)]);

        let result = actix_web::rt::System::new()
            .block_on(async { registry.ensure_active_connected(&state).await });
        assert!(result.is_ok());
    }
}
