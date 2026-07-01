use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordVerifier, SaltString};
use argon2::{Argon2, PasswordHasher};
use axum::{
    Json, Router,
    body::Body,
    extract::{FromRequestParts, Path as AxumPath, State},
    http::{
        HeaderMap, HeaderValue, Request, StatusCode,
        header::{COOKIE, SET_COOKIE},
        request::Parts,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::engine::{Engine, general_purpose::STANDARD_NO_PAD};
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use sea_orm::sea_query::Condition;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::Duration;

pub const DEFAULT_TENANT_ID: &str = "default";
const SESSION_COOKIE_NAME: &str = "cloudpbx_session";
const SESSION_TTL_HOURS: u64 = 12;
type HmacSha256 = Hmac<Sha256>;

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
            .and_then(TenantRole::from_header)
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct TenantSummary {
    id: String,
    name: String,
    status: String,
    domain: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SessionUser {
    id: i64,
    username: String,
    email: String,
    role: TenantRole,
    tenant: Option<TenantSummary>,
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
struct CreateTenantRequest {
    id: String,
    name: String,
    status: Option<String>,
    domain: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateTenantRequest {
    name: Option<String>,
    status: Option<String>,
    domain: Option<Option<String>>,
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
struct UpdateExtensionRequest {
    extension: Option<String>,
    display_name: Option<Option<String>>,
    email: Option<Option<String>>,
    status: Option<Option<String>>,
    login_disabled: Option<bool>,
    voicemail_disabled: Option<bool>,
    allow_guest_calls: Option<bool>,
    notes: Option<Option<String>>,
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

#[derive(Debug, Deserialize)]
struct UpdateSipTrunkRequest {
    name: Option<String>,
    display_name: Option<Option<String>>,
    carrier: Option<Option<String>>,
    description: Option<Option<String>>,
    status: Option<crate::models::sip_trunk::SipTrunkStatus>,
    direction: Option<crate::models::sip_trunk::SipTrunkDirection>,
    sip_server: Option<Option<String>>,
    sip_transport: Option<crate::models::sip_trunk::SipTransport>,
    outbound_proxy: Option<Option<String>>,
    auth_username: Option<Option<String>>,
    auth_password: Option<Option<String>>,
    is_active: Option<bool>,
    register_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CreateRouteRequest {
    name: String,
    description: Option<String>,
    direction: Option<crate::models::routing::RoutingDirection>,
    priority: Option<i32>,
    is_active: Option<bool>,
    selection_strategy: Option<crate::models::routing::RoutingSelectionStrategy>,
    source_trunk_id: Option<i64>,
    default_trunk_id: Option<i64>,
    source_pattern: Option<String>,
    destination_pattern: Option<String>,
    owner: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateRouteRequest {
    name: Option<String>,
    description: Option<Option<String>>,
    direction: Option<crate::models::routing::RoutingDirection>,
    priority: Option<i32>,
    is_active: Option<bool>,
    selection_strategy: Option<crate::models::routing::RoutingSelectionStrategy>,
    source_trunk_id: Option<Option<i64>>,
    default_trunk_id: Option<Option<i64>>,
    source_pattern: Option<Option<String>>,
    destination_pattern: Option<Option<String>>,
    owner: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
struct CreateUserRequest {
    username: String,
    email: String,
    password: String,
    is_active: Option<bool>,
    is_staff: Option<bool>,
    is_superuser: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateUserRequest {
    username: Option<String>,
    email: Option<String>,
    password: Option<String>,
    is_active: Option<bool>,
    is_staff: Option<bool>,
    is_superuser: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
    message: &'static str,
}

pub fn router(state: crate::app::AppState) -> Router {
    let protected = Router::new()
        .route("/api/tenants", get(list_tenants).post(create_tenant))
        .route("/api/tenants/{id}", axum::routing::patch(update_tenant))
        .route(
            "/api/cloudpbx/extensions",
            get(list_extensions).post(create_extension),
        )
        .route(
            "/api/cloudpbx/extensions/{id}",
            axum::routing::patch(update_extension).delete(delete_extension),
        )
        .route(
            "/api/cloudpbx/sip-trunks",
            get(list_sip_trunks).post(create_sip_trunk),
        )
        .route(
            "/api/cloudpbx/sip-trunks/{id}",
            axum::routing::patch(update_sip_trunk).delete(delete_sip_trunk),
        )
        .route("/api/cloudpbx/routes", get(list_routes).post(create_route))
        .route(
            "/api/cloudpbx/routes/{id}",
            axum::routing::patch(update_route).delete(delete_route),
        )
        .route("/api/cloudpbx/call-records", get(list_call_records))
        .route("/api/cloudpbx/users", get(list_users).post(create_user))
        .route(
            "/api/cloudpbx/users/{id}",
            axum::routing::patch(update_user).delete(delete_user),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            cloudpbx_session_middleware,
        ));

    Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/session", get(session))
        .merge(protected)
        .with_state(state)
}

async fn cloudpbx_session_middleware(
    State(state): State<crate::app::AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let secret = cloudpbx_session_secret(&state);
    let user = match current_session_user(state.db(), &secret, req.headers()).await {
        Ok(user) => user,
        Err(response) => return response,
    };

    let allow_inactive_tenant = !req.uri().path().starts_with("/api/cloudpbx/");
    let ctx = match request_tenant_context(state.db(), user, req.headers(), allow_inactive_tenant)
        .await
    {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };

    req.extensions_mut().insert(ctx);
    next.run(req).await
}

async fn login(
    State(state): State<crate::app::AppState>,
    Json(payload): Json<LoginRequest>,
) -> Response {
    let secret = cloudpbx_session_secret(&state);
    match authenticate_login(state.db(), &secret, payload).await {
        Ok((user, cookie)) => {
            let mut response = Json(user).into_response();
            response.headers_mut().append(SET_COOKIE, cookie);
            response
        }
        Err(response) => response,
    }
}

async fn logout() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .append(SET_COOKIE, clear_session_cookie_header());
    response
}

async fn session(State(state): State<crate::app::AppState>, headers: HeaderMap) -> Response {
    let secret = cloudpbx_session_secret(&state);
    match current_session_user(state.db(), &secret, &headers).await {
        Ok(user) => Json(user).into_response(),
        Err(response) => response,
    }
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

async fn create_tenant(
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
    Json(payload): Json<CreateTenantRequest>,
) -> Response {
    match create_tenant_for_platform(state.db(), &ctx, payload).await {
        Ok(summary) => (StatusCode::CREATED, Json(summary)).into_response(),
        Err(response) => response,
    }
}

async fn create_tenant_for_platform(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    payload: CreateTenantRequest,
) -> Result<TenantSummary, Response> {
    use crate::models::tenant::{ActiveModel, Column, Entity};

    require_platform_admin(ctx)?;

    let slug = normalize_tenant_slug(&payload.id)?;
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(bad_request(
            "invalid_tenant_name",
            "Tenant name is required.",
        ));
    }
    if name.len() > 255 {
        return Err(bad_request(
            "invalid_tenant_name",
            "Tenant name must be 255 characters or fewer.",
        ));
    }
    let status = normalize_tenant_status(payload.status)?;

    match Entity::find()
        .filter(Column::Slug.eq(slug.clone()))
        .one(db)
        .await
    {
        Ok(Some(_)) => {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorBody {
                    error: "tenant_exists",
                    message: "Tenant already exists.",
                }),
            )
                .into_response());
        }
        Ok(None) => {}
        Err(_) => return Err(tenant_query_failed()),
    }

    let now = chrono::Utc::now();
    let model = ActiveModel {
        slug: Set(slug),
        name: Set(name.to_string()),
        status: Set(status),
        domain: Set(clean_optional_string(payload.domain)),
        max_concurrent_calls: Set(None),
        max_trunks: Set(None),
        storage_prefix: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };

    match model.insert(db).await {
        Ok(model) => Ok(TenantSummary::from(model)),
        Err(_) => Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "tenant_create_failed",
                message: "Failed to create tenant.",
            }),
        )
            .into_response()),
    }
}

async fn update_tenant(
    AxumPath(id): AxumPath<String>,
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
    Json(payload): Json<UpdateTenantRequest>,
) -> Response {
    match update_tenant_for_platform(state.db(), &ctx, &id, payload).await {
        Ok(summary) => Json(summary).into_response(),
        Err(response) => response,
    }
}

async fn update_tenant_for_platform(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: &str,
    payload: UpdateTenantRequest,
) -> Result<TenantSummary, Response> {
    use crate::models::tenant::{Column, Entity};

    require_platform_admin(ctx)?;

    let slug = normalize_tenant_slug(id)?;
    let model = Entity::find()
        .filter(Column::Slug.eq(slug))
        .one(db)
        .await
        .map_err(|_| tenant_query_failed())?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody {
                    error: "tenant_not_found",
                    message: "Tenant does not exist.",
                }),
            )
                .into_response()
        })?;

    let mut active = model.into_active_model();
    if let Some(name) = payload.name {
        let name = name.trim();
        if name.is_empty() {
            return Err(bad_request(
                "invalid_tenant_name",
                "Tenant name is required.",
            ));
        }
        if name.len() > 255 {
            return Err(bad_request(
                "invalid_tenant_name",
                "Tenant name must be 255 characters or fewer.",
            ));
        }
        active.name = Set(name.to_string());
    }
    if payload.status.is_some() {
        active.status = Set(normalize_tenant_status(payload.status)?);
    }
    if let Some(domain) = payload.domain {
        active.domain = Set(clean_optional_string(domain));
    }
    active.updated_at = Set(chrono::Utc::now());

    match active.update(db).await {
        Ok(model) => Ok(TenantSummary::from(model)),
        Err(_) => Err(tenant_query_failed()),
    }
}

