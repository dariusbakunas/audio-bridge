//! Output-related API handlers.

use actix_web::{get, post, web, HttpResponse, Responder};

use crate::models::{OutputSelectRequest, OutputsResponse, ProvidersResponse};
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
        state.output.controller.list_outputs(&state),
    ))
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
