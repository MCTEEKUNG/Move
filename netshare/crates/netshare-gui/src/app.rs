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

        // ── Auto-reconnect: clear a disconnected client so the slot is free ────
        // After the network task exits it sets status = Disconnected; the handle
        // itself stays Some(…) forever unless we clear it here.  Without this
        // auto-connect never retries after the first drop.
        if let Some(client) = &self.services.client {
            use netshare_client::ConnectionStatus;
            let status = client.state.lock().unwrap_or_else(|e| e.into_inner()).status.clone();
            if matches!(status, ConnectionStatus::Disconnected(_)) {
                self.services.client = None;
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

        // ── Adaptive repaint rate ────────────────────────────────────────────────
        // • 16 ms  (~60 fps) while the user is dragging a screen box
        // • 200 ms (~5 fps)  when connected — keeps ping/status display fresh
        // • 2 000 ms (0.5 fps) when idle/disconnected — just enough to poll
        //   the mDNS browser and trigger auto-connect every 5 seconds
        // egui repaints instantly on any user interaction regardless of this timer,
        // so responsiveness is unaffected.
        let repaint_ms: u64 = if self.layout_drag.dragging_slot.is_some() {
            16
        } else {
            let connected = self.services.client.as_ref().map_or(false, |c| {
                matches!(
                    c.state.lock().unwrap_or_else(|e| e.into_inner()).status,
                    netshare_client::ConnectionStatus::Connected
                )
            });
            if connected { 200 } else { 2_000 }
        };
        ctx.request_repaint_after(std::time::Duration::from_millis(repaint_ms));
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

/// Find which edge the dragged box is closest to snapping onto.
fn closest_snap_edge(pos: egui::Pos2, srv: egui::Rect) -> ClientEdge {
    let dist_right = (pos.x - (srv.right()  + PLACE_GAP)).abs() + (pos.y - srv.center().y).abs() * 0.3;
    let dist_left  = (pos.x - (srv.left()   - PLACE_GAP)).abs() + (pos.y - srv.center().y).abs() * 0.3;
    let dist_below = (pos.y - (srv.bottom() + PLACE_GAP)).abs() + (pos.x - srv.center().x).abs() * 0.3;
    let dist_above = (pos.y - (srv.top()    - PLACE_GAP)).abs() + (pos.x - srv.center().x).abs() * 0.3;

    let min = dist_right.min(dist_left).min(dist_below).min(dist_above);
    if min == dist_right      { ClientEdge::Right }
    else if min == dist_left  { ClientEdge::Left  }
    else if min == dist_below { ClientEdge::Below }
    else                      { ClientEdge::Above }
}

impl NetShareApp {
    fn render_dashboard(&mut self, ui: &mut egui::Ui) {
        use netshare_core::layout::{ClientEdge, Placement};

        // ── Status header ──────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            ui.heading(RichText::new("System Topology").strong().color(Color32::WHITE));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(client) = &self.services.client {
                    let state = client.state.lock().unwrap_or_else(|e| e.into_inner());
                    use netshare_client::ConnectionStatus;
                    match &state.status {
                        ConnectionStatus::Connected => {
                            ui.label(RichText::new("● Connected").color(Color32::from_rgb(80, 220, 100)).strong());
                        }
                        ConnectionStatus::Connecting => {
                            ui.label(RichText::new("◌ Connecting...").color(Color32::from_rgb(255, 200, 50)));
                        }
                        ConnectionStatus::Disconnected(_) => {
                            ui.label(RichText::new("○ Disconnected").color(Color32::GRAY));
                        }
                    }
                } else {
                    ui.label(RichText::new("○ Not connected").color(Color32::GRAY));
                }
            });
        });
        ui.label(RichText::new("Drag a screen to reposition it  —  changes apply immediately").small().color(Color32::GRAY));
        ui.add_space(10.0);

        let Some(handle) = &mut self.services.server else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Starting server...").color(Color32::GRAY));
            });
            return;
        };

        let mut layout         = handle.layout();
        let clients            = handle.clients();
        let pings              = handle.pings();
        let mut layout_changed = false;

        // ── Canvas ─────────────────────────────────────────────────────────────
        let canvas_size = egui::vec2(ui.available_width(), 340.0);
        // We allocate with click_and_drag so the canvas receives pointer events.
        let (canvas_rect, canvas_resp) =
            ui.allocate_exact_size(canvas_size, Sense::click_and_drag());
        let painter = ui.painter_at(canvas_rect);

        // Background
        painter.rect_filled(canvas_rect, 12.0, Color32::from_rgb(20, 22, 26));
        painter.rect_stroke(canvas_rect, 12.0, Stroke::new(1.0, Color32::from_rgb(40, 42, 48)));

        let (monitors, srv_anchor) =
            server_monitor_rects(&layout, canvas_rect.min, canvas_rect.width(), canvas_rect.height());

        // ── Drag-start detection ───────────────────────────────────────────────
        // Must happen before drawing so we know which slot is active this frame.
        if canvas_resp.drag_started() {
            if let Some(ptr) = canvas_resp.interact_pointer_pos() {
                // Pick the client box the user clicked on.
                for (slot, _name) in &clients {
                    let p = layout.placements.get(slot)
                        .cloned()
                        .unwrap_or(Placement { edge: ClientEdge::Right, client_width: 1920, client_height: 1080 });
                    let rel_w = (p.client_width  as f32 / layout.server_width.max(1)  as f32) * 160.0;
                    let rel_h = (p.client_height as f32 / layout.server_height.max(1) as f32) * 100.0;
                    let size  = egui::vec2(rel_w.clamp(80.0, 200.0), rel_h.clamp(50.0, 120.0));
                    let center = placed_center(p.edge, size, srv_anchor);
                    let hit = egui::Rect::from_center_size(center, size);
                    if hit.contains(ptr) {
                        self.layout_drag.dragging_slot   = Some(*slot);
                        self.layout_drag.drag_screen_pos = center;
                        break;
                    }
                }
            }
        }

        // Update drag position while dragging.
        if canvas_resp.dragged() {
            if self.layout_drag.dragging_slot.is_some() {
                self.layout_drag.drag_screen_pos += canvas_resp.drag_delta();
            }
        }

        // ── Snap-zone ghost outlines (shown while dragging) ────────────────────
        let ghost_size = egui::vec2(110.0, 68.0);
        if self.layout_drag.dragging_slot.is_some() {
            for edge in [ClientEdge::Left, ClientEdge::Right, ClientEdge::Above, ClientEdge::Below] {
                let center = placed_center(edge, ghost_size, srv_anchor);
                let ghost  = egui::Rect::from_center_size(center, ghost_size);
                if canvas_rect.contains_rect(ghost) || canvas_rect.intersects(ghost) {
                    painter.rect_stroke(ghost, 6.0, Stroke::new(1.5,
                        Color32::from_rgba_premultiplied(0, 218, 243, 55)));
                    let arrow = match edge {
                        ClientEdge::Left  => "◀",
                        ClientEdge::Right => "▶",
                        ClientEdge::Above => "▲",
                        ClientEdge::Below => "▼",
                    };
                    painter.text(center, egui::Align2::CENTER_CENTER, arrow,
                        egui::FontId::proportional(18.0),
                        Color32::from_rgba_premultiplied(0, 218, 243, 90));
                }
            }
        }

        // ── Draw server monitors ───────────────────────────────────────────────
        let local_host  = gethostname();
        let server_name = handle.server_name();
        let is_local    = server_name == local_host;

        for (rect, primary) in &monitors {
            let fill   = if *primary { Color32::from_rgb(0, 40, 80)   } else { Color32::from_rgb(20, 25, 30) };
            let stroke = if *primary { Color32::from_rgb(0, 218, 243) } else { Color32::from_rgb(60, 65, 75) };
            painter.rect(*rect, 6.0, fill, Stroke::new(2.0, stroke));

            let label = if is_local {
                if *primary { "THIS MACHINE".to_string() } else { "MONITOR".to_string() }
            } else {
                if *primary { format!("REMOTE: {server_name}") } else { "MONITOR".to_string() }
            };
            painter.text(rect.center() - egui::vec2(0.0, 6.0),
                egui::Align2::CENTER_CENTER, label,
                egui::FontId::proportional(10.0), Color32::from_gray(200));
            if rect.height() > 30.0 {
                painter.text(rect.center() + egui::vec2(0.0, 8.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{}×{}", layout.server_width, layout.server_height),
                    egui::FontId::proportional(8.0), Color32::from_gray(100));
            }
        }

        // ── Draw client boxes (draggable) ──────────────────────────────────────
        for (slot, name) in &clients {
            let p = layout.placements.get(slot)
                .cloned()
                .unwrap_or(Placement { edge: ClientEdge::Right, client_width: 1920, client_height: 1080 });

            let rel_w = (p.client_width  as f32 / layout.server_width.max(1)  as f32) * 160.0;
            let rel_h = (p.client_height as f32 / layout.server_height.max(1) as f32) * 100.0;
            let size  = egui::vec2(rel_w.clamp(80.0, 200.0), rel_h.clamp(50.0, 120.0));

            let is_dragging = self.layout_drag.dragging_slot == Some(*slot);
            let center = if is_dragging {
                self.layout_drag.drag_screen_pos
            } else {
                placed_center(p.edge, size, srv_anchor)
            };
            let rect = egui::Rect::from_center_size(center, size);

            // Snap edge for connector line (live preview while dragging)
            let display_edge = if is_dragging {
                closest_snap_edge(center, srv_anchor)
            } else {
                p.edge
            };

            // Connector line from server to client
            let (p1, p2) = match display_edge {
                ClientEdge::Right => (srv_anchor.right_center(),  rect.left_center()),
                ClientEdge::Left  => (srv_anchor.left_center(),   rect.right_center()),
                ClientEdge::Below => (srv_anchor.center_bottom(), rect.center_top()),
                ClientEdge::Above => (srv_anchor.center_top(),    rect.center_bottom()),
            };
            painter.line_segment([p1, p2], Stroke::new(1.0,
                Color32::from_rgba_premultiplied(0, 218, 243, if is_dragging { 130 } else { 80 })));

            // Box fill / stroke
            let fill   = if is_dragging {
                Color32::from_rgba_premultiplied(0, 218, 243, 55)
            } else {
                Color32::from_rgba_premultiplied(0, 180, 200, 30)
            };
            let stroke_col = if is_dragging {
                Color32::from_rgb(0, 243, 180)
            } else {
                Color32::from_rgb(0, 218, 243)
            };
            painter.rect(rect, 6.0, fill, Stroke::new(if is_dragging { 2.0 } else { 1.5 }, stroke_col));

            let ping = pings.get(slot).copied().unwrap_or(0);
            painter.text(rect.center() - egui::vec2(0.0, 15.0), egui::Align2::CENTER_CENTER,
                name, egui::FontId::proportional(12.0), Color32::WHITE);
            painter.text(rect.center() + egui::vec2(0.0, 3.0), egui::Align2::CENTER_CENTER,
                format!("{}×{}", p.client_width, p.client_height),
                egui::FontId::proportional(9.0), Color32::from_gray(150));
            painter.text(rect.center() + egui::vec2(0.0, 19.0), egui::Align2::CENTER_CENTER,
                format!("{} ms", ping),
                egui::FontId::proportional(10.0), Color32::from_rgb(0, 218, 243));

            // Drag cursor hint
            if is_dragging {
                painter.text(rect.center_top() - egui::vec2(0.0, 12.0),
                    egui::Align2::CENTER_CENTER, "✥",
                    egui::FontId::proportional(14.0), Color32::from_gray(200));
            }
        }

        // ── Drag-release: snap & commit ────────────────────────────────────────
        if canvas_resp.drag_stopped() {
            if let Some(slot) = self.layout_drag.dragging_slot.take() {
                let snap_pos = self.layout_drag.drag_screen_pos;
                let new_edge = closest_snap_edge(snap_pos, srv_anchor);
                let (cw, ch) = layout.placements.get(&slot)
                    .map(|p| (p.client_width, p.client_height))
                    .unwrap_or((1920, 1080));
                // Evict any other client already at that edge.
                layout.placements.retain(|k, p| *k == slot || p.edge != new_edge);
                layout.placements.insert(slot, Placement { edge: new_edge, client_width: cw, client_height: ch });
                layout_changed = true;
                if let Some((_slot, name)) = clients.iter().find(|(s, _)| *s == slot) {
                    log_info(&format!("Layout: {name} → {:?}", new_edge));
                }
            }
        }

        // Empty-state hint
        if clients.is_empty() {
            painter.text(canvas_rect.center(), egui::Align2::CENTER_CENTER,
                "Waiting for a remote device to connect...",
                egui::FontId::proportional(13.0), Color32::from_gray(70));
        }

        if layout_changed {
            layout.save();
            handle.set_layout(layout);
        }
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
                    let s = c.state.lock().unwrap_or_else(|e| e.into_inner());
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
                    // audio_enabled is the inverse of muted
                    let mut audio_enabled = !handle.is_mic_muted();
                    if ui.checkbox(&mut audio_enabled, "🔊 Share desktop audio with remote device")
                        .changed()
                    {
                        handle.set_mic_muted(!audio_enabled);
                        log_info(if audio_enabled { "Desktop audio sharing enabled" } else { "Desktop audio sharing disabled" });
                    }
                    ui.label(
                        RichText::new(
                            "Streams this machine's speaker output to the connected device.\n\
                             ⚠ Enable on ONE machine only to avoid audio echo feedback.",
                        ).small().color(Color32::from_rgb(200, 160, 80)),
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
                        let logs = GLOBAL_LOGS.lock().unwrap_or_else(|e| e.into_inner());
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

fn log_info(msg: &str) { GLOBAL_LOGS.lock().unwrap_or_else(|e| e.into_inner()).add("INFO", msg); }
fn log_error(msg: &str) { GLOBAL_LOGS.lock().unwrap_or_else(|e| e.into_inner()).add("ERROR", msg); }

fn gethostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "netshare-node".to_owned())
}
