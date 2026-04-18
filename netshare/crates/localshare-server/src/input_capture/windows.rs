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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::warn;
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::VK_SCROLL,
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW,
            TranslateMessage, UnhookWindowsHookEx,
            HHOOK, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, MSG,
            WH_KEYBOARD_LL, WH_MOUSE_LL,
            WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
            WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
            WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
            WM_XBUTTONDOWN, WM_XBUTTONUP,
        },
    },
};
use localshare_core::{
    input::{ButtonAction, KeyEvent, KeyFlags, MouseButton, MouseClick, MouseMove, MouseScroll},
    protocol::ControlPacket,
};
use super::{CaptureEvent, HotkeyAction};

// ── Modifier state ────────────────────────────────────────────────────────
// Thread-local because the hook callbacks are invoked on this same thread.
thread_local! {
    static TX: Cell<Option<mpsc::UnboundedSender<CaptureEvent>>> = const { Cell::new(None) };
    static SUPPRESS:   Cell<Option<Arc<AtomicBool>>> = const { Cell::new(None) };
    static CTRL_DOWN:  Cell<bool> = const { Cell::new(false) };
    static SHIFT_DOWN: Cell<bool> = const { Cell::new(false) };
    static ALT_DOWN:   Cell<bool> = const { Cell::new(false) };
    static KBD_HOOK:   Cell<HHOOK> = const { Cell::new(HHOOK(std::ptr::null_mut())) };
    static MOUSE_HOOK: Cell<HHOOK> = const { Cell::new(HHOOK(std::ptr::null_mut())) };
}

fn should_suppress() -> bool {
    SUPPRESS.with(|cell| {
        let s = cell.take();
        let v = s.as_ref().map(|a| a.load(Ordering::Relaxed)).unwrap_or(false);
        cell.set(s);
        v
    })
}

// Virtual key constants not re-exported by the `windows` crate at this path.
const VK_LCONTROL: u16 = 0xA2;
const VK_RCONTROL: u16 = 0xA3;
const VK_LSHIFT:   u16 = 0xA0;
const VK_RSHIFT:   u16 = 0xA1;
const VK_LMENU:    u16 = 0xA4; // left Alt
const VK_RMENU:    u16 = 0xA5; // right Alt

/// Entry point — installs hooks and runs the message loop until WM_QUIT.
pub(super) fn run_hook(tx: mpsc::UnboundedSender<CaptureEvent>, suppress: Arc<AtomicBool>) {
    TX.with(|cell| cell.set(Some(tx)));
    SUPPRESS.with(|cell| cell.set(Some(suppress)));

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

    // ── Hotkey: Ctrl+Shift+Alt+[0-9] ──────────────────────────────────────
    // VK '0'..'9' = 0x30..0x39. Slot 0 = host (return control to this PC).
    if is_press && ctrl && shift && alt && (0x30..=0x39).contains(&vk) {
        let slot = (vk - 0x30) as u8; // '0'=0 (host) .. '9'=9
        send(CaptureEvent::Hotkey(HotkeyAction::SwitchToSlot(slot)));
        return LRESULT(1); // suppress — do not pass to OS
    }

    // ── Hotkey: Scroll Lock → cycle ────────────────────────────────────────
    if is_press && vk == VK_SCROLL.0 {
        send(CaptureEvent::Hotkey(HotkeyAction::Cycle));
        return LRESULT(1);
    }

    // ── Hotkey: Right-Ctrl while forwarding → snap back to host ──────────
    // Quick escape when the mouse feels "locked" on a remote peer. Only
    // consume the key when we're actually suppressing (active_slot != 0);
    // on the host it passes through so apps keep working normally.
    if is_press && vk == VK_RCONTROL && !shift && !alt && should_suppress() {
        send(CaptureEvent::Hotkey(HotkeyAction::SwitchToSlot(0)));
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

    if should_suppress() {
        send(CaptureEvent::InputPacket(ControlPacket::KeyEvent(evt)));
        LRESULT(1) // forward to remote, hide from local OS
    } else {
        // Passthrough: let the local OS process this key normally.
        CallNextHookEx(None, code, wparam, lparam)
    }
}

// ── Mouse hook ─────────────────────────────────────────────────────────────

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
    let msg  = wparam.0 as u32;

    let pkt: Option<ControlPacket> = match msg {
        WM_MOUSEMOVE => Some(ControlPacket::MouseMove(MouseMove {
            dx: info.pt.x, // raw screen position; server network layer converts to delta
            dy: info.pt.y,
        })),

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

        WM_MOUSEWHEEL => {
            let delta = (info.mouseData >> 16) as i16; // signed wheel delta
            Some(ControlPacket::Scroll(MouseScroll {
                delta_x: 0,
                delta_y: delta as i32,
            }))
        }

        _ => None,
    };

    if let Some(pkt) = pkt {
        if should_suppress() {
            send(CaptureEvent::InputPacket(pkt));
            LRESULT(1) // forward to remote, hide from local OS
        } else {
            // Passthrough: local machine keeps the cursor.
            CallNextHookEx(None, code, wparam, lparam)
        }
    } else {
        CallNextHookEx(None, code, wparam, lparam)
    }
}

fn mouse_click(button: MouseButton, action: ButtonAction, info: &MSLLHOOKSTRUCT) -> ControlPacket {
    ControlPacket::MouseClick(MouseClick {
        button,
        action,
        x: info.pt.x,
        y: info.pt.y,
    })
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
