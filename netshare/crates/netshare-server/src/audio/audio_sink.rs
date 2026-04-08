/// Audio sink: UDP :9001 ← client → Opus decode → CPAL speaker output.
///
/// Architecture
/// ┌──────────────────────────────────────────┐
/// │  tokio recv task: UdpSocket :9001        │
/// │  Opus decode → push to PlayoutBuffer     │
/// └──────────────────────────────────────────┘
///          │ Arc<Mutex<PlayoutBuffer>>
///          ▼
/// ┌─────────────────────────┐
/// │  CPAL output callback   │  (OS audio thread)
/// │  pop frame or silence   │
/// └─────────────────────────┘

use std::sync::{Arc, Mutex};

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use netshare_core::playout::PlayoutBuffer;
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::{CHANNELS, FRAME_INTERLEAVED, SAMPLE_RATE};

pub fn start(listen_port: u16) -> Result<()> {
    let playout = Arc::new(Mutex::new(PlayoutBuffer::new(4))); // 40 ms pre-buffer
    let playout_cpal = Arc::clone(&playout);

    // ── CPAL speaker output thread ─────────────────────────────────────────
    std::thread::spawn(move || {
        let host   = cpal::default_host();
        let device = match host.default_output_device() {
            Some(d) => d,
            None    => { warn!("no output device — audio sink disabled"); return; }
        };
        info!("Speaker device: {}", device.name().unwrap_or_default());

        let config = cpal::StreamConfig {
            channels:    CHANNELS,
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };

        let mut remainder: Vec<f32> = Vec::new();

        let stream = device.build_output_stream(
            &config,
            move |out: &mut [f32], _| {
                let mut written = 0;

                // Drain any leftover samples from the previous callback.
                let take = remainder.len().min(out.len());
                out[..take].copy_from_slice(&remainder[..take]);
                remainder.drain(..take);
                written += take;

                while written < out.len() {
                    match playout_cpal.lock().unwrap().pop() {
                        Some(frame) => {
                            let take = frame.len().min(out.len() - written);
                            out[written..written + take].copy_from_slice(&frame[..take]);
                            if take < frame.len() {
                                remainder.extend_from_slice(&frame[take..]);
                            }
                            written += take;
                        }
                        None => {
                            // Not enough buffered — output silence for remaining.
                            out[written..].fill(0.0);
                            break;
                        }
                    }
                }
            },
            |e| warn!("speaker stream error: {e}"),
            None,
        );

        match stream {
            Ok(s) => {
                s.play().ok();
                loop { std::thread::park(); }
            }
            Err(e) => warn!("failed to open speaker stream: {e}"),
        }
    });

    // ── Tokio UDP receive + Opus decode task ───────────────────────────────
    tokio::spawn(async move {
        let addr = format!("0.0.0.0:{listen_port}");
        let socket = match UdpSocket::bind(&addr).await {
            Ok(s) => s,
            Err(e) => { warn!("audio sink UDP bind {addr} error: {e}"); return; }
        };
        info!("Audio sink listening on UDP {addr}");

        let mut decoder = match opus::Decoder::new(SAMPLE_RATE, opus::Channels::Stereo) {
            Ok(d) => d,
            Err(e) => { warn!("Opus decoder init failed: {e}"); return; }
        };

        let mut udp_buf  = vec![0u8; 8192];
        let mut pcm_buf  = vec![0f32; FRAME_INTERLEAVED * 4];

        loop {
            let n = match socket.recv(&mut udp_buf).await {
                Ok(n) => n,
                Err(e) => { warn!("audio sink recv error: {e}"); break; }
            };

            match decoder.decode_float(&udp_buf[..n], &mut pcm_buf, false) {
                Ok(samples) => {
                    let frame = pcm_buf[..samples * CHANNELS as usize].to_vec();
                    playout.lock().unwrap().push(frame);
                }
                Err(e) => warn!("Opus decode error: {e}"),
            }
        }
    });

    Ok(())
}
