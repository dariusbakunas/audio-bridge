//! Output-related API handlers.

use actix_web::{HttpResponse, Responder, get, post, web};

use crate::bridge_manager::parse_output_id;
use crate::bridge_manager::{merge_bridges, parse_provider_id};
use crate::bridge_transport::BridgeTransportClient;
use crate::models::{
    BridgeUnregisterRequest, BridgeUnregisterResponse, OutputSelectRequest, OutputSettings,
    OutputSettingsResponse, OutputsResponse, ProviderOutputs, ProvidersResponse,
};
use crate::state::AppState;

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
    match state
        .output
        .controller
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
    HttpResponse::Ok().json(OutputSettingsResponse {
        settings,
        providers: payload,
    })
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

    // Re-apply exclusive mode for the active bridge output immediately so users
    // don't need to reselect the output for the change to take effect.
    let active_bridge_target = {
        let bridges = state
            .providers
            .bridge
            .bridges
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(active_output_id) = bridges.active_output_id.clone() {
            if let Ok((bridge_id, device_id)) = parse_output_id(&active_output_id) {
                let http_addr = bridges
                    .bridges
                    .iter()
                    .find(|b| b.id == bridge_id)
                    .map(|b| b.http_addr);
                http_addr.map(|addr| (addr, device_id, active_output_id))
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some((http_addr, device_id, active_output_id)) = active_bridge_target {
        let exclusive = new_settings.is_exclusive(&active_output_id);
        if let Err(err) = BridgeTransportClient::new(http_addr)
            .set_device_by_id(&device_id, Some(exclusive))
            .await
        {
            tracing::warn!(
                output_id = %active_output_id,
                device_id = %device_id,
                bridge_addr = %http_addr,
                error = %err,
                "failed to re-apply exclusive mode for active bridge output"
            );
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
pub async fn provider_refresh(state: web::Data<AppState>, id: web::Path<String>) -> impl Responder {
    match state
        .output
        .controller
        .refresh_provider(&state, id.as_str())
        .await
    {
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
    match state
        .output
        .controller
        .select_output(&state, &body.id)
        .await
    {
        Ok(()) => {
            state.events.outputs_changed();
            HttpResponse::Ok().finish()
        }
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/providers/bridge/unregister",
    request_body = BridgeUnregisterRequest,
    responses(
        (status = 200, description = "Bridge unregistered", body = BridgeUnregisterResponse),
        (status = 400, description = "Invalid request")
    )
)]
#[post("/providers/bridge/unregister")]
/// Unregister a bridge immediately on graceful bridge shutdown.
pub async fn bridge_unregister(
    state: web::Data<AppState>,
    body: web::Json<BridgeUnregisterRequest>,
) -> impl Responder {
    let bridge_id = body.bridge_id.trim().to_string();
    if bridge_id.is_empty() {
        return HttpResponse::BadRequest().body("bridge_id is required");
    }
    let output_prefix = format!("bridge:{bridge_id}:");

    let removed_discovered = state
        .providers
        .bridge
        .discovered_bridges
        .lock()
        .ok()
        .and_then(|mut map| map.remove(&bridge_id))
        .is_some();
    if let Ok(mut cache) = state.providers.bridge.device_cache.lock() {
        cache.remove(&bridge_id);
    }
    if let Ok(mut cache) = state.providers.bridge.status_cache.lock() {
        cache.remove(&bridge_id);
    }
    if let Ok(mut done) = state.providers.bridge.stop_on_join_done.lock() {
        done.remove(&bridge_id);
    }

    let mut cleared_active_output = false;
    if let Ok(mut bridges) = state.providers.bridge.bridges.lock() {
        let active_matches = bridges
            .active_output_id
            .as_deref()
            .map(|id| id.starts_with(&output_prefix))
            .unwrap_or(false);
        let bridge_matches = bridges.active_bridge_id.as_deref() == Some(bridge_id.as_str());
        if active_matches || bridge_matches {
            bridges.active_output_id = None;
            bridges.active_bridge_id = None;
            cleared_active_output = true;
        }
    }
    if cleared_active_output {
        if let Ok(player) = state.providers.bridge.player.lock() {
            let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::StopSilent);
        }
    }

    let sessions_to_release = crate::session_registry::list_sessions()
        .into_iter()
        .filter_map(|session| {
            let active = session.active_output_id.as_deref()?;
            if active.starts_with(&output_prefix) {
                Some(session.id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for session_id in &sessions_to_release {
        let _ = crate::session_registry::release_output(session_id);
        if let Ok(mut cache) = state.output.session_status_cache.lock() {
            cache.remove(session_id);
        }
    }

    tracing::info!(
        bridge_id = %bridge_id,
        removed_discovered,
        released_sessions = sessions_to_release.len(),
        cleared_active_output,
        "bridge unregistered via callback"
    );
    state.events.outputs_changed();
    state.events.status_changed();
    HttpResponse::Ok().json(BridgeUnregisterResponse {
        removed_discovered,
        released_sessions: sessions_to_release.len(),
        cleared_active_output,
    })
}

/// Ensure `active_id` points to an existing output entry.
pub(crate) fn normalize_outputs_response(mut resp: OutputsResponse) -> OutputsResponse {
    if let Some(active_id) = resp.active_id.as_deref() {
        if !resp.outputs.iter().any(|o| o.id == active_id) {
            resp.active_id = None;
        }
    }
    resp
}

/// Resolve provider address string for bridge-backed provider id.
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
