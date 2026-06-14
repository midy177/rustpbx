//! `RtpGatewayHandle` — the business-facing API for media operations.
//!
//! Cloneable and shareable across tasks. Business logic (IVR, queue,
//! recording triggers) holds a handle and issues `MediaCommand`s.
//! Media-originated information arrives via `subscribe()`.

use super::command::{MediaCommand, MediaEvent};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tracing::debug;

/// Default capacity for the command channel.
/// Commands are small enums; 64 is enough headroom for bursty IVR flows.
const CMD_CHANNEL_CAPACITY: usize = 64;

/// Default capacity for the event broadcast channel.
/// Broadcast so multiple subscribers (IVR + metrics + recording mgr) all see events.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Business-side handle to the rtp_gateway.
///
/// Clone it freely — one per concurrent task (IVR event loop, recording
/// manager, metrics collector). All clones share the same underlying
/// command channel and event broadcast.
#[derive(Clone)]
pub struct RtpGatewayHandle {
    cmd_tx: mpsc::Sender<MediaCommand>,
    event_tx: broadcast::Sender<MediaEvent>,
}

impl RtpGatewayHandle {
    /// Create a connected `(handle, command_receiver, event_receiver)` triple.
    ///
    /// The caller (typically `RtpGatewayBridge::spawn`) holds the receivers
    /// and processes commands / emits events. The handle is handed to
    /// business logic.
    pub fn new_pair() -> (
        RtpGatewayHandle,
        mpsc::Receiver<MediaCommand>,
        broadcast::Receiver<MediaEvent>,
    ) {
        let (cmd_tx, cmd_rx) = mpsc::channel(CMD_CHANNEL_CAPACITY);
        let (event_tx, event_rx) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let handle = RtpGatewayHandle { cmd_tx, event_tx };
        (handle, cmd_rx, event_rx)
    }

    /// Internal constructor used by the bridge (which owns the receivers).
    pub(crate) fn from_channels(
        cmd_tx: mpsc::Sender<MediaCommand>,
        event_tx: broadcast::Sender<MediaEvent>,
    ) -> Self {
        Self { cmd_tx, event_tx }
    }

    // ── Low-level channel access ────────────────────────────────────────────

    /// Send a raw `MediaCommand`. Async — applies backpressure when the
    /// command channel is full.
    pub async fn send(&self, cmd: MediaCommand) -> Result<(), SendError<MediaCommand>> {
        debug!(command = cmd.name(), "rtp_gateway command sent");
        self.cmd_tx.send(cmd).await
    }

    /// Non-blocking send. Returns `Err(Full)` if the channel is at capacity.
    pub fn try_send(&self, cmd: MediaCommand) -> Result<(), TrySendError<MediaCommand>> {
        self.cmd_tx.try_send(cmd)
    }

    /// Subscribe to the event broadcast. Each subscriber gets its own queue.
    pub fn subscribe(&self) -> broadcast::Receiver<MediaEvent> {
        self.event_tx.subscribe()
    }

    /// Sender clone — used by the bridge to inject events from the media side.
    pub(crate) fn event_sender(&self) -> broadcast::Sender<MediaEvent> {
        self.event_tx.clone()
    }

    // ── Convenience wrappers for common commands ────────────────────────────
    //
    // These exist so business code reads naturally. Each just builds the
    // corresponding MediaCommand and sends it.

    /// Start recording to `path`. Returns immediately; completion arrives as
    /// `MediaEvent::RecordingCompleted`.
    pub async fn start_recording(
        &self,
        path: impl Into<PathBuf>,
    ) -> Result<(), SendError<MediaCommand>> {
        self.send(MediaCommand::StartRecording {
            path: path.into(),
            format: Default::default(),
            tracks: super::command::TrackSelector::Both,
            max_duration_secs: None,
            beep: false,
        })
        .await
    }

