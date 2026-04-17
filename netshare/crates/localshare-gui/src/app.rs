use std::collections::HashMap;
use egui::{Color32, Context, Frame, Margin, Painter, Rect, RichText, Sense, Stroke, Vec2, vec2};
use serde::{Deserialize, Serialize};

use crate::theme;
use crate::views;

// ── Pages ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Page { Monitor, Setting }

// ── Monitor state ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub slot:       Option<u8>,
    pub label:      String,
    pub host:       String,
    pub resolution: String,
    pub hz:         u32,
    pub connected:  bool,
    pub active:     bool,
    pub pos:        egui::Vec2,
    pub anim_pos:   egui::Vec2,
    pub size:       egui::Vec2,
}

#[derive(Default)]
pub struct SnapGuides {
    pub h: Option<f32>,
    pub v: Option<f32>,
}

// ── Settings ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub share_audio:  bool,
    pub audio_device: usize,
    pub share_input:  bool,
    pub dark_mode:    bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { share_audio: false, audio_device: 0, share_input: true, dark_mode: true }
    }
}

// ── App ────────────────────────────────────────────────────────────────────

pub struct LocalShareApp {
    pub page:        Page,
    pub settings:    Settings,
    pub monitors:    Vec<MonitorInfo>,
    pub audio_devs:  Vec<String>,
    pub snap_guides: SnapGuides,
    pub dragging_id: Option<usize>,
    /// `false` until the primary monitor has been auto-centered (or the user
    /// drags it). Lets the canvas re-center on window resize before the user
    /// touches anything.
    pub primary_placed: bool,
    /// Last canvas size we centered for — if it changes and user hasn't
    /// dragged, re-center.
    pub last_canvas_size: egui::Vec2,
    prev_dark:       bool,
    pub bridge:      crate::bridge::ServerBridge,
}

