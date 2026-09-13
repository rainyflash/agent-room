//! Shared Agent Room tool access. Identity, permissions and message persistence stay in Bridge.
mod bridge;
mod inbox;
pub mod reception;
pub use bridge::{BridgeToolClient, BridgeToolFailure, BridgeToolFuture, LocalBridgeToolClient};
pub use inbox::{MAX_EXPLICIT_WAIT_SECONDS, MessageReadMode, MessageWait, wait_for_messages};
