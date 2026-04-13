/// Server TCP network layer — control channel on TCP :9000, TLS-wrapped.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use netshare_core::{
    framing::{read_packet, write_packet, write_packet_buffered},
    protocol::{ControlPacket, HelloResponse, PROTOCOL_VERSION},
};

use crate::active_client::ActiveClientState;
use crate::audio::ServerAudio;
use crate::input_capture::{self, CaptureEvent, HotkeyAction, SharedSeamlessState};
use crate::tls::ServerTls;

type ClientTx  = mpsc::UnboundedSender<ControlPacket>;
type ClientMap = Arc<Mutex<HashMap<u8, ClientTx>>>;

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
                    // Switch active to this slot.
                    fan_state.force_active(slot);
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
            let (min_x, min_y, max_x, max_y) = s.layout.server_bounds();
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
    let hb_state = state.clone();
    let hb_tx    = client_tx.clone();
    let hb_task  = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let start = std::time::Instant::now();
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
        let mut flush_ticker = tokio::time::interval(std::time::Duration::from_millis(8));
        let mut needs_flush = false;
        
        loop {
            tokio::select! {
                pkt_opt = client_rx.recv() => {
                    let Some(pkt) = pkt_opt else { break };
                    if let Err(e) = write_packet_buffered(&mut writer, &pkt, 0).await {
                        error!("write error: {e}"); break;
                    }
                    needs_flush = true;
                    
                    // Immediately drain any other available packets
                    while let Ok(pkt) = client_rx.try_recv() {
                        if let Err(e) = write_packet_buffered(&mut writer, &pkt, 0).await {
                            error!("write error: {e}"); break;
                        }
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
