use serde::{Deserialize, Serialize};

/// Audio stream configuration exchanged during Audio Config (0x10).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub sample_rate: u32,  // 48000
    pub channels: u8,      // 2 (stereo)
    pub bitrate_kbps: u16, // 128
    pub frame_ms: u8,      // 10 ms → 480 samples per frame
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            bitrate_kbps: 128,
            frame_ms: 10,
        }
    }
}

/// An encoded Opus audio frame sent over UDP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFrame {
    pub seq: u32,
    pub timestamp_us: u64, // microseconds since stream start
    pub data: Vec<u8>,     // raw Opus-encoded bytes
}