impl LocalShareApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, "settings"))
            .unwrap_or_default();

        // ── Load font ────────────────────────────────────────────────────
        let mut fonts = egui::FontDefinitions::default();
        for path in [
            "C:\\Windows\\Fonts\\segoeui.ttf",
            "C:\\Windows\\Fonts\\segoeui.TTF",
        ] {
            if let Ok(data) = std::fs::read(path) {
                fonts.font_data.insert(
                    "ui_font".into(),
                    egui::FontData::from_owned(data),
                );
                if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                    list.insert(0, "ui_font".into());
                }
                break;
            }
        }
        cc.egui_ctx.set_fonts(fonts);

        theme::apply(&cc.egui_ctx, settings.dark_mode);
        let prev_dark = settings.dark_mode;

        let bridge = crate::bridge::ServerBridge::start();
        let audio_devs = if bridge.audio_devices.is_empty() {
            vec![
                "Speakers (Realtek HD Audio)".into(),
                "Headphones (USB Audio Device)".into(),
            ]
        } else {
            bridge.audio_devices.clone()
        };

        // Query the real primary display. Fall back to "Unknown" if platform
        // query isn't available yet (non-Windows).
        let (resolution, hz) = crate::display::primary()
            .map(|d| (d.formatted(), d.hz))
            .unwrap_or_else(|| ("Unknown".into(), 0));

        let monitors = vec![
            MonitorInfo {
                slot: None,
                label: "Monitor 1".into(),
                host:  local_hostname(),
                resolution, hz,
                connected: true, active: true,
                pos: vec2(4.0, 42.0), anim_pos: vec2(4.0, 42.0),
                size: vec2(86.0, 58.0),
            },
        ];

        Self {
            page: Page::Monitor, settings, monitors, audio_devs,
            snap_guides: SnapGuides::default(),
            dragging_id: None,
            primary_placed: false,
            last_canvas_size: vec2(0.0, 0.0),
            prev_dark,
            bridge,
        }
    }

    // ── Palette helpers ────────────────────────────────────────────────────
    pub fn accent     (&self) -> Color32 { pick(self.settings.dark_mode, theme::dark::ACCENT,     theme::light::ACCENT)     }
    pub fn accent_bg  (&self) -> Color32 { tint(self.accent(), if self.settings.dark_mode { 30 } else { 24 }) }
    pub fn text       (&self) -> Color32 { pick(self.settings.dark_mode, theme::dark::TEXT,       theme::light::TEXT)       }
    pub fn text_muted (&self) -> Color32 { pick(self.settings.dark_mode, theme::dark::TEXT_MUTED, theme::light::TEXT_MUTED) }
    pub fn text_subtle(&self) -> Color32 { pick(self.settings.dark_mode, theme::dark::TEXT_SUBTLE,theme::light::TEXT_SUBTLE)}
    pub fn bg         (&self) -> Color32 { pick(self.settings.dark_mode, theme::dark::BG,         theme::light::BG)         }
    pub fn bg_panel   (&self) -> Color32 { pick(self.settings.dark_mode, theme::dark::BG_PANEL,   theme::light::BG_PANEL)   }
    pub fn bg_raised  (&self) -> Color32 { pick(self.settings.dark_mode, theme::dark::BG_RAISED,  theme::light::BG_RAISED)  }
    pub fn separator  (&self) -> Color32 { pick(self.settings.dark_mode, theme::dark::SEPARATOR,  theme::light::SEPARATOR)  }
    pub fn green      (&self) -> Color32 { pick(self.settings.dark_mode, theme::dark::GREEN,      theme::light::GREEN)      }

    pub fn sync_from_server(&mut self) {
        let snapshot = self.bridge.state.snapshot();
        let discovered = self.bridge.discovered.lock().unwrap().clone();

        // Remember existing positions keyed by host name (stable across TCP
        // reconnects and slot reassignments).
        let mut host_pos: HashMap<String, (egui::Vec2, egui::Vec2)> = HashMap::new();
        for mon in self.monitors.iter().skip(1) {
            host_pos.insert(mon.host.clone(), (mon.pos, mon.anim_pos));
        }

        let primary = self.monitors[0].clone();
        let mut new_mons = vec![primary];
        let mut seen_hosts: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 1) Connected TCP clients (real slots, can switch input to them).
        for (i, snap) in snapshot.iter().enumerate() {
            let (pos, anim_pos) = host_pos.get(&snap.name).copied().unwrap_or_else(|| {
                let last = new_mons.last().unwrap();
                let p = egui::vec2(last.pos.x + last.size.x, last.pos.y);
                (p, p)
            });
            seen_hosts.insert(snap.name.clone());
            new_mons.push(MonitorInfo {
                slot:       Some(snap.slot),
                label:      format!("Monitor {}", i + 2),
                host:       snap.name.clone(),
                resolution: "Connected".into(),
                hz:         60,
                connected:  true,
                active:     snap.is_active,
                pos,
                anim_pos,
                size:       egui::vec2(76.0, 52.0),
            });
        }

        // 2) Discovered-via-mDNS-but-not-TCP-connected peers. Show them as
        //    offline placeholders so the user sees "yes, we see the other PC,
        //    just couldn't finish the handshake (likely firewall)".
        for peer in discovered.iter() {
            if seen_hosts.contains(&peer.name) { continue; }
            let (pos, anim_pos) = host_pos.get(&peer.name).copied().unwrap_or_else(|| {
                let last = new_mons.last().unwrap();
                let p = egui::vec2(last.pos.x + last.size.x, last.pos.y);
                (p, p)
            });
            let label = format!("Monitor {}", new_mons.len() + 1);
            // Auto-connecting: bridge.rs spawned a dial_peer task for this
            // host as soon as mDNS saw it. It becomes a real slot (below)
            // once the TCP handshake completes on the peer's side and they
            // dial us back. Surface that as "Connecting…" so the user sees
            // progress instead of a bare "Offline".
            new_mons.push(MonitorInfo {
                slot:       None, // no TCP slot yet
                label,
                host:       peer.name.clone(),
                resolution: format!("{} · Connecting…", peer.addr),
                hz:         0,
                connected:  false,
                active:     false,
                pos,
                anim_pos,
                size:       egui::vec2(76.0, 52.0),
            });
        }

        let active_slot = self.bridge.state.active_slot();
        new_mons[0].active = active_slot == 0;

        // Replace when the set of hosts changed; otherwise just refresh flags
        // in place so drag positions don't get reset every frame.
        let old_hosts: Vec<String> = self.monitors.iter().skip(1).map(|m| m.host.clone()).collect();
        let new_hosts: Vec<String> = new_mons.iter().skip(1).map(|m| m.host.clone()).collect();
        if old_hosts != new_hosts {
            self.monitors = new_mons;
        } else {
            for mon in self.monitors.iter_mut().skip(1) {
                mon.active = snapshot.iter()
                    .any(|snap| snap.name == mon.host && snap.is_active);
                mon.connected = snapshot.iter().any(|snap| snap.name == mon.host);
            }
            self.monitors[0].active = active_slot == 0;
        }
    }
}

#[inline] fn pick(dark: bool, d: Color32, l: Color32) -> Color32 { if dark { d } else { l } }
#[inline] fn tint(c: Color32, a: u8) -> Color32 { Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a) }

fn local_hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "This PC".into())
}

// ── eframe::App ────────────────────────────────────────────────────────────

impl eframe::App for LocalShareApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "settings", &self.settings);
    }

    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.sync_from_server();

        self.bridge.share_input.store(self.settings.share_input, std::sync::atomic::Ordering::Relaxed);
        self.bridge.set_audio_enabled(self.settings.share_audio);

        if self.settings.dark_mode != self.prev_dark {
            theme::apply(ctx, self.settings.dark_mode);
            self.prev_dark = self.settings.dark_mode;
        }

        egui::SidePanel::left("sidebar")
            .exact_width(SIDEBAR_W)
            .resizable(false)
            .frame(
                Frame::none()
                    .fill(self.bg_panel())
                    .stroke(Stroke::new(1.0, self.separator())),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing   = vec2(0.0, 0.0);
                ui.spacing_mut().window_margin  = egui::Margin::same(0.0);
                self.draw_sidebar(ui);
            });

        egui::CentralPanel::default()
            .frame(Frame::none().fill(self.bg()))
            .show(ctx, |ui| {
                match self.page {
                    Page::Monitor => views::monitor::draw(ui, self),
                    Page::Setting => views::setting::draw(ui, self),
                }
            });
    }
}

