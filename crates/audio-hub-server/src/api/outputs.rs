//! Output-related API handlers.

use actix_web::{get, post, web, HttpResponse, Responder};

use crate::models::{
    OutputSelectRequest,
    OutputSettings,
    OutputSettingsResponse,
    OutputsResponse,
    ProviderOutputs,
    ProvidersResponse,
};
use crate::state::AppState;
use crate::bridge_manager::{merge_bridges, parse_provider_id};

#[utoipa::path(
    get,
    path = "/providers",
    responses(
        (status = 200, description = "Available output providers", body = ProvidersResponse)
    )
)]
#[get("/providers")]
/// List all available output providers.
pub async fn providers_list(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok().json(state.output.controller.list_providers(&state))
}

#[utoipa::path(
    get,
    path = "/providers/{id}/outputs",
    responses(
        (status = 200, description = "Provider outputs", body = OutputsResponse),
        (status = 400, description = "Unknown provider"),
        (status = 500, description = "Provider unavailable")
    )
)]
#[get("/providers/{id}/outputs")]
/// List outputs for the requested provider.
pub async fn provider_outputs_list(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    match state.output.controller
        .outputs_for_provider(&state, id.as_str())
        .await
    {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/outputs",
    responses(
        (status = 200, description = "Available outputs", body = OutputsResponse)
    )
)]
#[get("/outputs")]
/// List all outputs across providers.
pub async fn outputs_list(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok().json(normalize_outputs_response(
        state.output.controller.list_outputs(&state).await,
    ))
}

#[utoipa::path(
    get,
    path = "/outputs/settings",
    responses(
        (status = 200, description = "Output settings", body = OutputSettingsResponse)
    )
)]
#[get("/outputs/settings")]
/// Fetch output settings and unfiltered provider outputs.
pub async fn outputs_settings(state: web::Data<AppState>) -> impl Responder {
    let settings = state
        .output_settings
        .lock()
        .map(|s| s.to_api())
        .unwrap_or_default();
    let providers = state.output.controller.list_providers(&state).providers;
    let mut payload = Vec::new();
    for provider in providers {
        let outputs = match state
            .output
            .controller
            .outputs_for_provider_raw(&state, provider.id.as_str())
            .await
        {
            Ok(resp) => resp.outputs,
            Err(err) => return err.into_response(),
        };
        let address = provider_address(&state, &provider.id);
        payload.push(ProviderOutputs {
            provider,
            address,
            outputs,
        });
    }
    HttpResponse::Ok().json(OutputSettingsResponse { settings, providers: payload })
}

#[utoipa::path(
    post,
    path = "/outputs/settings",
    request_body = OutputSettings,
    responses(
        (status = 200, description = "Settings saved", body = OutputSettings)
    )
)]
#[post("/outputs/settings")]
/// Update output settings (disabled outputs and renames).
pub async fn outputs_settings_update(
    state: web::Data<AppState>,
    body: web::Json<OutputSettings>,
) -> impl Responder {
    let new_settings = crate::state::OutputSettingsState::from_api(&body);
    {
        let mut guard = state
            .output_settings
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        *guard = new_settings.clone();
    }

    if let Some(path) = state.config_path.as_ref() {
        if let Err(err) = crate::config::update_output_settings(path, &new_settings.to_config()) {
            return HttpResponse::InternalServerError().body(format!("{err:#}"));
        }
    } else {
        return HttpResponse::InternalServerError().body("config path unavailable");
    }

    if let Ok(mut bridges) = state.providers.bridge.bridges.lock() {
        if let Some(active_id) = bridges.active_output_id.as_ref() {
            if new_settings.disabled.contains(active_id) {
                bridges.active_output_id = None;
                bridges.active_bridge_id = None;
                if let Ok(player) = state.providers.bridge.player.lock() {
                    let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Stop);
                }
            }
        }
    }

    state.events.outputs_changed();
    HttpResponse::Ok().json(new_settings.to_api())
}

#[utoipa::path(
    post,
    path = "/providers/{id}/refresh",
    params(
        ("id" = String, Path, description = "Provider id")
    ),
    responses(
        (status = 200, description = "Provider refreshed"),
        (status = 400, description = "Unknown provider"),
        (status = 500, description = "Provider unavailable")
    )
)]
#[post("/providers/{id}/refresh")]
/// Refresh outputs for the requested provider.
pub async fn provider_refresh(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    match state.output.controller.refresh_provider(&state, id.as_str()).await {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/outputs/select",
    request_body = OutputSelectRequest,
    responses(
        (status = 200, description = "Active output set"),
        (status = 400, description = "Unknown output")
    )
)]
#[post("/outputs/select")]
/// Select the active output.
pub async fn outputs_select(
    state: web::Data<AppState>,
    body: web::Json<OutputSelectRequest>,
) -> impl Responder {
    match state.output.controller.select_output(&state, &body.id).await {
        Ok(()) => {
            state.events.outputs_changed();
            HttpResponse::Ok().finish()
        }
        Err(err) => err.into_response(),
    }
}

pub(crate) fn normalize_outputs_response(mut resp: OutputsResponse) -> OutputsResponse {
    if let Some(active_id) = resp.active_id.as_deref() {
        if !resp.outputs.iter().any(|o| o.id == active_id) {
            resp.active_id = None;
        }
    }
    resp
}

fn provider_address(state: &AppState, provider_id: &str) -> Option<String> {
    let bridge_id = parse_provider_id(provider_id).ok()?;
    let bridges_state = state.providers.bridge.bridges.lock().ok()?;
    let discovered = state.providers.bridge.discovered_bridges.lock().ok()?;
    let merged = merge_bridges(&bridges_state.bridges, &discovered);
    merged
        .iter()
        .find(|b| b.id == bridge_id)
        .map(|b| b.http_addr.to_string())
}
