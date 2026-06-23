//! HTTP/REST API for the Control Plane admin console.
//!
//! Serves the Vue3 single-page app (`web/dist`) plus a JSON API consumed by it.
//!
//! ## Principals
//! - **superadmin** — the config-backed platform operator (`tenant_id = None`).
//!   Logs in with just username + password (no domain).
//! - **tenant_admin / tenant_user** — DB-backed IAM accounts under a tenant.
//!   Log in with their tenant's domain + username + password. The domain
//!   resolves to a `tenant_id`; the username is matched within that tenant.
//!
//! Permissions (`auth::permissions`) gate tenant-scoped routes; admins bypass
//! the checks but every principal is confined to its own tenant by the scoping
//! helpers below.

use crate::auth::permissions::{self, roles};
use crate::did_service::{CreateDidRequest, DidService, UpdateDidRequest};
use crate::raft::registry::RaftRegistry;
use crate::settings::{KEY_BASE_DOMAIN, PlatformSettings};
use crate::store::Store;
use crate::store::crud::{AclInput, ExtensionInput, RouteInput, TrunkInput};
use crate::tenant_service::{
    CreateTenantRequest, TenantService, UpdateDomainRequest, UpdateTenantRequest,
};
use crate::tenant_user_service::{
    CreateTenantUserRequest, TenantUserService, UpdateTenantUserRequest,
};
use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::net::SocketAddr;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use dashmap::DashMap;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
};
use tracing::{info, warn};

const SESSION_TTL_HOURS: i64 = 12;

// Login brute-force throttle: after this many failures from one client within
// the window, further attempts are rejected with 429 until the window elapses.
const LOGIN_MAX_FAILS: u32 = 5;
const LOGIN_WINDOW_SECS: i64 = 60;

#[derive(Clone)]
pub struct LoginGate {
    fails: u32,
    window_start: DateTime<Utc>,
}

// ── Shared state ────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct HttpState {
    pub db: DatabaseConnection,
    pub store: Arc<Store>,
    pub workers: RaftRegistry,
    pub sessions: Arc<DashMap<String, Session>>,
    /// Per-client failed-login counters for brute-force throttling.
    pub login_gate: Arc<DashMap<String, LoginGate>>,
    pub admin_username: String,
    pub admin_password: String,
}

impl HttpState {
    /// Current platform wildcard base domain (empty if unset).
    async fn base_domain(&self) -> String {
        PlatformSettings::new(&self.db).base_domain().await
    }

    async fn tenants(&self) -> TenantService<'_> {
        TenantService::new(&self.db, self.base_domain().await)
    }
}

#[derive(Clone, Serialize)]
pub struct UserInfo {
    pub username: String,
    pub role: String,
    pub tenant_id: Option<i64>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Clone)]
pub struct Session {
    pub user: UserInfo,
    pub expires: DateTime<Utc>,
}

