//! Logs-related API handlers.

use actix_web::{HttpResponse, Responder, post, web};
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;

#[derive(Serialize, ToSchema)]
pub struct LogsClearResponse {
    pub cleared_at_ms: i64,
}

#[utoipa::path(
    post,
    path = "/logs/clear",
    responses(
        (status = 200, description = "Log buffer cleared", body = LogsClearResponse)
    )
)]
#[post("/logs/clear")]
/// Clear the in-memory log buffer.
pub async fn logs_clear(state: web::Data<AppState>) -> impl Responder {
    state.log_bus.clear();
    let cleared_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    HttpResponse::Ok().json(LogsClearResponse { cleared_at_ms })
}
