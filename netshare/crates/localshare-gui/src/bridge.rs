//! GUI ↔ Daemon bridge.
//!
//! Tries to connect to the running daemon via IPC.
//! If the daemon is not running, falls back to embedded server mode
//! (same behavior as before, so the GUI always works standalone).

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock, atomic::{AtomicBool, Ordering}};
use tokio::sync::mpsc;

use localshare_discovery::{Discovery, Peer};
use localshare_server::active_client::ActiveClientState;
use localshare_server::audio::ServerAudio;

pub struct ServerBridge {
    pub state:         ActiveClientState,
    pub switch_tx:     mpsc::UnboundedSender<u8>,
    pub share_input:   Arc<AtomicBool>,
    audio:             Arc<OnceLock<Arc<ServerAudio>>>,
    pub daemon_mode:   bool,  // true = connected to daemon, false = embedded
    pub audio_devices: Vec<String>,
    /// Peers discovered on the LAN via mDNS (may include peers we aren't yet
    /// connected to).
    pub discovered:    Arc<Mutex<Vec<Peer>>>,
    /// Keep Discovery alive for the lifetime of the bridge.
    _discovery:        Option<Arc<Discovery>>,
}

impl ServerBridge {
    pub fn start() -> Self {
        // Check if daemon IPC is reachable
        let daemon_running = is_daemon_running();

        let state       = ActiveClientState::default();
        let share_input = Arc::new(AtomicBool::new(true));
        let (switch_tx, switch_rx) = mpsc::unbounded_channel::<u8>();
        let audio_lock: Arc<OnceLock<Arc<ServerAudio>>> = Arc::new(OnceLock::new());
        let audio_devices = list_audio_output_devices();

        if daemon_running {
            tracing::info!("Bridge: connecting to running daemon via IPC");
            let switch_tx_ipc = switch_tx.clone();
            let state_ipc     = state.clone();
            let share_ipc     = Arc::clone(&share_input);
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("tokio");
                rt.block_on(ipc_sync_loop(state_ipc, switch_tx_ipc, share_ipc, switch_rx));
            });
            let (discovered, _discovery) = start_discovery();
            return Self {
                state, switch_tx, share_input, audio: audio_lock,
                daemon_mode: true, audio_devices,
                discovered, _discovery,
            };
        }

        // Fallback: embedded server (existing behaviour)
        tracing::info!("Bridge: daemon not found — starting embedded server");
        let bg_state       = state.clone();
        let bg_share_input = Arc::clone(&share_input);
        let bg_audio_lock  = Arc::clone(&audio_lock);

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                let audio = ServerAudio::start_or_stub();
                let audio_arc = Arc::new(audio);
                let _ = bg_audio_lock.set(Arc::clone(&audio_arc));

                let (_file_tx, file_rx) = mpsc::unbounded_channel();
                if let Err(e) = localshare_server::file::start(file_rx) {
                    tracing::warn!("File transfer: {e}");
                }

                // run_server takes ownership of ServerAudio; create a new stub for it
                // since arc is already set for the GUI to use
                let server_audio = ServerAudio::start_or_stub();
                if let Err(e) = localshare_server::network::run_server(
                    "0.0.0.0:9000",
                    server_audio,
                    bg_state,
                    switch_rx,
                    bg_share_input,
                ).await {
                    tracing::error!("Server error: {e}");
                }
            });
        });

        let (discovered, _discovery) = start_discovery();

        Self {
            state, switch_tx, share_input, audio: audio_lock,
            daemon_mode: false, audio_devices,
            discovered, _discovery,
        }
    }

    pub fn set_audio_enabled(&self, enabled: bool) {
        let Some(audio) = self.audio.get() else { return };
        if enabled {
            let addr = self.state.active_client_audio_addr();
            audio.set_mic_target(addr);
        } else {
            audio.set_mic_target(None);
        }
    }

    pub fn switch_to(&self, slot: u8) {
        let _ = self.switch_tx.send(slot);
    }
}

/// Enumerate CPAL output device names for the settings UI.
fn list_audio_output_devices() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    match host.output_devices() {
        Ok(devices) => devices
            .filter_map(|d| d.name().ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Check if the daemon IPC endpoint exists and is connectable.
fn is_daemon_running() -> bool {
    #[cfg(windows)]
    {
        let path = r"\\.\pipe\localshare";
        std::fs::metadata(path).is_ok()
    }
    #[cfg(not(windows))]
    {
        std::path::Path::new("/tmp/localshare.sock").exists()
    }
}

/// When daemon is running: poll IPC for state updates and forward switch commands.
async fn ipc_sync_loop(
    _state:      ActiveClientState,
    _switch_tx:  mpsc::UnboundedSender<u8>,
    share_input: Arc<AtomicBool>,
    mut switch_rx: mpsc::UnboundedReceiver<u8>,
) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    loop {
        // Connect to IPC
        #[cfg(windows)]
        let stream_result: std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> = {
            tokio::net::windows::named_pipe::ClientOptions::new()
                .open(r"\\.\pipe\localshare")
        };
        #[cfg(not(windows))]
        let stream_result = tokio::net::UnixStream::connect("/tmp/localshare.sock").await;

        let stream = match stream_result {
            Ok(s)  => s,
            Err(e) => {
                tracing::warn!("IPC connect failed: {e}, retrying in 2s");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };

        let (reader, mut writer) = tokio::io::split(stream);
        let mut lines = BufReader::new(reader).lines();

        // Poll loop: every 500ms request status, forward any switch commands
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if writer.write_all(b"{\"cmd\":\"status\"}\n").await.is_err() { break; }
                    if let Ok(Some(line)) = lines.next_line().await {
                        if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&line) {
                            if let Some(si) = resp.get("share_input").and_then(|v| v.as_bool()) {
                                share_input.store(si, Ordering::Relaxed);
                            }
                        }
                    }
                }
                Some(slot) = switch_rx.recv() => {
                    let cmd = format!("{{\"cmd\":\"switch\",\"slot\":{}}}\n", slot);
                    if writer.write_all(cmd.as_bytes()).await.is_err() { break; }
                }
            }
        }

        tracing::warn!("IPC connection lost, reconnecting…");
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

