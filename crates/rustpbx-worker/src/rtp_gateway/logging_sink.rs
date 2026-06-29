//! `LoggingSink` — a `CallCommandSink` that records translated commands
//! without executing them.
//!
//! Used in Phase 1 skeleton integration: the WorkerCallModule creates an
//! rtp_gateway per call, the bridge translates `MediaCommand`s into
//! `CallCommand`s, and this sink logs them. Phase 2 replaces this with a
//! real media-thread sink.

use super::bridge::CallCommandSink;
use anyhow::Result;
use rustpbx::call::domain::CallCommand;
use std::sync::{Arc, Mutex};

/// Sink that logs every CallCommand it receives. Thread-safe; cloned sinks
/// share the same command log.
#[derive(Clone)]
pub struct LoggingSink {
    id: String,
    log: Arc<Mutex<Vec<CallCommand>>>,
}

impl LoggingSink {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Snapshot of all commands received so far (for testing / introspection).
    #[allow(dead_code)]
    pub fn commands(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().map(command_label).collect()
    }

    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.log.lock().unwrap().len()
    }
}

impl CallCommandSink for LoggingSink {
    fn send(&self, cmd: CallCommand) -> Result<()> {
        let label = command_label(&cmd);
        tracing::info!(sink_id = %self.id, command = %label, "rtp_gateway command (logged)");
        self.log.lock().unwrap().push(cmd);
        Ok(())
    }

    fn id(&self) -> &str {
        &self.id
    }
}

fn command_label(cmd: &CallCommand) -> String {
    match cmd {
        CallCommand::Answer { .. } => "answer".into(),
        CallCommand::Hangup(_) => "hangup".into(),
        CallCommand::Play { .. } => "play".into(),
        CallCommand::StopPlayback { .. } => "stop_playback".into(),
        CallCommand::StartRecording { .. } => "start_recording".into(),
        CallCommand::StopRecording => "stop_recording".into(),
        CallCommand::SendDtmf { digits, .. } => format!("send_dtmf({})", digits),
        CallCommand::MuteTrack { .. } => "mute_track".into(),
        CallCommand::UnmuteTrack { .. } => "unmute_track".into(),
        CallCommand::Reject { .. } => "reject".into(),
        CallCommand::Ring { .. } => "ring".into(),
        CallCommand::Bridge { .. } => "bridge".into(),
        CallCommand::Unbridge { .. } => "unbridge".into(),
        _ => "other".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustpbx::call::domain::{HangupCommand, LegId};

    #[test]
    fn logs_play_command() {
        let sink = LoggingSink::new("test");
        sink.send(CallCommand::StopPlayback { leg_id: None })
            .unwrap();
        sink.send(CallCommand::StopRecording).unwrap();
        assert_eq!(sink.count(), 2);
        assert_eq!(sink.commands(), vec!["stop_playback", "stop_recording"]);
    }

    #[test]
    fn cloned_sinks_share_log() {
        let sink = LoggingSink::new("shared");
        let clone = sink.clone();
        sink.send(CallCommand::Hangup(HangupCommand::all(None, None)))
            .unwrap();
        assert_eq!(clone.count(), 1);
    }

    #[test]
    fn id_is_stable() {
        let sink = LoggingSink::new("my-sink");
        assert_eq!(sink.id(), "my-sink");
    }

    #[test]
    fn dtmf_label_includes_digits() {
        let sink = LoggingSink::new("t");
        sink.send(CallCommand::SendDtmf {
            leg_id: LegId::from("caller"),
            digits: "123".into(),
        })
        .unwrap();
        assert_eq!(sink.commands(), vec!["send_dtmf(123)"]);
    }
}
