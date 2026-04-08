//! Linux evdev input capture — reads from physical keyboard/mouse devices and
//! forwards events as CaptureEvents to the network layer.
//!
//! For each /dev/input/event* that is a keyboard or relative-axis mouse:
//!   - Spawn a blocking thread to read events from it.
//!   - Convert evdev events → ControlPacket and send as CaptureEvent::InputPacket.
//!
//! Requirements: the user must be in the `input` group (added by .deb postinst).

use tokio::sync::mpsc;
use tracing::{info, warn};

use netshare_core::input::{
    ButtonAction, KeyEvent, KeyFlags, MouseButton, MouseClick, MouseMove, MouseScroll,
};
use netshare_core::protocol::ControlPacket;
use super::{CaptureEvent, SharedSeamlessState};

pub(super) fn run_evdev(
    tx: mpsc::UnboundedSender<CaptureEvent>,
    _seamless: SharedSeamlessState,
) {
    let devices: Vec<_> = evdev::enumerate().collect();

    if devices.is_empty() {
        warn!("No evdev devices found. Make sure user is in 'input' group: sudo usermod -aG input $USER");
        return;
    }

    let mut spawned = 0usize;
    for (path, device) in devices {
        let supported = device.supported_events();
        let is_mouse    = supported.contains(evdev::EventType::RELATIVE);
        let is_keyboard = supported.contains(evdev::EventType::KEY)
            && device.supported_keys().map_or(false, |k| k.contains(evdev::Key::KEY_A));

        if !is_mouse && !is_keyboard {
            continue;
        }

        let name = device.name().unwrap_or("unknown").to_owned();
        info!("Capturing input device: {} ({:?})", name, path);
        let tx_clone = tx.clone();

        std::thread::spawn(move || {
            read_device_loop(device, is_mouse, is_keyboard, tx_clone);
        });
        spawned += 1;
    }

    if spawned == 0 {
        warn!("No keyboard/mouse devices found in /dev/input/. Check permissions.");
    } else {
        info!("evdev: spawned {} capture thread(s)", spawned);
    }
}

fn read_device_loop(
    mut device: evdev::Device,
    is_mouse: bool,
    is_keyboard: bool,
    tx: mpsc::UnboundedSender<CaptureEvent>,
) {
    loop {
        let events = match device.fetch_events() {
            Ok(e) => e,
            Err(e) => {
                warn!("evdev read error: {e}");
                break;
            }
        };

        for ev in events {
            let pkt: Option<ControlPacket> = match ev.kind() {
                evdev::InputEventKind::RelAxis(axis) if is_mouse => {
                    handle_rel_axis(axis, ev.value())
                }
                evdev::InputEventKind::Key(key) if is_keyboard => {
                    handle_keyboard_key(key, ev.value())
                }
                evdev::InputEventKind::Key(key) if is_mouse => {
                    handle_mouse_button(key, ev.value())
                }
                _ => None,
            };

            if let Some(pkt) = pkt {
                if tx.send(CaptureEvent::InputPacket(pkt)).is_err() {
                    return; // Channel closed — server shutting down.
                }
            }
        }
    }
}

fn handle_rel_axis(axis: evdev::RelativeAxisType, value: i32) -> Option<ControlPacket> {
    match axis {
        evdev::RelativeAxisType::REL_X => {
            Some(ControlPacket::MouseMove(MouseMove { dx: value, dy: 0 }))
        }
        evdev::RelativeAxisType::REL_Y => {
            Some(ControlPacket::MouseMove(MouseMove { dx: 0, dy: value }))
        }
        evdev::RelativeAxisType::REL_WHEEL => {
            Some(ControlPacket::Scroll(MouseScroll { delta_x: 0, delta_y: -value }))
        }
        evdev::RelativeAxisType::REL_HWHEEL => {
            Some(ControlPacket::Scroll(MouseScroll { delta_x: value, delta_y: 0 }))
        }
        _ => None,
    }
}

fn handle_keyboard_key(key: evdev::Key, value: i32) -> Option<ControlPacket> {
    // value: 1=press, 0=release, 2=autorepeat (skip autorepeat)
    let action = match value {
        1 => ButtonAction::Press,
        0 => ButtonAction::Release,
        _ => return None,
    };
    let vk = linux_to_vk(key)?;
    Some(ControlPacket::KeyEvent(KeyEvent {
        vk,
        action,
        scan: key.code(),
        flags: KeyFlags::empty(),
    }))
}

fn handle_mouse_button(key: evdev::Key, value: i32) -> Option<ControlPacket> {
    let action = match value {
        1 => ButtonAction::Press,
        0 => ButtonAction::Release,
        _ => return None,
    };
    let button = match key {
        evdev::Key::BTN_LEFT   => MouseButton::Left,
        evdev::Key::BTN_RIGHT  => MouseButton::Right,
        evdev::Key::BTN_MIDDLE => MouseButton::Middle,
        evdev::Key::BTN_SIDE   => MouseButton::X1,
        evdev::Key::BTN_EXTRA  => MouseButton::X2,
        _ => return None,
    };
    Some(ControlPacket::MouseClick(MouseClick { button, action, x: 0, y: 0 }))
}

