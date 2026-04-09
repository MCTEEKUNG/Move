/// Desktop audio capture (loopback) → Opus encode → UDP → active client.
///
/// Platform strategy:
///   Windows : WASAPI loopback — build an input stream on the default OUTPUT device.
///             This captures everything playing through the speakers.
///   Linux   : PulseAudio/PipeWire monitor source — enumerate input devices and pick
///             the first one whose name contains "monitor" (e.g.
///             "Monitor of Built-in Audio Analog Stereo").  Falls back to default
///             input if no monitor source is found.
///
/// Architecture
/// ┌────────────────────────────────┐
/// │  CPAL loopback callback        │  (OS audio thread)
/// │  accumulate → 960 f32 frames   │
/// └────────┬───────────────────────┘
///          │ std::sync::mpsc
///          ▼
/// ┌────────────────────────────┐
/// │  bridge thread             │  (blocking recv → tokio send)
/// └────────┬───────────────────┘
///          │ tokio::sync::mpsc
///          ▼
/// ┌──────────────────────────────────────────────────────────────────┐
/// │  tokio encode task: Opus encode → UdpSocket.send_to(target_ip)   │
/// └──────────────────────────────────────────────────────────────────┘

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::{CHANNELS, FRAME_INTERLEAVED, SAMPLE_RATE};

pub fn start(
    mic_target: Arc<Mutex<Option<SocketAddr>>>,
    muted: Arc<AtomicBool>,
) -> Result<()> {
    let (sync_tx, sync_rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(8);
    let (async_tx, mut async_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();

    // ── CPAL loopback capture thread ───────────────────────────────────────
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let host = cpal::default_host();

            // Pick the best loopback device for this platform.
            let device = match find_loopback_device(&host) {
                Some(d) => d,
                None => {
                    warn!("No loopback/monitor device found — desktop audio capture disabled.");
                    return;
                }
            };
            info!("Desktop audio capture device: {}", device.name().unwrap_or_default());

            // Use the device's native config where possible to avoid resampling
            // artifacts (especially important for WASAPI loopback on Windows).
            let config = device
                .default_input_config()
                .map(|c| c.into())
                .unwrap_or_else(|_| cpal::StreamConfig {
                    channels:    CHANNELS,
                    sample_rate: cpal::SampleRate(SAMPLE_RATE),
                    buffer_size: cpal::BufferSize::Default,
                });

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
                |e| warn!("loopback stream error: {e}"),
                None,
            );

            match stream {
                Ok(s) => {
                    s.play().ok();
                    loop { std::thread::park(); }
                }
                Err(e) => warn!("Failed to open loopback stream: {e}"),
            }
        }));
        if result.is_err() {
            warn!("Desktop audio capture thread panicked — audio sharing disabled.");
        }
    });

    // ── Bridge thread: blocking mpsc → tokio ──────────────────────────────
    std::thread::spawn(move || {
        for frame in sync_rx {
            if async_tx.send(frame).is_err() { break; }
        }
    });

    // ── Tokio encode + UDP send task ──────────────────────────────────────
    tokio::spawn(async move {
        let socket = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(e) => { warn!("audio UDP socket error: {e}"); return; }
        };

        // Use Opus `Audio` application (better for music/desktop sounds vs Voip).
        let mut encoder = match opus::Encoder::new(
            SAMPLE_RATE,
            opus::Channels::Stereo,
            opus::Application::Audio,
        ) {
            Ok(e) => e,
            Err(e) => { warn!("Opus encoder init failed: {e}"); return; }
        };
        encoder.set_bitrate(opus::Bitrate::Bits(128_000)).ok();

        let mut out_buf = vec![0u8; 4000];

        while let Some(pcm) = async_rx.recv().await {
            if muted.load(Ordering::Relaxed) { continue; }

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

// ── Platform-specific loopback device selection ────────────────────────────────

/// Returns the best device for capturing desktop (loopback) audio.
///
/// * **Windows** – WASAPI exposes the default *output* device as a loopback
///   source when you call `build_input_stream` on it. We return the default
///   output device so CPAL/WASAPI will do the right thing.
///
/// * **Linux** – PulseAudio and PipeWire expose monitor sources as regular
///   input devices. Their names contain "monitor" (case-insensitive). We pick
///   the first matching device; if none exists we fall back to the default
///   input (microphone) so audio is at least partially functional.
fn find_loopback_device(host: &cpal::Host) -> Option<cpal::Device> {
    #[cfg(target_os = "windows")]
    {
        // On Windows, WASAPI loopback = build_input_stream on an output device.
        // Use the default output device so we capture whatever the user hears.
        let dev = host.default_output_device();
        if dev.is_none() {
            warn!("No default output device for WASAPI loopback.");
        }
        return dev;
    }

    #[cfg(target_os = "linux")]
    {
        // PulseAudio/PipeWire exposes desktop audio as a monitor source —
        // an input device whose name contains "monitor".
        // We NEVER fall back to a real microphone here: the user wants
        // desktop audio only, not mic audio.
        let devices = match host.input_devices() {
            Ok(d) => d,
            Err(e) => {
                warn!("Failed to enumerate input devices: {e}");
                return None;
            }
        };

        for dev in devices {
            let name = dev.name().unwrap_or_default();
            if name.to_lowercase().contains("monitor") {
                info!("Found desktop audio monitor source: {name}");
                return Some(dev);
            }
        }

        // No monitor source found — disable audio capture entirely.
        // Do NOT fall back to microphone (would send wrong audio).
        warn!(
            "No PulseAudio/PipeWire monitor source found — desktop audio disabled.\n\
             Fix: open pavucontrol, or run:\n\
             \t pactl load-module module-null-sink\n\
             Then restart NetShare. On Ubuntu 24.04 with PipeWire this \
             should exist automatically."
        );
        return None;
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    host.default_input_device()
}
