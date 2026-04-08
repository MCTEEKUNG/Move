//! Linux evdev exclusive-grab input capture.
//! Stub — will be implemented in a follow-up when building on Linux.

use tokio::sync::mpsc;
use super::{CaptureEvent, SharedSeamlessState};

pub(super) fn run_evdev(_tx: mpsc::UnboundedSender<CaptureEvent>, _seamless: SharedSeamlessState) {
    todo!("evdev capture — implement for Linux target");
}
