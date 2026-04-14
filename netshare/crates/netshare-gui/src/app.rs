use crate::discovery::{MdnsAdvertiser, MdnsBrowser};
use crate::tray::TrayHandle;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Rounding, Sense, Stroke, Vec2};
use netshare_core::layout::{ClientEdge, LayoutConfig};
/// Main egui application — Refactored for Automatic Hub Mode & Modern UX.
use std::sync::{Arc, Mutex};

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
    drag_screen_pos: Pos2,
}

// ── App Handle (Background Services) ──────────────────────────────────────────

struct AppServices {
    server: Option<netshare_server::ServerHandle>,
    client: Option<netshare_client::ClientHandle>,
    _mdns: Option<MdnsAdvertiser>,
}

// ── Application ───────────────────────────────────────────────────────────────

pub struct NetShareApp {
    services: AppServices,
    active_page: ActivePage,

    // Config / Form State
    bind_addr: String,
    client_name: String,
    pairing_input: String,
    auto_connect: bool,

    // Audio Output
    output_devices: Vec<String>,
    selected_output_device: String,

    // UI Helpers
    browser: Option<MdnsBrowser>,
    tray: Option<TrayHandle>,
    window_visible: bool,
    rt: tokio::runtime::Handle,
    layout_drag: LayoutDragState,
}

impl NetShareApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Handle) -> Self {
        let browser = MdnsBrowser::start().ok();
        let tray = TrayHandle::create().ok();
        let client_name = gethostname();

        let mut app = Self {
            services: AppServices {
                server: None,
                client: None,
                _mdns: None,
            },
            active_page: ActivePage::Dashboard,
            bind_addr: "0.0.0.0:9000".into(),
            client_name,
            pairing_input: String::new(),
            auto_connect: true,
            output_devices: Vec::new(),
            selected_output_device: String::new(),
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
                let port = self
                    .bind_addr
                    .split(':')
                    .last()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(9000);
                let mdns = MdnsAdvertiser::start(&host_name, port).ok();

                self.services.server = Some(handle);
                self.services._mdns = mdns;
                log_info("Local server started automatically.");
            }
            Err(e) => log_error(&format!("Auto-server start failed: {e}")),
        }
    }
}

impl eframe::App for NetShareApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll background discovery
        if let Some(b) = &mut self.browser {
            b.poll();
        }

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
        if let Some(client) = &self.services.client {
            use netshare_client::ConnectionStatus;
            let status = client
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .status
                .clone();
            if matches!(status, ConnectionStatus::Disconnected(_)) {
                self.services.client = None;
            }
        }

        // AUTO-CONNECT (Discovery)
        static LAST_CONNECT_ATTEMPT: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if self.auto_connect
            && self.services.client.is_none()
            && (now - LAST_CONNECT_ATTEMPT.load(std::sync::atomic::Ordering::Relaxed) > 5)
        {
            let local_host = gethostname();
            if let Some(found) = self.browser.as_ref().and_then(|b| {
                let our_clients: Vec<String> = self
                    .services
                    .server
                    .as_ref()
                    .map(|srv| srv.clients().into_iter().map(|(_, n)| n).collect())
                    .unwrap_or_default();
                b.servers.iter().find(|s| {
                    if s.name.contains(&local_host) {
                        return false;
                    }
                    let already_our_client = our_clients.iter().any(|cn| {
                        let sn = s.name.to_lowercase();
                        let cn = cn.to_lowercase();
                        sn.contains(&cn) || cn.contains(&sn)
                    });
                    !already_our_client
                })
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

        // Render UI Visuals
        setup_modern_visuals(ctx);

        // Sidebar Panel
        egui::SidePanel::left("nav_panel")
            .resizable(false)
            .exact_width(80.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(18, 20, 24))
                    .inner_margin(8.0),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(24.0);
                    // Modern App Logo Placeholder
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(32.0, 32.0), Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        Rounding::same(10.0),
                        Color32::from_rgb(0, 150, 255),
                    );
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "NS",
                        egui::FontId::proportional(14.0).clone(),
                        Color32::WHITE,
                    );
                    ui.add_space(40.0);

                    nav_button(
                        ui,
                        "",
                        "Monitors",
                        &mut self.active_page,
                        ActivePage::Dashboard,
                    );
                    ui.add_space(20.0);
                    nav_button(
                        ui,
                        "⚙",
                        "Settings",
                        &mut self.active_page,
                        ActivePage::Settings,
                    );
                    ui.add_space(20.0);
                    nav_button(ui, "", "Logs", &mut self.active_page, ActivePage::Logs);
                });
            });

        // Main Panel
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(10, 11, 14))
                    .inner_margin(24.0),
            )
            .show(ctx, |ui| match self.active_page {
                ActivePage::Dashboard => self.render_dashboard(ui),
                ActivePage::Settings => self.render_settings(ui),
                ActivePage::Logs => self.render_logs(ui),
            });

        // Adaptive repaint rate
        let repaint_ms: u64 = if self.layout_drag.dragging_slot.is_some() {
            16
        } else {
            let connected = self.services.client.as_ref().map_or(false, |c| {
                matches!(
                    c.state.lock().unwrap_or_else(|e| e.into_inner()).status,
                    netshare_client::ConnectionStatus::Connected
                )
            });
            if connected {
                200
            } else {
                1_000
            }
        };
        ctx.request_repaint_after(std::time::Duration::from_millis(repaint_ms));
    }
}

