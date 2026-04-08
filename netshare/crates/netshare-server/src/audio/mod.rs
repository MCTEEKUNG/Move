/// Server-side audio subsystem.
///
/// Two independent streams run in the background:
///   • Mic capture  — CPAL input → Opus encode → UDP :9002 → active client
///   • Audio sink   — UDP :9001 ← client → Opus decode → CPAL output
pub mod mic_capture;
pub mod audio_sink;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use anyhow::Result;

/// 48 kHz, stereo, 10 ms frames → 480 samples/ch → 960 interleaved f32 per frame.
pub const SAMPLE_RATE:    u32 = 48_000;
pub const CHANNELS:       u16 = 2;
pub const FRAME_SAMPLES:  usize = 480;             // per channel
pub const FRAME_INTERLEAVED: usize = FRAME_SAMPLES * CHANNELS as usize; // 960

/// Manages the server audio lifecycle.
pub struct ServerAudio {
    /// Where to send mic audio (active client's IP + port 9002).
    /// Updated by the network layer when the active client changes.
    pub mic_target: Arc<Mutex<Option<SocketAddr>>>,
}

impl ServerAudio {
    /// Start both audio streams. Non-blocking — spawns background threads/tasks.
    pub fn start() -> Result<Self> {
        let mic_target: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));

        mic_capture::start(Arc::clone(&mic_target))?;
        audio_sink::start(9001)?;

        Ok(Self { mic_target })
    }

    /// Called by the network layer when the active client changes.
    pub fn set_mic_target(&self, addr: Option<SocketAddr>) {
        *self.mic_target.lock().unwrap() = addr;
    }
}
