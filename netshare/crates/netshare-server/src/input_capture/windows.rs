//! Windows Low-Level Hook input capture.
//!
//! # Threading model
//! Windows LL Hooks MUST be installed and serviced on a thread that runs a
//! Windows message loop (GetMessage / DispatchMessage). If the message loop
//! stalls for > 300 ms, Windows bypasses the hook automatically.
//!
//! This module is called from a dedicated OS thread (not a tokio task).
//! Captured events are sent through an mpsc channel to the tokio runtime.

use std::cell::Cell;
use tokio::sync::mpsc;
use tracing::warn;
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, RECT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::VK_SCROLL,
        WindowsAndMessaging::{
            CallNextHookEx, ClipCursor, DispatchMessageW, GetMessageW, SetCursorPos,
            SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
            HHOOK, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, MSG,
            WH_KEYBOARD_LL, WH_MOUSE_LL,
            WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
            WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
            WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
            WM_XBUTTONDOWN, WM_XBUTTONUP,
        },
    },
    System::Threading::{
        GetCurrentProcess, GetCurrentThread, SetPriorityClass, SetThreadPriority,
        HIGH_PRIORITY_CLASS, THREAD_PRIORITY_HIGHEST,
    },
};
use netshare_core::{
    input::{capture_timestamp_micros, ButtonAction, KeyEvent, KeyFlags, MouseButton, MouseClick, MouseMove, MouseScroll},
    protocol::ControlPacket,
};
use super::{CaptureEvent, HotkeyAction, SharedSeamlessState};

// ── Injected-input flag ───────────────────────────────────────────────────────
const LLMHF_INJECTED: u32 = 0x01;

// ── Modifier state ────────────────────────────────────────────────────────
// Thread-local because the hook callbacks are invoked on this same thread.
thread_local! {
    static TX:          Cell<Option<mpsc::UnboundedSender<CaptureEvent>>> = const { Cell::new(None) };
    static SEAMLESS:    Cell<Option<SharedSeamlessState>>                  = const { Cell::new(None) };
    static CTRL_DOWN:   Cell<bool>  = const { Cell::new(false) };
    static SHIFT_DOWN:  Cell<bool>  = const { Cell::new(false) };
    static ALT_DOWN:    Cell<bool>  = const { Cell::new(false) };
    static KBD_HOOK:    Cell<HHOOK> = const { Cell::new(HHOOK(std::ptr::null_mut())) };
    static MOUSE_HOOK:  Cell<HHOOK> = const { Cell::new(HHOOK(std::ptr::null_mut())) };
    static LAST_X:      Cell<i32>   = const { Cell::new(0) };
    static LAST_Y:      Cell<i32>   = const { Cell::new(0) };
}

// Virtual key constants not re-exported by the `windows` crate at this path.
const VK_LCONTROL: u16 = 0xA2;
const VK_RCONTROL: u16 = 0xA3;
const VK_LSHIFT:   u16 = 0xA0;
const VK_RSHIFT:   u16 = 0xA1;
const VK_LMENU:    u16 = 0xA4; // left Alt
const VK_RMENU:    u16 = 0xA5; // right Alt

/// Entry point — installs hooks and runs the message loop until WM_QUIT.
pub(super) fn run_hook(tx: mpsc::UnboundedSender<CaptureEvent>, seamless: SharedSeamlessState) {
    TX.with(|cell| cell.set(Some(tx)));
    SEAMLESS.with(|cell| cell.set(Some(seamless)));

    // ── Performance Optimization ───────────────────────────────────────────
    // Escalating the priority of this process and thread ensures that the 
    // Low-Level Hook is serviced as rapidly as possible, even when the CPU is busy.
    unsafe {
        let process = GetCurrentProcess();
        let _ = SetPriorityClass(process, HIGH_PRIORITY_CLASS);
        
        let thread = GetCurrentThread();
        let _ = SetThreadPriority(thread, THREAD_PRIORITY_HIGHEST);
    }

    // Safety: hooks are installed on this thread and unhooked on exit.
    unsafe {
        let kbd_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kbd_proc), None, 0)
            .expect("failed to install WH_KEYBOARD_LL hook");
        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0)
            .expect("failed to install WH_MOUSE_LL hook");

        KBD_HOOK.with(|c| c.set(kbd_hook));
        MOUSE_HOOK.with(|c| c.set(mouse_hook));

        // Message loop — required to keep LL Hooks alive and responsive.
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        UnhookWindowsHookEx(mouse_hook).ok();
        UnhookWindowsHookEx(kbd_hook).ok();
    }
}