// ── UI Components ─────────────────────────────────────────────────────────────

fn nav_button(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    current: &mut ActivePage,
    target: ActivePage,
) {
    let selected = *current == target;
    let color = if selected {
        Color32::from_rgb(0, 150, 255)
    } else {
        Color32::from_gray(120)
    };

    let resp = ui.add(egui::SelectableLabel::new(
        selected,
        RichText::new(icon).size(22.0).color(color),
    ));
    if resp.clicked() {
        *current = target;
    }
    resp.on_hover_text(label);
}

fn setup_modern_visuals(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    style.visuals.window_fill = Color32::from_rgb(18, 20, 24);
    style.visuals.panel_fill = Color32::from_rgb(10, 11, 14);

    style.visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(26, 28, 34);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(32, 34, 40);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 45, 52);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(52, 55, 64);

    style.visuals.selection.bg_fill = Color32::from_rgb(0, 150, 255);

    style.visuals.window_rounding = Rounding::same(12.0);
    style.visuals.widgets.noninteractive.rounding = Rounding::same(8.0);
    style.visuals.widgets.inactive.rounding = Rounding::same(8.0);
    style.visuals.widgets.hovered.rounding = Rounding::same(8.0);
    style.visuals.widgets.active.rounding = Rounding::same(8.0);

    ctx.set_style(style);
}

// ── Canvas Drawing Helpers ──

const PLACE_GAP: f32 = 20.0;

fn server_monitor_rects(
    layout: &LayoutConfig,
    canvas_min: Pos2,
    target_w: f32,
    target_h: f32,
) -> (Vec<(Rect, bool)>, Rect) {
    if layout.server_monitors.is_empty() {
        let srv = Rect::from_center_size(
            canvas_min + Vec2::new(target_w / 2.0, target_h / 2.0),
            Vec2::new(200.0, 125.0),
        );
        return (vec![(srv, true)], srv);
    }
    let (vx_min, vy_min, vx_max, vy_max) = layout.server_bounds();
    let v_w = (vx_max - vx_min).max(1) as f32;
    let v_h = (vy_max - vy_min).max(1) as f32;
    let pad = 40.0_f32;
    let scale = ((target_w - pad * 2.0) / v_w).min((target_h - pad * 2.0) / v_h);
    let off_x = canvas_min.x + (target_w - v_w * scale) / 2.0;
    let off_y = canvas_min.y + (target_h - v_h * scale) / 2.0;
    let mut rects = Vec::new();
    let mut primary = Rect::NOTHING;
    for m in &layout.server_monitors {
        let r = Rect::from_min_size(
            egui::pos2(
                off_x + (m.x - vx_min) as f32 * scale,
                off_y + (m.y - vy_min) as f32 * scale,
            ),
            Vec2::new(m.width as f32 * scale, m.height as f32 * scale),
        );
        if m.is_primary {
            primary = r;
        }
        rects.push((r, m.is_primary));
    }
    if primary == Rect::NOTHING {
        primary = rects.first().map(|(r, _)| *r).unwrap_or(Rect::NOTHING);
    }
    (rects, primary)
}

fn placed_center(edge: ClientEdge, size: Vec2, srv: Rect) -> Pos2 {
    match edge {
        ClientEdge::Right => egui::pos2(srv.right() + PLACE_GAP + size.x / 2.0, srv.center().y),
        ClientEdge::Left => egui::pos2(srv.left() - PLACE_GAP - size.x / 2.0, srv.center().y),
        ClientEdge::Below => egui::pos2(srv.center().x, srv.bottom() + PLACE_GAP + size.y / 2.0),
        ClientEdge::Above => egui::pos2(srv.center().x, srv.top() - PLACE_GAP - size.y / 2.0),
    }
}