// ── Sidebar ────────────────────────────────────────────────────────────────

const SIDEBAR_W: f32 = 62.0;

impl LocalShareApp {
    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        let full_h = ui.available_height();
        ui.allocate_ui(vec2(SIDEBAR_W, full_h), |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(14.0);
                self.draw_logo(ui);
                ui.add_space(20.0);

                self.nav_item(ui, Page::Monitor, "Monitor");
                ui.add_space(4.0);
                self.nav_item(ui, Page::Setting, "Setting");

                let remaining = ui.available_height() - 38.0;
                if remaining > 0.0 { ui.add_space(remaining); }
                self.theme_toggle_btn(ui);
                ui.add_space(10.0);
            });
        });
    }

    fn draw_logo(&self, ui: &mut egui::Ui) {
        let (r, p) = ui.allocate_painter(vec2(28.0, 28.0), Sense::hover());
        let c = r.rect.center();
        p.rect_filled(r.rect, 7.0, self.accent_bg());
        let mon = Rect::from_center_size(c + vec2(0.0, -2.0), vec2(14.0, 9.5));
        let s = Stroke::new(1.5, self.accent());
        p.rect_stroke(mon, 1.5, s);
        p.line_segment([c + vec2(0.0, 2.75), c + vec2(0.0, 5.0)],  s);
        p.line_segment([c + vec2(-3.5, 5.0), c + vec2(3.5, 5.0)],  s);
    }

    fn nav_item(&mut self, ui: &mut egui::Ui, page: Page, label: &str) {
        let active   = self.page == page;
        let accent   = self.accent();
        let icon_col = if active { accent } else { self.text_muted() };
        let fill     = if active { self.accent_bg() } else { Color32::TRANSPARENT };

        let item_w = SIDEBAR_W - 8.0;
        let (resp, p) = ui.allocate_painter(vec2(item_w, 52.0), Sense::click());
        let r = resp.rect;

        let bg = if resp.hovered() && !active { self.bg_raised() } else { fill };
        p.rect_filled(r, 8.0, bg);

        if active {
            let bar_y  = r.center().y;
            let bar_h  = 14.0;
            let bar_x  = r.left() - (SIDEBAR_W - item_w) / 2.0;
            p.line_segment(
                [egui::pos2(bar_x, bar_y - bar_h), egui::pos2(bar_x, bar_y + bar_h)],
                Stroke::new(3.0, accent),
            );
        }

        let icon_rect = Rect::from_center_size(r.center() - vec2(0.0, 7.0), vec2(18.0, 18.0));
        draw_icon(&p, page, icon_col, icon_rect);

        p.text(
            r.center() + vec2(0.0, 12.0),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::new(9.5, egui::FontFamily::Proportional),
            icon_col,
        );

        if resp.clicked() { self.page = page; }
        if resp.hovered() { ui.ctx().request_repaint(); }
    }

    fn theme_toggle_btn(&mut self, ui: &mut egui::Ui) {
        let (resp, p) = ui.allocate_painter(vec2(SIDEBAR_W - 8.0, 28.0), Sense::click());
        let r = resp.rect;
        if resp.hovered() { p.rect_filled(r, 6.0, self.bg_raised()); }
        let icon = if self.settings.dark_mode { "☀" } else { "🌙" };
        p.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::new(14.0, egui::FontFamily::Proportional),
            self.text_muted(),
        );
        if resp.clicked() { self.settings.dark_mode = !self.settings.dark_mode; }
        if resp.hovered() { ui.ctx().request_repaint(); }
    }
}

// ── Icon drawing ───────────────────────────────────────────────────────────

fn draw_icon(p: &Painter, page: Page, color: Color32, r: Rect) {
    let c  = r.center();
    let st = Stroke::new(1.6, color);
    match page {
        Page::Monitor => {
            let mon = Rect::from_center_size(c - vec2(0.0, 2.0), vec2(14.0, 9.5));
            p.rect_stroke(mon, 1.5, st);
            p.line_segment([c + vec2(0.0, 2.75), c + vec2(0.0, 5.5)],  st);
            p.line_segment([c + vec2(-3.5, 5.5), c + vec2(3.5, 5.5)],  st);
        }
        Page::Setting => {
            p.circle_stroke(c, 3.4, st);
            for deg in (0..360_i32).step_by(45) {
                let rad   = (deg as f32).to_radians();
                let inner = c + vec2(rad.cos(), rad.sin()) * 3.4;
                let outer = c + vec2(rad.cos(), rad.sin()) * 6.2;
                p.line_segment([inner, outer], st);
            }
        }
    }
}

// ── Shared helpers for views ───────────────────────────────────────────────

pub fn section_label(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).size(10.0).color(color).strong());
    ui.add_space(4.0);
}
