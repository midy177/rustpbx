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

use crate::audit_service::{AuditEntry, AuditFilter, AuditService};
use crate::auth::permissions::{self, roles};
use crate::did_service::{CreateDidRequest, DidService, UpdateDidRequest};
use crate::raft::registry::RaftRegistry;
use crate::settings::{KEY_BASE_DOMAIN, PlatformSettings};
use crate::store::Store;
use crate::store::crud::{AclInput, ExtensionInput, QueueInput, RouteInput, TrunkInput};
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
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

/// The built Vue admin SPA, embedded into the binary at compile time from
/// `web/dist` (produced by build.rs). Makes the control binary self-contained —
/// no external web directory to ship or path to configure.
#[derive(rust_embed::RustEmbed)]
#[folder = "web/dist"]
struct WebAssets;

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

    /// Best-effort, fire-and-forget audit record. Fills the actor from the
    /// session, then writes on a detached task so it never blocks the response
    /// and never fails the request — a flaky audit insert must not roll back a
    /// successful mutation.
    pub fn audit(&self, user: &UserInfo, mut entry: AuditEntry) {
        entry.actor_username = user.username.clone();
        entry.actor_role = user.role.clone();
        entry.actor_tenant_id = user.tenant_id;
        let db = self.db.clone();
        tokio::spawn(async move {
            if let Err(e) = AuditService::new(&db).record(entry).await {
                warn!(error = %e, "audit record failed");
            }
        });
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

/// Serve an embedded SPA asset by path, falling back to `index.html` for
/// client-side routes (anything that isn't a real file). Returns 404 only if
/// the SPA itself wasn't embedded.
async fn spa_fallback(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty() {
        if let Some(asset) = WebAssets::get(path) {
            let mime = asset.metadata.mimetype().to_string();
            return (
                [(axum::http::header::CONTENT_TYPE, mime)],
                asset.data.into_owned(),
            )
                .into_response();
        }
    }
    match WebAssets::get("index.html") {
        Some(index) => (
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            index.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "web assets not embedded").into_response(),
    }
}

/// Build the full HTTP router: embedded SPA + JSON API.
pub fn build_router(state: HttpState) -> Router {
    let api = Router::new()
        // public
        .route("/auth/login", post(login))
        // protected
        .route("/auth/logout", post(logout))
        .route("/me", get(me))
        .route("/me/password", post(change_my_password))
        .route("/stats", get(stats))
        .route("/tenant-stats", get(tenant_stats))
        .route("/tenant-quotas", get(list_tenant_quotas))
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
        .route("/queues", get(list_queues).post(create_queue))
        .route("/queues/{id}", post(update_queue).delete(delete_queue))
        .route("/call-records", get(list_call_records))
        .route("/dids", get(list_dids).post(create_did))
        .route("/dids/{id}", post(update_did).delete(delete_did))
        .route("/audit", get(list_audit))
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

    // Liveness/readiness probes — unauthenticated, outside /api (for k8s etc.).
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .nest("/api", api)
        .fallback(spa_fallback)
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
    /// Authoritative reserved per-tenant call slots across the platform
    /// (from the Raft state machine), vs `active_calls` which is the sum of
    /// worker-reported in-flight counts.
    call_slots: u32,
}

async fn stats(State(state): State<HttpState>) -> ApiResult<Json<Stats>> {
    let tenants = state.tenants().await.list().await.map_err(ApiError::internal)?.len();
    let all = state.workers.all().await;
    let timeout_ms = state.workers.heartbeat_timeout().as_millis() as i64;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let workers_healthy = all.iter().filter(|w| w.is_healthy(now_ms, timeout_ms)).count();
    let active_calls = all.iter().map(|w| w.active_calls).sum();
    let call_slots = state.workers.total_call_slots().await;
    Ok(Json(Stats {
        tenants,
        workers_total: all.len(),
        workers_healthy,
        active_calls,
        call_slots,
    }))
}

// ── Tenant-scoped dashboard stats ──────────────────────────────────────────────

#[derive(Serialize)]
struct TenantStats {
    trunks: usize,
    extensions: usize,
    dids: u64,
    recent_calls: usize,
    /// This tenant's currently-reserved call slots (live concurrency).
    active_calls: u32,
    /// The tenant's configured max_concurrent_calls (None = unlimited).
    max_concurrent_calls: Option<u32>,
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
    // Accurate total CDR count for the tenant (no 1000-row cap).
    let opts = crate::store::CdrListOpts { tenant_id: Some(tid), limit: 1, ..Default::default() };
    let recent_calls = state
        .store
        .list_call_records_paged(&opts)
        .await
        .map_err(ApiError::internal)?
        .1 as usize;
    let active_calls = state.workers.tenant_active_calls(tid).await;
    let max_concurrent_calls = state
        .store
        .tenant_max_concurrent(tid)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(TenantStats { trunks, extensions, dids, recent_calls, active_calls, max_concurrent_calls }))
}

