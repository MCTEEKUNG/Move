/// File sender: read file from disk → chunk → send over TCP :9003.
///
/// Supports single files and full directory trees (via walkdir).
/// For each file, the flow is:
///   SHA-256 upfront → FileRequest → wait Accept/Resume → send chunks → FileComplete
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{bail, Result};
use crc32fast::Hasher as Crc32Hasher;
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufWriter};
use tokio::net::TcpStream;
use tracing::{info, warn};
use walkdir::WalkDir;

use localshare_core::{
    file_transfer::{
        FileCancel, FileChunk, FileComplete, FilePacket, FileRequest, FileResponse,
        CHUNK_SIZE, PKT_FILE_CANCEL, PKT_FILE_CHUNK, PKT_FILE_COMPLETE, PKT_FILE_REQUEST,
    },
    framing::{read_value, write_value},
};

static TRANSFER_ID: AtomicU32 = AtomicU32::new(1);

fn next_transfer_id() -> u32 {
    TRANSFER_ID.fetch_add(1, Ordering::Relaxed)
}

/// Entry point: connect to client's :9003 and send `path` (file or folder).
pub async fn send_path(client_ip: std::net::IpAddr, path: PathBuf) -> Result<()> {
    let addr = format!("{client_ip}:9003");
    let stream = TcpStream::connect(&addr).await?;
    stream.set_nodelay(true)?;

    let (r, w) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(r);
    let mut writer = BufWriter::new(w);

    // Collect file list (single file = vec of one entry).
    let files: Vec<(PathBuf, String)> = if path.is_dir() {
        let prefix = path.parent().unwrap_or(&path);
        WalkDir::new(&path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                let rel = e.path().strip_prefix(prefix)
                    .unwrap_or(e.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                (e.into_path(), rel)
            })
            .collect()
    } else {
        let name = path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        vec![(path, name)]
    };

    info!("Sending {} file(s) to {addr}", files.len());

    for (file_path, relative_path) in files {
        if let Err(e) = send_one_file(&mut reader, &mut writer, &file_path, &relative_path).await {
            warn!("Failed to send {}: {e}", file_path.display());
        }
    }

    Ok(())
}

async fn send_one_file<R, W>(
    reader: &mut R,
    writer: &mut W,
    path: &Path,
    relative_path: &str,
) -> Result<()>
where
    R: AsyncReadExt + Unpin,
    W: tokio::io::AsyncWriteExt + Unpin,
{
    let transfer_id = next_transfer_id();
    info!("[{transfer_id}] Sending '{relative_path}'");

    // ── Pre-compute SHA-256 ────────────────────────────────────────────────
    let metadata = tokio::fs::metadata(path).await?;
    let total_size = metadata.len();
    let total_chunks = total_chunks_for(total_size);
    let sha256 = sha256_file(path).await?;

    // ── Send FileRequest ───────────────────────────────────────────────────
    let req = FilePacket::Request(FileRequest {
        transfer_id,
        relative_path: relative_path.to_owned(),
        total_size,
        total_chunks,
        sha256,
    });
    write_value(writer, PKT_FILE_REQUEST, &req).await?;

    // ── Wait for FileResponse ──────────────────────────────────────────────
    let (_, resp): (u8, FilePacket) = read_value(reader).await?;
    let resp = match resp {
        FilePacket::Response(r) => r,
        other => bail!("expected FileResponse, got {other:?}"),
    };

    if !resp.accepted {
        bail!("transfer rejected: {}", resp.reason.unwrap_or_default());
    }

    let start_chunk = resp.resume_from.unwrap_or(0);
    if start_chunk > 0 {
        info!("[{transfer_id}] Resuming from chunk {start_chunk}");
    }

    // ── Send chunks ────────────────────────────────────────────────────────
    let mut file = File::open(path).await?;
    // Seek to resume offset.
    if start_chunk > 0 {
        use tokio::io::AsyncSeekExt;
        file.seek(std::io::SeekFrom::Start(start_chunk as u64 * CHUNK_SIZE as u64)).await?;
    }

    let mut buf = vec![0u8; CHUNK_SIZE];
    for chunk_idx in start_chunk..total_chunks {
        let n = file.read(&mut buf).await?;
        if n == 0 { break; }

        let data = buf[..n].to_vec();
        let mut h = Crc32Hasher::new();
        h.update(&data);
        let crc32 = h.finalize();

        let chunk = FilePacket::Chunk(FileChunk {
            transfer_id,
            chunk_idx,
            crc32,
            data,
        });
        write_value(writer, PKT_FILE_CHUNK, &chunk).await?;

        // Wait for ChunkAck before sending the next chunk.
        let (_, ack): (u8, FilePacket) = read_value(reader).await?;
        match ack {
            FilePacket::ChunkAck(a) if a.chunk_idx == chunk_idx => {}
            FilePacket::Cancel(c) => bail!("receiver cancelled: {}", c.reason),
            other => warn!("[{transfer_id}] unexpected ack: {other:?}"),
        }

        if (chunk_idx + 1) % 64 == 0 || chunk_idx + 1 == total_chunks {
            info!("[{transfer_id}] {}/{total_chunks} chunks sent", chunk_idx + 1);
        }
    }

    // ── Send Complete ──────────────────────────────────────────────────────
    write_value(writer, PKT_FILE_COMPLETE, &FilePacket::Complete(FileComplete { transfer_id })).await?;
    info!("[{transfer_id}] Complete — '{relative_path}'");

    Ok(())
}

fn total_chunks_for(size: u64) -> u32 {
    ((size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64) as u32
}

async fn sha256_file(path: &Path) -> Result<[u8; 32]> {
    let mut file = File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}
