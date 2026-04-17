//! Screen-edge switching.
//!
//! When the cursor reaches the right edge of any monitor and stays there
//! for DWELL_MS milliseconds, we switch to the next connected peer.
//! Moving away from the edge before DWELL_MS resets the timer.

use tokio::sync::mpsc;
use tracing::debug;

use localshare_server::active_client::ActiveClientState;

const DWELL_MS: u64 = 150; // ms to hold at edge before switching
const EDGE_PX:  i32 = 2;   // pixels from edge that counts as "at edge"

#[cfg(windows)]
pub fn run(state: ActiveClientState, switch_tx: mpsc::UnboundedSender<u8>) {
    use std::time::{Duration, Instant};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    };
    use windows::Win32::Foundation::POINT;

    let dwell = Duration::from_millis(DWELL_MS);
    let mut edge_since: Option<Instant> = None;

    loop {
        std::thread::sleep(Duration::from_millis(8)); // ~120 Hz poll

        let mut pt = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut pt).is_err() } {
            continue;
        }

        let screen_w  = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let screen_x  = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let right_edge = screen_x + screen_w;

        let at_edge = pt.x >= right_edge - EDGE_PX;

        if at_edge {
            let since = edge_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= dwell {
                // Switch to next peer
                let snapshot = state.snapshot();
                if let Some(next) = snapshot.first() {
                    debug!("Edge trigger → switching to slot {}", next.slot);
                    let _ = switch_tx.send(next.slot);
                }
                edge_since = None;
                // Brief pause to avoid re-triggering immediately
                std::thread::sleep(Duration::from_millis(800));
            }
        } else {
            edge_since = None;
        }
    }
}

#[cfg(not(windows))]
pub fn run(_state: ActiveClientState, _switch_tx: mpsc::UnboundedSender<u8>) {
    // Linux screen-edge switching — Phase 2 (requires X11/Wayland integration)
    tracing::info!("Screen-edge switching not yet implemented on Linux");
}
