pub mod active_client;
pub mod audio;
pub mod file;
pub mod input_capture;
pub mod network;
pub mod tls;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use active_client::ActiveClientState;
use audio::ServerAudio;
use input_capture::{SeamlessState, SharedSeamlessState};
pub use tls::ServerTls;
use netshare_core::layout::LayoutConfig;

/// A pending file-transfer accept request from the GUI's perspective.
pub struct PendingFileRequest {
    pub id:        u32,
    pub filename:  String,
    pub size:      u64,
    pub from_peer: String,
    /// Send `true` to accept, `false` to reject.
    pub response:  tokio::sync::oneshot::Sender<bool>,
}

/// Shared list of incoming file-transfer requests waiting for user approval.
pub type PendingRequests = Arc<Mutex<Vec<PendingFileRequest>>>;

/// Live state the GUI reads.
pub struct ServerHandle {
    pub active:   ActiveClientState,
    pub pairing_code: String,
    pub pending_file_requests: PendingRequests,
    file_send_tx: tokio::sync::mpsc::UnboundedSender<PathBuf>,
    seamless: SharedSeamlessState,
}

impl ServerHandle {
    pub fn clients(&self) -> Vec<(u8, String)> {
        self.active.clients_snapshot()
    }
    pub fn active_slot(&self) -> u8 {
        self.active.active_slot()
    }
    pub fn pings(&self) -> std::collections::HashMap<u8, u64> {
        self.active.pings_snapshot()
    }
    pub fn broadcast_mode(&self) -> bool {
        self.active.broadcast_mode()
    }
    pub fn set_broadcast_mode(&self, val: bool) {
        self.active.set_broadcast_mode(val);
    }
    /// Queue a file to be sent to the currently active client.
    pub fn send_file(&self, path: PathBuf) {
        let _ = self.file_send_tx.send(path);
    }
    /// Update the layout config used by the seamless cursor engine.
    pub fn set_layout(&self, layout: LayoutConfig) {
        let mut s = self.seamless.lock().unwrap();
        s.layout = layout;
    }
    /// Get a snapshot of the current layout config.
    pub fn layout(&self) -> LayoutConfig {
        self.seamless.lock().unwrap().layout.clone()
    }
    /// Get the server's hostname.
    pub fn server_name(&self) -> String {
        std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "netshare-node".to_owned())
    }
}

/// Try to add Windows Firewall inbound rules for all NetShare ports.
/// Requires the process to be running as Administrator; silently skips if not.
#[cfg(target_os = "windows")]
fn open_firewall_ports() {
    let rules: &[(&str, &str, &str)] = &[
        ("NetShare-TCP", "TCP", "9000,9003,9004"),
        ("NetShare-UDP", "UDP", "9001,9002"),
    ];
    for (name, proto, ports) in rules {
        // Delete stale rule first (ignore errors), then re-add.
        let _ = std::process::Command::new("netsh")
            .args(["advfirewall", "firewall", "delete", "rule",
                   &format!("name={name}")])
            .output();
        let result = std::process::Command::new("netsh")
            .args(["advfirewall", "firewall", "add", "rule",
                   &format!("name={name}"),
                   "dir=in", "action=allow",
                   &format!("protocol={proto}"),
                   &format!("localport={ports}")])
            .output();
        match result {
            Ok(o) if o.status.success() =>
                tracing::info!("Firewall rule added: {name} ({proto} {ports})"),
            _ =>
                tracing::warn!("Could not add firewall rule {name} — run as Administrator once to fix"),
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn open_firewall_ports() {}

// ── Server screen resolution helpers ────────────────────────────────────────

#[cfg(target_os = "windows")]
fn enumerate_server_monitors() -> Vec<netshare_core::layout::MonitorInfo> {
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HMONITOR, HDC, MONITORINFOEXW,
    };
    use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
    use netshare_core::layout::MonitorInfo;

    let mut monitors: Vec<MonitorInfo> = Vec::new();

    unsafe extern "system" fn monitor_cb(
        hmon: HMONITOR, _hdc: HDC, _rect: *mut RECT, lparam: LPARAM,
    ) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<netshare_core::layout::MonitorInfo>);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmon, &mut info.monitorInfo).as_bool() {
            let r = info.monitorInfo.rcMonitor;
            let primary = info.monitorInfo.dwFlags & 1 != 0; // MONITORINFOF_PRIMARY = 1
            list.push(netshare_core::layout::MonitorInfo {
                x: r.left, y: r.top,
                width: r.right - r.left,
                height: r.bottom - r.top,
                is_primary: primary,
            });
        }
        BOOL(1)
    }

    unsafe {
        EnumDisplayMonitors(
            HDC::default(), None,
            Some(monitor_cb),
            LPARAM(&mut monitors as *mut _ as isize),
        );
    }

    if monitors.is_empty() {
        // Fallback: at least add a 1920×1080 primary.
        monitors.push(netshare_core::layout::MonitorInfo {
            x: 0, y: 0, width: 1920, height: 1080, is_primary: true,
        });
    }
    monitors
}

