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

        // Audio device capture (send this machine's sound to server).
        // On Windows, we prefer the default output for loopback capture.
        let capture_device = find_vbcable_input(&host)
            .or_else(|| host.default_output_device());
            
        match capture_device {
            Some(dev) => {
                if let Err(e) = audio_capture::start(dev, server_addr) {
                    tracing::warn!("System audio capture disabled: {e}");
                }
            }
            None => tracing::warn!("No output device found — system audio capture disabled"),
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

fn find_vbcable_input(host: &cpal::Host) -> Option<cpal::Device> {
    host.input_devices().ok()?.find(|d| {
        d.name()
            .map(|n| n.to_lowercase().contains("cable"))
            .unwrap_or(false)
    })
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
