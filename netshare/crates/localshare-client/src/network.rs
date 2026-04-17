/// Client TCP network layer.
///
/// Connects to the server, completes handshake, then loops receiving
/// ControlPackets and dispatching each to the appropriate injector.
use tokio::io::BufWriter;
use tokio::net::TcpStream;
use tracing::{info, warn};

use localshare_core::{
    framing::{read_packet, send_hello, write_packet},
    protocol::ControlPacket,
};
use crate::input_inject;

pub async fn run_client(server_addr: &str, client_name: &str) -> anyhow::Result<()> {
    let stream = TcpStream::connect(server_addr).await?;
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
        anyhow::bail!(
            "Server rejected connection: {}",
            resp.reject_reason.unwrap_or_default()
        );
    }

    info!(
        "Connected to server '{}' — assigned slot {}",
        resp.server_name, resp.assigned_slot
    );

    // ── Main receive loop ──────────────────────────────────────────────────
    loop {
        let (_, pkt) = read_packet(&mut reader).await?;
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
                // TODO: update tray indicator in Phase 4 GUI.
            }

            ControlPacket::Heartbeat => {
                // Echo heartbeat back so server knows we're alive.
                write_packet(&mut writer, &ControlPacket::Heartbeat, 0).await?;
            }

            ControlPacket::Disconnect => {
                info!("Server sent Disconnect");
                break;
            }

            other => warn!("Unexpected packet: {:?}", other),
        }
    }

    Ok(())
}