async fn list_extensions(
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
) -> Response {
    use crate::models::extension::{Column, Entity};

    let mut query = Entity::find().order_by_asc(Column::Extension).limit(500);
    match resolve_tenant_scope(state.db(), &ctx).await {
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

    require_tenant_admin(ctx)?;

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

async fn update_extension(
    AxumPath(id): AxumPath<i64>,
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
    Json(payload): Json<UpdateExtensionRequest>,
) -> Response {
    match update_extension_for_tenant(state.db(), &ctx, id, payload).await {
        Ok(summary) => Json(summary).into_response(),
        Err(response) => response,
    }
}

async fn update_extension_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
    payload: UpdateExtensionRequest,
) -> Result<ExtensionSummary, Response> {
    use crate::models::extension::{Column, Entity};

    require_tenant_admin(ctx)?;

    let model = load_extension_for_write(db, ctx, id).await?;
    let original_tenant_id = model.tenant_id;
    let mut active = model.into_active_model();

    if let Some(extension) = payload.extension {
        let extension = extension.trim();
        if extension.is_empty() {
            return Err(bad_request("invalid_extension", "Extension is required."));
        }
        if extension.len() > 32 {
            return Err(bad_request(
                "invalid_extension",
                "Extension must be 32 characters or fewer.",
            ));
        }

        if let Some(tenant_id) = original_tenant_id {
            match Entity::find()
                .filter(Column::TenantId.eq(tenant_id))
                .filter(Column::Extension.eq(extension))
                .filter(Column::Id.ne(id))
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
        }

        active.extension = Set(extension.to_string());
    }
    if let Some(display_name) = payload.display_name {
        active.display_name = Set(clean_optional_string(display_name));
    }
    if let Some(email) = payload.email {
        active.email = Set(clean_optional_string(email));
    }
    if let Some(status) = payload.status {
        active.status = Set(clean_optional_string(status));
    }
    if let Some(login_disabled) = payload.login_disabled {
        active.login_disabled = Set(login_disabled);
    }
    if let Some(voicemail_disabled) = payload.voicemail_disabled {
        active.voicemail_disabled = Set(voicemail_disabled);
    }
    if let Some(allow_guest_calls) = payload.allow_guest_calls {
        active.allow_guest_calls = Set(allow_guest_calls);
    }
    if let Some(notes) = payload.notes {
        active.notes = Set(clean_optional_string(notes));
    }
    active.updated_at = Set(chrono::Utc::now());

    match active.update(db).await {
        Ok(model) => Ok(ExtensionSummary::from(model)),
        Err(_) => Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "extension_update_failed",
                message: "Failed to update extension.",
            }),
        )
            .into_response()),
    }
}

async fn delete_extension(
    AxumPath(id): AxumPath<i64>,
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
) -> Response {
    match delete_extension_for_tenant(state.db(), &ctx, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(response) => response,
    }
}

async fn delete_extension_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
) -> Result<(), Response> {
    use crate::models::extension::Entity;

    require_tenant_admin(ctx)?;

    let model = load_extension_for_write(db, ctx, id).await?;
    match Entity::delete_by_id(model.id).exec(db).await {
        Ok(_) => Ok(()),
        Err(_) => Err(tenant_query_failed()),
    }
}

async fn load_extension_for_write(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
) -> Result<crate::models::extension::Model, Response> {
    use crate::models::extension::{Column, Entity};

    let mut query = Entity::find().filter(Column::Id.eq(id));
    if ctx.role != TenantRole::PlatformAdmin {
        let tenant_id = resolve_write_tenant_id(db, ctx).await?;
        query = query.filter(Column::TenantId.eq(tenant_id));
    }

    match query.one(db).await {
        Ok(Some(model)) => Ok(model),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "extension_not_found",
                message: "Extension not found.",
            }),
        )
            .into_response()),
        Err(_) => Err(tenant_query_failed()),
    }
}

async fn list_sip_trunks(
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
) -> Response {
    use crate::models::sip_trunk::{Column, Entity};

    let mut query = Entity::find().order_by_asc(Column::Name).limit(500);
    match resolve_tenant_scope(state.db(), &ctx).await {
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

    require_tenant_admin(ctx)?;

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

async fn update_sip_trunk(
    AxumPath(id): AxumPath<i64>,
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
    Json(payload): Json<UpdateSipTrunkRequest>,
) -> Response {
    match update_sip_trunk_for_tenant(state.db(), &ctx, id, payload).await {
        Ok(summary) => Json(summary).into_response(),
        Err(response) => response,
    }
}

async fn update_sip_trunk_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
    payload: UpdateSipTrunkRequest,
) -> Result<SipTrunkSummary, Response> {
    use crate::models::sip_trunk::{Column, Entity};

    require_tenant_admin(ctx)?;

    let model = load_sip_trunk_for_write(db, ctx, id).await?;
    let original_tenant_id = model.tenant_id;
    let mut active = model.into_active_model();

    if let Some(name) = payload.name {
        let name = name.trim();
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

        if let Some(tenant_id) = original_tenant_id {
            match Entity::find()
                .filter(Column::TenantId.eq(tenant_id))
                .filter(Column::Name.eq(name))
                .filter(Column::Id.ne(id))
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
        }

        active.name = Set(name.to_string());
    }
    if let Some(display_name) = payload.display_name {
        active.display_name = Set(clean_optional_string(display_name));
    }
    if let Some(carrier) = payload.carrier {
        active.carrier = Set(clean_optional_string(carrier));
    }
    if let Some(description) = payload.description {
        active.description = Set(clean_optional_string(description));
    }
    if let Some(status) = payload.status {
        active.status = Set(status);
    }
    if let Some(direction) = payload.direction {
        active.direction = Set(direction);
    }
    if let Some(sip_server) = payload.sip_server {
        active.sip_server = Set(clean_optional_string(sip_server));
    }
    if let Some(sip_transport) = payload.sip_transport {
        active.sip_transport = Set(sip_transport);
    }
    if let Some(outbound_proxy) = payload.outbound_proxy {
        active.outbound_proxy = Set(clean_optional_string(outbound_proxy));
    }
    if let Some(auth_username) = payload.auth_username {
        active.auth_username = Set(clean_optional_string(auth_username));
    }
    if let Some(auth_password) = payload.auth_password {
        active.auth_password = Set(clean_optional_string(auth_password));
    }
    if let Some(is_active) = payload.is_active {
        active.is_active = Set(is_active);
    }
    if let Some(register_enabled) = payload.register_enabled {
        active.register_enabled = Set(register_enabled);
    }
    active.updated_at = Set(chrono::Utc::now());

    match active.update(db).await {
        Ok(model) => Ok(SipTrunkSummary::from(model)),
        Err(_) => Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "sip_trunk_update_failed",
                message: "Failed to update SIP trunk.",
            }),
        )
            .into_response()),
    }
}

async fn delete_sip_trunk(
    AxumPath(id): AxumPath<i64>,
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
) -> Response {
    match delete_sip_trunk_for_tenant(state.db(), &ctx, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(response) => response,
    }
}

async fn delete_sip_trunk_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
) -> Result<(), Response> {
    use crate::models::sip_trunk::Entity;

    require_tenant_admin(ctx)?;

    let model = load_sip_trunk_for_write(db, ctx, id).await?;
    match Entity::delete_by_id(model.id).exec(db).await {
        Ok(_) => Ok(()),
        Err(_) => Err(tenant_query_failed()),
    }
}

async fn load_sip_trunk_for_write(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
) -> Result<crate::models::sip_trunk::Model, Response> {
    use crate::models::sip_trunk::{Column, Entity};

    let mut query = Entity::find().filter(Column::Id.eq(id));
    if ctx.role != TenantRole::PlatformAdmin {
        let tenant_id = resolve_write_tenant_id(db, ctx).await?;
        query = query.filter(Column::TenantId.eq(tenant_id));
    }

    match query.one(db).await {
        Ok(Some(model)) => Ok(model),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "sip_trunk_not_found",
                message: "SIP trunk not found.",
            }),
        )
            .into_response()),
        Err(_) => Err(tenant_query_failed()),
    }
}

async fn list_routes(State(state): State<crate::app::AppState>, ctx: TenantContext) -> Response {
    use crate::models::routing::{Column, Entity};

    let mut query = Entity::find()
        .order_by_asc(Column::Priority)
        .order_by_asc(Column::Name)
        .limit(500);
    match resolve_tenant_scope(state.db(), &ctx).await {
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

async fn create_route(
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
    Json(payload): Json<CreateRouteRequest>,
) -> Response {
    match create_route_for_tenant(state.db(), &ctx, payload).await {
        Ok(summary) => (StatusCode::CREATED, Json(summary)).into_response(),
        Err(response) => response,
    }
}

async fn create_route_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    payload: CreateRouteRequest,
) -> Result<RouteSummary, Response> {
    use crate::models::routing::{
        ActiveModel, Column, Entity, RoutingDirection, RoutingSelectionStrategy,
    };

    require_tenant_admin(ctx)?;

    let name = payload.name.trim();
    if name.is_empty() {
        return Err(bad_request("invalid_route_name", "Route name is required."));
    }
    if name.len() > 160 {
        return Err(bad_request(
            "invalid_route_name",
            "Route name must be 160 characters or fewer.",
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
                    error: "route_exists",
                    message: "Route already exists in this tenant.",
                }),
            )
                .into_response());
        }
        Ok(None) => {}
        Err(_) => return Err(tenant_query_failed()),
    }

    validate_route_trunk_ref(db, Some(tenant_id), payload.source_trunk_id).await?;
    validate_route_trunk_ref(db, Some(tenant_id), payload.default_trunk_id).await?;

    let now = chrono::Utc::now();
    let model = ActiveModel {
        tenant_id: Set(Some(tenant_id)),
        name: Set(name.to_string()),
        description: Set(clean_optional_string(payload.description)),
        direction: Set(payload.direction.unwrap_or(RoutingDirection::Outbound)),
        priority: Set(payload.priority.unwrap_or(100)),
        is_active: Set(payload.is_active.unwrap_or(true)),
        selection_strategy: Set(payload
            .selection_strategy
            .unwrap_or(RoutingSelectionStrategy::RoundRobin)),
        hash_key: Set(None),
        source_trunk_id: Set(payload.source_trunk_id),
        default_trunk_id: Set(payload.default_trunk_id),
        source_pattern: Set(clean_optional_string(payload.source_pattern)),
        destination_pattern: Set(clean_optional_string(payload.destination_pattern)),
        header_filters: Set(None),
        rewrite_rules: Set(None),
        target_trunks: Set(None),
        owner: Set(clean_optional_string(payload.owner)),
        notes: Set(None),
        metadata: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        last_deployed_at: Set(None),
        ..Default::default()
    };

    match model.insert(db).await {
        Ok(model) => Ok(RouteSummary::from(model)),
        Err(_) => Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "route_create_failed",
                message: "Failed to create route.",
            }),
        )
            .into_response()),
    }
}

