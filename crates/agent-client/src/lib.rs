//! Shared Agent Room tool access. Identity, permissions and message persistence stay in Bridge.
mod bridge;
mod desktop_launch;
mod inbox;
pub mod reception;
pub use bridge::{BridgeToolClient, BridgeToolFailure, BridgeToolFuture, LocalBridgeToolClient};
pub use desktop_launch::{
    DESKTOP_BACKGROUND_ARGUMENT, DesktopLauncher, DesktopLaunchingClient, InstalledDesktop,
    launching_desktop_when_absent,
};
pub use inbox::{MAX_EXPLICIT_WAIT_SECONDS, MessageReadMode, MessageWait, wait_for_messages};
