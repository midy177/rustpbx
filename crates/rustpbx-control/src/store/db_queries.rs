use crate::{grpc::proto::control::TrunkConfigProto, store::Store};
use anyhow::Result;
use sea_orm::ConnectionTrait;

/// Filters + paging for the admin-console CDR listing. All filters are optional
/// and combine with AND; empty/blank strings are ignored.
#[derive(Debug, Default, Clone)]
pub struct CdrListOpts {
    /// Restrict to a tenant (None → all tenants, superadmin scope).
    pub tenant_id: Option<i64>,
    /// Substring match against `from_number` OR `to_number`.
    pub search: Option<String>,
    /// Exact `status` match (e.g. "answered", "no_answer", "busy").
    pub status: Option<String>,
    /// Exact `direction` match ("inbound" / "outbound").
    pub direction: Option<String>,
    /// `started_at >= since`.
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    /// `started_at <= until`.
    pub until: Option<chrono::DateTime<chrono::Utc>>,
    /// Page size (clamped to 1..=500).
    pub limit: u64,
    /// Rows to skip.
    pub offset: u64,
}

impl Store {
    /// Persist a CDR report from a Worker into rustpbx_call_records.
    pub async fn persist_cdr(
        &self,
        rec: &crate::grpc::proto::control::CallRecordReport,
    ) -> Result<()> {
        use chrono::{TimeZone, Utc};
        use sea_orm::{Statement, Value};

        let start = Utc.timestamp_millis_opt(rec.start_time_unix_ms).single();
        let end = Utc.timestamp_millis_opt(rec.end_time_unix_ms).single();

        // Column names must match the canonical schema owned by the main binary
        // (`src/models/call_record.rs`): `started_at` / `ended_at`, no
        // `answer_time` / `sip_trunk_name` columns. The Worker reports a trunk
        // *name* (not the main schema's `sip_trunk_id`) and an answer time that
        // the base schema doesn't model, so both are folded into `metadata`.
        // tenant_id is written to its own column (for clean per-tenant CDR
        // filtering) as well as kept in metadata. 0 / unset = no tenant.
        let tenant_id = rec.tenant_id.filter(|&t| t != 0);

        let sql = "INSERT INTO rustpbx_call_records \
            (call_id, from_number, to_number, direction, status, \
             started_at, ended_at, duration_secs, tenant_id, recording_url, metadata) \
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) \
            ON CONFLICT (call_id) DO NOTHING";

        let meta = serde_json::json!({
            "worker_id": rec.worker_id,
            "edge_id": rec.edge_id,
            "recording_path": rec.recording_path,
            "hangup_cause": rec.hangup_cause,
            "tenant_id": rec.tenant_id,
            "sip_trunk_name": rec.trunk_name,
            "answer_time_unix_ms": (rec.answer_time_unix_ms > 0).then_some(rec.answer_time_unix_ms),
        });
        let recording_url = rec.recording_path.clone().filter(|s| !s.is_empty());

        self.db
            .execute(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                sql,
                vec![
                    Value::String(Some(Box::new(rec.call_id.clone()))),
                    Value::String(Some(Box::new(rec.caller.clone()))),
                    Value::String(Some(Box::new(rec.callee.clone()))),
                    Value::String(Some(Box::new(rec.direction.clone()))),
                    Value::String(Some(Box::new(rec.status.clone()))),
                    Value::ChronoDateTimeUtc(start.map(Box::new)),
                    Value::ChronoDateTimeUtc(end.map(Box::new)),
                    Value::Int(Some(rec.duration_secs)),
                    Value::BigInt(tenant_id),
                    Value::String(recording_url.map(Box::new)),
                    Value::Json(Some(Box::new(meta))),
                ],
            ))
            .await?;

