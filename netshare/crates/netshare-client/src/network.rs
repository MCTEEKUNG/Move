/// Client TCP network layer — control channel on TCP :9000, TLS-wrapped.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::io::BufWriter;
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use netshare_core::{
    framing::{read_packet, send_hello, write_packet},
    layout::ClientEdge,
    protocol::ControlPacket,
    tls::{make_client_config, SERVER_NAME},
};
use crate::input_inject;

struct RollingLatencyStats {
    label: &'static str,
    capacity: usize,
    samples: VecDeque<u64>,
}

impl RollingLatencyStats {
    fn new(label: &'static str, capacity: usize) -> Self {
        Self {
            label,
            capacity,
            samples: VecDeque::with_capacity(capacity),
        }
    }

    fn record(&mut self, sample_micros: u64) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample_micros);
        if self.samples.len() == self.capacity {
            self.log_snapshot();
        }
    }

    fn log_snapshot(&self) {
        let mut sorted: Vec<u64> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let p50 = percentile(&sorted, 50);
        let p95 = percentile(&sorted, 95);
        let p99 = percentile(&sorted, 99);
        let max = *sorted.last().unwrap_or(&0);
        debug!(
            target: "netshare::latency",
            label = self.label,
            sample_count = sorted.len(),
            p50_micros = p50,
            p95_micros = p95,
            p99_micros = p99,
            max_micros = max,
            "latency snapshot"
        );
    }
}

fn percentile(sorted: &[u64], pct: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() - 1) * pct) / 100;
    sorted[idx]
}

#[cfg(test)]
mod tests {
    use super::{percentile, RollingLatencyStats};

    #[test]
    fn percentile_handles_small_windows() {
        let values = [100, 200, 300, 400];
        assert_eq!(percentile(&values, 50), 200);
        assert_eq!(percentile(&values, 95), 300);
    }

    #[test]
    fn rolling_stats_drop_oldest_entry() {
        let mut stats = RollingLatencyStats::new("client", 2);
        stats.record(7);
        stats.record(8);
        stats.record(9);

        let samples: Vec<u64> = stats.samples.iter().copied().collect();
        assert_eq!(samples, vec![8, 9]);
    }
}

#[derive(Default)]
pub struct ClientGuiState {
    pub status:        ConnectionStatus,
    pub server_name:   String,
    pub assigned_slot: u8,
    pub active_slot:   u8,
    pub active_name:   String,
    /// Which edge of this client's screen leads back to the server.
    pub return_edge: Option<ClientEdge>,
    /// Client screen width (pixels).
    pub screen_width:  i32,
    /// Client screen height (pixels).
    pub screen_height: i32,
}

#[derive(Default, PartialEq, Clone)]
pub enum ConnectionStatus {
    #[default]
    Connecting,
    Connected,
    Disconnected(String),
}