// ── Error type ──────────────────────────────────────────────────────────────

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }
    fn internal(e: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
    fn bad(e: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::BAD_REQUEST, e.to_string())
    }
    fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden")
    }
    fn not_found(e: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::NOT_FOUND, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ── Authorization helpers ─────────────────────────────────────────────────────

fn is_superadmin(u: &UserInfo) -> bool {
    u.role == roles::SUPERADMIN
}

fn require_perm(u: &UserInfo, perm: &str) -> ApiResult<()> {
    if permissions::has_permission(&u.role, &u.permissions, perm) {
        Ok(())
    } else {
        Err(ApiError::new(StatusCode::FORBIDDEN, format!("missing permission: {perm}")))
    }
}

fn require_superadmin(u: &UserInfo) -> ApiResult<()> {
    is_superadmin(u).then_some(()).ok_or_else(ApiError::forbidden)
}

/// Resolve the tenant scope for a *read*. Superadmin may filter by an explicit
/// `tenant_id` (or `None` = all tenants); tenant principals are pinned to their
/// own tenant and may not query another.
fn read_scope(u: &UserInfo, q: Option<i64>) -> ApiResult<Option<i64>> {
    if is_superadmin(u) {
        return Ok(q);
    }
    let tid = u.tenant_id.ok_or_else(ApiError::forbidden)?;
    if matches!(q, Some(other) if other != tid) {
        return Err(ApiError::forbidden());
    }
    Ok(Some(tid))
}

/// Tenant a *newly created* row belongs to. Superadmin uses the explicit
/// `tenant_id` (may be `None` = a global/shared row); tenant principals always
/// create within their own tenant.
fn create_tenant_scope(u: &UserInfo, q: Option<i64>) -> ApiResult<Option<i64>> {
    if is_superadmin(u) {
        return Ok(q);
    }
    let tid = u.tenant_id.ok_or_else(ApiError::forbidden)?;
    if matches!(q, Some(other) if other != tid) {
        return Err(ApiError::forbidden());
    }
    Ok(Some(tid))
}

/// Tenant restriction applied to *update/delete*. Superadmin → `None` (may touch
/// any row by id); tenant principals → `Some(their tenant)` so they can only
/// mutate their own rows.
fn mutate_scope(u: &UserInfo) -> Option<i64> {
    if is_superadmin(u) { None } else { u.tenant_id }
}

/// The single tenant a tenant-admin self-service action applies to. Superadmin
/// must name it explicitly; tenant principals use their own.
fn required_tenant(u: &UserInfo, q: Option<i64>) -> ApiResult<i64> {
    if is_superadmin(u) {
        return q.ok_or_else(|| ApiError::bad("tenant_id is required"));
    }
    let tid = u.tenant_id.ok_or_else(ApiError::forbidden)?;
    if matches!(q, Some(other) if other != tid) {
        return Err(ApiError::forbidden());
    }
    Ok(tid)
}

/// Reject creating a trunk once the tenant is at its `max_trunks` quota.
async fn enforce_trunk_quota(state: &HttpState, tenant_id: i64) -> ApiResult<()> {
    let tenant = TenantService::new(&state.db, "")
        .get_model(tenant_id)
        .await
        .map_err(ApiError::bad)?;
    if let Some(max) = tenant.max_trunks {
        let count = state
            .store
            .count_trunks_for_tenant(tenant_id)
            .await
            .map_err(ApiError::internal)?;
        if count >= max as u64 {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                format!("trunk quota reached for this tenant ({count}/{max})"),
            ));
        }
    }
    Ok(())
}

/// Reject assigning a DID once the tenant is at its `max_dids` quota.
async fn enforce_did_quota(state: &HttpState, tenant_id: i64) -> ApiResult<()> {
    let tenant = TenantService::new(&state.db, "")
        .get_model(tenant_id)
        .await
        .map_err(ApiError::bad)?;
    if let Some(max) = tenant.max_dids {
        let count = DidService::new(&state.db)
            .count_for_tenant(tenant_id)
            .await
            .map_err(ApiError::internal)?;
        if count >= max as u64 {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                format!("DID quota reached for this tenant ({count}/{max})"),
            ));
        }
    }
    Ok(())
}

