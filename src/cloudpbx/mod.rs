use axum::{
    Json, Router,
    extract::{FromRequestParts, State},
    http::{HeaderMap, HeaderValue, StatusCode, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
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
    status: String,
    domain: Option<String>,
}

#[derive(Debug, Serialize)]
struct ExtensionSummary {
    id: i64,
    tenant_id: Option<i64>,
    extension: String,
    display_name: Option<String>,
    email: Option<String>,
    status: Option<String>,
    login_disabled: bool,
    voicemail_disabled: bool,
    allow_guest_calls: bool,
    registered_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<crate::models::extension::Model> for ExtensionSummary {
    fn from(value: crate::models::extension::Model) -> Self {
        Self {
            id: value.id,
            tenant_id: value.tenant_id,
            extension: value.extension,
            display_name: value.display_name,
            email: value.email,
            status: value.status,
            login_disabled: value.login_disabled,
            voicemail_disabled: value.voicemail_disabled,
            allow_guest_calls: value.allow_guest_calls,
            registered_at: value.registered_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct SipTrunkSummary {
    id: i64,
    tenant_id: Option<i64>,
    name: String,
    display_name: Option<String>,
    carrier: Option<String>,
    status: crate::models::sip_trunk::SipTrunkStatus,
    direction: crate::models::sip_trunk::SipTrunkDirection,
    sip_server: Option<String>,
    sip_transport: crate::models::sip_trunk::SipTransport,
    is_active: bool,
    register_enabled: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<crate::models::sip_trunk::Model> for SipTrunkSummary {
    fn from(value: crate::models::sip_trunk::Model) -> Self {
        Self {
            id: value.id,
            tenant_id: value.tenant_id,
            name: value.name,
            display_name: value.display_name,
            carrier: value.carrier,
            status: value.status,
            direction: value.direction,
            sip_server: value.sip_server,
            sip_transport: value.sip_transport,
            is_active: value.is_active,
            register_enabled: value.register_enabled,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<crate::models::tenant::Model> for TenantSummary {
    fn from(value: crate::models::tenant::Model) -> Self {
        Self {
            id: value.slug,
            name: value.name,
            status: value.status,
            domain: value.domain,
        }
    }
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

pub fn router(state: crate::app::AppState) -> Router {
    Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/session", get(session))
        .route("/api/tenants", get(list_tenants))
        .route("/api/cloudpbx/extensions", get(list_extensions))
        .route("/api/cloudpbx/sip-trunks", get(list_sip_trunks))
        .with_state(state)
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

async fn list_tenants(State(state): State<crate::app::AppState>, ctx: TenantContext) -> Response {
    use crate::models::tenant::{Column, Entity};

    let mut query = Entity::find().order_by_asc(Column::Name);
    if ctx.role != TenantRole::PlatformAdmin {
        query = query.filter(Column::Slug.eq(ctx.id.clone()));
    }

    match query.all(state.db()).await {
        Ok(tenants) => Json(
            tenants
                .into_iter()
                .map(TenantSummary::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: "tenant_query_failed",
                message: "Failed to load tenants.",
            }),
        )
            .into_response(),
    }
}

async fn list_extensions(
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
) -> Response {
    use crate::models::extension::{Column, Entity};

    let mut query = Entity::find().order_by_asc(Column::Extension).limit(500);
    match resolve_tenant_scope(state.db(), &ctx).await {
        Ok(TenantDbScope::All) => {}
        Ok(TenantDbScope::Tenant(tenant_id)) => {
            query = query.filter(Column::TenantId.eq(tenant_id));
        }
        Ok(TenantDbScope::Missing) => return Json(Vec::<ExtensionSummary>::new()).into_response(),
        Err(_) => return tenant_query_failed(),
    }

    match query.all(state.db()).await {
        Ok(items) => Json(
            items
                .into_iter()
                .map(ExtensionSummary::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => tenant_query_failed(),
    }
}

async fn list_sip_trunks(
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
) -> Response {
    use crate::models::sip_trunk::{Column, Entity};

    let mut query = Entity::find().order_by_asc(Column::Name).limit(500);
    match resolve_tenant_scope(state.db(), &ctx).await {
        Ok(TenantDbScope::All) => {}
        Ok(TenantDbScope::Tenant(tenant_id)) => {
            query = query.filter(Column::TenantId.eq(tenant_id));
        }
        Ok(TenantDbScope::Missing) => return Json(Vec::<SipTrunkSummary>::new()).into_response(),
        Err(_) => return tenant_query_failed(),
    }

    match query.all(state.db()).await {
        Ok(items) => Json(
            items
                .into_iter()
                .map(SipTrunkSummary::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => tenant_query_failed(),
    }
}

enum TenantDbScope {
    All,
    Tenant(i64),
    Missing,
}

async fn resolve_tenant_scope(
    db: &DatabaseConnection,
    ctx: &TenantContext,
) -> Result<TenantDbScope, DbErr> {
    use crate::models::tenant::{Column, Entity};

    if ctx.role == TenantRole::PlatformAdmin {
        return Ok(TenantDbScope::All);
    }

    let tenant = Entity::find()
        .filter(Column::Slug.eq(ctx.id.clone()))
        .one(db)
        .await?;

    Ok(match tenant {
        Some(tenant) => TenantDbScope::Tenant(tenant.id),
        None => TenantDbScope::Missing,
    })
}

fn tenant_query_failed() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: "tenant_query_failed",
            message: "Failed to load tenant-scoped resources.",
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::migration::Migrator;
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;

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

    #[tokio::test]
    async fn tenant_scope_resolves_database_id() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let default_ctx = TenantContext::default_tenant_admin();
        let default_scope = resolve_tenant_scope(&db, &default_ctx)
            .await
            .expect("default scope");
        assert!(matches!(default_scope, TenantDbScope::Tenant(_)));

        let platform_ctx = TenantContext {
            role: TenantRole::PlatformAdmin,
            ..TenantContext::default_tenant_admin()
        };
        let platform_scope = resolve_tenant_scope(&db, &platform_ctx)
            .await
            .expect("platform scope");
        assert!(matches!(platform_scope, TenantDbScope::All));

        let missing_ctx = TenantContext {
            id: "missing".to_string(),
            name: "Missing".to_string(),
            role: TenantRole::TenantAdmin,
        };
        let missing_scope = resolve_tenant_scope(&db, &missing_ctx)
            .await
            .expect("missing scope");
        assert!(matches!(missing_scope, TenantDbScope::Missing));
    }
}
