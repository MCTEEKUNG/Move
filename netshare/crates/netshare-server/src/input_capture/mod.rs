//! Platform-specific input capture.
//!
//! The public interface is `start_capture(tx, seamless)` which spawns the
//! platform hook and forwards captured events through `tx`.

use std::sync::{Arc, Mutex};
use netshare_core::layout::LayoutConfig;
use netshare_core::protocol::ControlPacket;
use tokio::sync::mpsc;

#[cfg(target_os = "windows")]
pub(crate) mod windows;
#[cfg(target_os = "linux")]
mod linux;

/// Hotkey action decoded from the raw key event.
#[derive(Debug, Clone)]
pub enum HotkeyAction {
    SwitchToSlot(u8),  // Ctrl+Shift+Alt+[1-9]
    Cycle,             // Scroll Lock
    /// Force-release the cursor back to the server screen immediately.
    /// Triggered by Ctrl+Shift+Alt+0.  Primary escape hatch when a topology
    /// mismatch leaves the cursor permanently locked to a client screen.
    ReleaseToLocal,
}

pub enum CaptureEvent {
    InputPacket(ControlPacket),
    Hotkey(HotkeyAction),
    /// The cursor just crossed an edge into a client screen.
    EdgeEnter { slot: u8, entry_x: i32, entry_y: i32, server_edge: netshare_core::layout::ClientEdge },
}

/// State shared between the network layer and the hook thread.
#[derive(Default)]
pub struct SeamlessState {
    pub layout:         LayoutConfig,
    /// If Some(slot), cursor is locked on server; all moves go to this client.
    pub locked_to_slot: Option<u8>,
    /// Where the cursor is locked on the server screen (edge pixel).
    pub lock_x: i32,
    pub lock_y: i32,
}

pub type SharedSeamlessState = Arc<Mutex<SeamlessState>>;

/// Start the platform input capture. Blocks the calling thread (must be run
/// on a dedicated OS thread, not a tokio task — required for Windows LL Hooks).
pub fn start_capture(tx: mpsc::UnboundedSender<CaptureEvent>, seamless: SharedSeamlessState) {
    #[cfg(target_os = "windows")]
    windows::run_hook(tx, seamless);

    #[cfg(target_os = "linux")]
    linux::run_evdev(tx, seamless);

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    compile_error!("netshare-server only supports Windows and Linux");
}
