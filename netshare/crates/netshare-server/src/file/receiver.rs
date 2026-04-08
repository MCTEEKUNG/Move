/// File receiver (server side): accept incoming chunks from a client, write to disk.
///
/// Handles:
///   • Auto-accept (Phase 4 GUI will add a prompt)
///   • Path sanitization — strips traversal components
///   • CRC32 per chunk
///   • SHA-256 of full file after reassembly
///   • Resume: if a partial .part file exists, resume from where it left off
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use crc32fast::Hasher as Crc32Hasher;
use sha2::{Digest, Sha256};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufWriter};
use tracing::{info, warn};

use netshare_core::{
    file_transfer::{
        FileCancel, FileChunkAck, FileComplete, FilePacket, FileRequest, FileResponse,
        PKT_FILE_CANCEL, PKT_FILE_CHUNK_ACK, PKT_FILE_RESPONSE,
        CHUNK_SIZE, sanitize_path,
    },
    framing::{read_value, write_value},
};

pub async fn handle_incoming<R, W>(
    reader: &mut R,
    writer: &mut W,
    req: FileRequest,
    recv_dir: &Path,
) where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    if let Err(e) = receive_file(reader, writer, req, recv_dir).await {
        warn!("file receive error: {e}");
    }
}

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
    info!("[{transfer_id}] Incoming '{}' ({} bytes, {} chunks)",
        req.relative_path, req.total_size, req.total_chunks);

    // ── Sanitize path ──────────────────────────────────────────────────────
    let rel = match sanitize_path(&req.relative_path) {
        Some(p) => p,
        None => {
            let cancel = FilePacket::Response(FileResponse {
                transfer_id,
                accepted: false,
                reason: Some("invalid path".into()),
                resume_from: None,
            });
            write_value(writer, PKT_FILE_RESPONSE, &cancel).await?;
            return Ok(());
        }
    };

    let final_path = recv_dir.join(&rel);
    let part_path  = final_path.with_extension(
        format!("{}.part", final_path.extension().and_then(|e| e.to_str()).unwrap_or(""))
    );

    // ── Check for partial file (resume support) ────────────────────────────
    let resume_from = if part_path.exists() {
        let partial_size = tokio::fs::metadata(&part_path).await?.len();
        let completed = (partial_size / CHUNK_SIZE as u64) as u32;
        info!("[{transfer_id}] Resuming from chunk {completed}");
        completed
    } else {
        0
    };

    // ── Accept + optionally resume ─────────────────────────────────────────
    let response = FilePacket::Response(FileResponse {
        transfer_id,
        accepted: true,
        reason: None,
        resume_from: if resume_from > 0 { Some(resume_from) } else { None },
    });
    write_value(writer, PKT_FILE_RESPONSE, &response).await?;

    // ── Open output file ───────────────────────────────────────────────────
    if let Some(parent) = final_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut file = if resume_from > 0 {
        let f = OpenOptions::new().write(true).open(&part_path).await?;
        f
    } else {
        File::create(&part_path).await?
    };

    // Seek to end of already-received data.
    if resume_from > 0 {
        file.seek(std::io::SeekFrom::Start(resume_from as u64 * CHUNK_SIZE as u64)).await?;
    }

    let mut file_writer = BufWriter::new(file);
    let mut sha256 = Sha256::new();

    // If resuming, hash the already-received bytes.
    if resume_from > 0 {
        let mut hasher_file = File::open(&part_path).await?;
        let mut buf = vec![0u8; 64 * 1024];
        let bytes_to_hash = resume_from as u64 * CHUNK_SIZE as u64;
        let mut hashed = 0u64;
        while hashed < bytes_to_hash {
            let take = ((bytes_to_hash - hashed) as usize).min(buf.len());
            let n = hasher_file.read(&mut buf[..take]).await?;
            if n == 0 { break; }
            sha256.update(&buf[..n]);
            hashed += n as u64;
        }
    }

    // ── Receive chunks ─────────────────────────────────────────────────────
    let mut received = resume_from;
    loop {
        let (_, pkt): (u8, FilePacket) = read_value(reader).await?;

        match pkt {
            FilePacket::Chunk(chunk) if chunk.transfer_id == transfer_id => {
                // Verify CRC32.
                let mut h = Crc32Hasher::new();
                h.update(&chunk.data);
                if h.finalize() != chunk.crc32 {
                    let cancel = FilePacket::Cancel(FileCancel {
                        transfer_id,
                        reason: format!("CRC32 mismatch on chunk {}", chunk.chunk_idx),
                    });
                    write_value(writer, PKT_FILE_CANCEL, &cancel).await?;
                    anyhow::bail!("CRC32 mismatch on chunk {}", chunk.chunk_idx);
                }

                sha256.update(&chunk.data);
                file_writer.write_all(&chunk.data).await?;
                received += 1;

                // Send ACK.
                let ack = FilePacket::ChunkAck(FileChunkAck {
                    transfer_id,
                    chunk_idx: chunk.chunk_idx,
                });
                write_value(writer, PKT_FILE_CHUNK_ACK, &ack).await?;

                if received % 64 == 0 || received == req.total_chunks {
                    info!("[{transfer_id}] {received}/{} chunks received", req.total_chunks);
                }
            }
            FilePacket::Complete(c) if c.transfer_id == transfer_id => {
                break;
            }
            FilePacket::Cancel(c) => {
                anyhow::bail!("sender cancelled: {}", c.reason);
            }
            other => warn!("[{transfer_id}] unexpected: {other:?}"),
        }
    }

    // ── Verify SHA-256 ─────────────────────────────────────────────────────
    file_writer.flush().await?;
    drop(file_writer);

    let computed: [u8; 32] = sha256.finalize().into();
    if computed != req.sha256 {
        tokio::fs::remove_file(&part_path).await.ok();
        anyhow::bail!("[{transfer_id}] SHA-256 mismatch — file discarded");
    }

    // ── Rename .part → final ───────────────────────────────────────────────
    tokio::fs::rename(&part_path, &final_path).await?;
    info!("[{transfer_id}] Saved to {}", final_path.display());

    Ok(())
}
