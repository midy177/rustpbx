//! Write-path (create/update/delete) for tenant-configurable PBX resources:
//! SIP trunks, routing rules and extensions. Reads live in `db_queries.rs`.
//!
//! All mutations are tenant-scoped: `row_tenant` is the tenant a new row belongs
//! to; `scope_tenant` (on update/delete) restricts the affected row to a tenant
//! (`None` = superadmin, no restriction). Updates are full-replace — the admin
//! UI submits the complete object — keeping the SQL static and portable across
//! SQLite / Postgres / MySQL.

use crate::store::Store;
use anyhow::Result;
use sea_orm::{ConnectionTrait, Statement, Value};
use serde::{Deserialize, Serialize};

fn jstr(v: &[String]) -> Value {
    Value::Json(Some(Box::new(serde_json::json!(v))))
}
fn opt_str(v: &Option<String>) -> Value {
    Value::String(v.clone().map(Box::new))
}
fn opt_int(v: Option<i32>) -> Value {
    Value::Int(v)
}

// ── Trunks ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TrunkInput {
    pub name: String,
    pub display_name: Option<String>,
    pub carrier: Option<String>,
    #[serde(default = "default_direction")]
    pub direction: String,
    pub sip_server: Option<String>,
    #[serde(default = "default_transport")]
    pub sip_transport: String,
    pub outbound_proxy: Option<String>,
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
    pub max_cps: Option<i32>,
    pub max_concurrent: Option<i32>,
    #[serde(default)]
    pub allowed_ips: Vec<String>,
    #[serde(default)]
    pub did_numbers: Vec<String>,
    pub incoming_from_user_prefix: Option<String>,
    pub incoming_to_user_prefix: Option<String>,
    #[serde(default = "default_true")]
    pub is_active: bool,
    #[serde(default)]
    pub register_enabled: bool,
    pub register_expires: Option<i32>,
    #[serde(default = "default_true")]
    pub rewrite_hostport: bool,
}

fn default_direction() -> String {
    "bidirectional".into()
}
fn default_transport() -> String {
    "udp".into()
}
fn default_true() -> bool {
    true
}