#[cfg(not(target_os = "windows"))]
fn enumerate_server_monitors() -> Vec<netshare_core::layout::MonitorInfo> {
    vec![netshare_core::layout::MonitorInfo {
        x: 0, y: 0, width: 1920, height: 1080, is_primary: true,
    }]
}

/// Start the server subsystems and return a handle the GUI can read from.
pub fn start(bind_addr: &str) -> anyhow::Result<ServerHandle> {
    // Open Windows Firewall ports (no-op if not running as Administrator).
    open_firewall_ports();

    let active  = ActiveClientState::default();
    let audio   = ServerAudio::start().unwrap_or_else(|e| {
        tracing::warn!("Audio subsystem unavailable (audio disabled): {e}");
        ServerAudio { mic_target: Arc::new(Mutex::new(None)) }
    });
    let tls     = ServerTls::generate()?;
    let pending: PendingRequests = Arc::new(Mutex::new(Vec::new()));

    // Seamless cursor sharing state.
    let mut seamless_inner = SeamlessState::default();
    // Load saved layout config (placements only — monitors are re-enumerated live).
    seamless_inner.layout = LayoutConfig::load();
    // Always enumerate monitors fresh on start.
    let monitors = enumerate_server_monitors();
    tracing::info!("Detected {} server monitor(s):", monitors.len());
    for (i, m) in monitors.iter().enumerate() {
        tracing::info!(
            "  Monitor {}: {}×{} at ({},{}){}",
            i + 1, m.width, m.height, m.x, m.y,
            if m.is_primary { " [PRIMARY]" } else { "" },
        );
    }
    // Use primary monitor for the "server_width/height" used by cursor math.
    if let Some(primary) = monitors.iter().find(|m| m.is_primary).or(monitors.first()) {
        seamless_inner.layout.server_width  = primary.width;
        seamless_inner.layout.server_height = primary.height;
    }
    seamless_inner.layout.server_monitors = monitors;
    let seamless: SharedSeamlessState = Arc::new(Mutex::new(seamless_inner));


    // File-transfer listener + clipboard.
    file::start(tls.clone(), pending.clone())?;

    // Channel for GUI-triggered file sends to the active client.
    let (file_send_tx, mut file_send_rx) =
        tokio::sync::mpsc::unbounded_channel::<PathBuf>();

    let active_for_send = active.clone();
    tokio::spawn(async move {
        while let Some(path) = file_send_rx.recv().await {
            if let Some(ip) = active_for_send.active_client_ip() {
                if let Err(e) = file::sender::send_path(ip, path).await {
                    tracing::warn!("GUI file send error: {e}");
                }
            } else {
                tracing::warn!("GUI file send: no active client");
            }
        }
    });

    // Control channel.
    let bind = bind_addr.to_owned();
    let active_for_net = active.clone();
    let pairing = tls.pairing_code.clone();
    let tls_for_net = tls.clone();
    let seamless_for_net = seamless.clone();
    tokio::spawn(async move {
        if let Err(e) = network::run_server(&bind, audio, active_for_net, tls_for_net, pairing, seamless_for_net).await {
            tracing::error!("Server network error: {e}");
        }
    });

    let pairing_code = tls.pairing_code.clone();
    Ok(ServerHandle { active, pairing_code, pending_file_requests: pending, file_send_tx, seamless })
}
