use crate::{
    app::AppState,
    models::{call_record, extension, routing, sip_trunk, user},
};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordVerifier},
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::engine::{Engine, general_purpose::STANDARD_NO_PAD};
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use sea_orm::sea_query::Condition;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Sha256;
use std::time::Duration;

const SESSION_COOKIE: &str = "cloudpbx_session";
const SESSION_TTL_SECS: u64 = 12 * 3600;

type HmacSha256 = Hmac<Sha256>;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/session", get(session))
        .route("/api/tenants", get(list_tenants))
        .route("/api/cloudpbx/extensions", get(list_extensions))
        .route("/api/cloudpbx/sip-trunks", get(list_sip_trunks))
        .route("/api/cloudpbx/routes", get(list_routes))
        .route("/api/cloudpbx/call-records", get(list_call_records))
        .route("/api/cloudpbx/users", get(list_users))
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
    tenant: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct TenantSummary {
    id: String,
    name: String,
    status: String,
    domain: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionUser {
    id: i64,
    username: String,
    email: Option<String>,
    role: String,
    tenant: TenantSummary,
}

async fn login(
    State(state): State<AppState>,
    _headers: HeaderMap,
    Json(payload): Json<LoginRequest>,
) -> Response {
    let username = payload.username.trim();
    let password = payload.password.as_str();
    if username.is_empty() || password.is_empty() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "username and password are required",
        );
    }

    let condition = Condition::any()
        .add(user::Column::Username.eq(username))
        .add(user::Column::Email.eq(username.to_ascii_lowercase()));

    let found = match user::Entity::find().filter(condition).one(state.db()).await {
        Ok(user) => user,
        Err(err) => {
            tracing::warn!(error = %err, "cloudpbx login user lookup failed");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "failed to query user");
        }
    };

    let Some(found) = found else {
        return json_error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    if !found.is_active || !verify_password(password, &found.password_hash) {
        return json_error(StatusCode::UNAUTHORIZED, "invalid credentials");
    }

    let mut active: user::ActiveModel = found.clone().into();
    active.last_login_at = Set(Some(Utc::now()));
    if let Err(err) = active.update(state.db()).await {
        tracing::warn!(error = %err, user_id = found.id, "failed to update cloudpbx last_login_at");
    }

    let token = match generate_session_token(&state, found.id) {
        Some(token) => token,
        None => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to create session",
            );
        }
    };
    let cookie = session_cookie(&token, Some(SESSION_TTL_SECS));
    let user = session_user(&found, payload.tenant.as_deref());

    match HeaderValue::from_str(&cookie) {
        Ok(cookie) => ([(header::SET_COOKIE, cookie)], Json(user)).into_response(),
        Err(err) => {
            tracing::warn!(error = %err, "failed to build cloudpbx session cookie");
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to create session",
            )
        }
    }
}

async fn logout() -> Response {
    let cookie = clear_session_cookie();
    match HeaderValue::from_str(&cookie) {
        Ok(cookie) => (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response(),
        Err(_) => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(user) = current_user(&state, &headers).await else {
        return json_error(StatusCode::UNAUTHORIZED, "not authenticated");
    };
    Json(session_user(&user, None)).into_response()
}

async fn list_tenants(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if current_user(&state, &headers).await.is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "not authenticated");
    }
    Json(vec![default_tenant(None)]).into_response()
}

async fn list_extensions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if current_user(&state, &headers).await.is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "not authenticated");
    }
    match extension::Entity::find()
        .order_by_asc(extension::Column::Extension)
        .all(state.db())
        .await
    {
        Ok(rows) => Json(rows).into_response(),
        Err(err) => query_failed(err),
    }
}

async fn list_sip_trunks(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if current_user(&state, &headers).await.is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "not authenticated");
    }
    match sip_trunk::Entity::find()
        .order_by_asc(sip_trunk::Column::Name)
        .all(state.db())
        .await
    {
        Ok(rows) => Json(rows).into_response(),
        Err(err) => query_failed(err),
    }
}

