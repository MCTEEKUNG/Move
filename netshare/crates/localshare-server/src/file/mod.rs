/// Server-side file transfer and clipboard subsystem.
///
/// TCP :9003 — file transfer (bidirectional)
/// TCP :9004 — clipboard sync (bidirectional)
pub mod sender;
pub mod receiver;
pub mod clipboard;

use std::path::PathBuf;
use anyhow::Result;
use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpListener;
use tracing::{info, warn};

use localshare_core::{
    file_transfer::{FilePacket, sanitize_path},
    framing::{read_value, write_value},
};

/// Default receive folder — files from clients land here.
pub fn receive_dir() -> PathBuf {
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("Downloads").join("LocalShare")
}

/// Start the file-transfer listener on TCP :9003 and clipboard on TCP :9004.
pub fn start(send_queue_rx: tokio::sync::mpsc::UnboundedReceiver<PathBuf>) -> Result<()> {
    // Ensure receive directory exists.
    let recv_dir = receive_dir();
    std::fs::create_dir_all(&recv_dir)?;
    info!("Receive folder: {}", recv_dir.display());

    // ── File transfer listener (:9003) ─────────────────────────────────────
    let recv_dir_clone = recv_dir.clone();
    tokio::spawn(async move {
        let listener = match TcpListener::bind("0.0.0.0:9003").await {
            Ok(l) => l,
            Err(e) => { warn!("file listener bind error: {e}"); return; }
        };
        info!("File transfer listening on TCP :9003");

        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => { warn!("file accept error: {e}"); break; }
            };
            info!("File channel connected from {peer}");
            stream.set_nodelay(true).ok();

            let recv_dir = recv_dir_clone.clone();
            tokio::spawn(async move {
                let (r, w) = stream.into_split();
                let mut reader = BufReader::new(r);
                let mut writer = BufWriter::new(w);

                loop {
                    let (pkt_type, pkt): (u8, FilePacket) = match read_value(&mut reader).await {
                        Ok(v) => v,
                        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                        Err(e) => { warn!("file read error: {e}"); break; }
                    };

                    match pkt {
                        FilePacket::Request(req) => {
                            receiver::handle_incoming(&mut reader, &mut writer, req, &recv_dir).await;
                        }
                        other => warn!("unexpected file packet type 0x{pkt_type:02x}: {other:?}"),
                    }
                }
            });
        }
    });

    // ── Clipboard listener (:9004) ─────────────────────────────────────────
    tokio::spawn(clipboard::run_server(recv_dir));

    Ok(())
}
