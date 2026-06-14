//! `RtpGatewayBridge` — translates `MediaCommand` ↔ `CallCommand`.
//!
//! Phase 1 design: the bridge is a background task that
//! 1. Receives `MediaCommand`s from the `RtpGatewayHandle`
//! 2. Translates each into the main crate's `CallCommand`
//! 3. Forwards via a [`CallCommandSink`] (abstract — see impls below)
//!
//! The main crate's `SipSessionHandle` is `pub(crate)` so the Worker can't
//! reference it directly. Instead, any code with access to a session
//! (inside the main crate's call flow) implements `CallCommandSink` and
//! passes it to `spawn_bridge`.
//!
//! Phase 2 will add a `MediaThreadSink` that forwards commands to a
//! dedicated media thread instead of the session.

use super::command::{MediaCommand, MediaEvent, MediaSourceSpec, RecordingFormat, TrackSelector};
use super::handle::RtpGatewayHandle;
use anyhow::{Error, Result};
use rustpbx::call::domain::{CallCommand, LegId, MediaSource, PlayOptions, RecordConfig};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Abstracts "where CallCommands go". Implementations:
/// - `ChannelCallSink` — wraps an `mpsc::Sender<CallCommand>` (testing,
///   Phase-1 integration from inside the main crate)
/// - Phase 2: `MediaThreadCallSink` — forwards to the dedicated media thread
pub trait CallCommandSink: Send + Sync + 'static {
    /// Forward a translated CallCommand. Return Err to signal the bridge
    /// should stop (e.g. session closed).
    fn send(&self, cmd: CallCommand) -> Result<()>;

    /// Human-readable identifier for logging (session_id, thread name, etc.).
    fn id(&self) -> &str;
}

/// Sink backed by an `mpsc::Sender<CallCommand>`. Used in tests and by
/// integrators that already own a CallCommand channel.
pub struct ChannelCallSink {
    id: String,
    tx: mpsc::Sender<CallCommand>,
}

impl ChannelCallSink {
    pub fn new(id: impl Into<String>, tx: mpsc::Sender<CallCommand>) -> Self {
        Self { id: id.into(), tx }
    }
}

impl CallCommandSink for ChannelCallSink {
    fn send(&self, cmd: CallCommand) -> Result<()> {
        self.tx
            .try_send(cmd)
            .map_err(|e| Error::msg(format!("call-command channel: {e}")))
    }
    fn id(&self) -> &str {
        &self.id
    }
}

/// Spawn a bridge task. Returns a `RtpGatewayHandle` for business logic.
///
/// The bridge task terminates when:
/// - The handle (and all clones) are dropped (command channel closes)
/// - `MediaCommand::Teardown` is received
/// - The sink returns an error on send (fatal)
pub fn spawn_bridge(sink: Arc<dyn CallCommandSink>) -> RtpGatewayHandle {
    let (handle, mut cmd_rx, _event_rx) = RtpGatewayHandle::new_pair();
    let event_tx = handle.event_sender();
    let sink_id = sink.id().to_string();

    tokio::spawn(async move {
        info!(sink_id = %sink_id, "rtp_gateway bridge started");

        while let Some(cmd) = cmd_rx.recv().await {
            debug!(sink_id = %sink_id, command = cmd.name(), "translating");
            let is_teardown = matches!(cmd, MediaCommand::Teardown);

            match translate_command(&cmd) {
                Ok(call_cmd) => {
                    if let Err(e) = sink.send(call_cmd) {
                        warn!(sink_id = %sink_id, error = %e, "sink send failed — stopping bridge");
                        let _ = event_tx.send(MediaEvent::MediaError {
                            error: format!("sink: {e}"),
                            fatal: true,
                        });
                        break;
                    }
                }
                Err(e) => {
                    // Phase-1 stubs (InjectPcm, Renegotiate) land here — non-fatal.
                    debug!(sink_id = %sink_id, error = %e, "translate error (non-fatal)");
                    let _ = event_tx.send(MediaEvent::MediaError {
                        error: format!("translate: {e}"),
                        fatal: false,
                    });
                }
            }

            if is_teardown {
                debug!(sink_id = %sink_id, "teardown — bridge exiting");
                break;
            }
        }

        info!(sink_id = %sink_id, "rtp_gateway bridge stopped");
    });

    handle
}