// ── Keyboard hook ──────────────────────────────────────────────────────────

unsafe extern "system" fn kbd_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    let vk = info.vkCode as u16;
    let is_press = matches!(
        wparam.0 as u32,
        WM_KEYDOWN | WM_SYSKEYDOWN
    );

    // Track modifier state.
    let is_ctrl  = matches!(vk, VK_LCONTROL | VK_RCONTROL);
    let is_shift = matches!(vk, VK_LSHIFT | VK_RSHIFT);
    let is_alt   = matches!(vk, VK_LMENU | VK_RMENU);

    if is_ctrl  { CTRL_DOWN.with(|c| c.set(is_press)); }
    if is_shift { SHIFT_DOWN.with(|c| c.set(is_press)); }
    if is_alt   { ALT_DOWN.with(|c| c.set(is_press)); }

    let ctrl  = CTRL_DOWN.with(|c| c.get());
    let shift = SHIFT_DOWN.with(|c| c.get());
    let alt   = ALT_DOWN.with(|c| c.get());

    // ── Hotkey: Ctrl+Shift+Alt+[1-9] ──────────────────────────────────────
    // VK '1'..'9' = 0x31..0x39
    if is_press && ctrl && shift && alt && (0x31..=0x39).contains(&vk) {
        let slot = (vk - 0x30) as u8; // '1'=1 .. '9'=9
        send(CaptureEvent::Hotkey(HotkeyAction::SwitchToSlot(slot)));
        return LRESULT(1); // suppress — do not pass to OS
    }

    // ── Hotkey: Ctrl+Shift+Alt+0 → force-release cursor to server ─────────
    // Emergency escape hatch: if a topology mismatch leaves the cursor
    // permanently locked on a client edge, this brings it back immediately.
    if is_press && ctrl && shift && alt && vk == 0x30 {
        send(CaptureEvent::Hotkey(HotkeyAction::ReleaseToLocal));
        return LRESULT(1);
    }

    // ── Hotkey: Scroll Lock → cycle ────────────────────────────────────────
    if is_press && vk == VK_SCROLL.0 {
        send(CaptureEvent::Hotkey(HotkeyAction::Cycle));
        return LRESULT(1);
    }

    // ── If locked to a slot, forward key events to that client ────────────
    let locked = with_seamless(|s| s.locked_to_slot);
    if locked.is_some() {
        // Build and forward the key event, then suppress.
        let mut flags = KeyFlags::empty();
        if info.flags.0 & 0x01 != 0 {
            flags |= KeyFlags::EXTENDED;
        }
        let evt = KeyEvent {
            vk: info.vkCode,
            action: if is_press { ButtonAction::Press } else { ButtonAction::Release },
            scan: info.scanCode as u16,
            flags,
        };
        send(CaptureEvent::InputPacket(ControlPacket::KeyEvent(evt)));
        return LRESULT(1);
    }

    // ── Regular key event ─────────────────────────────────────────────────
    let mut flags = KeyFlags::empty();
    if info.flags.0 & 0x01 != 0 {
        flags |= KeyFlags::EXTENDED;
    }
    let evt = KeyEvent {
        vk: info.vkCode,
        action: if is_press { ButtonAction::Press } else { ButtonAction::Release },
        scan: info.scanCode as u16,
        flags,
    };
    send(CaptureEvent::InputPacket(ControlPacket::KeyEvent(evt)));

    // Pass through — server machine keeps receiving its own keyboard input.
    CallNextHookEx(None, code, wparam, lparam)
}

