/// Main egui application.
use std::path::PathBuf;
use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use netshare_client::ConnectionStatus;

use crate::discovery::{MdnsAdvertiser, MdnsBrowser, FoundServer};
use crate::tray::TrayHandle;

// ── Mode state ─────────────────────────────────────────────────────────────

enum Mode {
    Selecting,
    Server {
        handle: netshare_server::ServerHandle,
        _mdns: MdnsAdvertiser,
        transfers: Vec<TransferEntry>,
    },
    Client {
        handle: netshare_client::ClientHandle,
        transfers: Vec<TransferEntry>,
    },
}

#[derive(Clone)]
struct TransferEntry {
    name: String,
    status: String,
}

// ── App ─────────────────────────────────────────────────────────────────────

pub struct NetShareApp {
    mode: Mode,
    // Inputs for Selecting screen.
    bind_addr:   String,
    server_addr: String,
    client_name: String,
    start_error: String,
    // mDNS browser (active in Selecting / Client modes).
    browser: Option<MdnsBrowser>,
    // System tray.
    tray: Option<TrayHandle>,
    window_visible: bool,
    // Runtime handle for spawning tasks.
    rt: tokio::runtime::Handle,
}

impl NetShareApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Handle) -> Self {
        // Start mDNS browser immediately so discovered servers show up quickly.
        let browser = MdnsBrowser::start().ok();

        // Create system tray icon (best-effort — not fatal if it fails).
        let tray = TrayHandle::create().ok();

        let client_name = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "client".to_owned());

        Self {
            mode: Mode::Selecting,
            bind_addr: "0.0.0.0:9000".into(),
            server_addr: "".into(),
            client_name,
            start_error: String::new(),
            browser,
            tray,
            window_visible: true,
            rt,
        }
    }
}

impl eframe::App for NetShareApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll mDNS browser.
        if let Some(b) = &mut self.browser { b.poll(); }

        // Poll tray events.
        if let Some(tray) = &self.tray {
            if tray.poll_toggle() {
                self.window_visible = !self.window_visible;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(self.window_visible));
            }
            if tray.poll_quit() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        // Handle dropped files.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for path in dropped {
            match &mut self.mode {
                Mode::Server { handle, transfers, .. } => {
                    let name = path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    handle.send_file(path);
                    transfers.push(TransferEntry { name, status: "Sending…".into() });
                }
                Mode::Client { handle, transfers } => {
                    let name = path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    handle.send_file(path);
                    transfers.push(TransferEntry { name, status: "Sending…".into() });
                }
                Mode::Selecting => {}
            }
        }

        // ── Close → minimize to taskbar ──────────────────────────────────
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }

        // Update tray tooltip.
        if let Some(tray) = &self.tray {
            let tooltip = match &self.mode {
                Mode::Selecting => "NetShare — mode not selected".into(),
                Mode::Server { handle, .. } => {
                    let slot = handle.active_slot();
                    let clients = handle.clients();
                    if slot == 0 {
                        "NetShare Server — no active client".into()
                    } else {
                        let name = clients.iter()
                            .find(|(s, _)| *s == slot)
                            .map(|(_, n)| n.as_str())
                            .unwrap_or("?");
                        format!("NetShare Server — active: {name} (slot {slot})")
                    }
                }
                Mode::Client { handle, .. } => {
                    let s = handle.state.lock().unwrap();
                    match &s.status {
                        ConnectionStatus::Connected =>
                            format!("NetShare Client — connected to '{}'", s.server_name),
                        ConnectionStatus::Connecting =>
                            "NetShare Client — connecting…".into(),
                        ConnectionStatus::Disconnected(r) =>
                            format!("NetShare Client — disconnected: {r}"),
                    }
                }
            };
            tray.set_tooltip(&tooltip);
        }

        // ── Render ────────────────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            match &mut self.mode {
                Mode::Selecting => render_selecting(
                    ui,
                    &mut self.bind_addr,
                    &mut self.server_addr,
                    &mut self.client_name,
                    &mut self.start_error,
                    &mut self.browser,
                    &mut self.mode,
                    &self.rt,
                ),
                Mode::Server { handle, transfers, .. } => {
                    render_server(ui, handle, transfers);
                }
                Mode::Client { handle, transfers } => {
                    render_client(ui, handle, transfers);
                }
            }
        });

        // Repaint continuously so status stays fresh.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

