use axum::{
    Json, Router,
    extract::{FromRequestParts, State},
    http::{HeaderMap, HeaderValue, StatusCode, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect,
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

#[derive(Debug, Serialize)]
struct RouteSummary {
    id: i64,
    tenant_id: Option<i64>,
    name: String,
    description: Option<String>,
    direction: crate::models::routing::RoutingDirection,
    priority: i32,
    is_active: bool,
    selection_strategy: crate::models::routing::RoutingSelectionStrategy,
    source_trunk_id: Option<i64>,
    default_trunk_id: Option<i64>,
    source_pattern: Option<String>,
    destination_pattern: Option<String>,
    owner: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    last_deployed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
struct CallRecordSummary {
    id: i64,
    tenant_id: Option<i64>,
    call_id: String,
    display_id: Option<String>,
    direction: String,
    status: String,
    started_at: chrono::DateTime<chrono::Utc>,
    ended_at: Option<chrono::DateTime<chrono::Utc>>,
    duration_secs: i32,
    from_number: Option<String>,
    to_number: Option<String>,
    caller_name: Option<String>,
    agent_name: Option<String>,
    queue: Option<String>,
    extension_id: Option<i64>,
    sip_trunk_id: Option<i64>,
    route_id: Option<i64>,
    has_transcript: bool,
    transcript_status: String,
    recording_duration_secs: Option<i32>,
}

#[derive(Debug, Serialize)]
struct UserSummary {
    id: i64,
    tenant_id: Option<i64>,
    email: String,
    username: String,
    last_login_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    is_active: bool,
    is_staff: bool,
    is_superuser: bool,
    mfa_enabled: bool,
    auth_source: String,
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

impl From<crate::models::routing::Model> for RouteSummary {
    fn from(value: crate::models::routing::Model) -> Self {
        Self {
            id: value.id,
            tenant_id: value.tenant_id,
            name: value.name,
            description: value.description,
            direction: value.direction,
            priority: value.priority,
            is_active: value.is_active,
            selection_strategy: value.selection_strategy,
            source_trunk_id: value.source_trunk_id,
            default_trunk_id: value.default_trunk_id,
            source_pattern: value.source_pattern,
            destination_pattern: value.destination_pattern,
            owner: value.owner,
            created_at: value.created_at,
            updated_at: value.updated_at,
            last_deployed_at: value.last_deployed_at,
        }
    }
}

impl From<crate::models::call_record::Model> for CallRecordSummary {
    fn from(value: crate::models::call_record::Model) -> Self {
        Self {
            id: value.id,
            tenant_id: value.tenant_id,
            call_id: value.call_id,
            display_id: value.display_id,
            direction: value.direction,
            status: value.status,
            started_at: value.started_at,
            ended_at: value.ended_at,
            duration_secs: value.duration_secs,
            from_number: value.from_number,
            to_number: value.to_number,
            caller_name: value.caller_name,
            agent_name: value.agent_name,
            queue: value.queue,
            extension_id: value.extension_id,
            sip_trunk_id: value.sip_trunk_id,
            route_id: value.route_id,
            has_transcript: value.has_transcript,
            transcript_status: value.transcript_status,
            recording_duration_secs: value.recording_duration_secs,
        }
    }
}

impl From<crate::models::user::Model> for UserSummary {
    fn from(value: crate::models::user::Model) -> Self {
        Self {
            id: value.id,
            tenant_id: value.tenant_id,
            email: value.email,
            username: value.username,
            last_login_at: value.last_login_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
            is_active: value.is_active,
            is_staff: value.is_staff,
            is_superuser: value.is_superuser,
            mfa_enabled: value.mfa_enabled,
            auth_source: value.auth_source,
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

#[derive(Debug, Deserialize)]
struct CreateExtensionRequest {
    extension: String,
    display_name: Option<String>,
    email: Option<String>,
    status: Option<String>,
    login_disabled: Option<bool>,
    voicemail_disabled: Option<bool>,
    allow_guest_calls: Option<bool>,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateSipTrunkRequest {
    name: String,
    display_name: Option<String>,
    carrier: Option<String>,
    description: Option<String>,
    status: Option<crate::models::sip_trunk::SipTrunkStatus>,
    direction: Option<crate::models::sip_trunk::SipTrunkDirection>,
    sip_server: Option<String>,
    sip_transport: Option<crate::models::sip_trunk::SipTransport>,
    outbound_proxy: Option<String>,
    auth_username: Option<String>,
    auth_password: Option<String>,
    is_active: Option<bool>,
    register_enabled: Option<bool>,
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
        .route(
            "/api/cloudpbx/extensions",
            get(list_extensions).post(create_extension),
        )
        .route(
            "/api/cloudpbx/sip-trunks",
            get(list_sip_trunks).post(create_sip_trunk),
        )
        .route("/api/cloudpbx/routes", get(list_routes))
        .route("/api/cloudpbx/call-records", get(list_call_records))
        .route("/api/cloudpbx/users", get(list_users))
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

async fn create_extension(
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
    Json(payload): Json<CreateExtensionRequest>,
) -> Response {
    match create_extension_for_tenant(state.db(), &ctx, payload).await {
        Ok(summary) => (StatusCode::CREATED, Json(summary)).into_response(),
        Err(response) => response,
    }
}

async fn create_extension_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    payload: CreateExtensionRequest,
) -> Result<ExtensionSummary, Response> {
    use crate::models::extension::{ActiveModel, Column, Entity};

    let extension = payload.extension.trim();
    if extension.is_empty() {
        return Err(bad_request("invalid_extension", "Extension is required."));
    }
    if extension.len() > 32 {
        return Err(bad_request(
            "invalid_extension",
            "Extension must be 32 characters or fewer.",
        ));
    }

    let tenant_id = match resolve_write_tenant_id(db, ctx).await {
        Ok(tenant_id) => tenant_id,
        Err(response) => return Err(response),
    };

    match Entity::find()
        .filter(Column::TenantId.eq(tenant_id))
        .filter(Column::Extension.eq(extension))
        .one(db)
        .await
    {
        Ok(Some(_)) => {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorBody {
                    error: "extension_exists",
                    message: "Extension already exists in this tenant.",
                }),
            )
                .into_response());
        }
        Ok(None) => {}
        Err(_) => return Err(tenant_query_failed()),
    }

    let now = chrono::Utc::now();
    let model = ActiveModel {
        tenant_id: Set(Some(tenant_id)),
        extension: Set(extension.to_string()),
        display_name: Set(clean_optional_string(payload.display_name)),
        email: Set(clean_optional_string(payload.email)),
        status: Set(clean_optional_string(payload.status).or_else(|| Some("active".to_string()))),
        login_disabled: Set(payload.login_disabled.unwrap_or(false)),
        voicemail_disabled: Set(payload.voicemail_disabled.unwrap_or(false)),
        allow_guest_calls: Set(payload.allow_guest_calls.unwrap_or(false)),
        sip_password: Set(None),
        call_forwarding_mode: Set(Some("none".to_string())),
        call_forwarding_destination: Set(None),
        call_forwarding_timeout: Set(None),
        registered_at: Set(None),
        notes: Set(clean_optional_string(payload.notes)),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };

    match model.insert(db).await {
        Ok(model) => Ok(ExtensionSummary::from(model)),
        Err(_) => Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "extension_create_failed",
                message: "Failed to create extension.",
            }),
        )
            .into_response()),
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

async fn create_sip_trunk(
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
    Json(payload): Json<CreateSipTrunkRequest>,
) -> Response {
    match create_sip_trunk_for_tenant(state.db(), &ctx, payload).await {
        Ok(summary) => (StatusCode::CREATED, Json(summary)).into_response(),
        Err(response) => response,
    }
}

async fn create_sip_trunk_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    payload: CreateSipTrunkRequest,
) -> Result<SipTrunkSummary, Response> {
    use crate::models::sip_trunk::{
        ActiveModel, Column, Entity, SipTransport, SipTrunkDirection, SipTrunkStatus,
    };

    let name = payload.name.trim();
    if name.is_empty() {
        return Err(bad_request(
            "invalid_trunk_name",
            "SIP trunk name is required.",
        ));
    }
    if name.len() > 120 {
        return Err(bad_request(
            "invalid_trunk_name",
            "SIP trunk name must be 120 characters or fewer.",
        ));
    }

    let tenant_id = match resolve_write_tenant_id(db, ctx).await {
        Ok(tenant_id) => tenant_id,
        Err(response) => return Err(response),
    };

    match Entity::find()
        .filter(Column::TenantId.eq(tenant_id))
        .filter(Column::Name.eq(name))
        .one(db)
        .await
    {
        Ok(Some(_)) => {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorBody {
                    error: "sip_trunk_exists",
                    message: "SIP trunk already exists in this tenant.",
                }),
            )
                .into_response());
        }
        Ok(None) => {}
        Err(_) => return Err(tenant_query_failed()),
    }

    let now = chrono::Utc::now();
    let model = ActiveModel {
        tenant_id: Set(Some(tenant_id)),
        name: Set(name.to_string()),
        display_name: Set(clean_optional_string(payload.display_name)),
        carrier: Set(clean_optional_string(payload.carrier)),
        description: Set(clean_optional_string(payload.description)),
        status: Set(payload.status.unwrap_or(SipTrunkStatus::Healthy)),
        direction: Set(payload
            .direction
            .unwrap_or(SipTrunkDirection::Bidirectional)),
        sip_server: Set(clean_optional_string(payload.sip_server)),
        sip_transport: Set(payload.sip_transport.unwrap_or(SipTransport::Udp)),
        outbound_proxy: Set(clean_optional_string(payload.outbound_proxy)),
        auth_username: Set(clean_optional_string(payload.auth_username)),
        auth_password: Set(clean_optional_string(payload.auth_password)),
        is_active: Set(payload.is_active.unwrap_or(true)),
        register_enabled: Set(payload.register_enabled.unwrap_or(false)),
        rewrite_hostport: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };

    match model.insert(db).await {
        Ok(model) => Ok(SipTrunkSummary::from(model)),
        Err(_) => Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "sip_trunk_create_failed",
                message: "Failed to create SIP trunk.",
            }),
        )
            .into_response()),
    }
}