/// 0 rows affected → 404.
fn affected_or_404(n: u64) -> ApiResult<StatusCode> {
    if n == 0 {
        Err(ApiError::not_found("not found or not in your tenant"))
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

// ── Router ──────────────────────────────────────────────────────────────────

/// Build the full HTTP router: SPA static files + JSON API.
pub fn build_router(state: HttpState, web_dir: &str) -> Router {
    let api = Router::new()
        // public
        .route("/auth/login", post(login))
        // protected
        .route("/auth/logout", post(logout))
        .route("/me", get(me))
        .route("/me/password", post(change_my_password))
        .route("/stats", get(stats))
        .route("/tenant-stats", get(tenant_stats))
        // platform (superadmin)
        .route("/platform/settings", get(get_platform_settings).put(put_platform_settings))
        .route("/permissions", get(list_permissions))
        // tenants (superadmin)
        .route("/tenants", get(list_tenants).post(create_tenant))
        .route("/tenants/{id}", get(get_tenant).put(update_tenant).delete(delete_tenant))
        // tenant IAM users
        .route("/tenant-user-counts", get(tenant_user_counts))
        .route("/tenant-users", get(list_tenant_users).post(create_tenant_user))
        .route("/tenant-users/{id}", post(update_tenant_user).delete(delete_tenant_user))
        // tenant domain self-service
        .route("/tenant-domain", get(get_tenant_domain).put(put_tenant_domain))
        // PBX config
        .route("/trunks", get(list_trunks).post(create_trunk))
        .route("/trunks/{id}", post(update_trunk).delete(delete_trunk))
        .route("/routes", get(list_routes).post(create_route))
        .route("/routes/{id}", post(update_route).delete(delete_route))
        .route("/extensions", get(list_extensions).post(create_extension))
        .route("/extensions/{id}", post(update_extension).delete(delete_extension))
        .route("/acl", get(list_acl).post(create_acl))
        .route("/acl/{id}", post(update_acl).delete(delete_acl))
        .route("/call-records", get(list_call_records))
        .route("/dids", get(list_dids).post(create_did))
        .route("/dids/{id}", post(update_did).delete(delete_did))
        .route("/workers", get(list_workers))
        .route("/workers/{id}/drain", post(drain_worker))
        .route("/workers/{id}", axum::routing::delete(remove_worker))
        .route("/edges", get(list_edges_admin))
        // Raft cluster admin (dynamic membership)
        .route("/raft/metrics", get(raft_metrics))
        .route("/raft/add-learner", post(raft_add_learner))
        .route("/raft/change-membership", post(raft_change_membership))
        .layer(middleware::from_fn_with_state(state.clone(), auth_guard))
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let index = format!("{web_dir}/index.html");
    let spa = ServeDir::new(web_dir).fallback(ServeFile::new(index));

    // Liveness/readiness probes — unauthenticated, outside /api (for k8s etc.).
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .nest("/api", api)
        .fallback_service(spa)
        .with_state(state)
}

/// Liveness: the process is up and serving.
async fn healthz() -> StatusCode {
    StatusCode::OK
}

/// Readiness: the process is up *and* its database is reachable.
async fn readyz(State(state): State<HttpState>) -> StatusCode {
    match state.db.ping().await {
        Ok(()) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

// ── Auth middleware ───────────────────────────────────────────────────────────

/// Validates the bearer token on every route EXCEPT `/api/auth/login`.
/// On success, injects the resolved `UserInfo` into request extensions.
async fn auth_guard(
    State(state): State<HttpState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if req.uri().path() == "/auth/login" {
        return Ok(next.run(req).await);
    }

    let token = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing bearer token"))?;

    let session = state
        .sessions
        .get(&token)
        .map(|s| s.clone())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "invalid token"))?;

    if session.expires < Utc::now() {
        state.sessions.remove(&token);
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "session expired"));
    }

    req.extensions_mut().insert(session.user.clone());
    Ok(next.run(req).await)
}

// ── Auth handlers ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginReq {
    username: String,
    password: String,
    /// Tenant domain (custom or `{id}.{base_domain}`). Omit for superadmin.
    #[serde(default)]
    domain: Option<String>,
}

#[derive(Serialize)]
struct LoginResp {
    token: String,
    user: UserInfo,
}

/// Client key for login throttling — the real client IP behind a proxy
/// (first X-Forwarded-For hop) or the direct peer address.
fn client_key(headers: &HeaderMap, peer: SocketAddr) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| peer.ip().to_string())
}

/// 429 if this client has exceeded the failed-login budget within the window.
fn check_login_gate(state: &HttpState, key: &str) -> ApiResult<()> {
    if let Some(g) = state.login_gate.get(key) {
        let fresh = (Utc::now() - g.window_start).num_seconds() < LOGIN_WINDOW_SECS;
        if fresh && g.fails >= LOGIN_MAX_FAILS {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too many failed login attempts; try again shortly",
            ));
        }
    }
    Ok(())
}

fn record_login_fail(state: &HttpState, key: &str) {
    let now = Utc::now();
    let mut g = state.login_gate.entry(key.to_string()).or_insert(LoginGate {
        fails: 0,
        window_start: now,
    });
    if (now - g.window_start).num_seconds() >= LOGIN_WINDOW_SECS {
        g.fails = 0;
        g.window_start = now;
    }
    g.fails += 1;
}

