//! Platform settings — a thin typed wrapper over the `rustpbx_platform_settings`
//! key/value table. Currently holds the superadmin-configured wildcard
//! `base_domain` used to mint each tenant's default `{id}.{base_domain}` domain.

use anyhow::Result;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement, Value};

pub const KEY_BASE_DOMAIN: &str = "base_domain";
pub const KEY_STUN_SERVERS: &str = "stun_servers";
pub const KEY_RECORDING_POLICY: &str = "recording_policy";
pub const KEY_CONFIG_VERSION: &str = "config_version";

pub struct PlatformSettings<'a> {
    db: &'a DatabaseConnection,
}

impl<'a> PlatformSettings<'a> {
    pub fn new(db: &'a DatabaseConnection) -> Self {
        Self { db }
    }

    /// Read a setting's value, or `None` if unset.
    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                "SELECT value FROM rustpbx_platform_settings WHERE key = $1",
                vec![Value::String(Some(Box::new(key.to_string())))],
            ))
            .await?;
        match row {
            Some(r) => Ok(r.try_get::<Option<String>>("", "value").ok().flatten()),
            None => Ok(None),
        }
    }

    /// Upsert a setting.
    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        self.db
            .execute(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                "INSERT INTO rustpbx_platform_settings (key, value) VALUES ($1, $2) \
                 ON CONFLICT (key) DO UPDATE SET value = $2",
                vec![
                    Value::String(Some(Box::new(key.to_string()))),
                    Value::String(Some(Box::new(value.to_string()))),
                ],
            ))
            .await?;
        Ok(())
    }

    /// Monotonic configuration version used by config-watch events.
    pub async fn config_version(&self) -> u64 {
        self.get(KEY_CONFIG_VERSION)
            .await
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or_default()
    }

    /// Increment and persist the configuration version.
    pub async fn bump_config_version(&self) -> Result<u64> {
        let next = self.config_version().await.saturating_add(1);
        self.set(KEY_CONFIG_VERSION, &next.to_string()).await?;
        Ok(next)
    }

    /// Convenience: the configured wildcard base domain (empty string if unset).
    pub async fn base_domain(&self) -> String {
        self.get(KEY_BASE_DOMAIN)
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Centrally-managed STUN server list (`host:port`), stored as a JSON array.
    /// Returns the shared defaults when unset — so the platform-settings UI
    /// shows a sensible list out of the box, and nodes pulling via gRPC always
    /// get a usable list (the node-local config is only a last-resort fallback
    /// if the control plane is unreachable).
    pub async fn stun_servers(&self) -> Vec<String> {
        let stored = self
            .get(KEY_STUN_SERVERS)
            .await
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_default();
        if stored.iter().any(|s| !s.trim().is_empty()) {
            stored
        } else {
            rustpbx_core::stun::default_stun_servers()
        }
    }

    /// Persist the STUN server list (as a JSON array).
    pub async fn set_stun_servers(&self, servers: &[String]) -> Result<()> {
        let json = serde_json::to_string(servers)?;
        self.set(KEY_STUN_SERVERS, &json).await
    }

    /// Global call-recording policy (a JSON-serialized `RecordingPolicy`),
    /// forwarded verbatim to workers. Empty/blank → None (no recording).
    pub async fn recording_policy_json(&self) -> Option<String> {
        self.get(KEY_RECORDING_POLICY)
            .await
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
    }

    /// Persist the recording policy JSON.
    pub async fn set_recording_policy_json(&self, json: &str) -> Result<()> {
        self.set(KEY_RECORDING_POLICY, json).await
    }

    /// Seed `base_domain` from the config file on startup *only if* it has never
    /// been set, so superadmin edits via the API are never overwritten.
    pub async fn seed_base_domain(&self, from_config: &str) -> Result<()> {
        let from_config = from_config.trim();
        if from_config.is_empty() {
            return Ok(());
        }
        if self.get(KEY_BASE_DOMAIN).await?.is_none() {
            self.set(KEY_BASE_DOMAIN, from_config).await?;
        }
        Ok(())
    }
}
