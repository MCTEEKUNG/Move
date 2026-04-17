//! LocalShare Daemon — runs 24/7 in the background.
//!
//! Responsibilities:
//!  • mDNS peer discovery (auto-finds other LocalShare instances on LAN)
//!  • Embeds server subsystems (audio, file, input capture, TCP server)
//!  • Named-pipe / Unix-socket IPC for the GUI
//!  • Screen-edge mouse switching
//!  • Registers auto-start at login

mod autostart;
mod edge;
mod ipc;

use std::sync::{Arc, atomic::{AtomicBool}};
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber::EnvFilter;

use localshare_discovery::Discovery;
use localshare_server::{
    active_client::ActiveClientState,
    audio::ServerAudio,
    network,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Logging ───────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("localshare=debug".parse()?)
        )
        .init();

    info!("LocalShare daemon starting…");

    // ── Auto-start registration (idempotent) ──────────────────────────────
    if let Err(e) = autostart::register() {
        tracing::warn!("Auto-start registration failed: {e}");
    }

    // ── Shared state ──────────────────────────────────────────────────────
    let state       = ActiveClientState::default();
    let share_input = Arc::new(AtomicBool::new(true));
    let (switch_tx, switch_rx) = mpsc::unbounded_channel::<u8>();

    // ── mDNS discovery ────────────────────────────────────────────────────
    let hostname = hostname();
    let disco    = Discovery::new(&hostname, 9000)?;
    disco.start()?;
    info!("mDNS discovery active — hostname: {hostname}");

    // Auto-connect: when a peer is discovered, log it (Phase 2 will dial out)
    let mut peer_rx = disco.subscribe();
    let state_clone = state.clone();
    tokio::spawn(async move {
        while let Ok(evt) = peer_rx.recv().await {
            match evt {
                localshare_discovery::PeerEvent::Added(peer) => {
                    info!("Peer discovered: {} @ {}:{} ({})", peer.name, peer.addr, peer.port, peer.os);
                    // Phase 2: dial out and register as connected peer
                }
                localshare_discovery::PeerEvent::Removed(name) => {
                    info!("Peer gone: {}", name);
                    state_clone.deregister_by_name(&name);
                }
            }
        }
    });

    // ── Audio subsystem (inside tokio context) ────────────────────────────
    let audio = ServerAudio::start_or_stub();

    // ── File transfer ─────────────────────────────────────────────────────
    let (_file_tx, file_rx) = mpsc::unbounded_channel();
    if let Err(e) = localshare_server::file::start(file_rx) {
        tracing::warn!("File transfer start failed: {e}");
    }

    // ── IPC server for GUI ────────────────────────────────────────────────
    let ipc_state  = state.clone();
    let ipc_switch = switch_tx.clone();
    let ipc_input  = Arc::clone(&share_input);
    tokio::spawn(async move {
        if let Err(e) = ipc::serve(ipc_state, ipc_switch, ipc_input).await {
            tracing::error!("IPC server error: {e}");
        }
    });

    // ── Screen-edge switching ─────────────────────────────────────────────
    let edge_switch = switch_tx.clone();
    let edge_state  = state.clone();
    std::thread::spawn(move || {
        edge::run(edge_state, edge_switch);
    });

    // ── TCP control server (blocks until shutdown) ────────────────────────
    info!("TCP server listening on 0.0.0.0:9000");
    network::run_server("0.0.0.0:9000", audio, state, switch_rx, share_input).await?;

    Ok(())
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localshare".to_owned())
}
