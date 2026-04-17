/// Client-side audio subsystem.
///
/// Two independent streams:
///   • Audio capture  — CPAL input (VB-Cable or default) → Opus → UDP :9001 → server
///   • Virtual mic    — UDP :9002 ← server → Opus decode → CPAL output (VB-Cable or default)
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
    pub fn start(server_addr: SocketAddr) -> Result<Self> {
        let (input_device, output_device) = find_audio_devices();

        audio_capture::start(input_device, server_addr)?;
        virtual_mic::start(output_device, 9002)?;

        Ok(Self)
    }
}

/// On Windows, prefer a VB-Cable device for both directions.
/// Falls back to the system default if VB-Cable is not installed.
fn find_audio_devices() -> (cpal::Device, cpal::Device) {
    let host = cpal::default_host();

    let input = find_vbcable_input(&host)
        .or_else(|| host.default_input_device())
        .expect("no audio input device");

    let output = find_vbcable_output(&host)
        .or_else(|| host.default_output_device())
        .expect("no audio output device");

    (input, output)
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
