use actix_web::{get, HttpResponse, Responder};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: &'static str,
}

/// Basic health check for clients and discovery.
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Hub server is healthy", body = HealthResponse)
    )
)]
#[get("/health")]
pub async fn health() -> impl Responder {
    HttpResponse::Ok().json(HealthResponse { status: "ok" })
}
