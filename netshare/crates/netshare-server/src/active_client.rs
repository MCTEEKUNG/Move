use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use netshare_core::protocol::ActiveClientChange;

#[derive(Debug, Clone)]
struct ClientInfo {
    slot: u8,
    name: String,
    /// TCP peer address — same IP used for UDP audio (port differs).
    peer: SocketAddr,
    /// Last measured round-trip time in milliseconds.
    last_ping_ms: u64,
}

#[derive(Debug)]
struct Inner {
    active_slot: u8,
    clients: Vec<ClientInfo>,
    next_slot: u8,
    broadcast_mode: bool,
}

impl Default for Inner {
    fn default() -> Self {
        Self { active_slot: 0, clients: Vec::new(), next_slot: 1, broadcast_mode: false }
    }
}

#[derive(Clone, Debug)]
pub struct ActiveClientState(Arc<Mutex<Inner>>);

impl Default for ActiveClientState {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Inner::default())))
    }
}

impl ActiveClientState {
    /// Register a new client. Returns the assigned slot (1-9).
    pub fn register(&self, name: String, peer: SocketAddr) -> u8 {
        let mut g = self.0.lock().unwrap();
        let slot = g.next_slot;
        g.next_slot = (slot % 9) + 1;
        g.clients.push(ClientInfo { slot, name, peer, last_ping_ms: 0 });
        // First client becomes active automatically.
        if g.active_slot == 0 {
            g.active_slot = slot;
        }
        slot
    }

    pub fn deregister(&self, slot: u8) {
        let mut g = self.0.lock().unwrap();
        g.clients.retain(|c| c.slot != slot);
        if g.active_slot == slot {
            g.active_slot = g.clients.first().map(|c| c.slot).unwrap_or(0);
        }
    }

    /// Switch to a specific slot. Returns the change notification, or `None`
    /// if the slot is not connected.
    pub fn switch_to(&self, slot: u8) -> Option<ActiveClientChange> {
        let mut g = self.0.lock().unwrap();
        let name = g.clients.iter().find(|c| c.slot == slot)?.name.clone();
        g.active_slot = slot;
        Some(ActiveClientChange { active_slot: slot, active_name: name })
    }

    /// Cycle to the next connected client.
    pub fn cycle(&self) -> Option<ActiveClientChange> {
        let mut g = self.0.lock().unwrap();
        if g.clients.is_empty() { return None; }
        let pos = g.clients.iter().position(|c| c.slot == g.active_slot);
        let next = match pos {
            Some(i) => (i + 1) % g.clients.len(),
            None    => 0,
        };
        let info = g.clients[next].clone();
        g.active_slot = info.slot;
        Some(ActiveClientChange { active_slot: info.slot, active_name: info.name })
    }

    pub fn active_slot(&self) -> u8 {
        self.0.lock().unwrap().active_slot
    }

    /// Returns the TCP peer IP of the currently active client, combined with
    /// the audio port (9002) — used to route mic UDP packets.
    pub fn active_client_audio_addr(&self) -> Option<SocketAddr> {
        let g = self.0.lock().unwrap();
        g.clients
            .iter()
            .find(|c| c.slot == g.active_slot)
            .map(|c| SocketAddr::new(c.peer.ip(), 9002))
    }

    /// Returns the IP of the currently active client — used for file sends.
    pub fn active_client_ip(&self) -> Option<std::net::IpAddr> {
        let g = self.0.lock().unwrap();
        g.clients
            .iter()
            .find(|c| c.slot == g.active_slot)
            .map(|c| c.peer.ip())
    }

    /// Snapshot of connected clients for GUI display: `(slot, name)`.
    pub fn clients_snapshot(&self) -> Vec<(u8, String)> {
        let g = self.0.lock().unwrap();
        g.clients.iter().map(|c| (c.slot, c.name.clone())).collect()
    }

    /// Snapshot of all client pings: `(slot, ping_ms)`.
    pub fn pings_snapshot(&self) -> std::collections::HashMap<u8, u64> {
        let g = self.0.lock().unwrap();
        g.clients.iter().map(|c| (c.slot, c.last_ping_ms)).collect()
    }

    /// Update the ping value for a specific slot.
    pub fn update_ping(&self, slot: u8, ping_ms: u64) {
        let mut g = self.0.lock().unwrap();
        if let Some(c) = g.clients.iter_mut().find(|c| c.slot == slot) {
            c.last_ping_ms = ping_ms;
        }
    }

    pub fn broadcast_mode(&self) -> bool {
        self.0.lock().unwrap().broadcast_mode
    }

    pub fn set_broadcast_mode(&self, val: bool) {
        self.0.lock().unwrap().broadcast_mode = val;
    }

    /// Force-set active slot without validation (used for seamless cursor events).
    /// slot=0 means server is active.
    pub fn force_active(&self, slot: u8) {
        self.0.lock().unwrap().active_slot = slot;
    }
}