        Ok(())
    }

    /// Load all active trunks from DB, optionally filtered by tenant_id.
    pub async fn load_trunks(&self, tenant_id: Option<i64>) -> Result<Vec<TrunkConfigProto>> {
        // We read directly from the shared rustpbx sip_trunk table.
        // The table name is rustpbx_sip_trunks — we use a raw query
        // so we don't need to re-declare the full sea-orm entity here.
        use sea_orm::ConnectionTrait;
        use sea_orm::Statement;

        let (sql, values) = if let Some(tid) = tenant_id {
            (
                "SELECT id, name, sip_server, outbound_proxy, sip_transport, \
                 auth_username, auth_password, direction, is_active, \
                 register_enabled, register_expires, register_extra_headers, \
                 rewrite_hostport, allowed_ips, did_numbers, \
                 incoming_from_user_prefix, incoming_to_user_prefix, \
                 max_cps, max_concurrent, metadata \
                 FROM rustpbx_sip_trunks \
                 WHERE is_active = TRUE AND (tenant_id = $1 OR tenant_id IS NULL) \
                 AND (tenant_id IS NULL OR tenant_id NOT IN \
                 (SELECT id FROM rustpbx_tenants WHERE status <> 'active')) \
                 ORDER BY name",
                vec![sea_orm::Value::BigInt(Some(tid))],
            )
        } else {
            (
                "SELECT id, name, sip_server, outbound_proxy, sip_transport, \
                 auth_username, auth_password, direction, is_active, \
                 register_enabled, register_expires, register_extra_headers, \
                 rewrite_hostport, allowed_ips, did_numbers, \
                 incoming_from_user_prefix, incoming_to_user_prefix, \
                 max_cps, max_concurrent, metadata \
                 FROM rustpbx_sip_trunks \
                 WHERE is_active = TRUE \
                 AND (tenant_id IS NULL OR tenant_id NOT IN \
                 (SELECT id FROM rustpbx_tenants WHERE status <> 'active')) \
                 ORDER BY name",
                vec![],
            )
        };

        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                sql,
                values,
            ))
            .await?;

        let mut trunks = Vec::with_capacity(rows.len());
        for row in rows {
            let trunk = row_to_trunk_proto(&row)?;
            trunks.push(trunk);
        }
        Ok(trunks)
    }

    /// Load active routes from DB, optionally filtered by tenant_id.
    pub async fn load_routes(
        &self,
        tenant_id: Option<i64>,
    ) -> Result<Vec<crate::grpc::proto::control::RouteRuleProto>> {
        use sea_orm::{ConnectionTrait, Statement};

        let (sql, values) = if let Some(tid) = tenant_id {
            (
                "SELECT id, name, description, direction, priority, \
                 source_pattern, destination_pattern, header_filters, \
                 rewrite_rules, target_trunks, selection_strategy, hash_key, \
                 source_trunk_id, metadata \
                 FROM rustpbx_routes \
                 WHERE is_active = TRUE AND (tenant_id = $1 OR tenant_id IS NULL) \
                 AND (tenant_id IS NULL OR tenant_id NOT IN \
                 (SELECT id FROM rustpbx_tenants WHERE status <> 'active')) \
                 ORDER BY priority ASC",
                vec![sea_orm::Value::BigInt(Some(tid))],
            )
        } else {
            (
                "SELECT id, name, description, direction, priority, \
                 source_pattern, destination_pattern, header_filters, \
                 rewrite_rules, target_trunks, selection_strategy, hash_key, \
                 source_trunk_id, metadata \
                 FROM rustpbx_routes \
                 WHERE is_active = TRUE \
                 AND (tenant_id IS NULL OR tenant_id NOT IN \
                 (SELECT id FROM rustpbx_tenants WHERE status <> 'active')) \
                 ORDER BY priority ASC",
                vec![],
            )
        };

        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                sql,
                values,
            ))
            .await?;

        let mut routes = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(rule) = row_to_route_proto(&row)? {
                routes.push(rule);
            }
        }
        Ok(routes)
    }

    // ── Admin console listings (include inactive; secret-safe) ────────────────

    /// List trunks for the admin console, optionally scoped to a tenant
    /// (matches the tenant plus global/NULL-tenant rows).
    pub async fn list_trunks(
        &self,
        tenant_id: Option<i64>,
    ) -> Result<Vec<crate::store::TrunkView>> {
        use sea_orm::Statement;

        let cols = "id, name, sip_server, outbound_proxy, sip_transport, \
            auth_username, direction, register_enabled, is_active, \
            allowed_ips, did_numbers, max_concurrent, tenant_id";
        let (sql, values) = match tenant_id {
            Some(tid) => (
                format!(
                    "SELECT {cols} FROM rustpbx_sip_trunks \
                     WHERE tenant_id = $1 OR tenant_id IS NULL ORDER BY name"
                ),
                vec![sea_orm::Value::BigInt(Some(tid))],
            ),
            None => (
                format!("SELECT {cols} FROM rustpbx_sip_trunks ORDER BY name"),
                vec![],
            ),
        };

        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                &sql,
                values,
            ))
            .await?;
        rows.iter().map(row_to_trunk_view).collect()
    }

    /// List routing rules for the admin console, optionally scoped to a tenant.
    pub async fn list_routes(
        &self,
        tenant_id: Option<i64>,
    ) -> Result<Vec<crate::store::RouteView>> {
        use sea_orm::Statement;

        let cols = "id, name, description, priority, direction, \
            source_pattern, destination_pattern, target_trunks, \
            is_active, tenant_id";
        let (sql, values) = match tenant_id {
            Some(tid) => (
                format!(
                    "SELECT {cols} FROM rustpbx_routes \
                     WHERE tenant_id = $1 OR tenant_id IS NULL ORDER BY priority ASC"
                ),
                vec![sea_orm::Value::BigInt(Some(tid))],
            ),
            None => (
                format!("SELECT {cols} FROM rustpbx_routes ORDER BY priority ASC"),
                vec![],
            ),
        };

        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                &sql,
                values,
            ))
            .await?;
        rows.iter().map(row_to_route_view).collect()
    }

    /// The tenant's `max_concurrent_calls` limit, or `None` when unset
    /// (unlimited) or the tenant doesn't exist. Used to enforce concurrency.
    pub async fn tenant_max_concurrent(&self, tenant_id: i64) -> Result<Option<u32>> {
        use sea_orm::Statement;
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                "SELECT max_concurrent_calls FROM rustpbx_tenants WHERE id = $1",
                vec![sea_orm::Value::BigInt(Some(tenant_id))],
            ))
            .await?;
        Ok(row
            .and_then(|r| r.try_get::<Option<i32>>("", "max_concurrent_calls").ok().flatten())
            .filter(|&v| v > 0)
            .map(|v| v as u32))
    }

    /// Paged + filtered CDR listing for the admin console. Returns the page of
    /// rows (newest first) plus the total matching count for pagination.
    pub async fn list_call_records_paged(
        &self,
        opts: &CdrListOpts,
    ) -> Result<(Vec<crate::store::CdrView>, u64)> {
        use sea_orm::{Statement, Value};

        // Build the shared WHERE clause + positional values once; reuse for the
        // COUNT and the page query. Placeholders are $1.. (sea-orm rewrites them
        // to the backend's style); LIMIT/OFFSET are u64 so they inline safely.
        let mut conds: Vec<String> = Vec::new();
        let mut vals: Vec<Value> = Vec::new();
        if let Some(tid) = opts.tenant_id {
            vals.push(Value::BigInt(Some(tid)));
            conds.push(format!("tenant_id = ${}", vals.len()));
        }
        if let Some(s) = opts
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let like = format!("%{s}%");
            vals.push(Value::String(Some(Box::new(like.clone()))));
            let a = vals.len();
            vals.push(Value::String(Some(Box::new(like))));
            let b = vals.len();
            conds.push(format!("(from_number LIKE ${a} OR to_number LIKE ${b})"));
        }
        if let Some(st) = opts
            .status
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            vals.push(Value::String(Some(Box::new(st.to_string()))));
            conds.push(format!("status = ${}", vals.len()));
        }
        if let Some(d) = opts
            .direction
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            vals.push(Value::String(Some(Box::new(d.to_string()))));
            conds.push(format!("direction = ${}", vals.len()));
        }
        if let Some(since) = opts.since {
            vals.push(Value::ChronoDateTimeUtc(Some(Box::new(since))));
            conds.push(format!("started_at >= ${}", vals.len()));
        }
        if let Some(until) = opts.until {
            vals.push(Value::ChronoDateTimeUtc(Some(Box::new(until))));
            conds.push(format!("started_at <= ${}", vals.len()));
        }
        let where_clause = if conds.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conds.join(" AND "))
        };
        let backend = self.db.get_database_backend();

        // Total count (filters only, no LIMIT/OFFSET).
        let count_sql = format!("SELECT COUNT(*) AS cnt FROM rustpbx_call_records {where_clause}");
        let total: u64 = self
            .db
            .query_one(Statement::from_sql_and_values(
                backend,
                &count_sql,
                vals.clone(),
            ))
            .await?
            .and_then(|r| r.try_get::<i64>("", "cnt").ok())
            .unwrap_or(0)
            .max(0) as u64;

        let limit = opts.limit.clamp(1, 500);
        let cols = "id, call_id, tenant_id, direction, status, from_number, to_number, \
            started_at, ended_at, duration_secs, recording_url";
        let page_sql = format!(
            "SELECT {cols} FROM rustpbx_call_records {where_clause} \
             ORDER BY started_at DESC LIMIT {limit} OFFSET {}",
            opts.offset
        );
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(backend, &page_sql, vals))
            .await?;
        let records: Vec<_> = rows
            .iter()
            .map(row_to_cdr_view)
            .collect::<Result<_>>()?;
        Ok((records, total))
    }

    /// Load active ACL rules as `"<action> <target>"` strings, optionally scoped
    /// to a tenant (matches the tenant plus global/NULL-tenant rows), ordered by
    /// priority ascending. Format matches the main binary's `acl_rules` config.
    pub async fn load_acl_rules(&self, tenant_id: Option<i64>) -> Result<Vec<String>> {
        use sea_orm::{ConnectionTrait, Statement};

        let (sql, values) = match tenant_id {
            Some(tid) => (
                "SELECT action, target FROM rustpbx_acl_rules \
                 WHERE is_active = TRUE AND (tenant_id = $1 OR tenant_id IS NULL) \
                 AND (tenant_id IS NULL OR tenant_id NOT IN \
                 (SELECT id FROM rustpbx_tenants WHERE status <> 'active')) \
                 ORDER BY priority ASC, id ASC",
                vec![sea_orm::Value::BigInt(Some(tid))],
            ),
            None => (
                "SELECT action, target FROM rustpbx_acl_rules \
                 WHERE is_active = TRUE \
                 AND (tenant_id IS NULL OR tenant_id NOT IN \
                 (SELECT id FROM rustpbx_tenants WHERE status <> 'active')) \
                 ORDER BY priority ASC, id ASC",
                vec![],
            ),
        };

        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                sql,
                values,
            ))
            .await?;

        let mut rules = Vec::with_capacity(rows.len());
        for row in rows {
            let action: String = row.try_get("", "action")?;
            let target: String = row.try_get("", "target")?;
            rules.push(format!("{action} {target}"));
        }
        Ok(rules)
    }

    /// Active queues for distribution to workers (gRPC GetQueues). Tenant-scoped
    /// (own + global), excluding queues of suspended tenants. Returns
    /// `(name, spec_json)` pairs — spec is the opaque `RouteQueueConfig` JSON
    /// the worker deserializes.
    pub async fn load_queues(&self, tenant_id: Option<i64>) -> Result<Vec<(String, String)>> {
        use sea_orm::{ConnectionTrait, Statement};

        let (sql, values) = match tenant_id {
            Some(tid) => (
                "SELECT name, spec FROM rustpbx_queues \
                 WHERE is_active = TRUE AND (tenant_id = $1 OR tenant_id IS NULL) \
                 AND (tenant_id IS NULL OR tenant_id NOT IN \
                 (SELECT id FROM rustpbx_tenants WHERE status <> 'active')) \
                 ORDER BY name",
                vec![sea_orm::Value::BigInt(Some(tid))],
            ),
            None => (
                "SELECT name, spec FROM rustpbx_queues \
                 WHERE is_active = TRUE \
                 AND (tenant_id IS NULL OR tenant_id NOT IN \
                 (SELECT id FROM rustpbx_tenants WHERE status <> 'active')) \
                 ORDER BY name",
                vec![],
            ),
        };

        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                sql,
                values,
            ))
            .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.try_get("", "name")?;
            let spec: Option<serde_json::Value> = row
                .try_get::<Option<serde_json::Value>>("", "spec")
                .ok()
                .flatten();
            let spec_json = match spec {
                Some(v) => serde_json::to_string(&v).unwrap_or_else(|_| "{}".into()),
                None => "{}".into(),
            };
            out.push((name, spec_json));
        }
        Ok(out)
    }

    /// Active IVRs for distribution to workers (gRPC GetIvrs). Tenant-scoped,
    /// excluding suspended tenants. Returns `(name, spec_json)` pairs — spec is
    /// the opaque `IvrDefinition` JSON the worker writes to a TOML file.
    pub async fn load_ivrs(&self, tenant_id: Option<i64>) -> Result<Vec<(String, String)>> {
        use sea_orm::{ConnectionTrait, Statement};
        let (sql, values) = match tenant_id {
            Some(tid) => (
                "SELECT name, spec FROM rustpbx_ivrs \
                 WHERE is_active = TRUE AND (tenant_id = $1 OR tenant_id IS NULL) \
                 AND (tenant_id IS NULL OR tenant_id NOT IN \
                 (SELECT id FROM rustpbx_tenants WHERE status <> 'active')) \
                 ORDER BY name",
                vec![sea_orm::Value::BigInt(Some(tid))],
            ),
            None => (
                "SELECT name, spec FROM rustpbx_ivrs \
                 WHERE is_active = TRUE \
                 AND (tenant_id IS NULL OR tenant_id NOT IN \
                 (SELECT id FROM rustpbx_tenants WHERE status <> 'active')) \
                 ORDER BY name",
                vec![],
            ),
        };
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                sql,
                values,
            ))
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.try_get("", "name")?;
            let spec: Option<serde_json::Value> = row
                .try_get::<Option<serde_json::Value>>("", "spec")
                .ok()
                .flatten();
            let spec_json = match spec {
                Some(v) => serde_json::to_string(&v).unwrap_or_else(|_| "{}".into()),
                None => "{}".into(),
            };
            out.push((name, spec_json));
        }
        Ok(out)
    }
}

