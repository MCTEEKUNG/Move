pub mod active_client;
pub mod audio;
pub mod file;
pub mod input_capture;
pub mod network;

use std::path::PathBuf;
use active_client::ActiveClientState;
use audio::ServerAudio;

/// Live state the GUI reads.
pub struct ServerHandle {
    pub active: ActiveClientState,
    file_send_tx: tokio::sync::mpsc::UnboundedSender<PathBuf>,
}

impl ServerHandle {
    /// Snapshot of connected clients: `(slot, name)`.
    pub fn clients(&self) -> Vec<(u8, String)> {
        self.active.clients_snapshot()
    }
    pub fn active_slot(&self) -> u8 {
        self.active.active_slot()
    }
    pub fn broadcast_mode(&self) -> bool {
        self.active.broadcast_mode()
    }
    pub fn set_broadcast_mode(&self, val: bool) {
        self.active.set_broadcast_mode(val);
    }
    /// Queue a file to be sent to the currently active client.
    pub fn send_file(&self, path: PathBuf) {
        let _ = self.file_send_tx.send(path);
    }
}

/// Start the server subsystems and return a handle the GUI can read from.
/// The actual TCP/audio/file tasks are spawned on the current tokio runtime.
pub fn start(bind_addr: &str) -> anyhow::Result<ServerHandle> {
    let active = ActiveClientState::default();
    let audio  = ServerAudio::start()?;

    // File-transfer listener + clipboard.
    let (_clip_tx, send_queue_rx) = tokio::sync::mpsc::unbounded_channel();
    file::start(send_queue_rx)?;

    // Channel for GUI-triggered file sends to the active client.
    let (file_send_tx, mut file_send_rx) =
        tokio::sync::mpsc::unbounded_channel::<PathBuf>();

    let active_for_send = active.clone();
    tokio::spawn(async move {
        while let Some(path) = file_send_rx.recv().await {
            if let Some(ip) = active_for_send.active_client_ip() {
                if let Err(e) = file::sender::send_path(ip, path).await {
                    tracing::warn!("GUI file send error: {e}");
                }
            } else {
                tracing::warn!("GUI file send: no active client");
            }
        }
    });

    // Control channel (blocks until shutdown — run as a detached task).
    let bind = bind_addr.to_owned();
    let active_for_net = active.clone();
    tokio::spawn(async move {
        if let Err(e) = network::run_server(&bind, audio, active_for_net).await {
            tracing::error!("Server network error: {e}");
        }
    });

    Ok(ServerHandle { active, file_send_tx })
}
