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

async fn publish_if_version_advanced(
    db: &DatabaseConnection,
    change_tx: &broadcast::Sender<ConfigChangeEvent>,
    observed_version: &AtomicU64,
) {
    let current = PlatformSettings::new(db).config_version().await;
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
