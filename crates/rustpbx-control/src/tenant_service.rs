use crate::models::tenant::{self, ActiveModel, Entity, Model, TenantStatus};
use anyhow::{Result, anyhow};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub max_concurrent_calls: Option<i32>,
    pub max_trunks: Option<i32>,
    pub max_dids: Option<i32>,
    pub storage_prefix: Option<String>,
    pub metadata: Option<serde_json::Value>,
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

#[derive(Debug, Serialize)]
pub struct TenantResponse {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub max_concurrent_calls: Option<i32>,
    pub max_trunks: Option<i32>,
    pub max_dids: Option<i32>,
    pub storage_prefix: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Model> for TenantResponse {
    fn from(m: Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            status: format!("{:?}", m.status).to_lowercase(),
            max_concurrent_calls: m.max_concurrent_calls,
            max_trunks: m.max_trunks,
            max_dids: m.max_dids,
            storage_prefix: m.storage_prefix,
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

// ── Service ───────────────────────────────────────────────────────────────────

pub struct TenantService<'a> {
    db: &'a DatabaseConnection,
}

impl<'a> TenantService<'a> {
    pub fn new(db: &'a DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> Result<Vec<TenantResponse>> {
        let rows = Entity::find()
            .filter(tenant::Column::Status.ne("deleted"))
            .order_by_asc(tenant::Column::Name)
            .all(self.db)
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn get(&self, id: i64) -> Result<TenantResponse> {
        Entity::find_by_id(id)
            .one(self.db)
            .await?
            .ok_or_else(|| anyhow!("tenant {} not found", id))
            .map(Into::into)
    }

    pub async fn create(&self, req: CreateTenantRequest) -> Result<TenantResponse> {
        let now = Utc::now();
        let model = ActiveModel {
            name: Set(req.name),
            status: Set(TenantStatus::Active),
            max_concurrent_calls: Set(req.max_concurrent_calls),
            max_trunks: Set(req.max_trunks),
            max_dids: Set(req.max_dids),
            storage_prefix: Set(req.storage_prefix),
            metadata: Set(req.metadata.map(sea_orm::prelude::Json::from)),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let row = model.insert(self.db).await?;
        Ok(row.into())
    }

    pub async fn update(&self, id: i64, req: UpdateTenantRequest) -> Result<TenantResponse> {
        let existing = Entity::find_by_id(id)
            .one(self.db)
            .await?
            .ok_or_else(|| anyhow!("tenant {} not found", id))?;

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
        Ok(row.into())
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        let existing = Entity::find_by_id(id)
            .one(self.db)
            .await?
            .ok_or_else(|| anyhow!("tenant {} not found", id))?;

        let mut model: ActiveModel = existing.into();
        model.status = Set(TenantStatus::Deleted);
        model.updated_at = Set(Utc::now());
        model.update(self.db).await?;
        Ok(())
    }
}