pub async fn run_client(
    server_addr: &str,
    client_name: &str,
    pairing_code: Option<String>,
    gui: Arc<Mutex<ClientGuiState>>,
) -> anyhow::Result<()> {
    {
        let mut s = gui.lock().unwrap_or_else(|e| e.into_inner());
        s.status = ConnectionStatus::Connecting;
    }

    let tcp = TcpStream::connect(server_addr).await
        .map_err(|e| {
            let msg = e.to_string();
            gui.lock().unwrap_or_else(|e| e.into_inner()).status = ConnectionStatus::Disconnected(msg.clone());
            anyhow::anyhow!(msg)
        })?;
    tcp.set_nodelay(true)?;

    // TLS handshake.
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(make_client_config()));
    let server_name = rustls::pki_types::ServerName::try_from(SERVER_NAME)
        .unwrap()
        .to_owned();
    let tls_stream = connector.connect(server_name, tcp).await
        .map_err(|e| {
            let msg = format!("TLS handshake failed: {e}");
            gui.lock().unwrap_or_else(|e| e.into_inner()).status = ConnectionStatus::Disconnected(msg.clone());
            anyhow::anyhow!(msg)
        })?;
    info!("TLS connected to {server_addr}");

    let (reader, writer) = tokio::io::split(tls_stream);
    let mut reader = tokio::io::BufReader::new(reader);

    // Wrap writer in an Arc<Mutex<>> so we can share it with the cursor-watcher task.
    let writer = Arc::new(tokio::sync::Mutex::new(BufWriter::new(writer)));

    // ── Handshake ──────────────────────────────────────────────────────────
    {
        let mut w = writer.lock().await;
        send_hello(&mut *w, client_name, pairing_code, get_screen_width(), get_screen_height()).await?;
    }

    let (_, pkt) = read_packet(&mut reader).await?;
    let resp = match pkt {
        ControlPacket::HelloResponse(r) => r,
        other => anyhow::bail!("expected HelloResponse, got {:?}", other),
    };

    if !resp.accepted {
        let reason = resp.reject_reason.unwrap_or_default();
        gui.lock().unwrap_or_else(|e| e.into_inner()).status = ConnectionStatus::Disconnected(reason.clone());
        anyhow::bail!("Server rejected: {reason}");
    }

    {
        let mut s = gui.lock().unwrap_or_else(|e| e.into_inner());
        s.status        = ConnectionStatus::Connected;
        s.server_name   = resp.server_name.clone();
        s.assigned_slot = resp.assigned_slot;
        // Detect screen resolution.
        s.screen_width  = get_screen_width();
        s.screen_height = get_screen_height();
    }
    info!("Connected to '{}' — slot {}", resp.server_name, resp.assigned_slot);

    // ── Main receive loop ──────────────────────────────────────────────────
    // Track the cursor-return watcher so we can abort the previous one when
    // a new CursorEnter arrives (rapid edge-crossing would otherwise accumulate
    // unbounded tasks, eventually causing OOM on 24/7 systems).
    let mut return_task: Option<tokio::task::AbortHandle> = None;
    let mut last_mouse_arrival: Option<std::time::Instant> = None;
    let mut arrival_gap = RollingLatencyStats::new("client_mouse_arrival_gap", 2048);
    let mut inject_latency = RollingLatencyStats::new("client_mouse_inject", 2048);

    let result: anyhow::Result<()> = loop {
        let (_, pkt) = match read_packet(&mut reader).await {
            Ok(v) => v,
            Err(e) => break Err(e.into()),
        };
        match pkt {
            ControlPacket::MouseMove(ev)   => {
                let now = std::time::Instant::now();
                if let Some(previous) = last_mouse_arrival.replace(now) {
                    let gap_micros = now.duration_since(previous).as_micros().min(u64::MAX as u128) as u64;
                    arrival_gap.record(gap_micros);
                    if gap_micros > 10_000 {
                        warn!(
                            target: "netshare::latency",
                            gap_micros,
                            "client mouse arrival gap spike"
                        );
                    }
                }
                let inject_start = std::time::Instant::now();
                input_inject::inject_mouse_move(ev);
                inject_latency.record(inject_start.elapsed().as_micros().min(u64::MAX as u128) as u64);
            }
            ControlPacket::MouseClick(ev)  => input_inject::inject_mouse_click(ev),
            ControlPacket::KeyEvent(ev)    => input_inject::inject_key(ev),
            ControlPacket::Scroll(ev)      => input_inject::inject_scroll(ev),

            ControlPacket::CursorEnter { x, y, return_edge } => {
                info!("CursorEnter at ({x},{y}), return_edge={return_edge:?}");
                // Place cursor at the specified position.
                set_cursor_pos(x, y);

                // Release all modifier keys so the client starts with a clean
                // key-state.  The server's hook only begins forwarding AFTER the
                // edge crossing, so any modifiers already held at the moment of
                // crossing (Win, Ctrl, Shift, Alt) were never sent here — leaving
                // the client thinking no modifier is pressed, which breaks combos
                // like Win+E or Ctrl+C.  The server will re-send key-down for any
                // modifier still physically held on its next key event.
                input_inject::release_all_modifiers();

                // Use the return_edge from the packet; also store in gui state.
                let (sw, sh) = {
                    let mut s = gui.lock().unwrap_or_else(|e| e.into_inner());
                    s.return_edge = Some(return_edge);
                    (s.screen_width, s.screen_height)
                };

                // Abort any previous watcher before spawning a new one.
                // Without this, each edge-crossing spawns a permanent task → OOM.
                if let Some(old) = return_task.take() {
                    old.abort();
                }

                let writer_clone = writer.clone();
                let task = tokio::spawn(async move {
                    watch_for_return(return_edge, sw, sh, writer_clone).await;
                });
                return_task = Some(task.abort_handle());
            }

            ControlPacket::ActiveClientChange(change) => {
                info!("Active client → slot {} ('{}')", change.active_slot, change.active_name);
                let mut s = gui.lock().unwrap_or_else(|e| e.into_inner());
                s.active_slot = change.active_slot;
                s.active_name = change.active_name;
            }

            ControlPacket::Heartbeat => {
                let mut w = writer.lock().await;
                if let Err(e) = write_packet(&mut *w, &ControlPacket::Heartbeat, 0).await {
                    break Err(e.into());
                }
            }

            ControlPacket::Disconnect => {
                info!("Server sent Disconnect");
                break Ok(());
            }

            other => warn!("Unexpected packet: {:?}", other),
        }
    };

    gui.lock().unwrap_or_else(|e| e.into_inner()).status = ConnectionStatus::Disconnected(
        result.as_ref().err().map(|e| e.to_string()).unwrap_or_default()
    );
    result
}

