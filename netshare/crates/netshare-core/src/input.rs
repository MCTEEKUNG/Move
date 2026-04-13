use serde::{Deserialize, Serialize};

pub fn capture_timestamp_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u64::MAX as u128) as u64
}

/// Normalised mouse move — delta from last position.
/// Using delta (not absolute coords) avoids server/client resolution mismatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseMove {
    pub dx: i32,
    pub dy: i32,
    pub captured_at_micros: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseClick {
    pub button: MouseButton,
    pub action: ButtonAction,
    /// Absolute position on the *client* screen (filled in by server from
    /// its own cursor position scaled to client resolution).
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseScroll {
    pub delta_x: i32,
    pub delta_y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonAction {
    Press,
    Release,
}

/// A keyboard event.
/// `vk` is the platform-independent virtual key code (Windows VK_* values
/// used as the canonical form; Linux evdev codes are translated on send).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyEvent {
    pub vk: u32,
    pub action: ButtonAction,
    /// Scan code — optional, used for games / apps that read scan codes.
    pub scan: u16,
    pub flags: KeyFlags,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct KeyFlags: u8 {
        const EXTENDED = 0x01;   // extended key (e.g. right Ctrl, numpad Enter)
        const UNICODE  = 0x02;   // vk carries a Unicode codepoint, not a VK_* code
    }
}

#[cfg(test)]
mod tests {
    use super::capture_timestamp_micros;

    #[test]
    fn capture_timestamp_is_non_zero() {
        assert!(capture_timestamp_micros() > 0);
    }
}
