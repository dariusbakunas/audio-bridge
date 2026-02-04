use actix_web::HttpResponse;

use crate::models::{OutputsResponse, ProvidersResponse, StatusResponse};
use crate::output_providers::registry::OutputRegistry;
use crate::state::AppState;

fn registry() -> OutputRegistry {
    OutputRegistry::default()
}

pub(crate) async fn select_output(state: &AppState, output_id: &str) -> Result<(), HttpResponse> {
    registry().select_output(state, output_id).await
}

pub(crate) async fn status_for_output(
    state: &AppState,
    output_id: &str,
) -> Result<StatusResponse, HttpResponse> {
    registry().status_for_output(state, output_id).await
}

pub(crate) fn outputs_for_provider(
    state: &AppState,
    provider_id: &str,
) -> Result<OutputsResponse, HttpResponse> {
    registry().outputs_for_provider(state, provider_id)
}

pub(crate) fn list_outputs(state: &AppState) -> OutputsResponse {
    registry().list_outputs(state)
}

pub(crate) fn list_providers(state: &AppState) -> ProvidersResponse {
    registry().list_providers(state)
}

pub(crate) async fn ensure_active_output_connected(
    state: &AppState,
) -> Result<(), HttpResponse> {
    registry().ensure_active_connected(state).await
}
