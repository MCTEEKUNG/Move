/// Mic capture: CPAL input → Opus encode → UDP → active client.
///
/// Architecture
/// ┌─────────────────────────┐
/// │  CPAL mic callback      │  (OS audio thread)
/// │  accumulate → 960 f32   │
/// └────────┬────────────────┘
///          │ std::sync::mpsc
///          ▼
/// ┌────────────────────────────┐
/// │  bridge thread             │  (blocking recv → tokio send)
/// └────────┬───────────────────┘
///          │ tokio::sync::mpsc
///          ▼
/// ┌─────────────────────────────────────────────────────────────┐
/// │  tokio encode task: Opus encode → UdpSocket.send_to(target) │
/// └─────────────────────────────────────────────────────────────┘

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::{CHANNELS, FRAME_INTERLEAVED, SAMPLE_RATE};

pub fn start(mic_target: Arc<Mutex<Option<SocketAddr>>>) -> Result<()> {
    let (sync_tx, sync_rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(8);
    let (async_tx, mut async_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();

    // ── CPAL mic capture thread ────────────────────────────────────────────
    std::thread::spawn(move || {
        let host   = cpal::default_host();
        let device = match host.default_input_device() {
            Some(d) => d,
            None    => { warn!("no input device — mic capture disabled"); return; }
        };
        info!("Mic device: {}", device.name().unwrap_or_default());

        let config = cpal::StreamConfig {
            channels:    CHANNELS,
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };

        // Accumulate samples until we have a full Opus frame.
        let mut buf: Vec<f32> = Vec::with_capacity(FRAME_INTERLEAVED * 2);

        let stream = device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                buf.extend_from_slice(data);
                while buf.len() >= FRAME_INTERLEAVED {
                    let frame: Vec<f32> = buf.drain(..FRAME_INTERLEAVED).collect();
                    sync_tx.try_send(frame).ok(); // drop if full (backpressure)
                }
            },
            |e| warn!("mic stream error: {e}"),
            None,
        );

        match stream {
            Ok(s) => {
                s.play().ok();
                // Keep the stream (and thread) alive.
                loop { std::thread::park(); }
            }
            Err(e) => warn!("failed to open mic stream: {e}"),
        }
    });

    // ── Bridge thread: blocking mpsc → tokio channel ───────────────────────
    std::thread::spawn(move || {
        for frame in sync_rx {
            if async_tx.send(frame).is_err() { break; }
        }
    });

    // ── Tokio encode + send task ───────────────────────────────────────────
    tokio::spawn(async move {
        let socket = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(e) => { warn!("mic UDP socket error: {e}"); return; }
        };

        let mut encoder = match opus::Encoder::new(SAMPLE_RATE, opus::Channels::Stereo, opus::Application::Voip) {
            Ok(e) => e,
            Err(e) => { warn!("Opus encoder init failed: {e}"); return; }
        };
        encoder.set_bitrate(opus::Bitrate::Bits(128_000)).ok();

        let mut out_buf = vec![0u8; 4000]; // max Opus packet size

        while let Some(pcm) = async_rx.recv().await {
            let target = *mic_target.lock().unwrap();
            let Some(addr) = target else { continue };

            match encoder.encode_float(&pcm, &mut out_buf) {
                Ok(n) => { socket.send_to(&out_buf[..n], addr).await.ok(); }
                Err(e) => warn!("Opus encode error: {e}"),
            }
        }
    });

    Ok(())
}
