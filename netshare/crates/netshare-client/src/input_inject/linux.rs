//! Linux uinput virtual input injection via the `evdev` crate.
//!
//! On first call a single VirtualDevice is created (uinput) that supports
//! relative mouse axes, mouse buttons, and a full keyboard layout.
//! All inject_* functions are synchronous and thread-safe (OnceLock + Mutex).
//!
//! Requirements: the running user must be in the `input` group, OR the binary
//! must have `CAP_SYS_ADMIN`, OR /dev/uinput must be world-writable.
//! The .deb postinst script adds the user to `input` automatically.

use std::sync::{Mutex, OnceLock};

use evdev::{
    uinput::{VirtualDevice, VirtualDeviceBuilder},
    AttributeSet, EventType, InputEvent, Key, RelativeAxisType,
};
use netshare_core::input::{ButtonAction, KeyEvent, MouseButton, MouseClick, MouseMove, MouseScroll};

// ── Virtual device singleton ──────────────────────────────────────────────────

static DEVICE: OnceLock<Mutex<VirtualDevice>> = OnceLock::new();

fn device() -> &'static Mutex<VirtualDevice> {
    DEVICE.get_or_init(|| {
        Mutex::new(build_device().expect("Failed to create uinput virtual device"))
    })
}

fn build_device() -> anyhow::Result<VirtualDevice> {
    // Relative axes: mouse X/Y movement and scroll wheels.
    let mut rel = AttributeSet::<RelativeAxisType>::new();
    rel.insert(RelativeAxisType::REL_X);
    rel.insert(RelativeAxisType::REL_Y);
    rel.insert(RelativeAxisType::REL_WHEEL);
    rel.insert(RelativeAxisType::REL_HWHEEL);

    // Keys: mouse buttons + full keyboard.
    let mut keys = AttributeSet::<Key>::new();
    // Mouse buttons
    keys.insert(Key::BTN_LEFT);
    keys.insert(Key::BTN_RIGHT);
    keys.insert(Key::BTN_MIDDLE);
    keys.insert(Key::BTN_SIDE);   // X1
    keys.insert(Key::BTN_EXTRA);  // X2
    // Letter keys A-Z
    for k in [
        Key::KEY_A, Key::KEY_B, Key::KEY_C, Key::KEY_D, Key::KEY_E,
        Key::KEY_F, Key::KEY_G, Key::KEY_H, Key::KEY_I, Key::KEY_J,
        Key::KEY_K, Key::KEY_L, Key::KEY_M, Key::KEY_N, Key::KEY_O,
        Key::KEY_P, Key::KEY_Q, Key::KEY_R, Key::KEY_S, Key::KEY_T,
        Key::KEY_U, Key::KEY_V, Key::KEY_W, Key::KEY_X, Key::KEY_Y,
        Key::KEY_Z,
    ] { keys.insert(k); }
    // Number row 0-9
    for k in [
        Key::KEY_1, Key::KEY_2, Key::KEY_3, Key::KEY_4, Key::KEY_5,
        Key::KEY_6, Key::KEY_7, Key::KEY_8, Key::KEY_9, Key::KEY_0,
    ] { keys.insert(k); }
    // Function keys F1-F12
    for k in [
        Key::KEY_F1,  Key::KEY_F2,  Key::KEY_F3,  Key::KEY_F4,
        Key::KEY_F5,  Key::KEY_F6,  Key::KEY_F7,  Key::KEY_F8,
        Key::KEY_F9,  Key::KEY_F10, Key::KEY_F11, Key::KEY_F12,
    ] { keys.insert(k); }
    // Navigation and editing
    for k in [
        Key::KEY_ESC, Key::KEY_TAB, Key::KEY_BACKSPACE, Key::KEY_ENTER,
        Key::KEY_LEFTSHIFT, Key::KEY_RIGHTSHIFT,
        Key::KEY_LEFTCTRL,  Key::KEY_RIGHTCTRL,
        Key::KEY_LEFTALT,   Key::KEY_RIGHTALT,
        Key::KEY_LEFTMETA,  Key::KEY_RIGHTMETA,
        Key::KEY_SPACE, Key::KEY_CAPSLOCK,
        Key::KEY_UP, Key::KEY_DOWN, Key::KEY_LEFT, Key::KEY_RIGHT,
        Key::KEY_HOME, Key::KEY_END, Key::KEY_PAGEUP, Key::KEY_PAGEDOWN,
        Key::KEY_INSERT, Key::KEY_DELETE,
        Key::KEY_MINUS, Key::KEY_EQUAL, Key::KEY_BACKSLASH,
        Key::KEY_LEFTBRACE, Key::KEY_RIGHTBRACE,
        Key::KEY_SEMICOLON, Key::KEY_APOSTROPHE, Key::KEY_GRAVE,
        Key::KEY_COMMA, Key::KEY_DOT, Key::KEY_SLASH,
        Key::KEY_SYSRQ, Key::KEY_SCROLLLOCK, Key::KEY_PAUSE,
        Key::KEY_NUMLOCK,
        Key::KEY_KP0,  Key::KEY_KP1,  Key::KEY_KP2,  Key::KEY_KP3,
        Key::KEY_KP4,  Key::KEY_KP5,  Key::KEY_KP6,  Key::KEY_KP7,
        Key::KEY_KP8,  Key::KEY_KP9,
        Key::KEY_KPPLUS, Key::KEY_KPMINUS, Key::KEY_KPASTERISK,
        Key::KEY_KPSLASH, Key::KEY_KPDOT, Key::KEY_KPENTER,
    ] { keys.insert(k); }

    let dev = VirtualDeviceBuilder::new()?
        .name("NetShare Virtual Input")
        .with_relative_axes(&rel)?
        .with_keys(&keys)?
        .build()?;

    Ok(dev)
}