async fn login(
    State(state): State<HttpState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> ApiResult<Json<LoginResp>> {
    let key = client_key(&headers, peer);
    check_login_gate(&state, &key)?;

    let domain = req.domain.as_deref().map(str::trim).filter(|s| !s.is_empty());

    // Resolve the principal; any failure here is a throttled attempt.
    let auth: ApiResult<UserInfo> = async {
        match domain {
            None => {
                if req.username != state.admin_username || req.password != state.admin_password {
                    warn!(username = %req.username, "failed superadmin login");
                    return Err(ApiError::new(StatusCode::UNAUTHORIZED, "invalid credentials"));
                }
                Ok(UserInfo {
                    username: state.admin_username.clone(),
                    role: roles::SUPERADMIN.to_string(),
                    tenant_id: None,
                    permissions: vec![],
                })
            }
            Some(domain) => {
                let tenant = state
                    .tenants()
                    .await
                    .resolve_by_domain(domain)
                    .await
                    .map_err(ApiError::internal)?
                    .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unknown domain"))?;
                let principal = TenantUserService::new(&state.db)
                    .authenticate(tenant.id, req.username.trim(), &req.password)
                    .await
                    .map_err(ApiError::internal)?
                    .ok_or_else(|| {
                        warn!(domain, username = %req.username, "failed tenant login");
                        ApiError::new(StatusCode::UNAUTHORIZED, "invalid credentials")
                    })?;
                Ok(UserInfo {
                    username: principal.username,
                    role: principal.session_role,
                    tenant_id: Some(principal.tenant_id),
                    permissions: principal.permissions,
                })
            }
        }
    }
    .await;

    let user = match auth {
        Ok(u) => {
            state.login_gate.remove(&key); // success clears the counter
            u
        }
        Err(e) => {
            record_login_fail(&state, &key);
            return Err(e);
        }
    };

    let token = uuid::Uuid::new_v4().to_string();
    state.sessions.insert(
        token.clone(),
        Session {
            user: user.clone(),
            expires: Utc::now() + ChronoDuration::hours(SESSION_TTL_HOURS),
        },
    );
    info!(username = %user.username, role = %user.role, tenant_id = ?user.tenant_id, "login ok");
    Ok(Json(LoginResp { token, user }))
}

async fn logout(State(state): State<HttpState>, req: Request) -> StatusCode {
    if let Some(token) = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        state.sessions.remove(token);
    }
    StatusCode::NO_CONTENT
}

async fn me(Extension(user): Extension<UserInfo>) -> Json<UserInfo> {
    Json(user)
}

#[derive(Deserialize)]
struct ChangePasswordReq {
    current_password: String,
    new_password: String,
}

/// Self-service password change for the logged-in tenant account. The platform
/// super-admin password is config-managed, so it's rejected here.
async fn change_my_password(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Json(req): Json<ChangePasswordReq>,
) -> ApiResult<StatusCode> {
    let tid = user.tenant_id.ok_or_else(|| {
        ApiError::bad("the platform administrator password is managed via config")
    })?;
    TenantUserService::new(&state.db)
        .change_password(tid, &user.username, &req.current_password, &req.new_password)
        .await
        .map_err(ApiError::bad)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The permission catalogue, for the tenant-admin user editor.
async fn list_permissions(Extension(user): Extension<UserInfo>) -> ApiResult<Json<Vec<&'static str>>> {
    // Any authenticated principal that can manage users may read the catalogue.
    require_perm(&user, permissions::USERS_READ)?;
    Ok(Json(permissions::ALL_PERMISSIONS.to_vec()))
}

// ── Platform settings (superadmin) ─────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct PlatformSettingsBody {
    base_domain: String,
    #[serde(default)]
    stun_servers: Vec<String>,
}

async fn get_platform_settings(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
) -> ApiResult<Json<PlatformSettingsBody>> {
    require_superadmin(&user)?;
    let s = PlatformSettings::new(&state.db);
    Ok(Json(PlatformSettingsBody {
        base_domain: s.base_domain().await,
        stun_servers: s.stun_servers().await,
    }))
}

async fn put_platform_settings(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Json(body): Json<PlatformSettingsBody>,
) -> ApiResult<Json<PlatformSettingsBody>> {
    require_superadmin(&user)?;
    let s = PlatformSettings::new(&state.db);
    s.set(KEY_BASE_DOMAIN, body.base_domain.trim())
        .await
        .map_err(ApiError::internal)?;
    let stun: Vec<String> = body
        .stun_servers
        .iter()
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect();
    s.set_stun_servers(&stun).await.map_err(ApiError::internal)?;
    Ok(Json(PlatformSettingsBody {
        base_domain: s.base_domain().await,
        stun_servers: s.stun_servers().await,
    }))
}