/// Map Linux evdev Key → Windows VK code (canonical cross-platform form).
fn linux_to_vk(key: evdev::Key) -> Option<u32> {
    Some(match key {
        evdev::Key::KEY_ESC         => 0x1B,
        evdev::Key::KEY_1           => 0x31,
        evdev::Key::KEY_2           => 0x32,
        evdev::Key::KEY_3           => 0x33,
        evdev::Key::KEY_4           => 0x34,
        evdev::Key::KEY_5           => 0x35,
        evdev::Key::KEY_6           => 0x36,
        evdev::Key::KEY_7           => 0x37,
        evdev::Key::KEY_8           => 0x38,
        evdev::Key::KEY_9           => 0x39,
        evdev::Key::KEY_0           => 0x30,
        evdev::Key::KEY_MINUS       => 0xBD,
        evdev::Key::KEY_EQUAL       => 0xBB,
        evdev::Key::KEY_BACKSPACE   => 0x08,
        evdev::Key::KEY_TAB         => 0x09,
        evdev::Key::KEY_Q           => 0x51,
        evdev::Key::KEY_W           => 0x57,
        evdev::Key::KEY_E           => 0x45,
        evdev::Key::KEY_R           => 0x52,
        evdev::Key::KEY_T           => 0x54,
        evdev::Key::KEY_Y           => 0x59,
        evdev::Key::KEY_U           => 0x55,
        evdev::Key::KEY_I           => 0x49,
        evdev::Key::KEY_O           => 0x4F,
        evdev::Key::KEY_P           => 0x50,
        evdev::Key::KEY_LEFTBRACE   => 0xDB,
        evdev::Key::KEY_RIGHTBRACE  => 0xDD,
        evdev::Key::KEY_ENTER       => 0x0D,
        evdev::Key::KEY_LEFTCTRL    => 0xA2,
        evdev::Key::KEY_A           => 0x41,
        evdev::Key::KEY_S           => 0x53,
        evdev::Key::KEY_D           => 0x44,
        evdev::Key::KEY_F           => 0x46,
        evdev::Key::KEY_G           => 0x47,
        evdev::Key::KEY_H           => 0x48,
        evdev::Key::KEY_J           => 0x4A,
        evdev::Key::KEY_K           => 0x4B,
        evdev::Key::KEY_L           => 0x4C,
        evdev::Key::KEY_SEMICOLON   => 0xBA,
        evdev::Key::KEY_APOSTROPHE  => 0xDE,
        evdev::Key::KEY_GRAVE       => 0xC0,
        evdev::Key::KEY_LEFTSHIFT   => 0xA0,
        evdev::Key::KEY_BACKSLASH   => 0xDC,
        evdev::Key::KEY_Z           => 0x5A,
        evdev::Key::KEY_X           => 0x58,
        evdev::Key::KEY_C           => 0x43,
        evdev::Key::KEY_V           => 0x56,
        evdev::Key::KEY_B           => 0x42,
        evdev::Key::KEY_N           => 0x4E,
        evdev::Key::KEY_M           => 0x4D,
        evdev::Key::KEY_COMMA       => 0xBC,
        evdev::Key::KEY_DOT         => 0xBE,
        evdev::Key::KEY_SLASH       => 0xBF,
        evdev::Key::KEY_RIGHTSHIFT  => 0xA1,
        evdev::Key::KEY_KPASTERISK  => 0x6A,
        evdev::Key::KEY_LEFTALT     => 0xA4,
        evdev::Key::KEY_SPACE       => 0x20,
        evdev::Key::KEY_CAPSLOCK    => 0x14,
        evdev::Key::KEY_F1          => 0x70,
        evdev::Key::KEY_F2          => 0x71,
        evdev::Key::KEY_F3          => 0x72,
        evdev::Key::KEY_F4          => 0x73,
        evdev::Key::KEY_F5          => 0x74,
        evdev::Key::KEY_F6          => 0x75,
        evdev::Key::KEY_F7          => 0x76,
        evdev::Key::KEY_F8          => 0x77,
        evdev::Key::KEY_F9          => 0x78,
        evdev::Key::KEY_F10         => 0x79,
        evdev::Key::KEY_NUMLOCK     => 0x90,
        evdev::Key::KEY_SCROLLLOCK  => 0x91,
        evdev::Key::KEY_KP7         => 0x67,
        evdev::Key::KEY_KP8         => 0x68,
        evdev::Key::KEY_KP9         => 0x69,
        evdev::Key::KEY_KPMINUS     => 0x6D,
        evdev::Key::KEY_KP4         => 0x64,
        evdev::Key::KEY_KP5         => 0x65,
        evdev::Key::KEY_KP6         => 0x66,
        evdev::Key::KEY_KPPLUS      => 0x6B,
        evdev::Key::KEY_KP1         => 0x61,
        evdev::Key::KEY_KP2         => 0x62,
        evdev::Key::KEY_KP3         => 0x63,
        evdev::Key::KEY_KP0         => 0x60,
        evdev::Key::KEY_KPDOT       => 0x6E,
        evdev::Key::KEY_F11         => 0x7A,
        evdev::Key::KEY_F12         => 0x7B,
        evdev::Key::KEY_RIGHTCTRL   => 0xA3,
        evdev::Key::KEY_KPSLASH     => 0x6F,
        evdev::Key::KEY_RIGHTALT    => 0xA5,
        evdev::Key::KEY_HOME        => 0x24,
        evdev::Key::KEY_UP          => 0x26,
        evdev::Key::KEY_PAGEUP      => 0x21,
        evdev::Key::KEY_LEFT        => 0x25,
        evdev::Key::KEY_RIGHT       => 0x27,
        evdev::Key::KEY_END         => 0x23,
        evdev::Key::KEY_DOWN        => 0x28,
        evdev::Key::KEY_PAGEDOWN    => 0x22,
        evdev::Key::KEY_INSERT      => 0x2D,
        evdev::Key::KEY_DELETE      => 0x2E,
        evdev::Key::KEY_LEFTMETA    => 0x5B,
        evdev::Key::KEY_RIGHTMETA   => 0x5C,
        evdev::Key::KEY_PAUSE       => 0x13,
        evdev::Key::KEY_PRINTSCREEN => 0x2C,
        evdev::Key::KEY_KPENTER     => 0x0D,
        _ => return None,
    })
}