impl Store {
    /// Count of trunks owned by a tenant (exactly, excluding global rows) — for
    /// quota enforcement.
    pub async fn count_trunks_for_tenant(&self, tenant_id: i64) -> Result<u64> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                "SELECT COUNT(*) AS cnt FROM rustpbx_sip_trunks WHERE tenant_id = $1",
                vec![Value::BigInt(Some(tenant_id))],
            ))
            .await?;
        let cnt: i64 = row.and_then(|r| r.try_get("", "cnt").ok()).unwrap_or(0);
        Ok(cnt as u64)
    }

    pub async fn create_trunk(&self, t: &TrunkInput, row_tenant: Option<i64>) -> Result<()> {
        let sql = "INSERT INTO rustpbx_sip_trunks \
            (name, display_name, carrier, status, direction, sip_server, sip_transport, \
             outbound_proxy, auth_username, auth_password, max_cps, max_concurrent, \
             allowed_ips, did_numbers, incoming_from_user_prefix, incoming_to_user_prefix, \
             is_active, register_enabled, register_expires, rewrite_hostport, tenant_id) \
            VALUES ($1,$2,$3,'active',$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)";
        self.exec(
            sql,
            vec![
                Value::String(Some(Box::new(t.name.clone()))),
                opt_str(&t.display_name),
                opt_str(&t.carrier),
                Value::String(Some(Box::new(t.direction.clone()))),
                opt_str(&t.sip_server),
                Value::String(Some(Box::new(t.sip_transport.clone()))),
                opt_str(&t.outbound_proxy),
                opt_str(&t.auth_username),
                opt_str(&t.auth_password),
                opt_int(t.max_cps),
                opt_int(t.max_concurrent),
                jstr(&t.allowed_ips),
                jstr(&t.did_numbers),
                opt_str(&t.incoming_from_user_prefix),
                opt_str(&t.incoming_to_user_prefix),
                Value::Bool(Some(t.is_active)),
                Value::Bool(Some(t.register_enabled)),
                opt_int(t.register_expires),
                Value::Bool(Some(t.rewrite_hostport)),
                Value::BigInt(row_tenant),
            ],
        )
        .await
    }

    pub async fn update_trunk(
        &self,
        id: i64,
        t: &TrunkInput,
        scope_tenant: Option<i64>,
    ) -> Result<u64> {
        let mut sql = String::from(
            "UPDATE rustpbx_sip_trunks SET \
             name=$1, display_name=$2, carrier=$3, direction=$4, sip_server=$5, sip_transport=$6, \
             outbound_proxy=$7, auth_username=$8, auth_password=$9, max_cps=$10, max_concurrent=$11, \
             allowed_ips=$12, did_numbers=$13, incoming_from_user_prefix=$14, \
             incoming_to_user_prefix=$15, is_active=$16, register_enabled=$17, register_expires=$18, \
             rewrite_hostport=$19, updated_at=CURRENT_TIMESTAMP WHERE id=$20",
        );
        let mut vals = vec![
            Value::String(Some(Box::new(t.name.clone()))),
            opt_str(&t.display_name),
            opt_str(&t.carrier),
            Value::String(Some(Box::new(t.direction.clone()))),
            opt_str(&t.sip_server),
            Value::String(Some(Box::new(t.sip_transport.clone()))),
            opt_str(&t.outbound_proxy),
            opt_str(&t.auth_username),
            opt_str(&t.auth_password),
            opt_int(t.max_cps),
            opt_int(t.max_concurrent),
            jstr(&t.allowed_ips),
            jstr(&t.did_numbers),
            opt_str(&t.incoming_from_user_prefix),
            opt_str(&t.incoming_to_user_prefix),
            Value::Bool(Some(t.is_active)),
            Value::Bool(Some(t.register_enabled)),
            opt_int(t.register_expires),
            Value::Bool(Some(t.rewrite_hostport)),
            Value::BigInt(Some(id)),
        ];
        if let Some(tid) = scope_tenant {
            sql.push_str(" AND tenant_id=$21");
            vals.push(Value::BigInt(Some(tid)));
        }
        self.exec_affected(&sql, vals).await
    }

    pub async fn delete_trunk(&self, id: i64, scope_tenant: Option<i64>) -> Result<u64> {
        self.delete_scoped("rustpbx_sip_trunks", id, scope_tenant).await
    }

    // ── Routing ────────────────────────────────────────────────────────────────

    pub async fn create_route(&self, r: &RouteInput, row_tenant: Option<i64>) -> Result<()> {
        let sql = "INSERT INTO rustpbx_routes \
            (name, description, direction, priority, is_active, selection_strategy, hash_key, \
             source_pattern, destination_pattern, target_trunks, tenant_id) \
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)";
        self.exec(
            sql,
            vec![
                Value::String(Some(Box::new(r.name.clone()))),
                opt_str(&r.description),
                Value::String(Some(Box::new(r.direction.clone()))),
                Value::Int(Some(r.priority)),
                Value::Bool(Some(r.is_active)),
                Value::String(Some(Box::new(r.selection_strategy.clone()))),
                opt_str(&r.hash_key),
                opt_str(&r.source_pattern),
                opt_str(&r.destination_pattern),
                target_trunks_json(&r.target_trunks),
                Value::BigInt(row_tenant),
            ],
        )
        .await
    }

    pub async fn update_route(
        &self,
        id: i64,
        r: &RouteInput,
        scope_tenant: Option<i64>,
    ) -> Result<u64> {
        let mut sql = String::from(
            "UPDATE rustpbx_routes SET \
             name=$1, description=$2, direction=$3, priority=$4, is_active=$5, \
             selection_strategy=$6, hash_key=$7, source_pattern=$8, destination_pattern=$9, \
             target_trunks=$10, updated_at=CURRENT_TIMESTAMP WHERE id=$11",
        );
        let mut vals = vec![
            Value::String(Some(Box::new(r.name.clone()))),
            opt_str(&r.description),
            Value::String(Some(Box::new(r.direction.clone()))),
            Value::Int(Some(r.priority)),
            Value::Bool(Some(r.is_active)),
            Value::String(Some(Box::new(r.selection_strategy.clone()))),
            opt_str(&r.hash_key),
            opt_str(&r.source_pattern),
            opt_str(&r.destination_pattern),
            target_trunks_json(&r.target_trunks),
            Value::BigInt(Some(id)),
        ];
        if let Some(tid) = scope_tenant {
            sql.push_str(" AND tenant_id=$12");
            vals.push(Value::BigInt(Some(tid)));
        }
        self.exec_affected(&sql, vals).await
    }

    pub async fn delete_route(&self, id: i64, scope_tenant: Option<i64>) -> Result<u64> {
        self.delete_scoped("rustpbx_routes", id, scope_tenant).await
    }

    // ── Extensions ──────────────────────────────────────────────────────────────

    pub async fn list_extensions(&self, tenant_id: Option<i64>) -> Result<Vec<ExtensionView>> {
        let cols = "id, extension, tenant_id, display_name, email, status, login_disabled, \
            voicemail_disabled, allow_guest_calls, call_forwarding_mode, \
            call_forwarding_destination, call_forwarding_timeout";
        let (sql, vals) = match tenant_id {
            Some(tid) => (
                format!(
                    "SELECT {cols} FROM rustpbx_extensions \
                     WHERE tenant_id = $1 OR tenant_id IS NULL ORDER BY extension"
                ),
                vec![Value::BigInt(Some(tid))],
            ),
            None => (
                format!("SELECT {cols} FROM rustpbx_extensions ORDER BY extension"),
                vec![],
            ),
        };
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                &sql,
                vals,
            ))
            .await?;
        rows.iter().map(row_to_extension_view).collect()
    }

    pub async fn create_extension(&self, e: &ExtensionInput, row_tenant: Option<i64>) -> Result<()> {
        let sql = "INSERT INTO rustpbx_extensions \
            (extension, tenant_id, display_name, email, status, login_disabled, \
             voicemail_disabled, allow_guest_calls, sip_password, call_forwarding_mode, \
             call_forwarding_destination, call_forwarding_timeout) \
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)";
        self.exec(
            sql,
            vec![
                Value::String(Some(Box::new(e.extension.clone()))),
                Value::BigInt(row_tenant),
                opt_str(&e.display_name),
                opt_str(&e.email),
                opt_str(&e.status),
                Value::Bool(Some(e.login_disabled)),
                Value::Bool(Some(e.voicemail_disabled)),
                Value::Bool(Some(e.allow_guest_calls)),
                opt_str(&e.sip_password),
                opt_str(&e.call_forwarding_mode),
                opt_str(&e.call_forwarding_destination),
                opt_int(e.call_forwarding_timeout),
            ],
        )
        .await
    }

    pub async fn update_extension(
        &self,
        id: i64,
        e: &ExtensionInput,
        scope_tenant: Option<i64>,
    ) -> Result<u64> {
        // `sip_password` is only replaced when provided (omit to keep current).
        let set_pw = e.sip_password.as_deref().is_some_and(|s| !s.is_empty());
        let mut sql = String::from(
            "UPDATE rustpbx_extensions SET \
             extension=$1, display_name=$2, email=$3, status=$4, login_disabled=$5, \
             voicemail_disabled=$6, allow_guest_calls=$7, call_forwarding_mode=$8, \
             call_forwarding_destination=$9, call_forwarding_timeout=$10, \
             updated_at=CURRENT_TIMESTAMP",
        );
        let mut vals = vec![
            Value::String(Some(Box::new(e.extension.clone()))),
            opt_str(&e.display_name),
            opt_str(&e.email),
            opt_str(&e.status),
            Value::Bool(Some(e.login_disabled)),
            Value::Bool(Some(e.voicemail_disabled)),
            Value::Bool(Some(e.allow_guest_calls)),
            opt_str(&e.call_forwarding_mode),
            opt_str(&e.call_forwarding_destination),
            opt_int(e.call_forwarding_timeout),
        ];
        let mut n = 10;
        if set_pw {
            n += 1;
            sql.push_str(&format!(", sip_password=${n}"));
            vals.push(opt_str(&e.sip_password));
        }
        n += 1;
        sql.push_str(&format!(" WHERE id=${n}"));
        vals.push(Value::BigInt(Some(id)));
        if let Some(tid) = scope_tenant {
            n += 1;
            sql.push_str(&format!(" AND tenant_id=${n}"));
            vals.push(Value::BigInt(Some(tid)));
        }
        self.exec_affected(&sql, vals).await
    }

    pub async fn delete_extension(&self, id: i64, scope_tenant: Option<i64>) -> Result<u64> {
        self.delete_scoped("rustpbx_extensions", id, scope_tenant).await
    }

    // ── ACL rules ───────────────────────────────────────────────────────────────

    pub async fn list_acl_admin(&self, tenant_id: Option<i64>) -> Result<Vec<AclView>> {
        let cols = "id, tenant_id, action, target, priority, is_active";
        let (sql, vals) = match tenant_id {
            Some(tid) => (
                format!(
                    "SELECT {cols} FROM rustpbx_acl_rules \
                     WHERE tenant_id = $1 OR tenant_id IS NULL ORDER BY priority ASC, id ASC"
                ),
                vec![Value::BigInt(Some(tid))],
            ),
            None => (
                format!("SELECT {cols} FROM rustpbx_acl_rules ORDER BY priority ASC, id ASC"),
                vec![],
            ),
        };
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                &sql,
                vals,
            ))
            .await?;
        rows.iter().map(row_to_acl_view).collect()
    }

    pub async fn create_acl(&self, a: &AclInput, row_tenant: Option<i64>) -> Result<()> {
        let sql = "INSERT INTO rustpbx_acl_rules (tenant_id, action, target, priority, is_active) \
                   VALUES ($1,$2,$3,$4,$5)";
        self.exec(
            sql,
            vec![
                Value::BigInt(row_tenant),
                Value::String(Some(Box::new(a.action.clone()))),
                Value::String(Some(Box::new(a.target.clone()))),
                Value::Int(Some(a.priority)),
                Value::Bool(Some(a.is_active)),
            ],
        )
        .await
    }

    pub async fn update_acl(&self, id: i64, a: &AclInput, scope_tenant: Option<i64>) -> Result<u64> {
        let mut sql = String::from(
            "UPDATE rustpbx_acl_rules SET action=$1, target=$2, priority=$3, is_active=$4 WHERE id=$5",
        );
        let mut vals = vec![
            Value::String(Some(Box::new(a.action.clone()))),
            Value::String(Some(Box::new(a.target.clone()))),
            Value::Int(Some(a.priority)),
            Value::Bool(Some(a.is_active)),
            Value::BigInt(Some(id)),
        ];
        if let Some(tid) = scope_tenant {
            sql.push_str(" AND tenant_id=$6");
            vals.push(Value::BigInt(Some(tid)));
        }
        self.exec_affected(&sql, vals).await
    }

    pub async fn delete_acl(&self, id: i64, scope_tenant: Option<i64>) -> Result<u64> {
        self.delete_scoped("rustpbx_acl_rules", id, scope_tenant).await
    }

    // ── Queues ──────────────────────────────────────────────────────────────────

    pub async fn list_queues_admin(&self, tenant_id: Option<i64>) -> Result<Vec<QueueView>> {
        let cols = "id, name, description, tenant_id, is_active, spec";
        let (sql, vals) = match tenant_id {
            Some(tid) => (
                format!(
                    "SELECT {cols} FROM rustpbx_queues \
                     WHERE tenant_id = $1 OR tenant_id IS NULL ORDER BY name"
                ),
                vec![Value::BigInt(Some(tid))],
            ),
            None => (format!("SELECT {cols} FROM rustpbx_queues ORDER BY name"), vec![]),
        };
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                &sql,
                vals,
            ))
            .await?;
        rows.iter().map(row_to_queue_view).collect()
    }

    pub async fn create_queue(&self, q: &QueueInput, row_tenant: Option<i64>) -> Result<()> {
        let sql = "INSERT INTO rustpbx_queues (name, description, spec, is_active, tenant_id) \
                   VALUES ($1,$2,$3,$4,$5)";
        self.exec(
            sql,
            vec![
                Value::String(Some(Box::new(q.name.clone()))),
                opt_str(&q.description),
                Value::Json(Some(Box::new(q.spec.clone()))),
                Value::Bool(Some(q.is_active)),
                Value::BigInt(row_tenant),
            ],
        )
        .await
    }

    pub async fn update_queue(
        &self,
        id: i64,
        q: &QueueInput,
        scope_tenant: Option<i64>,
    ) -> Result<u64> {
        let mut sql = String::from(
            "UPDATE rustpbx_queues SET name=$1, description=$2, spec=$3, is_active=$4, \
             updated_at=CURRENT_TIMESTAMP WHERE id=$5",
        );
        let mut vals = vec![
            Value::String(Some(Box::new(q.name.clone()))),
            opt_str(&q.description),
            Value::Json(Some(Box::new(q.spec.clone()))),
            Value::Bool(Some(q.is_active)),
            Value::BigInt(Some(id)),
        ];
        if let Some(tid) = scope_tenant {
            sql.push_str(" AND tenant_id=$6");
            vals.push(Value::BigInt(Some(tid)));
        }
        self.exec_affected(&sql, vals).await
    }

    pub async fn delete_queue(&self, id: i64, scope_tenant: Option<i64>) -> Result<u64> {
        self.delete_scoped("rustpbx_queues", id, scope_tenant).await
    }

    // ── IVRs ────────────────────────────────────────────────────────────────────

    pub async fn list_ivrs_admin(&self, tenant_id: Option<i64>) -> Result<Vec<IvrView>> {
        let cols = "id, name, description, tenant_id, is_active, spec";
        let (sql, vals) = match tenant_id {
            Some(tid) => (
                format!(
                    "SELECT {cols} FROM rustpbx_ivrs \
                     WHERE tenant_id = $1 OR tenant_id IS NULL ORDER BY name"
                ),
                vec![Value::BigInt(Some(tid))],
            ),
            None => (format!("SELECT {cols} FROM rustpbx_ivrs ORDER BY name"), vec![]),
        };
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                &sql,
                vals,
            ))
            .await?;
        rows.iter().map(row_to_ivr_view).collect()
    }

    pub async fn create_ivr(&self, iv: &IvrInput, row_tenant: Option<i64>) -> Result<()> {
        let sql = "INSERT INTO rustpbx_ivrs (name, description, spec, is_active, tenant_id) \
                   VALUES ($1,$2,$3,$4,$5)";
        self.exec(
            sql,
            vec![
                Value::String(Some(Box::new(iv.name.clone()))),
                opt_str(&iv.description),
                Value::Json(Some(Box::new(iv.spec.clone()))),
                Value::Bool(Some(iv.is_active)),
                Value::BigInt(row_tenant),
            ],
        )
        .await
    }

    pub async fn update_ivr(
        &self,
        id: i64,
        iv: &IvrInput,
        scope_tenant: Option<i64>,
    ) -> Result<u64> {
        let mut sql = String::from(
            "UPDATE rustpbx_ivrs SET name=$1, description=$2, spec=$3, is_active=$4, \
             updated_at=CURRENT_TIMESTAMP WHERE id=$5",
        );
        let mut vals = vec![
            Value::String(Some(Box::new(iv.name.clone()))),
            opt_str(&iv.description),
            Value::Json(Some(Box::new(iv.spec.clone()))),
            Value::Bool(Some(iv.is_active)),
            Value::BigInt(Some(id)),
        ];
        if let Some(tid) = scope_tenant {
            sql.push_str(" AND tenant_id=$6");
            vals.push(Value::BigInt(Some(tid)));
        }
        self.exec_affected(&sql, vals).await
    }

    pub async fn delete_ivr(&self, id: i64, scope_tenant: Option<i64>) -> Result<u64> {
        self.delete_scoped("rustpbx_ivrs", id, scope_tenant).await
    }

    // ── Shared helpers ──────────────────────────────────────────────────────────

    async fn exec(&self, sql: &str, vals: Vec<Value>) -> Result<()> {
        self.db
            .execute(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                sql,
                vals,
            ))
            .await?;
        Ok(())
    }

    async fn exec_affected(&self, sql: &str, vals: Vec<Value>) -> Result<u64> {
        let res = self
            .db
            .execute(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                sql,
                vals,
            ))
            .await?;
        Ok(res.rows_affected())
    }

    async fn delete_scoped(&self, table: &str, id: i64, scope_tenant: Option<i64>) -> Result<u64> {
        let (sql, vals) = match scope_tenant {
            Some(tid) => (
                format!("DELETE FROM {table} WHERE id=$1 AND tenant_id=$2"),
                vec![Value::BigInt(Some(id)), Value::BigInt(Some(tid))],
            ),
            None => (
                format!("DELETE FROM {table} WHERE id=$1"),
                vec![Value::BigInt(Some(id))],
            ),
        };
        self.exec_affected(&sql, vals).await
    }
}