// ── Stats ─────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Stats {
    tenants: usize,
    workers_total: usize,
    workers_healthy: usize,
    active_calls: u32,
}

async fn stats(State(state): State<HttpState>) -> ApiResult<Json<Stats>> {
    let tenants = state.tenants().await.list().await.map_err(ApiError::internal)?.len();
    let all = state.workers.all().await;
    let timeout_ms = state.workers.heartbeat_timeout().as_millis() as i64;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let workers_healthy = all.iter().filter(|w| w.is_healthy(now_ms, timeout_ms)).count();
    let active_calls = all.iter().map(|w| w.active_calls).sum();
    Ok(Json(Stats {
        tenants,
        workers_total: all.len(),
        workers_healthy,
        active_calls,
    }))
}

// ── Tenant-scoped dashboard stats ──────────────────────────────────────────────

#[derive(Serialize)]
struct TenantStats {
    trunks: usize,
    extensions: usize,
    dids: u64,
    recent_calls: usize,
}

async fn tenant_stats(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
) -> ApiResult<Json<TenantStats>> {
    let tid = required_tenant(&user, q.tenant_id)?;
    let trunks = state.store.list_trunks(Some(tid)).await.map_err(ApiError::internal)?.len();
    let extensions = state.store.list_extensions(Some(tid)).await.map_err(ApiError::internal)?.len();
    let dids = DidService::new(&state.db).count_for_tenant(tid).await.map_err(ApiError::internal)?;
    let recent_calls = state
        .store
        .list_call_records(Some(tid), 1000)
        .await
        .map_err(ApiError::internal)?
        .len();
    Ok(Json(TenantStats { trunks, extensions, dids, recent_calls }))
}

// ── Tenant handlers (superadmin) ───────────────────────────────────────────────

async fn list_tenants(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let tenants = state.tenants().await.list().await.map_err(ApiError::internal)?;
    Ok(Json(tenants).into_response())
}

async fn get_tenant(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    // Superadmin (any) or a tenant principal reading its own tenant.
    if !is_superadmin(&user) && user.tenant_id != Some(id) {
        return Err(ApiError::forbidden());
    }
    let tenant = state.tenants().await.get(id).await.map_err(ApiError::not_found)?;
    Ok(Json(tenant).into_response())
}

async fn create_tenant(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Json(req): Json<CreateTenantRequest>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;

    // Validate the optional initial-admin credentials BEFORE creating the
    // tenant, so an invalid password doesn't leave behind an orphan tenant.
    let admin_creds = match (req.admin_username.as_deref().map(str::trim), req.admin_password.as_deref()) {
        (Some(u), p) if !u.is_empty() => {
            let p = p.unwrap_or("");
            if p.len() < 6 {
                return Err(ApiError::bad("admin password must be at least 6 characters"));
            }
            Some((u.to_string(), p.to_string()))
        }
        _ => None,
    };

    let tenant = state.tenants().await.create(&req).await.map_err(ApiError::bad)?;

    // Provision the tenant's first admin account (creds already validated).
    let mut provisioned_admin = None;
    if let Some((u, p)) = admin_creds {
        match TenantUserService::new(&state.db)
            .create_initial_admin(tenant.id, &u, &p)
            .await
        {
            Ok(admin) => provisioned_admin = Some(admin),
            Err(e) => {
                return Err(ApiError::bad(format!(
                    "tenant created (id {}) but admin account failed: {e}",
                    tenant.id
                )));
            }
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "tenant": tenant, "admin": provisioned_admin })),
    )
        .into_response())
}

async fn update_tenant(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateTenantRequest>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let tenant = state.tenants().await.update(id, req).await.map_err(ApiError::bad)?;
    Ok(Json(tenant).into_response())
}