/// Translate a `MediaCommand` into the main crate's `CallCommand`.
///
/// Pure function — no I/O, safe to unit-test without a runtime.
pub(crate) fn translate_command(cmd: &MediaCommand) -> Result<CallCommand> {
    match cmd {
        MediaCommand::StartRecording {
            path,
            format,
            max_duration_secs,
            beep,
            ..
        } => Ok(CallCommand::StartRecording {
            config: RecordConfig {
                path: path.to_string_lossy().into_owned(),
                max_duration_secs: *max_duration_secs,
                beep: *beep,
                format: match format {
                    RecordingFormat::Wav => None,
                    RecordingFormat::OpusOgg => Some("opus".to_string()),
                },
            },
        }),

        MediaCommand::StopRecording => Ok(CallCommand::StopRecording),

        MediaCommand::Play {
            source,
            track_id,
            loop_playback,
            interrupt_on_dtmf,
            ..
        } => Ok(CallCommand::Play {
            leg_id: None,
            source: match source {
                MediaSourceSpec::File(p) => MediaSource::File {
                    path: p.to_string_lossy().into_owned(),
                },
                MediaSourceSpec::Url(u) => MediaSource::Url { url: u.clone() },
            },
            options: Some(PlayOptions {
                loop_playback: *loop_playback,
                await_completion: false,
                interrupt_on_dtmf: *interrupt_on_dtmf,
                track_id: track_id.clone(),
                send_progress: false,
            }),
        }),

        MediaCommand::StopPlay => Ok(CallCommand::StopPlayback { leg_id: None }),

        MediaCommand::SendDtmf { digit, track, .. } => Ok(CallCommand::SendDtmf {
            leg_id: leg_id_from_track(*track),
            digits: digit.to_string(),
        }),

        MediaCommand::Mute { track, muted } => {
            let track_id = format!("audio-{}", track.as_str());
            Ok(if *muted {
                CallCommand::MuteTrack { track_id }
            } else {
                CallCommand::UnmuteTrack { track_id }
            })
        }

        // Phase 1 stubs — require the Phase-2 media thread.
        MediaCommand::InjectPcmStart { .. }
        | MediaCommand::InjectPcmChunk { .. }
        | MediaCommand::InjectPcmStop
        | MediaCommand::Renegotiate { .. } => Err(Error::msg(format!(
            "command `{}` requires phase-2 media thread",
            cmd.name()
        ))),

        MediaCommand::Teardown => Ok(CallCommand::Hangup(
            rustpbx::call::domain::HangupCommand::all(None, None),
        )),
    }
}

