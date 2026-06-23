//! DID (phone number) inventory service over `rustpbx_did_numbers`.
//!
//! DIDs are a platform-allocated resource: the superadmin adds numbers to the
//! inventory and assigns them to tenants (optionally pinning them to a trunk).
//! Tenants see their own assigned numbers read-only.

use crate::models::did_number::{self, ActiveModel, DidStatus, Entity, Model};
use anyhow::{Result, anyhow, bail};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};
use serde::{Deserialize, Serialize};

fn parse_status(s: &str) -> Result<DidStatus> {
    Ok(match s {
        "available" => DidStatus::Available,
        "assigned" => DidStatus::Assigned,
        "reserved" => DidStatus::Reserved,
        "porting" => DidStatus::Porting,
        other => bail!("unknown DID status '{}'", other),
    })
}

#[derive(Debug, Deserialize)]
pub struct CreateDidRequest {
    pub number: String,
    pub tenant_id: Option<i64>,
    pub trunk_id: Option<i64>,
    pub status: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub monthly_cost: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDidRequest {
    pub tenant_id: Option<i64>,
    pub trunk_id: Option<i64>,
    pub status: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub monthly_cost: Option<i32>,
    /// Set true to clear the tenant assignment (release the number).
    #[serde(default)]
    pub unassign: bool,
}

#[derive(Debug, Serialize)]
pub struct DidResponse {
    pub id: i64,
    pub number: String,
    pub tenant_id: Option<i64>,
    pub trunk_id: Option<i64>,
    pub status: String,
    pub country: Option<String>,
    pub city: Option<String>,
    pub monthly_cost: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Model> for DidResponse {
    fn from(m: Model) -> Self {
        Self {
            id: m.id,
            number: m.number,
            tenant_id: m.tenant_id,
            trunk_id: m.trunk_id,
            status: format!("{:?}", m.status).to_lowercase(),
            country: m.country,
            city: m.city,
            monthly_cost: m.monthly_cost,
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        }
    }
}

pub struct DidService<'a> {
    db: &'a DatabaseConnection,
}

impl<'a> DidService<'a> {
    pub fn new(db: &'a DatabaseConnection) -> Self {
        Self { db }
    }

    /// List DIDs. `Some(tid)` → only that tenant's numbers; `None` → entire
    /// inventory (superadmin).
    pub async fn list(&self, tenant_id: Option<i64>) -> Result<Vec<DidResponse>> {
        let mut q = Entity::find().order_by_asc(did_number::Column::Number);
        if let Some(tid) = tenant_id {
            q = q.filter(did_number::Column::TenantId.eq(tid));
        }
        Ok(q.all(self.db).await?.into_iter().map(Into::into).collect())
    }

    pub async fn get(&self, id: i64) -> Result<Model> {
        Entity::find_by_id(id)
            .one(self.db)
            .await?
            .ok_or_else(|| anyhow!("DID {} not found", id))
    }

    pub async fn create(&self, req: &CreateDidRequest) -> Result<DidResponse> {
        let number = req.number.trim();
        if number.is_empty() {
            bail!("number is required");
        }
        // Default status: assigned if a tenant is given, else available.
        let status = match req.status.as_deref() {
            Some(s) => parse_status(s)?,
            None if req.tenant_id.is_some() => DidStatus::Assigned,
            None => DidStatus::Available,
        };
        let now = Utc::now();
        let model = ActiveModel {
            number: Set(number.to_string()),
            tenant_id: Set(req.tenant_id),
            trunk_id: Set(req.trunk_id),
            status: Set(status),
            country: Set(req.country.clone()),
            city: Set(req.city.clone()),
            monthly_cost: Set(req.monthly_cost),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        Ok(model.insert(self.db).await?.into())
    }

    pub async fn update(&self, id: i64, req: UpdateDidRequest) -> Result<DidResponse> {
        let existing = self.get(id).await?;
        let mut model: ActiveModel = existing.into();
        if req.unassign {
            model.tenant_id = Set(None);
            model.trunk_id = Set(None);
            model.status = Set(DidStatus::Available);
        } else if let Some(tid) = req.tenant_id {
            model.tenant_id = Set(Some(tid));
            model.status = Set(DidStatus::Assigned);
        }
        if let Some(trunk) = req.trunk_id {
            model.trunk_id = Set(Some(trunk));
        }
        if let Some(s) = req.status {
            model.status = Set(parse_status(&s)?);
        }
        if let Some(c) = req.country {
            model.country = Set(Some(c));
        }
        if let Some(c) = req.city {
            model.city = Set(Some(c));
        }
        if let Some(c) = req.monthly_cost {
            model.monthly_cost = Set(Some(c));
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

    /// Count of DIDs assigned to a tenant (for dashboards).
    pub async fn count_for_tenant(&self, tenant_id: i64) -> Result<u64> {
        use sea_orm::PaginatorTrait;
        Ok(Entity::find()
            .filter(did_number::Column::TenantId.eq(tenant_id))
            .count(self.db)
            .await?)
    }
}
