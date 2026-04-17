use egui::{Color32, FontFamily, FontId, Rounding, Stroke, Style, Visuals};

// ── Palette ────────────────────────────────────────────────────────────────
//
// Neutrals are tinted slightly cool (hue ~220°) per impeccable guidelines.
// Accent is muted steel-blue — not vivid, not neon.

pub mod dark {
    use super::*;

    pub const BG:          Color32 = Color32::from_rgb(14,  15,  18);   // near-black, cool tint
    pub const BG_PANEL:    Color32 = Color32::from_rgb(20,  22,  28);
    pub const BG_RAISED:   Color32 = Color32::from_rgb(28,  31,  39);
    pub const BG_HOVER:    Color32 = Color32::from_rgb(36,  40,  52);
    pub const SEPARATOR:   Color32 = Color32::from_rgb(38,  42,  54);

    pub const TEXT:        Color32 = Color32::from_rgb(220, 222, 228);  // not pure white
    pub const TEXT_MUTED:  Color32 = Color32::from_rgb(110, 116, 134);
    pub const TEXT_SUBTLE: Color32 = Color32::from_rgb(66,  72,  90);

    pub const ACCENT:      Color32 = Color32::from_rgb(82,  130, 195);  // muted steel-blue
    pub const ACCENT_DIM:  Color32 = Color32::from_rgb(44,  78,  128);

    pub const GREEN:       Color32 = Color32::from_rgb(74,  179, 120);
    pub const AMBER:       Color32 = Color32::from_rgb(210, 155, 64);
    pub const RED:         Color32 = Color32::from_rgb(210, 80,  80);
}

pub mod light {
    use super::*;

    pub const BG:          Color32 = Color32::from_rgb(247, 248, 250);
    pub const BG_PANEL:    Color32 = Color32::from_rgb(255, 255, 255);
    pub const BG_RAISED:   Color32 = Color32::from_rgb(242, 244, 248);
    pub const BG_HOVER:    Color32 = Color32::from_rgb(234, 237, 245);
    pub const SEPARATOR:   Color32 = Color32::from_rgb(222, 226, 236);

    pub const TEXT:        Color32 = Color32::from_rgb(22,  24,  32);
    pub const TEXT_MUTED:  Color32 = Color32::from_rgb(100, 108, 130);
    pub const TEXT_SUBTLE: Color32 = Color32::from_rgb(170, 178, 200);

    pub const ACCENT:      Color32 = Color32::from_rgb(52,  110, 180);
    pub const ACCENT_DIM:  Color32 = Color32::from_rgb(180, 207, 240);

    pub const GREEN:       Color32 = Color32::from_rgb(34,  160, 90);
    pub const AMBER:       Color32 = Color32::from_rgb(190, 135, 30);
    pub const RED:         Color32 = Color32::from_rgb(195, 55,  55);
}

/// Status dot colors — same meaning in both themes.
#[derive(Clone, Copy)]
pub enum StatusColor { Green, Amber, Red, Muted }

impl StatusColor {
    pub fn to_color32(self, is_dark: bool) -> Color32 {
        match (self, is_dark) {
            (Self::Green, true)  => dark::GREEN,
            (Self::Green, false) => light::GREEN,
            (Self::Amber, true)  => dark::AMBER,
            (Self::Amber, false) => light::AMBER,
            (Self::Red,   true)  => dark::RED,
            (Self::Red,   false) => light::RED,
            (Self::Muted, true)  => dark::TEXT_SUBTLE,
            (Self::Muted, false) => light::TEXT_SUBTLE,
        }
    }
}

/// Apply LocalShare visuals to a context. Call once at startup and on theme toggle.
pub fn apply(ctx: &egui::Context, dark_mode: bool) {
    let mut style = Style::default();

    // ── Typography ──────────────────────────────────────────────────────────
    style.text_styles = [
        (egui::TextStyle::Small,   FontId::new(10.5, FontFamily::Proportional)),
        (egui::TextStyle::Body,    FontId::new(12.0, FontFamily::Proportional)),
        (egui::TextStyle::Button,  FontId::new(12.0, FontFamily::Proportional)),
        (egui::TextStyle::Heading, FontId::new(14.5, FontFamily::Proportional)),
        (egui::TextStyle::Monospace, FontId::new(11.5, FontFamily::Monospace)),
    ]
    .into();

    // ── Spacing ─────────────────────────────────────────────────────────────
    style.spacing.item_spacing    = egui::vec2(6.0, 4.0);
    style.spacing.button_padding  = egui::vec2(10.0, 5.0);
    style.spacing.indent          = 16.0;
    style.spacing.window_margin   = egui::Margin::same(0.0);

    // ── Rounding ─────────────────────────────────────────────────────────────
    style.visuals.window_rounding       = Rounding::same(8.0);
    style.visuals.widgets.noninteractive.rounding = Rounding::same(5.0);
    style.visuals.widgets.inactive.rounding       = Rounding::same(5.0);
    style.visuals.widgets.hovered.rounding        = Rounding::same(5.0);
    style.visuals.widgets.active.rounding         = Rounding::same(5.0);
    style.visuals.widgets.open.rounding           = Rounding::same(5.0);
    style.visuals.menu_rounding                   = Rounding::same(6.0);

    if dark_mode {
        apply_dark(&mut style.visuals);
    } else {
        apply_light(&mut style.visuals);
    }

    ctx.set_style(style);
}