fn leg_id_from_track(track: TrackSelector) -> LegId {
    match track {
        TrackSelector::Caller | TrackSelector::Both => LegId::from("caller"),
        TrackSelector::Callee => LegId::from("callee"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn translate_start_recording_wav() {
        let cmd = MediaCommand::StartRecording {
            path: PathBuf::from("/tmp/rec.wav"),
            format: RecordingFormat::Wav,
            tracks: TrackSelector::Both,
            max_duration_secs: Some(60),
            beep: true,
        };
        match translate_command(&cmd).unwrap() {
            CallCommand::StartRecording { config } => {
                assert_eq!(config.path, "/tmp/rec.wav");
                assert_eq!(config.max_duration_secs, Some(60));
                assert!(config.beep);
                assert!(config.format.is_none());
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn translate_start_recording_opus() {
        let cmd = MediaCommand::StartRecording {
            path: PathBuf::from("/tmp/rec.ogg"),
            format: RecordingFormat::OpusOgg,
            tracks: TrackSelector::Both,
            max_duration_secs: None,
            beep: false,
        };
        match translate_command(&cmd).unwrap() {
            CallCommand::StartRecording { config } => {
                assert_eq!(config.format.as_deref(), Some("opus"));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn translate_play_file_and_url() {
        let file_cmd = MediaCommand::Play {
            source: MediaSourceSpec::File(PathBuf::from("/tmp/x.wav")),
            track_id: Some("t1".into()),
            loop_playback: false,
            interrupt_on_dtmf: true,
        };
        match translate_command(&file_cmd).unwrap() {
            CallCommand::Play { source, options, leg_id } => {
                assert!(leg_id.is_none());
                assert!(matches!(source, MediaSource::File { .. }));
                assert_eq!(options.unwrap().track_id.as_deref(), Some("t1"));
            }
            other => panic!("got {other:?}"),
        }

        let url_cmd = MediaCommand::Play {
            source: MediaSourceSpec::Url("https://x.com/a.wav".into()),
            track_id: None,
            loop_playback: true,
            interrupt_on_dtmf: false,
        };
        match translate_command(&url_cmd).unwrap() {
            CallCommand::Play { source, options, .. } => {
                assert!(matches!(source, MediaSource::Url { .. }));
                assert!(options.unwrap().loop_playback);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn translate_dtmf_and_mute() {
        let dtmf = MediaCommand::SendDtmf {
            digit: '5',
            duration_ms: 200,
            track: TrackSelector::Caller,
        };
        match translate_command(&dtmf).unwrap() {
            CallCommand::SendDtmf { digits, leg_id } => {
                assert_eq!(digits, "5");
                assert_eq!(leg_id, LegId::from("caller"));
            }
            other => panic!("got {other:?}"),
        }

        assert!(matches!(
            translate_command(&MediaCommand::Mute {
                track: TrackSelector::Caller,
                muted: true,
            })
            .unwrap(),
            CallCommand::MuteTrack { .. }
        ));
        assert!(matches!(
            translate_command(&MediaCommand::Mute {
                track: TrackSelector::Callee,
                muted: false,
            })
            .unwrap(),
            CallCommand::UnmuteTrack { .. }
        ));
    }

    #[test]
    fn phase1_stubs_return_error() {
        let cmds = [
            MediaCommand::InjectPcmStart {
                sample_rate: 16000,
                track: TrackSelector::Caller,
            },
            MediaCommand::InjectPcmChunk { samples: vec![0i16; 160] },
            MediaCommand::InjectPcmStop,
            MediaCommand::Renegotiate { offer_sdp: vec![] },
        ];
        for cmd in &cmds {
            assert!(translate_command(cmd).is_err(), "{cmd:?} should error");
        }
    }

    #[test]
    fn teardown_maps_to_hangup() {
        assert!(matches!(
            translate_command(&MediaCommand::Teardown).unwrap(),
            CallCommand::Hangup(_)
        ));
    }

    #[tokio::test]
    async fn bridge_forwards_translated_command_via_sink() {
        use std::sync::Mutex;

        struct RecordingSink {
            id: String,
            received: Mutex<Vec<CallCommand>>,
        }
        impl CallCommandSink for RecordingSink {
            fn send(&self, cmd: CallCommand) -> Result<()> {
                self.received.lock().unwrap().push(cmd);
                Ok(())
            }
            fn id(&self) -> &str {
                &self.id
            }
        }

        let sink = Arc::new(RecordingSink {
            id: "test-sink".into(),
            received: Mutex::new(Vec::new()),
        });
        let received_ptr = Arc::clone(&sink);

        let handle = spawn_bridge(sink);
        handle.stop_recording().await.unwrap();
        handle.teardown().await.unwrap();

        // Give the bridge task time to process.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let received = received_ptr.received.lock().unwrap();
        assert!(received.iter().any(|c| matches!(c, CallCommand::StopRecording)));
        assert!(received.iter().any(|c| matches!(c, CallCommand::Hangup(_))));
    }

    #[tokio::test]
    async fn channel_sink_forwards_to_receiver() {
        let (tx, mut rx) = mpsc::channel::<CallCommand>(8);
        let sink = Arc::new(ChannelCallSink::new("ch-sink", tx));

        let handle = spawn_bridge(Arc::clone(&sink) as Arc<dyn CallCommandSink>);
        handle.stop_play().await.unwrap();

        match rx.recv().await.unwrap() {
            CallCommand::StopPlayback { .. } => {}
            other => panic!("expected StopPlayback, got {other:?}"),
        }
    }
}

