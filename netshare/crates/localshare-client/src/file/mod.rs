/// Client-side file transfer and clipboard subsystem.
pub mod receiver;
pub mod sender;
pub mod clipboard;

use std::net::SocketAddr;
use std::path::PathBuf;
use anyhow::Result;
use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpStream;
use tracing::{info, warn};

use localshare_core::{
    file_transfer::{FilePacket, PKT_FILE_RESPONSE},
    framing::{read_value, write_value},
};

/// Default receive folder on the client.
pub fn receive_dir() -> PathBuf {
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("Downloads").join("LocalShare")
}

/// Connect to the server's file-transfer port (:9003) and clipboard port (:9004).
pub fn start(server_addr: SocketAddr) -> Result<()> {
    let recv_dir = receive_dir();
    std::fs::create_dir_all(&recv_dir)?;
    info!("Receive folder: {}", recv_dir.display());

    let server_file_addr = SocketAddr::new(server_addr.ip(), 9003);
    let server_clip_addr = SocketAddr::new(server_addr.ip(), 9004);

    // ── File transfer connection ───────────────────────────────────────────
    let recv_dir_clone = recv_dir.clone();
    tokio::spawn(async move {
        // Retry until server is ready (server might start a moment after client).
        let stream = loop {
            match TcpStream::connect(server_file_addr).await {
                Ok(s) => break s,
                Err(_) => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }
            }
        };
        stream.set_nodelay(true).ok();
        info!("File channel connected to {server_file_addr}");

        let (r, w) = stream.into_split();
        let mut reader = BufReader::new(r);
        let mut writer = BufWriter::new(w);

        // Wait for incoming file requests from the server.
        loop {
            let (_, pkt): (u8, FilePacket) = match read_value(&mut reader).await {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    info!("File channel closed by server");
                    break;
                }
                Err(e) => { warn!("file read error: {e}"); break; }
            };

            match pkt {
                FilePacket::Request(req) => {
                    receiver::handle_incoming(&mut reader, &mut writer, req, &recv_dir_clone).await;
                }
                other => warn!("unexpected file packet: {other:?}"),
            }
        }
    });

    // ── Clipboard connection ───────────────────────────────────────────────
    tokio::spawn(clipboard::run_client(server_clip_addr));

    Ok(())
}
