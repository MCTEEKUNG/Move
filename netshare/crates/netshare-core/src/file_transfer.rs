/// File transfer and clipboard protocol types.
///
/// Channels:
///   TCP :9003 — file transfer (chunked, bidirectional)
///   TCP :9004 — clipboard sync (bidirectional)

use serde::{Deserialize, Serialize};

// ── Packet type constants ──────────────────────────────────────────────────
pub const PKT_FILE_REQUEST:   u8 = 0x21;
pub const PKT_FILE_RESPONSE:  u8 = 0x22;
pub const PKT_FILE_CHUNK:     u8 = 0x23;
pub const PKT_FILE_CHUNK_ACK: u8 = 0x24;
pub const PKT_FILE_COMPLETE:  u8 = 0x25;
pub const PKT_FILE_CANCEL:    u8 = 0x26;
pub const PKT_CLIP_TEXT:      u8 = 0x27;
pub const PKT_CLIP_IMAGE:     u8 = 0x28;
pub const PKT_FILE_RESUME:    u8 = 0x29;

/// 256 KiB per chunk — good balance between overhead and granularity for resume.
pub const CHUNK_SIZE: usize = 256 * 1024;

/// Clipboard image limit: skip sync if larger than this.
pub const CLIP_IMAGE_MAX_BYTES: usize = 10 * 1024 * 1024;

// ── File transfer packets ──────────────────────────────────────────────────

/// Sender → Receiver: "I want to send you this file."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRequest {
    /// Unique transfer ID for this file within the session.
    pub transfer_id: u32,
    /// Relative path (sanitized before use on receiver side).
    /// For folder transfers this preserves directory structure.
    pub relative_path: String,
    pub total_size: u64,
    pub total_chunks: u32,
    /// SHA-256 of the whole file, computed before sending.
    pub sha256: [u8; 32],
}

/// Receiver → Sender: accept, reject, or resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResponse {
    pub transfer_id: u32,
    pub accepted: bool,
    pub reason: Option<String>,
    /// If Some(n), receiver already has chunks 0..n-1 — resume from chunk n.
    pub resume_from: Option<u32>,
}

/// Sender → Receiver: one chunk of file data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChunk {
    pub transfer_id: u32,
    pub chunk_idx: u32,
    pub crc32: u32,
    pub data: Vec<u8>,
}

/// Receiver → Sender: chunk received and verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChunkAck {
    pub transfer_id: u32,
    pub chunk_idx: u32,
}

/// Sender → Receiver: all chunks have been sent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileComplete {
    pub transfer_id: u32,
}

/// Either side → other: abort this transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCancel {
    pub transfer_id: u32,
    pub reason: String,
}

/// Receiver → Sender: resume from a specific chunk (after reconnect).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResumeRequest {
    pub transfer_id: u32,
    pub resume_from_chunk: u32,
}

// ── File transfer packet enum ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FilePacket {
    Request(FileRequest),
    Response(FileResponse),
    Chunk(FileChunk),
    ChunkAck(FileChunkAck),
    Complete(FileComplete),
    Cancel(FileCancel),
    Resume(FileResumeRequest),
}

impl FilePacket {
    pub fn pkt_type(&self) -> u8 {
        match self {
            Self::Request(_)  => PKT_FILE_REQUEST,
            Self::Response(_) => PKT_FILE_RESPONSE,
            Self::Chunk(_)    => PKT_FILE_CHUNK,
            Self::ChunkAck(_) => PKT_FILE_CHUNK_ACK,
            Self::Complete(_) => PKT_FILE_COMPLETE,
            Self::Cancel(_)   => PKT_FILE_CANCEL,
            Self::Resume(_)   => PKT_FILE_RESUME,
        }
    }
}

// ── Clipboard packets ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardText {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardImage {
    pub width: usize,
    pub height: usize,
    /// Raw RGBA bytes.
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClipboardPacket {
    Text(ClipboardText),
    Image(ClipboardImage),
}

impl ClipboardPacket {
    pub fn pkt_type(&self) -> u8 {
        match self {
            Self::Text(_)  => PKT_CLIP_TEXT,
            Self::Image(_) => PKT_CLIP_IMAGE,
        }
    }
}

// ── Path sanitization ──────────────────────────────────────────────────────

/// Sanitize a relative path received from the network.
/// Returns `None` if the path is rejected (traversal, absolute, empty).
pub fn sanitize_path(raw: &str) -> Option<std::path::PathBuf> {
    use std::path::{Component, Path};

    let path = Path::new(raw);

    // Must be relative.
    if path.is_absolute() { return None; }

    // Reject any component that is `..` or a root directory.
    let clean: std::path::PathBuf = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(p) => Some(p),
            _ => None, // drop `.`, `..`, prefix, root
        })
        .collect();

    if clean.as_os_str().is_empty() { return None; }

    Some(clean)
}
