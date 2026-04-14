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
/// │  pop frame or silence  │
/// └─────────────────────────┘

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use netshare_core::playout::PlayoutBuffer;
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::{CHANNELS, FRAME_INTERLEAVED, SAMPLE_RATE};

#[derive(Clone)]
pub struct AudioSinkHandle {
    selected_device_name: Arc<Mutex<String>>,
    restart_trigger: Arc<AtomicU32>,
}

impl AudioSinkHandle {
    pub fn selected_device(&self) -> String {
        self.selected_device_name.lock().unwrap().clone()
    }

    pub fn set_device(&self, name: String) {
        *self.selected_device_name.lock().unwrap() = name;
        self.restart_trigger.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn enumerate_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| {
            devices
                .filter_map(|d| d.name().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub fn start(listen_port: u16) -> Result<AudioSinkHandle> {
    let selected_device_name: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let restart_trigger: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));

    let handle = AudioSinkHandle {
        selected_device_name: Arc::clone(&selected_device_name),
        restart_trigger: Arc::clone(&restart_trigger),
    };

    let playout = Arc::new(Mutex::new(PlayoutBuffer::new(4)));
    let playout_cpal = Arc::clone(&playout);
    let dev_name_clone = Arc::clone(&selected_device_name);
    let restart = Arc::clone(&restart_trigger);

    std::thread::spawn(move || {
        loop {
            let restart_version = restart.load(Ordering::Relaxed);
            let dev_name = dev_name_clone.lock().unwrap().clone();
            
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let host = cpal::default_host();
                let device = if dev_name.is_empty() {
                    host.default_output_device()
                } else {
                    host.output_devices()
                        .ok()
                        .and_then(|mut devices| devices.find(|d| d.name().map(|n| n == dev_name).unwrap_or(false)))
                }.unwrap_or_else(|| {
                    host.default_output_device().expect("no output device")
                });
                
                info!("Speaker device: {}", device.name().unwrap_or_default());
                *dev_name_clone.lock().unwrap() = device.name().unwrap_or_default();

                let config = cpal::StreamConfig {
                    channels:    CHANNELS,
                    sample_rate: cpal::SampleRate(SAMPLE_RATE),
                    buffer_size: cpal::BufferSize::Default,
                };

                let mut remainder: Vec<f32> = Vec::new();
                let playout_for_stream = Arc::clone(&playout_cpal);
                let restart_check = Arc::clone(&restart);

                let stream = device.build_output_stream(
                    &config,
                    move |out: &mut [f32], _| {
                        if restart_check.load(Ordering::Relaxed) != restart_version {
                            out.fill(0.0);
                            return;
                        }

                        let mut written = 0;
                        let take = remainder.len().min(out.len());
                        out[..take].copy_from_slice(&remainder[..take]);
                        remainder.drain(..take);
                        written += take;

                        while written < out.len() {
                            match playout_for_stream.lock().unwrap().pop() {
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
                    |e| warn!("speaker stream error: {e}"),
                    None,
                );

                match stream {
                    Ok(s) => {
                        s.play().ok();
                        loop {
                            std::thread::park();
                            if restart.load(Ordering::Relaxed) != restart_version {
                                break;
                            }
                        }
                    }
                    Err(e) => warn!("failed to open speaker stream: {e}"),
                }
            }));

            if result.is_err() {
                warn!("Speaker thread panicked — audio sink disabled");
            }
            
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });

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
                    playout.lock().unwrap_or_else(|e| e.into_inner()).push(frame);
                }
                Err(e) => warn!("Opus decode error: {e}"),
            }
        }
    });

    Ok(handle)
}