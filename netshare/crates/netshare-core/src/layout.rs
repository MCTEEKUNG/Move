use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Which edge of the server screen connects to a client screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientEdge {
    Left,
    Right,
    Above,
    Below,
}

/// How a client screen is placed relative to the server screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Placement {
    pub edge: ClientEdge,
    /// Client screen width in pixels (for entry-position scaling).
    pub client_width: i32,
    /// Client screen height in pixels.
    pub client_height: i32,
}

/// One physical monitor on the server machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorInfo {
    /// Left edge in virtual-screen coordinates (negative for non-primary).
    pub x: i32,
    /// Top edge in virtual-screen coordinates.
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// true = this is the Windows primary monitor.
    pub is_primary: bool,
}

/// Full desktop layout, stored and managed on the server.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LayoutConfig {
    pub server_width:  i32,
    pub server_height: i32,
    /// All physical monitors on the server (virtual-screen coords). Not persisted across runs.
    #[serde(default)]
    pub server_monitors: Vec<MonitorInfo>,
    /// slot → placement
    pub placements: HashMap<u8, Placement>,
}

impl LayoutConfig {
    const PATH: &'static str = "netshare_layout.json";

    pub fn load() -> Self {
        let p = Self::config_path();
        if let Ok(data) = std::fs::read_to_string(&p) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                return cfg;
            }
        }
        Self::default()
    }

    /// Returns the virtual bounding box of all server monitors (min_x, min_y, max_x, max_y).
    /// If no monitors are defined, defaults to (0, 0, server_width, server_height).
    pub fn server_bounds(&self) -> (i32, i32, i32, i32) {
        if self.server_monitors.is_empty() {
            return (0, 0, self.server_width.max(1), self.server_height.max(1));
        }
        let min_x = self.server_monitors.iter().map(|m| m.x).min().unwrap_or(0);
        let min_y = self.server_monitors.iter().map(|m| m.y).min().unwrap_or(0);
        let max_x = self.server_monitors.iter().map(|m| m.x + m.width).max().unwrap_or(self.server_width);
        let max_y = self.server_monitors.iter().map(|m| m.y + m.height).max().unwrap_or(self.server_height);
        (min_x, min_y, max_x, max_y)
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::config_path(), json);
        }
    }

    fn config_path() -> std::path::PathBuf {
        // Save next to the exe.
        if let Ok(mut p) = std::env::current_exe() {
            p.set_file_name(Self::PATH);
            return p;
        }
        std::path::PathBuf::from(Self::PATH)
    }

    /// Calculate the entry position on the client screen when the cursor
    /// exits the server screen at `(cursor_x, cursor_y)` toward `slot`.
    pub fn entry_pos(&self, slot: u8, cursor_x: i32, cursor_y: i32) -> Option<(i32, i32)> {
        let p = self.placements.get(&slot)?;
        let (min_x, min_y, max_x, max_y) = self.server_bounds();
        let b_width  = (max_x - min_x).max(1) as f64;
        let b_height = (max_y - min_y).max(1) as f64;
        
        let cw = p.client_width.max(1) as f64;
        let ch = p.client_height.max(1) as f64;

        let (x, y) = match p.edge {
            ClientEdge::Right => {
                let rel_y = (cursor_y - min_y) as f64 / b_height;
                (0_i32, (rel_y * ch).clamp(0.0, ch - 1.0) as i32)
            }
            ClientEdge::Left => {
                let rel_y = (cursor_y - min_y) as f64 / b_height;
                ((cw as i32) - 1, (rel_y * ch).clamp(0.0, ch - 1.0) as i32)
            }
            ClientEdge::Below => {
                let rel_x = (cursor_x - min_x) as f64 / b_width;
                ((rel_x * cw).clamp(0.0, cw - 1.0) as i32, 0_i32)
            }
            ClientEdge::Above => {
                let rel_x = (cursor_x - min_x) as f64 / b_width;
                ((rel_x * cw).clamp(0.0, cw - 1.0) as i32, (ch as i32) - 1)
            }
        };
        Some((x, y))
    }

    /// Which edge of the CLIENT screen leads back to the server,
    /// given that the client entered from the server's `edge` side.
    pub fn return_edge(edge: ClientEdge) -> ClientEdge {
        match edge {
            ClientEdge::Right  => ClientEdge::Left,
            ClientEdge::Left   => ClientEdge::Right,
            ClientEdge::Below  => ClientEdge::Above,
            ClientEdge::Above  => ClientEdge::Below,
        }
    }
}