    /// Stop the active recording.
    pub async fn stop_recording(&self) -> Result<(), SendError<MediaCommand>> {
        self.send(MediaCommand::StopRecording).await
    }

    /// Play a file (local path or URL). Returns the track_id assigned.
    pub async fn play(
        &self,
        source: impl Into<super::command::MediaSourceSpec>,
        interrupt_on_dtmf: bool,
    ) -> Result<String, SendError<MediaCommand>> {
        let track_id = uuid::Uuid::new_v4().to_string();
        self.send(MediaCommand::Play {
            source: source.into(),
            track_id: Some(track_id.clone()),
            loop_playback: false,
            interrupt_on_dtmf,
        })
        .await?;
        Ok(track_id)
    }

    /// Stop current playback.
    pub async fn stop_play(&self) -> Result<(), SendError<MediaCommand>> {
        self.send(MediaCommand::StopPlay).await
    }

    /// Send an outbound DTMF digit.
    pub async fn send_dtmf(
        &self,
        digit: char,
        duration: Duration,
    ) -> Result<(), SendError<MediaCommand>> {
        self.send(MediaCommand::SendDtmf {
            digit,
            duration_ms: duration.as_millis() as u32,
            track: super::command::TrackSelector::Caller,
        })
        .await
    }

    /// Mute / unmute the caller's outbound audio.
    pub async fn mute_caller(&self, muted: bool) -> Result<(), SendError<MediaCommand>> {
        self.send(MediaCommand::Mute {
            track: super::command::TrackSelector::Caller,
            muted,
        })
        .await
    }

    /// Signal teardown — rtp_gateway releases all resources.
    pub async fn teardown(&self) -> Result<(), SendError<MediaCommand>> {
        self.send(MediaCommand::Teardown).await
    }
}

/// Re-export tokio send error for convenience.
pub use tokio::sync::mpsc::error::{SendError, TrySendError};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handle_delivers_command_to_receiver() {
        let (handle, mut cmd_rx, _event_rx) = RtpGatewayHandle::new_pair();
        handle.stop_recording().await.unwrap();

        match cmd_rx.recv().await.unwrap() {
            MediaCommand::StopRecording => {}
            other => panic!("expected StopRecording, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn event_broadcast_reaches_multiple_subscribers() {
        let (handle, _cmd_rx, _event_rx) = RtpGatewayHandle::new_pair();
        let mut sub1 = handle.subscribe();
        let mut sub2 = handle.subscribe();

        handle
            .event_sender()
            .send(MediaEvent::PlayCompleted { track_id: "t1".into() })
            .unwrap();

        assert!(matches!(
            sub1.recv().await.unwrap(),
            MediaEvent::PlayCompleted { .. }
        ));
        assert!(matches!(
            sub2.recv().await.unwrap(),
            MediaEvent::PlayCompleted { .. }
        ));
    }

    #[tokio::test]
    async fn play_returns_track_id() {
        let (handle, mut cmd_rx, _event_rx) = RtpGatewayHandle::new_pair();
        let id = handle.play("/tmp/x.wav", false).await.unwrap();
        assert!(!id.is_empty());

        match cmd_rx.recv().await.unwrap() {
            MediaCommand::Play { track_id, .. } => {
                assert_eq!(track_id.as_deref(), Some(id.as_str()));
            }
            other => panic!("expected Play, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn backpressure_when_channel_full() {
        // Use a 1-capacity channel by constructing manually.
        let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = broadcast::channel(4);
        let handle = RtpGatewayHandle::from_channels(cmd_tx, event_tx);

        // Fill the channel.
        handle.try_send(MediaCommand::StopPlay).unwrap();
        // Next try_send should fail with Full.
        let err = handle.try_send(MediaCommand::StopPlay).unwrap_err();
        assert!(matches!(err, TrySendError::Full(_)));

        // Drain and retry succeeds.
        cmd_rx.recv().await.unwrap();
        handle.try_send(MediaCommand::StopPlay).unwrap();
    }
}