async fn update_route(
    AxumPath(id): AxumPath<i64>,
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
    Json(payload): Json<UpdateRouteRequest>,
) -> Response {
    match update_route_for_tenant(state.db(), &ctx, id, payload).await {
        Ok(summary) => Json(summary).into_response(),
        Err(response) => response,
    }
}

async fn update_route_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
    payload: UpdateRouteRequest,
) -> Result<RouteSummary, Response> {
    use crate::models::routing::{Column, Entity};

    require_tenant_admin(ctx)?;

    let model = load_route_for_write(db, ctx, id).await?;
    let original_tenant_id = model.tenant_id;
    let mut active = model.into_active_model();

    if let Some(name) = payload.name {
        let name = name.trim();
        if name.is_empty() {
            return Err(bad_request("invalid_route_name", "Route name is required."));
        }
        if name.len() > 160 {
            return Err(bad_request(
                "invalid_route_name",
                "Route name must be 160 characters or fewer.",
            ));
        }

        if let Some(tenant_id) = original_tenant_id {
            match Entity::find()
                .filter(Column::TenantId.eq(tenant_id))
                .filter(Column::Name.eq(name))
                .filter(Column::Id.ne(id))
                .one(db)
                .await
            {
                Ok(Some(_)) => {
                    return Err((
                        StatusCode::CONFLICT,
                        Json(ErrorBody {
                            error: "route_exists",
                            message: "Route already exists in this tenant.",
                        }),
                    )
                        .into_response());
                }
                Ok(None) => {}
                Err(_) => return Err(tenant_query_failed()),
            }
        }

        active.name = Set(name.to_string());
    }
    if let Some(description) = payload.description {
        active.description = Set(clean_optional_string(description));
    }
    if let Some(direction) = payload.direction {
        active.direction = Set(direction);
    }
    if let Some(priority) = payload.priority {
        active.priority = Set(priority);
    }
    if let Some(is_active) = payload.is_active {
        active.is_active = Set(is_active);
    }
    if let Some(selection_strategy) = payload.selection_strategy {
        active.selection_strategy = Set(selection_strategy);
    }
    if let Some(source_trunk_id) = payload.source_trunk_id {
        validate_route_trunk_ref(db, original_tenant_id, source_trunk_id).await?;
        active.source_trunk_id = Set(source_trunk_id);
    }
    if let Some(default_trunk_id) = payload.default_trunk_id {
        validate_route_trunk_ref(db, original_tenant_id, default_trunk_id).await?;
        active.default_trunk_id = Set(default_trunk_id);
    }
    if let Some(source_pattern) = payload.source_pattern {
        active.source_pattern = Set(clean_optional_string(source_pattern));
    }
    if let Some(destination_pattern) = payload.destination_pattern {
        active.destination_pattern = Set(clean_optional_string(destination_pattern));
    }
    if let Some(owner) = payload.owner {
        active.owner = Set(clean_optional_string(owner));
    }
    active.updated_at = Set(chrono::Utc::now());

    match active.update(db).await {
        Ok(model) => Ok(RouteSummary::from(model)),
        Err(_) => Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "route_update_failed",
                message: "Failed to update route.",
            }),
        )
            .into_response()),
    }
}

async fn delete_route(
    AxumPath(id): AxumPath<i64>,
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
) -> Response {
    match delete_route_for_tenant(state.db(), &ctx, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(response) => response,
    }
}

async fn delete_route_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
) -> Result<(), Response> {
    use crate::models::routing::Entity;

    require_tenant_admin(ctx)?;

    let model = load_route_for_write(db, ctx, id).await?;
    match Entity::delete_by_id(model.id).exec(db).await {
        Ok(_) => Ok(()),
        Err(_) => Err(tenant_query_failed()),
    }
}

async fn load_route_for_write(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
) -> Result<crate::models::routing::Model, Response> {
    use crate::models::routing::{Column, Entity};

    let mut query = Entity::find().filter(Column::Id.eq(id));
    if ctx.role != TenantRole::PlatformAdmin {
        let tenant_id = resolve_write_tenant_id(db, ctx).await?;
        query = query.filter(Column::TenantId.eq(tenant_id));
    }

    match query.one(db).await {
        Ok(Some(model)) => Ok(model),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "route_not_found",
                message: "Route not found.",
            }),
        )
            .into_response()),
        Err(_) => Err(tenant_query_failed()),
    }
}

async fn validate_route_trunk_ref(
    db: &DatabaseConnection,
    route_tenant_id: Option<i64>,
    trunk_id: Option<i64>,
) -> Result<(), Response> {
    use crate::models::sip_trunk::{Column, Entity};

    let Some(trunk_id) = trunk_id else {
        return Ok(());
    };

    let mut query = Entity::find().filter(Column::Id.eq(trunk_id));
    query = match route_tenant_id {
        Some(tenant_id) => query.filter(Column::TenantId.eq(tenant_id)),
        None => query.filter(Column::TenantId.is_null()),
    };

    match query.one(db).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(bad_request(
            "invalid_route_trunk",
            "Route trunk must exist in the same tenant.",
        )),
        Err(_) => Err(tenant_query_failed()),
    }
}

async fn list_call_records(
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
) -> Response {
    use crate::models::call_record::{Column, Entity};

    let mut query = Entity::find().order_by_desc(Column::StartedAt).limit(500);
    match resolve_tenant_scope(state.db(), &ctx).await {
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

async fn create_user(
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
    Json(payload): Json<CreateUserRequest>,
) -> Response {
    match create_user_for_tenant(state.db(), &ctx, payload).await {
        Ok(summary) => (StatusCode::CREATED, Json(summary)).into_response(),
        Err(response) => response,
    }
}

async fn create_user_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    payload: CreateUserRequest,
) -> Result<UserSummary, Response> {
    use crate::models::user::{ActiveModel, Column, Entity};

    require_tenant_admin(ctx)?;

    let username = payload.username.trim();
    let email = payload.email.trim().to_lowercase();
    if username.is_empty() {
        return Err(bad_request("invalid_username", "Username is required."));
    }
    if username.len() > 100 {
        return Err(bad_request(
            "invalid_username",
            "Username must be 100 characters or fewer.",
        ));
    }
    if email.is_empty() || !email.contains('@') {
        return Err(bad_request("invalid_email", "Valid email is required."));
    }
    if email.len() > 255 {
        return Err(bad_request(
            "invalid_email",
            "Email must be 255 characters or fewer.",
        ));
    }
    if payload.password.is_empty() {
        return Err(bad_request("invalid_password", "Password is required."));
    }
    if payload.is_superuser.unwrap_or(false) && ctx.role != TenantRole::PlatformAdmin {
        return Err(forbidden(
            "CloudPBX platform administrator access is required to grant superuser privileges.",
        ));
    }

    let tenant_id = match resolve_write_tenant_id(db, ctx).await {
        Ok(tenant_id) => tenant_id,
        Err(response) => return Err(response),
    };

    let mut duplicate = Entity::find()
        .filter(Column::TenantId.eq(tenant_id))
        .filter(Column::Username.eq(username))
        .one(db)
        .await
        .map_err(|_| tenant_query_failed())?;
    if duplicate.is_none() {
        duplicate = Entity::find()
            .filter(Column::TenantId.eq(tenant_id))
            .filter(Column::Email.eq(email.clone()))
            .one(db)
            .await
            .map_err(|_| tenant_query_failed())?;
    }
    if duplicate.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "user_exists",
                message: "User already exists in this tenant.",
            }),
        )
            .into_response());
    }

    let password_hash = hash_password(&payload.password)?;

    let now = chrono::Utc::now();
    let model = ActiveModel {
        tenant_id: Set(Some(tenant_id)),
        email: Set(email),
        username: Set(username.to_string()),
        password_hash: Set(password_hash),
        reset_token: Set(None),
        reset_token_expires: Set(None),
        last_login_at: Set(None),
        last_login_ip: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        is_active: Set(payload.is_active.unwrap_or(true)),
        is_staff: Set(payload.is_staff.unwrap_or(false)),
        is_superuser: Set(payload.is_superuser.unwrap_or(false)),
        mfa_enabled: Set(false),
        mfa_secret: Set(None),
        auth_source: Set("local".to_string()),
        ..Default::default()
    };

    match model.insert(db).await {
        Ok(model) => Ok(UserSummary::from(model)),
        Err(_) => Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "user_create_failed",
                message: "Failed to create user.",
            }),
        )
            .into_response()),
    }
}

