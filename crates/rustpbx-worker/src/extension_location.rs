//! Best-effort REGISTER location reporter.
//!
//! The main registrar stores contacts in the local Worker process. In a
//! multi-Worker deployment, Control also needs to know which Worker owns an
//! extension so extension-to-extension calls can be routed to the right node.

use anyhow::Result;
use async_trait::async_trait;
use metrics::{counter, gauge};
use prost::Message;
use rsipstack::sip::Method;
use rsipstack::sip::prelude::HeadersExt;
use rsipstack::transaction::transaction::Transaction;
use rustpbx::call::{TenantId, TransactionCookie};
use rustpbx::config::ProxyConfig;
use rustpbx::proxy::server::SipServerRef;
use rustpbx::proxy::{ProxyAction, ProxyModule};
use rustpbx_proto::control::{ExtensionLocationReport, control_plane_client::ControlPlaneClient};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;
use tracing::{debug, info, warn};

const REPORT_RETRY_DELAYS_SECS: &[u64] = &[1, 2, 4, 8, 16];
const REPORT_REPLAY_INTERVAL_SECS: u64 = 30;

#[derive(Clone)]
struct ReporterConfig {
    control_plane_addr: String,
    tls: Option<rustpbx_proto::tls::ClientTls>,
    worker_id: String,
    spool_dir: PathBuf,
}

static REPORTER: OnceLock<ReporterConfig> = OnceLock::new();

#[derive(Clone)]
struct PendingReport {
    cfg: ReporterConfig,
    tenant_id: Option<i64>,
    extension: String,
    contact: String,
    expires_secs: u32,
}

pub fn init_extension_location_reporter(
    control_plane_addr: String,
    tls: Option<rustpbx_proto::tls::ClientTls>,
    worker_id: String,
    spool_dir: impl Into<PathBuf>,
) {
    if REPORTER
        .set(ReporterConfig {
            control_plane_addr,
            tls,
            worker_id,
            spool_dir: spool_dir.into(),
        })
        .is_err()
    {
        warn!("extension location reporter already initialised");
    }
}

pub async fn run_extension_location_spool_replay(cancel: CancellationToken) {
    let Some(cfg) = REPORTER.get().cloned() else {
        warn!("extension location replay requested before reporter initialised");
        return;
    };
    let spool = ExtensionLocationSpool::new(cfg.spool_dir.clone());
    let mut tick = tokio::time::interval(Duration::from_secs(REPORT_REPLAY_INTERVAL_SECS));
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tick.tick() => {
                match spool.replay_once(&cfg).await {
                    Ok(n) if n > 0 => info!(count = n, "replayed extension location reports"),
                    Ok(_) => {}
                    Err(e) => warn!(error = %e, "extension location replay failed"),
                }
            }
        }
    }
}

pub struct ExtensionLocationModule {
    pending: Arc<Mutex<HashMap<String, PendingReport>>>,
}

impl ExtensionLocationModule {
    pub fn create(
        _server: SipServerRef,
        _config: Arc<ProxyConfig>,
    ) -> Result<Box<dyn ProxyModule>> {
        Ok(Box::new(Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
        }))
    }
}

#[async_trait]
impl ProxyModule for ExtensionLocationModule {
    fn name(&self) -> &str {
        "extension-location"
    }

    fn allow_methods(&self) -> Vec<Method> {
        vec![Method::Register]
    }

    async fn on_start(&mut self) -> Result<()> {
        Ok(())
    }

    async fn on_stop(&self) -> Result<()> {
        Ok(())
    }

