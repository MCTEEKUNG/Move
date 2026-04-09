/// Main egui application — Refactored for Automatic Hub Mode.
use std::sync::{Arc, Mutex};
use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use netshare_core::layout::{ClientEdge, LayoutConfig};
use crate::discovery::{MdnsAdvertiser, MdnsBrowser};
use crate::tray::TrayHandle;

// ── Shared Application Log State ─────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub time: String,
    pub level: String,
    pub message: String,
}

#[derive(Default)]
pub struct AppLogs {
    pub entries: Vec<LogEntry>,
}

impl AppLogs {
    pub fn add(&mut self, level: &str, message: &str) {
        let time = chrono::Local::now().format("%H:%M:%S").to_string();
        self.entries.push(LogEntry {
            time,
            level: level.to_owned(),
            message: message.to_owned(),
        });
        if self.entries.len() > 1000 {
            self.entries.remove(0);
        }
    }
}

lazy_static::lazy_static! {
    pub static ref GLOBAL_LOGS: Arc<Mutex<AppLogs>> = Arc::new(Mutex::new(AppLogs::default()));
}

// ── UI State & Selection ─────────────────────────────────────────────────────

#[derive(PartialEq, Eq)]
enum ActivePage {
    Dashboard,
    Settings,
    Logs,
}

#[derive(Default)]
struct LayoutDragState {
    dragging_slot: Option<u8>,
    drag_screen_pos: egui::Pos2,
}

#[derive(Clone)]
struct TransferEntry {
    name:   String,
    status: String,
}

// ── App Handle (Background Services) ──────────────────────────────────────────

struct AppServices {
    server: Option<netshare_server::ServerHandle>,
    client: Option<netshare_client::ClientHandle>,
    _mdns:  Option<MdnsAdvertiser>,
}

// ── Application ───────────────────────────────────────────────────────────────

pub struct NetShareApp {
    services:      AppServices,
    active_page:   ActivePage,
    
    // Config / Form State
    bind_addr:     String,
    client_name:   String,
    pairing_input: String,
    auto_connect:  bool,
    
    // UI Helpers
    browser:        Option<MdnsBrowser>,
    tray:           Option<TrayHandle>,
    window_visible: bool,
    rt:             tokio::runtime::Handle,
    layout_drag:    LayoutDragState,
}

impl NetShareApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Handle) -> Self {
        let browser     = MdnsBrowser::start().ok();
        let tray        = TrayHandle::create().ok();
        let client_name = gethostname();
        
        let mut app = Self {
            services: AppServices { server: None, client: None, _mdns: None },
            active_page: ActivePage::Dashboard,
            bind_addr: "0.0.0.0:9000".into(),
            client_name,
            pairing_input: String::new(),
            auto_connect: true,
            browser,
            tray,
            window_visible: true,
            rt,
            layout_drag: LayoutDragState::default(),
        };

        // AUTO-START: Start server immediately
        app.start_server();

        app
    }

    fn start_server(&mut self) {
        let _guard = self.rt.enter();
        match netshare_server::start(&self.bind_addr) {
            Ok(handle) => {
                let host_name = gethostname();
                let port = self.bind_addr.split(':').last()
                    .and_then(|p| p.parse().ok()).unwrap_or(9000);
                let mdns = MdnsAdvertiser::start(&host_name, port).ok();
                
                self.services.server = Some(handle);
                self.services._mdns  = mdns;
                log_info("Local server started automatically.");
            }
            Err(e) => log_error(&format!("Auto-server start failed: {e}")),
        }
    }
}