async fn update_user(
    AxumPath(id): AxumPath<i64>,
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
    Json(payload): Json<UpdateUserRequest>,
) -> Response {
    match update_user_for_tenant(state.db(), &ctx, id, payload).await {
        Ok(summary) => Json(summary).into_response(),
        Err(response) => response,
    }
}

async fn update_user_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
    payload: UpdateUserRequest,
) -> Result<UserSummary, Response> {
    use crate::models::user::{Column, Entity};

    require_tenant_admin(ctx)?;

    let model = load_user_for_write(db, ctx, id).await?;
    let original_tenant_id = model.tenant_id;
    let mut active = model.into_active_model();

    if let Some(username) = payload.username {
        let username = username.trim();
        if username.is_empty() {
            return Err(bad_request("invalid_username", "Username is required."));
        }
        if username.len() > 100 {
            return Err(bad_request(
                "invalid_username",
                "Username must be 100 characters or fewer.",
            ));
        }
        if let Some(tenant_id) = original_tenant_id {
            match Entity::find()
                .filter(Column::TenantId.eq(tenant_id))
                .filter(Column::Username.eq(username))
                .filter(Column::Id.ne(id))
                .one(db)
                .await
            {
                Ok(Some(_)) => {
                    return Err((
                        StatusCode::CONFLICT,
                        Json(ErrorBody {
                            error: "user_exists",
                            message: "User already exists in this tenant.",
                        }),
                    )
                        .into_response());
                }
                Ok(None) => {}
                Err(_) => return Err(tenant_query_failed()),
            }
        }
        active.username = Set(username.to_string());
    }

    if let Some(email) = payload.email {
        let email = email.trim().to_lowercase();
        if email.is_empty() || !email.contains('@') {
            return Err(bad_request("invalid_email", "Valid email is required."));
        }
        if email.len() > 255 {
            return Err(bad_request(
                "invalid_email",
                "Email must be 255 characters or fewer.",
            ));
        }
        if let Some(tenant_id) = original_tenant_id {
            match Entity::find()
                .filter(Column::TenantId.eq(tenant_id))
                .filter(Column::Email.eq(email.clone()))
                .filter(Column::Id.ne(id))
                .one(db)
                .await
            {
                Ok(Some(_)) => {
                    return Err((
                        StatusCode::CONFLICT,
                        Json(ErrorBody {
                            error: "user_exists",
                            message: "User already exists in this tenant.",
                        }),
                    )
                        .into_response());
                }
                Ok(None) => {}
                Err(_) => return Err(tenant_query_failed()),
            }
        }
        active.email = Set(email);
    }

    if let Some(password) = payload.password {
        if password.is_empty() {
            return Err(bad_request("invalid_password", "Password is required."));
        }
        active.password_hash = Set(hash_password(&password)?);
        active.reset_token = Set(None);
        active.reset_token_expires = Set(None);
    }
    if let Some(is_active) = payload.is_active {
        active.is_active = Set(is_active);
    }
    if let Some(is_staff) = payload.is_staff {
        active.is_staff = Set(is_staff);
    }
    if let Some(is_superuser) = payload.is_superuser {
        if ctx.role != TenantRole::PlatformAdmin {
            return Err(forbidden(
                "CloudPBX platform administrator access is required to modify superuser privileges.",
            ));
        }
        active.is_superuser = Set(is_superuser);
    }
    active.updated_at = Set(chrono::Utc::now());

    match active.update(db).await {
        Ok(model) => Ok(UserSummary::from(model)),
        Err(_) => Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: "user_update_failed",
                message: "Failed to update user.",
            }),
        )
            .into_response()),
    }
}

async fn delete_user(
    AxumPath(id): AxumPath<i64>,
    State(state): State<crate::app::AppState>,
    ctx: TenantContext,
) -> Response {
    match delete_user_for_tenant(state.db(), &ctx, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(response) => response,
    }
}

async fn delete_user_for_tenant(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
) -> Result<(), Response> {
    use crate::models::user::Entity;

    require_tenant_admin(ctx)?;

    let model = load_user_for_write(db, ctx, id).await?;
    match Entity::delete_by_id(model.id).exec(db).await {
        Ok(_) => Ok(()),
        Err(_) => Err(tenant_query_failed()),
    }
}

async fn load_user_for_write(
    db: &DatabaseConnection,
    ctx: &TenantContext,
    id: i64,
) -> Result<crate::models::user::Model, Response> {
    use crate::models::user::{Column, Entity};

    let mut query = Entity::find().filter(Column::Id.eq(id));
    if ctx.role != TenantRole::PlatformAdmin {
        let tenant_id = resolve_write_tenant_id(db, ctx).await?;
        query = query.filter(Column::TenantId.eq(tenant_id));
    }

    match query.one(db).await {
        Ok(Some(model)) => Ok(model),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "user_not_found",
                message: "User not found.",
            }),
        )
            .into_response()),
        Err(_) => Err(tenant_query_failed()),
    }
}

enum TenantDbScope {
    Tenant(i64),
    Missing,
}

async fn resolve_tenant_scope(
    db: &DatabaseConnection,
    ctx: &TenantContext,
) -> Result<TenantDbScope, DbErr> {
    use crate::models::tenant::{Column, Entity};

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

#[allow(clippy::result_large_err)]
fn hash_password(password: &str) -> Result<String, Response> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "password_hash_failed",
                    message: "Failed to hash password.",
                }),
            )
                .into_response()
        })
        .map(|hash| hash.to_string())
}

async fn authenticate_login(
    db: &DatabaseConnection,
    secret: &str,
    payload: LoginRequest,
) -> Result<(SessionUser, HeaderValue), Response> {
    use crate::models::{
        tenant::{Column as TenantColumn, Entity as TenantEntity},
        user::{Column as UserColumn, Entity as UserEntity},
    };

    let username = payload.username.trim();
    let password = payload.password;
    if username.is_empty() || password.is_empty() {
        return Err(unauthorized());
    }

    let tenant_slug = payload
        .tenant
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_TENANT_ID);

    let tenant = TenantEntity::find()
        .filter(TenantColumn::Slug.eq(tenant_slug))
        .one(db)
        .await
        .map_err(|_| tenant_query_failed())?
        .ok_or_else(unauthorized)?;

    if tenant.status != "active" {
        return Err(unauthorized());
    }

    let user = UserEntity::find()
        .filter(
            Condition::any()
                .add(UserColumn::Username.eq(username))
                .add(UserColumn::Email.eq(username.to_lowercase())),
        )
        .one(db)
        .await
        .map_err(|_| tenant_query_failed())?
        .filter(|user| user.is_superuser || user.tenant_id == Some(tenant.id))
        .filter(|user| user.is_active)
        .ok_or_else(unauthorized)?;

    let parsed_hash = PasswordHash::new(&user.password_hash).map_err(|_| unauthorized())?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .map_err(|_| unauthorized())?;

    let mut active = user.clone().into_active_model();
    active.last_login_at = Set(Some(Utc::now()));
    active.updated_at = Set(Utc::now());
    let user = active.update(db).await.map_err(|_| tenant_query_failed())?;
    let cookie = session_cookie_header(secret, &tenant.slug, user.id)?;

    Ok((session_user_from_model(user, Some(tenant)), cookie))
}

async fn current_session_user(
    db: &DatabaseConnection,
    secret: &str,
    headers: &HeaderMap,
) -> Result<SessionUser, Response> {
    use crate::models::{
        tenant::{Column as TenantColumn, Entity as TenantEntity},
        user::{Column as UserColumn, Entity as UserEntity},
    };

    let token = get_cookie(headers, SESSION_COOKIE_NAME).ok_or_else(unauthorized)?;
    let session = verify_session_token(secret, &token).ok_or_else(unauthorized)?;
    let tenant = TenantEntity::find()
        .filter(TenantColumn::Slug.eq(session.tenant_slug.clone()))
        .one(db)
        .await
        .map_err(|_| tenant_query_failed())?
        .ok_or_else(unauthorized)?;

    if tenant.status != "active" {
        return Err(unauthorized());
    }

    let user = UserEntity::find()
        .filter(UserColumn::Id.eq(session.user_id))
        .one(db)
        .await
        .map_err(|_| tenant_query_failed())?
        .filter(|user| user.is_superuser || user.tenant_id == Some(tenant.id))
        .filter(|user| user.is_active)
        .ok_or_else(unauthorized)?;

    Ok(session_user_from_model(user, Some(tenant)))
}

async fn request_tenant_context(
    db: &DatabaseConnection,
    user: SessionUser,
    headers: &HeaderMap,
    allow_inactive_tenant: bool,
) -> Result<TenantContext, Response> {
    use crate::models::tenant::{Column, Entity};

    let session_tenant = user.tenant.ok_or_else(unauthorized)?;
    if user.role != TenantRole::PlatformAdmin {
        return Ok(TenantContext {
            id: session_tenant.id,
            name: session_tenant.name,
            role: user.role,
        });
    }

    let requested_tenant = header_string(headers, "x-tenant-id").unwrap_or(session_tenant.id);
    let requested_tenant = normalize_tenant_slug(&requested_tenant)?;
    let tenant = Entity::find()
        .filter(Column::Slug.eq(requested_tenant))
        .one(db)
        .await
        .map_err(|_| tenant_query_failed())?
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "tenant_not_found",
                    message: "Tenant does not exist.",
                }),
            )
                .into_response()
        })?;

    if !allow_inactive_tenant && tenant.status != "active" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorBody {
                error: "tenant_inactive",
                message: "Tenant is not active.",
            }),
        )
            .into_response());
    }

    Ok(TenantContext {
        id: tenant.slug,
        name: tenant.name,
        role: user.role,
    })
}

