//! Media command and event types — the channel protocol between business
//! logic (call_session / ivr_engine) and the rtp_gateway.
//!
//! **Invariant**: business logic MUST NOT import `rustpbx::media::*` directly.
//! All media operations go through `MediaCommand`. All media-originated
//! information arrives via `MediaEvent`.

use std::path::PathBuf;

/// Which leg(s) a media operation targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackSelector {
    /// The calling party (carrier-side in inbound, extension-side in outbound).
    Caller,
    /// The called party.
    Callee,
    /// Both legs — typical for recording / mixing.
    Both,
}

impl TrackSelector {
    pub fn as_str(self) -> &'static str {
        match self {
            TrackSelector::Caller => "caller",
            TrackSelector::Callee => "callee",
            TrackSelector::Both => "both",
        }
    }
}

/// Recording container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingFormat {
    /// Uncompressed WAV — default, lowest CPU.
    Wav,
    /// Opus in OGG container — smaller files, more CPU.
    OpusOgg,
}

impl Default for RecordingFormat {
    fn default() -> Self {
        RecordingFormat::Wav
    }
}

/// Source for playback operations.
#[derive(Debug, Clone)]
pub enum MediaSourceSpec {
    /// Local file path.
    File(PathBuf),
    /// HTTP/HTTPS URL (fetched on demand).
    Url(String),
}

impl From<&str> for MediaSourceSpec {
    fn from(s: &str) -> Self {
        if s.starts_with("http://") || s.starts_with("https://") {
            MediaSourceSpec::Url(s.to_string())
        } else {
            MediaSourceSpec::File(PathBuf::from(s))
        }
    }
}

/// Business → rtp_gateway commands.
///
/// Every media I/O operation MUST be expressed as a variant here.
/// Adding a new media capability = adding a variant + bridge translation.
#[derive(Debug, Clone)]
pub enum MediaCommand {
    // ── Recording ───────────────────────────────────────────────────────────
    /// Start recording the specified track(s) to `path`.
    StartRecording {
        path: PathBuf,
        format: RecordingFormat,
        tracks: TrackSelector,
        /// Hard stop after N seconds; `None` = until StopRecording.
        max_duration_secs: Option<u32>,
        /// Play a beep before recording starts.
        beep: bool,
    },
    /// Stop the active recording. rtp_gateway replies with
    /// `MediaEvent::RecordingCompleted` (or `RecordingFailed`).
    StopRecording,

    // ── Playback ────────────────────────────────────────────────────────────
    /// Play audio from a file or URL.
    Play {
        source: MediaSourceSpec,
        /// Caller-assigned ID; `None` = auto-generated UUID.
        track_id: Option<String>,
        loop_playback: bool,
        /// Whether incoming DTMF should interrupt (the app decides what to do).
        interrupt_on_dtmf: bool,
    },
    /// Stop current playback.
    StopPlay,

    // ── Real-time injection (TTS / AI voice) ────────────────────────────────
    /// Begin accepting PCM chunks on the given track.
    InjectPcmStart {
        sample_rate: u32,
        track: TrackSelector,
    },
    /// Push a PCM chunk (i16 mono samples, little-endian on the wire).
    InjectPcmChunk { samples: Vec<i16> },
    /// Finish injection and flush any buffered frames.
    InjectPcmStop,

    // ── DTMF ─────────────────────────────────────────────────────────────────
    /// Send an outbound DTMF digit (towards the network).
    SendDtmf {
        digit: char,
        duration_ms: u32,
        track: TrackSelector,
    },

    // ── Mute / hold ─────────────────────────────────────────────────────────
    /// Mute or unmute a track's outbound audio (towards the far end).
    Mute {
        track: TrackSelector,
        muted: bool,
    },

    // ── Lifecycle ────────────────────────────────────────────────────────────
    /// Renegotiate media after a re-INVITE (rtp_gateway re-evaluates codecs).
    Renegotiate { offer_sdp: Vec<u8> },
    /// Clean up all resources (RTP sockets, file handles, codecs).
    /// Always the last command before the handle is dropped.
    Teardown,
}