fn row_to_trunk_view(row: &sea_orm::QueryResult) -> Result<crate::store::TrunkView> {
    let sip_server: Option<String> = row.try_get("", "sip_server").ok();
    let outbound_proxy: Option<String> = row.try_get("", "outbound_proxy").ok();
    let allowed_ips_json: Option<serde_json::Value> = row
        .try_get::<Option<serde_json::Value>>("", "allowed_ips")
        .ok()
        .flatten();
    let did_json: Option<serde_json::Value> = row
        .try_get::<Option<serde_json::Value>>("", "did_numbers")
        .ok()
        .flatten();
    let auth_username: Option<String> = row.try_get("", "auth_username").ok();

    Ok(crate::store::TrunkView {
        id: row.try_get("", "id")?,
        name: row.try_get("", "name")?,
        dest: sip_server.or(outbound_proxy),
        transport: row
            .try_get("", "sip_transport")
            .unwrap_or_else(|_| "udp".into()),
        direction: row
            .try_get("", "direction")
            .unwrap_or_else(|_| "bidirectional".into()),
        has_auth: auth_username.as_deref().is_some_and(|u| !u.is_empty()),
        register_enabled: row.try_get("", "register_enabled").unwrap_or(false),
        is_active: row.try_get("", "is_active").unwrap_or(true),
        did_numbers: json_string_array(did_json),
        allowed_ips: json_string_array(allowed_ips_json),
        max_concurrent: row.try_get("", "max_concurrent").ok(),
        tenant_id: row.try_get("", "tenant_id").ok(),
    })
}

