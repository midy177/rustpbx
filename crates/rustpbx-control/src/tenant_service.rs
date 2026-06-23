use crate::models::tenant::{self, ActiveModel, Entity, Model, TenantStatus};
use anyhow::{Result, anyhow};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};
use serde::{Deserialize, Serialize};

/// Lower/upper bounds for AWS-account-style tenant ids: always exactly 12
/// digits, so default domains look like `100035381533.pbx.example.com`.
const TENANT_ID_MIN: i64 = 100_000_000_000;
const TENANT_ID_SPAN: u64 = 900_000_000_000;

/// Generate a random, non-sequential 12-digit tenant id (AWS-account style).
/// Derived from a v4 UUID's randomness — no extra RNG dependency.
fn random_tenant_id() -> i64 {
    let r = (uuid::Uuid::new_v4().as_u128() as u64) % TENANT_ID_SPAN;
    TENANT_ID_MIN + r as i64
}

// ── Domain helpers ────────────────────────────────────────────────────────────

/// The auto-assigned default domain for a tenant: `{id}.{base_domain}`.
/// `None` when no platform `base_domain` is configured.
pub fn default_domain(tenant_id: i64, base_domain: &str) -> Option<String> {
    let b = base_domain.trim();
    (!b.is_empty()).then(|| format!("{tenant_id}.{b}"))
}