async fn delete_tenant(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_superadmin(&user)?;
    state.tenants().await.delete(id).await.map_err(ApiError::not_found)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Tenant IAM users ───────────────────────────────────────────────────────────

/// Per-tenant account counts (superadmin) — surfaces tenants with no users.
async fn tenant_user_counts(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
) -> ApiResult<Json<std::collections::HashMap<i64, i64>>> {
    require_superadmin(&user)?;
    let counts = TenantUserService::new(&state.db)
        .count_by_tenant()
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(counts))
}

async fn list_tenant_users(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::USERS_READ)?;
    let tid = required_tenant(&user, q.tenant_id)?;
    let users = TenantUserService::new(&state.db).list(tid).await.map_err(ApiError::internal)?;
    Ok(Json(users).into_response())
}

async fn create_tenant_user(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
    Json(req): Json<CreateTenantUserRequest>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::USERS_WRITE)?;
    let tid = required_tenant(&user, q.tenant_id)?;
    // Only superadmin may mint another tenant admin; tenant admins create users.
    if req.role.as_deref() == Some(permissions::db_role::ADMIN) && !is_superadmin(&user) {
        return Err(ApiError::forbidden());
    }
    let created = TenantUserService::new(&state.db)
        .create(tid, &req)
        .await
        .map_err(ApiError::bad)?;
    Ok((StatusCode::CREATED, Json(created)).into_response())
}

async fn update_tenant_user(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateTenantUserRequest>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::USERS_WRITE)?;
    let svc = TenantUserService::new(&state.db);
    let target = svc.get(id).await.map_err(ApiError::not_found)?;
    if !is_superadmin(&user) && user.tenant_id != Some(target.tenant_id) {
        return Err(ApiError::forbidden());
    }
    let updated = svc.update(id, req).await.map_err(ApiError::bad)?;
    Ok(Json(updated).into_response())
}

async fn delete_tenant_user(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::USERS_WRITE)?;
    let svc = TenantUserService::new(&state.db);
    let target = svc.get(id).await.map_err(ApiError::not_found)?;
    if !is_superadmin(&user) && user.tenant_id != Some(target.tenant_id) {
        return Err(ApiError::forbidden());
    }
    svc.delete(id).await.map_err(ApiError::bad)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Tenant domain self-service ─────────────────────────────────────────────────

async fn get_tenant_domain(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::DOMAIN_READ)?;
    let tid = required_tenant(&user, q.tenant_id)?;
    let tenant = state.tenants().await.get(tid).await.map_err(ApiError::not_found)?;
    Ok(Json(tenant).into_response())
}

async fn put_tenant_domain(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
    Json(req): Json<UpdateDomainRequest>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::DOMAIN_WRITE)?;
    let tid = required_tenant(&user, q.tenant_id)?;
    let tenant = state.tenants().await.update_domain(tid, req).await.map_err(ApiError::bad)?;
    Ok(Json(tenant).into_response())
}

// ── PBX config: trunks ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TenantQuery {
    tenant_id: Option<i64>,
}

async fn list_trunks(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::TRUNKS_READ)?;
    let scope = read_scope(&user, q.tenant_id)?;
    let rows = state.store.list_trunks(scope).await.map_err(ApiError::internal)?;
    Ok(Json(rows).into_response())
}

async fn create_trunk(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
    Json(input): Json<TrunkInput>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::TRUNKS_WRITE)?;
    let row_tenant = create_tenant_scope(&user, q.tenant_id)?;
    if let Some(tid) = row_tenant {
        enforce_trunk_quota(&state, tid).await?;
    }
    state.store.create_trunk(&input, row_tenant).await.map_err(ApiError::bad)?;
    Ok(StatusCode::CREATED)
}

async fn update_trunk(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
    Json(input): Json<TrunkInput>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::TRUNKS_WRITE)?;
    let n = state.store.update_trunk(id, &input, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    affected_or_404(n)
}

async fn delete_trunk(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::TRUNKS_WRITE)?;
    let n = state.store.delete_trunk(id, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    affected_or_404(n)
}

// ── PBX config: routes ─────────────────────────────────────────────────────────

async fn list_routes(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::ROUTING_READ)?;
    let scope = read_scope(&user, q.tenant_id)?;
    let rows = state.store.list_routes(scope).await.map_err(ApiError::internal)?;
    Ok(Json(rows).into_response())
}

