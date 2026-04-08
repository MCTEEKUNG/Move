mod audio;
mod file;
mod input_inject;
mod network;

use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("netshare=debug".parse()?))
        .init();

    let server_addr_str = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:9000".to_owned());

    let server_tcp: std::net::SocketAddr = server_addr_str.parse()
        .map_err(|e| anyhow::anyhow!("invalid server address '{server_addr_str}': {e}"))?;

    let client_name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "client".to_owned());

    info!("NetShare Client → {server_tcp} as '{client_name}'");

    // Phase 2: Audio.
    audio::log_available_devices();
    audio::ClientAudio::start(server_tcp)?;
    info!("Audio subsystem started");

    // Phase 3: File transfer + clipboard.
    file::start(server_tcp)?;
    info!("File transfer subsystem started (recv: {})", file::receive_dir().display());

    // Phase 1: TCP control channel (blocks until shutdown).
    network::run_client(&server_addr_str, &client_name).await
}