// ── Platform-wide quota usage (superadmin) ────────────────────────────────────

/// One resource's usage vs its configured cap. `max == None` → unlimited.
#[derive(Serialize)]
struct QuotaUsage {
    used: u64,
    max: Option<u32>,
}

#[derive(Serialize)]
struct TenantQuota {
    id: i64,
    name: String,
    status: String,
    trunks: QuotaUsage,
    dids: QuotaUsage,
    /// Live reserved call slots vs the tenant's concurrency cap.
    concurrent: QuotaUsage,
}

/// Per-tenant quota usage across the whole platform — the superadmin's
/// "who's near their limits" view. N tenants × 3 counts; fine for the modest
/// tenant counts a control plane manages (aggregate to GROUP BY if it ever
/// becomes hot).
async fn list_tenant_quotas(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
) -> ApiResult<Json<Vec<TenantQuota>>> {
    require_superadmin(&user)?;
    let tenants = state.tenants().await.list().await.map_err(ApiError::internal)?;
    let did_svc = DidService::new(&state.db);
    let mut out = Vec::with_capacity(tenants.len());
    for t in tenants {
        let used_trunks = state
            .store
            .count_trunks_for_tenant(t.id)
            .await
            .map_err(ApiError::internal)?;
        let used_dids = did_svc.count_for_tenant(t.id).await.map_err(ApiError::internal)?;
        let active = state.workers.tenant_active_calls(t.id).await as u64;
        out.push(TenantQuota {
            id: t.id,
            name: t.name,
            status: t.status,
            trunks: QuotaUsage { used: used_trunks, max: t.max_trunks.map(|v| v as u32) },
            dids: QuotaUsage { used: used_dids, max: t.max_dids.map(|v| v as u32) },
            concurrent: QuotaUsage { used: active, max: t.max_concurrent_calls.map(|v| v as u32) },
        });
    }
    // Most-loaded first (by how close each resource is to its cap).
    out.sort_by_key(|q| std::cmp::Reverse(q.saturation()));
    Ok(Json(out))
}