// ── Mouse hook ─────────────────────────────────────────────────────────────

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
    let msg  = wparam.0 as u32;

    // Skip injected mouse events (e.g. from our own SetCursorPos call).
    if msg == WM_MOUSEMOVE && (info.flags & LLMHF_INJECTED) != 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    match msg {
        WM_MOUSEMOVE => {
            let last_x = LAST_X.with(|c| c.get());
            let last_y = LAST_Y.with(|c| c.get());
            let dx = info.pt.x - last_x;
            let dy = info.pt.y - last_y;
            LAST_X.with(|c| c.set(info.pt.x));
            LAST_Y.with(|c| c.set(info.pt.y));

            // Check if locked to a client.
            let locked_info = with_seamless(|s| {
                s.locked_to_slot.map(|slot| (slot, s.lock_x, s.lock_y))
            });

            if let Some((slot, lock_x, lock_y)) = locked_info {
                // Locked to a client — forward delta, suppress, clamp back.
                let _ = slot;
                send(CaptureEvent::InputPacket(ControlPacket::MouseMove(MouseMove {
                    dx,
                    dy,
                    captured_at_micros: capture_timestamp_micros(),
                })));
                // Warp cursor back to the lock pixel.
                let _ = SetCursorPos(lock_x, lock_y);
                // CRITICAL: reset LAST_X/Y to the warped position.
                // Without this, the next real event calculates dx from the
                // pre-warp position, producing a large spurious negative delta
                // that appears as a visible "jitter" on the client screen.
                LAST_X.with(|c| c.set(lock_x));
                LAST_Y.with(|c| c.set(lock_y));
                return LRESULT(1);
            }

            // ── Edge detection ─────────────────────────────────────────────
            // Check-and-lock is done atomically inside a single write-lock
            // acquisition to prevent the TOCTOU race where two events could
            // both see locked_to_slot == None and both trigger an EdgeEnter.
            use netshare_core::layout::ClientEdge;
            let edge_result = with_seamless_mut(|s| {
                // Already locked — nothing to do.
                if s.locked_to_slot.is_some() { return None; }
                if s.layout.server_width == 0 || s.layout.server_height == 0 {
                    return None;
                }
                let (vx_min, vy_min, vx_max, vy_max) = s.layout.server_bounds();
                let x = info.pt.x;
                let y = info.pt.y;

                let found_slot = s.layout.placements.iter().find_map(|(&slot, placement)| {
                    let on_edge = match placement.edge {
                        ClientEdge::Right  => x >= vx_max - 1,
                        ClientEdge::Left   => x <= vx_min,
                        ClientEdge::Below  => y >= vy_max - 1,
                        ClientEdge::Above  => y <= vy_min,
                    };
                    if on_edge { Some(slot) } else { None }
                });

                if let Some(slot) = found_slot {
                    let entry = s.layout.entry_pos(slot, x, y);
                    let server_edge = s.layout.placements[&slot].edge;
                    let (lx, ly) = match server_edge {
                        ClientEdge::Right  => (vx_max - 1, y.clamp(vy_min, vy_max - 1)),
                        ClientEdge::Left   => (vx_min,     y.clamp(vy_min, vy_max - 1)),
                        ClientEdge::Below  => (x.clamp(vx_min, vx_max - 1), vy_max - 1),
                        ClientEdge::Above  => (x.clamp(vx_min, vx_max - 1), vy_min),
                    };
                    // Atomically set the lock — no other thread can race here.
                    s.locked_to_slot = Some(slot);
                    s.lock_x = lx;
                    s.lock_y = ly;
                    return Some((slot, entry, lx, ly, server_edge));
                }
                None
            });

            if let Some((slot, entry, lock_x, lock_y, server_edge)) = edge_result {
                let (entry_x, entry_y) = entry.unwrap_or((0, 0));
                // Lock cursor to lock position.
                let _ = SetCursorPos(lock_x, lock_y);
                let clip = RECT {
                    left:   lock_x,
                    top:    lock_y,
                    right:  lock_x + 1,
                    bottom: lock_y + 1,
                };
                let _ = ClipCursor(Some(&clip));
                LAST_X.with(|c| c.set(lock_x));
                LAST_Y.with(|c| c.set(lock_y));
                send(CaptureEvent::EdgeEnter {
                    slot,
                    entry_x,
                    entry_y,
                    server_edge,
                });
                return LRESULT(1);
            }

            // Normal: forward as relative delta.
            send(CaptureEvent::InputPacket(ControlPacket::MouseMove(MouseMove {
                dx,
                dy,
                captured_at_micros: capture_timestamp_micros(),
            })));
            CallNextHookEx(None, code, wparam, lparam)
        }

        WM_LBUTTONDOWN | WM_LBUTTONUP
        | WM_RBUTTONDOWN | WM_RBUTTONUP
        | WM_MBUTTONDOWN | WM_MBUTTONUP
        | WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let locked = with_seamless(|s| s.locked_to_slot);
            let pkt = match msg {
                WM_LBUTTONDOWN => Some(mouse_click(MouseButton::Left,   ButtonAction::Press,   info)),
                WM_LBUTTONUP   => Some(mouse_click(MouseButton::Left,   ButtonAction::Release, info)),
                WM_RBUTTONDOWN => Some(mouse_click(MouseButton::Right,  ButtonAction::Press,   info)),
                WM_RBUTTONUP   => Some(mouse_click(MouseButton::Right,  ButtonAction::Release, info)),
                WM_MBUTTONDOWN => Some(mouse_click(MouseButton::Middle, ButtonAction::Press,   info)),
                WM_MBUTTONUP   => Some(mouse_click(MouseButton::Middle, ButtonAction::Release, info)),
                WM_XBUTTONDOWN | WM_XBUTTONUP => {
                    let btn = if (info.mouseData >> 16) == 1 { MouseButton::X1 } else { MouseButton::X2 };
                    let act = if msg == WM_XBUTTONDOWN { ButtonAction::Press } else { ButtonAction::Release };
                    Some(mouse_click(btn, act, info))
                }
                _ => None,
            };
            if let Some(pkt) = pkt {
                send(CaptureEvent::InputPacket(pkt));
            }
            if locked.is_some() {
                return LRESULT(1); // suppress when locked to a client
            }
            CallNextHookEx(None, code, wparam, lparam)
        }

        WM_MOUSEWHEEL => {
            let delta = (info.mouseData >> 16) as i16; // signed wheel delta
            send(CaptureEvent::InputPacket(ControlPacket::Scroll(MouseScroll {
                delta_x: 0,
                delta_y: delta as i32,
            })));
            let locked = with_seamless(|s| s.locked_to_slot);
            if locked.is_some() {
                return LRESULT(1);
            }
            CallNextHookEx(None, code, wparam, lparam)
        }

        _ => CallNextHookEx(None, code, wparam, lparam),
    }
}

