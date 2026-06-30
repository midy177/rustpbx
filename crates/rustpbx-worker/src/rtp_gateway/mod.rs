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
//! # Phase 2 (current boundary)
//!
//! `MediaThreadCallSink` lets a bridge forward translated commands to a
//! dedicated media thread over an SPSC channel. RTP send/receive and codec
//! processing still need to move into that loop, but the tokio-to-thread
//! boundary is now a concrete API.
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
#[allow(dead_code)]
pub mod media_thread;

#[allow(dead_code, unused_imports)]
pub use bridge::{CallCommandSink, ChannelCallSink, spawn_bridge, spawn_media_thread_bridge};
#[allow(dead_code, unused_imports)]
pub use command::{
    MediaCommand, MediaEvent, MediaSourceSpec, QualityReport, RecordingFormat, RecordingResult,
    TrackSelector,
};
pub use handle::RtpGatewayHandle;
pub use logging_sink::LoggingSink;
#[allow(dead_code, unused_imports)]
pub use media_thread::MediaThreadCallSink;
