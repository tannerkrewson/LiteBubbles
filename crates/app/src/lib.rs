//! Reusable GTK views and view models for the LiteBubbles application.
//!
//! The binary shell in main.rs remains intentionally separate from this
//! library boundary. Timeline and composer code can therefore be exercised
//! with deterministic core-domain values without coupling tests to a window.

pub mod composer;
pub mod setup;
pub mod timeline;
pub mod transport;

pub use composer::{
    ComposerModel, ComposerView, OutgoingMessageDraft, SendAffordance, build_composer,
};
pub use timeline::{
    DaySeparator, GroupPosition, LoadState, RenderedPart, ScrollState, TimelineEventEffect,
    TimelineMessage, TimelineModel, TimelineRow, TimelineView, TimestampLabels, build_timeline,
};
pub use transport::{
    AppTransport, DbusTransport, MOCK_TRANSPORT_VALUE, NegotiatedProtocol, ShellTransportMode,
    ShellTransportStatus, TRANSPORT_ENVIRONMENT_VARIABLE, TransportError,
};
