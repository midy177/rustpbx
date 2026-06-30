/// GrpcCdrHook implements CallRecordHook.
///
/// After each call completes, converts the CallRecord to a protobuf
/// CallRecordReport and sends it to the Control Plane via gRPC.
/// Spools to disk if the Control Plane is unreachable.
use crate::{
    control_client::ControlClient,
    proto::control::CallRecordReport,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use prost::Message;
use rustpbx::callrecord::{CallRecord, CallRecordHook};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::Mutex;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const CDR_REPLAY_INTERVAL_SECS: u64 = 10;

pub struct GrpcCdrHook {
    /// Shared Control Plane client (Mutex because gRPC client is not Clone).
    client: Arc<Mutex<ControlClient>>,
    worker_id: String,
    spool: CdrSpool,
}

impl GrpcCdrHook {
    pub fn new(client: Arc<Mutex<ControlClient>>, worker_id: String, spool: CdrSpool) -> Self {
        Self {
            client,
            worker_id,
            spool,
        }
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

        let upload = {
            let mut client = self.client.lock().await;
            client.report_cdr(report.clone()).await
        };
        if let Err(e) = upload {
            warn!(error = %e, "CDR upload failed — spooling for retry");
            if let Err(spool_err) = self.spool.enqueue(&report).await {
                warn!(error = %spool_err, call_id = %report.call_id, "CDR spool write failed");
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct CdrSpool {
    dir: Arc<PathBuf>,
}

impl CdrSpool {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: Arc::new(dir.into()),
        }
    }

    pub async fn enqueue(&self, report: &CallRecordReport) -> Result<PathBuf> {
        fs::create_dir_all(self.dir.as_ref()).await?;
        let path = self.dir.join(format!(
            "{}-{}-{}.pb",
            chrono::Utc::now().timestamp_millis(),
            std::process::id(),
            sanitize_file_component(&report.call_id)
        ));
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, report.encode_to_vec())
            .await
            .with_context(|| format!("write CDR spool temp file {}", tmp.display()))?;
        fs::rename(&tmp, &path)
            .await
            .with_context(|| format!("commit CDR spool file {}", path.display()))?;
        Ok(path)
    }

    pub async fn replay_once(&self, client: &Arc<Mutex<ControlClient>>) -> Result<u32> {
        fs::create_dir_all(self.dir.as_ref()).await?;
        let mut entries = fs::read_dir(self.dir.as_ref()).await?;
        let mut sent = 0;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("pb") {
                continue;
            }
            let bytes = match fs::read(&path).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "failed to read spooled CDR");
                    continue;
                }
            };
            let report = match CallRecordReport::decode(bytes.as_slice()) {
                Ok(report) => report,
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "invalid spooled CDR; moving aside");
                    move_bad_spool_file(&path).await.ok();
                    continue;
                }
            };
            let call_id = report.call_id.clone();
            let upload = {
                let mut client = client.lock().await;
                client.report_cdr(report).await
            };
            match upload {
                Ok(()) => {
                    if let Err(e) = fs::remove_file(&path).await {
                        warn!(error = %e, path = %path.display(), "failed to remove sent CDR spool file");
                    } else {
                        sent += 1;
                        info!(call_id = %call_id, path = %path.display(), "replayed spooled CDR");
                    }
                }
                Err(e) => {
                    warn!(error = %e, call_id = %call_id, "spooled CDR replay failed");
                    break;
                }
            }
        }
        Ok(sent)
    }

    pub async fn run_replay_loop(
        self,
        client: Arc<Mutex<ControlClient>>,
        cancel: CancellationToken,
    ) {
        let mut ticker = tokio::time::interval(Duration::from_secs(CDR_REPLAY_INTERVAL_SECS));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {
                    match self.replay_once(&client).await {
                        Ok(n) if n > 0 => info!(count = n, "replayed spooled CDRs"),
                        Ok(_) => {}
                        Err(e) => warn!(error = %e, "CDR spool replay failed"),
                    }
                }
            }
        }
    }
}

async fn move_bad_spool_file(path: &Path) -> Result<()> {
    fs::rename(path, path.with_extension("bad")).await?;
    Ok(())
}

fn sanitize_file_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len().min(96));
    for ch in value.chars().take(96) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cdr_spool_writes_decodable_report() {
        let dir = std::env::temp_dir().join(format!("rustpbx-cdr-spool-{}", uuid::Uuid::new_v4()));
        let spool = CdrSpool::new(&dir);
        let report = CallRecordReport {
            call_id: "call/with spaces".to_string(),
            caller: "1001".to_string(),
            callee: "1002".to_string(),
            direction: "internal".to_string(),
            status: "completed".to_string(),
            duration_secs: 3,
            worker_id: Some("worker-a".to_string()),
            ..Default::default()
        };

        let path = spool.enqueue(&report).await.unwrap();
        assert_eq!(path.extension().and_then(|s| s.to_str()), Some("pb"));
        let bytes = fs::read(&path).await.unwrap();
        let decoded = CallRecordReport::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.call_id, report.call_id);
        assert_eq!(decoded.worker_id, report.worker_id);

        let _ = fs::remove_dir_all(dir).await;
    }

    #[test]
    fn sanitize_file_component_replaces_unsafe_chars() {
        assert_eq!(sanitize_file_component("call/id with spaces"), "call_id_with_spaces");
        assert_eq!(sanitize_file_component(""), "unknown");
    }
}
