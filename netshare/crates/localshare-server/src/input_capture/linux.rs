//! Linux evdev exclusive-grab input capture.
//! Stub — will be implemented in a follow-up when building on Linux.

use tokio::sync::mpsc;
use super::CaptureEvent;

pub(super) fn run_evdev(_tx: mpsc::UnboundedSender<CaptureEvent>) {
    todo!("evdev capture — implement for Linux target");
}