fn closest_snap_edge(pos: Pos2, srv: Rect) -> ClientEdge {
    let dist_right =
        (pos.x - (srv.right() + PLACE_GAP)).abs() + (pos.y - srv.center().y).abs() * 0.3;
    let dist_left = (pos.x - (srv.left() - PLACE_GAP)).abs() + (pos.y - srv.center().y).abs() * 0.3;
    let dist_below =
        (pos.y - (srv.bottom() + PLACE_GAP)).abs() + (pos.x - srv.center().x).abs() * 0.3;
    let dist_above = (pos.y - (srv.top() - PLACE_GAP)).abs() + (pos.x - srv.center().x).abs() * 0.3;

    let min = dist_right.min(dist_left).min(dist_below).min(dist_above);
    if min == dist_right {
        ClientEdge::Right
    } else if min == dist_left {
        ClientEdge::Left
    } else if min == dist_below {
        ClientEdge::Below
    } else {
        ClientEdge::Above
    }
}

fn render_ping_indicator(ui: &mut egui::Ui, ping_ms: u16) {
    let (color, text) = if ping_ms == 0 {
        (Color32::GRAY, "--- ms".to_string())
    } else if ping_ms < 20 {
        (Color32::from_rgb(50, 220, 100), format!("{} ms", ping_ms))
    } else if ping_ms < 60 {
        (Color32::from_rgb(255, 200, 50), format!("{} ms", ping_ms))
    } else {
        (Color32::from_rgb(255, 80, 80), format!("{} ms", ping_ms))
    };

    let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    ui.add_space(4.0);
    ui.label(RichText::new(text).color(Color32::LIGHT_GRAY).size(11.0));
}

