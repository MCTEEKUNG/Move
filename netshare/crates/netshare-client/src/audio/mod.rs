/// Client-side audio subsystem.
///
/// Two independent streams:
///   • Audio capture  — CPAL output (System Loopback) → Opus → UDP :9001 → server
///   • System Audio   — UDP :9002 ← server → Opus decode → CPAL output
pub mod audio_capture;
pub mod virtual_mic;

use std::net::SocketAddr;
use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait};
use tracing::warn;

pub const SAMPLE_RATE:       u32   = 48_000;
pub const CHANNELS:          u16   = 2;
pub const FRAME_INTERLEAVED: usize = 480 * 2; // 960 f32

pub struct ClientAudio;

impl ClientAudio {
    /// Start audio subsystems. Returns Ok even if audio is unavailable —
    /// individual failures are logged as warnings and skipped gracefully.
    pub fn start(server_addr: SocketAddr) -> Result<Self> {
        let host = cpal::default_host();

        // Audio device capture (send this machine's desktop audio to server).
        // Platform strategy:
        //   Windows: prefer VB-Cable input, else WASAPI loopback via output device.
        //   Linux:   look for PulseAudio/PipeWire monitor source (never use mic).
        let capture_device = find_loopback_input(&host);

        match capture_device {
            Some(dev) => {
                if let Err(e) = audio_capture::start(dev, server_addr) {
                    tracing::warn!("System audio capture disabled: {e}");
                }
            }
            None => tracing::warn!("No loopback/monitor device found — desktop audio capture disabled"),
        }

        // Playback device (received sound from server → this machine's speaker).
        let playback_device = find_vbcable_output(&host)
            .or_else(|| host.default_output_device());
        match playback_device {
            Some(dev) => {
                if let Err(e) = virtual_mic::start(dev, 9002) {
                    tracing::warn!("Remote audio playback disabled: {e}");
                }
            }
            None => tracing::warn!("No output device — remote audio playback disabled"),
        }

        Ok(Self)
    }
}

/// Find the best device for capturing desktop (loopback) audio on this platform.
///
/// * Windows — prefer VB-Cable input; fall back to default output device
///   (WASAPI will do loopback capture when build_input_stream is called on an
///   output device).
/// * Linux — look for a PulseAudio/PipeWire monitor source in the input device
///   list. NEVER fall back to a real microphone.
fn find_loopback_input(host: &cpal::Host) -> Option<cpal::Device> {
    // Prefer VB-Cable on any platform (user may have installed it).
    if let Some(dev) = host.input_devices().ok()?.find(|d| {
        d.name().map(|n| n.to_lowercase().contains("cable")).unwrap_or(false)
    }) {
        return Some(dev);
    }

    #[cfg(target_os = "windows")]
    {
        // WASAPI loopback: build_input_stream on an output device captures
        // everything playing through the speakers.
        return host.default_output_device();
    }

    #[cfg(target_os = "linux")]
    {
        // Same strategy as server-side: set PULSE_SOURCE to the default sink's
        // monitor, then open the "pulse" or "pipewire" ALSA device.
        if let Ok(out) = std::process::Command::new("pactl")
            .arg("get-default-sink")
            .output()
        {
            let sink = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !sink.is_empty() {
                let monitor = format!("{sink}.monitor");
                tracing::info!("Client audio: using monitor source {monitor}");
                std::env::set_var("PULSE_SOURCE", &monitor);
            }
        }

        let mut fallback: Option<cpal::Device> = None;
        for dev in host.input_devices().ok()?.collect::<Vec<_>>() {
            let name = dev.name().unwrap_or_default();
            let lower = name.to_lowercase();
            if lower == "pulse" || lower == "pipewire" || lower.contains("monitor") {
                return Some(dev);
            }
            if lower == "default" { fallback = Some(dev); }
        }

        if fallback.is_some() {
            tracing::info!("Client audio: using 'default' device with PULSE_SOURCE override");
        } else {
            tracing::warn!("No loopback/monitor device found — desktop audio capture disabled");
        }
        return fallback;
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    None
}

fn find_vbcable_output(host: &cpal::Host) -> Option<cpal::Device> {
    host.output_devices().ok()?.find(|d| {
        d.name()
            .map(|n| n.to_lowercase().contains("cable"))
            .unwrap_or(false)
    })
}

/// Log which audio devices are available — helps the user diagnose VB-Cable.
pub fn log_available_devices() {
    let host = cpal::default_host();
    if let Ok(devs) = host.input_devices() {
        for d in devs.filter_map(|d| d.name().ok()) {
            tracing::debug!("  input : {d}");
        }
    }
    if let Ok(devs) = host.output_devices() {
        for d in devs.filter_map(|d| d.name().ok()) {
            tracing::debug!("  output: {d}");
        }
    }
    let has_cable = host
        .input_devices()
        .ok()
        .map(|mut d| d.any(|dev| dev.name().map(|n| n.to_lowercase().contains("cable")).unwrap_or(false)))
        .unwrap_or(false);

    if !has_cable {
        warn!("VB-Cable not detected — using default audio device. Install VB-Cable for proper audio routing.");
    }
}