#[derive(Debug, PartialEq, Eq)]
struct SessionToken {
    tenant_slug: String,
    user_id: i64,
}

fn cloudpbx_session_secret(state: &crate::app::AppState) -> String {
    state
        .config()
        .console
        .as_ref()
        .map(|config| config.session_secret.clone())
        .unwrap_or_else(|| "cloudpbx-local-session-secret".to_string())
}

fn session_user_from_model(
    user: crate::models::user::Model,
    tenant: Option<crate::models::tenant::Model>,
) -> SessionUser {
    let role = if user.is_superuser {
        TenantRole::PlatformAdmin
    } else if user.is_staff {
        TenantRole::TenantAdmin
    } else {
        TenantRole::TenantUser
    };

    SessionUser {
        id: user.id,
        username: user.username,
        email: user.email,
        role,
        tenant: tenant.map(TenantSummary::from),
    }
}

#[allow(clippy::result_large_err)]
fn session_cookie_header(
    secret: &str,
    tenant_slug: &str,
    user_id: i64,
) -> Result<HeaderValue, Response> {
    let token = generate_session_token(secret, tenant_slug, user_id)?;
    let cookie = format!(
        "{}={}; Path=/; HttpOnly; Max-Age={}; SameSite=Lax",
        SESSION_COOKIE_NAME,
        token,
        SESSION_TTL_HOURS * 3600
    );
    HeaderValue::from_str(&cookie).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: "session_cookie_failed",
                message: "Failed to create session cookie.",
            }),
        )
            .into_response()
    })
}

fn clear_session_cookie_header() -> HeaderValue {
    HeaderValue::from_static("cloudpbx_session=; Path=/; HttpOnly; Max-Age=0; SameSite=Lax")
}

#[allow(clippy::result_large_err)]
fn generate_session_token(
    secret: &str,
    tenant_slug: &str,
    user_id: i64,
) -> Result<String, Response> {
    let expires_at = Utc::now() + Duration::from_secs(SESSION_TTL_HOURS * 3600);
    let payload = format!("{}:{}:{}", tenant_slug, user_id, expires_at.timestamp());
    let signature = sign_session_payload(secret, &payload)?;
    Ok(format!("{}:{}", payload, signature))
}

fn verify_session_token(secret: &str, token: &str) -> Option<SessionToken> {
    let mut segments = token.split(':');
    let tenant_slug = segments.next()?.to_string();
    let user_id: i64 = segments.next()?.parse().ok()?;
    let expires_at: i64 = segments.next()?.parse().ok()?;
    let signature = segments.next()?;
    if segments.next().is_some() || tenant_slug.is_empty() || expires_at <= Utc::now().timestamp() {
        return None;
    }

    let payload = format!("{}:{}:{}", tenant_slug, user_id, expires_at);
    let expected = sign_session_payload(secret, &payload).ok()?;
    if expected != signature {
        return None;
    }

    Some(SessionToken {
        tenant_slug,
        user_id,
    })
}

#[allow(clippy::result_large_err)]
fn sign_session_payload(secret: &str, payload: &str) -> Result<String, Response> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: "session_sign_failed",
                message: "Failed to sign session token.",
            }),
        )
            .into_response()
    })?;
    mac.update(payload.as_bytes());
    Ok(STANDARD_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn get_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    for header in headers.get_all(COOKIE) {
        let header = header.to_str().ok()?;
        for part in header.split(';') {
            let mut pair = part.trim().splitn(2, '=');
            if pair.next()? == name {
                return pair.next().map(ToOwned::to_owned);
            }
        }
    }
    None
}

fn clean_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[allow(clippy::result_large_err)]
fn normalize_tenant_slug(value: &str) -> Result<String, Response> {
    let slug = value.trim().to_lowercase();
    if slug.is_empty() {
        return Err(bad_request("invalid_tenant_id", "Tenant id is required."));
    }
    if slug.len() > 120 {
        return Err(bad_request(
            "invalid_tenant_id",
            "Tenant id must be 120 characters or fewer.",
        ));
    }
    if !slug
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
    {
        return Err(bad_request(
            "invalid_tenant_id",
            "Tenant id may only contain lowercase letters, digits, hyphens, and underscores.",
        ));
    }
    Ok(slug)
}

#[allow(clippy::result_large_err)]
fn normalize_tenant_status(value: Option<String>) -> Result<String, Response> {
    let status = value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("active");

    match status {
        "active" | "suspended" | "disabled" => Ok(status.to_string()),
        _ => Err(bad_request(
            "invalid_tenant_status",
            "Tenant status must be active, suspended, or disabled.",
        )),
    }
}

fn bad_request(error: &'static str, message: &'static str) -> Response {
    (StatusCode::BAD_REQUEST, Json(ErrorBody { error, message })).into_response()
}

fn forbidden(message: &'static str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorBody {
            error: "forbidden",
            message,
        }),
    )
        .into_response()
}

#[allow(clippy::result_large_err)]
fn require_platform_admin(ctx: &TenantContext) -> Result<(), Response> {
    if ctx.role == TenantRole::PlatformAdmin {
        return Ok(());
    }

    Err(forbidden(
        "CloudPBX tenant management requires a platform administrator.",
    ))
}

