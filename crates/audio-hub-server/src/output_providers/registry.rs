//! Output provider registry and adapter glue.
//!
//! Routes requests to the active provider and normalizes provider errors.

use actix_web::HttpResponse;
use async_trait::async_trait;

use crate::models::{OutputInfo, OutputsResponse, ProvidersResponse, StatusResponse};
use crate::output_providers::bridge_provider::BridgeProvider;
use crate::output_providers::local_provider::LocalProvider;
use crate::state::AppState;

#[derive(Debug)]
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
    fn list_outputs(&self, state: &AppState) -> Vec<OutputInfo>;
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
    async fn select_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), ProviderError>;
    /// Return status for the requested output id.
    async fn status_for_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<StatusResponse, ProviderError>;
}

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
        Self::new(vec![Box::new(BridgeProvider), Box::new(LocalProvider)])
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
        for provider in &self.providers {
            if provider.can_handle_provider_id(state, provider_id) {
                return provider.outputs_for_provider(state, provider_id).await;
            }
        }
        Err(ProviderError::BadRequest("unknown provider id".to_string()))
    }

    /// List all outputs across providers and ensure active output is present.
    pub(crate) fn list_outputs(&self, state: &AppState) -> OutputsResponse {
        let mut outputs = Vec::new();
        for provider in &self.providers {
            outputs.extend(provider.list_outputs(state));
        }
        let active_id = state.bridge.bridges.lock().unwrap().active_output_id.clone();
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

    /// Ensure the active output is connected and reachable.
    pub(crate) async fn ensure_active_connected(
        &self,
        state: &AppState,
    ) -> Result<(), ProviderError> {
        let active_id = state.bridge.bridges.lock().unwrap().active_output_id.clone();
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
}
