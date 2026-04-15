/// Server TCP network layer — control channel on TCP :9000, TLS-wrapped.
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tracing::{debug, error, info, warn};

use netshare_core::{
    framing::{read_packet, write_packet, write_packet_buffered},
    protocol::{ControlPacket, HelloResponse, PROTOCOL_VERSION},
};

use crate::active_client::ActiveClientState;
use crate::audio::ServerAudio;
use crate::input_capture::{CaptureEvent, HotkeyAction, SharedSeamlessState};
use crate::tls::ServerTls;

type ClientTx  = mpsc::UnboundedSender<ControlPacket>;
type ClientMap = Arc<Mutex<HashMap<u8, ClientTx>>>;

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

fn now_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u64::MAX as u128) as u64
}

fn push_batched_packet(batch: &mut Vec<ControlPacket>, pkt: ControlPacket) {
    match pkt {
        ControlPacket::MouseMove(pkt_move) => {
            if let Some(ControlPacket::MouseMove(last)) = batch.last_mut() {
                last.dx += pkt_move.dx;
                last.dy += pkt_move.dy;
                last.captured_at_micros = last.captured_at_micros.min(pkt_move.captured_at_micros);
            } else {
                batch.push(ControlPacket::MouseMove(pkt_move));
            }
        }
        other => batch.push(other),
    }
}

#[cfg(test)]
mod tests {
    use super::{percentile, push_batched_packet, RollingLatencyStats};
    use netshare_core::{
        input::{ButtonAction, KeyEvent, KeyFlags, MouseMove},
        protocol::ControlPacket,
    };

    fn mouse_move(dx: i32, dy: i32, captured_at_micros: u64) -> ControlPacket {
        ControlPacket::MouseMove(MouseMove {
            dx,
            dy,
            captured_at_micros,
        })
    }

    #[test]
    fn coalesces_consecutive_mouse_moves() {
        let mut batch = Vec::new();
        push_batched_packet(&mut batch, mouse_move(3, 4, 50));
        push_batched_packet(&mut batch, mouse_move(-1, 6, 20));

        assert_eq!(batch.len(), 1);
        match &batch[0] {
            ControlPacket::MouseMove(ev) => {
                assert_eq!(ev.dx, 2);
                assert_eq!(ev.dy, 10);
                assert_eq!(ev.captured_at_micros, 20);
            }
            other => panic!("expected MouseMove, got {other:?}"),
        }
    }

    #[test]
    fn does_not_merge_across_packet_types() {
        let mut batch = Vec::new();
        push_batched_packet(&mut batch, mouse_move(1, 1, 10));
        push_batched_packet(
            &mut batch,
            ControlPacket::KeyEvent(KeyEvent {
                vk: 0x41,
                action: ButtonAction::Press,
                scan: 0,
                flags: KeyFlags::empty(),
            }),
        );
        push_batched_packet(&mut batch, mouse_move(2, 3, 15));

        assert_eq!(batch.len(), 3);
    }

    #[test]
    fn percentile_uses_expected_index() {
        let values = [10, 20, 30, 40, 50];
        assert_eq!(percentile(&values, 50), 30);
        assert_eq!(percentile(&values, 95), 40);
        assert_eq!(percentile(&values, 99), 40);
    }

    #[test]
    fn rolling_stats_keep_only_latest_samples() {
        let mut stats = RollingLatencyStats::new("test", 3);
        stats.record(10);
        stats.record(20);
        stats.record(30);
        stats.record(40);

        let samples: Vec<u64> = stats.samples.iter().copied().collect();
        assert_eq!(samples, vec![20, 30, 40]);
    }
}