async fn create_route(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
    Json(input): Json<RouteInput>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::ROUTING_WRITE)?;
    let row_tenant = create_tenant_scope(&user, q.tenant_id)?;
    state.store.create_route(&input, row_tenant).await.map_err(ApiError::bad)?;
    Ok(StatusCode::CREATED)
}

async fn update_route(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
    Json(input): Json<RouteInput>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::ROUTING_WRITE)?;
    let n = state.store.update_route(id, &input, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    affected_or_404(n)
}

async fn delete_route(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::ROUTING_WRITE)?;
    let n = state.store.delete_route(id, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    affected_or_404(n)
}

// ── PBX config: extensions ─────────────────────────────────────────────────────

async fn list_extensions(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::EXTENSIONS_READ)?;
    let scope = read_scope(&user, q.tenant_id)?;
    let rows = state.store.list_extensions(scope).await.map_err(ApiError::internal)?;
    Ok(Json(rows).into_response())
}

async fn create_extension(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
    Json(input): Json<ExtensionInput>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::EXTENSIONS_WRITE)?;
    let row_tenant = create_tenant_scope(&user, q.tenant_id)?;
    state.store.create_extension(&input, row_tenant).await.map_err(ApiError::bad)?;
    Ok(StatusCode::CREATED)
}

async fn update_extension(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
    Json(input): Json<ExtensionInput>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::EXTENSIONS_WRITE)?;
    let n = state
        .store
        .update_extension(id, &input, mutate_scope(&user))
        .await
        .map_err(ApiError::bad)?;
    affected_or_404(n)
}

async fn delete_extension(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::EXTENSIONS_WRITE)?;
    let n = state.store.delete_extension(id, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    affected_or_404(n)
}

// ── PBX config: ACL rules ──────────────────────────────────────────────────────

async fn list_acl(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::ACL_READ)?;
    let scope = read_scope(&user, q.tenant_id)?;
    let rows = state.store.list_acl_admin(scope).await.map_err(ApiError::internal)?;
    Ok(Json(rows).into_response())
}

async fn create_acl(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
    Json(input): Json<AclInput>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::ACL_WRITE)?;
    let row_tenant = create_tenant_scope(&user, q.tenant_id)?;
    state.store.create_acl(&input, row_tenant).await.map_err(ApiError::bad)?;
    Ok(StatusCode::CREATED)
}

async fn update_acl(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
    Json(input): Json<AclInput>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::ACL_WRITE)?;
    let n = state.store.update_acl(id, &input, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    affected_or_404(n)
}

async fn delete_acl(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::ACL_WRITE)?;
    let n = state.store.delete_acl(id, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    affected_or_404(n)
}

// ── Call records (CDR) ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CdrQuery {
    tenant_id: Option<i64>,
    limit: Option<u64>,
}

async fn list_call_records(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<CdrQuery>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::CDR_READ)?;
    let scope = read_scope(&user, q.tenant_id)?;
    let limit = q.limit.unwrap_or(200).min(1000);
    let rows = state
        .store
        .list_call_records(scope, limit)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(rows).into_response())
}

// ── DIDs (phone number inventory) ──────────────────────────────────────────────

async fn list_dids(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::DIDS_READ)?;
    let scope = read_scope(&user, q.tenant_id)?;
    let dids = DidService::new(&state.db).list(scope).await.map_err(ApiError::internal)?;
    Ok(Json(dids).into_response())
}

/// DID inventory mutations are platform operations (superadmin allocates numbers
/// and assigns them to tenants).
async fn create_did(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Json(req): Json<CreateDidRequest>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    if let Some(tid) = req.tenant_id {
        enforce_did_quota(&state, tid).await?;
    }
    let did = DidService::new(&state.db).create(&req).await.map_err(ApiError::bad)?;
    Ok((StatusCode::CREATED, Json(did)).into_response())
}

async fn update_did(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateDidRequest>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let did = DidService::new(&state.db).update(id, req).await.map_err(ApiError::bad)?;
    Ok(Json(did).into_response())
}