/// A tenant's currently active domain: the custom domain when enabled and set,
/// otherwise the auto-assigned default. The default is always *reserved* for the
/// tenant even while a custom domain is active.
pub fn active_domain(t: &Model, base_domain: &str) -> Option<String> {
    if t.custom_domain_enabled {
        if let Some(c) = t.custom_domain.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            return Some(c.to_string());
        }
    }
    default_domain(t.id, base_domain)
}

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub max_concurrent_calls: Option<i32>,
    pub max_trunks: Option<i32>,
    pub max_dids: Option<i32>,
    pub storage_prefix: Option<String>,
    pub custom_domain: Option<String>,
    pub metadata: Option<serde_json::Value>,
    /// Optional initial tenant-admin account provisioned with the tenant.
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTenantRequest {
    pub name: Option<String>,
    pub status: Option<String>,
    pub max_concurrent_calls: Option<i32>,
    pub max_trunks: Option<i32>,
    pub max_dids: Option<i32>,
    pub storage_prefix: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Tenant-admin's domain settings update.
#[derive(Debug, Deserialize)]
pub struct UpdateDomainRequest {
    /// New custom domain (`""`/null clears it).
    pub custom_domain: Option<String>,
    /// Whether the custom domain is the active one.
    pub custom_domain_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct TenantResponse {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub max_concurrent_calls: Option<i32>,
    pub max_trunks: Option<i32>,
    pub max_dids: Option<i32>,
    pub storage_prefix: Option<String>,
    pub custom_domain: Option<String>,
    pub custom_domain_enabled: bool,
    /// Auto-assigned `{id}.{base_domain}` (always reserved for the tenant).
    pub default_domain: Option<String>,
    /// Currently effective domain (custom when enabled, else default).
    pub active_domain: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ── Service ───────────────────────────────────────────────────────────────────

pub struct TenantService<'a> {
    db: &'a DatabaseConnection,
    /// Platform wildcard base domain (for computing default domains).
    base_domain: String,
}

impl<'a> TenantService<'a> {
    pub fn new(db: &'a DatabaseConnection, base_domain: impl Into<String>) -> Self {
        Self { db, base_domain: base_domain.into() }
    }

    fn to_response(&self, m: Model) -> TenantResponse {
        TenantResponse {
            id: m.id,
            name: m.name.clone(),
            status: format!("{:?}", m.status).to_lowercase(),
            max_concurrent_calls: m.max_concurrent_calls,
            max_trunks: m.max_trunks,
            max_dids: m.max_dids,
            storage_prefix: m.storage_prefix.clone(),
            custom_domain: m.custom_domain.clone(),
            custom_domain_enabled: m.custom_domain_enabled,
            default_domain: default_domain(m.id, &self.base_domain),
            active_domain: active_domain(&m, &self.base_domain),
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }
    }

    pub async fn list(&self) -> Result<Vec<TenantResponse>> {
        let rows = Entity::find()
            .filter(tenant::Column::Status.ne("deleted"))
            .order_by_asc(tenant::Column::Name)
            .all(self.db)
            .await?;
        Ok(rows.into_iter().map(|m| self.to_response(m)).collect())
    }

    pub async fn get(&self, id: i64) -> Result<TenantResponse> {
        let m = self.get_model(id).await?;
        Ok(self.to_response(m))
    }

    /// Fetch the raw tenant model (used by callers needing the domain fields).
    pub async fn get_model(&self, id: i64) -> Result<Model> {
        Entity::find_by_id(id)
            .one(self.db)
            .await?
            .ok_or_else(|| anyhow!("tenant {} not found", id))
    }

    pub async fn create(&self, req: &CreateTenantRequest) -> Result<TenantResponse> {
        let now = Utc::now();
        let custom_domain = req
            .custom_domain
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        if let Some(ref d) = custom_domain {
            self.ensure_domain_unique(d, None).await?;
        }

        // Allocate an AWS-style 12-digit id, retrying on the (astronomically
        // unlikely) collision rather than relying on DB auto-increment.
        let id = self.allocate_tenant_id().await?;

        let model = ActiveModel {
            id: Set(id),
            name: Set(req.name.clone()),
            status: Set(TenantStatus::Active),
            max_concurrent_calls: Set(req.max_concurrent_calls),
            max_trunks: Set(req.max_trunks),
            max_dids: Set(req.max_dids),
            storage_prefix: Set(req.storage_prefix.clone()),
            custom_domain: Set(custom_domain.clone()),
            custom_domain_enabled: Set(custom_domain.is_some()),
            metadata: Set(req.metadata.clone().map(sea_orm::prelude::Json::from)),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let row = model.insert(self.db).await?;
        Ok(self.to_response(row))
    }

    pub async fn update(&self, id: i64, req: UpdateTenantRequest) -> Result<TenantResponse> {
        let existing = self.get_model(id).await?;
        let mut model: ActiveModel = existing.into();
        if let Some(name) = req.name {
            model.name = Set(name);
        }
        if let Some(status) = req.status {
            model.status = Set(match status.as_str() {
                "active" => TenantStatus::Active,
                "suspended" => TenantStatus::Suspended,
                "deleted" => TenantStatus::Deleted,
                other => return Err(anyhow!("unknown status: {}", other)),
            });
        }
        if let Some(v) = req.max_concurrent_calls {
            model.max_concurrent_calls = Set(Some(v));
        }
        if let Some(v) = req.max_trunks {
            model.max_trunks = Set(Some(v));
        }
        if let Some(v) = req.max_dids {
            model.max_dids = Set(Some(v));
        }
        if let Some(v) = req.storage_prefix {
            model.storage_prefix = Set(Some(v));
        }
        if let Some(v) = req.metadata {
            model.metadata = Set(Some(sea_orm::prelude::Json::from(v)));
        }
        model.updated_at = Set(Utc::now());

        let row = model.update(self.db).await?;
        Ok(self.to_response(row))
    }

    /// Update a tenant's domain settings (tenant-admin self-service). Enforces
    /// custom-domain uniqueness across tenants. The default domain is never
    /// reassigned — disabling the custom domain simply reactivates the default.
    pub async fn update_domain(&self, id: i64, req: UpdateDomainRequest) -> Result<TenantResponse> {
        let existing = self.get_model(id).await?;
        let custom = req
            .custom_domain
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        if let Some(ref d) = custom {
            self.ensure_domain_unique(d, Some(id)).await?;
        }
        // Can't enable a domain that isn't set.
        let enabled = req.custom_domain_enabled && custom.is_some();

        let mut model: ActiveModel = existing.into();
        model.custom_domain = Set(custom);
        model.custom_domain_enabled = Set(enabled);
        model.updated_at = Set(Utc::now());
        let row = model.update(self.db).await?;
        Ok(self.to_response(row))
    }

    /// Resolve a login domain to its tenant. A custom domain matches only when
    /// enabled; a default `{id}.{base_domain}` domain matches only while the
    /// custom domain is *not* active (the default is paused, not removed, when a
    /// custom domain takes over).
    pub async fn resolve_by_domain(&self, domain: &str) -> Result<Option<Model>> {
        let domain = domain.trim();
        if domain.is_empty() {
            return Ok(None);
        }

        // Custom-domain match (must be the active one).
        if let Some(t) = Entity::find()
            .filter(tenant::Column::CustomDomain.eq(domain))
            .filter(tenant::Column::Status.ne("deleted"))
            .one(self.db)
            .await?
            && t.custom_domain_enabled
        {
            return Ok(Some(t));
        }

        // Default-domain match: `{id}.{base_domain}`, active only when no custom
        // domain is in force.
        let base = self.base_domain.trim();
        if !base.is_empty() {
            let suffix = format!(".{base}");
            if let Some(prefix) = domain.strip_suffix(&suffix) {
                if let Ok(id) = prefix.parse::<i64>() {
                    if let Some(t) = Entity::find_by_id(id).one(self.db).await?
                        && t.status != TenantStatus::Deleted
                        && active_domain(&t, base).as_deref() == Some(domain)
                    {
                        return Ok(Some(t));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Pick a free 12-digit tenant id, retrying on collision.
    async fn allocate_tenant_id(&self) -> Result<i64> {
        for _ in 0..8 {
            let candidate = random_tenant_id();
            if Entity::find_by_id(candidate).one(self.db).await?.is_none() {
                return Ok(candidate);
            }
        }
        Err(anyhow!("could not allocate a unique tenant id after several attempts"))
    }

    /// Reject a custom domain already taken by another tenant.
    async fn ensure_domain_unique(&self, domain: &str, exclude_id: Option<i64>) -> Result<()> {
        let mut q = Entity::find()
            .filter(tenant::Column::CustomDomain.eq(domain))
            .filter(tenant::Column::Status.ne("deleted"));
        if let Some(id) = exclude_id {
            q = q.filter(tenant::Column::Id.ne(id));
        }
        if q.one(self.db).await?.is_some() {
            return Err(anyhow!("domain '{}' is already in use by another tenant", domain));
        }
        Ok(())
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        let existing = self.get_model(id).await?;
        let mut model: ActiveModel = existing.into();
        model.status = Set(TenantStatus::Deleted);
        model.updated_at = Set(Utc::now());
        model.update(self.db).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::ControlMigrator;
    use sea_orm::Database;
    use sea_orm_migration::{MigratorTrait, SchemaManager};

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);
        for m in ControlMigrator::migrations() {
            m.up(&manager).await.unwrap();
        }
        db
    }

    fn req(name: &str, custom: Option<&str>) -> CreateTenantRequest {
        CreateTenantRequest {
            name: name.into(),
            max_concurrent_calls: None,
            max_trunks: None,
            max_dids: None,
            storage_prefix: None,
            custom_domain: custom.map(Into::into),
            metadata: None,
            admin_username: None,
            admin_password: None,
        }
    }

    #[tokio::test]
    async fn default_domain_derives_from_base_and_id() {
        let db = fresh_db().await;
        let svc = TenantService::new(&db, "pbx.example.com");
        let t = svc.create(&req("acme", None)).await.unwrap();
        assert_eq!(t.default_domain.as_deref(), Some(&*format!("{}.pbx.example.com", t.id)));
        // No custom domain → default is the active one.
        assert_eq!(t.active_domain, t.default_domain);
        assert!(!t.custom_domain_enabled);
    }

    #[tokio::test]
    async fn custom_domain_takes_over_and_default_is_reserved() {
        let db = fresh_db().await;
        let svc = TenantService::new(&db, "pbx.example.com");
        let t = svc.create(&req("acme", None)).await.unwrap();

        let updated = svc
            .update_domain(
                t.id,
                UpdateDomainRequest {
                    custom_domain: Some("voip.acme.io".into()),
                    custom_domain_enabled: true,
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.active_domain.as_deref(), Some("voip.acme.io"));
        // Default still computed (reserved), just not active.
        assert_eq!(updated.default_domain.as_deref(), Some(&*format!("{}.pbx.example.com", t.id)));

        // Login via the custom domain resolves; via the (now paused) default does not.
        let by_custom = svc.resolve_by_domain("voip.acme.io").await.unwrap();
        assert_eq!(by_custom.map(|m| m.id), Some(t.id));
        let by_default = svc.resolve_by_domain(&format!("{}.pbx.example.com", t.id)).await.unwrap();
        assert!(by_default.is_none(), "default domain is paused while custom is active");
    }

    #[tokio::test]
    async fn default_domain_resolves_when_no_custom() {
        let db = fresh_db().await;
        let svc = TenantService::new(&db, "pbx.example.com");
        let t = svc.create(&req("acme", None)).await.unwrap();
        let resolved = svc.resolve_by_domain(&format!("{}.pbx.example.com", t.id)).await.unwrap();
        assert_eq!(resolved.map(|m| m.id), Some(t.id));
    }

    #[tokio::test]
    async fn tenant_id_is_aws_style_12_digits() {
        let db = fresh_db().await;
        let svc = TenantService::new(&db, "pbx.example.com");
        let t = svc.create(&req("acme", None)).await.unwrap();
        assert!(
            (100_000_000_000..=999_999_999_999).contains(&t.id),
            "tenant id must be 12 digits, got {}",
            t.id
        );
        // Default domain embeds the 12-digit id.
        assert_eq!(t.default_domain, Some(format!("{}.pbx.example.com", t.id)));
        // Distinct tenants get distinct ids.
        let t2 = svc.create(&req("beta", None)).await.unwrap();
        assert_ne!(t.id, t2.id);
    }

    #[tokio::test]
    async fn custom_domain_must_be_unique() {
        let db = fresh_db().await;
        let svc = TenantService::new(&db, "pbx.example.com");
        let _a = svc.create(&req("a", Some("shared.example.com"))).await.unwrap();
        let err = svc.create(&req("b", Some("shared.example.com"))).await;
        assert!(err.is_err(), "duplicate custom domain rejected");
    }
}
