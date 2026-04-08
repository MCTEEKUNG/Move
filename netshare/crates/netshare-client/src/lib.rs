pub mod audio;
pub mod file;
pub mod input_inject;
pub mod network;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
pub use network::{ClientGuiState, ConnectionStatus};

/// Handle the GUI holds to observe and interact with the running client.
pub struct ClientHandle {
    pub state: Arc<Mutex<ClientGuiState>>,
    server_addr: SocketAddr,
    file_send_tx: tokio::sync::mpsc::UnboundedSender<PathBuf>,
}

impl ClientHandle {
    /// Queue a file to be sent to the server.
    pub fn send_file(&self, path: PathBuf) {
        let _ = self.file_send_tx.send(path);
    }
    pub fn server_addr(&self) -> SocketAddr {
        self.server_addr
    }
}

/// Start all client subsystems and return a handle.
/// Spawns async tasks on the current tokio runtime.
pub fn start(server_addr_str: &str, client_name: &str) -> anyhow::Result<ClientHandle> {
    let server_tcp: SocketAddr = server_addr_str.parse()
        .map_err(|e| anyhow::anyhow!("invalid server address '{server_addr_str}': {e}"))?;

    // Audio.
    audio::log_available_devices();
    audio::ClientAudio::start(server_tcp)?;

    // File transfer listener (client receives files from server on :9003).
    file::start(server_tcp)?;

    // File send queue (GUI → server).
    let (file_send_tx, mut file_send_rx) =
        tokio::sync::mpsc::unbounded_channel::<PathBuf>();

    let server_for_send = server_tcp;
    tokio::spawn(async move {
        while let Some(path) = file_send_rx.recv().await {
            if let Err(e) = file::sender::send_path(server_for_send, path).await {
                tracing::warn!("GUI file send error: {e}");
            }
        }
    });

    // Control channel.
    let gui_state: Arc<Mutex<ClientGuiState>> = Arc::new(Mutex::new(ClientGuiState::default()));
    let gui_for_net = gui_state.clone();
    let addr_str = server_addr_str.to_owned();
    let name = client_name.to_owned();
    tokio::spawn(async move {
        if let Err(e) = network::run_client(&addr_str, &name, gui_for_net).await {
            tracing::error!("Client network error: {e}");
        }
    });

    Ok(ClientHandle { state: gui_state, server_addr: server_tcp, file_send_tx })
}
