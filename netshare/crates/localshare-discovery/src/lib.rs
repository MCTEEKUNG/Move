//! Zero-config LAN peer discovery via mDNS / DNS-SD.
//!
//! Each LocalShare instance announces itself as `_localshare._tcp.local`
//! and listens for other instances on the same network.
//!
//! Usage:
//! ```no_run
//! let disco = Discovery::new("MyPC", 9000).unwrap();
//! disco.start();                           // non-blocking
//! let peers = disco.peers();               // snapshot of known peers
//! let mut rx = disco.subscribe();          // channel for peer events
//! ```

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::broadcast;
use tracing::{debug, info};

const SERVICE_TYPE: &str = "_localshare._tcp.local.";

/// A discovered peer on the LAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// Human-readable machine name (e.g. "DESKTOP-ABC").
    pub name:    String,
    /// Primary IPv4 address.
    pub addr:    Ipv4Addr,
    /// TCP control port (default 9000).
    pub port:    u16,
    /// OS string from the TXT record ("windows" | "linux" | "macos").
    pub os:      String,
    /// App version from the TXT record.
    pub version: String,
}

/// Event emitted when the peer list changes.
#[derive(Debug, Clone)]
pub enum PeerEvent {
    Added(Peer),
    Removed(String), // name
}

/// Manages mDNS announcement + browsing.
pub struct Discovery {
    daemon:    ServiceDaemon,
    peers:     Arc<Mutex<HashMap<String, Peer>>>,
    event_tx:  broadcast::Sender<PeerEvent>,
    host_name: String,
    port:      u16,
}

impl Discovery {
    /// Create a new Discovery instance.
    /// `host_name` — this machine's display name.
    /// `port`      — the TCP port this instance listens on.
    pub fn new(host_name: &str, port: u16) -> anyhow::Result<Self> {
        let daemon   = ServiceDaemon::new()?;
        let (tx, _)  = broadcast::channel(64);
        Ok(Self {
            daemon,
            peers:     Arc::new(Mutex::new(HashMap::new())),
            event_tx:  tx,
            host_name: host_name.to_owned(),
            port,
        })
    }

    /// Announce this instance on the LAN and start browsing for peers.
    /// Returns immediately — runs in background threads.
    pub fn start(&self) -> anyhow::Result<()> {
        self.announce()?;
        self.browse();
        Ok(())
    }

    /// Snapshot of currently known peers.
    pub fn peers(&self) -> Vec<Peer> {
        self.peers.lock().unwrap().values().cloned().collect()
    }

    /// Subscribe to peer-change events.
    pub fn subscribe(&self) -> broadcast::Receiver<PeerEvent> {
        self.event_tx.subscribe()
    }

    // ── Private ──────────────────────────────────────────────────────────

    fn announce(&self) -> anyhow::Result<()> {
        let os = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else {
            "macos"
        };

        let props = [
            ("os",      os),
            ("version", env!("CARGO_PKG_VERSION")),
        ];

        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &self.host_name,
            &format!("{}.local.", self.host_name),
            "",          // let mdns-sd pick the local IP
            self.port,
            &props[..],
        )?;

        self.daemon.register(info)?;
        info!("mDNS: announced '{}' on port {}", self.host_name, self.port);
        Ok(())
    }

    fn browse(&self) {
        let receiver = self.daemon.browse(SERVICE_TYPE)
            .expect("mDNS browse failed");

        let peers    = Arc::clone(&self.peers);
        let event_tx = self.event_tx.clone();
        let my_name  = self.host_name.clone();

        std::thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let name = info.get_fullname()
                            .trim_end_matches(&format!(".{}", SERVICE_TYPE))
                            .trim_end_matches('.')
                            .to_owned();

                        // Don't add ourselves.
                        if name == my_name { continue; }

                        let addr = info
                            .get_addresses()
                            .iter()
                            .find_map(|a| if let IpAddr::V4(v4) = a { Some(*v4) } else { None });

                        let Some(addr) = addr else { continue };

                        let os      = info.get_property_val_str("os")     .unwrap_or("unknown").to_owned();
                        let version = info.get_property_val_str("version").unwrap_or("?").to_owned();

                        let peer = Peer { name: name.clone(), addr, port: info.get_port(), os, version };
                        info!("mDNS: peer found → {} @ {}", name, addr);
                        debug!("  os={} version={}", peer.os, peer.version);

                        peers.lock().unwrap().insert(name, peer.clone());
                        let _ = event_tx.send(PeerEvent::Added(peer));
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        let name = fullname
                            .trim_end_matches(&format!(".{}", SERVICE_TYPE))
                            .trim_end_matches('.')
                            .to_owned();
                        info!("mDNS: peer gone → {}", name);
                        peers.lock().unwrap().remove(&name);
                        let _ = event_tx.send(PeerEvent::Removed(name));
                    }
                    _ => {}
                }
            }
        });
    }
}