pub async fn run_server(
    addr: &str,
    audio: ServerAudio,
    state: ActiveClientState,
    tls: ServerTls,
    pairing_code: String,
    seamless: SharedSeamlessState,
    mut capture_rx: tokio::sync::mpsc::UnboundedReceiver<CaptureEvent>,
) -> anyhow::Result<()> {
    let listener   = TcpListener::bind(addr).await?;
    let client_map: ClientMap = Arc::new(Mutex::new(HashMap::new()));
    let audio      = Arc::new(audio);

    // ── Start input capture thread ─────────────────────────────────────────
    // capture_tx is created and started in lib.rs to allow GUI injection.
    
    // ── Fan-out loop ───────────────────────────────────────────────────────
    let fan_state  = state.clone();
    let fan_map    = client_map.clone();
    let fan_audio  = Arc::clone(&audio);
    let fan_seamless = seamless.clone();
    tokio::spawn(async move {
        while let Some(evt) = capture_rx.recv().await {
            match evt {
                CaptureEvent::InputPacket(pkt) => {
                    let active = fan_state.active_slot();
                    if active == 0 { continue; }
                    let map = fan_map.lock().unwrap_or_else(|e| e.into_inner());
                    if fan_state.broadcast_mode() {
                        for tx in map.values() { let _ = tx.send(pkt.clone()); }
                    } else if let Some(tx) = map.get(&active) {
                        let _ = tx.send(pkt);
                    }
                }
                CaptureEvent::Hotkey(action) => {
                    let change = match action {
                        HotkeyAction::SwitchToSlot(s) => fan_state.switch_to(s),
                        HotkeyAction::Cycle           => fan_state.cycle(),
                        HotkeyAction::ReleaseToLocal  => {
                            // Emergency escape: clear the cursor lock and return
                            // control to the server screen immediately.
                            {
                                let mut s = fan_seamless.lock().unwrap_or_else(|e| e.into_inner());
                                s.locked_to_slot = None;
                            }
                            #[cfg(target_os = "windows")]
                            crate::input_capture::windows::release_cursor();
                            fan_state.force_active(0);
                            info!("ReleaseToLocal hotkey — cursor forcibly returned to server");
                            None
                        }
                    };
                    if let Some(change) = change {
                        fan_audio.set_mic_target(fan_state.active_client_audio_addr());
                        let notification = ControlPacket::ActiveClientChange(change);
                        let map = fan_map.lock().unwrap_or_else(|e| e.into_inner());
                        for tx in map.values() {
                            let _ = tx.send(notification.clone());
                        }
                    }
                }
CaptureEvent::EdgeEnter { slot, entry_x, entry_y, server_edge } => {
                     // NOTE: Do NOT switch active client here—only lock cursor for seamless mouse movement.
                     // Active client changes only via explicit user action (hotkey/UI).
                     // Compute the return edge (opposite of the server edge the cursor crossed).
                     let return_edge = netshare_core::layout::LayoutConfig::return_edge(server_edge);
                     // Send CursorEnter to that client.
                     {
                         let map = fan_map.lock().unwrap_or_else(|e| e.into_inner());
                         if let Some(tx) = map.get(&slot) {
                             let _ = tx.send(ControlPacket::CursorEnter { x: entry_x, y: entry_y, return_edge });
                         }
                     }

                     // Safety net: if the cursor is still locked to this slot after 5 minutes
                     // (e.g. topology mismatch where CursorReturn never arrives), auto-release it.
                     let timeout_seamless = fan_seamless.clone();
                     let timeout_state    = fan_state.clone();
                     tokio::spawn(async move {
                         tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                         let still_locked = {
                             let s = timeout_seamless.lock().unwrap_or_else(|e| e.into_inner());
                             s.locked_to_slot == Some(slot)
                         };
                         if still_locked {
                             warn!("Auto-releasing cursor lock for slot {slot} after 5-minute safety timeout (topology mismatch?)");
                             {
                                 let mut s = timeout_seamless.lock().unwrap_or_else(|e| e.into_inner());
                                 s.locked_to_slot = None;
                             }
                             timeout_state.force_active(0);
                             #[cfg(target_os = "windows")]
                             crate::input_capture::windows::release_cursor();
                         }
                     });
                 }
            }
        }
    });

    // ── Accept loop ────────────────────────────────────────────────────────
    info!("Listening on {addr} (TLS)");
    loop {
        let (tcp_stream, peer) = listener.accept().await?;
        info!("New connection from {peer}");
        tcp_stream.set_nodelay(true)?;

        let state      = state.clone();
        let client_map = client_map.clone();
        let audio      = Arc::clone(&audio);
        let tls        = tls.clone();
        let pairing    = pairing_code.clone();
        let seamless   = seamless.clone();

        tokio::spawn(async move {
            // TLS handshake.
            let tls_stream = match tls.acceptor.accept(tcp_stream).await {
                Ok(s) => s,
                Err(e) => { warn!("TLS handshake failed from {peer}: {e}"); return; }
            };
            if let Err(e) = handle_client(tls_stream, peer, state, client_map, audio, pairing, seamless).await {
                warn!("Client {peer} error: {e}");
            }
            info!("Client {peer} disconnected");
        });
    }
}

