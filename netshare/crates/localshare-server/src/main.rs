mod active_client;
mod audio;
mod file;
mod input_capture;
mod network;

use std::sync::{Arc, atomic::AtomicBool};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("localshare=debug".parse()?))
        .init();

    // Phase 2: Audio subsystem.
    let server_audio = audio::ServerAudio::start()?;
    info!("Audio subsystem started");

    // Phase 3: File transfer + clipboard subsystem.
    // The send_queue_rx is reserved for GUI-triggered sends (Phase 4).
    let (_send_queue_tx, send_queue_rx) = tokio::sync::mpsc::unbounded_channel();
    file::start(send_queue_rx)?;
    info!("File transfer subsystem started (recv: {})", file::receive_dir().display());

    // Phase 1: TCP control server (blocks until shutdown).
    let state       = active_client::ActiveClientState::default();
    let share_input = Arc::new(AtomicBool::new(true));
    let (_switch_tx, switch_rx) = tokio::sync::mpsc::unbounded_channel::<u8>();
    network::run_server("0.0.0.0:9000", server_audio, state, switch_rx, share_input).await
}