/// Returns true when the cursor has reached the edge that leads back to the server.
#[inline]
fn at_return_edge(edge: ClientEdge, cx: i32, cy: i32, sw: i32, sh: i32) -> bool {
    match edge {
        ClientEdge::Left   => cx <= 0,
        ClientEdge::Right  => cx >= sw.saturating_sub(1),
        ClientEdge::Above  => cy <= 0,
        ClientEdge::Below  => cy >= sh.saturating_sub(1),
    }
}

/// Poll the cursor position until it hits the return edge, then send CursorReturn.
///
/// On Linux this function opens one X11 connection that is reused for every
/// poll tick (previously a new connection was created every 8 ms — a severe
/// resource leak on 24/7 systems).
async fn watch_for_return<W>(
    edge: ClientEdge,
    screen_width: i32,
    screen_height: i32,
    writer: Arc<tokio::sync::Mutex<BufWriter<W>>>,
)
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    #[cfg(target_os = "linux")]
    {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::ConnectionExt;

        // Open a single connection for the entire lifetime of this task.
        // Avoids the overhead of creating a new socket + auth handshake every 8 ms.
        let Ok((conn, screen_num)) = x11rb::connect(None) else {
            warn!("watch_for_return: cannot open X11 connection, falling back");
            // fall through to generic path below
            return;
        };
        let root = conn.setup().roots[screen_num].root;

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(4)).await;
            let Ok(cookie) = conn.query_pointer(root) else { break };
            let Ok(reply)  = cookie.reply()            else { break };
            if at_return_edge(edge, reply.root_x as i32, reply.root_y as i32,
                              screen_width, screen_height)
            {
                let mut w = writer.lock().await;
                let _ = write_packet(&mut *w, &ControlPacket::CursorReturn, 0).await;
                break;
            }
        }
        return;
    }

    // Windows / other: platform helpers are cheap per call (no new connection).
    #[allow(unreachable_code)]
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(4)).await;
        let (cx, cy) = get_cursor_pos();
        if at_return_edge(edge, cx, cy, screen_width, screen_height) {
            let mut w = writer.lock().await;
            let _ = write_packet(&mut *w, &ControlPacket::CursorReturn, 0).await;
            break;
        }
    }
}

// ── Platform helpers ──────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn get_screen_width() -> i32 {
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN
        )
    }
}
#[cfg(target_os = "windows")]
fn get_screen_height() -> i32 {
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN
        )
    }
}
#[cfg(target_os = "windows")]
fn get_cursor_pos() -> (i32, i32) {
    let mut pt = windows::Win32::Foundation::POINT::default();
    unsafe { let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt); }
    (pt.x, pt.y)
}
#[cfg(target_os = "windows")]
fn set_cursor_pos(x: i32, y: i32) {
    unsafe { let _ = windows::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y); }
}

#[cfg(target_os = "linux")]
fn get_screen_width() -> i32 {
    use x11rb::connection::Connection;
    let Ok((conn, screen_num)) = x11rb::connect(None) else { return 1920 };
    conn.setup().roots[screen_num].width_in_pixels as i32
}

#[cfg(target_os = "linux")]
fn get_screen_height() -> i32 {
    use x11rb::connection::Connection;
    let Ok((conn, screen_num)) = x11rb::connect(None) else { return 1080 };
    conn.setup().roots[screen_num].height_in_pixels as i32
}

#[cfg(target_os = "linux")]
fn get_cursor_pos() -> (i32, i32) {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    let Ok((conn, screen_num)) = x11rb::connect(None) else { return (0, 0) };
    let root = conn.setup().roots[screen_num].root;
    let Ok(cookie) = conn.query_pointer(root) else { return (0, 0) };
    let Ok(reply)  = cookie.reply() else { return (0, 0) };
    (reply.root_x as i32, reply.root_y as i32)
}

#[cfg(target_os = "linux")]
fn set_cursor_pos(x: i32, y: i32) {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    let Ok((conn, screen_num)) = x11rb::connect(None) else { return };
    let root = conn.setup().roots[screen_num].root;
    conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, x as i16, y as i16).ok();
    conn.flush().ok();
}

// Fallback for any other OS (macOS, etc.)
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn get_screen_width() -> i32 { 1920 }
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn get_screen_height() -> i32 { 1080 }
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn get_cursor_pos() -> (i32, i32) { (0, 0) }
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn set_cursor_pos(_x: i32, _y: i32) {}