impl MediaCommand {
    /// Human-readable name for logging / metrics labels.
    pub fn name(&self) -> &'static str {
        match self {
            MediaCommand::StartRecording { .. } => "start_recording",
            MediaCommand::StopRecording => "stop_recording",
            MediaCommand::Play { .. } => "play",
            MediaCommand::StopPlay => "stop_play",
            MediaCommand::InjectPcmStart { .. } => "inject_pcm_start",
            MediaCommand::InjectPcmChunk { .. } => "inject_pcm_chunk",
            MediaCommand::InjectPcmStop => "inject_pcm_stop",
            MediaCommand::SendDtmf { .. } => "send_dtmf",
            MediaCommand::Mute { .. } => "mute",
            MediaCommand::Renegotiate { .. } => "renegotiate",
            MediaCommand::Teardown => "teardown",
        }
    }
}

/// Recording metadata returned when recording completes.
#[derive(Debug, Clone)]
pub struct RecordingResult {
    pub path: PathBuf,
    pub duration_secs: f64,
    pub file_size_bytes: u64,
}

/// Periodic or one-shot quality metrics for a leg.
#[derive(Debug, Clone, Default)]
pub struct QualityReport {
    pub jitter_ms: f64,
    pub loss_rate: f64,
    pub rtt_ms: Option<f64>,
    pub packets_received: u64,
    pub packets_sent: u64,
}

/// rtp_gateway → business events.
///
/// All media-originated information (DTMF detected, recording done, quality
/// reports, errors) arrives as a `MediaEvent`. Business logic subscribes via
/// `RtpGatewayHandle::subscribe()`.
#[derive(Debug, Clone)]
pub enum MediaEvent {
    /// Inbound DTMF digit detected from the far end.
    DtmfDetected {
        digit: char,
        track: TrackSelector,
    },

    /// Recording finished successfully (StopRecording or max_duration reached).
    RecordingCompleted(RecordingResult),
    /// Recording failed (disk full, permission denied, codec error).
    RecordingFailed {
        path: PathBuf,
        error: String,
    },

    /// Playback finished naturally (end of file, not interrupted).
    PlayCompleted { track_id: String },
    /// Playback was interrupted (StopPlay, new Play, or hangup).
    PlayInterrupted { track_id: String },

    /// Periodic RTP quality report (every ~5s by default).
    QualityReport(QualityReport),

    /// Codecs were negotiated after offer/answer exchange.
    CodecNegotiated {
        caller_codec: String,
        callee_codec: String,
    },

    /// Non-fatal warning (e.g., jitter buffer underrun recovered).
    MediaWarning { message: String },
    /// Fatal media error — business logic should hang up the call.
    MediaError {
        error: String,
        fatal: bool,
    },
}

impl MediaEvent {
    pub fn name(&self) -> &'static str {
        match self {
            MediaEvent::DtmfDetected { .. } => "dtmf_detected",
            MediaEvent::RecordingCompleted(_) => "recording_completed",
            MediaEvent::RecordingFailed { .. } => "recording_failed",
            MediaEvent::PlayCompleted { .. } => "play_completed",
            MediaEvent::PlayInterrupted { .. } => "play_interrupted",
            MediaEvent::QualityReport(_) => "quality_report",
            MediaEvent::CodecNegotiated { .. } => "codec_negotiated",
            MediaEvent::MediaWarning { .. } => "media_warning",
            MediaEvent::MediaError { .. } => "media_error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_name_is_stable() {
        // Names are used as Prometheus label values — must not change without
        // coordinating dashboards / alerts.
        assert_eq!(MediaCommand::StopRecording.name(), "stop_recording");
        assert_eq!(MediaCommand::Teardown.name(), "teardown");
    }

    #[test]
    fn media_source_url_vs_file() {
        assert!(matches!(
            MediaSourceSpec::from("https://x.com/a.wav"),
            MediaSourceSpec::Url(_)
        ));
        assert!(matches!(
            MediaSourceSpec::from("/tmp/a.wav"),
            MediaSourceSpec::File(_)
        ));
    }

    #[test]
    fn track_selector_roundtrip() {
        let pairs = [
            (TrackSelector::Caller, "caller"),
            (TrackSelector::Callee, "callee"),
            (TrackSelector::Both, "both"),
        ];
        for (sel, expected) in pairs {
            assert_eq!(sel.as_str(), expected);
        }
    }
}