// ── Discovery ─────────────────────────────────────────────────────────────

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localshare".into())
}

/// Start mDNS announce + browse. Returns the shared peer list + the Discovery
/// handle (kept alive for the lifetime of the bridge).
///
/// For every newly-discovered peer, spawns a background task that dials its
/// server and keeps a minimal client handshake alive — so this PC appears as a
/// connected slot on the peer's server.
fn start_discovery() -> (Arc<Mutex<Vec<Peer>>>, Option<Arc<Discovery>>) {
    let host  = hostname();
    let disco = match Discovery::new(&host, 9000) {
        Ok(d)  => Arc::new(d),
        Err(e) => {
            tracing::warn!("mDNS discovery disabled: {e}");
            return (Arc::new(Mutex::new(Vec::new())), None);
        }
    };
    if let Err(e) = disco.start() {
        tracing::warn!("mDNS start failed: {e}");
        return (Arc::new(Mutex::new(Vec::new())), None);
    }

    let peers_shared: Arc<Mutex<Vec<Peer>>> = Arc::new(Mutex::new(Vec::new()));

    // Poll Discovery::peers() every second. This avoids the
    // subscribe-after-start race with broadcast channels (any PeerEvent sent
    // before subscribe() is lost). Discovery maintains its own canonical
    // peer map — we just mirror it into `peers_shared` and spawn a dial task
    // for each newly-seen peer.
    {
        let peers_shared = Arc::clone(&peers_shared);
        let disco2 = Arc::clone(&disco);
        let host2  = host.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all().build().expect("tokio (discovery)");
            rt.block_on(async move {
                let mut dialed: HashSet<String> = HashSet::new();
                loop {
                    let peers_now = disco2.peers();
                    tracing::debug!("mDNS peers visible: {}", peers_now.len());

                    // Refresh the UI-visible list.
                    *peers_shared.lock().unwrap() = peers_now.clone();

                    // Dial new ones.
                    for peer in &peers_now {
                        if dialed.insert(peer.name.clone()) {
                            tracing::info!("Dialing newly discovered peer {} @ {}:{}", peer.name, peer.addr, peer.port);
                            let addr: SocketAddr = (std::net::IpAddr::V4(peer.addr), peer.port).into();
                            let me = host2.clone();
                            tokio::spawn(async move {
                                if let Err(e) = dial_peer(addr, me).await {
                                    tracing::warn!("Connect to {addr} failed: {e}");
                                }
                            });
                        }
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            });
        });
    }

    (peers_shared, Some(disco))
}

/// Minimal client handshake: open TCP, send Hello, then heartbeat forever.
/// Doesn't inject input — this task exists only so our PC appears as a connected
/// slot on the peer's server (and later to receive input packets when this PC
/// becomes the active slot there).
async fn dial_peer(addr: SocketAddr, client_name: String) -> anyhow::Result<()> {
    use tokio::io::BufWriter;
    use tokio::net::TcpStream;
    use localshare_core::{
        framing::{read_packet, send_hello, write_packet},
        protocol::ControlPacket,
    };

    // Retry loop: if the peer isn't accepting yet, wait and try again.
    loop {
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                stream.set_nodelay(true).ok();
                let (reader, writer) = stream.into_split();
                let mut reader = tokio::io::BufReader::new(reader);
                let mut writer = BufWriter::new(writer);

                send_hello(&mut writer, &client_name).await?;

                // Read HelloResponse
                let (_, pkt) = read_packet(&mut reader).await?;
                match pkt {
                    ControlPacket::HelloResponse(r) if r.accepted => {
                        tracing::info!("Peer {} accepted us as slot {}", r.server_name, r.assigned_slot);
                    }
                    ControlPacket::HelloResponse(r) => {
                        anyhow::bail!("Peer rejected: {}", r.reject_reason.unwrap_or_default());
                    }
                    other => anyhow::bail!("Peer sent unexpected packet: {:?}", other),
                }

                // Keep-alive: echo heartbeats, ignore input packets (no injector yet
                // on the GUI side — full client binary still handles that).
                loop {
                    match read_packet(&mut reader).await {
                        Ok((_, ControlPacket::Heartbeat)) => {
                            write_packet(&mut writer, &ControlPacket::Heartbeat, 0).await?;
                        }
                        Ok((_, ControlPacket::Disconnect)) => break,
                        Ok(_) => {} // ignore everything else for now
                        Err(e) => {
                            tracing::warn!("Peer {addr} disconnected: {e}");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::debug!("Peer {addr} connect retry: {e}");
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}
