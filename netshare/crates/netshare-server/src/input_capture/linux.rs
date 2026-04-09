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
    seamless: SharedSeamlessState,
) {
    // Enumerate and separate readable devices from permission-denied ones so
    // the user gets an actionable error message instead of a silent failure.
    let all_paths: Vec<_> = evdev::enumerate().map(|(p, _)| p).collect();
    let (readable_paths, denied_paths): (Vec<_>, Vec<_>) = all_paths
        .iter()
        .partition(|p| std::fs::metadata(p).is_ok());

    if !denied_paths.is_empty() {
        warn!(
            "Cannot read {} evdev device(s) — permission denied:\n  {}\n  \
             Fix: sudo usermod -aG input $USER  (then log out and back in)",
            denied_paths.len(),
            denied_paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n  ")
        );
    }

    let devices: Vec<_> = evdev::enumerate()
        .filter(|(p, _)| readable_paths.contains(&p))
        .collect();

    if devices.is_empty() {
        if denied_paths.is_empty() {
            warn!("No evdev keyboard/mouse devices found in /dev/input/");
        }
        // Watcher still starts so seamless state is maintained (edge detection
        // will be a no-op until a device becomes readable).
    }

    // Spawn the x11rb seamless cursor watcher (edge detection + cursor lock).
    {
        let tx_s = tx.clone();
        let sem  = seamless.clone();
        std::thread::spawn(move || seamless_watcher(tx_s, sem));
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
        let tx_clone   = tx.clone();
        let sem_clone  = seamless.clone();

        std::thread::spawn(move || {
            read_device_loop(device, is_mouse, is_keyboard, tx_clone, sem_clone);
        });
        spawned += 1;
    }

    if spawned == 0 {
        warn!("No keyboard/mouse devices found in /dev/input/. Check permissions.");
    } else {
        info!("evdev: spawned {} capture thread(s)", spawned);
    }
}

/// Poll the X11 cursor position every 8 ms.
///
/// * When **not locked**: check if cursor is at a configured edge → fire
///   `EdgeEnter`, update seamless state, warp cursor to lock pixel.
/// * When **locked**: keep warping the cursor back to the lock pixel so the
///   system cursor stays pinned at the edge while input flows to the client.
fn seamless_watcher(tx: mpsc::UnboundedSender<CaptureEvent>, seamless: SharedSeamlessState) {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    use netshare_core::layout::ClientEdge;

    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        warn!("x11rb: cannot connect to X11 display — seamless cursor crossing disabled");
        return;
    };
    let root = conn.setup().roots[screen_num].root;
    info!("Seamless cursor watcher started (x11rb)");

    loop {
        std::thread::sleep(std::time::Duration::from_millis(8));

        // Query current absolute cursor position.
        let Ok(cookie) = conn.query_pointer(root) else { continue };
        let Ok(reply)  = cookie.reply()            else { continue };
        let cx = reply.root_x as i32;
        let cy = reply.root_y as i32;

        let mut state = seamless.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(_slot) = state.locked_to_slot {
            // Already locked — keep warping back to the lock pixel.
            let (lx, ly) = (state.lock_x, state.lock_y);
            drop(state);
            if cx != lx || cy != ly {
                conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, lx as i16, ly as i16).ok();
                conn.flush().ok();
            }
            continue;
        }

        // Not locked — check if cursor is at a configured edge.
        let layout = &state.layout;
        if layout.server_width == 0 || layout.placements.is_empty() {
            continue;
        }
        let (vx_min, vy_min, vx_max, vy_max) = layout.server_bounds();

        let found: Option<(u8, ClientEdge, i32, i32)> =
            layout.placements.iter().find_map(|(&slot, placement)| {
                let on_edge = match placement.edge {
                    ClientEdge::Right  => cx >= vx_max - 1,
                    ClientEdge::Left   => cx <= vx_min,
                    ClientEdge::Below  => cy >= vy_max - 1,
                    ClientEdge::Above  => cy <= vy_min,
                };
                if !on_edge { return None; }
                let (lx, ly) = match placement.edge {
                    ClientEdge::Right  => (vx_max - 1, cy.clamp(vy_min, vy_max - 1)),
                    ClientEdge::Left   => (vx_min,     cy.clamp(vy_min, vy_max - 1)),
                    ClientEdge::Below  => (cx.clamp(vx_min, vx_max - 1), vy_max - 1),
                    ClientEdge::Above  => (cx.clamp(vx_min, vx_max - 1), vy_min),
                };
                Some((slot, placement.edge, lx, ly))
            });

        if let Some((slot, server_edge, lx, ly)) = found {
            let entry = layout.entry_pos(slot, cx, cy);
            state.locked_to_slot = Some(slot);
            state.lock_x = lx;
            state.lock_y = ly;
            drop(state);

            // Warp cursor to lock pixel.
            conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, lx as i16, ly as i16).ok();
            conn.flush().ok();

            let (entry_x, entry_y) = entry.unwrap_or((0, 0));
            let _ = tx.send(CaptureEvent::EdgeEnter { slot, entry_x, entry_y, server_edge });
            info!("EdgeEnter → slot {slot} at ({entry_x},{entry_y}) via {:?}", server_edge);
        }
    }
}

fn read_device_loop(
    mut device: evdev::Device,
    is_mouse: bool,
    is_keyboard: bool,
    tx: mpsc::UnboundedSender<CaptureEvent>,
    seamless: SharedSeamlessState,
) {
    loop {
        let events = match device.fetch_events() {
            Ok(e) => e,
            Err(e) => {
                warn!("evdev read error: {e}");
                break;
            }
        };

        let locked = seamless.lock().unwrap_or_else(|e| e.into_inner()).locked_to_slot;

        for ev in events {
            let pkt: Option<ControlPacket> = match ev.kind() {
                evdev::InputEventKind::RelAxis(axis) if is_mouse => {
                    // When locked, forward mouse deltas to client.
                    // When unlocked, skip — no active client to route to.
                    if locked.is_some() {
                        handle_rel_axis(axis, ev.value())
                    } else {
                        None
                    }
                }
                evdev::InputEventKind::Key(key) if is_keyboard => {
                    handle_keyboard_key(key, ev.value())
                }
                evdev::InputEventKind::Key(key) if is_mouse => {
                    if locked.is_some() {
                        handle_mouse_button(key, ev.value())
                    } else {
                        None
                    }
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
        evdev::Key::KEY_SYSRQ       => 0x2C,
        evdev::Key::KEY_KPENTER     => 0x0D,
        _ => return None,
    })
}