fn syn() -> InputEvent {
    InputEvent::new(EventType::SYNCHRONIZATION, 0, 0)
}

// ── Public injection functions ────────────────────────────────────────────────

pub fn inject_mouse_move(ev: MouseMove) {
    let mut dev = match device().lock() { Ok(d) => d, Err(_) => return };
    let events = [
        InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_X.0, ev.dx),
        InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_Y.0, ev.dy),
        syn(),
    ];
    dev.emit(&events).ok();
}

pub fn inject_mouse_click(ev: MouseClick) {
    let btn = match ev.button {
        MouseButton::Left   => Key::BTN_LEFT,
        MouseButton::Right  => Key::BTN_RIGHT,
        MouseButton::Middle => Key::BTN_MIDDLE,
        MouseButton::X1     => Key::BTN_SIDE,
        MouseButton::X2     => Key::BTN_EXTRA,
    };
    let value = match ev.action {
        ButtonAction::Press   => 1,
        ButtonAction::Release => 0,
    };
    let mut dev = match device().lock() { Ok(d) => d, Err(_) => return };
    let events = [
        InputEvent::new(EventType::KEY, btn.code(), value),
        syn(),
    ];
    dev.emit(&events).ok();
}

pub fn inject_scroll(ev: MouseScroll) {
    let mut dev = match device().lock() { Ok(d) => d, Err(_) => return };
    let mut events = Vec::with_capacity(3);
    if ev.delta_y != 0 {
        events.push(InputEvent::new(
            EventType::RELATIVE, RelativeAxisType::REL_WHEEL.0, -ev.delta_y,
        ));
    }
    if ev.delta_x != 0 {
        events.push(InputEvent::new(
            EventType::RELATIVE, RelativeAxisType::REL_HWHEEL.0, ev.delta_x,
        ));
    }
    events.push(syn());
    dev.emit(&events).ok();
}

/// Synthesise key-up events for every modifier key.
/// See Windows implementation for rationale.
pub fn release_all_modifiers() {
    let mod_keys = [
        Key::KEY_LEFTSHIFT,  Key::KEY_RIGHTSHIFT,
        Key::KEY_LEFTCTRL,   Key::KEY_RIGHTCTRL,
        Key::KEY_LEFTALT,    Key::KEY_RIGHTALT,
        Key::KEY_LEFTMETA,   Key::KEY_RIGHTMETA,
    ];
    let mut dev = match device().lock() {
        Ok(d) => d,
        Err(e) => e.into_inner(),
    };
    let mut events: Vec<InputEvent> = mod_keys.iter()
        .map(|k| InputEvent::new(EventType::KEY, k.code(), 0)) // 0 = key-up
        .collect();
    events.push(syn());
    dev.emit(&events).ok();
}

pub fn inject_key(ev: KeyEvent) {
    let Some(linux_key) = vk_to_linux(ev.vk) else { return };
    let value = match ev.action {
        ButtonAction::Press   => 1,
        ButtonAction::Release => 0,
    };
    let mut dev = match device().lock() { Ok(d) => d, Err(_) => return };
    let events = [
        InputEvent::new(EventType::KEY, linux_key.code(), value),
        syn(),
    ];
    dev.emit(&events).ok();
}

// ── Windows VK → Linux evdev key mapping ─────────────────────────────────────

