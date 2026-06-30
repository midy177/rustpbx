//! Best-effort REGISTER location reporter.
//!
//! The main registrar stores contacts in the local Worker process. In a
//! multi-Worker deployment, Control also needs to know which Worker owns an
//! extension so extension-to-extension calls can be routed to the right node.

use anyhow::Result;
use async_trait::async_trait;
use rsipstack::sip::Method;
use rsipstack::sip::prelude::HeadersExt;
use rsipstack::transaction::transaction::Transaction;
use rustpbx::call::{TenantId, TransactionCookie};
use rustpbx::config::ProxyConfig;
use rustpbx::proxy::server::SipServerRef;
use rustpbx::proxy::{ProxyAction, ProxyModule};
use rustpbx_proto::control::{ExtensionLocationReport, control_plane_client::ControlPlaneClient};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use tonic::transport::Channel;
use tracing::{debug, warn};

#[derive(Clone)]
struct ReporterConfig {
    control_plane_addr: String,
    tls: Option<rustpbx_proto::tls::ClientTls>,
    worker_id: String,
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
) {
    if REPORTER
        .set(ReporterConfig {
            control_plane_addr,
            tls,
            worker_id,
        })
        .is_err()
    {
        warn!("extension location reporter already initialised");
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
            if let Err(e) = report_location(report).await {
                warn!(error = %e, "extension location report failed");
            }
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

async fn report_location(report: PendingReport) -> Result<()> {
    let mut client = connect_control(&report.cfg).await?;
    client
        .report_extension_location(ExtensionLocationReport {
            tenant_id: report.tenant_id,
            extension: report.extension.clone(),
            worker_id: report.cfg.worker_id.clone(),
            contact: report.contact,
            expires_secs: report.expires_secs,
        })
        .await?;
    debug!(
        extension = %report.extension,
        tenant_id = ?report.tenant_id,
        worker_id = %report.cfg.worker_id,
        expires_secs = report.expires_secs,
        "reported extension location"
    );
    Ok(())
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
}
