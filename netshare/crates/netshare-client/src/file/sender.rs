/// File sender (client side): send files to the server.
/// Public API: `send_path(server_addr, path)`.
use std::net::SocketAddr;
use std::path::PathBuf;

pub async fn send_path(server_addr: SocketAddr, path: PathBuf) -> anyhow::Result<()> {
    // Client connects outbound to the server's :9003.
    let target = SocketAddr::new(server_addr.ip(), 9003);
    // Reuse server sender logic — same TCP protocol.
    netshare_server_send(target, path).await
}

// Inline the send logic (mirrors server/file/sender.rs).
// Phase 4: extract to netshare-core shared lib.
use std::sync::atomic::{AtomicU32, Ordering};
use anyhow::{bail, Result};
use crc32fast::Hasher as Crc32Hasher;
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufWriter};
use tokio::net::TcpStream;
use tracing::{info, warn};
use walkdir::WalkDir;

use netshare_core::{
    file_transfer::{
        FileCancel, FileChunk, FileComplete, FilePacket, FileRequest,
        CHUNK_SIZE, PKT_FILE_CANCEL, PKT_FILE_CHUNK, PKT_FILE_COMPLETE, PKT_FILE_REQUEST,
    },
    framing::{read_value, write_value},
};

static TRANSFER_ID: AtomicU32 = AtomicU32::new(1000); // different range from server

fn next_id() -> u32 { TRANSFER_ID.fetch_add(1, Ordering::Relaxed) }

async fn netshare_server_send(addr: SocketAddr, path: PathBuf) -> Result<()> {
    let stream = TcpStream::connect(addr).await?;
    stream.set_nodelay(true)?;
    let (r, w) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(r);
    let mut writer = BufWriter::new(w);

    let files: Vec<(PathBuf, String)> = if path.is_dir() {
        let prefix = path.parent().unwrap_or(&path);
        WalkDir::new(&path).into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                let rel = e.path().strip_prefix(prefix).unwrap_or(e.path())
                    .to_string_lossy().replace('\\', "/");
                (e.into_path(), rel)
            })
            .collect()
    } else {
        let name = path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        vec![(path, name)]
    };

    for (fp, rel) in files {
        if let Err(e) = send_one(&mut reader, &mut writer, &fp, &rel).await {
            warn!("send '{}' failed: {e}", fp.display());
        }
    }
    Ok(())
}

async fn send_one<R, W>(reader: &mut R, writer: &mut W, path: &std::path::Path, rel: &str) -> Result<()>
where R: AsyncReadExt + Unpin, W: tokio::io::AsyncWriteExt + Unpin
{
    let transfer_id = next_id();
    let meta = tokio::fs::metadata(path).await?;
    let total_size = meta.len();
    let total_chunks = ((total_size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64) as u32;
    let sha256 = hash_file(path).await?;

    write_value(writer, PKT_FILE_REQUEST, &FilePacket::Request(FileRequest {
        transfer_id, relative_path: rel.to_owned(),
        total_size, total_chunks, sha256,
    })).await?;

    let (_, resp): (u8, FilePacket) = read_value(reader).await?;
    let resp = match resp {
        FilePacket::Response(r) => r,
        other => bail!("expected Response, got {other:?}"),
    };
    if !resp.accepted { bail!("rejected: {}", resp.reason.unwrap_or_default()); }

    let start = resp.resume_from.unwrap_or(0);
    let mut file = File::open(path).await?;
    if start > 0 {
        use tokio::io::AsyncSeekExt;
        file.seek(std::io::SeekFrom::Start(start as u64 * CHUNK_SIZE as u64)).await?;
    }

    let mut buf = vec![0u8; CHUNK_SIZE];
    for idx in start..total_chunks {
        let n = file.read(&mut buf).await?;
        if n == 0 { break; }
        let data = buf[..n].to_vec();
        let mut h = Crc32Hasher::new(); h.update(&data);
        write_value(writer, PKT_FILE_CHUNK, &FilePacket::Chunk(FileChunk {
            transfer_id, chunk_idx: idx, crc32: h.finalize(), data,
        })).await?;
        let (_, ack): (u8, FilePacket) = read_value(reader).await?;
        match ack {
            FilePacket::ChunkAck(_) => {}
            FilePacket::Cancel(c) => bail!("cancelled: {}", c.reason),
            _ => {}
        }
    }

    write_value(writer, PKT_FILE_COMPLETE, &FilePacket::Complete(FileComplete { transfer_id })).await?;
    info!("[{transfer_id}] Done: '{rel}'");
    Ok(())
}

async fn hash_file(path: &std::path::Path) -> Result<[u8; 32]> {
    let mut f = File::open(path).await?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop { let n = f.read(&mut buf).await?; if n == 0 { break; } h.update(&buf[..n]); }
    Ok(h.finalize().into())
}
