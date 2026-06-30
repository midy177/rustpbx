use axum::{
    Json, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_TENANT_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantContext {
    pub id: String,
    pub name: String,
    pub role: TenantRole,
}

impl TenantContext {
    pub fn default_tenant_admin() -> Self {
        Self {
            id: DEFAULT_TENANT_ID.to_string(),
            name: "Default".to_string(),
            role: TenantRole::TenantAdmin,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantRole {
    PlatformAdmin,
    TenantAdmin,
    TenantUser,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct TenantSummary {
    id: String,
    name: String,
    status: TenantStatus,
    domain: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum TenantStatus {
    Active,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
    tenant: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
    message: &'static str,
}

pub fn router() -> Router {
    Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/session", get(session))
        .route("/api/tenants", get(list_tenants))
}

async fn login(Json(payload): Json<LoginRequest>) -> Response {
    let _ = (&payload.username, &payload.password, &payload.tenant);
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(ErrorBody {
            error: "not_implemented",
            message: "CloudPBX SPA auth is not wired to the monolith user store yet.",
        }),
    )
        .into_response()
}

async fn session() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorBody {
            error: "unauthorized",
            message: "No CloudPBX SPA session is active.",
        }),
    )
        .into_response()
}

async fn list_tenants() -> Json<Vec<TenantSummary>> {
    Json(vec![TenantSummary {
        id: DEFAULT_TENANT_ID.to_string(),
        name: "Default".to_string(),
        status: TenantStatus::Active,
        domain: None,
    }])
}
