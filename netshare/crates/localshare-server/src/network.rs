/// Server TCP network layer.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};

use tokio::io::BufWriter;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use localshare_core::{
    framing::{read_packet, write_packet},
    protocol::{ControlPacket, HelloResponse, PROTOCOL_VERSION},
};

use crate::active_client::ActiveClientState;
use crate::audio::ServerAudio;
use crate::input_capture::{self, CaptureEvent, HotkeyAction};

type ClientTx  = mpsc::UnboundedSender<ControlPacket>;
type ClientMap = Arc<Mutex<HashMap<u8, ClientTx>>>;

/// Run the server with an externally-provided state, switch channel, and
/// share-input flag. Used by the daemon.
pub async fn run_server(
    addr:        &str,
    audio:       ServerAudio,
    state:       ActiveClientState,
    mut switch_rx: mpsc::UnboundedReceiver<u8>,
    share_input: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let listener   = TcpListener::bind(addr).await?;
    let client_map: ClientMap = Arc::new(Mutex::new(HashMap::new()));
    let audio      = Arc::new(audio);

    // ── Start input capture thread ─────────────────────────────────────────
    let (capture_tx, mut capture_rx) = mpsc::unbounded_channel::<CaptureEvent>();
    let share_input_cap = Arc::clone(&share_input);
    // `suppress` controls whether the OS-level hook swallows local input.
    // Only true when input is being forwarded to a remote client.
    let suppress = Arc::new(AtomicBool::new(false));
    let suppress_hook = Arc::clone(&suppress);
    std::thread::spawn({
        let tx = capture_tx.clone();
        move || input_capture::start_capture(tx, suppress_hook)
    });

    // ── Keep `suppress` in sync with (share_input && active_slot != 0) ────
    // Polled every 100ms — active_slot can change from IPC or hotkey.
    {
        let state_s    = state.clone();
        let share_s    = Arc::clone(&share_input);
        let suppress_s = Arc::clone(&suppress);
        tokio::spawn(async move {
            let mut last = false;
            loop {
                let want = share_s.load(Ordering::Relaxed) && state_s.active_slot() != 0;
                if want != last {
                    suppress_s.store(want, Ordering::Relaxed);
                    last = want;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        });
    }

    // ── Fan-out loop (capture events) ─────────────────────────────────────
    let fan_state  = state.clone();
    let fan_map    = client_map.clone();
    let fan_audio  = Arc::clone(&audio);
    tokio::spawn(async move {
        while let Some(evt) = capture_rx.recv().await {
            match evt {
                CaptureEvent::InputPacket(pkt) => {
                    // Respect the share_input flag.
                    if !share_input_cap.load(Ordering::Relaxed) { continue; }
                    let active = fan_state.active_slot();
                    if active == 0 { continue; }
                    if let Some(tx) = fan_map.lock().unwrap().get(&active) {
                        let _ = tx.send(pkt);
                    }
                }
                CaptureEvent::Hotkey(action) => {
                    let change = match action {
                        HotkeyAction::SwitchToSlot(s) => fan_state.switch_to(s),
                        HotkeyAction::Cycle           => fan_state.cycle(),
                    };
                    if let Some(change) = change {
                        // Update mic audio target.
                        fan_audio.set_mic_target(fan_state.active_client_audio_addr());

                        let notification = ControlPacket::ActiveClientChange(change);
                        let map = fan_map.lock().unwrap();
                        for tx in map.values() {
                            let _ = tx.send(notification.clone());
                        }
                    }
                }
            }
        }
    });

    // ── External switch channel (from IPC / edge detection) ───────────────
    {
        let sw_state  = state.clone();
        let sw_map    = client_map.clone();
        let sw_audio  = Arc::clone(&audio);
        tokio::spawn(async move {
            while let Some(slot) = switch_rx.recv().await {
                if let Some(change) = sw_state.switch_to(slot) {
                    sw_audio.set_mic_target(sw_state.active_client_audio_addr());
                    let notification = ControlPacket::ActiveClientChange(change);
                    let map = sw_map.lock().unwrap();
                    for tx in map.values() {
                        let _ = tx.send(notification.clone());
                    }
                }
            }
        });
    }

    // ── Accept loop ────────────────────────────────────────────────────────
    info!("Listening on {addr}");
    loop {
        let (stream, peer) = listener.accept().await?;
        info!("New connection from {peer}");
        stream.set_nodelay(true)?;

        let state      = state.clone();
        let client_map = client_map.clone();
        let audio      = Arc::clone(&audio);

        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, peer, state, client_map, audio).await {
                warn!("Client {peer} error: {e}");
            }
            info!("Client {peer} disconnected");
        });
    }
}

async fn handle_client(
    stream: TcpStream,
    peer: std::net::SocketAddr,
    state: ActiveClientState,
    client_map: ClientMap,
    audio: Arc<ServerAudio>,
) -> anyhow::Result<()> {
    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    // ── Handshake ──────────────────────────────────────────────────────────
    let (_, pkt) = read_packet(&mut reader).await?;
    let hello = match pkt {
        ControlPacket::Hello(h) => h,
        other => anyhow::bail!("expected Hello, got {:?}", other),
    };

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
        });
        write_packet(&mut writer, &resp, 0).await?;
        anyhow::bail!(
            "Hello rejected: version mismatch (server={PROTOCOL_VERSION} \
             client={} from='{}')",
            hello.protocol_version, hello.client_name
        );
    }
    info!(
        "Hello from '{}' (version {}) accepted",
        hello.client_name, hello.protocol_version
    );

    let slot = state.register(hello.client_name.clone(), peer);
    info!("Client '{}' → slot {slot} (peer {peer})", hello.client_name);

    // If this is the first/active client, point mic audio at it.
    if state.active_slot() == slot {
        audio.set_mic_target(state.active_client_audio_addr());
    }

    let resp = ControlPacket::HelloResponse(HelloResponse {
        protocol_version: PROTOCOL_VERSION,
        server_name: hostname(),
        assigned_slot: slot,
        accepted: true,
        reject_reason: None,
    });
    write_packet(&mut writer, &resp, 0).await?;

    // ── Register in fan-out map ────────────────────────────────────────────
    let (client_tx, mut client_rx) = mpsc::unbounded_channel::<ControlPacket>();
    client_map.lock().unwrap().insert(slot, client_tx);

    // ── Writer task ────────────────────────────────────────────────────────
    let writer_task = tokio::spawn(async move {
        while let Some(pkt) = client_rx.recv().await {
            if let Err(e) = write_packet(&mut writer, &pkt, 0).await {
                error!("write error: {e}");
                break;
            }
        }
    });

    // ── Reader loop (heartbeat / disconnect) ───────────────────────────────
    let result = loop {
        match read_packet(&mut reader).await {
            Ok((_, ControlPacket::Heartbeat))  => {}
            Ok((_, ControlPacket::Disconnect)) => {
                info!("Client slot {slot} disconnected cleanly");
                break Ok(());
            }
            Ok((_, other)) => warn!("unexpected packet from client: {:?}", other),
            Err(e)         => break Err(e),
        }
    };

    // ── Cleanup ────────────────────────────────────────────────────────────
    client_map.lock().unwrap().remove(&slot);
    state.deregister(slot);
    // If the removed client was active, point mic at new active (or None).
    audio.set_mic_target(state.active_client_audio_addr());
    writer_task.abort();

    Ok(result?)
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "server".to_owned())
}
