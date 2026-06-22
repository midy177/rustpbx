use crate::{grpc::proto::control::TrunkConfigProto, store::Store};
use anyhow::Result;
use sea_orm::ConnectionTrait;

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
        let sql = "INSERT INTO rustpbx_call_records \
            (call_id, from_number, to_number, direction, status, \
             started_at, ended_at, duration_secs, metadata) \
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
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
                 FROM rustpbx_routing \
                 WHERE is_active = TRUE AND (tenant_id = $1 OR tenant_id IS NULL) \
                 ORDER BY priority ASC",
                vec![sea_orm::Value::BigInt(Some(tid))],
            )
        } else {
            (
                "SELECT id, name, description, direction, priority, \
                 source_pattern, destination_pattern, header_filters, \
                 rewrite_rules, target_trunks, selection_strategy, hash_key, \
                 source_trunk_id, metadata \
                 FROM rustpbx_routing \
                 WHERE is_active = TRUE \
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
                    "SELECT {cols} FROM rustpbx_routing \
                     WHERE tenant_id = $1 OR tenant_id IS NULL ORDER BY priority ASC"
                ),
                vec![sea_orm::Value::BigInt(Some(tid))],
            ),
            None => (
                format!("SELECT {cols} FROM rustpbx_routing ORDER BY priority ASC"),
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

    /// Load active ACL rules as `"<action> <target>"` strings, optionally scoped
    /// to a tenant (matches the tenant plus global/NULL-tenant rows), ordered by
    /// priority ascending. Format matches the main binary's `acl_rules` config.
    pub async fn load_acl_rules(&self, tenant_id: Option<i64>) -> Result<Vec<String>> {
        use sea_orm::{ConnectionTrait, Statement};

        let (sql, values) = match tenant_id {
            Some(tid) => (
                "SELECT action, target FROM rustpbx_acl_rules \
                 WHERE is_active = TRUE AND (tenant_id = $1 OR tenant_id IS NULL) \
                 ORDER BY priority ASC, id ASC",
                vec![sea_orm::Value::BigInt(Some(tid))],
            ),
            None => (
                "SELECT action, target FROM rustpbx_acl_rules \
                 WHERE is_active = TRUE \
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
