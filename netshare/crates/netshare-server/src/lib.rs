pub mod active_client;
pub mod audio;
pub mod file;
pub mod input_capture;
pub mod network;
pub mod tls;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use active_client::ActiveClientState;
use audio::ServerAudio;
pub use tls::ServerTls;

/// A pending file-transfer accept request from the GUI's perspective.
pub struct PendingFileRequest {
    pub id:        u32,
    pub filename:  String,
    pub size:      u64,
    pub from_peer: String,
    /// Send `true` to accept, `false` to reject.
    pub response:  tokio::sync::oneshot::Sender<bool>,
}

/// Shared list of incoming file-transfer requests waiting for user approval.
pub type PendingRequests = Arc<Mutex<Vec<PendingFileRequest>>>;

/// Live state the GUI reads.
pub struct ServerHandle {
    pub active:   ActiveClientState,
    pub pairing_code: String,
    pub pending_file_requests: PendingRequests,
    file_send_tx: tokio::sync::mpsc::UnboundedSender<PathBuf>,
}

impl ServerHandle {
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
pub fn start(bind_addr: &str) -> anyhow::Result<ServerHandle> {
    let active  = ActiveClientState::default();
    let audio   = ServerAudio::start()?;
    let tls     = ServerTls::generate()?;
    let pending: PendingRequests = Arc::new(Mutex::new(Vec::new()));

    // File-transfer listener + clipboard.
    file::start(tls.clone(), pending.clone())?;

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

    // Control channel.
    let bind = bind_addr.to_owned();
    let active_for_net = active.clone();
    let pairing = tls.pairing_code.clone();
    let tls_for_net = tls.clone();
    tokio::spawn(async move {
        if let Err(e) = network::run_server(&bind, audio, active_for_net, tls_for_net, pairing).await {
            tracing::error!("Server network error: {e}");
        }
    });

    let pairing_code = tls.pairing_code.clone();
    Ok(ServerHandle { active, pairing_code, pending_file_requests: pending, file_send_tx })
}
