//! Platform settings — a thin typed wrapper over the `rustpbx_platform_settings`
//! key/value table. Currently holds the superadmin-configured wildcard
//! `base_domain` used to mint each tenant's default `{id}.{base_domain}` domain.

use anyhow::Result;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement, Value};

pub const KEY_BASE_DOMAIN: &str = "base_domain";

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

    /// Convenience: the configured wildcard base domain (empty string if unset).
    pub async fn base_domain(&self) -> String {
        self.get(KEY_BASE_DOMAIN).await.ok().flatten().unwrap_or_default()
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
