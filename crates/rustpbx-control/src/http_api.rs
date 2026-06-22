//! HTTP/REST API for the Control Plane admin console.
//!
//! Serves the Vue3 single-page app (`web/dist`) plus a JSON API consumed by it:
//! authentication, tenant CRUD, worker status and dashboard stats.
//!
//! Authentication is intentionally lightweight: a single super-admin account
//! (from config) logs in and receives an opaque bearer token tracked in an
//! in-memory session map. Tenant-scoped accounts are a future enhancement —
//! for now the super-admin can scope views to a tenant from the UI.

use crate::raft::registry::RaftRegistry;
use crate::store::Store;
use crate::tenant_service::{CreateTenantRequest, TenantService, UpdateTenantRequest};
use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
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

// ── Shared state ────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct HttpState {
    pub db: DatabaseConnection,
    pub store: Arc<Store>,
    pub workers: RaftRegistry,
    pub sessions: Arc<DashMap<String, Session>>,
    pub admin_username: String,
    pub admin_password: String,
}

#[derive(Clone, Serialize)]
pub struct UserInfo {
    pub username: String,
    pub role: String,
    pub tenant_id: Option<i64>,
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
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ── Router ──────────────────────────────────────────────────────────────────

/// Build the full HTTP router: SPA static files + JSON API.
pub fn build_router(state: HttpState, web_dir: &str) -> Router {
    let api = Router::new()
        // public
        .route("/auth/login", post(login))
        // protected
        .route("/auth/logout", post(logout))
        .route("/me", get(me))
        .route("/stats", get(stats))
        .route("/tenants", get(list_tenants).post(create_tenant))
        .route(
            "/tenants/{id}",
            get(get_tenant).put(update_tenant).delete(delete_tenant),
        )
        .route("/workers", get(list_workers))
        .route("/trunks", get(list_trunks))
        .route("/routes", get(list_routes))
        // Raft cluster admin (dynamic membership)
        .route("/raft/metrics", get(raft_metrics))
        .route("/raft/add-learner", post(raft_add_learner))
        .route("/raft/change-membership", post(raft_change_membership))
        .layer(middleware::from_fn_with_state(state.clone(), auth_guard))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let index = format!("{web_dir}/index.html");
    let spa = ServeDir::new(web_dir).fallback(ServeFile::new(index));

    Router::new().nest("/api", api).fallback_service(spa)
}

// ── Auth middleware ───────────────────────────────────────────────────────────

/// Validates the bearer token on every route EXCEPT `/api/auth/login`.
/// On success, injects the resolved `UserInfo` into request extensions.
async fn auth_guard(
    State(state): State<HttpState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // Login is the only unauthenticated endpoint. The middleware runs inside the
    // `/api`-nested router, so the path here is prefix-stripped (`/auth/login`).
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
}

#[derive(Serialize)]
struct LoginResp {
    token: String,
    user: UserInfo,
}

async fn login(State(state): State<HttpState>, Json(req): Json<LoginReq>) -> ApiResult<Json<LoginResp>> {
    if req.username != state.admin_username || req.password != state.admin_password {
        warn!(username = %req.username, "failed login attempt");
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "invalid credentials"));
    }

    let user = UserInfo {
        username: state.admin_username.clone(),
        role: "superadmin".to_string(),
        tenant_id: None,
    };
    let token = uuid::Uuid::new_v4().to_string();
    state.sessions.insert(
        token.clone(),
        Session {
            user: user.clone(),
            expires: Utc::now() + ChronoDuration::hours(SESSION_TTL_HOURS),
        },
    );
    info!(username = %user.username, "login ok");
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

async fn me(req: Request) -> ApiResult<Json<UserInfo>> {
    req.extensions()
        .get::<UserInfo>()
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "no session"))
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
    let tenants = TenantService::new(&state.db)
        .list()
        .await
        .map_err(ApiError::internal)?
        .len();
    let all = state.workers.all().await;
    let timeout_ms = state.workers.heartbeat_timeout().as_millis() as i64;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let workers_healthy = all
        .iter()
        .filter(|w| w.is_healthy(now_ms, timeout_ms))
        .count();
    let active_calls = all.iter().map(|w| w.active_calls).sum();
    Ok(Json(Stats {
        tenants,
        workers_total: all.len(),
        workers_healthy,
        active_calls,
    }))
}