fn apply_dark(v: &mut Visuals) {
    v.dark_mode = true;

    v.window_fill               = dark::BG_PANEL;
    v.panel_fill                = dark::BG;
    v.faint_bg_color            = dark::BG_RAISED;
    v.extreme_bg_color          = dark::BG;
    v.window_stroke             = Stroke::new(1.0, dark::SEPARATOR);

    v.override_text_color       = Some(dark::TEXT);
    v.hyperlink_color           = dark::ACCENT;
    v.selection.bg_fill         = dark::ACCENT_DIM;
    v.selection.stroke          = Stroke::new(1.0, dark::ACCENT);

    v.widgets.noninteractive.bg_fill   = dark::BG_RAISED;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, dark::SEPARATOR);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, dark::TEXT_MUTED);

    v.widgets.inactive.bg_fill         = dark::BG_RAISED;
    v.widgets.inactive.bg_stroke       = Stroke::new(1.0, dark::SEPARATOR);
    v.widgets.inactive.fg_stroke       = Stroke::new(1.5, dark::TEXT);
    v.widgets.inactive.weak_bg_fill    = dark::BG_RAISED;

    v.widgets.hovered.bg_fill          = dark::BG_HOVER;
    v.widgets.hovered.bg_stroke        = Stroke::new(1.0, dark::ACCENT_DIM);
    v.widgets.hovered.fg_stroke        = Stroke::new(1.5, dark::TEXT);
    v.widgets.hovered.weak_bg_fill     = dark::BG_HOVER;

    v.widgets.active.bg_fill           = dark::ACCENT_DIM;
    v.widgets.active.bg_stroke         = Stroke::new(1.0, dark::ACCENT);
    v.widgets.active.fg_stroke         = Stroke::new(2.0, dark::TEXT);
    v.widgets.active.weak_bg_fill      = dark::ACCENT_DIM;

    v.widgets.open.bg_fill             = dark::BG_HOVER;
    v.widgets.open.bg_stroke           = Stroke::new(1.0, dark::ACCENT);

    v.popup_shadow                     = egui::Shadow::NONE;
    v.window_shadow                    = egui::Shadow::NONE;
}

fn apply_light(v: &mut Visuals) {
    v.dark_mode = false;

    v.window_fill               = light::BG_PANEL;
    v.panel_fill                = light::BG;
    v.faint_bg_color            = light::BG_RAISED;
    v.extreme_bg_color          = light::BG;
    v.window_stroke             = Stroke::new(1.0, light::SEPARATOR);

    v.override_text_color       = Some(light::TEXT);
    v.hyperlink_color           = light::ACCENT;
    v.selection.bg_fill         = light::ACCENT_DIM;
    v.selection.stroke          = Stroke::new(1.0, light::ACCENT);

    v.widgets.noninteractive.bg_fill   = light::BG_RAISED;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, light::SEPARATOR);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, light::TEXT_MUTED);

    v.widgets.inactive.bg_fill         = light::BG_PANEL;
    v.widgets.inactive.bg_stroke       = Stroke::new(1.0, light::SEPARATOR);
    v.widgets.inactive.fg_stroke       = Stroke::new(1.5, light::TEXT);
    v.widgets.inactive.weak_bg_fill    = light::BG_RAISED;

    v.widgets.hovered.bg_fill          = light::BG_HOVER;
    v.widgets.hovered.bg_stroke        = Stroke::new(1.0, light::ACCENT);
    v.widgets.hovered.fg_stroke        = Stroke::new(1.5, light::TEXT);
    v.widgets.hovered.weak_bg_fill     = light::BG_HOVER;

    v.widgets.active.bg_fill           = light::ACCENT_DIM;
    v.widgets.active.bg_stroke         = Stroke::new(1.0, light::ACCENT);
    v.widgets.active.fg_stroke         = Stroke::new(2.0, light::TEXT);
    v.widgets.active.weak_bg_fill      = light::ACCENT_DIM;

    v.widgets.open.bg_fill             = light::BG_HOVER;
    v.widgets.open.bg_stroke           = Stroke::new(1.0, light::ACCENT);

    v.popup_shadow                     = egui::Shadow::NONE;
    v.window_shadow                    = egui::Shadow::NONE;
}