/// Map a Windows Virtual Key code to an evdev `Key`.
/// Returns `None` for unmapped / irrelevant codes.
fn vk_to_linux(vk: u32) -> Option<Key> {
    Some(match vk {
        // Control characters
        0x08 => Key::KEY_BACKSPACE,
        0x09 => Key::KEY_TAB,
        0x0D => Key::KEY_ENTER,
        0x1B => Key::KEY_ESC,
        0x20 => Key::KEY_SPACE,

        // Modifier keys
        0x10 => Key::KEY_LEFTSHIFT,
        0x11 => Key::KEY_LEFTCTRL,
        0x12 => Key::KEY_LEFTALT,
        0xA0 => Key::KEY_LEFTSHIFT,
        0xA1 => Key::KEY_RIGHTSHIFT,
        0xA2 => Key::KEY_LEFTCTRL,
        0xA3 => Key::KEY_RIGHTCTRL,
        0xA4 => Key::KEY_LEFTALT,
        0xA5 => Key::KEY_RIGHTALT,
        0x5B => Key::KEY_LEFTMETA,
        0x5C => Key::KEY_RIGHTMETA,

        // Number row 0-9
        0x30 => Key::KEY_0,
        0x31 => Key::KEY_1,
        0x32 => Key::KEY_2,
        0x33 => Key::KEY_3,
        0x34 => Key::KEY_4,
        0x35 => Key::KEY_5,
        0x36 => Key::KEY_6,
        0x37 => Key::KEY_7,
        0x38 => Key::KEY_8,
        0x39 => Key::KEY_9,

        // Letters A-Z (VK codes are uppercase ASCII)
        0x41 => Key::KEY_A,
        0x42 => Key::KEY_B,
        0x43 => Key::KEY_C,
        0x44 => Key::KEY_D,
        0x45 => Key::KEY_E,
        0x46 => Key::KEY_F,
        0x47 => Key::KEY_G,
        0x48 => Key::KEY_H,
        0x49 => Key::KEY_I,
        0x4A => Key::KEY_J,
        0x4B => Key::KEY_K,
        0x4C => Key::KEY_L,
        0x4D => Key::KEY_M,
        0x4E => Key::KEY_N,
        0x4F => Key::KEY_O,
        0x50 => Key::KEY_P,
        0x51 => Key::KEY_Q,
        0x52 => Key::KEY_R,
        0x53 => Key::KEY_S,
        0x54 => Key::KEY_T,
        0x55 => Key::KEY_U,
        0x56 => Key::KEY_V,
        0x57 => Key::KEY_W,
        0x58 => Key::KEY_X,
        0x59 => Key::KEY_Y,
        0x5A => Key::KEY_Z,

        // Function keys
        0x70 => Key::KEY_F1,
        0x71 => Key::KEY_F2,
        0x72 => Key::KEY_F3,
        0x73 => Key::KEY_F4,
        0x74 => Key::KEY_F5,
        0x75 => Key::KEY_F6,
        0x76 => Key::KEY_F7,
        0x77 => Key::KEY_F8,
        0x78 => Key::KEY_F9,
        0x79 => Key::KEY_F10,
        0x7A => Key::KEY_F11,
        0x7B => Key::KEY_F12,

        // Navigation
        0x25 => Key::KEY_LEFT,
        0x26 => Key::KEY_UP,
        0x27 => Key::KEY_RIGHT,
        0x28 => Key::KEY_DOWN,
        0x24 => Key::KEY_HOME,
        0x23 => Key::KEY_END,
        0x21 => Key::KEY_PAGEUP,
        0x22 => Key::KEY_PAGEDOWN,
        0x2D => Key::KEY_INSERT,
        0x2E => Key::KEY_DELETE,

        // Locks / system
        0x14 => Key::KEY_CAPSLOCK,
        0x90 => Key::KEY_NUMLOCK,
        0x91 => Key::KEY_SCROLLLOCK,
        0x2C => Key::KEY_SYSRQ,
        0x13 => Key::KEY_PAUSE,

        // Punctuation / symbols (US layout)
        0xBD => Key::KEY_MINUS,      // VK_OEM_MINUS
        0xBB => Key::KEY_EQUAL,      // VK_OEM_PLUS  (= key without shift)
        0xDB => Key::KEY_LEFTBRACE,  // VK_OEM_4  [
        0xDD => Key::KEY_RIGHTBRACE, // VK_OEM_6  ]
        0xDC => Key::KEY_BACKSLASH,  // VK_OEM_5  backslash
        0xBA => Key::KEY_SEMICOLON,  // VK_OEM_1  ;
        0xDE => Key::KEY_APOSTROPHE, // VK_OEM_7  '
        0xC0 => Key::KEY_GRAVE,      // VK_OEM_3  `
        0xBC => Key::KEY_COMMA,      // VK_OEM_COMMA
        0xBE => Key::KEY_DOT,        // VK_OEM_PERIOD
        0xBF => Key::KEY_SLASH,      // VK_OEM_2  /

        // Numpad
        0x60 => Key::KEY_KP0,
        0x61 => Key::KEY_KP1,
        0x62 => Key::KEY_KP2,
        0x63 => Key::KEY_KP3,
        0x64 => Key::KEY_KP4,
        0x65 => Key::KEY_KP5,
        0x66 => Key::KEY_KP6,
        0x67 => Key::KEY_KP7,
        0x68 => Key::KEY_KP8,
        0x69 => Key::KEY_KP9,
        0x6A => Key::KEY_KPASTERISK,
        0x6B => Key::KEY_KPPLUS,
        0x6D => Key::KEY_KPMINUS,
        0x6E => Key::KEY_KPDOT,
        0x6F => Key::KEY_KPSLASH,

        _ => return None,
    })
}