    async fn on_transaction_begin(
        &self,
        _token: tokio_util::sync::CancellationToken,
        tx: &mut Transaction,
        cookie: TransactionCookie,
    ) -> Result<ProxyAction> {
        if tx.original.method != Method::Register {
            return Ok(ProxyAction::Continue);
        }

        let Some(cfg) = REPORTER.get().cloned() else {
            return Ok(ProxyAction::Continue);
        };
        let Some(extension) = registered_extension(tx) else {
            return Ok(ProxyAction::Continue);
        };
        let tenant_id = cookie.get_extension::<TenantId>().map(|t| t.0);
        let report = PendingReport {
            cfg,
            tenant_id,
            extension,
            contact: first_contact_header(tx).unwrap_or_default(),
            expires_secs: register_expires(tx),
        };
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(tx.key.to_string(), report);
        } else {
            warn!("extension location pending map is poisoned");
        }

        Ok(ProxyAction::Continue)
    }

    async fn on_transaction_end(&self, tx: &mut Transaction) -> Result<()> {
        if tx.original.method != Method::Register {
            return Ok(());
        }

        let report = match self.pending.lock() {
            Ok(mut pending) => pending.remove(&tx.key.to_string()),
            Err(_) => {
                warn!("extension location pending map is poisoned");
                None
            }
        };
        let Some(report) = report else {
            return Ok(());
        };
        if !register_succeeded(tx) {
            debug!(key = %tx.key, "skip extension location report for failed register");
            return Ok(());
        }

        tokio::spawn(async move {
            spool_and_report_location(report).await;
        });
        Ok(())
    }
}

fn registered_extension(tx: &Transaction) -> Option<String> {
    tx.original
        .to_header()
        .ok()
        .and_then(|to| to.uri().ok())
        .and_then(|uri| uri.user().map(|user| user.to_string()))
        .filter(|user| !user.trim().is_empty())
}

fn register_expires(tx: &Transaction) -> u32 {
    let global = tx
        .original
        .expires_header()
        .and_then(|header| header.value().parse::<u32>().ok());
    let contact_expires = first_contact_header(tx).and_then(|contact| expires_param(&contact));
    contact_expires.or(global).unwrap_or(300)
}

fn first_contact_header(tx: &Transaction) -> Option<String> {
    tx.original.headers.iter().find_map(|header| match header {
        rsipstack::sip::Header::Contact(contact) => Some(contact.to_string()),
        rsipstack::sip::Header::Other(name, value) if name.eq_ignore_ascii_case("Contact") => {
            Some(value.clone())
        }
        _ => None,
    })
}

fn expires_param(contact: &str) -> Option<u32> {
    contact.split(';').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("expires")
            .then(|| value.trim().parse::<u32>().ok())
            .flatten()
    })
}

fn register_succeeded(tx: &Transaction) -> bool {
    tx.last_response
        .as_ref()
        .is_some_and(|resp| *resp.status_code() == rsipstack::sip::StatusCode::OK)
}

async fn report_location_with_retry(report: PendingReport) {
    let mut elapsed_secs = 0u64;
    for (attempt, delay_secs) in REPORT_RETRY_DELAYS_SECS
        .iter()
        .copied()
        .map(Some)
        .chain(std::iter::once(None))
        .enumerate()
    {
        match report_location(&report).await {
            Ok(()) => return,
            Err(e) => {
                let Some(delay_secs) = delay_secs else {
                    warn!(
                        error = %e,
                        extension = %report.extension,
                        expires_secs = report.expires_secs,
                        "extension location report failed after retries"
                    );
                    return;
                };

                let max_retry_window = report.expires_secs.max(30) as u64;
                if elapsed_secs.saturating_add(delay_secs) > max_retry_window {
                    warn!(
                        error = %e,
                        extension = %report.extension,
                        expires_secs = report.expires_secs,
                        "extension location report retry window expired"
                    );
                    return;
                }

                warn!(
                    error = %e,
                    extension = %report.extension,
                    delay_secs,
                    attempt = attempt + 1,
                    "extension location report failed; retrying"
                );
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                elapsed_secs = elapsed_secs.saturating_add(delay_secs);
            }
        }
    }
}

