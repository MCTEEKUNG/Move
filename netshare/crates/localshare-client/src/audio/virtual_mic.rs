/// Virtual mic: UDP :9002 ← server mic → Opus decode → CPAL output (VB-Cable).
use std::sync::{Arc, Mutex};

use anyhow::Result;
use cpal::traits::{DeviceTrait, StreamTrait};
use localshare_core::playout::PlayoutBuffer;
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::{CHANNELS, FRAME_INTERLEAVED, SAMPLE_RATE};

pub fn start(device: cpal::Device, listen_port: u16) -> Result<()> {
    let playout = Arc::new(Mutex::new(PlayoutBuffer::new(4)));
    let playout_cpal = Arc::clone(&playout);

    // ── CPAL output thread ─────────────────────────────────────────────────
    std::thread::spawn(move || {
        info!("Virtual mic output device: {}", device.name().unwrap_or_default());

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
                            out[written..].fill(0.0);
                            break;
                        }
                    }
                }
            },
            |e| warn!("virtual mic output error: {e}"),
            None,
        );

        match stream {
            Ok(s) => { s.play().ok(); loop { std::thread::park(); } }
            Err(e) => warn!("failed to open virtual mic output stream: {e}"),
        }
    });

    // ── Tokio UDP receive + Opus decode task ───────────────────────────────
    tokio::spawn(async move {
        let addr = format!("0.0.0.0:{listen_port}");
        let socket = match UdpSocket::bind(&addr).await {
            Ok(s) => s,
            Err(e) => { warn!("virtual mic UDP bind {addr} error: {e}"); return; }
        };
        info!("Virtual mic listening on UDP {addr}");

        let mut decoder = match opus::Decoder::new(SAMPLE_RATE, opus::Channels::Stereo) {
            Ok(d) => d,
            Err(e) => { warn!("Opus decoder init failed: {e}"); return; }
        };

        let mut udp_buf = vec![0u8; 8192];
        let mut pcm_buf = vec![0f32; FRAME_INTERLEAVED * 4];

        loop {
            let n = match socket.recv(&mut udp_buf).await {
                Ok(n) => n,
                Err(e) => { warn!("virtual mic recv error: {e}"); break; }
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
