/// Client-side clipboard sync (TCP :9004).
///
/// The client:
///   • Connects to server :9004.
///   • Polls local clipboard every 500 ms, sends changes to the server.
///   • Receives clipboard updates from the server and injects them locally.
use std::net::SocketAddr;
use arboard::Clipboard;
use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpStream;
use tokio::time::{interval, Duration};
use tracing::{info, warn};

use netshare_core::{
    file_transfer::{ClipboardImage, ClipboardPacket, ClipboardText, CLIP_IMAGE_MAX_BYTES,
                    PKT_CLIP_IMAGE, PKT_CLIP_TEXT},
    framing::{read_value, write_value},
};

pub async fn run_client(server_addr: SocketAddr) {
    // Retry until server clipboard port is ready.
    let stream = loop {
        match TcpStream::connect(server_addr).await {
            Ok(s) => break s,
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    };
    stream.set_nodelay(true).ok();
    info!("Clipboard channel connected to {server_addr}");

    let (r, w) = stream.into_split();
    let reader = BufReader::new(r);
    let writer = BufWriter::new(w);

    // Use a channel to fan clipboard packets from two sources into one writer.
    let (pkt_tx, mut pkt_rx) = tokio::sync::mpsc::unbounded_channel::<ClipboardPacket>();

    // ── Poll local clipboard → send to server ─────────────────────────────
    let poll_tx = pkt_tx.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis(500));
        let mut last_text: Option<String> = None;
        let mut last_img_hash: u64 = 0;

        loop {
            ticker.tick().await;

            let pkt: Option<ClipboardPacket> = tokio::task::spawn_blocking(|| {
                let mut cb = Clipboard::new().ok()?;
                if let Ok(text) = cb.get_text() {
                    return Some(ClipboardPacket::Text(ClipboardText { content: text }));
                }
                if let Ok(img) = cb.get_image() {
                    if img.bytes.len() <= CLIP_IMAGE_MAX_BYTES {
                        return Some(ClipboardPacket::Image(ClipboardImage {
                            width: img.width, height: img.height,
                            rgba: img.bytes.into_owned(),
                        }));
                    }
                }
                None
            }).await.ok().flatten();

            let Some(pkt) = pkt else { continue };

            let changed = match &pkt {
                ClipboardPacket::Text(t) => {
                    let c = last_text.as_deref() != Some(&t.content);
                    if c { last_text = Some(t.content.clone()); }
                    c
                }
                ClipboardPacket::Image(img) => {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    img.rgba.hash(&mut h);
                    let hash = h.finish();
                    let c = hash != last_img_hash;
                    if c { last_img_hash = hash; }
                    c
                }
            };

            if changed {
                poll_tx.send(pkt).ok();
            }
        }
    });

    // ── Writer task: drain channel → write to TCP ──────────────────────────
    let mut writer = writer;
    let write_task = tokio::spawn(async move {
        while let Some(pkt) = pkt_rx.recv().await {
            let pkt_type = pkt.pkt_type();
            if let Err(e) = write_value(&mut writer, pkt_type, &pkt).await {
                warn!("clipboard write error: {e}");
                break;
            }
        }
    });

    // ── Read incoming clipboard from server and inject locally ─────────────
    let mut reader = reader;
    loop {
        let (_, pkt): (u8, ClipboardPacket) = match read_value(&mut reader).await {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                info!("Clipboard channel closed by server");
                break;
            }
            Err(e) => { warn!("clipboard read error: {e}"); break; }
        };

        let result = tokio::task::spawn_blocking(move || {
            let mut cb = Clipboard::new()?;
            match pkt {
                ClipboardPacket::Text(t) => cb.set_text(t.content)?,
                ClipboardPacket::Image(img) => {
                    cb.set_image(arboard::ImageData {
                        width: img.width, height: img.height,
                        bytes: img.rgba.into(),
                    })?;
                }
            }
            anyhow::Ok(())
        }).await;

        if let Err(e) = result { warn!("clipboard inject error: {e}"); }
    }

    write_task.abort();
}