async fn spool_and_report_location(report: PendingReport) {
    let spool = ExtensionLocationSpool::new(report.cfg.spool_dir.clone());
    match spool.enqueue(&report.to_proto()).await {
        Ok(path) => {
            counter!("worker_extension_location_spooled_total").increment(1);
            match report_location_with_retry_result(&report).await {
                Ok(()) => {
                    if let Err(e) = tokio::fs::remove_file(&path).await {
                        warn!(error = %e, path = %path.display(), "failed to remove sent extension location spool file");
                    }
                    spool.update_pending_gauge().await;
                }
                Err(e) => {
                    warn!(error = %e, path = %path.display(), extension = %report.extension, "extension location report remains spooled");
                    spool.update_pending_gauge().await;
                }
            }
        }
        Err(e) => {
            counter!("worker_extension_location_spool_write_failures_total").increment(1);
            warn!(error = %e, "extension location spool write failed; falling back to in-memory retry");
            report_location_with_retry(report).await;
        }
    }
}

async fn report_location_with_retry_result(report: &PendingReport) -> Result<()> {
    let mut elapsed_secs = 0u64;
    for delay_secs in REPORT_RETRY_DELAYS_SECS
        .iter()
        .copied()
        .map(Some)
        .chain(std::iter::once(None))
    {
        match report_location(report).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                let Some(delay_secs) = delay_secs else {
                    return Err(e);
                };

                let max_retry_window = report.expires_secs.max(30) as u64;
                if elapsed_secs.saturating_add(delay_secs) > max_retry_window {
                    return Err(e);
                }

                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                elapsed_secs = elapsed_secs.saturating_add(delay_secs);
            }
        }
    }
    Ok(())
}

async fn report_location(report: &PendingReport) -> Result<()> {
    let mut client = connect_control(&report.cfg).await?;
    client.report_extension_location(report.to_proto()).await?;
    debug!(
        extension = %report.extension,
        tenant_id = ?report.tenant_id,
        worker_id = %report.cfg.worker_id,
        expires_secs = report.expires_secs,
        "reported extension location"
    );
    Ok(())
}

impl PendingReport {
    fn to_proto(&self) -> ExtensionLocationReport {
        ExtensionLocationReport {
            tenant_id: self.tenant_id,
            extension: self.extension.clone(),
            worker_id: self.cfg.worker_id.clone(),
            contact: self.contact.clone(),
            expires_secs: self.expires_secs,
        }
    }
}

#[derive(Clone)]
struct ExtensionLocationSpool {
    dir: PathBuf,
}

impl ExtensionLocationSpool {
    fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    async fn enqueue(&self, report: &ExtensionLocationReport) -> Result<PathBuf> {
        tokio::fs::create_dir_all(&self.dir).await?;
        let file = format!(
            "{}-{}-{}.pb",
            sanitize_file_component(&report.worker_id),
            sanitize_file_component(&report.extension),
            uuid::Uuid::new_v4()
        );
        let path = self.dir.join(file);
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, report.encode_to_vec()).await?;
        tokio::fs::rename(&tmp, &path).await?;
        self.update_pending_gauge().await;
        Ok(path)
    }

    async fn replay_once(&self, cfg: &ReporterConfig) -> Result<usize> {
        let mut dir = match tokio::fs::read_dir(&self.dir).await {
            Ok(dir) => dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        let mut sent = 0usize;
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("pb") {
                continue;
            }
            let bytes = match tokio::fs::read(&path).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "failed to read extension location spool file");
                    continue;
                }
            };
            let report = match ExtensionLocationReport::decode(bytes.as_slice()) {
                Ok(report) => report,
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "invalid extension location spool file; moving aside");
                    counter!("worker_extension_location_spool_invalid_total").increment(1);
                    move_bad_spool_file(&path).await.ok();
                    continue;
                }
            };
            if report.expires_secs > 0 && report_is_expired(&path, report.expires_secs).await {
                debug!(path = %path.display(), extension = %report.extension, "dropping expired extension location spool file");
                tokio::fs::remove_file(&path).await.ok();
                counter!("worker_extension_location_spool_expired_total").increment(1);
                continue;
            }
            let pending = PendingReport {
                cfg: cfg.clone(),
                tenant_id: report.tenant_id,
                extension: report.extension.clone(),
                contact: report.contact.clone(),
                expires_secs: report.expires_secs,
            };
            if let Err(e) = report_location(&pending).await {
                counter!("worker_extension_location_replay_failures_total").increment(1);
                warn!(error = %e, path = %path.display(), extension = %report.extension, "extension location replay failed");
                continue;
            }
            tokio::fs::remove_file(&path).await?;
            counter!("worker_extension_location_replay_success_total").increment(1);
            sent += 1;
        }
        self.update_pending_gauge().await;
        Ok(sent)
    }

    async fn pending_count(&self) -> Result<u64> {
        let mut dir = match tokio::fs::read_dir(&self.dir).await {
            Ok(dir) => dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        let mut count = 0;
        while let Some(entry) = dir.next_entry().await? {
            if entry.path().extension().and_then(|s| s.to_str()) == Some("pb") {
                count += 1;
            }
        }
        Ok(count)
    }

    async fn update_pending_gauge(&self) {
        match self.pending_count().await {
            Ok(count) => gauge!("worker_extension_location_spool_pending").set(count as f64),
            Err(e) => {
                warn!(error = %e, dir = %self.dir.display(), "failed to count extension location spool files")
            }
        }
    }
}