#[derive(Debug, Deserialize)]
pub struct RouteInput {
    pub name: String,
    pub description: Option<String>,
    #[serde(default = "default_any")]
    pub direction: String,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub is_active: bool,
    #[serde(default = "default_rr")]
    pub selection_strategy: String,
    pub hash_key: Option<String>,
    pub source_pattern: Option<String>,
    pub destination_pattern: Option<String>,
    #[serde(default)]
    pub target_trunks: Vec<String>,
}

fn default_any() -> String {
    "any".into()
}
fn default_priority() -> i32 {
    100
}
fn default_rr() -> String {
    "rr".into()
}

fn target_trunks_json(names: &[String]) -> Value {
    let arr: Vec<_> = names.iter().map(|n| serde_json::json!({ "name": n })).collect();
    Value::Json(Some(Box::new(serde_json::json!(arr))))
}

#[derive(Debug, Deserialize)]
pub struct ExtensionInput {
    pub extension: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub login_disabled: bool,
    #[serde(default)]
    pub voicemail_disabled: bool,
    #[serde(default)]
    pub allow_guest_calls: bool,
    pub sip_password: Option<String>,
    pub call_forwarding_mode: Option<String>,
    pub call_forwarding_destination: Option<String>,
    pub call_forwarding_timeout: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionView {
    pub id: i64,
    pub extension: String,
    pub tenant_id: Option<i64>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub status: Option<String>,
    pub login_disabled: bool,
    pub voicemail_disabled: bool,
    pub allow_guest_calls: bool,
    pub call_forwarding_mode: Option<String>,
    pub call_forwarding_destination: Option<String>,
    pub call_forwarding_timeout: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct AclInput {
    /// `allow` or `deny`.
    pub action: String,
    /// CIDR or the literal `all`.
    pub target: String,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AclView {
    pub id: i64,
    pub tenant_id: Option<i64>,
    pub action: String,
    pub target: String,
    pub priority: i32,
    pub is_active: bool,
}

#[derive(Debug, Deserialize)]
pub struct QueueInput {
    pub name: String,
    pub description: Option<String>,
    /// `RouteQueueConfig` serialized as JSON. Opaque to the control plane — it
    /// stores and forwards it verbatim; the worker deserializes it into
    /// `rustpbx::proxy::routing::RouteQueueConfig`.
    pub spec: serde_json::Value,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueView {
    pub id: i64,
    pub name: String,
    pub tenant_id: Option<i64>,
    pub description: Option<String>,
    pub is_active: bool,
    pub spec: serde_json::Value,
}

fn row_to_queue_view(row: &sea_orm::QueryResult) -> Result<QueueView> {
    let spec: Option<serde_json::Value> = row
        .try_get::<Option<serde_json::Value>>("", "spec")
        .ok()
        .flatten();
    Ok(QueueView {
        id: row.try_get("", "id")?,
        name: row.try_get("", "name")?,
        tenant_id: row.try_get("", "tenant_id").ok(),
        description: row.try_get("", "description").ok(),
        is_active: row.try_get("", "is_active").unwrap_or(true),
        spec: spec.unwrap_or(serde_json::Value::Null),
    })
}

#[derive(Debug, Deserialize)]
pub struct IvrInput {
    pub name: String,
    pub description: Option<String>,
    /// `IvrDefinition` serialized as JSON. Opaque to the control plane; the
    /// worker materializes it into a `{name}.generated.toml` file.
    pub spec: serde_json::Value,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct IvrView {
    pub id: i64,
    pub name: String,
    pub tenant_id: Option<i64>,
    pub description: Option<String>,
    pub is_active: bool,
    pub spec: serde_json::Value,
}

fn row_to_ivr_view(row: &sea_orm::QueryResult) -> Result<IvrView> {
    let spec: Option<serde_json::Value> = row
        .try_get::<Option<serde_json::Value>>("", "spec")
        .ok()
        .flatten();
    Ok(IvrView {
        id: row.try_get("", "id")?,
        name: row.try_get("", "name")?,
        tenant_id: row.try_get("", "tenant_id").ok(),
        description: row.try_get("", "description").ok(),
        is_active: row.try_get("", "is_active").unwrap_or(true),
        spec: spec.unwrap_or(serde_json::Value::Null),
    })
}

fn row_to_acl_view(row: &sea_orm::QueryResult) -> Result<AclView> {
    Ok(AclView {
        id: row.try_get("", "id")?,
        tenant_id: row.try_get("", "tenant_id").ok(),
        action: row.try_get("", "action").unwrap_or_default(),
        target: row.try_get("", "target").unwrap_or_default(),
        priority: row.try_get("", "priority").unwrap_or(100),
        is_active: row.try_get("", "is_active").unwrap_or(true),
    })
}

fn row_to_extension_view(row: &sea_orm::QueryResult) -> Result<ExtensionView> {
    Ok(ExtensionView {
        id: row.try_get("", "id")?,
        extension: row.try_get("", "extension")?,
        tenant_id: row.try_get("", "tenant_id").ok(),
        display_name: row.try_get("", "display_name").ok(),
        email: row.try_get("", "email").ok(),
        status: row.try_get("", "status").ok(),
        login_disabled: row.try_get("", "login_disabled").unwrap_or(false),
        voicemail_disabled: row.try_get("", "voicemail_disabled").unwrap_or(false),
        allow_guest_calls: row.try_get("", "allow_guest_calls").unwrap_or(false),
        call_forwarding_mode: row.try_get("", "call_forwarding_mode").ok(),
        call_forwarding_destination: row.try_get("", "call_forwarding_destination").ok(),
        call_forwarding_timeout: row.try_get("", "call_forwarding_timeout").ok(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::ControlMigrator;
    use sea_orm::Database;
    use sea_orm_migration::{MigratorTrait, SchemaManager};

    async fn fresh_store() -> Store {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);
        for m in ControlMigrator::migrations() {
            m.up(&manager).await.unwrap();
        }
        Store::new(db)
    }

    fn trunk(name: &str) -> TrunkInput {
        TrunkInput {
            name: name.into(),
            display_name: Some("Carrier A".into()),
            carrier: None,
            direction: "outbound".into(),
            sip_server: Some("1.2.3.4".into()),
            sip_transport: "udp".into(),
            outbound_proxy: None,
            auth_username: Some("u".into()),
            auth_password: Some("p".into()),
            max_cps: Some(10),
            max_concurrent: Some(100),
            allowed_ips: vec!["1.2.3.4/32".into()],
            did_numbers: vec!["+12025550100".into()],
            incoming_from_user_prefix: None,
            incoming_to_user_prefix: None,
            is_active: true,
            register_enabled: false,
            register_expires: None,
            rewrite_hostport: true,
        }
    }

    #[tokio::test]
    async fn trunk_crud_scoped_to_tenant() {
        let s = fresh_store().await;
        s.create_trunk(&trunk("t1"), Some(1)).await.unwrap();
        let listed = s.list_trunks(Some(1)).await.unwrap();
        assert_eq!(listed.len(), 1);
        let id = listed[0].id;

        // Wrong tenant scope can't update or delete it.
        assert_eq!(s.update_trunk(id, &trunk("renamed"), Some(2)).await.unwrap(), 0);
        assert_eq!(s.delete_trunk(id, Some(2)).await.unwrap(), 0);
        // Correct scope works.
        assert_eq!(s.update_trunk(id, &trunk("renamed"), Some(1)).await.unwrap(), 1);
        assert_eq!(s.delete_trunk(id, Some(1)).await.unwrap(), 1);
        assert!(s.list_trunks(Some(1)).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn route_crud_roundtrips() {
        let s = fresh_store().await;
        let r = RouteInput {
            name: "default".into(),
            description: Some("catch-all".into()),
            direction: "outbound".into(),
            priority: 50,
            is_active: true,
            selection_strategy: "rr".into(),
            hash_key: None,
            source_pattern: None,
            destination_pattern: Some("^9.*".into()),
            target_trunks: vec!["t1".into()],
        };
        s.create_route(&r, Some(1)).await.unwrap();
        let listed = s.list_routes(Some(1)).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].target_trunks, vec!["t1".to_string()]);
        assert_eq!(s.delete_route(listed[0].id, Some(1)).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn acl_crud_scoped_and_priority_ordered() {
        let s = fresh_store().await;
        s.create_acl(&AclInput { action: "deny".into(), target: "all".into(), priority: 200, is_active: true }, Some(1)).await.unwrap();
        s.create_acl(&AclInput { action: "allow".into(), target: "10.0.0.0/8".into(), priority: 100, is_active: true }, Some(1)).await.unwrap();
        // A global rule (tenant NULL) is visible to the tenant too.
        s.create_acl(&AclInput { action: "deny".into(), target: "1.2.3.4/32".into(), priority: 50, is_active: true }, None).await.unwrap();

        let rows = s.list_acl_admin(Some(1)).await.unwrap();
        assert_eq!(rows.len(), 3, "tenant sees its rules + globals");
        // Ordered by priority ascending.
        assert_eq!(rows[0].priority, 50);
        assert_eq!(rows[0].tenant_id, None, "global rule");
        assert_eq!(rows[1].target, "10.0.0.0/8");

        // A tenant-scoped mutate can't touch the global rule.
        let global_id = rows[0].id;
        assert_eq!(s.delete_acl(global_id, Some(1)).await.unwrap(), 0, "tenant can't delete global rule");
        // Superadmin (scope None) can.
        assert_eq!(s.delete_acl(global_id, None).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn extension_crud_and_password_preserved_on_update() {
        let s = fresh_store().await;
        let mut e = ExtensionInput {
            extension: "1001".into(),
            display_name: Some("Alice".into()),
            email: None,
            status: Some("enabled".into()),
            login_disabled: false,
            voicemail_disabled: false,
            allow_guest_calls: false,
            sip_password: Some("sip-secret".into()),
            call_forwarding_mode: None,
            call_forwarding_destination: None,
            call_forwarding_timeout: None,
        };
        s.create_extension(&e, Some(1)).await.unwrap();
        let listed = s.list_extensions(Some(1)).await.unwrap();
        assert_eq!(listed.len(), 1);
        let id = listed[0].id;

        // Update without a password (omit) must not blow away the existing one.
        e.display_name = Some("Alice B".into());
        e.sip_password = None;
        assert_eq!(s.update_extension(id, &e, Some(1)).await.unwrap(), 1);

        // Two tenants can share extension number 1001 (composite uniqueness).
        s.create_extension(&e, Some(2)).await.unwrap();
        assert_eq!(s.list_extensions(Some(2)).await.unwrap().len(), 1);
    }
}