impl eframe::App for NetShareApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll background discovery
        if let Some(b) = &mut self.browser { b.poll(); }

        // Poll tray
        if let Some(tray) = &self.tray {
            if tray.poll_toggle() {
                self.window_visible = !self.window_visible;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(self.window_visible));
            }
            if tray.poll_quit() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        // AUTO-CONNECT (Discovery) - Throttled to avoid starving the main thread
        static LAST_CONNECT_ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        
        if self.auto_connect && self.services.client.is_none() && (now - LAST_CONNECT_ATTEMPT.load(std::sync::atomic::Ordering::Relaxed) > 5) {
            let local_host = gethostname();
            if let Some(found) = self.browser.as_ref().and_then(|b| {
                // Find first server that ISN'T us
                b.servers.iter().find(|s| !s.name.contains(&local_host))
            }) {
                LAST_CONNECT_ATTEMPT.store(now, std::sync::atomic::Ordering::Relaxed);
                let addr = format!("{}:{}", found.addr, found.port);
                let _guard = self.rt.enter();
                if let Ok(handle) = netshare_client::start(&addr, &self.client_name, "") {
                    self.services.client = Some(handle);
                    log_info(&format!("Auto-connected to remote server: {}", found.name));
                }
            }
        }

        // Render Glassmorphism UI
        setup_mica_visuals(ctx);

        egui::SidePanel::left("nav_panel")
            .resizable(false)
            .default_width(70.0)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    // App Logo (Cyan Circle)
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::hover());
                    ui.painter().circle_filled(rect.center(), 12.0, Color32::from_rgb(0, 218, 243));
                    ui.add_space(40.0);

                    nav_button(ui, "", "Dashboard", &mut self.active_page, ActivePage::Dashboard);
                    ui.add_space(20.0);
                    nav_button(ui, "⚙", "Settings", &mut self.active_page, ActivePage::Settings);
                    ui.add_space(20.0);
                    nav_button(ui, "", "Logs", &mut self.active_page, ActivePage::Logs);
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.active_page {
                ActivePage::Dashboard => self.render_dashboard(ui),
                ActivePage::Settings  => self.render_settings(ui),
                ActivePage::Logs      => self.render_logs(ui),
            }
        });

        // Request repaint for animations at 30fps (33ms) to save CPU/prevent lag
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

// ── UI Components ─────────────────────────────────────────────────────────────

fn nav_button(ui: &mut egui::Ui, icon: &str, _label: &str, current: &mut ActivePage, target: ActivePage) {
    let selected = *current == target;
    let color = if selected { Color32::from_rgb(0, 218, 243) } else { Color32::from_gray(100) };
    
    if ui.selectable_label(false, RichText::new(icon).size(24.0).color(color)).clicked() {
        *current = target;
    }
}

fn setup_mica_visuals(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.window_fill = Color32::from_rgba_premultiplied(17, 19, 23, 240);
    style.visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(26, 28, 32);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(30, 32, 36);
    ctx.set_style(style);
}

// ── Canvas Drawing Helpers ──

const PLACE_GAP: f32 = 12.0;

fn server_monitor_rects(
    layout: &LayoutConfig,
    canvas_min: egui::Pos2,
    target_w: f32,
    target_h: f32,
) -> (Vec<(egui::Rect, bool)>, egui::Rect) {
    if layout.server_monitors.is_empty() {
        let srv = egui::Rect::from_center_size(
            canvas_min + egui::vec2(target_w / 2.0, target_h / 2.0),
            egui::vec2(160.0, 100.0),
        );
        return (vec![(srv, true)], srv);
    }
    let (vx_min, vy_min, vx_max, vy_max) = layout.server_bounds();
    let v_w = (vx_max - vx_min).max(1) as f32;
    let v_h = (vy_max - vy_min).max(1) as f32;
    let pad = 30.0_f32;
    let scale = ((target_w - pad*2.0) / v_w).min((target_h - pad*2.0) / v_h);
    let off_x = canvas_min.x + (target_w - v_w * scale)/2.0;
    let off_y = canvas_min.y + (target_h - v_h * scale)/2.0;
    let mut rects = Vec::new();
    let mut primary = egui::Rect::NOTHING;
    for m in &layout.server_monitors {
        let r = egui::Rect::from_min_size(
            egui::pos2(off_x + (m.x - vx_min) as f32 * scale, off_y + (m.y - vy_min) as f32 * scale),
            egui::vec2(m.width as f32 * scale, m.height as f32 * scale)
        );
        if m.is_primary { primary = r; }
        rects.push((r, m.is_primary));
    }
    if primary == egui::Rect::NOTHING {
        primary = rects.first().map(|(r,_)| *r).unwrap_or(egui::Rect::NOTHING);
    }
    (rects, primary)
}

