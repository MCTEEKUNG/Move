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

/// Open Windows Firewall for inbound audio (UDP :9002) on the client machine.
/// Requires Administrator; silently skips if not elevated.
#[cfg(target_os = "windows")]
fn open_client_firewall_ports() {
    let rules: &[(&str, &str, &str)] = &[
        ("NetShare-Client-UDP", "UDP", "9002"),       // server mic → client speaker
        ("NetShare-Client-TCP", "TCP", "9003,9004"),  // file receive + clipboard
    ];
    for (name, proto, ports) in rules {
        let _ = std::process::Command::new("netsh")
            .args(["advfirewall", "firewall", "delete", "rule",
                   &format!("name={name}")])
            .output();
        let result = std::process::Command::new("netsh")
            .args(["advfirewall", "firewall", "add", "rule",
                   &format!("name={name}"),
                   "dir=in", "action=allow",
                   &format!("protocol={proto}"),
                   &format!("localport={ports}")])
            .output();
        match result {
            Ok(o) if o.status.success() =>
                tracing::info!("Client firewall rule added: {name} ({proto} {ports})"),
            _ =>
                tracing::warn!("Could not add client firewall rule {name} — run as Administrator once"),
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn open_client_firewall_ports() {}

/// Start all client subsystems and return a handle.
/// Spawns async tasks on the current tokio runtime.
pub fn start(server_addr_str: &str, client_name: &str, pairing_code: &str) -> anyhow::Result<ClientHandle> {
    // Open firewall ports on client machine (needed for receiving server mic audio UDP :9002).
    open_client_firewall_ports();

    let server_tcp: SocketAddr = server_addr_str.parse()
        .map_err(|e| anyhow::anyhow!("invalid server address '{server_addr_str}': {e}"))?;

    // Audio (optional — if unavailable, client still works for input/file sharing).
    audio::log_available_devices();
    if let Err(e) = audio::ClientAudio::start(server_tcp) {
        tracing::warn!("Client audio unavailable (audio disabled): {e}");
    }

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
    let code = if pairing_code.is_empty() { None } else { Some(pairing_code.to_owned()) };
    tokio::spawn(async move {
        if let Err(e) = network::run_client(&addr_str, &name, code, gui_for_net).await {
            tracing::error!("Client network error: {e}");
        }
    });

    Ok(ClientHandle { state: gui_state, server_addr: server_tcp, file_send_tx })
}
