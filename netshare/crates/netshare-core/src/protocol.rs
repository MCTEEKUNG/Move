use serde::{Deserialize, Serialize};
use crate::input::{KeyEvent, MouseClick, MouseMove, MouseScroll};
use crate::audio::AudioConfig;

// ── Packet type constants ──────────────────────────────────────────────────
pub const PKT_HELLO:                u8 = 0x01;
pub const PKT_HELLO_RESPONSE:       u8 = 0xF1;
pub const PKT_MOUSE_MOVE:           u8 = 0x02;
pub const PKT_MOUSE_CLICK:          u8 = 0x03;
pub const PKT_KEY_EVENT:            u8 = 0x04;
pub const PKT_SCROLL:               u8 = 0x05;
pub const PKT_AUDIO_CONFIG:         u8 = 0x10;
pub const PKT_HEARTBEAT:            u8 = 0x11;
pub const PKT_ACTIVE_CLIENT_CHANGE: u8 = 0x12;
pub const PKT_DISCONNECT:           u8 = 0xFF;

// ── Flags byte bitmask ─────────────────────────────────────────────────────
pub const FLAG_COMPRESSED: u8 = 0x01;
pub const FLAG_BROADCAST:  u8 = 0x02;

// ── Wire header ────────────────────────────────────────────────────────────

/// Fixed 8-byte header that precedes every packet on the wire.
///
/// Layout:
///  [0]     pkt_type  (u8)
///  [1..2]  seq       (u16 LE)
///  [3]     flags     (u8)
///  [4..7]  length    (u32 LE) — byte length of the payload that follows
#[derive(Debug, Clone, Copy)]
pub struct PacketHeader {
    pub pkt_type: u8,
    pub seq: u16,
    pub flags: u8,
    pub length: u32,
}

impl PacketHeader {
    pub const SIZE: usize = 8;

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let seq = self.seq.to_le_bytes();
        let len = self.length.to_le_bytes();
        [
            self.pkt_type,
            seq[0], seq[1],
            self.flags,
            len[0], len[1], len[2], len[3],
        ]
    }

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Self {
        Self {
            pkt_type: b[0],
            seq:      u16::from_le_bytes([b[1], b[2]]),
            flags:    b[3],
            length:   u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        }
    }
}

// ── High-level packet enum ─────────────────────────────────────────────────

/// All packet payloads that can travel over the control channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlPacket {
    Hello(Hello),
    HelloResponse(HelloResponse),
    MouseMove(MouseMove),
    MouseClick(MouseClick),
    KeyEvent(KeyEvent),
    Scroll(MouseScroll),
    AudioConfig(AudioConfig),
    Heartbeat,
    ActiveClientChange(ActiveClientChange),
    Disconnect,
}

// ── Handshake payloads ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    /// Protocol version — must match server's expected version.
    pub protocol_version: u16,
    pub client_name: String,
    /// Pairing code entered by the user.  Required when the server has
    /// pairing enabled (`HelloResponse::pairing_required == true`).
    pub pairing_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloResponse {
    pub protocol_version: u16,
    pub server_name: String,
    /// Slot assigned to this client for hotkey switching (1-9).
    pub assigned_slot: u8,
    pub accepted: bool,
    /// Human-readable rejection reason when `accepted == false`.
    pub reject_reason: Option<String>,
    /// Whether a pairing code is required.  Clients should prompt the user
    /// and re-connect with `Hello::pairing_code` set when `true`.
    pub pairing_required: bool,
}

// ── Active client notification ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveClientChange {
    /// Slot of the newly active client (1-9), or 0 = server itself is active.
    pub active_slot: u8,
    pub active_name: String,
}

// ── Protocol version ───────────────────────────────────────────────────────
pub const PROTOCOL_VERSION: u16 = 1;
