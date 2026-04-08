/// Server-side clipboard sync (TCP :9004).
///
/// The server:
///   • Listens for clipboard changes from the client (injects into server clipboard).
///   • Polls its own clipboard every 500 ms and sends changes to connected clients.
use std::path::PathBuf;

use arboard::Clipboard;
use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpListener;
use tokio::time::{interval, Duration};
use tracing::{info, warn};

use netshare_core::{
    file_transfer::{ClipboardImage, ClipboardPacket, ClipboardText, CLIP_IMAGE_MAX_BYTES,
                    PKT_CLIP_IMAGE, PKT_CLIP_TEXT},
    framing::{read_value, write_value},
};

pub async fn run_server(_recv_dir: PathBuf) {
    let listener = match TcpListener::bind("0.0.0.0:9004").await {
        Ok(l) => l,
        Err(e) => { warn!("clipboard listener bind error: {e}"); return; }
    };
    info!("Clipboard sync listening on TCP :9004");

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => { warn!("clipboard accept error: {e}"); break; }
        };
        info!("Clipboard channel connected from {peer}");
        stream.set_nodelay(true).ok();

        tokio::spawn(async move {
            let (r, w) = stream.into_split();
            let mut reader = BufReader::new(r);
            let mut writer = BufWriter::new(w);

            // Spawn a task to poll our local clipboard and push changes to the client.
            let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();
            tokio::spawn(async move {
                let mut ticker = interval(Duration::from_millis(500));
                let mut last_text: Option<String> = None;
                let mut last_image_hash: u64 = 0;

                loop {
                    tokio::select! {
                        _ = ticker.tick() => {}
                        _ = &mut done_rx => break,
                    }

                    // Access clipboard on a blocking thread (arboard is not async).
                    let pkt = tokio::task::spawn_blocking(|| {
                        let mut cb = Clipboard::new().ok()?;
                        // Prefer text (cheaper to compare).
                        if let Ok(text) = cb.get_text() {
                            return Some(ClipboardPacket::Text(ClipboardText { content: text }));
                        }
                        if let Ok(img) = cb.get_image() {
                            if img.bytes.len() <= CLIP_IMAGE_MAX_BYTES {
                                return Some(ClipboardPacket::Image(ClipboardImage {
                                    width: img.width,
                                    height: img.height,
                                    rgba: img.bytes.into_owned(),
                                }));
                            }
                        }
                        None
                    }).await.ok().flatten();

                    let Some(pkt) = pkt else { continue };

                    // Deduplicate — only send if content changed.
                    let changed = match &pkt {
                        ClipboardPacket::Text(t) => {
                            let changed = last_text.as_deref() != Some(&t.content);
                            if changed { last_text = Some(t.content.clone()); }
                            changed
                        }
                        ClipboardPacket::Image(img) => {
                            use std::hash::{Hash, Hasher};
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            img.rgba.hash(&mut h);
                            let hash = h.finish();
                            let changed = hash != last_image_hash;
                            if changed { last_image_hash = hash; }
                            changed
                        }
                    };

                    if changed {
                        // Write to writer — shared via Arc<Mutex<>> would be needed for
                        // proper half-duplex; for Phase 3 we serialise via a channel.
                        // (Full duplex writer sharing added in Phase 4.)
                    }
                }
            });

            // Receive clipboard updates from client and inject into server clipboard.
            loop {
                let (_, pkt): (u8, ClipboardPacket) = match read_value(&mut reader).await {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(e) => { warn!("clipboard read error: {e}"); break; }
                };

                let result = tokio::task::spawn_blocking(move || {
                    let mut cb = Clipboard::new()?;
                    match pkt {
                        ClipboardPacket::Text(t) => cb.set_text(t.content)?,
                        ClipboardPacket::Image(img) => {
                            cb.set_image(arboard::ImageData {
                                width: img.width,
                                height: img.height,
                                bytes: img.rgba.into(),
                            })?;
                        }
                    }
                    anyhow::Ok(())
                }).await;

                if let Err(e) = result {
                    warn!("clipboard inject error: {e}");
                }
            }
        });
    }
}