impl TenantQuota {
    /// 0–100+: highest saturation across the three resources (capped resources
    /// only). Used to order the dashboard.
    fn saturation(&self) -> u32 {
        [(&self.trunks, "trunks"), (&self.dids, "dids"), (&self.concurrent, "conc")]
            .into_iter()
            .filter_map(|(u, _)| u.max.map(|m| if m == 0 { 0 } else { (u.used * 100 / m as u64) as u32 }))
            .max()
            .unwrap_or(0)
    }
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
    audit_tenant(&state, &user, "create", tenant.id, &tenant.name);

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

/// Audit helper for tenant mutations (superadmin actor).
fn audit_tenant(state: &HttpState, user: &UserInfo, action: &str, id: i64, name: &str) {
    let summary = if name.is_empty() {
        format!("{action} tenant (id {id})")
    } else {
        format!("{action} tenant '{name}' (id {id})")
    };
    state.audit(user, AuditEntry::action(action, "tenant", Some(id), summary));
}

async fn update_tenant(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateTenantRequest>,
) -> ApiResult<Response> {
    require_superadmin(&user)?;
    let tenant = state.tenants().await.update(id, req).await.map_err(ApiError::bad)?;
    audit_tenant(&state, &user, "update", tenant.id, &tenant.name);
    Ok(Json(tenant).into_response())
}

async fn delete_tenant(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_superadmin(&user)?;
    state.tenants().await.delete(id).await.map_err(ApiError::not_found)?;
    audit_tenant(&state, &user, "delete", id, "");
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
    state.audit(
        &user,
        AuditEntry::action("create", "tenant_user", Some(created.id), format!("created user '{}'", created.username)),
    );
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
    state.audit(
        &user,
        AuditEntry::action("update", "tenant_user", Some(id), format!("updated user '{}' (id {id})", updated.username)),
    );
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
    state.audit(
        &user,
        AuditEntry::action("delete", "tenant_user", Some(id), format!("deleted user '{}' (id {id})", target.username)),
    );
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
    state.audit(
        &user,
        AuditEntry::action("update", "domain", Some(tid), format!("updated domain for tenant '{name}'", name = tenant.name)),
    );
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
    state.audit(&user, AuditEntry::action("create", "trunk", None, format!("created trunk '{}'", input.name)));
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
    if n > 0 {
        state.audit(&user, AuditEntry::action("update", "trunk", Some(id), format!("updated trunk '{}' (id {id})", input.name)));
    }
    affected_or_404(n)
}

async fn delete_trunk(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::TRUNKS_WRITE)?;
    let n = state.store.delete_trunk(id, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    if n > 0 {
        state.audit(&user, AuditEntry::action("delete", "trunk", Some(id), format!("deleted trunk (id {id})")));
    }
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
    state.audit(&user, AuditEntry::action("create", "route", None, format!("created route '{}'", input.name)));
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
    if n > 0 {
        state.audit(&user, AuditEntry::action("update", "route", Some(id), format!("updated route '{}' (id {id})", input.name)));
    }
    affected_or_404(n)
}

async fn delete_route(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::ROUTING_WRITE)?;
    let n = state.store.delete_route(id, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    if n > 0 {
        state.audit(&user, AuditEntry::action("delete", "route", Some(id), format!("deleted route (id {id})")));
    }
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
    state.audit(&user, AuditEntry::action("create", "extension", None, format!("created extension '{}'", input.extension)));
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
    if n > 0 {
        state.audit(&user, AuditEntry::action("update", "extension", Some(id), format!("updated extension '{}' (id {id})", input.extension)));
    }
    affected_or_404(n)
}

async fn delete_extension(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::EXTENSIONS_WRITE)?;
    let n = state.store.delete_extension(id, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    if n > 0 {
        state.audit(&user, AuditEntry::action("delete", "extension", Some(id), format!("deleted extension (id {id})")));
    }
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
    state.audit(&user, AuditEntry::action("create", "acl", None, format!("created ACL rule: {} {}", input.action, input.target)));
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
    if n > 0 {
        state.audit(&user, AuditEntry::action("update", "acl", Some(id), format!("updated ACL rule (id {id}): {} {}", input.action, input.target)));
    }
    affected_or_404(n)
}

async fn delete_acl(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::ACL_WRITE)?;
    let n = state.store.delete_acl(id, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    if n > 0 {
        state.audit(&user, AuditEntry::action("delete", "acl", Some(id), format!("deleted ACL rule (id {id})")));
    }
    affected_or_404(n)
}

// ── PBX config: call queues ───────────────────────────────────────────────────

async fn list_queues(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::QUEUE_READ)?;
    let scope = read_scope(&user, q.tenant_id)?;
    let rows = state.store.list_queues_admin(scope).await.map_err(ApiError::internal)?;
    Ok(Json(rows).into_response())
}