async fn delete_did(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_superadmin(&user)?;
    DidService::new(&state.db).delete(id).await.map_err(ApiError::not_found)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Workers ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct WorkerView {
    worker_id: String,
    sip_addr: String,
    rtp_external_ip: String,
    active_calls: u32,
    max_concurrent: u32,
    available_capacity: u32,
    cpu_usage: f32,
    nat_type: String,
    registered_at: String,
    last_heartbeat_secs_ago: u64,
    healthy: bool,
    draining: bool,
}

async fn list_workers(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
) -> ApiResult<Json<Vec<WorkerView>>> {
    require_superadmin(&user)?;
    let timeout_ms = state.workers.heartbeat_timeout().as_millis() as i64;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let views = state
        .workers
        .all()
        .await
        .into_iter()
        .map(|w| WorkerView {
            healthy: w.is_healthy(now_ms, timeout_ms),
            available_capacity: w.available_capacity(),
            last_heartbeat_secs_ago: now_ms.saturating_sub(w.last_heartbeat_ms).max(0) as u64 / 1000,
            registered_at: chrono::DateTime::from_timestamp_millis(w.registered_at_ms)
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
            worker_id: w.worker_id,
            sip_addr: w.sip_addr,
            rtp_external_ip: w.rtp_external_ip,
            active_calls: w.active_calls,
            max_concurrent: w.max_concurrent,
            cpu_usage: w.cpu_usage,
            nat_type: w.nat_type,
            draining: w.draining,
        })
        .collect();
    Ok(Json(views))
}

/// Gracefully drain a worker: it stops being selected for new calls (excluded
/// from `available()`) while existing calls finish. Superadmin only.
async fn drain_worker(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_superadmin(&user)?;
    state.workers.drain(&id).await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Force-remove a worker entry from the registry (for stuck/dead nodes).
async fn remove_worker(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_superadmin(&user)?;
    state.workers.remove(&id).await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Edges ─────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct EdgeView {
    edge_id: String,
    public_ip: String,
    sip_addr: String,
    transports: Vec<String>,
    region: String,
    version: String,
    active_calls: u32,
    nat_type: String,
    registered_at: String,
    last_heartbeat_secs_ago: u64,
    healthy: bool,
}

async fn list_edges_admin(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
) -> ApiResult<Json<Vec<EdgeView>>> {
    require_superadmin(&user)?;
    let timeout_ms = state.workers.heartbeat_timeout().as_millis() as i64;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let views = state
        .workers
        .all_edges()
        .await
        .into_iter()
        .map(|e| EdgeView {
            healthy: e.is_healthy(now_ms, timeout_ms),
            last_heartbeat_secs_ago: now_ms.saturating_sub(e.last_heartbeat_ms).max(0) as u64 / 1000,
            registered_at: chrono::DateTime::from_timestamp_millis(e.registered_at_ms)
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
            edge_id: e.edge_id,
            public_ip: e.public_ip,
            sip_addr: e.sip_addr,
            transports: e.transports,
            region: e.region,
            version: e.version,
            active_calls: e.active_calls,
            nat_type: e.nat_type,
        })
        .collect();
    Ok(Json(views))
}

// ── Raft cluster admin ─────────────────────────────────────────────────────

async fn raft_metrics(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
) -> ApiResult<Json<crate::raft::registry::RaftMetricsSummary>> {
    require_superadmin(&user)?;
    Ok(Json(state.workers.metrics_summary()))
}

#[derive(serde::Deserialize)]
struct AddLearnerRequest {
    node_id: u64,
    addr: String,
    #[serde(default)]
    grpc_addr: String,
}

async fn raft_add_learner(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Json(req): Json<AddLearnerRequest>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let grpc_addr = if req.grpc_addr.trim().is_empty() {
        req.addr.as_str()
    } else {
        req.grpc_addr.as_str()
    };
    state
        .workers
        .add_learner(req.node_id, &req.addr, grpc_addr)
        .await
        .map_err(ApiError::bad)?;
    Ok((StatusCode::OK, Json(serde_json::json!({"added": req.node_id}))).into_response())
}

#[derive(serde::Deserialize)]
struct ChangeMembershipRequest {
    voters: std::collections::BTreeSet<u64>,
}

async fn raft_change_membership(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Json(req): Json<ChangeMembershipRequest>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    state.workers.change_membership(req.voters).await.map_err(ApiError::bad)?;
    Ok((StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response())
}
