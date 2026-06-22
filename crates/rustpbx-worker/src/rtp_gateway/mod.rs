//! `rtp_gateway` — the media I/O boundary for the Worker.
//!
//! # Purpose
//!
//! All time-consuming and network I/O media operations (RTP send/receive,
//! codec transcoding, DTMF detection, recording file writes, audio file
//! reads) are encapsulated within this module. Business logic layers
//! (`call_session`, `ivr_engine`) interact with media exclusively through
//! [`RtpGatewayHandle`] — they never import `rustpbx::media::*` directly.
//!
//! # Phase 1 (current)
//!
//! The bridge translates [`MediaCommand`]s into main-crate `CallController`
//! calls. The channel boundary exists in code structure but media still runs
//! inside the main crate's `SipSession` (tokio-hosted).
//!
//! # Phase 2 (planned)
//!
//! RTP send/receive and codec processing move to a dedicated media thread,
//! communicating with tokio via SPSC channels. This removes media from
//! tokio's scheduler and makes latency deterministic.
//!
//! # Architecture
//!
//! ```text
//! ┌───────────────────────┐                ┌─────────────────────────┐
//! │  call_session /       │  MediaCommand  │                         │
//! │  ivr_engine           │───────────────▶│    rtp_gateway          │
//! │  (business logic)     │◀───────────────│                         │
//! └───────────────────────┘  MediaEvent    │  ┌───────────────────┐  │
//!                                          │  │ RtpGatewayBridge  │  │
//!                                          │  │ (translates cmds) │  │
//!                                          │  └────────┬──────────┘  │
//!                                          │           │              │
//!                                          │  ┌────────▼──────────┐  │
//!                                          │  │ [P2] media thread │  │
//!                                          │  │ RTP / codec / IO  │  │
//!                                          │  └───────────────────┘  │
//!                                          └─────────────────────────┘
//! ```

#[allow(dead_code)]
pub mod bridge;
#[allow(dead_code)]
pub mod command;
#[allow(dead_code)]
pub mod handle;
pub mod logging_sink;

#[allow(dead_code, unused_imports)]
pub use bridge::{CallCommandSink, ChannelCallSink, spawn_bridge};
#[allow(dead_code, unused_imports)]
pub use command::{
    MediaCommand, MediaEvent, MediaSourceSpec, QualityReport, RecordingFormat, RecordingResult,
    TrackSelector,
};
pub use handle::RtpGatewayHandle;
pub use logging_sink::LoggingSink;
