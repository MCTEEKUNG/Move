/// Client-side file transfer and clipboard subsystem (TLS-wrapped).
pub mod receiver;
pub mod sender;
pub mod clipboard;

use std::net::SocketAddr;
use std::path::PathBuf;
use anyhow::Result;
use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpStream;
use tracing::{info, warn};

use netshare_core::{
    file_transfer::{FilePacket, PKT_FILE_RESPONSE},
    framing::{read_value, write_value},
    tls::{make_client_config, SERVER_NAME},
};

/// Default receive folder on the client.
pub fn receive_dir() -> PathBuf {
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("Downloads").join("NetShare")
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
        let tcp = loop {
            match TcpStream::connect(server_file_addr).await {
                Ok(s) => break s,
                Err(_) => tokio::time::sleep(tokio::time::Duration::from_millis(500)).await,
            }
        };
        tcp.set_nodelay(true).ok();

        let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(make_client_config()));
        let server_name = rustls::pki_types::ServerName::try_from(SERVER_NAME).unwrap().to_owned();
        let stream = match connector.connect(server_name, tcp).await {
            Ok(s) => s,
            Err(e) => { warn!("file TLS handshake failed: {e}"); return; }
        };
        info!("File channel connected to {server_file_addr} (TLS)");

        let (r, w) = tokio::io::split(stream);
        let mut reader = BufReader::new(r);
        let mut writer = BufWriter::new(w);

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