impl NetShareApp {
    fn render_dashboard(&mut self, ui: &mut egui::Ui) {
        use netshare_core::layout::{ClientEdge, Placement};

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(
                    RichText::new("Monitor Manager")
                        .size(24.0)
                        .strong()
                        .color(Color32::WHITE),
                );
                ui.label(
                    RichText::new(format!("Local Hostname: {}", gethostname()))
                        .color(Color32::GRAY),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.auto_connect, "Auto Connect to LAN");
            });
        });
        ui.add_space(20.0);

        // ── TOP SECTION: Network & Discovery ──
        ui.horizontal(|ui| {
            // Live Status Panel
            egui::Frame::none()
                .fill(Color32::from_rgb(18, 20, 24))
                .rounding(Rounding::same(12.0))
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() * 0.45);
                    ui.label(
                        RichText::new("Live Connection")
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add_space(8.0);

                    if let Some(client) = &self.services.client {
                        let state = client.state.lock().unwrap_or_else(|e| e.into_inner());
                        use netshare_client::ConnectionStatus;
                        match &state.status {
                            ConnectionStatus::Connected => {
                                ui.horizontal(|ui| {
                                    let (rect, _) = ui
                                        .allocate_exact_size(Vec2::new(12.0, 12.0), Sense::hover());
                                    ui.painter().circle_filled(
                                        rect.center(),
                                        5.0,
                                        Color32::from_rgb(50, 220, 100),
                                    );
                                    ui.label(
                                        RichText::new("Connected")
                                            .color(Color32::from_rgb(50, 220, 100))
                                            .strong(),
                                    );
                                });
                                ui.label(
                                    RichText::new(format!("To: {}", client.server_addr()))
                                        .color(Color32::LIGHT_GRAY),
                                );
                            }
                            ConnectionStatus::Connecting => {
                                ui.horizontal(|ui| {
                                    let (rect, _) = ui
                                        .allocate_exact_size(Vec2::new(12.0, 12.0), Sense::hover());
                                    ui.painter().circle_filled(
                                        rect.center(),
                                        5.0,
                                        Color32::from_rgb(255, 200, 50),
                                    );
                                    ui.label(
                                        RichText::new("Connecting...")
                                            .color(Color32::from_rgb(255, 200, 50))
                                            .strong(),
                                    );
                                });
                            }
                            ConnectionStatus::Disconnected(_) => {
                                ui.label(RichText::new("Disconnected").color(Color32::GRAY));
                            }
                        }
                    } else {
                        ui.label(
                            RichText::new("No active outbound connection.").color(Color32::GRAY),
                        );
                    }
                });

            // Auto-Discovery Panel
            egui::Frame::none()
                .fill(Color32::from_rgb(18, 20, 24))
                .rounding(Rounding::same(12.0))
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new("Detected LAN Devices")
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add_space(8.0);

                    if let Some(browser) = &self.browser {
                        if browser.servers.is_empty() {
                            ui.label(
                                RichText::new("Searching for devices...")
                                    .color(Color32::GRAY)
                                    .italics(),
                            );
                        } else {
                            for s in &browser.servers {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("💻").size(14.0));
                                    ui.label(RichText::new(&s.name).color(Color32::LIGHT_GRAY));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(format!("{}:{}", s.addr, s.port))
                                                    .color(Color32::DARK_GRAY)
                                                    .size(11.0),
                                            );
                                        },
                                    );
                                });
                            }
                        }
                    } else {
                        ui.label(RichText::new("mDNS discovery disabled.").color(Color32::GRAY));
                    }
                });
        });
        ui.add_space(20.0);

        // Secondary screen view
        {
            use netshare_client::ConnectionStatus;
            let client_connected = self.services.client.as_ref().map_or(false, |c| {
                matches!(
                    c.state.lock().unwrap_or_else(|e| e.into_inner()).status,
                    ConnectionStatus::Connected
                )
            });
            let local_client_count = self
                .services
                .server
                .as_ref()
                .map_or(0, |s| s.clients().len());

            if client_connected && local_client_count == 0 {
                self.render_secondary_screen_dashboard(ui);
                return;
            }
        }

        let Some(handle) = &mut self.services.server else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Starting server...").color(Color32::GRAY));
            });
            return;
        };

        let mut layout = handle.layout();
        let clients = handle.clients();
        let pings = handle.pings();
        let mut layout_changed = false;

        ui.label(
            RichText::new("Virtual Displays")
                .strong()
                .color(Color32::WHITE),
        );
        ui.label(
            RichText::new("Drag virtual displays to edge of the primary monitor to snap. Click on a display to manually focus keyboard/mouse on it.")
                .color(Color32::GRAY)
                .size(12.0),
        );
        ui.add_space(8.0);

        // ── Canvas ─────────────────────────────────────────────────────────────
        let canvas_size = Vec2::new(ui.available_width(), ui.available_height());
        let (canvas_rect, canvas_resp) =
            ui.allocate_exact_size(canvas_size, Sense::click_and_drag());
        let painter = ui.painter_at(canvas_rect);

        // Canvas Background
        painter.rect_filled(
            canvas_rect,
            Rounding::same(16.0),
            Color32::from_rgb(18, 20, 24),
        );
        painter.rect_stroke(
            canvas_rect,
            Rounding::same(16.0),
            Stroke::new(1.0, Color32::from_rgb(32, 34, 40)),
        );

        let (monitors, srv_anchor) = server_monitor_rects(
            &layout,
            canvas_rect.min,
            canvas_rect.width(),
            canvas_rect.height(),
        );

        // ── Drag-start detection ───────────────────────────────────────────────
        if canvas_resp.drag_started() {
            if let Some(ptr) = canvas_resp.interact_pointer_pos() {
                for (slot, _name) in &clients {
                    let p = layout.placements.get(slot).cloned().unwrap_or(Placement {
                        edge: ClientEdge::Right,
                        client_width: 1920,
                        client_height: 1080,
                    });
                    let rel_w = (p.client_width as f32 / layout.server_width.max(1) as f32) * 200.0;
                    let rel_h =
                        (p.client_height as f32 / layout.server_height.max(1) as f32) * 125.0;
                    let size = Vec2::new(rel_w.clamp(100.0, 250.0), rel_h.clamp(60.0, 150.0));
                    let center = placed_center(p.edge, size, srv_anchor);
                    let hit = Rect::from_center_size(center, size);
                    if hit.contains(ptr) {
                        self.layout_drag.dragging_slot = Some(*slot);
                        self.layout_drag.drag_screen_pos = center;
                        break;
                    }
                }
            }
        }

        // ── Click detection (Focus PC) ─────────────────────────────────────────
        if canvas_resp.clicked() {
            if let Some(ptr) = canvas_resp.interact_pointer_pos() {
                let mut clicked_slot = None;
                for (slot, _name) in &clients {
                    let p = layout.placements.get(slot).cloned().unwrap_or(Placement {
                        edge: ClientEdge::Right,
                        client_width: 1920,
                        client_height: 1080,
                    });
                    let rel_w = (p.client_width as f32 / layout.server_width.max(1) as f32) * 200.0;
                    let rel_h =
                        (p.client_height as f32 / layout.server_height.max(1) as f32) * 125.0;
                    let size = Vec2::new(rel_w.clamp(100.0, 250.0), rel_h.clamp(60.0, 150.0));
                    let center = placed_center(p.edge, size, srv_anchor);
                    let hit = Rect::from_center_size(center, size);
                    if hit.contains(ptr) {
                        clicked_slot = Some(*slot);
                        break;
                    }
                }
                if let Some(slot) = clicked_slot {
                    handle.focus_client(slot);
                } else if srv_anchor.contains(ptr) {
                    handle.focus_client(0); // click server to return focus
                }
            }
        }

        if canvas_resp.dragged() {
            if self.layout_drag.dragging_slot.is_some() {
                self.layout_drag.drag_screen_pos += canvas_resp.drag_delta();
            }
        }

        // Snap-zone ghost outlines
        let ghost_size = Vec2::new(140.0, 85.0);
        if self.layout_drag.dragging_slot.is_some() {
            for edge in [
                ClientEdge::Left,
                ClientEdge::Right,
                ClientEdge::Above,
                ClientEdge::Below,
            ] {
                let center = placed_center(edge, ghost_size, srv_anchor);
                let ghost = Rect::from_center_size(center, ghost_size);
                if canvas_rect.contains_rect(ghost) || canvas_rect.intersects(ghost) {
                    painter.rect_stroke(
                        ghost,
                        Rounding::same(12.0),
                        Stroke::new(2.0, Color32::from_rgba_premultiplied(0, 150, 255, 100)),
                    );
                    let arrow = match edge {
                        ClientEdge::Left => "◀",
                        ClientEdge::Right => "▶",
                        ClientEdge::Above => "▲",
                        ClientEdge::Below => "▼",
                    };
                    painter.text(
                        center,
                        egui::Align2::CENTER_CENTER,
                        arrow,
                        egui::FontId::proportional(22.0),
                        Color32::from_rgba_premultiplied(0, 150, 255, 120),
                    );
                }
            }
        }

        // Draw server monitors
        let local_host = gethostname();
        let server_name = handle.server_name();
        let is_local = server_name == local_host;

        for (rect, primary) in &monitors {
            let fill = if *primary {
                Color32::from_rgb(25, 40, 60)
            } else {
                Color32::from_rgb(30, 32, 38)
            };
            let stroke = if *primary {
                Color32::from_rgb(0, 150, 255)
            } else {
                Color32::from_rgb(80, 85, 95)
            };

            // Draw a subtle shadow/glow if primary
            if *primary {
                painter.rect_stroke(
                    rect.expand(2.0),
                    Rounding::same(14.0),
                    Stroke::new(1.0, Color32::from_rgba_premultiplied(0, 150, 255, 50)),
                );
            }

            painter.rect(*rect, Rounding::same(12.0), fill, Stroke::new(2.0, stroke));

            let label = if is_local {
                if *primary {
                    "PRIMARY"
                } else {
                    "MONITOR"
                }
            } else {
                if *primary {
                    "REMOTE MAIN"
                } else {
                    "MONITOR"
                }
            };

            painter.text(
                rect.center() - Vec2::new(0.0, 10.0),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );

            if rect.height() > 40.0 {
                painter.text(
                    rect.center() + Vec2::new(0.0, 10.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{}×{}", layout.server_width, layout.server_height),
                    egui::FontId::proportional(11.0),
                    Color32::LIGHT_GRAY,
                );
            }
        }

        // Draw client boxes
        for (slot, name) in &clients {
            let p = layout.placements.get(slot).cloned().unwrap_or(Placement {
                edge: ClientEdge::Right,
                client_width: 1920,
                client_height: 1080,
            });

            let rel_w = (p.client_width as f32 / layout.server_width.max(1) as f32) * 200.0;
            let rel_h = (p.client_height as f32 / layout.server_height.max(1) as f32) * 125.0;
            let size = Vec2::new(rel_w.clamp(100.0, 250.0), rel_h.clamp(60.0, 150.0));

            let is_dragging = self.layout_drag.dragging_slot == Some(*slot);
            let center = if is_dragging {
                self.layout_drag.drag_screen_pos
            } else {
                placed_center(p.edge, size, srv_anchor)
            };
            let rect = Rect::from_center_size(center, size);

            let display_edge = if is_dragging {
                closest_snap_edge(center, srv_anchor)
            } else {
                p.edge
            };

            // Connector line
            let (p1, p2) = match display_edge {
                ClientEdge::Right => (srv_anchor.right_center(), rect.left_center()),
                ClientEdge::Left => (srv_anchor.left_center(), rect.right_center()),
                ClientEdge::Below => (srv_anchor.center_bottom(), rect.center_top()),
                ClientEdge::Above => (srv_anchor.center_top(), rect.center_bottom()),
            };
            painter.line_segment(
                [p1, p2],
                Stroke::new(
                    if is_dragging { 2.0 } else { 1.5 },
                    Color32::from_rgba_premultiplied(
                        0,
                        150,
                        255,
                        if is_dragging { 200 } else { 100 },
                    ),
                ),
            );

            let fill = if is_dragging {
                Color32::from_rgba_premultiplied(0, 150, 255, 40)
            } else {
                Color32::from_rgba_premultiplied(0, 120, 200, 20)
            };
            let stroke_col = if is_dragging {
                Color32::from_rgb(0, 200, 255)
            } else {
                Color32::from_rgb(0, 150, 255)
            };

            painter.rect(
                rect,
                Rounding::same(12.0),
                fill,
                Stroke::new(if is_dragging { 2.5 } else { 1.5 }, stroke_col),
            );

            let ping = pings.get(slot).copied().unwrap_or(0);

            painter.text(
                rect.center() - Vec2::new(0.0, 16.0),
                egui::Align2::CENTER_CENTER,
                name,
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );

            painter.text(
                rect.center() + Vec2::new(0.0, 2.0),
                egui::Align2::CENTER_CENTER,
                format!("{}×{}", p.client_width, p.client_height),
                egui::FontId::proportional(11.0),
                Color32::LIGHT_GRAY,
            );

            // Draw Ping Dot explicitly on canvas rect
            let ping_color = if ping < 20 {
                Color32::from_rgb(50, 220, 100)
            } else if ping < 60 {
                Color32::from_rgb(255, 200, 50)
            } else {
                Color32::from_rgb(255, 80, 80)
            };
            let ping_pos = rect.center() + Vec2::new(0.0, 20.0);
            painter.circle_filled(ping_pos - Vec2::new(20.0, 0.0), 3.0, ping_color);
            painter.text(
                ping_pos,
                egui::Align2::LEFT_CENTER,
                format!("{} ms", ping),
                egui::FontId::proportional(11.0),
                ping_color,
            );

            if is_dragging {
                painter.text(
                    rect.center_top() - Vec2::new(0.0, 16.0),
                    egui::Align2::CENTER_CENTER,
                    "✥",
                    egui::FontId::proportional(18.0),
                    Color32::WHITE,
                );
            }
        }

        // Drag-release
        if canvas_resp.drag_stopped() {
            if let Some(slot) = self.layout_drag.dragging_slot.take() {
                let snap_pos = self.layout_drag.drag_screen_pos;
                let new_edge = closest_snap_edge(snap_pos, srv_anchor);
                let (cw, ch) = layout
                    .placements
                    .get(&slot)
                    .map(|p| (p.client_width, p.client_height))
                    .unwrap_or((1920, 1080));
                layout
                    .placements
                    .retain(|k, p| *k == slot || p.edge != new_edge);
                layout.placements.insert(
                    slot,
                    Placement {
                        edge: new_edge,
                        client_width: cw,
                        client_height: ch,
                    },
                );
                layout_changed = true;
            }
        }

        if clients.is_empty() {
            painter.text(
                canvas_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for devices...",
                egui::FontId::proportional(16.0),
                Color32::from_gray(80),
            );
        }

        if layout_changed {
            layout.save();
            handle.set_layout(layout);
        }
    }

    fn render_secondary_screen_dashboard(&mut self, ui: &mut egui::Ui) {
        let (server_name, return_edge, sw, sh) = match &self.services.client {
            Some(c) => {
                let s = c.state.lock().unwrap_or_else(|e| e.into_inner());
                (
                    s.server_name.clone(),
                    s.return_edge,
                    s.screen_width,
                    s.screen_height,
                )
            }
            None => return,
        };

        ui.horizontal(|ui| {
            ui.heading(
                RichText::new("Secondary Monitor")
                    .size(24.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new("● Connected")
                        .color(Color32::from_rgb(50, 220, 100))
                        .strong(),
                );
            });
        });
        ui.label(
            RichText::new(format!("Acting as an extended display for  {server_name}"))
                .color(Color32::GRAY),
        );
        ui.add_space(20.0);

        let canvas_size = Vec2::new(ui.available_width(), 340.0);
        let (canvas_rect, _) = ui.allocate_exact_size(canvas_size, Sense::hover());
        let painter = ui.painter_at(canvas_rect);

        painter.rect_filled(
            canvas_rect,
            Rounding::same(16.0),
            Color32::from_rgb(18, 20, 24),
        );
        painter.rect_stroke(
            canvas_rect,
            Rounding::same(16.0),
            Stroke::new(1.0, Color32::from_rgb(32, 34, 40)),
        );

        let center = canvas_rect.center();
        let srv_sz = Vec2::new(180.0, 110.0);
        let cli_sz = Vec2::new(150.0, 90.0);
        let gap = 110.0_f32;

        let (srv_center, cli_center) = match return_edge {
            Some(ClientEdge::Left) | None => (
                egui::pos2(center.x - gap - srv_sz.x / 2.0, center.y),
                egui::pos2(center.x + gap / 2.0, center.y),
            ),
            Some(ClientEdge::Right) => (
                egui::pos2(center.x + gap + srv_sz.x / 2.0, center.y),
                egui::pos2(center.x - gap / 2.0, center.y),
            ),
            Some(ClientEdge::Above) => (
                egui::pos2(center.x, center.y - gap - srv_sz.y / 2.0),
                egui::pos2(center.x, center.y + gap / 2.0),
            ),
            Some(ClientEdge::Below) => (
                egui::pos2(center.x, center.y + gap + srv_sz.y / 2.0),
                egui::pos2(center.x, center.y - gap / 2.0),
            ),
        };

        let srv_rect = Rect::from_center_size(srv_center, srv_sz);
        let cli_rect = Rect::from_center_size(cli_center, cli_sz);

        let (p1, p2) = match return_edge {
            Some(ClientEdge::Left) | None => (srv_rect.right_center(), cli_rect.left_center()),
            Some(ClientEdge::Right) => (srv_rect.left_center(), cli_rect.right_center()),
            Some(ClientEdge::Above) => (srv_rect.center_bottom(), cli_rect.center_top()),
            Some(ClientEdge::Below) => (srv_rect.center_top(), cli_rect.center_bottom()),
        };
        painter.line_segment(
            [p1, p2],
            Stroke::new(2.0, Color32::from_rgba_premultiplied(0, 150, 255, 120)),
        );

        painter.rect(
            srv_rect,
            Rounding::same(12.0),
            Color32::from_rgb(25, 40, 60),
            Stroke::new(2.0, Color32::from_rgb(0, 150, 255)),
        );
        painter.text(
            srv_rect.center() - Vec2::new(0.0, 12.0),
            egui::Align2::CENTER_CENTER,
            &server_name,
            egui::FontId::proportional(14.0),
            Color32::WHITE,
        );
        painter.text(
            srv_rect.center() + Vec2::new(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            "PRIMARY",
            egui::FontId::proportional(11.0),
            Color32::from_rgb(0, 150, 255),
        );

        painter.rect(
            cli_rect,
            Rounding::same(12.0),
            Color32::from_rgba_premultiplied(0, 150, 255, 30),
            Stroke::new(2.0, Color32::from_rgb(0, 200, 255)),
        );
        painter.text(
            cli_rect.center() - Vec2::new(0.0, 12.0),
            egui::Align2::CENTER_CENTER,
            &gethostname(),
            egui::FontId::proportional(14.0),
            Color32::WHITE,
        );
        painter.text(
            cli_rect.center() + Vec2::new(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            format!("{}×{}", sw, sh),
            egui::FontId::proportional(11.0),
            Color32::LIGHT_GRAY,
        );

        let hint = match return_edge {
            Some(ClientEdge::Left) => "← Move cursor left to return to primary",
            Some(ClientEdge::Right) => "Move cursor right to return to primary →",
            Some(ClientEdge::Above) => "↑ Move cursor up to return to primary",
            Some(ClientEdge::Below) => "Move cursor down to return to primary ↓",
            None => "Waiting for cursor handoff from primary...",
        };
        painter.text(
            canvas_rect.center_bottom() - Vec2::new(0.0, 20.0),
            egui::Align2::CENTER_CENTER,
            hint,
            egui::FontId::proportional(14.0),
            Color32::from_gray(140),
        );
    }

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading(
            RichText::new("Configuration")
                .size(24.0)
                .strong()
                .color(Color32::WHITE),
        );
        ui.add_space(20.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::none()
                .fill(Color32::from_rgb(18, 20, 24))
                .rounding(Rounding::same(12.0))
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Machine Identity")
                            .strong()
                            .color(Color32::from_rgb(0, 150, 255)),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Hostname:").color(Color32::GRAY));
                        ui.label(gethostname());
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Bind Addr:").color(Color32::GRAY));
                        ui.label(&self.bind_addr);
                    });
                });

            ui.add_space(16.0);

            egui::Frame::none()
                .fill(Color32::from_rgb(18, 20, 24))
                .rounding(Rounding::same(12.0))
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Audio and Sharing")
                            .strong()
                            .color(Color32::from_rgb(0, 150, 255)),
                    );
                    ui.add_space(10.0);
                    if let Some(handle) = &self.services.server {
                        let mut audio_enabled = !handle.is_mic_muted();
                        if ui
                            .checkbox(
                                &mut audio_enabled,
                                "Share desktop audio with the connected device",
                            )
                            .changed()
                        {
                            handle.set_mic_muted(!audio_enabled);
                            log_info(if audio_enabled {
                                "Audio enabled"
                            } else {
                                "Audio disabled"
                            });
                        }

                        ui.add_space(12.0);
                        ui.label(RichText::new("Output Device:").color(Color32::GRAY));
                        ui.add_space(4.0);

                        self.output_devices = handle.output_devices();
                        let current = handle.selected_output_device().unwrap_or_default();
                        if self.output_devices.is_empty() && !current.is_empty() {
                            self.output_devices.insert(0, current.clone());
                        }
                        if self.selected_output_device.is_empty() && !current.is_empty() {
                            self.selected_output_device = current;
                        }

                        let mut selected_idx: usize = self
                            .output_devices
                            .iter()
                            .position(|d| d == &self.selected_output_device)
                            .unwrap_or(0);

                        egui::ComboBox::from_id_source("output_device")
                            .selected_text(&self.output_devices[selected_idx])
                            .show_ui(ui, |ui: &mut egui::Ui| {
                                for (i, name) in self.output_devices.iter().enumerate() {
                                    ui.selectable_value(&mut selected_idx, i, name);
                                }
                            });

                        if selected_idx
                            != self
                                .output_devices
                                .iter()
                                .position(|d| d == &self.selected_output_device)
                                .unwrap_or(0)
                        {
                            self.selected_output_device = self.output_devices[selected_idx].clone();
                            handle.set_output_device(self.selected_output_device.clone());
                        }
                    }
                });

            ui.add_space(16.0);

            if ui
                .button(
                    RichText::new("⟳ Restart Background Service")
                        .color(Color32::from_rgb(255, 100, 100)),
                )
                .clicked()
            {
                self.start_server();
            }
        });
    }

    fn render_logs(&mut self, ui: &mut egui::Ui) {
        ui.heading(
            RichText::new("System Logs")
                .size(24.0)
                .strong()
                .color(Color32::WHITE),
        );
        ui.add_space(20.0);

        egui::Frame::none()
            .fill(Color32::from_rgb(12, 14, 18))
            .rounding(Rounding::same(12.0))
            .inner_margin(16.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .max_height(f32::INFINITY)
                    .show(ui, |ui| {
                        let logs = GLOBAL_LOGS.lock().unwrap_or_else(|e| e.into_inner());
                        for entry in &logs.entries {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(&entry.time)
                                        .color(Color32::DARK_GRAY)
                                        .monospace()
                                        .size(12.0),
                                );
                                let color = match entry.level.as_str() {
                                    "ERROR" => Color32::from_rgb(255, 80, 80),
                                    "WARN" => Color32::from_rgb(255, 200, 50),
                                    _ => Color32::from_rgb(0, 150, 255),
                                };
                                ui.label(
                                    RichText::new(format!("[{}]", entry.level))
                                        .color(color)
                                        .monospace()
                                        .size(12.0),
                                );
                                ui.label(
                                    RichText::new(&entry.message)
                                        .color(Color32::from_rgb(200, 200, 200))
                                        .monospace()
                                        .size(12.0),
                                );
                            });
                        }
                    });
            });
    }
}

// ── Log Helpers ─────────────────────────────────────────────────────────────

fn log_info(msg: &str) {
    GLOBAL_LOGS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .add("INFO", msg);
}
fn log_error(msg: &str) {
    GLOBAL_LOGS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .add("ERROR", msg);
}

fn gethostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "netshare-node".to_owned())
}
