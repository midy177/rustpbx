//! Edge dialog path pinning.
//!
//! In a multi-Edge deployment, initial INVITEs can be load-balanced across
//! nodes, but in-dialog requests must return to the same Edge because dialog
//! state is local. This module adds a Record-Route header to successful INVITE
//! responses so SIP peers keep this Edge in the route set.

use anyhow::Result;
use async_trait::async_trait;
use rsipstack::sip::Method;
use rsipstack::transaction::transaction::Transaction;
use rustpbx::call::TransactionCookie;
use rustpbx::config::ProxyConfig;
use rustpbx::proxy::server::SipServerRef;
use rustpbx::proxy::{ProxyAction, ProxyModule};
use std::sync::{Arc, OnceLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

static RECORD_ROUTE: OnceLock<Option<String>> = OnceLock::new();

pub fn init_record_route(record_route: Option<String>) {
    if RECORD_ROUTE.set(record_route).is_err() {
        warn!("record-route already initialised");
    }
}

pub fn build_record_route(host: &str, port: u16) -> Option<String> {
    let host = host.trim();
    if host.is_empty() || host == "0.0.0.0" || host == "::" {
        return None;
    }
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    Some(format!("<sip:{host}:{port};transport=udp;lr>"))
}

pub struct DialogPathModule {
    record_route: Option<String>,
}

impl DialogPathModule {
    pub fn create(
        _server: SipServerRef,
        _config: Arc<ProxyConfig>,
    ) -> Result<Box<dyn ProxyModule>> {
        Ok(Box::new(Self {
            record_route: RECORD_ROUTE.get().cloned().unwrap_or_default(),
        }))
    }
}

#[async_trait]
impl ProxyModule for DialogPathModule {
    fn name(&self) -> &str {
        "dialog-path"
    }

    fn allow_methods(&self) -> Vec<Method> {
        vec![Method::Invite]
    }

    async fn on_start(&mut self) -> Result<()> {
        debug!(record_route = ?self.record_route, "edge dialog-path module started");
        Ok(())
    }

    async fn on_stop(&self) -> Result<()> {
        Ok(())
    }

    async fn on_transaction_begin(
        &self,
        _token: CancellationToken,
        _tx: &mut Transaction,
        _cookie: TransactionCookie,
    ) -> Result<ProxyAction> {
        Ok(ProxyAction::Continue)
    }

    async fn on_transaction_end(&self, tx: &mut Transaction) -> Result<()> {
        if tx.original.method != Method::Invite {
            return Ok(());
        }
        let Some(record_route) = self.record_route.as_deref() else {
            return Ok(());
        };
        let Some(response) = tx.last_response.as_mut() else {
            return Ok(());
        };
        if response.status_code().code() / 100 != 2 {
            return Ok(());
        }
        if response.headers.iter().any(|header| match header {
            rsipstack::sip::Header::Other(name, value) => {
                name.eq_ignore_ascii_case("Record-Route") && value == record_route
            }
            _ => false,
        }) {
            return Ok(());
        }
        response.headers.push_front(rsipstack::sip::Header::Other(
            "Record-Route".into(),
            record_route.to_string(),
        ));
        debug!(record_route, "added edge Record-Route");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_record_route() {
        assert_eq!(
            build_record_route("203.0.113.7", 5060).as_deref(),
            Some("<sip:203.0.113.7:5060;transport=udp;lr>")
        );
        assert_eq!(
            build_record_route("2001:db8::7", 5060).as_deref(),
            Some("<sip:[2001:db8::7]:5060;transport=udp;lr>")
        );
        assert!(build_record_route("0.0.0.0", 5060).is_none());
    }
}
