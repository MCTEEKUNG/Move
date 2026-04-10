//! Platform-specific input injection.

use netshare_core::input::{KeyEvent, MouseClick, MouseMove, MouseScroll};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
mod linux;

pub fn inject_mouse_move(ev: MouseMove) {
    #[cfg(target_os = "windows")]
    windows::inject_mouse_move(ev);
    #[cfg(target_os = "linux")]
    linux::inject_mouse_move(ev);
}

pub fn inject_mouse_click(ev: MouseClick) {
    #[cfg(target_os = "windows")]
    windows::inject_mouse_click(ev);
    #[cfg(target_os = "linux")]
    linux::inject_mouse_click(ev);
}

pub fn inject_scroll(ev: MouseScroll) {
    #[cfg(target_os = "windows")]
    windows::inject_scroll(ev);
    #[cfg(target_os = "linux")]
    linux::inject_scroll(ev);
}

pub fn inject_key(ev: KeyEvent) {
    #[cfg(target_os = "windows")]
    windows::inject_key(ev);
    #[cfg(target_os = "linux")]
    linux::inject_key(ev);
}

/// Synthesise key-up for all modifier keys (Win, Ctrl, Shift, Alt — both sides).
///
/// Must be called on `CursorEnter`: the server's low-level hook begins
/// forwarding keys only after the cursor has crossed the edge, so any
/// modifiers already held at the moment of crossing were silently "lost".
/// Releasing them here prevents stuck-key / wrong-combo issues such as
/// regular keystrokes unexpectedly triggering Windows-key shortcuts.
pub fn release_all_modifiers() {
    #[cfg(target_os = "windows")]
    windows::release_all_modifiers();
    #[cfg(target_os = "linux")]
    linux::release_all_modifiers();
}
