/// Selects the best available Media Worker for an incoming call.
///
/// Queries the Control Plane for the current worker list on each call
/// (list is small and cached server-side).  Selects the worker with the
/// most available capacity (max_concurrent - active_calls).
use crate::grpc_client::GrpcControlClient;
use anyhow::{Result, anyhow};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct WorkerEndpoint {
    pub worker_id: String,
    /// Internal SIP contact: sip:<ip>:<port>
    pub sip_contact: String,
    pub available_capacity: u32,
}

pub struct WorkerSelector {
    client: Arc<RwLock<GrpcControlClient>>,
}

impl WorkerSelector {
    pub fn new(client: Arc<RwLock<GrpcControlClient>>) -> Self {
        Self { client }
    }

    /// Query Control Plane and return the best worker for a new call.
    pub async fn select(&self, tenant_id: Option<i64>) -> Result<WorkerEndpoint> {
        let mut client = self.client.write().await;
        let list = client.get_available_workers(tenant_id).await?;

        if list.workers.is_empty() {
            return Err(anyhow!("no available workers"));
        }

        // Workers are already sorted by available capacity (Control Plane does this).
        let best = &list.workers[0];
        let capacity = best.max_concurrent.saturating_sub(best.active_calls);

        debug!(
            worker_id = %best.worker_id,
            sip_addr = %best.sip_addr,
            active = best.active_calls,
            max = best.max_concurrent,
            "selected worker"
        );

        Ok(WorkerEndpoint {
            worker_id: best.worker_id.clone(),
            sip_contact: format!("sip:{}", best.sip_addr),
            available_capacity: capacity,
        })
    }
}