#[allow(clippy::result_large_err)]
fn require_tenant_admin(ctx: &TenantContext) -> Result<(), Response> {
    match ctx.role {
        TenantRole::PlatformAdmin | TenantRole::TenantAdmin => Ok(()),
        TenantRole::TenantUser => Err(forbidden(
            "CloudPBX write access requires a tenant administrator.",
        )),
    }
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

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorBody {
            error: "unauthorized",
            message: "Invalid CloudPBX credentials or session.",
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
    async fn tenant_context_prefers_request_extension() {
        let mut request = Request::builder()
            .header("x-tenant-id", "spoofed")
            .body(())
            .expect("request");
        request.extensions_mut().insert(TenantContext {
            id: "trusted".to_string(),
            name: "Trusted".to_string(),
            role: TenantRole::TenantAdmin,
        });
        let (mut parts, _) = request.into_parts();

        let ctx = TenantContext::from_request_parts(&mut parts, &())
            .await
            .expect("tenant context");

        assert_eq!(ctx.id, "trusted");
        assert_eq!(ctx.name, "Trusted");
        assert_eq!(ctx.role, TenantRole::TenantAdmin);
    }

    #[tokio::test]
    async fn logout_clears_session_cookie() {
        let response = logout().await;
        let cookie = response
            .headers()
            .get(SET_COOKIE)
            .expect("set-cookie")
            .to_str()
            .expect("cookie str");

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(cookie.starts_with("cloudpbx_session=;"));
        assert!(cookie.contains("Max-Age=0"));
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
        assert!(matches!(platform_scope, TenantDbScope::Tenant(_)));

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
    async fn platform_admin_can_create_and_update_tenant() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext {
            id: DEFAULT_TENANT_ID.to_string(),
            name: "Default".to_string(),
            role: TenantRole::PlatformAdmin,
        };

        let created = create_tenant_for_platform(
            &db,
            &ctx,
            CreateTenantRequest {
                id: "Tenant-A".to_string(),
                name: "Tenant A".to_string(),
                status: None,
                domain: Some("tenant-a.example.com".to_string()),
            },
        )
        .await
        .expect("create tenant");

        assert_eq!(created.id, "tenant-a");
        assert_eq!(created.status, "active");

        let updated = update_tenant_for_platform(
            &db,
            &ctx,
            "tenant-a",
            UpdateTenantRequest {
                name: Some("Tenant A Updated".to_string()),
                status: Some("suspended".to_string()),
                domain: Some(None),
            },
        )
        .await
        .expect("update tenant");

        assert_eq!(updated.name, "Tenant A Updated");
        assert_eq!(updated.status, "suspended");
        assert!(updated.domain.is_none());
    }

    #[tokio::test]
    async fn tenant_admin_cannot_create_tenant() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        let ctx = TenantContext::default_tenant_admin();

        let response = create_tenant_for_platform(
            &db,
            &ctx,
            CreateTenantRequest {
                id: "tenant-denied".to_string(),
                name: "Denied".to_string(),
                status: None,
                domain: None,
            },
        )
        .await
        .expect_err("tenant admin cannot create tenant");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn platform_admin_request_context_uses_selected_tenant_header() {
        use crate::models::tenant::{ActiveModel as TenantActiveModel, Entity as TenantEntity};

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        TenantActiveModel {
            slug: Set("tenant-selected".to_string()),
            name: Set("Tenant Selected".to_string()),
            status: Set("active".to_string()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert tenant");

        let mut headers = HeaderMap::new();
        headers.insert("x-tenant-id", HeaderValue::from_static("tenant-selected"));
        let ctx = request_tenant_context(
            &db,
            SessionUser {
                id: 1,
                username: "platform".to_string(),
                email: "platform@example.com".to_string(),
                role: TenantRole::PlatformAdmin,
                tenant: Some(TenantSummary {
                    id: DEFAULT_TENANT_ID.to_string(),
                    name: "Default".to_string(),
                    status: "active".to_string(),
                    domain: None,
                }),
            },
            &headers,
            false,
        )
        .await
        .expect("tenant context");

        assert_eq!(ctx.id, "tenant-selected");
        assert_eq!(ctx.name, "Tenant Selected");
        assert_eq!(
            TenantEntity::find().all(&db).await.expect("tenants").len(),
            2
        );
    }

    #[tokio::test]
    async fn platform_admin_resource_context_rejects_inactive_selected_tenant() {
        use crate::models::tenant::ActiveModel as TenantActiveModel;

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        TenantActiveModel {
            slug: Set("tenant-suspended".to_string()),
            name: Set("Tenant Suspended".to_string()),
            status: Set("suspended".to_string()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert tenant");

        let user = SessionUser {
            id: 1,
            username: "platform".to_string(),
            email: "platform@example.com".to_string(),
            role: TenantRole::PlatformAdmin,
            tenant: Some(TenantSummary {
                id: DEFAULT_TENANT_ID.to_string(),
                name: "Default".to_string(),
                status: "active".to_string(),
                domain: None,
            }),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-tenant-id", HeaderValue::from_static("tenant-suspended"));

        let response = request_tenant_context(&db, user.clone(), &headers, false)
            .await
            .expect_err("resource context rejects inactive tenant");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let ctx = request_tenant_context(&db, user, &headers, true)
            .await
            .expect("tenant management context allows inactive tenant");

        assert_eq!(ctx.id, "tenant-suspended");
    }

    #[tokio::test]
    async fn tenant_admin_request_context_ignores_selected_tenant_header() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        let mut headers = HeaderMap::new();
        headers.insert("x-tenant-id", HeaderValue::from_static("other-tenant"));

        let ctx = request_tenant_context(
            &db,
            SessionUser {
                id: 1,
                username: "tenant-admin".to_string(),
                email: "admin@example.com".to_string(),
                role: TenantRole::TenantAdmin,
                tenant: Some(TenantSummary {
                    id: DEFAULT_TENANT_ID.to_string(),
                    name: "Default".to_string(),
                    status: "active".to_string(),
                    domain: None,
                }),
            },
            &headers,
            false,
        )
        .await
        .expect("tenant context");

        assert_eq!(ctx.id, DEFAULT_TENANT_ID);
        assert_eq!(ctx.role, TenantRole::TenantAdmin);
    }

    #[tokio::test]
    async fn tenant_user_cannot_create_extension() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        let ctx = TenantContext {
            id: DEFAULT_TENANT_ID.to_string(),
            name: "Default".to_string(),
            role: TenantRole::TenantUser,
        };

        let response = create_extension_for_tenant(
            &db,
            &ctx,
            CreateExtensionRequest {
                extension: "1001".to_string(),
                display_name: None,
                email: None,
                status: None,
                login_disabled: None,
                voicemail_disabled: None,
                allow_guest_calls: None,
                notes: None,
            },
        )
        .await
        .expect_err("tenant user write denied");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
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
    async fn update_extension_preserves_tenant_scope() {
        use crate::models::{
            extension::{Column as ExtensionColumn, Entity as ExtensionEntity},
            tenant::{ActiveModel as TenantActiveModel, Entity as TenantEntity},
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
                extension: "1002".to_string(),
                display_name: Some("Bob".to_string()),
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

        let updated = update_extension_for_tenant(
            &db,
            &ctx,
            summary.id,
            UpdateExtensionRequest {
                extension: Some("1003".to_string()),
                display_name: Some(Some("Bob Updated".to_string())),
                email: Some(Some("bob@example.com".to_string())),
                status: Some(Some("disabled".to_string())),
                login_disabled: Some(true),
                voicemail_disabled: None,
                allow_guest_calls: None,
                notes: Some(Some("updated".to_string())),
            },
        )
        .await
        .expect("update extension");

        assert_eq!(updated.extension, "1003");
        assert_eq!(updated.display_name.as_deref(), Some("Bob Updated"));
        assert_eq!(updated.email.as_deref(), Some("bob@example.com"));
        assert_eq!(updated.status.as_deref(), Some("disabled"));
        assert!(updated.login_disabled);
        assert_eq!(updated.tenant_id, summary.tenant_id);

        let now = chrono::Utc::now();
        TenantActiveModel {
            slug: Set("tenant-b".to_string()),
            name: Set("Tenant B".to_string()),
            status: Set("active".to_string()),
            domain: Set(None),
            max_concurrent_calls: Set(None),
            max_trunks: Set(None),
            storage_prefix: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert tenant b");

        let other_ctx = TenantContext {
            id: "tenant-b".to_string(),
            name: "Tenant B".to_string(),
            role: TenantRole::TenantAdmin,
        };
        let response = update_extension_for_tenant(
            &db,
            &other_ctx,
            summary.id,
            UpdateExtensionRequest {
                extension: None,
                display_name: Some(Some("Should Not Apply".to_string())),
                email: None,
                status: None,
                login_disabled: None,
                voicemail_disabled: None,
                allow_guest_calls: None,
                notes: None,
            },
        )
        .await
        .expect_err("cross tenant update is hidden");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let unchanged = ExtensionEntity::find()
            .filter(ExtensionColumn::Id.eq(summary.id))
            .one(&db)
            .await
            .expect("query extension")
            .expect("extension");
        assert_eq!(unchanged.display_name.as_deref(), Some("Bob Updated"));
        assert_eq!(
            TenantEntity::find().all(&db).await.expect("tenants").len(),
            2
        );
    }

    #[tokio::test]
    async fn delete_extension_requires_tenant_scope() {
        use crate::models::extension::Entity as ExtensionEntity;

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_extension_for_tenant(
            &db,
            &ctx,
            CreateExtensionRequest {
                extension: "1004".to_string(),
                display_name: Some("Carol".to_string()),
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

        delete_extension_for_tenant(&db, &ctx, summary.id)
            .await
            .expect("delete extension");

        let deleted = ExtensionEntity::find_by_id(summary.id)
            .one(&db)
            .await
            .expect("query extension");
        assert!(deleted.is_none());
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

    #[tokio::test]
    async fn update_sip_trunk_preserves_tenant_scope() {
        use crate::models::{
            sip_trunk::{Column as SipTrunkColumn, Entity as SipTrunkEntity, SipTrunkStatus},
            tenant::{ActiveModel as TenantActiveModel, Entity as TenantEntity},
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
                name: "carrier-b".to_string(),
                display_name: Some("Carrier B".to_string()),
                carrier: None,
                description: None,
                status: None,
                direction: None,
                sip_server: Some("sip-b.example.com".to_string()),
                sip_transport: None,
                outbound_proxy: None,
                auth_username: Some("user-b".to_string()),
                auth_password: Some("secret-b".to_string()),
                is_active: None,
                register_enabled: None,
            },
        )
        .await
        .expect("create sip trunk");

        let updated = update_sip_trunk_for_tenant(
            &db,
            &ctx,
            summary.id,
            UpdateSipTrunkRequest {
                name: Some("carrier-b-updated".to_string()),
                display_name: Some(Some("Carrier B Updated".to_string())),
                carrier: Some(Some("CarrierCo".to_string())),
                description: Some(Some("updated".to_string())),
                status: Some(SipTrunkStatus::Warning),
                direction: None,
                sip_server: Some(Some("sip-b2.example.com".to_string())),
                sip_transport: None,
                outbound_proxy: None,
                auth_username: Some(Some("user-b2".to_string())),
                auth_password: Some(Some("secret-b2".to_string())),
                is_active: Some(false),
                register_enabled: Some(true),
            },
        )
        .await
        .expect("update sip trunk");

        assert_eq!(updated.name, "carrier-b-updated");
        assert_eq!(updated.display_name.as_deref(), Some("Carrier B Updated"));
        assert_eq!(updated.carrier.as_deref(), Some("CarrierCo"));
        assert_eq!(updated.status, SipTrunkStatus::Warning);
        assert_eq!(updated.sip_server.as_deref(), Some("sip-b2.example.com"));
        assert!(!updated.is_active);
        assert!(updated.register_enabled);
        assert_eq!(updated.tenant_id, summary.tenant_id);

        let now = chrono::Utc::now();
        TenantActiveModel {
            slug: Set("tenant-c".to_string()),
            name: Set("Tenant C".to_string()),
            status: Set("active".to_string()),
            domain: Set(None),
            max_concurrent_calls: Set(None),
            max_trunks: Set(None),
            storage_prefix: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert tenant c");

        let other_ctx = TenantContext {
            id: "tenant-c".to_string(),
            name: "Tenant C".to_string(),
            role: TenantRole::TenantAdmin,
        };
        let response = update_sip_trunk_for_tenant(
            &db,
            &other_ctx,
            summary.id,
            UpdateSipTrunkRequest {
                name: None,
                display_name: Some(Some("Should Not Apply".to_string())),
                carrier: None,
                description: None,
                status: None,
                direction: None,
                sip_server: None,
                sip_transport: None,
                outbound_proxy: None,
                auth_username: None,
                auth_password: None,
                is_active: None,
                register_enabled: None,
            },
        )
        .await
        .expect_err("cross tenant update is hidden");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let unchanged = SipTrunkEntity::find()
            .filter(SipTrunkColumn::Id.eq(summary.id))
            .one(&db)
            .await
            .expect("query sip trunk")
            .expect("sip trunk");
        assert_eq!(unchanged.display_name.as_deref(), Some("Carrier B Updated"));
        assert_eq!(unchanged.auth_password.as_deref(), Some("secret-b2"));
        assert_eq!(
            TenantEntity::find().all(&db).await.expect("tenants").len(),
            2
        );
    }

    #[tokio::test]
    async fn delete_sip_trunk_requires_tenant_scope() {
        use crate::models::sip_trunk::Entity as SipTrunkEntity;

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_sip_trunk_for_tenant(
            &db,
            &ctx,
            CreateSipTrunkRequest {
                name: "carrier-delete".to_string(),
                display_name: Some("Carrier Delete".to_string()),
                carrier: None,
                description: None,
                status: None,
                direction: None,
                sip_server: Some("sip-delete.example.com".to_string()),
                sip_transport: None,
                outbound_proxy: None,
                auth_username: None,
                auth_password: None,
                is_active: None,
                register_enabled: None,
            },
        )
        .await
        .expect("create sip trunk");

        delete_sip_trunk_for_tenant(&db, &ctx, summary.id)
            .await
            .expect("delete sip trunk");

        let deleted = SipTrunkEntity::find_by_id(summary.id)
            .one(&db)
            .await
            .expect("query sip trunk");
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn create_route_sets_current_tenant_id() {
        use crate::models::{
            routing::{Column as RouteColumn, Entity as RouteEntity},
            tenant::{Column as TenantColumn, Entity as TenantEntity},
        };

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_route_for_tenant(
            &db,
            &ctx,
            CreateRouteRequest {
                name: "outbound-default".to_string(),
                description: None,
                direction: None,
                priority: None,
                is_active: None,
                selection_strategy: None,
                source_trunk_id: None,
                default_trunk_id: None,
                source_pattern: None,
                destination_pattern: Some("^\\+?[0-9]+$".to_string()),
                owner: None,
            },
        )
        .await
        .expect("create route");

        let tenant = TenantEntity::find()
            .filter(TenantColumn::Slug.eq(DEFAULT_TENANT_ID))
            .one(&db)
            .await
            .expect("query tenant")
            .expect("default tenant");
        let route = RouteEntity::find()
            .filter(RouteColumn::Name.eq("outbound-default"))
            .one(&db)
            .await
            .expect("query route")
            .expect("route");

        assert_eq!(summary.tenant_id, Some(tenant.id));
        assert_eq!(route.tenant_id, Some(tenant.id));
    }

    #[tokio::test]
    async fn update_route_preserves_tenant_scope() {
        use crate::models::{
            routing::{
                Column as RouteColumn, Entity as RouteEntity, RoutingDirection,
                RoutingSelectionStrategy,
            },
            tenant::{ActiveModel as TenantActiveModel, Entity as TenantEntity},
        };

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_route_for_tenant(
            &db,
            &ctx,
            CreateRouteRequest {
                name: "outbound-a".to_string(),
                description: None,
                direction: None,
                priority: None,
                is_active: None,
                selection_strategy: None,
                source_trunk_id: None,
                default_trunk_id: None,
                source_pattern: None,
                destination_pattern: Some("^1[0-9]+$".to_string()),
                owner: None,
            },
        )
        .await
        .expect("create route");

        let updated = update_route_for_tenant(
            &db,
            &ctx,
            summary.id,
            UpdateRouteRequest {
                name: Some("outbound-a-updated".to_string()),
                description: Some(Some("updated".to_string())),
                direction: Some(RoutingDirection::Inbound),
                priority: Some(10),
                is_active: Some(false),
                selection_strategy: Some(RoutingSelectionStrategy::Hash),
                source_trunk_id: None,
                default_trunk_id: None,
                source_pattern: Some(Some("^1001$".to_string())),
                destination_pattern: Some(Some("^2[0-9]+$".to_string())),
                owner: Some(Some("ops".to_string())),
            },
        )
        .await
        .expect("update route");

        assert_eq!(updated.name, "outbound-a-updated");
        assert_eq!(updated.description.as_deref(), Some("updated"));
        assert_eq!(updated.direction, RoutingDirection::Inbound);
        assert_eq!(updated.priority, 10);
        assert!(!updated.is_active);
        assert_eq!(updated.selection_strategy, RoutingSelectionStrategy::Hash);
        assert_eq!(updated.source_pattern.as_deref(), Some("^1001$"));
        assert_eq!(updated.destination_pattern.as_deref(), Some("^2[0-9]+$"));
        assert_eq!(updated.owner.as_deref(), Some("ops"));
        assert_eq!(updated.tenant_id, summary.tenant_id);

        let now = chrono::Utc::now();
        TenantActiveModel {
            slug: Set("tenant-d".to_string()),
            name: Set("Tenant D".to_string()),
            status: Set("active".to_string()),
            domain: Set(None),
            max_concurrent_calls: Set(None),
            max_trunks: Set(None),
            storage_prefix: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert tenant d");

        let other_ctx = TenantContext {
            id: "tenant-d".to_string(),
            name: "Tenant D".to_string(),
            role: TenantRole::TenantAdmin,
        };
        let response = update_route_for_tenant(
            &db,
            &other_ctx,
            summary.id,
            UpdateRouteRequest {
                name: None,
                description: Some(Some("Should Not Apply".to_string())),
                direction: None,
                priority: None,
                is_active: None,
                selection_strategy: None,
                source_trunk_id: None,
                default_trunk_id: None,
                source_pattern: None,
                destination_pattern: None,
                owner: None,
            },
        )
        .await
        .expect_err("cross tenant update is hidden");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let unchanged = RouteEntity::find()
            .filter(RouteColumn::Id.eq(summary.id))
            .one(&db)
            .await
            .expect("query route")
            .expect("route");
        assert_eq!(unchanged.description.as_deref(), Some("updated"));
        assert_eq!(
            TenantEntity::find().all(&db).await.expect("tenants").len(),
            2
        );
    }

    #[tokio::test]
    async fn delete_route_requires_tenant_scope() {
        use crate::models::routing::Entity as RouteEntity;

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_route_for_tenant(
            &db,
            &ctx,
            CreateRouteRequest {
                name: "outbound-delete".to_string(),
                description: None,
                direction: None,
                priority: None,
                is_active: None,
                selection_strategy: None,
                source_trunk_id: None,
                default_trunk_id: None,
                source_pattern: None,
                destination_pattern: Some("^9[0-9]+$".to_string()),
                owner: None,
            },
        )
        .await
        .expect("create route");

        delete_route_for_tenant(&db, &ctx, summary.id)
            .await
            .expect("delete route");

        let deleted = RouteEntity::find_by_id(summary.id)
            .one(&db)
            .await
            .expect("query route");
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn route_trunk_references_must_stay_in_tenant() {
        use crate::models::{
            routing::Entity as RouteEntity, tenant::ActiveModel as TenantActiveModel,
        };

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let default_trunk = create_sip_trunk_for_tenant(
            &db,
            &ctx,
            CreateSipTrunkRequest {
                name: "default-route-trunk".to_string(),
                display_name: None,
                carrier: None,
                description: None,
                status: None,
                direction: None,
                sip_server: Some("sip-default.example.com".to_string()),
                sip_transport: None,
                outbound_proxy: None,
                auth_username: None,
                auth_password: None,
                is_active: None,
                register_enabled: None,
            },
        )
        .await
        .expect("create default trunk");

        let route = create_route_for_tenant(
            &db,
            &ctx,
            CreateRouteRequest {
                name: "route-with-trunk".to_string(),
                description: None,
                direction: None,
                priority: None,
                is_active: None,
                selection_strategy: None,
                source_trunk_id: Some(default_trunk.id),
                default_trunk_id: Some(default_trunk.id),
                source_pattern: None,
                destination_pattern: Some("^3[0-9]+$".to_string()),
                owner: None,
            },
        )
        .await
        .expect("create route with same-tenant trunk");
        assert_eq!(route.source_trunk_id, Some(default_trunk.id));
        assert_eq!(route.default_trunk_id, Some(default_trunk.id));

        let now = chrono::Utc::now();
        TenantActiveModel {
            slug: Set("tenant-route-ref".to_string()),
            name: Set("Tenant Route Ref".to_string()),
            status: Set("active".to_string()),
            domain: Set(None),
            max_concurrent_calls: Set(None),
            max_trunks: Set(None),
            storage_prefix: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert other tenant");

        let other_ctx = TenantContext {
            id: "tenant-route-ref".to_string(),
            name: "Tenant Route Ref".to_string(),
            role: TenantRole::TenantAdmin,
        };
        let other_trunk = create_sip_trunk_for_tenant(
            &db,
            &other_ctx,
            CreateSipTrunkRequest {
                name: "other-route-trunk".to_string(),
                display_name: None,
                carrier: None,
                description: None,
                status: None,
                direction: None,
                sip_server: Some("sip-other.example.com".to_string()),
                sip_transport: None,
                outbound_proxy: None,
                auth_username: None,
                auth_password: None,
                is_active: None,
                register_enabled: None,
            },
        )
        .await
        .expect("create other tenant trunk");

        let create_response = create_route_for_tenant(
            &db,
            &ctx,
            CreateRouteRequest {
                name: "route-cross-trunk".to_string(),
                description: None,
                direction: None,
                priority: None,
                is_active: None,
                selection_strategy: None,
                source_trunk_id: Some(other_trunk.id),
                default_trunk_id: None,
                source_pattern: None,
                destination_pattern: Some("^4[0-9]+$".to_string()),
                owner: None,
            },
        )
        .await
        .expect_err("cross tenant trunk is rejected on create");
        assert_eq!(create_response.status(), StatusCode::BAD_REQUEST);

        let update_response = update_route_for_tenant(
            &db,
            &ctx,
            route.id,
            UpdateRouteRequest {
                name: None,
                description: None,
                direction: None,
                priority: None,
                is_active: None,
                selection_strategy: None,
                source_trunk_id: Some(Some(other_trunk.id)),
                default_trunk_id: None,
                source_pattern: None,
                destination_pattern: None,
                owner: None,
            },
        )
        .await
        .expect_err("cross tenant trunk is rejected on update");
        assert_eq!(update_response.status(), StatusCode::BAD_REQUEST);

        let cleared = update_route_for_tenant(
            &db,
            &ctx,
            route.id,
            UpdateRouteRequest {
                name: None,
                description: None,
                direction: None,
                priority: None,
                is_active: None,
                selection_strategy: None,
                source_trunk_id: Some(None),
                default_trunk_id: Some(None),
                source_pattern: None,
                destination_pattern: None,
                owner: None,
            },
        )
        .await
        .expect("clear trunk refs");
        assert!(cleared.source_trunk_id.is_none());
        assert!(cleared.default_trunk_id.is_none());

        let stored = RouteEntity::find_by_id(route.id)
            .one(&db)
            .await
            .expect("query route")
            .expect("route");
        assert!(stored.source_trunk_id.is_none());
        assert!(stored.default_trunk_id.is_none());
    }

    #[tokio::test]
    async fn create_user_sets_current_tenant_id_and_hashes_password() {
        use crate::models::{
            tenant::{Column as TenantColumn, Entity as TenantEntity},
            user::{Column as UserColumn, Entity as UserEntity},
        };

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_user_for_tenant(
            &db,
            &ctx,
            CreateUserRequest {
                username: "alice".to_string(),
                email: "alice@example.com".to_string(),
                password: "secret-password".to_string(),
                is_active: None,
                is_staff: Some(true),
                is_superuser: None,
            },
        )
        .await
        .expect("create user");

        let tenant = TenantEntity::find()
            .filter(TenantColumn::Slug.eq(DEFAULT_TENANT_ID))
            .one(&db)
            .await
            .expect("query tenant")
            .expect("default tenant");
        let user = UserEntity::find()
            .filter(UserColumn::Username.eq("alice"))
            .one(&db)
            .await
            .expect("query user")
            .expect("user");

        assert_eq!(summary.tenant_id, Some(tenant.id));
        assert_eq!(user.tenant_id, Some(tenant.id));
        assert_ne!(user.password_hash, "secret-password");
        assert!(user.password_hash.starts_with("$argon2"));
    }

    #[tokio::test]
    async fn tenant_admin_cannot_create_superuser() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let response = create_user_for_tenant(
            &db,
            &ctx,
            CreateUserRequest {
                username: "super-denied".to_string(),
                email: "super-denied@example.com".to_string(),
                password: "secret-password".to_string(),
                is_active: None,
                is_staff: Some(true),
                is_superuser: Some(true),
            },
        )
        .await
        .expect_err("tenant admin cannot create superuser");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn tenant_admin_cannot_promote_superuser() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_user_for_tenant(
            &db,
            &ctx,
            CreateUserRequest {
                username: "promote-denied".to_string(),
                email: "promote-denied@example.com".to_string(),
                password: "secret-password".to_string(),
                is_active: None,
                is_staff: None,
                is_superuser: None,
            },
        )
        .await
        .expect("create user");

        let response = update_user_for_tenant(
            &db,
            &ctx,
            summary.id,
            UpdateUserRequest {
                username: None,
                email: None,
                password: None,
                is_active: None,
                is_staff: None,
                is_superuser: Some(true),
            },
        )
        .await
        .expect_err("tenant admin cannot promote superuser");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn tenant_admin_cannot_modify_superuser_flag() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_user_for_tenant(
            &db,
            &ctx,
            CreateUserRequest {
                username: "super-flag-denied".to_string(),
                email: "super-flag-denied@example.com".to_string(),
                password: "secret-password".to_string(),
                is_active: None,
                is_staff: None,
                is_superuser: None,
            },
        )
        .await
        .expect("create user");

        let response = update_user_for_tenant(
            &db,
            &ctx,
            summary.id,
            UpdateUserRequest {
                username: None,
                email: None,
                password: None,
                is_active: None,
                is_staff: None,
                is_superuser: Some(false),
            },
        )
        .await
        .expect_err("tenant admin cannot modify superuser flag");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn login_sets_signed_session_cookie() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        create_user_for_tenant(
            &db,
            &ctx,
            CreateUserRequest {
                username: "login-user".to_string(),
                email: "login-user@example.com".to_string(),
                password: "secret-password".to_string(),
                is_active: None,
                is_staff: Some(true),
                is_superuser: None,
            },
        )
        .await
        .expect("create user");

        let (user, cookie) = authenticate_login(
            &db,
            "test-secret",
            LoginRequest {
                username: "login-user".to_string(),
                password: "secret-password".to_string(),
                tenant: Some(DEFAULT_TENANT_ID.to_string()),
            },
        )
        .await
        .expect("login");

        assert_eq!(user.username, "login-user");
        assert_eq!(user.role, TenantRole::TenantAdmin);
        assert!(cookie.to_str().unwrap().starts_with("cloudpbx_session="));
        assert!(cookie.to_str().unwrap().contains("HttpOnly"));
    }

    #[tokio::test]
    async fn session_restores_user_from_cookie() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        create_user_for_tenant(
            &db,
            &ctx,
            CreateUserRequest {
                username: "session-user".to_string(),
                email: "session-user@example.com".to_string(),
                password: "secret-password".to_string(),
                is_active: None,
                is_staff: None,
                is_superuser: None,
            },
        )
        .await
        .expect("create user");

        let (_, cookie) = authenticate_login(
            &db,
            "test-secret",
            LoginRequest {
                username: "session-user@example.com".to_string(),
                password: "secret-password".to_string(),
                tenant: Some(DEFAULT_TENANT_ID.to_string()),
            },
        )
        .await
        .expect("login");

        let cookie_pair = cookie.to_str().unwrap().split(';').next().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_str(cookie_pair).unwrap());

        let user = current_session_user(&db, "test-secret", &headers)
            .await
            .expect("session user");
        assert_eq!(user.username, "session-user");
        assert_eq!(user.role, TenantRole::TenantUser);
        assert_eq!(user.tenant.unwrap().id, DEFAULT_TENANT_ID);
    }

    #[tokio::test]
    async fn login_rejects_wrong_password() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        create_user_for_tenant(
            &db,
            &ctx,
            CreateUserRequest {
                username: "reject-user".to_string(),
                email: "reject-user@example.com".to_string(),
                password: "secret-password".to_string(),
                is_active: None,
                is_staff: None,
                is_superuser: None,
            },
        )
        .await
        .expect("create user");

        let response = authenticate_login(
            &db,
            "test-secret",
            LoginRequest {
                username: "reject-user".to_string(),
                password: "wrong-password".to_string(),
                tenant: Some(DEFAULT_TENANT_ID.to_string()),
            },
        )
        .await
        .expect_err("reject wrong password");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn update_user_preserves_tenant_scope_and_hashes_password() {
        use crate::models::{
            tenant::{ActiveModel as TenantActiveModel, Entity as TenantEntity},
            user::{Column as UserColumn, Entity as UserEntity},
        };

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_user_for_tenant(
            &db,
            &ctx,
            CreateUserRequest {
                username: "bob".to_string(),
                email: "bob@example.com".to_string(),
                password: "secret-password".to_string(),
                is_active: None,
                is_staff: None,
                is_superuser: None,
            },
        )
        .await
        .expect("create user");

        let original = UserEntity::find_by_id(summary.id)
            .one(&db)
            .await
            .expect("query user")
            .expect("user");

        let updated = update_user_for_tenant(
            &db,
            &ctx,
            summary.id,
            UpdateUserRequest {
                username: Some("bob-updated".to_string()),
                email: Some("bob-updated@example.com".to_string()),
                password: Some("new-secret-password".to_string()),
                is_active: Some(false),
                is_staff: Some(true),
                is_superuser: None,
            },
        )
        .await
        .expect("update user");

        assert_eq!(updated.username, "bob-updated");
        assert_eq!(updated.email, "bob-updated@example.com");
        assert!(!updated.is_active);
        assert!(updated.is_staff);
        assert!(!updated.is_superuser);
        assert_eq!(updated.tenant_id, summary.tenant_id);

        let changed = UserEntity::find()
            .filter(UserColumn::Id.eq(summary.id))
            .one(&db)
            .await
            .expect("query updated user")
            .expect("updated user");
        assert_ne!(changed.password_hash, original.password_hash);
        assert!(changed.password_hash.starts_with("$argon2"));
        assert!(changed.reset_token.is_none());
        assert!(changed.reset_token_expires.is_none());

        let now = chrono::Utc::now();
        TenantActiveModel {
            slug: Set("tenant-e".to_string()),
            name: Set("Tenant E".to_string()),
            status: Set("active".to_string()),
            domain: Set(None),
            max_concurrent_calls: Set(None),
            max_trunks: Set(None),
            storage_prefix: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert tenant e");

        let other_ctx = TenantContext {
            id: "tenant-e".to_string(),
            name: "Tenant E".to_string(),
            role: TenantRole::TenantAdmin,
        };
        let response = update_user_for_tenant(
            &db,
            &other_ctx,
            summary.id,
            UpdateUserRequest {
                username: Some("should-not-apply".to_string()),
                email: None,
                password: None,
                is_active: None,
                is_staff: None,
                is_superuser: None,
            },
        )
        .await
        .expect_err("cross tenant update is hidden");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let unchanged = UserEntity::find_by_id(summary.id)
            .one(&db)
            .await
            .expect("query user")
            .expect("user");
        assert_eq!(unchanged.username, "bob-updated");
        assert_eq!(
            TenantEntity::find().all(&db).await.expect("tenants").len(),
            2
        );
    }

    #[tokio::test]
    async fn delete_user_requires_tenant_scope() {
        use crate::models::user::Entity as UserEntity;

        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");
        Migrator::up(&db, None).await.expect("run migrations");

        let ctx = TenantContext::default_tenant_admin();
        let summary = create_user_for_tenant(
            &db,
            &ctx,
            CreateUserRequest {
                username: "delete-user".to_string(),
                email: "delete-user@example.com".to_string(),
                password: "secret-password".to_string(),
                is_active: None,
                is_staff: None,
                is_superuser: None,
            },
        )
        .await
        .expect("create user");

        delete_user_for_tenant(&db, &ctx, summary.id)
            .await
            .expect("delete user");

        let deleted = UserEntity::find_by_id(summary.id)
            .one(&db)
            .await
            .expect("query user");
        assert!(deleted.is_none());
    }
}