fn row_to_route_view(row: &sea_orm::QueryResult) -> Result<crate::store::RouteView> {
    let target_trunks_json: Option<serde_json::Value> = row
        .try_get::<Option<serde_json::Value>>("", "target_trunks")
        .ok()
        .flatten();
    let target_trunks = target_trunks_json
        .as_ref()
        .and_then(|v| {
            #[derive(serde::Deserialize)]
            struct TrunkRef {
                name: String,
            }
            serde_json::from_value::<Vec<TrunkRef>>(v.clone()).ok()
        })
        .map(|refs| refs.into_iter().map(|r| r.name).collect())
        .unwrap_or_default();

    Ok(crate::store::RouteView {
        id: row.try_get("", "id")?,
        name: row.try_get("", "name")?,
        description: row.try_get("", "description").ok(),
        priority: row.try_get("", "priority").unwrap_or(0),
        direction: row
            .try_get("", "direction")
            .unwrap_or_else(|_| "any".into()),
        source_pattern: row.try_get("", "source_pattern").ok(),
        destination_pattern: row.try_get("", "destination_pattern").ok(),
        target_trunks,
        is_active: row.try_get("", "is_active").unwrap_or(true),
        tenant_id: row.try_get("", "tenant_id").ok(),
    })
}

fn row_to_cdr_view(row: &sea_orm::QueryResult) -> Result<crate::store::CdrView> {
    let started_at: Option<chrono::DateTime<chrono::Utc>> = row.try_get("", "started_at").ok();
    let ended_at: Option<chrono::DateTime<chrono::Utc>> = row.try_get("", "ended_at").ok();
    Ok(crate::store::CdrView {
        id: row.try_get("", "id")?,
        call_id: row.try_get("", "call_id")?,
        tenant_id: row.try_get("", "tenant_id").ok(),
        direction: row.try_get("", "direction").unwrap_or_default(),
        status: row.try_get("", "status").unwrap_or_default(),
        from_number: row.try_get("", "from_number").ok(),
        to_number: row.try_get("", "to_number").ok(),
        started_at: started_at.map(|t| t.to_rfc3339()),
        ended_at: ended_at.map(|t| t.to_rfc3339()),
        duration_secs: row.try_get("", "duration_secs").unwrap_or(0),
        recording_url: row.try_get("", "recording_url").ok(),
    })
}

