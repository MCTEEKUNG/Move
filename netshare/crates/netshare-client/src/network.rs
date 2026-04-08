/// Client TCP network layer — control channel on TCP :9000, TLS-wrapped.
use std::sync::{Arc, Mutex};
use tokio::io::BufWriter;
use tokio::net::TcpStream;
use tracing::{info, warn};

use netshare_core::{
    framing::{read_packet, send_hello, write_packet},
    protocol::ControlPacket,
    tls::{make_client_config, SERVER_NAME},
};
use crate::input_inject;

#[derive(Default)]
pub struct ClientGuiState {
    pub status:        ConnectionStatus,
    pub server_name:   String,
    pub assigned_slot: u8,
    pub active_slot:   u8,
    pub active_name:   String,
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
    pairing_code: Option<String>,
    gui: Arc<Mutex<ClientGuiState>>,
) -> anyhow::Result<()> {
    {
        let mut s = gui.lock().unwrap();
        s.status = ConnectionStatus::Connecting;
    }

    let tcp = TcpStream::connect(server_addr).await
        .map_err(|e| {
            let msg = e.to_string();
            gui.lock().unwrap().status = ConnectionStatus::Disconnected(msg.clone());
            anyhow::anyhow!(msg)
        })?;
    tcp.set_nodelay(true)?;

    // TLS handshake.
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(make_client_config()));
    let server_name = rustls::pki_types::ServerName::try_from(SERVER_NAME)
        .unwrap()
        .to_owned();
    let tls_stream = connector.connect(server_name, tcp).await
        .map_err(|e| {
            let msg = format!("TLS handshake failed: {e}");
            gui.lock().unwrap().status = ConnectionStatus::Disconnected(msg.clone());
            anyhow::anyhow!(msg)
        })?;
    info!("TLS connected to {server_addr}");

    let (reader, writer) = tokio::io::split(tls_stream);
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    // ── Handshake ──────────────────────────────────────────────────────────
    send_hello(&mut writer, client_name, pairing_code).await?;

    let (_, pkt) = read_packet(&mut reader).await?;
    let resp = match pkt {
        ControlPacket::HelloResponse(r) => r,
        other => anyhow::bail!("expected HelloResponse, got {:?}", other),
    };

    if !resp.accepted {
        let reason = resp.reject_reason.unwrap_or_default();
        gui.lock().unwrap().status = ConnectionStatus::Disconnected(reason.clone());
        anyhow::bail!("Server rejected: {reason}");
    }

    {
        let mut s = gui.lock().unwrap();
        s.status        = ConnectionStatus::Connected;
        s.server_name   = resp.server_name.clone();
        s.assigned_slot = resp.assigned_slot;
    }
    info!("Connected to '{}' — slot {}", resp.server_name, resp.assigned_slot);

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
                info!("Active client → slot {} ('{}')", change.active_slot, change.active_name);
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