async fn list_routes(State(state): State<crate::app::AppState>, ctx: TenantContext) -> Response {
    use crate::models::routing::{Column, Entity};

    let mut query = Entity::find()
        .order_by_asc(Column::Priority)
        .order_by_asc(Column::Name)
        .limit(500);
    match resolve_tenant_scope(state.db(), &ctx).await {
        Ok(TenantDbScope::All) => {}
        Ok(TenantDbScope::Tenant(tenant_id)) => {
            query = query.filter(Column::TenantId.eq(tenant_id));
        }
        Ok(TenantDbScope::Missing) => return Json(Vec::<RouteSummary>::new()).into_response(),
        Err(_) => return tenant_query_failed(),
    }

    match query.all(state.db()).await {
        Ok(items) => Json(
            items
                .into_iter()
                .map(RouteSummary::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => tenant_query_failed(),
    }
}

async fn list_call_records(
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
) -> Response {
    use crate::models::call_record::{Column, Entity};

    let mut query = Entity::find().order_by_desc(Column::StartedAt).limit(500);
    match resolve_tenant_scope(state.db(), &ctx).await {
        Ok(TenantDbScope::All) => {}
        Ok(TenantDbScope::Tenant(tenant_id)) => {
            query = query.filter(Column::TenantId.eq(tenant_id));
        }
        Ok(TenantDbScope::Missing) => {
            return Json(Vec::<CallRecordSummary>::new()).into_response();
        }
        Err(_) => return tenant_query_failed(),
    }

    match query.all(state.db()).await {
        Ok(items) => Json(
            items
                .into_iter()
                .map(CallRecordSummary::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => tenant_query_failed(),
    }
}

async fn list_users(State(state): State<crate::app::AppState>, ctx: TenantContext) -> Response {
    use crate::models::user::{Column, Entity};

    let mut query = Entity::find().order_by_asc(Column::Username).limit(500);
    match resolve_tenant_scope(state.db(), &ctx).await {
        Ok(TenantDbScope::All) => {}
        Ok(TenantDbScope::Tenant(tenant_id)) => {
            query = query.filter(Column::TenantId.eq(tenant_id));
        }
        Ok(TenantDbScope::Missing) => return Json(Vec::<UserSummary>::new()).into_response(),
        Err(_) => return tenant_query_failed(),
    }

    match query.all(state.db()).await {
        Ok(items) => {
            Json(items.into_iter().map(UserSummary::from).collect::<Vec<_>>()).into_response()
        }
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

async fn resolve_write_tenant_id(
    db: &DatabaseConnection,
    ctx: &TenantContext,
) -> Result<i64, Response> {
    use crate::models::tenant::{Column, Entity};

    match Entity::find()
        .filter(Column::Slug.eq(ctx.id.clone()))
        .one(db)
        .await
    {
        Ok(Some(tenant)) => Ok(tenant.id),
        Ok(None) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "tenant_not_found",
                message: "Tenant does not exist.",
            }),
        )
            .into_response()),
        Err(_) => Err(tenant_query_failed()),
    }
}

fn clean_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn bad_request(error: &'static str, message: &'static str) -> Response {
    (StatusCode::BAD_REQUEST, Json(ErrorBody { error, message })).into_response()
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
    use sea_orm::{ColumnTrait, Database, EntityTrait, QueryFilter};
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

    #[tokio::test]
    async fn create_extension_sets_current_tenant_id() {
        use crate::models::{
            extension::{Column as ExtensionColumn, Entity as ExtensionEntity},
            tenant::{Column as TenantColumn, Entity as TenantEntity},
        };

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_extension_for_tenant(
            &db,
            &ctx,
            CreateExtensionRequest {
                extension: "1001".to_string(),
                display_name: Some("Alice".to_string()),
                email: None,
                status: None,
                login_disabled: None,
                voicemail_disabled: None,
                allow_guest_calls: None,
                notes: None,
            },
        )
        .await
        .expect("create extension");

        let tenant = TenantEntity::find()
            .filter(TenantColumn::Slug.eq(DEFAULT_TENANT_ID))
            .one(&db)
            .await
            .expect("query tenant")
            .expect("default tenant");
        let extension = ExtensionEntity::find()
            .filter(ExtensionColumn::Extension.eq("1001"))
            .one(&db)
            .await
            .expect("query extension")
            .expect("extension");

        assert_eq!(summary.tenant_id, Some(tenant.id));
        assert_eq!(extension.tenant_id, Some(tenant.id));
    }

    #[tokio::test]
    async fn create_sip_trunk_sets_current_tenant_id() {
        use crate::models::{
            sip_trunk::{Column as SipTrunkColumn, Entity as SipTrunkEntity},
            tenant::{Column as TenantColumn, Entity as TenantEntity},
        };

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_sip_trunk_for_tenant(
            &db,
            &ctx,
            CreateSipTrunkRequest {
                name: "carrier-a".to_string(),
                display_name: Some("Carrier A".to_string()),
                carrier: None,
                description: None,
                status: None,
                direction: None,
                sip_server: Some("sip.example.com".to_string()),
                sip_transport: None,
                outbound_proxy: None,
                auth_username: Some("user".to_string()),
                auth_password: Some("secret".to_string()),
                is_active: None,
                register_enabled: None,
            },
        )
        .await
        .expect("create sip trunk");

        let tenant = TenantEntity::find()
            .filter(TenantColumn::Slug.eq(DEFAULT_TENANT_ID))
            .one(&db)
            .await
            .expect("query tenant")
            .expect("default tenant");
        let trunk = SipTrunkEntity::find()
            .filter(SipTrunkColumn::Name.eq("carrier-a"))
            .one(&db)
            .await
            .expect("query trunk")
            .expect("trunk");

        assert_eq!(summary.tenant_id, Some(tenant.id));
        assert_eq!(trunk.tenant_id, Some(tenant.id));
        assert_eq!(trunk.auth_password.as_deref(), Some("secret"));
    }
}