async fn list_routes(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if current_user(&state, &headers).await.is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "not authenticated");
    }
    match routing::Entity::find()
        .order_by_asc(routing::Column::Priority)
        .order_by_asc(routing::Column::Name)
        .all(state.db())
        .await
    {
        Ok(rows) => Json(rows).into_response(),
        Err(err) => query_failed(err),
    }
}

async fn list_call_records(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if current_user(&state, &headers).await.is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "not authenticated");
    }
    match call_record::Entity::find()
        .order_by_desc(call_record::Column::StartedAt)
        .limit(100)
        .all(state.db())
        .await
    {
        Ok(rows) => Json(rows).into_response(),
        Err(err) => query_failed(err),
    }
}

async fn list_users(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if current_user(&state, &headers).await.is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "not authenticated");
    }
    match user::Entity::find()
        .order_by_asc(user::Column::Username)
        .all(state.db())
        .await
    {
        Ok(rows) => Json(rows).into_response(),
        Err(err) => query_failed(err),
    }
}

async fn current_user(state: &AppState, headers: &HeaderMap) -> Option<user::Model> {
    let token = extract_cookie(headers, SESSION_COOKIE)?;
    let user_id = session_user_id(state, &token)?;
    match user::Entity::find_by_id(user_id).one(state.db()).await {
        Ok(Some(user)) if user.is_active => Some(user),
        Ok(_) => None,
        Err(err) => {
            tracing::warn!(error = %err, "failed to load cloudpbx session user");
            None
        }
    }
}

fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn session_user(user: &user::Model, tenant: Option<&str>) -> SessionUser {
    SessionUser {
        id: user.id,
        username: user.username.clone(),
        email: Some(user.email.clone()),
        role: if user.is_superuser {
            "platform_admin"
        } else if user.is_staff {
            "tenant_admin"
        } else {
            "tenant_user"
        }
        .to_string(),
        tenant: default_tenant(tenant),
    }
}

fn default_tenant(id: Option<&str>) -> TenantSummary {
    let id = id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default");
    TenantSummary {
        id: id.to_string(),
        name: if id == "default" {
            "Default".to_string()
        } else {
            id.to_string()
        },
        status: "active".to_string(),
        domain: None,
    }
}

fn generate_session_token(state: &AppState, user_id: i64) -> Option<String> {
    let expires = Utc::now() + Duration::from_secs(SESSION_TTL_SECS);
    let payload = format!("{}:{}", user_id, expires.timestamp());
    let signature = sign(state, &payload)?;
    Some(format!("{}:{}", payload, signature))
}

fn session_user_id(state: &AppState, token: &str) -> Option<i64> {
    let mut segments = token.split(':');
    let user_id: i64 = segments.next()?.parse().ok()?;
    let expires: i64 = segments.next()?.parse().ok()?;
    let signature = segments.next()?;
    if segments.next().is_some() || expires <= Utc::now().timestamp() {
        return None;
    }
    let payload = format!("{}:{}", user_id, expires);
    let expected = sign(state, &payload)?;
    (expected == signature).then_some(user_id)
}

fn sign(state: &AppState, payload: &str) -> Option<String> {
    let key = format!(
        "cloudpbx:{}:{}",
        state.config().http_addr,
        state.config().database_url
    );
    let mut mac = HmacSha256::new_from_slice(key.as_bytes()).ok()?;
    mac.update(payload.as_bytes());
    Some(STANDARD_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    for value in headers.get_all(header::COOKIE) {
        let Ok(raw) = value.to_str() else {
            continue;
        };
        for part in raw.split(';') {
            let trimmed = part.trim();
            let Some((cookie_name, cookie_value)) = trimmed.split_once('=') else {
                continue;
            };
            if cookie_name == name {
                return Some(cookie_value.to_string());
            }
        }
    }
    None
}

fn session_cookie(token: &str, max_age: Option<u64>) -> String {
    let max_age = max_age
        .map(|secs| format!("; Max-Age={secs}"))
        .unwrap_or_default();
    format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax{max_age}")
}

fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
}

fn query_failed(err: sea_orm::DbErr) -> Response {
    tracing::warn!(error = %err, "cloudpbx api query failed");
    json_error(StatusCode::INTERNAL_SERVER_ERROR, "query failed")
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}
