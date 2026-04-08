//! Windows SendInput injection.
use tracing::warn;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE,
    KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP,
    MOUSEINPUT, MOUSE_EVENT_FLAGS,
};
use netshare_core::input::{ButtonAction, KeyEvent, KeyFlags, MouseButton, MouseClick, MouseMove, MouseScroll};

pub fn inject_mouse_move(ev: MouseMove) {
    // The server sends raw screen coordinates. We convert to the
    // normalised 0..65535 range that MOUSEEVENTF_ABSOLUTE expects.
    // (Server and client may have different resolutions — a proper
    //  coordinate-scaling pass will be added in the GUI phase.)
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: ev.dx,
                dy: ev.dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send(&[input]);
}

pub fn inject_mouse_click(ev: MouseClick) {
    let (down_flag, up_flag, data) = button_flags(ev.button);
    let flags = if ev.action == ButtonAction::Press { down_flag } else { up_flag };
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: ev.x,
                dy: ev.y,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send(&[input]);
}

pub fn inject_scroll(ev: MouseScroll) {
    let mut inputs = Vec::with_capacity(2);
    if ev.delta_y != 0 {
        inputs.push(INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    mouseData: ev.delta_y as u32,
                    dwFlags: MOUSEEVENTF_WHEEL,
                    ..Default::default()
                },
            },
        });
    }
    if ev.delta_x != 0 {
        inputs.push(INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    mouseData: ev.delta_x as u32,
                    dwFlags: MOUSEEVENTF_HWHEEL,
                    ..Default::default()
                },
            },
        });
    }
    if !inputs.is_empty() {
        send(&inputs);
    }
}

pub fn inject_key(ev: KeyEvent) {
    let mut flags = if ev.action == ButtonAction::Release {
        KEYEVENTF_KEYUP
    } else {
        Default::default()
    };
    if ev.flags.contains(KeyFlags::EXTENDED) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if ev.scan != 0 {
        flags |= KEYEVENTF_SCANCODE;
    }

    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk:         windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(ev.vk as u16),
                wScan:       ev.scan,
                dwFlags:     flags,
                time:        0,
                dwExtraInfo: 0,
            },
        },
    };
    send(&[input]);
}

fn send(inputs: &[INPUT]) {
    let sent = unsafe {
        SendInput(inputs, std::mem::size_of::<INPUT>() as i32)
    };
    if sent != inputs.len() as u32 {
        warn!("SendInput sent {sent}/{} events", inputs.len());
    }
}

fn button_flags(btn: MouseButton) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS, u32) {
    match btn {
        MouseButton::Left   => (MOUSEEVENTF_LEFTDOWN,   MOUSEEVENTF_LEFTUP,   0),
        MouseButton::Right  => (MOUSEEVENTF_RIGHTDOWN,  MOUSEEVENTF_RIGHTUP,  0),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
        MouseButton::X1     => (MOUSEEVENTF_XDOWN,      MOUSEEVENTF_XUP,      1),
        MouseButton::X2     => (MOUSEEVENTF_XDOWN,      MOUSEEVENTF_XUP,      2),
    }
}
