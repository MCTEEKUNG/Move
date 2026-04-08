/// File receiver (client side): same logic as server receiver, just different call site.
/// Re-exports the server receiver's `handle_incoming` with the same signature.
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use netshare_core::file_transfer::FileRequest;

// The actual implementation is identical to the server side — same algorithm.
// Instead of duplicating, we call into a shared helper from netshare-core.
// For Phase 3, we inline it here to avoid adding a public dep on server crate.

pub async fn handle_incoming<R, W>(
    reader: &mut R,
    writer: &mut W,
    req: FileRequest,
    recv_dir: &Path,
) where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    // Delegate to the same receive logic (duplicated from server/file/receiver.rs).
    // Phase 4: extract shared impl into netshare-core.
    if let Err(e) = receive_file(reader, writer, req, recv_dir).await {
        tracing::warn!("file receive error: {e}");
    }
}

// -- identical to server receiver --

use anyhow::Result;
use crc32fast::Hasher as Crc32Hasher;
use sha2::{Digest, Sha256};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncSeekExt, BufWriter};
use tracing::info;

use netshare_core::{
    file_transfer::{
        FileCancel, FileChunkAck, FileComplete, FilePacket, FileResponse,
        PKT_FILE_CANCEL, PKT_FILE_CHUNK_ACK, PKT_FILE_RESPONSE,
        CHUNK_SIZE, sanitize_path,
    },
    framing::{read_value, write_value},
};

async fn receive_file<R, W>(
    reader: &mut R,
    writer: &mut W,
    req: FileRequest,
    recv_dir: &Path,
) -> Result<()>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let transfer_id = req.transfer_id;
    info!("[{transfer_id}] Incoming '{}' ({} bytes)", req.relative_path, req.total_size);

    let rel = match sanitize_path(&req.relative_path) {
        Some(p) => p,
        None => {
            write_value(writer, PKT_FILE_RESPONSE, &FilePacket::Response(FileResponse {
                transfer_id, accepted: false,
                reason: Some("invalid path".into()), resume_from: None,
            })).await?;
            return Ok(());
        }
    };

    let final_path = recv_dir.join(&rel);
    let part_path = {
        let ext = final_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        final_path.with_extension(format!("{ext}.part"))
    };

    let resume_from = if part_path.exists() {
        let sz = tokio::fs::metadata(&part_path).await?.len();
        (sz / CHUNK_SIZE as u64) as u32
    } else { 0 };

    write_value(writer, PKT_FILE_RESPONSE, &FilePacket::Response(FileResponse {
        transfer_id, accepted: true, reason: None,
        resume_from: if resume_from > 0 { Some(resume_from) } else { None },
    })).await?;

    if let Some(p) = final_path.parent() { tokio::fs::create_dir_all(p).await?; }

    let mut file = if resume_from > 0 {
        OpenOptions::new().write(true).open(&part_path).await?
    } else {
        File::create(&part_path).await?
    };
    if resume_from > 0 {
        file.seek(std::io::SeekFrom::Start(resume_from as u64 * CHUNK_SIZE as u64)).await?;
    }

    let mut fw = BufWriter::new(file);
    let mut sha256 = Sha256::new();

    if resume_from > 0 {
        let mut hf = File::open(&part_path).await?;
        let mut buf = vec![0u8; 64 * 1024];
        let to_hash = resume_from as u64 * CHUNK_SIZE as u64;
        let mut done = 0u64;
        while done < to_hash {
            let take = ((to_hash - done) as usize).min(buf.len());
            let n = hf.read(&mut buf[..take]).await?;
            if n == 0 { break; }
            sha256.update(&buf[..n]);
            done += n as u64;
        }
    }

    let mut received = resume_from;
    loop {
        let (_, pkt): (u8, FilePacket) = read_value(reader).await?;
        match pkt {
            FilePacket::Chunk(chunk) if chunk.transfer_id == transfer_id => {
                let mut h = Crc32Hasher::new();
                h.update(&chunk.data);
                if h.finalize() != chunk.crc32 {
                    write_value(writer, PKT_FILE_CANCEL, &FilePacket::Cancel(FileCancel {
                        transfer_id,
                        reason: format!("CRC32 mismatch chunk {}", chunk.chunk_idx),
                    })).await?;
                    anyhow::bail!("CRC32 mismatch");
                }
                sha256.update(&chunk.data);
                fw.write_all(&chunk.data).await?;
                received += 1;
                write_value(writer, PKT_FILE_CHUNK_ACK, &FilePacket::ChunkAck(FileChunkAck {
                    transfer_id, chunk_idx: chunk.chunk_idx,
                })).await?;
                if received % 64 == 0 || received == req.total_chunks {
                    info!("[{transfer_id}] {received}/{} chunks", req.total_chunks);
                }
            }
            FilePacket::Complete(c) if c.transfer_id == transfer_id => break,
            FilePacket::Cancel(c) => anyhow::bail!("sender cancelled: {}", c.reason),
            other => tracing::warn!("[{transfer_id}] unexpected: {other:?}"),
        }
    }

    fw.flush().await?;
    drop(fw);

    let computed: [u8; 32] = sha256.finalize().into();
    if computed != req.sha256 {
        tokio::fs::remove_file(&part_path).await.ok();
        anyhow::bail!("[{transfer_id}] SHA-256 mismatch");
    }

    tokio::fs::rename(&part_path, &final_path).await?;
    info!("[{transfer_id}] Saved to {}", final_path.display());
    Ok(())
}
