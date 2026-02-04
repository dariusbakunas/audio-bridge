use actix_web::HttpResponse;

use crate::models::{OutputInfo, OutputsResponse, ProvidersResponse, StatusResponse};
use crate::output_providers::bridge_provider::BridgeProvider;
use crate::output_providers::local_provider::LocalProvider;
use crate::state::AppState;

pub(crate) trait OutputProvider: Send + Sync {
    fn list_providers(&self, state: &AppState) -> Vec<crate::models::ProviderInfo>;
    fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, HttpResponse>;
    fn list_outputs(&self, state: &AppState) -> Vec<OutputInfo>;
    fn can_handle_output_id(&self, output_id: &str) -> bool;
    fn can_handle_provider_id(&self, state: &AppState, provider_id: &str) -> bool;
    fn inject_active_output_if_missing(
        &self,
        state: &AppState,
        outputs: &mut Vec<OutputInfo>,
        active_output_id: &str,
    );
    fn ensure_active_connected<'a>(
        &'a self,
        state: &'a AppState,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), HttpResponse>> + Send + 'a>,
    >;
    fn select_output<'a>(
        &'a self,
        state: &'a AppState,
        output_id: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), HttpResponse>> + Send + 'a>,
    >;
    fn status_for_output<'a>(
        &'a self,
        state: &'a AppState,
        output_id: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<StatusResponse, HttpResponse>> + Send + 'a>,
    >;
}

pub(crate) struct OutputRegistry {
    providers: Vec<Box<dyn OutputProvider>>,
}

impl OutputRegistry {
    pub(crate) fn new(providers: Vec<Box<dyn OutputProvider>>) -> Self {
        Self { providers }
    }

    pub(crate) fn default() -> Self {
        Self::new(vec![Box::new(BridgeProvider), Box::new(LocalProvider)])
    }

    pub(crate) fn list_providers(&self, state: &AppState) -> ProvidersResponse {
        let mut providers = Vec::new();
        for provider in &self.providers {
            providers.extend(provider.list_providers(state));
        }
        ProvidersResponse { providers }
    }

    pub(crate) fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, HttpResponse> {
        for provider in &self.providers {
            if provider.can_handle_provider_id(state, provider_id) {
                return provider.outputs_for_provider(state, provider_id);
            }
        }
        Err(HttpResponse::BadRequest().body("unknown provider id"))
    }

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

    pub(crate) async fn select_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), HttpResponse> {
        for provider in &self.providers {
            if provider.can_handle_output_id(output_id) {
                return provider.select_output(state, output_id).await;
            }
        }
        Err(HttpResponse::BadRequest().body("invalid output id"))
    }

    pub(crate) async fn status_for_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<StatusResponse, HttpResponse> {
        for provider in &self.providers {
            if provider.can_handle_output_id(output_id) {
                return provider.status_for_output(state, output_id).await;
            }
        }
        Err(HttpResponse::BadRequest().body("invalid output id"))
    }

    pub(crate) async fn ensure_active_connected(
        &self,
        state: &AppState,
    ) -> Result<(), HttpResponse> {
        let active_id = state.bridge.bridges.lock().unwrap().active_output_id.clone();
        let Some(active_id) = active_id else {
            return Err(HttpResponse::ServiceUnavailable().body("no active output selected"));
        };
        for provider in &self.providers {
            if provider.can_handle_output_id(&active_id) {
                return provider.ensure_active_connected(state).await;
            }
        }
        Err(HttpResponse::BadRequest().body("invalid output id"))
    }
}