async fn create_queue(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<TenantQuery>,
    Json(input): Json<QueueInput>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::QUEUE_WRITE)?;
    let row_tenant = create_tenant_scope(&user, q.tenant_id)?;
    state.store.create_queue(&input, row_tenant).await.map_err(ApiError::bad)?;
    state.audit(&user, AuditEntry::action("create", "queue", None, format!("created queue '{}'", input.name)));
    Ok(StatusCode::CREATED)
}

async fn update_queue(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
    Json(input): Json<QueueInput>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::QUEUE_WRITE)?;
    let n = state.store.update_queue(id, &input, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    if n > 0 {
        state.audit(&user, AuditEntry::action("update", "queue", Some(id), format!("updated queue '{}' (id {id})", input.name)));
    }
    affected_or_404(n)
}

async fn delete_queue(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_perm(&user, permissions::QUEUE_WRITE)?;
    let n = state.store.delete_queue(id, mutate_scope(&user)).await.map_err(ApiError::bad)?;
    if n > 0 {
        state.audit(&user, AuditEntry::action("delete", "queue", Some(id), format!("deleted queue (id {id})")));
    }
    affected_or_404(n)
}

// ── Call records (CDR) ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CdrQuery {
    tenant_id: Option<i64>,
    limit: Option<u64>,
    offset: Option<u64>,
    /// Substring match on caller or callee number.
    search: Option<String>,
    status: Option<String>,
    direction: Option<String>,
    /// RFC3339 timestamps (`started_at` >= since / <= until).
    since: Option<String>,
    until: Option<String>,
}

#[derive(serde::Serialize)]
struct CdrPage {
    records: Vec<crate::store::CdrView>,
    total: u64,
    limit: u64,
    offset: u64,
}

fn parse_rfc3339(s: &Option<String>) -> Option<chrono::DateTime<chrono::Utc>> {
    s.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

async fn list_call_records(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<CdrQuery>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::CDR_READ)?;
    let scope = read_scope(&user, q.tenant_id)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let offset = q.offset.unwrap_or(0);
    let opts = crate::store::CdrListOpts {
        tenant_id: scope,
        search: q.search.clone(),
        status: q.status.clone(),
        direction: q.direction.clone(),
        since: parse_rfc3339(&q.since),
        until: parse_rfc3339(&q.until),
        limit,
        offset,
    };
    let (records, total) = state
        .store
        .list_call_records_paged(&opts)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(CdrPage { records, total, limit, offset }).into_response())
}

// ── Audit trail ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AuditQuery {
    tenant_id: Option<i64>,
    limit: Option<u64>,
    action: Option<String>,
    target_type: Option<String>,
}

/// List audit entries. Tenant admins see only their own tenant's entries; the
/// superadmin sees all. A plain tenant user (no admin role) is denied.
async fn list_audit(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Response> {
    require_perm(&user, permissions::AUDIT_READ)?;
    // Tenant admins are locked to their own tenant; only superadmin may cross
    // tenants (and only then is tenant_id=None meaningful → all entries).
    let scope = if is_superadmin(&user) { q.tenant_id } else { user.tenant_id };
    let filter = AuditFilter {
        tenant_id: scope,
        action: q.action,
        target_type: q.target_type,
        limit: q.limit.unwrap_or(100),
    };
    let rows = AuditService::new(&state.db)
        .list(&filter)
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
    state.audit(&user, AuditEntry::action("create", "did", Some(did.id), format!("created DID '{}'", did.number)));
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
    state.audit(&user, AuditEntry::action("update", "did", Some(id), format!("updated DID '{}' (id {id})", did.number)));
    Ok(Json(did).into_response())
}

async fn delete_did(
    State(state): State<HttpState>,
    Extension(user): Extension<UserInfo>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    require_superadmin(&user)?;
    DidService::new(&state.db).delete(id).await.map_err(ApiError::not_found)?;
    state.audit(&user, AuditEntry::action("delete", "did", Some(id), format!("deleted DID (id {id})")));
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