fn placed_center(edge: ClientEdge, size: egui::Vec2, srv: egui::Rect) -> egui::Pos2 {
    match edge {
        ClientEdge::Right => egui::pos2(srv.right() + PLACE_GAP + size.x/2.0, srv.center().y),
        ClientEdge::Left  => egui::pos2(srv.left()  - PLACE_GAP - size.x/2.0, srv.center().y),
        ClientEdge::Below => egui::pos2(srv.center().x, srv.bottom() + PLACE_GAP + size.y/2.0),
        ClientEdge::Above => egui::pos2(srv.center().x, srv.top()    - PLACE_GAP - size.y/2.0),
    }
}

impl NetShareApp {
    fn render_dashboard(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading(RichText::new("System Topology").strong().color(Color32::WHITE));
            ui.label(RichText::new("Visual layout of your workspace").small().color(Color32::GRAY));
            ui.add_space(20.0);

            if let Some(handle) = &mut self.services.server {
                let layout  = handle.layout();
                let clients = handle.clients();
                let pings   = handle.pings();
                
                let (canvas_rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 350.0), Sense::hover());
                let painter = ui.painter_at(canvas_rect);
                
                // Background
                painter.rect_filled(canvas_rect, 12.0, Color32::from_rgb(20, 22, 26));
                painter.rect_stroke(canvas_rect, 12.0, Stroke::new(1.0, Color32::from_rgb(40, 42, 48)));

                let (monitors, srv_anchor) = server_monitor_rects(&layout, canvas_rect.min, canvas_rect.width(), canvas_rect.height());

                // Draw Server Monitors
                let local_host = gethostname();
                let server_name = handle.server_name(); // We need to add this method to ServerHandle
                let is_local_server = server_name == local_host; 

                for (rect, primary) in monitors {
                    let fill = if primary { Color32::from_rgb(0, 40, 80) } else { Color32::from_rgb(20, 25, 30) };
                    let stroke = if primary { Color32::from_rgb(0, 218, 243) } else { Color32::from_rgb(60, 65, 75) };
                    painter.rect(rect, 6.0, fill, Stroke::new(2.0, stroke));
                    
                    let label = if is_local_server {
                        if primary { "THIS MACHINE (HOST)".to_string() } else { "MONITOR".to_string() }
                    } else {
                        if primary { format!("REMOTE: {}", server_name) } else { "MONITOR".to_string() }
                    };
                    
                    let res_label = format!("{}×{}", rect.width() as i32, rect.height() as i32); // Note: this is scaled px
                    painter.text(rect.center() - egui::vec2(0.0, 5.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.0), Color32::from_gray(200));
                    // Check if rect is big enough for res label
                    if rect.height() > 30.0 {
                        painter.text(rect.center() + egui::vec2(0.0, 10.0), egui::Align2::CENTER_CENTER, format!("{}×{}", layout.server_width, layout.server_height), egui::FontId::proportional(8.0), Color32::from_gray(100));
                    }
                }

                // Draw Clients
                for (slot, name) in clients {
                    if let Some(p) = layout.placements.get(&slot) {
                        // Calculate proportional size for client box
                        // Use a base scale relative to the server's primary monitor
                        let rel_w = (p.client_width as f32 / layout.server_width.max(1) as f32) * 160.0;
                        let rel_h = (p.client_height as f32 / layout.server_height.max(1) as f32) * 100.0;
                        
                        // Clamp size so it doesn't get too small or too huge on canvas
                        let size = egui::vec2(rel_w.clamp(80.0, 200.0), rel_h.clamp(50.0, 120.0));
                        
                        let center = placed_center(p.edge, size, srv_anchor);
                        let rect = egui::Rect::from_center_size(center, size);
                        
                        // Glassmorphic Client Box
                        painter.rect(rect, 6.0, Color32::from_rgba_premultiplied(0, 180, 200, 30), Stroke::new(1.5, Color32::from_rgb(0, 218, 243)));
                        
                        // Real Ping from network handle
                        let ping = pings.get(&slot).copied().unwrap_or(0);
                        
                        painter.text(rect.center() - egui::vec2(0.0, 15.0), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(12.0), Color32::WHITE);
                        painter.text(rect.center() + egui::vec2(0.0, 3.0), egui::Align2::CENTER_CENTER, format!("{}×{}", p.client_width, p.client_height), egui::FontId::proportional(9.0), Color32::from_gray(150));
                        painter.text(rect.center() + egui::vec2(0.0, 20.0), egui::Align2::CENTER_CENTER, format!("{} ms", ping), egui::FontId::proportional(10.0), Color32::from_rgb(0, 218, 243));
                        
                        // Connection line
                        let (p1, p2) = match p.edge {
                            ClientEdge::Right => (srv_anchor.right_center(), rect.left_center()),
                            ClientEdge::Left  => (srv_anchor.left_center(), rect.right_center()),
                            ClientEdge::Below => (srv_anchor.center_bottom(), rect.center_top()),
                            ClientEdge::Above => (srv_anchor.center_top(), rect.center_bottom()),
                        };
                        painter.line_segment([p1, p2], Stroke::new(1.0, Color32::from_rgba_premultiplied(0, 218, 243, 80)));
                    }
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Starting Server Subsystems...").color(Color32::GRAY));
                });
            }
        });
    }

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("System Configuration");
        ui.add_space(12.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            // ── Local machine ─────────────────────────────────────────────
            ui.group(|ui| {
                ui.label(RichText::new("Local Machine").strong()
                    .color(Color32::from_rgb(0, 218, 243)));
                ui.add_space(6.0);
                ui.label(format!("Hostname : {}", gethostname()));
                ui.label(format!("Address  : {}", self.bind_addr));
                if let Some(c) = &self.services.client {
                    let s = c.state.lock().unwrap();
                    ui.label(format!("Connected to : {}", c.server_addr()));
                    use netshare_client::ConnectionStatus;
                    let status = match &s.status {
                        ConnectionStatus::Connected => "Connected",
                        ConnectionStatus::Connecting => "Connecting…",
                        ConnectionStatus::Disconnected(_) => "Disconnected",
                    };
                    ui.label(format!("Client status: {status}"));
                }
            });

            ui.add_space(12.0);

            // ── Audio controls ────────────────────────────────────────────
            ui.group(|ui| {
                ui.label(RichText::new("Audio").strong()
                    .color(Color32::from_rgb(0, 218, 243)));
                ui.add_space(6.0);
                if let Some(handle) = &self.services.server {
                    let mut muted = handle.is_mic_muted();
                    if ui.checkbox(&mut muted, "🔇 Mute desktop audio sharing")
                        .changed()
                    {
                        handle.set_mic_muted(muted);
                        log_info(if muted { "Desktop audio muted" } else { "Desktop audio unmuted" });
                    }
                    ui.label(
                        RichText::new(
                            "Streams this machine's speaker output to the other device. \
                             On Linux: requires a PulseAudio/PipeWire monitor source.",
                        ).small().color(Color32::GRAY),
                    );
                }
            });

            ui.add_space(12.0);

            // ── Display layout — edge assignment per client ───────────────
            if let Some(handle) = &mut self.services.server {
                let clients = handle.clients();
                if !clients.is_empty() {
                    ui.group(|ui| {
                        ui.label(RichText::new("Display Layout").strong()
                            .color(Color32::from_rgb(0, 218, 243)));
                        ui.label(
                            RichText::new(
                                "Set which edge of THIS screen each client is positioned at.\n\
                                 The cursor will transfer when it reaches that edge.",
                            ).small().color(Color32::GRAY),
                        );
                        ui.add_space(8.0);

                        let mut layout  = handle.layout();
                        let mut changed = false;

                        for (slot, name) in &clients {
                            ui.horizontal(|ui| {
                                ui.label(format!("[{slot}] {name}:"));
                                ui.add_space(8.0);

                                use netshare_core::layout::{ClientEdge, Placement};
                                let current_edge = layout.placements.get(slot).map(|p| p.edge);

                                for (edge, label) in [
                                    (ClientEdge::Left,  "◀ Left"),
                                    (ClientEdge::Right, "Right ▶"),
                                    (ClientEdge::Above, "▲ Above"),
                                    (ClientEdge::Below, "Below ▼"),
                                ] {
                                    let selected = current_edge == Some(edge);
                                    if ui.selectable_label(selected, label).clicked() && !selected {
                                        let (cw, ch) = layout.placements.get(slot)
                                            .map(|p| (p.client_width, p.client_height))
                                            .unwrap_or((1920, 1080));
                                        // Remove any other slot at this edge first.
                                        layout.placements.retain(|_, p| p.edge != edge);
                                        layout.placements.insert(*slot, Placement {
                                            edge,
                                            client_width: cw,
                                            client_height: ch,
                                        });
                                        changed = true;
                                        log_info(&format!(
                                            "Layout: [{}] {} → {:?} edge", slot, name, edge
                                        ));
                                    }
                                }

                                // Clear button
                                if current_edge.is_some() &&
                                    ui.small_button("✕").on_hover_text("Unassign").clicked()
                                {
                                    layout.placements.remove(slot);
                                    changed = true;
                                }
                            });
                        }

                        if changed {
                            layout.save();
                            handle.set_layout(layout);
                        }
                    });

                    ui.add_space(12.0);
                }
            }

            // ── Behaviour ────────────────────────────────────────────────
            ui.group(|ui| {
                ui.label(RichText::new("Behavior").strong());
                ui.add_space(6.0);
                ui.checkbox(&mut self.auto_connect, "Auto-discover and connect to peers on LAN");
            });

            ui.add_space(12.0);
            if ui.button(RichText::new("⟳  Restart Server Engine")
                .color(Color32::from_rgb(255, 120, 80))).clicked()
            {
                self.start_server();
            }
        });
    }

    fn render_logs(&mut self, ui: &mut egui::Ui) {
        ui.heading("Live Diagnostics");
        ui.add_space(10.0);

        egui::Frame::none()
            .fill(Color32::from_rgb(12, 14, 18))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .max_height(f32::INFINITY)
                    .show(ui, |ui| {
                        let logs = GLOBAL_LOGS.lock().unwrap();
                        for entry in &logs.entries {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&entry.time).color(Color32::DARK_GRAY).monospace().size(11.0));
                                let color = match entry.level.as_str() {
                                    "ERROR" => Color32::from_rgb(255, 80, 80),
                                    "WARN"  => Color32::from_rgb(255, 200, 50),
                                    _       => Color32::from_rgb(0, 218, 243),
                                };
                                ui.label(RichText::new(format!("[{}]", entry.level)).color(color).monospace().size(11.0));
                                ui.label(RichText::new(&entry.message).color(Color32::from_rgb(200, 200, 200)).monospace().size(11.0));
                            });
                        }
                });
            });
    }
}

// ── Log Helpers ─────────────────────────────────────────────────────────────

fn log_info(msg: &str) { GLOBAL_LOGS.lock().unwrap().add("INFO", msg); }
fn log_error(msg: &str) { GLOBAL_LOGS.lock().unwrap().add("ERROR", msg); }

fn gethostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "netshare-node".to_owned())
}