// ── Row converters ────────────────────────────────────────────────────────────

fn row_to_trunk_proto(row: &sea_orm::QueryResult) -> Result<TrunkConfigProto> {
    let id: i64 = row.try_get("", "id")?;
    let name: String = row.try_get("", "name")?;
    let sip_server: Option<String> = row.try_get("", "sip_server").ok();
    let outbound_proxy: Option<String> = row.try_get("", "outbound_proxy").ok();
    let sip_transport: String = row
        .try_get("", "sip_transport")
        .unwrap_or_else(|_| "udp".into());
    let auth_username: Option<String> = row.try_get("", "auth_username").ok();
    let auth_password: Option<String> = row.try_get("", "auth_password").ok();
    let direction: String = row
        .try_get("", "direction")
        .unwrap_or_else(|_| "bidirectional".into());
    let register_enabled: bool = row.try_get("", "register_enabled").unwrap_or(false);
    let register_expires: Option<i32> = row.try_get("", "register_expires").ok();
    let rewrite_hostport: bool = row.try_get("", "rewrite_hostport").unwrap_or(true);
    let incoming_from: Option<String> = row.try_get("", "incoming_from_user_prefix").ok();
    let incoming_to: Option<String> = row.try_get("", "incoming_to_user_prefix").ok();
    let max_cps: Option<i32> = row.try_get("", "max_cps").ok();
    let max_concurrent: Option<i32> = row.try_get("", "max_concurrent").ok();

    let dest = sip_server
        .clone()
        .or(outbound_proxy.clone())
        .unwrap_or_default();
    let backup_dest = outbound_proxy.filter(|p| p != &dest);

    let allowed_ips_json: Option<serde_json::Value> = row
        .try_get::<Option<serde_json::Value>>("", "allowed_ips")
        .ok()
        .flatten();
    let inbound_hosts = json_string_array(allowed_ips_json);

    let did_json: Option<serde_json::Value> = row
        .try_get::<Option<serde_json::Value>>("", "did_numbers")
        .ok()
        .flatten();
    let did_numbers = json_string_array(did_json);

    let reg_headers_json: Option<serde_json::Value> = row
        .try_get::<Option<serde_json::Value>>("", "register_extra_headers")
        .ok()
        .flatten();
    let register_extra_headers = reg_headers_json
        .as_ref()
        .and_then(|v| {
            serde_json::from_value::<std::collections::HashMap<String, String>>(v.clone()).ok()
        })
        .unwrap_or_default();

    let metadata_json: Option<serde_json::Value> = row
        .try_get::<Option<serde_json::Value>>("", "metadata")
        .ok()
        .flatten();
    let sbc_metadata_json = metadata_json
        .as_ref()
        .filter(|v| v.get("sbc").is_some())
        .map(|v| v.to_string());

    Ok(TrunkConfigProto {
        name,
        id: Some(id),
        tenant_id: None, // populated by caller if needed
        dest,
        backup_dest,
        transport: Some(sip_transport),
        direction: Some(direction),
        inbound_hosts,
        did_numbers,
        username: auth_username,
        password: auth_password,
        codec: vec![],
        disabled: None,
        max_calls: max_concurrent.map(|v| v as u32),
        max_cps: max_cps.map(|v| v as u32),
        weight: None,
        register_enabled: Some(register_enabled),
        register_expires: register_expires.map(|v| v as u32),
        register_extra_headers,
        rewrite_hostport: Some(rewrite_hostport),
        incoming_from_user_prefix: incoming_from,
        incoming_to_user_prefix: incoming_to,
        country: None,
        sbc_metadata_json,
    })
}

