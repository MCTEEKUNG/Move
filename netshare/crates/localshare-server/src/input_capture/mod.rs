//! Platform-specific input capture.
//!
//! The public interface is `start_capture(tx)` which spawns the platform hook
//! and forwards captured events through `tx`.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use localshare_core::protocol::ControlPacket;
use tokio::sync::mpsc;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
mod linux;

/// Hotkey action decoded from the raw key event.
#[derive(Debug, Clone)]
pub enum HotkeyAction {
    SwitchToSlot(u8),  // Ctrl+Shift+Alt+[1-9]
    Cycle,             // Scroll Lock
}

pub enum CaptureEvent {
    InputPacket(ControlPacket),
    Hotkey(HotkeyAction),
}

/// Start the platform input capture. Blocks the calling thread (must be run
/// on a dedicated OS thread, not a tokio task — required for Windows LL Hooks).
///
/// `suppress` is shared state controlling whether local input is swallowed
/// (true = forward to remote client, suppress locally) or passed through to the
/// local OS (false = local machine is in control). Updated by network.rs on every
/// active-slot / share_input change.
pub fn start_capture(tx: mpsc::UnboundedSender<CaptureEvent>, suppress: Arc<AtomicBool>) {
    #[cfg(target_os = "windows")]
    windows::run_hook(tx, suppress);

    #[cfg(target_os = "linux")]
    { let _ = suppress; linux::run_evdev(tx); }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    compile_error!("localshare-server only supports Windows and Linux");
}
