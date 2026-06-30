use crate::{
    grpc::proto::control::{ConfigChangeEvent, config_change_event::ChangeType},
    settings::PlatformSettings,
};
use sea_orm::DatabaseConnection;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::{sync::broadcast, time::Duration};
use tracing::{info, warn};

/// Periodically observes the shared config version and re-broadcasts changes
/// made by another Control replica to local config-watch subscribers.
pub async fn run_config_version_watcher(
    db: DatabaseConnection,
    change_tx: broadcast::Sender<ConfigChangeEvent>,
    observed_version: Arc<AtomicU64>,
    interval: Duration,
) {
    let mut tick = tokio::time::interval(interval);
    tick.tick().await;
    loop {
        tick.tick().await;
        publish_if_version_advanced(&db, &change_tx, &observed_version).await;
    }
}

/// Postgres acceleration path for cross-Control config broadcasts. The periodic
/// watcher remains the portable fallback for missed notifications and non-PG DBs.
pub async fn run_postgres_config_notify_listener(
    db: DatabaseConnection,
    database_url: String,
    change_tx: broadcast::Sender<ConfigChangeEvent>,
    observed_version: Arc<AtomicU64>,
) {
    loop {
        match listen_once(
            &db,
            &database_url,
            &change_tx,
            Arc::clone(&observed_version),
        )
        .await
        {
            Ok(()) => {}
            Err(e) => warn!(error = %e, "postgres config notify listener failed; retrying"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn listen_once(
    db: &DatabaseConnection,
    database_url: &str,
    change_tx: &broadcast::Sender<ConfigChangeEvent>,
    observed_version: Arc<AtomicU64>,
) -> anyhow::Result<()> {
    let mut listener = sqlx::postgres::PgListener::connect(database_url).await?;
    listener.listen("rustpbx_config_changed").await?;
    info!("postgres config notify listener started");
    loop {
        let notification = listener.recv().await?;
        let payload_version = notification.payload().parse::<u64>().ok();
        if let Some(version) = payload_version {
            publish_version_if_advanced(change_tx, &observed_version, version);
        } else {
            publish_if_version_advanced(db, change_tx, &observed_version).await;
        }
    }
}

async fn publish_if_version_advanced(
    db: &DatabaseConnection,
    change_tx: &broadcast::Sender<ConfigChangeEvent>,
    observed_version: &AtomicU64,
) {
    let current = PlatformSettings::new(db).config_version().await;
    publish_version_if_advanced(change_tx, observed_version, current);
}

fn publish_version_if_advanced(
    change_tx: &broadcast::Sender<ConfigChangeEvent>,
    observed_version: &AtomicU64,
    current: u64,
) {
    let observed = observed_version.load(Ordering::Relaxed);
    if current <= observed {
        return;
    }

    if observed_version
        .compare_exchange(observed, current, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        let _ = change_tx.send(ConfigChangeEvent {
            change_type: ChangeType::PlatformChanged as i32,
            name: Some("platform".to_string()),
            trunk: None,
            version: current,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;
    use sea_orm_migration::{MigrationTrait, SchemaManager};

    #[tokio::test]
    async fn broadcasts_when_external_config_version_advances() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        crate::migration::create_platform_settings::Migration
            .up(&SchemaManager::new(&db))
            .await
            .unwrap();
        let settings = PlatformSettings::new(&db);
        let observed_version = Arc::new(AtomicU64::new(settings.config_version().await));
        let (tx, mut rx) = broadcast::channel(8);

        let version = settings.bump_config_version().await.unwrap();
        publish_if_version_advanced(&db, &tx, &observed_version).await;

        let event = rx.recv().await.unwrap();
        assert_eq!(event.version, version);
        assert_eq!(event.change_type, ChangeType::PlatformChanged as i32);

        publish_if_version_advanced(&db, &tx, &observed_version).await;
        assert!(rx.try_recv().is_err());
    }
}