// ── Selecting screen ────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render_selecting(
    ui: &mut egui::Ui,
    bind_addr: &mut String,
    server_addr: &mut String,
    client_name: &mut String,
    start_error: &mut String,
    browser: &mut Option<MdnsBrowser>,
    mode: &mut Mode,
    rt: &tokio::runtime::Handle,
) {
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        ui.heading(RichText::new("NetShare").size(28.0).strong());
        ui.label("Network Device Sharing — choose how to use this machine.");
        ui.add_space(20.0);

        // ── Server Mode ──────────────────────────────────────────────────
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(360.0);
            ui.label(RichText::new("Server Mode").strong().size(16.0));
            ui.label("Share this PC's mouse and keyboard with clients on your LAN.");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Bind address:");
                ui.text_edit_singleline(bind_addr);
            });
            ui.add_space(4.0);
            if ui.button(RichText::new("  Start Server  ").size(14.0)).clicked() {
                let _guard = rt.enter();
                match netshare_server::start(bind_addr) {
                    Ok(handle) => {
                        let mdns_host = gethostname();
                        let port = bind_addr.split(':').last()
                            .and_then(|p| p.parse().ok()).unwrap_or(9000);
                        let _mdns = MdnsAdvertiser::start(&mdns_host, port).ok()
                            .unwrap_or_else(|| MdnsAdvertiser::dummy());
                        *start_error = String::new();
                        *mode = Mode::Server { handle, _mdns, transfers: Vec::new() };
                    }
                    Err(e) => *start_error = format!("Server start failed: {e}"),
                }
            }
        });

        ui.add_space(16.0);

        // ── Client Mode ──────────────────────────────────────────────────
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(360.0);
            ui.label(RichText::new("Client Mode").strong().size(16.0));
            ui.label("Receive input from your main PC and share audio/files.");
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label("Your name:");
                ui.text_edit_singleline(client_name);
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Server address:");
                ui.text_edit_singleline(server_addr);
            });

            // mDNS discovered servers dropdown.
            if let Some(b) = browser {
                if !b.servers.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Discovered on LAN:").small());
                    for s in b.servers.clone() {
                        let label = format!("{} — {}:{}", s.name, s.addr, s.port);
                        if ui.small_button(&label).clicked() {
                            *server_addr = format!("{}:{}", s.addr, s.port);
                        }
                    }
                }
            }

            ui.add_space(4.0);
            if ui.button(RichText::new("  Connect  ").size(14.0)).clicked() && !server_addr.is_empty() {
                let _guard = rt.enter();
                match netshare_client::start(server_addr, client_name) {
                    Ok(handle) => {
                        *start_error = String::new();
                        *mode = Mode::Client { handle, transfers: Vec::new() };
                    }
                    Err(e) => *start_error = format!("Client start failed: {e}"),
                }
            }
        });

        if !start_error.is_empty() {
            ui.add_space(8.0);
            ui.colored_label(Color32::RED, start_error.as_str());
        }
    });
}

// ── Server view ─────────────────────────────────────────────────────────────

fn render_server(
    ui: &mut egui::Ui,
    handle: &mut netshare_server::ServerHandle,
    transfers: &mut Vec<TransferEntry>,
) {
    ui.horizontal(|ui| {
        ui.heading("NetShare — Server");
        ui.add_space(8.0);
        ui.colored_label(Color32::from_rgb(0x40, 0xC0, 0x40), "● Running");
    });
    ui.separator();

    // ── Clients ──────────────────────────────────────────────────────────
    let clients = handle.clients();
    let active  = handle.active_slot();

    ui.columns(2, |cols| {
        // Left: client list.
        cols[0].label(RichText::new("Connected Clients").strong());
        if clients.is_empty() {
            cols[0].label(RichText::new("No clients connected").color(Color32::GRAY));
        } else {
            for (slot, name) in &clients {
                let is_active = *slot == active;
                cols[0].horizontal(|ui| {
                    let dot = if is_active { "●" } else { "○" };
                    let color = if is_active { Color32::from_rgb(0x40, 0xC0, 0x40) } else { Color32::GRAY };
                    ui.colored_label(color, dot);
                    ui.label(format!("[{slot}] {name}"));
                    if is_active {
                        ui.label(RichText::new("active").small().color(Color32::from_rgb(0x40, 0xC0, 0x40)));
                    }
                });
            }
        }

        // Right: controls.
        cols[1].label(RichText::new("Controls").strong());

        // Active client label.
        if active == 0 {
            cols[1].label("No active client");
        } else {
            let name = clients.iter().find(|(s, _)| *s == active)
                .map(|(_, n)| n.as_str()).unwrap_or("?");
            cols[1].label(format!("Active: {} (slot {})", name, active));
            cols[1].label(RichText::new("Switch: Ctrl+Shift+Alt+[1-9]  |  Cycle: Scroll Lock")
                .small().color(Color32::GRAY));
        }

        cols[1].add_space(8.0);

        // Broadcast mode toggle.
        let mut bcast = handle.broadcast_mode();
        let resp = cols[1].checkbox(&mut bcast, "Broadcast mode");
        if resp.changed() {
            handle.set_broadcast_mode(bcast);
        }
        if bcast {
            cols[1].horizontal(|ui| {
                ui.colored_label(Color32::RED, "⚠");
                ui.colored_label(Color32::RED, "Input sent to ALL clients simultaneously!");
            });
        }
    });

    ui.separator();

    // ── File drop zone ────────────────────────────────────────────────────
    let active_name = if active == 0 { None } else {
        clients.iter().find(|(s, _)| *s == active).map(|(_, n)| n.clone())
    };

    ui.label(RichText::new("File Transfers").strong());
    let target_label = active_name
        .as_deref()
        .map(|n| format!("Drop files here to send to {n}"))
        .unwrap_or_else(|| "Connect a client to enable file sending".into());

    drop_zone(ui, &target_label, active_name.is_some());

    if !transfers.is_empty() {
        ui.add_space(4.0);
        egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
            for t in transfers.iter() {
                ui.horizontal(|ui| {
                    ui.label(&t.name);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(&t.status).small().color(Color32::GRAY));
                    });
                });
            }
        });
    }

    // Receive folder path.
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Receive folder:");
        ui.label(
            RichText::new(netshare_server::file::receive_dir().display().to_string())
                .small().color(Color32::GRAY),
        );
        if ui.small_button("Open…").clicked() {
            let dir = netshare_server::file::receive_dir();
            let _ = open_folder(&dir);
        }
    });
}