fn mouse_click(button: MouseButton, action: ButtonAction, _info: &MSLLHOOKSTRUCT) -> ControlPacket {
    // x/y are not transmitted: the client injects at its current cursor
    // position (dx=0, dy=0 without MOUSEEVENTF_ABSOLUTE).
    ControlPacket::MouseClick(MouseClick { button, action, x: 0, y: 0 })
}

fn send(evt: CaptureEvent) {
    TX.with(|cell| {
        // Safety: take + put back (Cell doesn't let us borrow).
        let tx = cell.take();
        if let Some(ref t) = tx {
            if t.send(evt).is_err() {
                warn!("capture channel closed");
            }
        }
        cell.set(tx);
    });
}

/// Read-only access to seamless state from a hook callback.
///
/// Uses `try_lock` so the hook thread **never blocks** waiting for the network
/// thread to release the mutex.  A missed check on one mouse event is harmless
/// at 1 kHz poll rate.
fn with_seamless<F, R>(f: F) -> R
where
    F: FnOnce(&super::SeamlessState) -> R,
    R: Default,
{
    SEAMLESS.with(|cell| {
        let arc = cell.take();
        let result = if let Some(ref a) = arc {
            // try_lock: non-blocking — skip if network thread holds the lock.
            if let Ok(guard) = a.try_lock() {
                f(&guard)
            } else {
                R::default()
            }
        } else {
            R::default()
        };
        cell.set(arc);
        result
    })
}

/// Mutable access to seamless state from a hook callback.
/// Returns the value produced by `f` so callers can do atomic check-and-set.
fn with_seamless_mut<F, R>(f: F) -> R
where
    F: FnOnce(&mut super::SeamlessState) -> R,
    R: Default,
{
    SEAMLESS.with(|cell| {
        let arc = cell.take();
        let result = if let Some(ref a) = arc {
            if let Ok(mut guard) = a.try_lock() {
                f(&mut guard)
            } else {
                R::default()
            }
        } else {
            R::default()
        };
        cell.set(arc);
        result
    })
}

/// Release the cursor clip and seamless lock.  Called from the network layer
/// when the client sends `CursorReturn` or the ReleaseToLocal hotkey fires.
pub(crate) fn release_cursor() {
    // Read screen center before clearing lock so we can warp there.
    let (cx, cy) = with_seamless_mut(|s| {
        s.locked_to_slot = None;
        // Use the layout's virtual-desktop center as the warp target.
        // Falls back to 0,0 (Default) if layout is not yet configured.
        let w = s.layout.server_width;
        let h = s.layout.server_height;
        (w / 2, h / 2)
    });
    unsafe {
        let _ = ClipCursor(None);
        // Warp cursor to screen center so the next mouse move does not
        // immediately re-trigger an EdgeEnter from the same edge pixel.
        if cx > 0 || cy > 0 {
            let _ = SetCursorPos(cx, cy);
            LAST_X.with(|c| c.set(cx));
            LAST_Y.with(|c| c.set(cy));
        }
    }
}
