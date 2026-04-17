use egui::{Frame, Margin, RichText, Sense, Stroke, vec2};

use crate::app::{section_label, LocalShareApp};

pub fn draw(ui: &mut egui::Ui, app: &mut LocalShareApp) {
    draw_topbar(ui, app);

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            Frame::none()
                .inner_margin(Margin::symmetric(14.0, 10.0))
                .show(ui, |ui| {
                    section_label(ui, "AUDIO", app.text_muted());
                    draw_audio_group(ui, app);

                    section_label(ui, "INPUT", app.text_muted());
                    draw_input_group(ui, app);
                });
        });
}

fn draw_topbar(ui: &mut egui::Ui, app: &LocalShareApp) {
    let panel_bg = app.bg_panel();
    let sep      = app.separator();

    Frame::none()
        .fill(panel_bg)
        .inner_margin(Margin { left: 14.0, right: 14.0, top: 12.0, bottom: 10.0 })
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Setting").size(13.5).strong().color(app.text()));
                ui.label(
                    RichText::new("Configure audio & sharing")
                        .size(10.5)
                        .color(app.text_muted()),
                );
            });
        });

    ui.painter().hline(
        ui.max_rect().x_range(),
        ui.cursor().top(),
        Stroke::new(1.0, sep),
    );
}

fn draw_audio_group(ui: &mut egui::Ui, app: &mut LocalShareApp) {
    let sep     = app.separator();
    let card_bg = app.bg_raised();
    let text_c  = app.text();
    let muted   = app.text_muted();

    Frame::none()
        .fill(card_bg)
        .rounding(egui::Rounding::same(9.0))
        .stroke(Stroke::new(1.0, sep))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                Frame::none()
                    .inner_margin(Margin::symmetric(14.0, 11.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new("Share this device audio")
                                        .size(12.5)
                                        .strong()
                                        .color(text_c),
                                );
                                ui.label(
                                    RichText::new("Stream audio output to connected devices")
                                        .size(10.5)
                                        .color(muted),
                                );
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                toggle_switch(ui, app, |a| &mut a.settings.share_audio);
                            });
                        });
                    });

                if app.settings.share_audio {
                    ui.painter().hline(
                        ui.max_rect().x_range(),
                        ui.cursor().top(),
                        Stroke::new(1.0, sep),
                    );

                    Frame::none()
                        .inner_margin(Margin { left: 14.0, right: 14.0, top: 10.0, bottom: 12.0 })
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("Output device")
                                    .size(11.0)
                                    .strong()
                                    .color(muted),
                            );
                            ui.add_space(5.0);
                            draw_device_dropdown(ui, app);
                        });
                }
            });
        });
}

fn draw_device_dropdown(ui: &mut egui::Ui, app: &mut LocalShareApp) {
    let sep    = app.separator();
    let text_c = app.text();
    let muted  = app.text_muted();
    let accent = app.accent();
    let bg     = app.bg_panel();

    let selected_name = app.audio_devs
        .get(app.settings.audio_device)
        .cloned()
        .unwrap_or_default();

    let row_h = 34.0;
    let avail_w = ui.available_width();

    Frame::none()
        .fill(bg)
        .rounding(egui::Rounding::same(7.0))
        .stroke(Stroke::new(1.0, sep))
        .inner_margin(Margin::symmetric(10.0, 0.0))
        .show(ui, |ui| {
            ui.set_min_height(row_h);
            ui.set_min_width(avail_w - 0.0);

            ui.horizontal(|ui| {
                ui.set_min_height(row_h);
                ui.label(RichText::new(&selected_name).size(12.0).color(text_c));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (r, p) = ui.allocate_painter(vec2(14.0, 14.0), Sense::hover());
                    let c = r.rect.center();
                    let col = muted;
                    p.line_segment([c + vec2(-4.0, -2.0), c + vec2(0.0, 2.0)],
                        egui::Stroke::new(1.5, col));
                    p.line_segment([c + vec2(0.0, 2.0), c + vec2(4.0, -2.0)],
                        egui::Stroke::new(1.5, col));
                });
            });
        });

    ui.add_space(4.0);

    egui::ComboBox::new("audio_device_select", "")
        .selected_text(
            RichText::new(&selected_name).size(12.0).color(text_c),
        )
        .width(avail_w)
        .show_ui(ui, |ui| {
            let devs: Vec<String> = app.audio_devs.clone();
            for (i, dev) in devs.iter().enumerate() {
                let is_sel = i == app.settings.audio_device;
                let col    = if is_sel { accent } else { text_c };
                if ui.selectable_label(is_sel, RichText::new(dev).size(12.0).color(col)).clicked() {
                    app.settings.audio_device = i;
                }
            }
        });
}

fn draw_input_group(ui: &mut egui::Ui, app: &mut LocalShareApp) {
    let sep     = app.separator();
    let card_bg = app.bg_raised();
    let text_c  = app.text();
    let muted   = app.text_muted();
    let subtle  = app.text_subtle();

    Frame::none()
        .fill(card_bg)
        .rounding(egui::Rounding::same(9.0))
        .stroke(Stroke::new(1.0, sep))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                Frame::none()
                    .inner_margin(Margin::symmetric(14.0, 11.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new("Share mouse & keyboard")
                                        .size(12.5)
                                        .strong()
                                        .color(text_c),
                                );
                                ui.label(
                                    RichText::new("Control connected devices from this machine")
                                        .size(10.5)
                                        .color(muted),
                                );
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                toggle_switch(ui, app, |a| &mut a.settings.share_input);
                            });
                        });
                    });

                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().top(),
                    Stroke::new(1.0, sep),
                );

                Frame::none()
                    .inner_margin(Margin::symmetric(14.0, 10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Switch device")
                                    .size(12.5)
                                    .strong()
                                    .color(text_c),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                for key in ["1–9", "Alt", "Shift", "Ctrl"].iter() {
                                    kbd_chip(ui, app, key);
                                    if *key != "Ctrl" {
                                        ui.label(RichText::new("+").size(10.0).color(subtle));
                                    }
                                }
                            });
                        });
                    });
            });
        });
}

fn toggle_switch<F>(ui: &mut egui::Ui, app: &mut LocalShareApp, mut get: F)
where
    F: FnMut(&mut LocalShareApp) -> &mut bool,
{
    let value = *get(app);
    let accent = app.accent();
    let sep    = app.separator();
    let w = 34.0; let h = 20.0;

    let (resp, painter) = ui.allocate_painter(vec2(w, h), Sense::click());
    let rect = resp.rect;

    let track_bg = if value { accent } else { sep };
    let knob_x   = if value { rect.right() - h / 2.0 } else { rect.left() + h / 2.0 };
    let center_y = rect.center().y;

    painter.rect_filled(rect, h / 2.0, track_bg);
    painter.circle_filled(egui::pos2(knob_x, center_y), h / 2.0 - 3.0, egui::Color32::WHITE);

    if resp.clicked() {
        *get(app) = !value;
    }
}

fn kbd_chip(ui: &mut egui::Ui, app: &LocalShareApp, text: &str) {
    let bg     = app.bg_panel();
    let sep    = app.separator();
    let subtle = app.text_subtle();

    Frame::none()
        .fill(bg)
        .rounding(egui::Rounding::same(4.0))
        .stroke(Stroke::new(1.0, sep))
        .inner_margin(Margin::symmetric(5.0, 2.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(10.0)
                    .color(subtle)
                    .family(egui::FontFamily::Monospace),
            );
        });
}
