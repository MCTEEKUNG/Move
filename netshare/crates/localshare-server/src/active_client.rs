use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use localshare_core::protocol::ActiveClientChange;

#[derive(Debug, Clone)]
struct ClientInfo {
    slot: u8,
    name: String,
    /// TCP peer address — same IP used for UDP audio (port differs).
    peer: SocketAddr,
}

/// Public snapshot of a connected client, returned by `ActiveClientState::snapshot()`.
#[derive(Debug, Clone)]
pub struct ClientSnapshot {
    pub slot:      u8,
    pub name:      String,
    pub is_active: bool,
}

#[derive(Debug, Default)]
struct Inner {
    active_slot: u8,
    clients: Vec<ClientInfo>,
    next_slot: u8,
}

#[derive(Clone, Debug)]
pub struct ActiveClientState(Arc<Mutex<Inner>>);

impl Default for ActiveClientState {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Inner {
            active_slot: 0,
            clients: Vec::new(),
            next_slot: 1,
        })))
    }
}

impl ActiveClientState {
    /// Register a new client. Returns the assigned slot (1-9).
    pub fn register(&self, name: String, peer: SocketAddr) -> u8 {
        let mut g = self.0.lock().unwrap();
        let slot = g.next_slot;
        g.next_slot = (slot % 9) + 1;
        g.clients.push(ClientInfo { slot, name, peer });
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

    /// Returns a snapshot of all connected clients.
    pub fn snapshot(&self) -> Vec<ClientSnapshot> {
        let g = self.0.lock().unwrap();
        g.clients.iter().map(|c| ClientSnapshot {
            slot:      c.slot,
            name:      c.name.clone(),
            is_active: c.slot == g.active_slot,
        }).collect()
    }

    /// Remove a client by name (used by mDNS "peer gone" events).
    pub fn deregister_by_name(&self, name: &str) {
        let mut g = self.0.lock().unwrap();
        if let Some(pos) = g.clients.iter().position(|c| c.name == name) {
            let slot = g.clients[pos].slot;
            g.clients.remove(pos);
            if g.active_slot == slot {
                g.active_slot = g.clients.first().map(|c| c.slot).unwrap_or(0);
            }
        }
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
}