// ── Tenant handlers ─────────────────────────────────────────────────────────

async fn list_tenants(State(state): State<HttpState>) -> ApiResult<Response> {
    let tenants = TenantService::new(&state.db)
        .list()
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(tenants).into_response())
}

async fn get_tenant(State(state): State<HttpState>, Path(id): Path<i64>) -> ApiResult<Response> {
    let tenant = TenantService::new(&state.db)
        .get(id)
        .await
        .map_err(|e| ApiError::new(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(tenant).into_response())
}

async fn create_tenant(
    State(state): State<HttpState>,
    Json(req): Json<CreateTenantRequest>,
) -> ApiResult<Response> {
    let tenant = TenantService::new(&state.db)
        .create(req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(tenant)).into_response())
}

async fn update_tenant(
    State(state): State<HttpState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateTenantRequest>,
) -> ApiResult<Response> {
    let tenant = TenantService::new(&state.db)
        .update(id, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(tenant).into_response())
}

async fn delete_tenant(State(state): State<HttpState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    TenantService::new(&state.db)
        .delete(id)
        .await
        .map_err(|e| ApiError::new(StatusCode::NOT_FOUND, e.to_string()))?;
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
    registered_at: String,
    last_heartbeat_secs_ago: u64,
    healthy: bool,
    draining: bool,
}

// ── Trunks & Routes (read-only, secret-safe) ─────────────────────────────────

#[derive(Deserialize)]
struct TenantQuery {
    tenant_id: Option<i64>,
}

async fn list_trunks(
    State(state): State<HttpState>,
    Query(q): Query<TenantQuery>,
) -> ApiResult<Response> {
    let rows = state
        .store
        .list_trunks(q.tenant_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(rows).into_response())
}

async fn list_routes(
    State(state): State<HttpState>,
    Query(q): Query<TenantQuery>,
) -> ApiResult<Response> {
    let rows = state
        .store
        .list_routes(q.tenant_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(rows).into_response())
}

async fn list_workers(State(state): State<HttpState>) -> Json<Vec<WorkerView>> {
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
            draining: w.draining,
        })
        .collect();
    Json(views)
}

// ── Raft cluster admin ─────────────────────────────────────────────────────

/// Current Raft state (term, leader, members, applied index).
async fn raft_metrics(State(state): State<HttpState>) -> Json<crate::raft::registry::RaftMetricsSummary> {
    Json(state.workers.metrics_summary())
}

#[derive(serde::Deserialize)]
struct AddLearnerRequest {
    node_id: u64,
    /// Address peers use to reach the new node's Raft transport server (host:port).
    addr: String,
    /// The new node's business gRPC (`ControlPlane`) address, used for
    /// write-forwarding. Defaults to `addr` if omitted.
    #[serde(default)]
    grpc_addr: String,
}

/// Add a node as a non-voting learner. Must be called on the current leader.
async fn raft_add_learner(
    State(state): State<HttpState>,
    Json(req): Json<AddLearnerRequest>,
) -> ApiResult<Response> {
    let grpc_addr = if req.grpc_addr.trim().is_empty() {
        req.addr.as_str()
    } else {
        req.grpc_addr.as_str()
    };
    state
        .workers
        .add_learner(req.node_id, &req.addr, grpc_addr)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok((StatusCode::OK, Json(serde_json::json!({"added": req.node_id}))).into_response())
}

#[derive(serde::Deserialize)]
struct ChangeMembershipRequest {
    /// The complete set of voter node ids after the change.
    voters: std::collections::BTreeSet<u64>,
}

/// Set the cluster's voter membership. Promotes learners / removes voters.
/// Must be called on the current leader.
async fn raft_change_membership(
    State(state): State<HttpState>,
    Json(req): Json<ChangeMembershipRequest>,
) -> ApiResult<Response> {
    state
        .workers
        .change_membership(req.voters)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok((StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response())
}
