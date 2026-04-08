//! Platform-specific input capture.
//!
//! The public interface is `start_capture(tx)` which spawns the platform hook
//! and forwards captured events through `tx`.

use netshare_core::protocol::ControlPacket;
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
pub fn start_capture(tx: mpsc::UnboundedSender<CaptureEvent>) {
    #[cfg(target_os = "windows")]
    windows::run_hook(tx);

    #[cfg(target_os = "linux")]
    linux::run_evdev(tx);

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    compile_error!("netshare-server only supports Windows and Linux");
}