fn row_to_route_proto(
    row: &sea_orm::QueryResult,
) -> Result<Option<crate::grpc::proto::control::RouteRuleProto>> {
    use crate::grpc::proto::control::{
        MatchConditionsProto, RewriteRulesProto, RouteActionProto, RouteRuleProto,
    };

    let name: String = row.try_get("", "name")?;
    let description: Option<String> = row.try_get("", "description").ok();
    let priority: i32 = row.try_get("", "priority").unwrap_or(0);
    let direction: String = row
        .try_get("", "direction")
        .unwrap_or_else(|_| "any".into());
    let source_pattern: Option<String> = row.try_get("", "source_pattern").ok();
    let dest_pattern: Option<String> = row.try_get("", "destination_pattern").ok();
    let selection_strategy: String = row
        .try_get("", "selection_strategy")
        .unwrap_or_else(|_| "rr".into());
    let hash_key: Option<String> = row.try_get("", "hash_key").ok();

    let match_conditions = MatchConditionsProto {
        from_user: source_pattern,
        to_user: dest_pattern,
        ..Default::default()
    };

    let target_trunks_json: Option<serde_json::Value> = row
        .try_get::<Option<serde_json::Value>>("", "target_trunks")
        .ok()
        .flatten();
    let dest: Vec<String> = target_trunks_json
        .as_ref()
        .and_then(|v| {
            #[derive(serde::Deserialize)]
            struct TrunkRef {
                name: String,
            }
            serde_json::from_value::<Vec<TrunkRef>>(v.clone()).ok()
        })
        .map(|refs| refs.into_iter().map(|r| r.name).collect())
        .unwrap_or_default();

    let rewrite_json: Option<serde_json::Value> = row
        .try_get::<Option<serde_json::Value>>("", "rewrite_rules")
        .ok()
        .flatten();
    let rewrite = rewrite_json.as_ref().map(|v| {
        let s = |k: &str| -> Option<String> { v.get(k).and_then(|x| x.as_str()).map(String::from) };
        let headers: std::collections::HashMap<String, String> = v
            .get("headers")
            .and_then(|h| serde_json::from_value(h.clone()).ok())
            .unwrap_or_default();
        RewriteRulesProto {
            from_user: s("from.user").or_else(|| s("from_user")),
            from_host: s("from.host").or_else(|| s("from_host")),
            to_user: s("to.user").or_else(|| s("to_user")),
            to_host: s("to.host").or_else(|| s("to_host")),
            to_port: s("to.port").or_else(|| s("to_port")),
            request_uri_user: s("request_uri.user").or_else(|| s("request_uri_user")),
            request_uri_host: s("request_uri.host").or_else(|| s("request_uri_host")),
            request_uri_port: s("request_uri.port").or_else(|| s("request_uri_port")),
            headers,
        }
    });

    let action = RouteActionProto {
        dest,
        select: selection_strategy,
        hash_key,
        auto_answer: true,
        ..Default::default()
    };

    Ok(Some(RouteRuleProto {
        name,
        description,
        priority,
        direction,
        match_conditions: Some(match_conditions),
        rewrite,
        action: Some(action),
        ..Default::default()
    }))
}

