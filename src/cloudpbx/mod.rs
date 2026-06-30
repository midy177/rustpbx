use axum::{
    Json, Router,
    extract::FromRequestParts,
    http::{HeaderMap, HeaderValue, StatusCode, request::Parts},
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

    pub fn from_headers(headers: &HeaderMap) -> Self {
        let mut tenant = Self::default_tenant_admin();
        if let Some(id) = header_string(headers, "x-tenant-id") {
            tenant.id = id;
        }
        if let Some(name) = header_string(headers, "x-tenant-name") {
            tenant.name = name;
        }
        if let Some(role) = headers
            .get("x-tenant-role")
            .and_then(|value| TenantRole::from_header(value))
        {
            tenant.role = role;
        }
        tenant
    }
}

impl<S> FromRequestParts<S> for TenantContext
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if let Some(ctx) = parts.extensions.get::<TenantContext>() {
            return Ok(ctx.clone());
        }
        Ok(TenantContext::from_headers(&parts.headers))
    }
}

fn header_string(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(name)?
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantRole {
    PlatformAdmin,
    TenantAdmin,
    TenantUser,
}

impl TenantRole {
    fn from_header(value: &HeaderValue) -> Option<Self> {
        match value.to_str().ok()?.trim() {
            "platform_admin" => Some(Self::PlatformAdmin),
            "tenant_admin" => Some(Self::TenantAdmin),
            "tenant_user" => Some(Self::TenantUser),
            _ => None,
        }
    }
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

async fn list_tenants(ctx: TenantContext) -> Json<Vec<TenantSummary>> {
    Json(vec![TenantSummary {
        id: ctx.id,
        name: ctx.name,
        status: TenantStatus::Active,
        domain: None,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_context_defaults_when_headers_are_absent() {
        let headers = HeaderMap::new();
        let ctx = TenantContext::from_headers(&headers);

        assert_eq!(ctx.id, DEFAULT_TENANT_ID);
        assert_eq!(ctx.name, "Default");
        assert_eq!(ctx.role, TenantRole::TenantAdmin);
    }

    #[test]
    fn tenant_context_reads_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-tenant-id", HeaderValue::from_static("tenant-a"));
        headers.insert("x-tenant-name", HeaderValue::from_static("Tenant A"));
        headers.insert("x-tenant-role", HeaderValue::from_static("platform_admin"));

        let ctx = TenantContext::from_headers(&headers);

        assert_eq!(ctx.id, "tenant-a");
        assert_eq!(ctx.name, "Tenant A");
        assert_eq!(ctx.role, TenantRole::PlatformAdmin);
    }
}