async fn report_is_expired(path: &Path, expires_secs: u32) -> bool {
    let Ok(meta) = tokio::fs::metadata(path).await else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    modified
        .elapsed()
        .map(|age| age > Duration::from_secs(expires_secs.max(30) as u64))
        .unwrap_or(false)
}

async fn move_bad_spool_file(path: &Path) -> Result<()> {
    let bad_path = path.with_extension("bad");
    tokio::fs::rename(path, bad_path).await?;
    Ok(())
}

fn sanitize_file_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

async fn connect_control(cfg: &ReporterConfig) -> Result<ControlPlaneClient<Channel>> {
    let channel = rustpbx_proto::tls::endpoint(&cfg.control_plane_addr, cfg.tls.as_ref())
        .map_err(|e| anyhow::anyhow!("invalid control plane addr/TLS: {e}"))?
        .connect()
        .await?;
    Ok(ControlPlaneClient::new(channel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_contact_expires_param() {
        assert_eq!(
            expires_param("<sip:1001@host>;expires=60;transport=ws"),
            Some(60)
        );
        assert_eq!(expires_param("<sip:1001@host>;expires=0"), Some(0));
        assert_eq!(expires_param("<sip:1001@host>"), None);
    }

    #[tokio::test]
    async fn extension_location_spool_writes_decodable_report() {
        let dir = std::env::temp_dir().join(format!(
            "rustpbx-extension-location-spool-{}",
            uuid::Uuid::new_v4()
        ));
        let spool = ExtensionLocationSpool::new(&dir);
        let report = ExtensionLocationReport {
            tenant_id: Some(42),
            extension: "1001".to_string(),
            worker_id: "worker/a".to_string(),
            contact: "<sip:1001@host>;expires=60".to_string(),
            expires_secs: 60,
        };

        let path = spool.enqueue(&report).await.unwrap();
        assert_eq!(spool.pending_count().await.unwrap(), 1);
        let bytes = tokio::fs::read(&path).await.unwrap();
        let decoded = ExtensionLocationReport::decode(bytes.as_slice()).unwrap();

        assert_eq!(decoded.tenant_id, Some(42));
        assert_eq!(decoded.extension, "1001");
        assert_eq!(decoded.worker_id, "worker/a");
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("worker_a-1001-")
        );

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[test]
    fn sanitize_file_component_replaces_unsafe_chars() {
        assert_eq!(sanitize_file_component("worker/a b"), "worker_a_b");
        assert_eq!(sanitize_file_component(""), "unknown");
    }
}
