/// Background task that watches the Control Plane for config change events
/// and re-pulls + re-injects config into ProxyDataContext.
///
/// Runs as a long-lived tokio task. On stream error it waits and retries.
use crate::config_source::{ConfigSource, GrpcConfigSource};
use crate::grpc_client::GrpcControlClient;
use anyhow::Result;
use rustpbx::proxy::data::ProxyDataContext;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

pub async fn run_config_watcher(
    client: Arc<RwLock<GrpcControlClient>>,
    config_source: Arc<GrpcConfigSource>,
    data_context: Arc<ProxyDataContext>,
    poll_secs: u64,
    cancel: CancellationToken,
) {
    let mut version = 0u64;

    loop {
        if cancel.is_cancelled() {
            break;
        }

        match watch_once(
            Arc::clone(&client),
            Arc::clone(&config_source),
            Arc::clone(&data_context),
            version,
            poll_secs,
            &cancel,
        )
        .await
        {
            Ok(last_version) => {
                version = last_version;
            }
            Err(e) => {
                warn!(error = %e, "config watch stream error, retrying in 5s");
                tokio::select! {
                    _ = sleep(Duration::from_secs(5)) => {}
                    _ = cancel.cancelled() => break,
                }
            }
        }
    }

    info!("config watcher stopped");
}

async fn watch_once(
    client: Arc<RwLock<GrpcControlClient>>,
    config_source: Arc<GrpcConfigSource>,
    data_context: Arc<ProxyDataContext>,
    from_version: u64,
    poll_secs: u64,
    cancel: &CancellationToken,
) -> Result<u64> {
    let mut stream = {
        let mut c = client.write().await;
        c.watch_config_changes(from_version).await?
    };

    info!(from_version, "config watch stream opened");
    let mut last_version = from_version;
    let mut poll = (poll_secs > 0).then(|| tokio::time::interval(Duration::from_secs(poll_secs)));
    if let Some(poll) = poll.as_mut() {
        poll.tick().await;
    }

    loop {
        tokio::select! {
            _ = async {
                if let Some(poll) = poll.as_mut() {
                    poll.tick().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                info!(poll_secs, "config poll tick — re-pulling edge-facing config");
                handle_event(
                    &config_source,
                    &data_context,
                    crate::proto::control::ConfigChangeEvent {
                        change_type: crate::proto::control::config_change_event::ChangeType::PlatformChanged as i32,
                        name: Some("poll".to_string()),
                        trunk: None,
                        version: last_version,
                    },
                ).await;
            }
            msg = stream.message() => {
                match msg? {
                    Some(event) => {
                        last_version = event.version.max(last_version);
                        handle_event(&config_source, &data_context, event).await;
                    }
                    None => {
                        info!("config watch stream closed by server");
                        break;
                    }
                }
            }
            _ = cancel.cancelled() => {
                break;
            }
        }
    }

    Ok(last_version)
}

async fn handle_event(
    config_source: &GrpcConfigSource,
    data_context: &Arc<ProxyDataContext>,
    event: crate::proto::control::ConfigChangeEvent,
) {
    use crate::proto::control::config_change_event::ChangeType;

    let change_type = ChangeType::try_from(event.change_type).unwrap_or(ChangeType::TrunkUpdated);
    match change_type {
        ChangeType::TrunkAdded | ChangeType::TrunkUpdated | ChangeType::TrunkRemoved => {
            info!(name = ?event.name, "config change: trunk — re-pulling from control plane");
            match config_source.load_trunks().await {
                Ok(trunks) => {
                    let mut config = (*data_context.config()).clone();
                    config.trunks = trunks;
                    if let Err(e) = data_context
                        .reload_trunks(false, Some(Arc::new(config)))
                        .await
                    {
                        error!(error = %e, "trunk reload failed");
                    }
                }
                Err(e) => error!(error = %e, "trunk re-pull failed"),
            }
        }
        ChangeType::RouteChanged => {
            info!("config change: routes — re-pulling from control plane");
            match config_source.load_routes().await {
                Ok(routes) => {
                    let mut config = (*data_context.config()).clone();
                    config.routes = Some(routes);
                    if let Err(e) = data_context
                        .reload_routes(false, Some(Arc::new(config)))
                        .await
                    {
                        error!(error = %e, "route reload failed");
                    }
                }
                Err(e) => error!(error = %e, "route re-pull failed"),
            }
        }
        ChangeType::AclChanged => {
            info!("config change: acl — re-pulling from control plane");
            match config_source.load_acl_rules().await {
                Ok(rules) => data_context.set_acl_rules(rules),
                Err(e) => error!(error = %e, "acl re-pull failed"),
            }
        }
        ChangeType::PlatformChanged => {
            info!("config change: platform — re-pulling edge-facing config");
            if let Ok(trunks) = config_source.load_trunks().await {
                let mut config = (*data_context.config()).clone();
                config.trunks = trunks;
                if let Err(e) = data_context
                    .reload_trunks(false, Some(Arc::new(config)))
                    .await
                {
                    error!(error = %e, "trunk reload failed");
                }
            }
            if let Ok(routes) = config_source.load_routes().await {
                let mut config = (*data_context.config()).clone();
                config.routes = Some(routes);
                if let Err(e) = data_context
                    .reload_routes(false, Some(Arc::new(config)))
                    .await
                {
                    error!(error = %e, "route reload failed");
                }
            }
            if let Ok(rules) = config_source.load_acl_rules().await {
                data_context.set_acl_rules(rules);
            }
        }
        ChangeType::QueueChanged | ChangeType::IvrChanged => {
            info!("config change is worker-only; edge ignoring");
        }
    }
}
