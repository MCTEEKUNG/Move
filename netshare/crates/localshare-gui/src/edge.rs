//! Layout-aware screen-edge switching.
//!
//! Reads the user's monitor arrangement (from the canvas in the GUI),
//! figures out for each connected peer which edge of the host's physical
//! screen they sit on, and when the cursor dwells at that edge for
//! DWELL_MS, sends a switch_to(slot) so input starts flowing to that peer.
//!
//! This is the embedded-GUI equivalent of `localshare-daemon::edge`, but
//! driven by the GUI's canvas layout rather than a hard-coded "right edge
//! only" rule — so dragging a peer monitor to the left of the primary on
//! the canvas makes the *left* screen edge the trigger.

use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side { Left, Right, Top, Bottom }

#[derive(Debug, Clone, Copy)]
pub struct EdgeTrigger {
    pub side: Side,
    pub slot: u8,
}

/// Shared by GUI (writer) and the edge-polling thread (reader).
pub type EdgeLayout = Arc<Mutex<Vec<EdgeTrigger>>>;

const DWELL_MS: u64 = 120;
const EDGE_PX:  i32 = 2;

#[cfg(windows)]
pub fn spawn(edges: EdgeLayout, switch_tx: mpsc::UnboundedSender<u8>) {
    std::thread::spawn(move || run(edges, switch_tx));
}

#[cfg(not(windows))]
pub fn spawn(_edges: EdgeLayout, _switch_tx: mpsc::UnboundedSender<u8>) {
    tracing::info!("Screen-edge switching: not yet implemented on this OS");
}

#[cfg(windows)]
fn run(edges: EdgeLayout, switch_tx: mpsc::UnboundedSender<u8>) {
    use std::time::{Duration, Instant};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetSystemMetrics,
        SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
        SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };
    use windows::Win32::Foundation::POINT;

    let dwell = Duration::from_millis(DWELL_MS);
    let mut dwell_since: Option<(Side, Instant)> = None;

    loop {
        std::thread::sleep(Duration::from_millis(8)); // ~120 Hz poll

        // Pull fresh layout every tick (cheap — small Vec).
        let triggers = { edges.lock().unwrap().clone() };
        if triggers.is_empty() {
            dwell_since = None;
            continue;
        }

        let mut pt = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut pt).is_err() } { continue; }

        let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let vw = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let vh = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };

        // Which edge is the cursor currently at (if any)?
        let cur_side: Option<Side> =
            if pt.x <= vx + EDGE_PX                { Some(Side::Left)   }
            else if pt.x >= vx + vw - 1 - EDGE_PX  { Some(Side::Right)  }
            else if pt.y <= vy + EDGE_PX          { Some(Side::Top)    }
            else if pt.y >= vy + vh - 1 - EDGE_PX  { Some(Side::Bottom) }
            else { None };

        let Some(side) = cur_side else {
            dwell_since = None;
            continue;
        };

        // Only dwell if there's a peer on that side.
        let Some(trig) = triggers.iter().find(|t| t.side == side) else {
            dwell_since = None;
            continue;
        };

        let (tracked_side, since) =
            dwell_since.get_or_insert((side, Instant::now()));

        if *tracked_side != side {
            *tracked_side = side;
            *since = Instant::now();
            continue;
        }

        if since.elapsed() >= dwell {
            tracing::info!("Edge {:?} dwell → switch to slot {}", side, trig.slot);
            let _ = switch_tx.send(trig.slot);
            dwell_since = None;
            // Avoid re-triggering while cursor still parks at edge on
            // the remote's side.
            std::thread::sleep(Duration::from_millis(600));
        }
    }
}

/// Given the canvas layout (primary at index 0, peers after), build the
/// list of edge→slot triggers. Only peers with a slot (inbound-connected)
/// can be switched to, so those are the only ones we emit triggers for.
pub fn layout_from_monitors(monitors: &[crate::app::MonitorInfo]) -> Vec<EdgeTrigger> {
    if monitors.len() < 2 { return Vec::new(); }
    let prim = &monitors[0];
    let pc = prim.pos + prim.size * 0.5;

    let mut out = Vec::new();
    for mon in monitors.iter().skip(1) {
        let Some(slot) = mon.slot else { continue }; // only real TCP slots
        let c = mon.pos + mon.size * 0.5;
        let dx = c.x - pc.x;
        let dy = c.y - pc.y;
        let side = if dx.abs() > dy.abs() {
            if dx < 0.0 { Side::Left } else { Side::Right }
        } else {
            if dy < 0.0 { Side::Top  } else { Side::Bottom }
        };
        // Last-write-wins if the user stacked two peers on the same side.
        out.retain(|t: &EdgeTrigger| t.side != side);
        out.push(EdgeTrigger { side, slot });
    }
    out
}
