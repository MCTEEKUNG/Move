/// Client TCP network layer.
use std::sync::{Arc, Mutex};
use tokio::io::BufWriter;
use tokio::net::TcpStream;
use tracing::{info, warn};

use netshare_core::{
    framing::{read_packet, send_hello, write_packet},
    protocol::ControlPacket,
};
use crate::input_inject;

#[derive(Default)]
pub struct ClientGuiState {
    pub status: ConnectionStatus,
    pub server_name: String,
    pub assigned_slot: u8,
    pub active_slot: u8,
    pub active_name: String,
}

#[derive(Default, PartialEq, Clone)]
pub enum ConnectionStatus {
    #[default]
    Connecting,
    Connected,
    Disconnected(String),
}

pub async fn run_client(
    server_addr: &str,
    client_name: &str,
    gui: Arc<Mutex<ClientGuiState>>,
) -> anyhow::Result<()> {
    {
        let mut s = gui.lock().unwrap();
        s.status = ConnectionStatus::Connecting;
    }

    let stream = TcpStream::connect(server_addr).await
        .map_err(|e| {
            let msg = e.to_string();
            gui.lock().unwrap().status = ConnectionStatus::Disconnected(msg.clone());
            anyhow::anyhow!(msg)
        })?;
    stream.set_nodelay(true)?;
    info!("Connected to {server_addr}");

    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    // ── Handshake ──────────────────────────────────────────────────────────
    send_hello(&mut writer, client_name).await?;

    let (_, pkt) = read_packet(&mut reader).await?;
    let resp = match pkt {
        ControlPacket::HelloResponse(r) => r,
        other => anyhow::bail!("expected HelloResponse, got {:?}", other),
    };

    if !resp.accepted {
        let reason = resp.reject_reason.unwrap_or_default();
        gui.lock().unwrap().status = ConnectionStatus::Disconnected(reason.clone());
        anyhow::bail!("Server rejected connection: {reason}");
    }

    {
        let mut s = gui.lock().unwrap();
        s.status = ConnectionStatus::Connected;
        s.server_name = resp.server_name.clone();
        s.assigned_slot = resp.assigned_slot;
    }
    info!(
        "Connected to server '{}' — assigned slot {}",
        resp.server_name, resp.assigned_slot
    );

    // ── Main receive loop ──────────────────────────────────────────────────
    let result: anyhow::Result<()> = loop {
        let (_, pkt) = match read_packet(&mut reader).await {
            Ok(v) => v,
            Err(e) => break Err(e.into()),
        };
        match pkt {
            ControlPacket::MouseMove(ev)   => input_inject::inject_mouse_move(ev),
            ControlPacket::MouseClick(ev)  => input_inject::inject_mouse_click(ev),
            ControlPacket::KeyEvent(ev)    => input_inject::inject_key(ev),
            ControlPacket::Scroll(ev)      => input_inject::inject_scroll(ev),

            ControlPacket::ActiveClientChange(change) => {
                info!(
                    "Active client → slot {} ('{}')",
                    change.active_slot, change.active_name
                );
                let mut s = gui.lock().unwrap();
                s.active_slot = change.active_slot;
                s.active_name = change.active_name;
            }

            ControlPacket::Heartbeat => {
                if let Err(e) = write_packet(&mut writer, &ControlPacket::Heartbeat, 0).await {
                    break Err(e.into());
                }
            }

            ControlPacket::Disconnect => {
                info!("Server sent Disconnect");
                break Ok(());
            }

            other => warn!("Unexpected packet: {:?}", other),
        }
    };

    gui.lock().unwrap().status = ConnectionStatus::Disconnected(
        result.as_ref().err().map(|e| e.to_string()).unwrap_or_default()
    );
    result
}
