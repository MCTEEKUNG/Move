/// Server TCP network layer — control channel on TCP :9000, TLS-wrapped.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::BufWriter;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use netshare_core::{
    framing::{read_packet, write_packet},
    protocol::{ControlPacket, HelloResponse, PROTOCOL_VERSION},
};

use crate::active_client::ActiveClientState;
use crate::audio::ServerAudio;
use crate::input_capture::{self, CaptureEvent, HotkeyAction};
use crate::tls::ServerTls;

type ClientTx  = mpsc::UnboundedSender<ControlPacket>;
type ClientMap = Arc<Mutex<HashMap<u8, ClientTx>>>;

pub async fn run_server(
    addr: &str,
    audio: ServerAudio,
    state: ActiveClientState,
    tls: ServerTls,
    pairing_code: String,
) -> anyhow::Result<()> {
    let listener   = TcpListener::bind(addr).await?;
    let client_map: ClientMap = Arc::new(Mutex::new(HashMap::new()));
    let audio      = Arc::new(audio);

    // ── Start input capture thread ─────────────────────────────────────────
    let (capture_tx, mut capture_rx) = mpsc::unbounded_channel::<CaptureEvent>();
    std::thread::spawn({
        let tx = capture_tx.clone();
        move || input_capture::start_capture(tx)
    });

    // ── Fan-out loop ───────────────────────────────────────────────────────
    let fan_state  = state.clone();
    let fan_map    = client_map.clone();
    let fan_audio  = Arc::clone(&audio);
    tokio::spawn(async move {
        while let Some(evt) = capture_rx.recv().await {
            match evt {
                CaptureEvent::InputPacket(pkt) => {
                    let active = fan_state.active_slot();
                    if active == 0 { continue; }
                    let map = fan_map.lock().unwrap();
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
                    };
                    if let Some(change) = change {
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

        tokio::spawn(async move {
            // TLS handshake.
            let tls_stream = match tls.acceptor.accept(tcp_stream).await {
                Ok(s) => s,
                Err(e) => { warn!("TLS handshake failed from {peer}: {e}"); return; }
            };
            if let Err(e) = handle_client(tls_stream, peer, state, client_map, audio, pairing).await {
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
    let code_ok = match &hello.pairing_code {
        Some(code) => code == &pairing_code,
        None       => false,
    };
    if !code_ok {
        let resp = ControlPacket::HelloResponse(HelloResponse {
            protocol_version: PROTOCOL_VERSION,
            server_name: hostname(),
            assigned_slot: 0,
            accepted: false,
            reject_reason: Some("Incorrect or missing pairing code.".into()),
            pairing_required: true,
        });
        write_packet(&mut writer, &resp, 0).await?;
        return Ok(());
    }

    let slot = state.register(hello.client_name.clone(), peer);
    info!("Client '{}' → slot {slot} (peer {peer})", hello.client_name);

    if state.active_slot() == slot {
        audio.set_mic_target(state.active_client_audio_addr());
    }

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
    audio.set_mic_target(state.active_client_audio_addr());
    writer_task.abort();

    Ok(result.map_err(anyhow::Error::from)?)
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "server".to_owned())
}
