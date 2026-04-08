/// Audio capture: CPAL input (VB-Cable) → Opus encode → UDP :9001 → server.
use std::net::SocketAddr;

use anyhow::Result;
use cpal::traits::{DeviceTrait, StreamTrait};
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::{CHANNELS, FRAME_INTERLEAVED, SAMPLE_RATE};

pub fn start(device: cpal::Device, server_addr: SocketAddr) -> Result<()> {
    let (sync_tx, sync_rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(8);
    let (async_tx, mut async_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();

    // ── CPAL capture thread ────────────────────────────────────────────────
    std::thread::spawn(move || {
        info!("Audio capture device: {}", device.name().unwrap_or_default());

        let config = cpal::StreamConfig {
            channels:    CHANNELS,
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };

        let mut buf: Vec<f32> = Vec::with_capacity(FRAME_INTERLEAVED * 2);

        let stream = device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                buf.extend_from_slice(data);
                while buf.len() >= FRAME_INTERLEAVED {
                    let frame: Vec<f32> = buf.drain(..FRAME_INTERLEAVED).collect();
                    sync_tx.try_send(frame).ok();
                }
            },
            |e| warn!("audio capture error: {e}"),
            None,
        );

        match stream {
            Ok(s) => { s.play().ok(); loop { std::thread::park(); } }
            Err(e) => warn!("failed to open audio capture stream: {e}"),
        }
    });

    // ── Bridge thread ──────────────────────────────────────────────────────
    std::thread::spawn(move || {
        for frame in sync_rx {
            if async_tx.send(frame).is_err() { break; }
        }
    });

    // ── Tokio encode + send task ───────────────────────────────────────────
    // Client sends to server port 9001.
    let server_audio_addr = SocketAddr::new(server_addr.ip(), 9001);

    tokio::spawn(async move {
        let socket = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(e) => { warn!("audio capture UDP socket error: {e}"); return; }
        };

        let mut encoder = match opus::Encoder::new(SAMPLE_RATE, opus::Channels::Stereo, opus::Application::Audio) {
            Ok(e) => e,
            Err(e) => { warn!("Opus encoder init failed: {e}"); return; }
        };
        encoder.set_bitrate(opus::Bitrate::Bits(128_000)).ok();

        let mut out_buf = vec![0u8; 4000];

        info!("Audio capture sending to {server_audio_addr}");

        while let Some(pcm) = async_rx.recv().await {
            match encoder.encode_float(&pcm, &mut out_buf) {
                Ok(n) => { socket.send_to(&out_buf[..n], server_audio_addr).await.ok(); }
                Err(e) => warn!("Opus encode error: {e}"),
            }
        }
    });

    Ok(())
}