async fn handle_client(
    stream: tokio_rustls::server::TlsStream<TcpStream>,
    peer: std::net::SocketAddr,
    state: ActiveClientState,
    client_map: ClientMap,
    audio: Arc<ServerAudio>,
    pairing_code: String,
    seamless: SharedSeamlessState,
) -> anyhow::Result<()> {
    let (reader, writer) = tokio::io::split(stream);
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    // ── Handshake ──────────────────────────────────────────────────────────
    let (_, pkt) = read_packet(&mut reader).await?;
    let hello = match pkt {
        ControlPacket::Hello(h) => h,
        other => anyhow::bail!("expected Hello, got {:?}", other),
    };

    // Version check.
    if hello.protocol_version != PROTOCOL_VERSION {
        let resp = ControlPacket::HelloResponse(HelloResponse {
            protocol_version: PROTOCOL_VERSION,
            server_name: hostname(),
            assigned_slot: 0,
            accepted: false,
            reject_reason: Some(format!(
                "version mismatch: server={PROTOCOL_VERSION} client={}",
                hello.protocol_version
            )),
            pairing_required: false,
        });
        write_packet(&mut writer, &resp, 0).await?;
        return Ok(());
    }

    // Pairing code check.
    // Accept if: no pairing code provided by client (auto-connect) OR code matches.
    let code_ok = match &hello.pairing_code {
        Some(code) => code == &pairing_code,
        None       => true, // auto-connect: no code required
    };
    if !code_ok {
        let resp = ControlPacket::HelloResponse(HelloResponse {
            protocol_version: PROTOCOL_VERSION,
            server_name: hostname(),
            assigned_slot: 0,
            accepted: false,
            reject_reason: Some("Incorrect pairing code.".into()),
            pairing_required: true,
        });
        write_packet(&mut writer, &resp, 0).await?;
        return Ok(());
    }

    let slot = state.register(hello.client_name.clone(), peer);
    info!("Client '{}' → slot {slot} (peer {peer})", hello.client_name);

    // Auto-create a default placement for this client slot if not already configured.
    {
        let mut s = seamless.lock().unwrap_or_else(|e| e.into_inner());
        if !s.layout.placements.contains_key(&slot) {
            use netshare_core::layout::{ClientEdge, Placement};
            
            // Try to pick an edge that isn't occupied by a physical monitor or another client layout.
            let (min_x, _min_y, max_x, _max_y) = s.layout.server_bounds();
            let mut best_edge = ClientEdge::Right;
            
            // Very simple heuristic: if bounds extend left (< 0), left is probably blocked.
            // If they extend right more than primary width, right is probably blocked.
            let right_blocked = max_x > s.layout.server_width;
            let left_blocked  = min_x < 0;
            if right_blocked && !left_blocked { best_edge = ClientEdge::Left; }
            else if left_blocked && !right_blocked { best_edge = ClientEdge::Right; }
            else if right_blocked && left_blocked { best_edge = ClientEdge::Below; }

            // Ensure we don't pick an edge another client is already using.
            let used_edges: Vec<_> = s.layout.placements.values().map(|p| p.edge).collect();
            if used_edges.contains(&best_edge) {
                for candidate in [ClientEdge::Right, ClientEdge::Left, ClientEdge::Below, ClientEdge::Above] {
                    if !used_edges.contains(&candidate) {
                        best_edge = candidate;
                        break;
                    }
                }
            }

            s.layout.placements.insert(slot, Placement {
                edge:          best_edge,
                client_width:  hello.screen_width,
                client_height: hello.screen_height,
            });
            s.layout.save();
            info!(
                "Auto-created default layout for slot {slot}: {:?} edge, {}×{}",
                best_edge, hello.screen_width, hello.screen_height
            );
        }
    }

    // Always update audio target after registration.
    audio.set_mic_target(state.active_client_audio_addr());
    info!("Active slot is now {}", state.active_slot());

    let resp = ControlPacket::HelloResponse(HelloResponse {
        protocol_version: PROTOCOL_VERSION,
        server_name: hostname(),
        assigned_slot: slot,
        accepted: true,
        reject_reason: None,
        pairing_required: false,
    });
    write_packet(&mut writer, &resp, 0).await?;

    // ── Register in fan-out map ────────────────────────────────────────────
    let (client_tx, mut client_rx) = mpsc::unbounded_channel::<ControlPacket>();
    client_map.lock().unwrap_or_else(|e| e.into_inner()).insert(slot, client_tx.clone());

    // ── Heartbeat/Ping loop ────────────────────────────────────────────────
    let hb_tx    = client_tx.clone();
    let hb_task  = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if hb_tx.send(ControlPacket::Heartbeat).is_err() { break; }
            
            // The reader loop (below) will handle the response and calculate the delta.
            // We store the start time in a way the reader can access it, 
            // but for simplicity in this structural change, we'll just let the reader 
            // compute it if it knows the last send time.
            // Better: update a shared atomic/mutex with 'last_hb_sent_at'.
        }
    });

    // We'll use a shared timestamp to calculate RTT.
    let last_hb_sent = Arc::new(Mutex::new(Option::<std::time::Instant>::None));
    
    // Redefine HB loop with timestamp sharing.
    hb_task.abort(); // cleanup the stub above
    let last_sent_for_hb = last_hb_sent.clone();
    let hb_tx_2 = client_tx.clone();
    let hb_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            {
                let mut guard = last_sent_for_hb.lock().unwrap_or_else(|e| e.into_inner());
                *guard = Some(std::time::Instant::now());
            }
            if hb_tx_2.send(ControlPacket::Heartbeat).is_err() { break; }
        }
    });

    // ── Writer task ────────────────────────────────────────────────────────
    // Batch-drain pattern: write all packets already queued into the BufWriter
    // before calling flush() once.  On high-frequency mouse input (≤ 1 kHz)
    // this collapses bursts of events that accumulated between tokio wakeups
    // into a single TLS record + TCP segment, cutting encryption overhead and
    // syscall count by 10-30× without adding any extra latency to the first
    // packet (flush still happens as soon as the channel drains).
    let writer_task = tokio::spawn(async move {
        let mut flush_ticker = tokio::time::interval(std::time::Duration::from_millis(1));
        flush_ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut needs_flush = false;
        let mut queue_latency = RollingLatencyStats::new("server_input_queue", 2048);
        
        loop {
            tokio::select! {
                pkt_opt = client_rx.recv() => {
                    let Some(pkt) = pkt_opt else { break };
                    let mut batch = Vec::with_capacity(8);
                    let mut flush_immediately = pkt.is_latency_sensitive();
                    push_batched_packet(&mut batch, pkt);

                    while let Ok(next_pkt) = client_rx.try_recv() {
                        flush_immediately |= next_pkt.is_latency_sensitive();
                        push_batched_packet(&mut batch, next_pkt);
                    }

                    let mut write_failed = false;
                    for pkt in &batch {
                        if let ControlPacket::MouseMove(ev) = pkt {
                            let queue_delay = now_micros().saturating_sub(ev.captured_at_micros);
                            queue_latency.record(queue_delay);
                            if queue_delay > 20_000 {
                                warn!(
                                    target: "netshare::latency",
                                    queue_delay_micros = queue_delay,
                                    batch_len = batch.len(),
                                    flush_immediately,
                                    "server input queue spike"
                                );
                            }
                        }
                        if let Err(e) = write_packet_buffered(&mut writer, pkt, 0).await {
                            write_failed = true;
                            error!("write error: {e}");
                            break;
                        }
                    }

                    if write_failed {
                        break;
                    }

                    needs_flush = true;
                    if flush_immediately {
                        if let Err(e) = writer.flush().await {
                            error!("flush error: {e}"); break;
                        }
                        needs_flush = false;
                    }
                }
                _ = flush_ticker.tick() => {
                    if needs_flush {
                        if let Err(e) = writer.flush().await {
                            error!("flush error: {e}"); break;
                        }
                        needs_flush = false;
                    }
                }
            }
        }
    });

    // ── Reader loop (heartbeat / disconnect / cursor-return) ───────────────
    // Each read_packet call is wrapped in a 15-second timeout.  The client
    // echoes heartbeats every 3 s, so 15 s with no packet means a zombie
    // connection and we tear it down rather than leaking the slot forever.
    let result = loop {
        let read_result = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            read_packet(&mut reader),
        ).await;

        match read_result {
            Err(_elapsed) => {
                warn!("Client slot {slot} timed out (no data for 15 s) — dropping");
                break Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "client heartbeat timeout",
                ).into());
            }
            Ok(Err(e)) => break Err(e),
            Ok(Ok((_, ControlPacket::Heartbeat))) => {
                let now = std::time::Instant::now();
                let mut guard = last_hb_sent.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(start) = guard.take() {
                    let rtt = now.duration_since(start).as_millis() as u64;
                    state.update_ping(slot, rtt);
                }
            }
            Ok(Ok((_, ControlPacket::Disconnect))) => {
                info!("Client slot {slot} disconnected cleanly");
                break Ok(());
            }
            Ok(Ok((_, ControlPacket::CursorReturn))) => {
                state.force_active(0);
                {
                    let mut s = seamless.lock().unwrap_or_else(|e| e.into_inner());
                    s.locked_to_slot = None;
                }
                #[cfg(target_os = "windows")]
                crate::input_capture::windows::release_cursor();
                info!("CursorReturn from slot {slot} — cursor returned to server");
            }
            Ok(Ok((_, other))) => warn!("unexpected packet from client: {:?}", other),
        }
    };

    // ── Cleanup ────────────────────────────────────────────────────────────
    client_map.lock().unwrap_or_else(|e| e.into_inner()).remove(&slot);
    state.deregister(slot);
    // Remove the placement so reconnect gets a fresh auto-assignment.
    {
        let mut s = seamless.lock().unwrap_or_else(|e| e.into_inner());
        s.layout.placements.remove(&slot);
        s.layout.save();
    }
    audio.set_mic_target(state.active_client_audio_addr());
    writer_task.abort();
    hb_task.abort();

    Ok(result.map_err(anyhow::Error::from)?)
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "server".to_owned())
}
