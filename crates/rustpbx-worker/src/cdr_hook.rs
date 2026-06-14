/// GrpcCdrHook implements CallRecordHook.
///
/// After each call completes, converts the CallRecord to a protobuf
/// CallRecordReport and sends it to the Control Plane via gRPC.
/// Falls back to a warning log if the Control Plane is unreachable.
use crate::{
    control_client::ControlClient,
    proto::control::CallRecordReport,
};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use rustpbx::callrecord::{CallRecord, CallRecordHook};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

pub struct GrpcCdrHook {
    /// Shared Control Plane client (Mutex because gRPC client is not Clone).
    client: Arc<Mutex<ControlClient>>,
    worker_id: String,
}

impl GrpcCdrHook {
    pub fn new(client: Arc<Mutex<ControlClient>>, worker_id: String) -> Self {
        Self { client, worker_id }
    }
}

#[async_trait]
impl CallRecordHook for GrpcCdrHook {
    async fn on_record_completed(&self, record: &mut CallRecord) -> Result<()> {
        let report = cdr_to_proto(record, &self.worker_id);
        info!(
            call_id = %report.call_id,
            status = %report.status,
            duration = report.duration_secs,
            "uploading CDR to control plane"
        );

        let mut client = self.client.lock().await;
        if let Err(e) = client.report_cdr(report).await {
            warn!(error = %e, "CDR upload failed — call record will be lost");
        }

        Ok(())
    }
}

// ── Converter ─────────────────────────────────────────────────────────────────

fn cdr_to_proto(r: &CallRecord, worker_id: &str) -> CallRecordReport {
    let duration_secs = (r.end_time - r.start_time).num_seconds().max(0) as i32;

    let tenant_id = r
        .extensions
        .get::<rustpbx::call::TenantId>()
        .map(|t| t.0);

    let trunk_name = r
        .extensions
        .get::<rustpbx::call::TrunkContext>()
        .map(|t| t.name.clone());

    let hangup_cause = r
        .hangup_reason
        .as_ref()
        .map(|_| r.status_code as u32);

    CallRecordReport {
        call_id: r.call_id.clone(),
        tenant_id,
        caller: r.caller.clone(),
        callee: r.callee.clone(),
        direction: r.details.direction.clone(),
        status: r.details.status.clone(),
        start_time_unix_ms: r.start_time.timestamp_millis(),
        answer_time_unix_ms: r
            .answer_time
            .map(|t| t.timestamp_millis())
            .unwrap_or(0),
        end_time_unix_ms: r.end_time.timestamp_millis(),
        duration_secs,
        trunk_name,
        worker_id: Some(worker_id.to_string()),
        edge_id: None,
        recording_path: r.recorder.first().map(|m| m.path.clone()),
        metadata: Default::default(),
        hangup_cause,
    }
}