// ── Client view ─────────────────────────────────────────────────────────────

fn render_client(
    ui: &mut egui::Ui,
    handle: &mut netshare_client::ClientHandle,
    transfers: &mut Vec<TransferEntry>,
) {
    let state = handle.state.lock().unwrap().clone_fields();

    ui.horizontal(|ui| {
        ui.heading("NetShare — Client");
        ui.add_space(8.0);
        match &state.status {
            ConnectionStatus::Connected => {
                ui.colored_label(Color32::from_rgb(0x40, 0xC0, 0x40), "● Connected");
            }
            ConnectionStatus::Connecting => {
                ui.colored_label(Color32::YELLOW, "○ Connecting…");
            }
            ConnectionStatus::Disconnected(r) => {
                ui.colored_label(Color32::RED, "✕ Disconnected");
                if !r.is_empty() { ui.label(RichText::new(r.as_str()).small()); }
            }
        }
    });
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("Server:");
        ui.label(handle.server_addr().to_string());
    });
    if !state.server_name.is_empty() {
        ui.horizontal(|ui| {
            ui.label("Server name:");
            ui.label(&state.server_name);
        });
    }
    if state.assigned_slot > 0 {
        ui.horizontal(|ui| {
            ui.label("My slot:");
            ui.label(state.assigned_slot.to_string());
        });
    }
    if state.active_slot > 0 {
        ui.horizontal(|ui| {
            ui.label("Active client:");
            if state.active_slot == state.assigned_slot {
                ui.colored_label(
                    Color32::from_rgb(0x40, 0xC0, 0x40),
                    format!("● {} (me — slot {})", state.active_name, state.active_slot),
                );
            } else {
                ui.label(format!("{} (slot {})", state.active_name, state.active_slot));
            }
        });
    }

    ui.separator();
    ui.label(RichText::new("File Transfers").strong());
    drop_zone(ui, "Drop files here to send to server", true);

    if !transfers.is_empty() {
        ui.add_space(4.0);
        egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
            for t in transfers.iter() {
                ui.horizontal(|ui| {
                    ui.label(&t.name);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(&t.status).small().color(Color32::GRAY));
                    });
                });
            }
        });
    }

    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Receive folder:");
        ui.label(
            RichText::new(netshare_client::file::receive_dir().display().to_string())
                .small().color(Color32::GRAY),
        );
        if ui.small_button("Open…").clicked() {
            let dir = netshare_client::file::receive_dir();
            let _ = open_folder(&dir);
        }
    });
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// A visual "drag files here" zone.
fn drop_zone(ui: &mut egui::Ui, label: &str, enabled: bool) {
    let size = egui::vec2(ui.available_width(), 56.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let style = ui.style();
    let color = if enabled {
        Color32::from_gray(50)
    } else {
        Color32::from_gray(30)
    };
    let text_color = if enabled { Color32::from_gray(160) } else { Color32::from_gray(80) };
    ui.painter().rect(rect, 4.0, color, Stroke::new(1.0, Color32::from_gray(80)));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(13.0),
        text_color,
    );
}

fn gethostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "server".to_owned())
}

fn open_folder(path: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    { std::process::Command::new("explorer").arg(path).spawn()?; }
    #[cfg(target_os = "linux")]
    { std::process::Command::new("xdg-open").arg(path).spawn()?; }
    Ok(())
}

// ── Clone helper on ClientGuiState ──────────────────────────────────────────

struct ClientSnapshot {
    status:        ConnectionStatus,
    server_name:   String,
    assigned_slot: u8,
    active_slot:   u8,
    active_name:   String,
}

trait CloneFields {
    fn clone_fields(&self) -> ClientSnapshot;
}

impl CloneFields for netshare_client::ClientGuiState {
    fn clone_fields(&self) -> ClientSnapshot {
        ClientSnapshot {
            status:        self.status.clone(),
            server_name:   self.server_name.clone(),
            assigned_slot: self.assigned_slot,
            active_slot:   self.active_slot,
            active_name:   self.active_name.clone(),
        }
    }
}

