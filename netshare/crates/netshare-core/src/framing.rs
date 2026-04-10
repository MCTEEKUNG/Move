/// Framing helpers for the control channel.
///
/// Wire format per message:
///   [header: 8 bytes] [payload: header.length bytes]
///
/// Callers work with `ControlPacket` values; this module handles
/// serialisation (bincode) and the PacketHeader bookkeeping.
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::protocol::{ControlPacket, PacketHeader, PROTOCOL_VERSION};

// ── Generic framing (used by file-transfer and clipboard channels) ─────────

/// Write any `bincode`-serialisable value with the standard 8-byte header.
pub async fn write_value<W, T>(writer: &mut W, pkt_type: u8, value: &T) -> io::Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: serde::Serialize,
{
    let payload = bincode::serde::encode_to_vec(value, bincode::config::standard())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let header = PacketHeader {
        pkt_type,
        seq: next_seq(),
        flags: 0,
        length: payload.len() as u32,
    };

    writer.write_all(&header.to_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Read one value from the wire, returning `(pkt_type, value)`.
pub async fn read_value<R, T>(reader: &mut R) -> io::Result<(u8, T)>
where
    R: AsyncReadExt + Unpin,
    T: serde::de::DeserializeOwned,
{
    let mut hdr_buf = [0u8; PacketHeader::SIZE];
    reader.read_exact(&mut hdr_buf).await?;
    let header = PacketHeader::from_bytes(&hdr_buf);

    if header.length > MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("payload too large: {} bytes", header.length),
        ));
    }

    let mut payload = vec![0u8; header.length as usize];
    reader.read_exact(&mut payload).await?;

    let (value, _) =
        bincode::serde::decode_from_slice::<T, _>(&payload, bincode::config::standard())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    Ok((header.pkt_type, value))
}

/// Maximum payload for file/clipboard channels (4 MB per chunk).
const MAX_PAYLOAD: u32 = 4 * 1024 * 1024;

/// Maximum payload for control-channel packets (64 KB).
/// Control packets (Hello, MouseMove, KeyEvent, etc.) are always tiny.
/// A hard cap prevents an OOM DoS from a malicious peer claiming a huge length.
const MAX_CONTROL_PAYLOAD: u32 = 64 * 1024;

static SEQ: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

fn next_seq() -> u16 {
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Write one `ControlPacket` to `writer` and flush immediately.
/// Use this for low-frequency one-shot packets (Hello, Heartbeat, etc.)
/// where you need the data on the wire right away.
pub async fn write_packet<W>(writer: &mut W, pkt: &ControlPacket, flags: u8) -> io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    write_packet_buffered(writer, pkt, flags).await?;
    writer.flush().await?;
    Ok(())
}

/// Write one `ControlPacket` into the writer's buffer **without** flushing.
///
/// Call `writer.flush()` once after writing the last packet of a batch.
/// This allows multiple packets that arrived between scheduler wakeups to
/// share a single TLS record and TCP segment, dramatically reducing per-packet
/// encryption + syscall overhead on high-frequency input streams (mouse moves
/// at 1 kHz → 1 TLS record per batch instead of one per event).
pub async fn write_packet_buffered<W>(writer: &mut W, pkt: &ControlPacket, flags: u8) -> io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let payload = bincode::serde::encode_to_vec(pkt, bincode::config::standard())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let header = PacketHeader {
        pkt_type: packet_type_of(pkt),
        seq: next_seq(),
        flags,
        length: payload.len() as u32,
    };

    writer.write_all(&header.to_bytes()).await?;
    writer.write_all(&payload).await?;
    Ok(())
}

/// Read one `ControlPacket` from `reader`.
///
/// Enforces `MAX_CONTROL_PAYLOAD` (64 KB) to prevent OOM from a malicious peer
/// claiming a multi-MB control packet.
pub async fn read_packet<R>(reader: &mut R) -> io::Result<(PacketHeader, ControlPacket)>
where
    R: AsyncReadExt + Unpin,
{
    let mut hdr_buf = [0u8; PacketHeader::SIZE];
    reader.read_exact(&mut hdr_buf).await?;
    let header = PacketHeader::from_bytes(&hdr_buf);

    if header.length > MAX_CONTROL_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "control packet too large: {} bytes (max {})",
                header.length, MAX_CONTROL_PAYLOAD
            ),
        ));
    }

    let mut payload = vec![0u8; header.length as usize];
    reader.read_exact(&mut payload).await?;

    let (pkt, _) = bincode::serde::decode_from_slice::<ControlPacket, _>(
        &payload,
        bincode::config::standard(),
    )
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    Ok((header, pkt))
}

fn packet_type_of(pkt: &ControlPacket) -> u8 {
    use crate::protocol::*;
    match pkt {
        ControlPacket::Hello(_)              => PKT_HELLO,
        ControlPacket::HelloResponse(_)      => PKT_HELLO_RESPONSE,
        ControlPacket::MouseMove(_)          => PKT_MOUSE_MOVE,
        ControlPacket::MouseClick(_)         => PKT_MOUSE_CLICK,
        ControlPacket::KeyEvent(_)           => PKT_KEY_EVENT,
        ControlPacket::Scroll(_)             => PKT_SCROLL,
        ControlPacket::AudioConfig(_)        => PKT_AUDIO_CONFIG,
        ControlPacket::Heartbeat             => PKT_HEARTBEAT,
        ControlPacket::ActiveClientChange(_) => PKT_ACTIVE_CLIENT_CHANGE,
        ControlPacket::CursorEnter { .. }    => PKT_CURSOR_ENTER,
        ControlPacket::CursorReturn          => PKT_CURSOR_RETURN,
        ControlPacket::Disconnect            => PKT_DISCONNECT,
    }
}

/// Convenience: send a Hello from the client side.
pub async fn send_hello<W>(
    writer: &mut W,
    client_name: &str,
    pairing_code: Option<String>,
    screen_width: i32,
    screen_height: i32,
) -> io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    use crate::protocol::Hello;
    let pkt = ControlPacket::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        client_name: client_name.to_owned(),
        pairing_code,
        screen_width,
        screen_height,
    });
    write_packet(writer, &pkt, 0).await
}
