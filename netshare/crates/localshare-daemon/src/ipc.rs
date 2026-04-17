//! IPC server — lets the GUI read daemon state and send commands.
//!
//! Transport:
//!   Windows : \\.\pipe\localshare
//!   Linux   : /tmp/localshare.sock
//!
//! Protocol: newline-delimited JSON
//!   GUI → daemon: { "cmd": "switch", "slot": 2 }
//!                 { "cmd": "set_share_input", "enabled": true }
//!                 { "cmd": "status" }
//!   daemon → GUI: { "clients": [...], "active_slot": 1, "share_input": true }

use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use serde::{Deserialize, Serialize};

use localshare_server::active_client::ActiveClientState;

// ── Message types ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum GuiCommand {
    Status,
    Switch        { slot: u8 },
    SetShareInput { enabled: bool },
}

#[derive(Serialize)]
struct StatusResponse {
    clients:     Vec<ClientInfo>,
    active_slot: u8,
    share_input: bool,
}

#[derive(Serialize)]
struct ClientInfo {
    slot:   u8,
    name:   String,
    active: bool,
}

// ── Platform transport ────────────────────────────────────────────────────

#[cfg(windows)]
pub async fn serve(
    state:       ActiveClientState,
    switch_tx:   mpsc::UnboundedSender<u8>,
    share_input: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    const PIPE: &str = r"\\.\pipe\localshare";
    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(false)
            .create(PIPE)?;

        debug!("IPC: waiting for GUI connection on {}", PIPE);
        server.connect().await?;
        debug!("IPC: GUI connected");

        let state       = state.clone();
        let switch_tx   = switch_tx.clone();
        let share_input = Arc::clone(&share_input);

        tokio::spawn(async move {
            if let Err(e) = handle_conn(server, state, switch_tx, share_input).await {
                warn!("IPC conn error: {e}");
            }
        });
    }
}

#[cfg(unix)]
pub async fn serve(
    state:       ActiveClientState,
    switch_tx:   mpsc::UnboundedSender<u8>,
    share_input: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    use tokio::net::UnixListener;

    const SOCK: &str = "/tmp/localshare.sock";
    let _ = std::fs::remove_file(SOCK);
    let listener = UnixListener::bind(SOCK)?;
    debug!("IPC: listening on {}", SOCK);

    loop {
        let (stream, _) = listener.accept().await?;
        let state       = state.clone();
        let switch_tx   = switch_tx.clone();
        let share_input = Arc::clone(&share_input);

        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, state, switch_tx, share_input).await {
                warn!("IPC conn error: {e}");
            }
        });
    }
}

// ── Connection handler (generic over stream type) ─────────────────────────

async fn handle_conn<S>(
    stream:      S,
    state:       ActiveClientState,
    switch_tx:   mpsc::UnboundedSender<u8>,
    share_input: Arc<AtomicBool>,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let cmd: GuiCommand = match serde_json::from_str(&line) {
            Ok(c)  => c,
            Err(e) => { warn!("IPC: bad JSON: {e}"); continue; }
        };

        match cmd {
            GuiCommand::Status => {
                let snapshot    = state.snapshot();
                let active_slot = state.active_slot();
                let resp = StatusResponse {
                    clients: snapshot.iter().map(|s| ClientInfo {
                        slot:   s.slot,
                        name:   s.name.clone(),
                        active: s.is_active,
                    }).collect(),
                    active_slot,
                    share_input: share_input.load(Ordering::Relaxed),
                };
                let mut json = serde_json::to_string(&resp)?;
                json.push('\n');
                writer.write_all(json.as_bytes()).await?;
            }
            GuiCommand::Switch { slot } => {
                let _ = switch_tx.send(slot);
            }
            GuiCommand::SetShareInput { enabled } => {
                share_input.store(enabled, Ordering::Relaxed);
            }
        }
    }
    Ok(())
}