fn json_string_array(value: Option<serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::ControlMigrator;
    use sea_orm::{Database, Statement};
    use sea_orm_migration::{MigratorTrait, SchemaManager};

    /// Spin up an in-memory SQLite DB with the full control schema applied.
    async fn fresh_store() -> Store {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);
        for m in ControlMigrator::migrations() {
            m.up(&manager).await.unwrap();
        }
        Store::new(db)
    }

    async fn insert_rule(store: &Store, tenant: Option<i64>, action: &str, target: &str, prio: i32) {
        let sql = "INSERT INTO rustpbx_acl_rules (tenant_id, action, target, priority, is_active) \
                   VALUES ($1,$2,$3,$4,TRUE)";
        store
            .db
            .execute(Statement::from_sql_and_values(
                store.db.get_database_backend(),
                sql,
                vec![
                    sea_orm::Value::BigInt(tenant),
                    sea_orm::Value::String(Some(Box::new(action.into()))),
                    sea_orm::Value::String(Some(Box::new(target.into()))),
                    sea_orm::Value::Int(Some(prio)),
                ],
            ))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn load_acl_rules_orders_by_priority_and_formats() {
        let store = fresh_store().await;
        insert_rule(&store, None, "deny", "all", 200).await;
        insert_rule(&store, None, "allow", "10.0.0.0/8", 100).await;

        let rules = store.load_acl_rules(None).await.unwrap();
        // priority ascending → allow (100) before deny (200), formatted "action target".
        assert_eq!(rules, vec!["allow 10.0.0.0/8".to_string(), "deny all".to_string()]);
    }

    async fn insert_cdr(store: &Store, call_id: &str, tenant: Option<i64>) {
        let sql = "INSERT INTO rustpbx_call_records \
            (call_id, direction, status, duration_secs, tenant_id, from_number, to_number) \
            VALUES ($1,'inbound','completed',42,$2,'1001','1002')";
        store
            .db
            .execute(Statement::from_sql_and_values(
                store.db.get_database_backend(),
                sql,
                vec![
                    sea_orm::Value::String(Some(Box::new(call_id.into()))),
                    sea_orm::Value::BigInt(tenant),
                ],
            ))
            .await
            .unwrap();
    }

    async fn insert_tenant(store: &Store, id: i64, status: &str) {
        let sql = "INSERT INTO rustpbx_tenants (id, name, status, created_at, updated_at) \
                   VALUES ($1, $2, $3, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)";
        store
            .db
            .execute(Statement::from_sql_and_values(
                store.db.get_database_backend(),
                sql,
                vec![
                    sea_orm::Value::BigInt(Some(id)),
                    sea_orm::Value::String(Some(Box::new(format!("tenant-{id}")))),
                    sea_orm::Value::String(Some(Box::new(status.into()))),
                ],
            ))
            .await
            .unwrap();
    }

    async fn insert_trunk(store: &Store, name: &str, tenant: Option<i64>) {
        let sql = "INSERT INTO rustpbx_sip_trunks (name, direction, sip_transport, tenant_id) \
                   VALUES ($1, 'outbound', 'udp', $2)";
        store
            .db
            .execute(Statement::from_sql_and_values(
                store.db.get_database_backend(),
                sql,
                vec![
                    sea_orm::Value::String(Some(Box::new(name.into()))),
                    sea_orm::Value::BigInt(tenant),
                ],
            ))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn tenant_max_concurrent_reads_limit_or_none() {
        let store = fresh_store().await;
        insert_tenant(&store, 1, "active").await; // no max set → NULL
        store
            .db
            .execute(Statement::from_sql_and_values(
                store.db.get_database_backend(),
                "UPDATE rustpbx_tenants SET max_concurrent_calls = $1 WHERE id = 1",
                vec![sea_orm::Value::Int(Some(5))],
            ))
            .await
            .unwrap();
        insert_tenant(&store, 2, "active").await; // stays NULL (unlimited)
        // Tenant 3 explicitly 0 → treated as unlimited (None).
        insert_tenant(&store, 3, "active").await;
        store
            .db
            .execute(Statement::from_sql_and_values(
                store.db.get_database_backend(),
                "UPDATE rustpbx_tenants SET max_concurrent_calls = $1 WHERE id = 3",
                vec![sea_orm::Value::Int(Some(0))],
            ))
            .await
            .unwrap();

        assert_eq!(store.tenant_max_concurrent(1).await.unwrap(), Some(5));
        assert_eq!(store.tenant_max_concurrent(2).await.unwrap(), None, "NULL → unlimited");
        assert_eq!(store.tenant_max_concurrent(3).await.unwrap(), None, "0 → unlimited");
        assert_eq!(store.tenant_max_concurrent(999).await.unwrap(), None, "missing tenant → None");
    }

    #[tokio::test]
    async fn config_distribution_excludes_suspended_tenants() {
        let store = fresh_store().await;
        insert_tenant(&store, 1, "active").await;
        insert_tenant(&store, 2, "suspended").await;
        insert_trunk(&store, "t1", Some(1)).await;
        insert_trunk(&store, "t2", Some(2)).await;
        insert_trunk(&store, "global", None).await;

        // Serving all config drops the suspended tenant's trunk, keeps active +
        // global.
        let names: Vec<String> = store.load_trunks(None).await.unwrap().into_iter().map(|t| t.name).collect();
        assert!(names.contains(&"t1".to_string()));
        assert!(names.contains(&"global".to_string()));
        assert!(!names.contains(&"t2".to_string()), "suspended tenant's trunk must not be distributed");

        // Even asking for the suspended tenant explicitly yields only global rows.
        let for_t2: Vec<String> = store.load_trunks(Some(2)).await.unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(for_t2, vec!["global".to_string()]);
    }

    #[tokio::test]
    async fn list_call_records_scopes_by_tenant() {
        let store = fresh_store().await;
        insert_cdr(&store, "c-t1", Some(1)).await;
        insert_cdr(&store, "c-t2", Some(2)).await;
        insert_cdr(&store, "c-global", None).await;

        let opts = CdrListOpts { tenant_id: Some(1), limit: 100, ..Default::default() };
        let (t1, t1_total) = store.list_call_records_paged(&opts).await.unwrap();
        assert_eq!(t1.len(), 1, "tenant 1 sees only its own CDR");
        assert_eq!(t1_total, 1, "total reflects the tenant scope");
        assert_eq!(t1[0].call_id, "c-t1");
        assert_eq!(t1[0].duration_secs, 42);

        let all = CdrListOpts { limit: 100, ..Default::default() };
        let (all_rows, all_total) = store.list_call_records_paged(&all).await.unwrap();
        assert_eq!(all_rows.len(), 3, "no scope → all CDRs");
        assert_eq!(all_total, 3);
    }

    #[tokio::test]
    async fn list_call_records_filters_and_pages() {
        let store = fresh_store().await;
        // All seeded CDRs share from=1001 to=1002 status=completed direction=inbound.
        for i in 0..5 {
            insert_cdr(&store, &format!("c-{i}"), Some(1)).await;
        }

        // Number search hits both legs (from_number/to_number).
        let by_num = CdrListOpts {
            tenant_id: Some(1),
            search: Some("100".into()),
            limit: 100,
            ..Default::default()
        };
        assert_eq!(store.list_call_records_paged(&by_num).await.unwrap().1, 5);

        // A non-matching search yields nothing.
        let miss = CdrListOpts {
            tenant_id: Some(1),
            search: Some("9999".into()),
            limit: 100,
            ..Default::default()
        };
        assert_eq!(store.list_call_records_paged(&miss).await.unwrap().1, 0);

        // Status filter matches; a wrong status excludes all.
        let busy = CdrListOpts {
            tenant_id: Some(1),
            status: Some("busy".into()),
            limit: 100,
            ..Default::default()
        };
        assert_eq!(store.list_call_records_paged(&busy).await.unwrap().1, 0);

        // Pagination: page size 2 returns 2 rows but the full total (5).
        let page = CdrListOpts { tenant_id: Some(1), limit: 2, offset: 0, ..Default::default() };
        let (rows, total) = store.list_call_records_paged(&page).await.unwrap();
        assert_eq!(rows.len(), 2, "page is limited");
        assert_eq!(total, 5, "total counts all matches, not just the page");

        // Last page (offset 4) has the single remaining row.
        let last = CdrListOpts { tenant_id: Some(1), limit: 2, offset: 4, ..Default::default() };
        assert_eq!(store.list_call_records_paged(&last).await.unwrap().0.len(), 1);
    }

    #[tokio::test]
    async fn load_acl_rules_scopes_by_tenant_including_globals() {
        let store = fresh_store().await;
        insert_rule(&store, None, "allow", "192.168.0.0/16", 10).await; // global
        insert_rule(&store, Some(1), "deny", "all", 20).await; // tenant 1
        insert_rule(&store, Some(2), "allow", "172.16.0.0/12", 30).await; // tenant 2 only

        // tenant 1 sees its own rule + globals, not tenant 2's.
        let t1 = store.load_acl_rules(Some(1)).await.unwrap();
        assert_eq!(t1, vec!["allow 192.168.0.0/16".to_string(), "deny all".to_string()]);

        // no tenant filter → all rules.
        let all = store.load_acl_rules(None).await.unwrap();
        assert_eq!(all.len(), 3);
    }
}
