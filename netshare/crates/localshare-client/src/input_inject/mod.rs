//! Platform-specific input injection.

use localshare_core::input::{KeyEvent, MouseClick, MouseMove, MouseScroll};

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
