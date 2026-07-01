use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHasher};
use axum::{
    Json, Router,
    extract::{FromRequestParts, Path as AxumPath, State},
    http::{HeaderMap, HeaderValue, StatusCode, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect,
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
struct UpdateExtensionRequest {
    extension: Option<String>,
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

#[derive(Debug, Deserialize)]
struct UpdateSipTrunkRequest {
    name: Option<String>,
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
struct CreateUserRequest {
    username: String,
    email: String,
    password: String,
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
    Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/session", get(session))
        .route("/api/tenants", get(list_tenants))
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
    if payload.display_name.is_some() {
        active.display_name = Set(clean_optional_string(payload.display_name));
    }
    if payload.email.is_some() {
        active.email = Set(clean_optional_string(payload.email));
    }
    if payload.status.is_some() {
        active.status = Set(clean_optional_string(payload.status));
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
    if payload.notes.is_some() {
        active.notes = Set(clean_optional_string(payload.notes));
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
    if payload.display_name.is_some() {
        active.display_name = Set(clean_optional_string(payload.display_name));
    }
    if payload.carrier.is_some() {
        active.carrier = Set(clean_optional_string(payload.carrier));
    }
    if payload.description.is_some() {
        active.description = Set(clean_optional_string(payload.description));
    }
    if let Some(status) = payload.status {
        active.status = Set(status);
    }
    if let Some(direction) = payload.direction {
        active.direction = Set(direction);
    }
    if payload.sip_server.is_some() {
        active.sip_server = Set(clean_optional_string(payload.sip_server));
    }
    if let Some(sip_transport) = payload.sip_transport {
        active.sip_transport = Set(sip_transport);
    }
    if payload.outbound_proxy.is_some() {
        active.outbound_proxy = Set(clean_optional_string(payload.outbound_proxy));
    }
    if payload.auth_username.is_some() {
        active.auth_username = Set(clean_optional_string(payload.auth_username));
    }
    if payload.auth_password.is_some() {
        active.auth_password = Set(clean_optional_string(payload.auth_password));
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
    if payload.description.is_some() {
        active.description = Set(clean_optional_string(payload.description));
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
    if payload.source_trunk_id.is_some() {
        active.source_trunk_id = Set(payload.source_trunk_id);
    }
    if payload.default_trunk_id.is_some() {
        active.default_trunk_id = Set(payload.default_trunk_id);
    }
    if payload.source_pattern.is_some() {
        active.source_pattern = Set(clean_optional_string(payload.source_pattern));
    }
    if payload.destination_pattern.is_some() {
        active.destination_pattern = Set(clean_optional_string(payload.destination_pattern));
    }
    if payload.owner.is_some() {
        active.owner = Set(clean_optional_string(payload.owner));
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

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(payload.password.as_bytes(), &salt)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "password_hash_failed",
                    message: "Failed to hash password.",
                }),
            )
                .into_response()
        })?
        .to_string();

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
                display_name: Some("Bob Updated".to_string()),
                email: Some("bob@example.com".to_string()),
                status: Some("disabled".to_string()),
                login_disabled: Some(true),
                voicemail_disabled: None,
                allow_guest_calls: None,
                notes: Some("updated".to_string()),
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
                display_name: Some("Should Not Apply".to_string()),
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
                display_name: Some("Carrier B Updated".to_string()),
                carrier: Some("CarrierCo".to_string()),
                description: Some("updated".to_string()),
                status: Some(SipTrunkStatus::Warning),
                direction: None,
                sip_server: Some("sip-b2.example.com".to_string()),
                sip_transport: None,
                outbound_proxy: None,
                auth_username: Some("user-b2".to_string()),
                auth_password: Some("secret-b2".to_string()),
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
                display_name: Some("Should Not Apply".to_string()),
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
                description: Some("updated".to_string()),
                direction: Some(RoutingDirection::Inbound),
                priority: Some(10),
                is_active: Some(false),
                selection_strategy: Some(RoutingSelectionStrategy::Hash),
                source_trunk_id: None,
                default_trunk_id: None,
                source_pattern: Some("^1001$".to_string()),
                destination_pattern: Some("^2[0-9]+$".to_string()),
                owner: Some("ops".to_string()),
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
                description: Some("Should Not Apply".to_string()),
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
}
