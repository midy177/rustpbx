//! Tenant IAM accounts service — CRUD over `rustpbx_tenant_users` plus
//! authentication. Passwords are bcrypt-hashed; usernames are unique within a
//! tenant; permissions are validated against the catalogue in `auth::permissions`.

use crate::auth::password::{hash_password, verify_password};
use crate::auth::permissions::{self, db_role};
use crate::models::tenant_user::{self, ActiveModel, Entity, Model};
use anyhow::{Result, anyhow, bail};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};
use serde::{Deserialize, Serialize};

// ── Request / Response ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateTenantUserRequest {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
    /// `admin` or `user` (defaults to `user`).
    pub role: Option<String>,
    /// Granted permission strings (only meaningful for `user`).
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTenantUserRequest {
    pub display_name: Option<String>,
    /// New password (omit/empty to keep the current one).
    pub password: Option<String>,
    pub role: Option<String>,
    pub permissions: Option<Vec<String>>,
    /// `active` or `suspended`.
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TenantUserResponse {
    pub id: i64,
    pub tenant_id: i64,
    pub username: String,
    pub display_name: Option<String>,
    pub role: String,
    pub permissions: Vec<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_login_at: Option<String>,
}

impl From<Model> for TenantUserResponse {
    fn from(m: Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            username: m.username,
            display_name: m.display_name,
            role: m.role,
            permissions: parse_permissions(&m.permissions),
            status: m.status,
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
            last_login_at: m.last_login_at.map(|t| t.to_rfc3339()),
        }
    }
}

/// The resolved principal returned on successful authentication.
pub struct AuthenticatedUser {
    pub tenant_id: i64,
    pub username: String,
    /// Session role (`tenant_admin` / `tenant_user`).
    pub session_role: String,
    pub permissions: Vec<String>,
}

fn parse_permissions(json: &Option<sea_orm::prelude::Json>) -> Vec<String> {
    json.as_ref()
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default()
}

fn validate_role(role: &str) -> Result<&'static str> {
    match role {
        db_role::ADMIN => Ok(db_role::ADMIN),
        db_role::USER => Ok(db_role::USER),
        other => Err(anyhow!("unknown role '{}' (expected admin|user)", other)),
    }
}

fn validate_permissions(perms: &[String]) -> Result<()> {
    if let Some(bad) = permissions::first_unknown_permission(perms) {
        bail!("unknown permission '{}'", bad);
    }
    Ok(())
}

// ── Service ───────────────────────────────────────────────────────────────────

pub struct TenantUserService<'a> {
    db: &'a DatabaseConnection,
}

