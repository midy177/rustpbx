//! Dedicated media-thread command sink.
//!
//! This is the Phase-2 boundary: the tokio-side bridge can hand translated
//! `CallCommand`s to a per-call OS thread. The current loop only records and
//! logs commands; RTP sockets, codec processing, and PCM injection can replace
//! the loop body without changing business-side `RtpGatewayHandle` callers.

use super::bridge::CallCommandSink;
use anyhow::{Result, anyhow};
use rustpbx::call::domain::CallCommand;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use tracing::{debug, info};

enum MediaThreadMessage {
    Command(CallCommand),
    Shutdown,
}

/// Sink that forwards translated call commands onto a dedicated media thread.
pub struct MediaThreadCallSink {
    id: String,
    tx: mpsc::Sender<MediaThreadMessage>,
    processed: Arc<AtomicUsize>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl MediaThreadCallSink {
    pub fn spawn(id: impl Into<String>) -> Self {
        let id = id.into();
        let (tx, rx) = mpsc::channel();
        let processed = Arc::new(AtomicUsize::new(0));
        let thread_id = id.clone();
        let thread_processed = Arc::clone(&processed);

        let join = thread::Builder::new()
            .name(format!("rtp-gateway-{thread_id}"))
            .spawn(move || media_thread_loop(thread_id, rx, thread_processed))
            .expect("spawn rtp_gateway media thread");

        Self {
            id,
            tx,
            processed,
            join: Mutex::new(Some(join)),
        }
    }

    #[cfg(test)]
    fn processed_count(&self) -> usize {
        self.processed.load(Ordering::Relaxed)
    }
}

impl CallCommandSink for MediaThreadCallSink {
    fn send(&self, cmd: CallCommand) -> Result<()> {
        self.tx
            .send(MediaThreadMessage::Command(cmd))
            .map_err(|e| anyhow!("media thread channel closed: {e}"))
    }

    fn id(&self) -> &str {
        &self.id
    }
}

impl Drop for MediaThreadCallSink {
    fn drop(&mut self) {
        let _ = self.tx.send(MediaThreadMessage::Shutdown);
        if let Some(join) = self.join.lock().unwrap().take() {
            let _ = join.join();
        }
    }
}

fn media_thread_loop(
    id: String,
    rx: mpsc::Receiver<MediaThreadMessage>,
    processed: Arc<AtomicUsize>,
) {
    info!(sink_id = %id, "rtp_gateway media thread started");
    while let Ok(message) = rx.recv() {
        match message {
            MediaThreadMessage::Command(cmd) => {
                debug!(sink_id = %id, command = command_label(&cmd), "media thread command");
                processed.fetch_add(1, Ordering::Relaxed);
                if matches!(cmd, CallCommand::Hangup(_)) {
                    break;
                }
            }
            MediaThreadMessage::Shutdown => break,
        }
    }
    info!(sink_id = %id, "rtp_gateway media thread stopped");
}

fn command_label(cmd: &CallCommand) -> &'static str {
    match cmd {
        CallCommand::Answer { .. } => "answer",
        CallCommand::Hangup(_) => "hangup",
        CallCommand::Play { .. } => "play",
        CallCommand::StopPlayback { .. } => "stop_playback",
        CallCommand::StartRecording { .. } => "start_recording",
        CallCommand::StopRecording => "stop_recording",
        CallCommand::SendDtmf { .. } => "send_dtmf",
        CallCommand::MuteTrack { .. } => "mute_track",
        CallCommand::UnmuteTrack { .. } => "unmute_track",
        CallCommand::Reject { .. } => "reject",
        CallCommand::Ring { .. } => "ring",
        CallCommand::Bridge { .. } => "bridge",
        CallCommand::Unbridge { .. } => "unbridge",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustpbx::call::domain::HangupCommand;

    #[test]
    fn media_thread_sink_processes_commands() {
        let sink = MediaThreadCallSink::spawn("test");
        sink.send(CallCommand::StopRecording).unwrap();

        for _ in 0..50 {
            if sink.processed_count() >= 1 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("media thread did not process command");
    }

    #[test]
    fn hangup_stops_media_thread() {
        let sink = MediaThreadCallSink::spawn("hangup");
        sink.send(CallCommand::Hangup(HangupCommand::all(None, None)))
            .unwrap();

        for _ in 0..50 {
            if sink.processed_count() >= 1 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("media thread did not process hangup");
    }
}