impl<'a> TenantUserService<'a> {
    pub fn new(db: &'a DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn list(&self, tenant_id: i64) -> Result<Vec<TenantUserResponse>> {
        let rows = Entity::find()
            .filter(tenant_user::Column::TenantId.eq(tenant_id))
            .order_by_asc(tenant_user::Column::Username)
            .all(self.db)
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn get(&self, id: i64) -> Result<Model> {
        Entity::find_by_id(id)
            .one(self.db)
            .await?
            .ok_or_else(|| anyhow!("user {} not found", id))
    }

    pub async fn create(
        &self,
        tenant_id: i64,
        req: &CreateTenantUserRequest,
    ) -> Result<TenantUserResponse> {
        let username = req.username.trim();
        if username.is_empty() {
            bail!("username is required");
        }
        if req.password.len() < 6 {
            bail!("password must be at least 6 characters");
        }
        let role = validate_role(req.role.as_deref().unwrap_or(db_role::USER))?;
        validate_permissions(&req.permissions)?;

        if self.find_by_username(tenant_id, username).await?.is_some() {
            bail!("username '{}' already exists in this tenant", username);
        }

        let now = Utc::now();
        let model = ActiveModel {
            tenant_id: Set(tenant_id),
            username: Set(username.to_string()),
            display_name: Set(req.display_name.clone()),
            password_hash: Set(hash_password(&req.password)?),
            role: Set(role.to_string()),
            permissions: Set(Some(serde_json::to_value(&req.permissions)?)),
            status: Set("active".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
            last_login_at: Set(None),
            ..Default::default()
        };
        Ok(model.insert(self.db).await?.into())
    }

    /// Provision a tenant's first admin account (used at tenant creation).
    pub async fn create_initial_admin(
        &self,
        tenant_id: i64,
        username: &str,
        password: &str,
    ) -> Result<TenantUserResponse> {
        self.create(
            tenant_id,
            &CreateTenantUserRequest {
                username: username.to_string(),
                password: password.to_string(),
                display_name: Some("Tenant Admin".to_string()),
                role: Some(db_role::ADMIN.to_string()),
                permissions: vec![],
            },
        )
        .await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateTenantUserRequest,
    ) -> Result<TenantUserResponse> {
        let existing = self.get(id).await?;
        let mut model: ActiveModel = existing.into();

        if let Some(dn) = req.display_name {
            model.display_name = Set(Some(dn));
        }
        if let Some(pw) = req.password.filter(|p| !p.is_empty()) {
            if pw.len() < 6 {
                bail!("password must be at least 6 characters");
            }
            model.password_hash = Set(hash_password(&pw)?);
        }
        if let Some(role) = req.role {
            model.role = Set(validate_role(&role)?.to_string());
        }
        if let Some(perms) = req.permissions {
            validate_permissions(&perms)?;
            model.permissions = Set(Some(serde_json::to_value(&perms)?));
        }
        if let Some(status) = req.status {
            match status.as_str() {
                "active" | "suspended" => model.status = Set(status),
                other => bail!("unknown status '{}' (expected active|suspended)", other),
            }
        }
        model.updated_at = Set(Utc::now());
        Ok(model.update(self.db).await?.into())
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        let existing = self.get(id).await?;
        let model: ActiveModel = existing.into();
        model.delete(self.db).await?;
        Ok(())
    }

    pub async fn find_by_username(&self, tenant_id: i64, username: &str) -> Result<Option<Model>> {
        Ok(Entity::find()
            .filter(tenant_user::Column::TenantId.eq(tenant_id))
            .filter(tenant_user::Column::Username.eq(username))
            .one(self.db)
            .await?)
    }

    /// Authenticate a tenant user within `tenant_id`. Returns the resolved
    /// principal on success; updates `last_login_at`.
    pub async fn authenticate(
        &self,
        tenant_id: i64,
        username: &str,
        password: &str,
    ) -> Result<Option<AuthenticatedUser>> {
        let Some(user) = self.find_by_username(tenant_id, username).await? else {
            return Ok(None);
        };
        if user.status != "active" {
            return Ok(None);
        }
        if !verify_password(password, &user.password_hash) {
            return Ok(None);
        }

        let principal = AuthenticatedUser {
            tenant_id: user.tenant_id,
            username: user.username.clone(),
            session_role: permissions::session_role_for(&user.role).to_string(),
            permissions: parse_permissions(&user.permissions),
        };

        // Best-effort last-login stamp.
        let mut am: ActiveModel = user.into();
        am.last_login_at = Set(Some(Utc::now()));
        let _ = am.update(self.db).await;

        Ok(Some(principal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::permissions::{TRUNKS_READ, roles};
    use crate::migration::ControlMigrator;
    use sea_orm::Database;
    use sea_orm_migration::{MigratorTrait, SchemaManager};

    async fn svc_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);
        for m in ControlMigrator::migrations() {
            m.up(&manager).await.unwrap();
        }
        db
    }

    #[tokio::test]
    async fn initial_admin_authenticates_as_tenant_admin() {
        let db = svc_db().await;
        let svc = TenantUserService::new(&db);
        svc.create_initial_admin(1, "root", "hunter2").await.unwrap();

        let ok = svc.authenticate(1, "root", "hunter2").await.unwrap().unwrap();
        assert_eq!(ok.session_role, roles::TENANT_ADMIN);
        assert_eq!(ok.tenant_id, 1);

        assert!(svc.authenticate(1, "root", "wrong").await.unwrap().is_none());
        // Wrong tenant scope → no match (usernames are per-tenant).
        assert!(svc.authenticate(2, "root", "hunter2").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn user_carries_granted_permissions() {
        let db = svc_db().await;
        let svc = TenantUserService::new(&db);
        svc.create(
            7,
            &CreateTenantUserRequest {
                username: "agent".into(),
                password: "secret1".into(),
                display_name: None,
                role: Some("user".into()),
                permissions: vec![TRUNKS_READ.into()],
            },
        )
        .await
        .unwrap();

        let p = svc.authenticate(7, "agent", "secret1").await.unwrap().unwrap();
        assert_eq!(p.session_role, roles::TENANT_USER);
        assert_eq!(p.permissions, vec![TRUNKS_READ.to_string()]);
    }

    #[tokio::test]
    async fn username_unique_within_tenant_only() {
        let db = svc_db().await;
        let svc = TenantUserService::new(&db);
        svc.create_initial_admin(1, "admin", "pass123").await.unwrap();
        // Same username, different tenant → allowed.
        svc.create_initial_admin(2, "admin", "pass123").await.unwrap();
        // Same username, same tenant → rejected.
        assert!(svc.create_initial_admin(1, "admin", "pass123").await.is_err());
    }

    #[tokio::test]
    async fn suspended_user_cannot_authenticate() {
        let db = svc_db().await;
        let svc = TenantUserService::new(&db);
        let u = svc.create_initial_admin(1, "root", "hunter2").await.unwrap();
        svc.update(
            u.id,
            UpdateTenantUserRequest {
                display_name: None,
                password: None,
                role: None,
                permissions: None,
                status: Some("suspended".into()),
            },
        )
        .await
        .unwrap();
        assert!(svc.authenticate(1, "root", "hunter2").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rejects_unknown_permission() {
        let db = svc_db().await;
        let svc = TenantUserService::new(&db);
        let err = svc
            .create(
                1,
                &CreateTenantUserRequest {
                    username: "x".into(),
                    password: "secret1".into(),
                    display_name: None,
                    role: Some("user".into()),
                    permissions: vec!["bogus:perm".into()],
                },
            )
            .await;
        assert!(err.is_err());
    }
}
