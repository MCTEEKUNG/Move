use egui::{Color32, Frame, Margin, Rect, RichText, Sense, Stroke, Vec2, vec2};

use crate::app::{LocalShareApp, SnapGuides, section_label};

// ── Constants ─────────────────────────────────────────────────────────────────

const LERP:       f32 = 0.55;
/// Distance (in scaled screen px) within which a dragged monitor's edge
/// magnetically attaches to another monitor's opposite edge. Generous, so
/// the pull is obvious — but only along the axis of proximity, never
/// fighting the user's direction.
const MAG_THRESH: f32 = 32.0;

fn canvas_h(ui: &egui::Ui) -> f32 {
    (ui.ctx().screen_rect().height() * 0.33).clamp(160.0, 290.0)
}

fn canvas_scale(n: usize) -> f32 {
    match n {
        0 | 1 => 2.00,
        2     => 1.60,
        3     => 1.28,
        4     => 1.00,
        5     => 0.82,
        _     => 0.68,
    }
}

pub fn draw(ui: &mut egui::Ui, app: &mut LocalShareApp) {
    draw_topbar(ui, app);
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            Frame::none()
                .inner_margin(Margin::symmetric(14.0, 10.0))
                .show(ui, |ui| {
                    draw_canvas(ui, app);
                    section_label(ui, "CONNECTIONS", app.text_muted());
                    draw_connections(ui, app);
                    ui.add_space(8.0);
                });
        });
}

fn draw_topbar(ui: &mut egui::Ui, app: &LocalShareApp) {
    let panel_bg  = app.bg_panel();
    let sep       = app.separator();
    let connected = app.monitors.iter().filter(|m| m.connected).count();

    Frame::none()
        .fill(panel_bg)
        .inner_margin(Margin { left: 14.0, right: 14.0, top: 12.0, bottom: 10.0 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Monitor").size(13.5).strong().color(app.text()));
                    ui.label(
                        RichText::new("Drag to arrange display layout")
                            .size(10.5).color(app.text_muted()),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if connected > 0 {
                        let g = app.green();
                        let pill_bg = Color32::from_rgba_unmultiplied(g.r(), g.g(), g.b(), 22);
                        let pill_bd = Color32::from_rgba_unmultiplied(g.r(), g.g(), g.b(), 65);
                        Frame::none()
                            .fill(pill_bg).stroke(Stroke::new(1.0, pill_bd))
                            .rounding(egui::Rounding::same(20.0))
                            .inner_margin(Margin { left: 8.0, right: 9.0, top: 3.0, bottom: 3.0 })
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let (r, p) = ui.allocate_painter(vec2(6.0, 6.0), Sense::hover());
                                    p.circle_filled(r.rect.center(), 2.8, g);
                                    ui.label(
                                        RichText::new(format!("{} connected", connected))
                                            .size(10.5).color(g).strong(),
                                    );
                                });
                            });
                    }
                });
            });
        });

    ui.painter().hline(
        ui.max_rect().x_range(),
        ui.cursor().top(),
        Stroke::new(1.0, sep),
    );
}

fn draw_canvas(ui: &mut egui::Ui, app: &mut LocalShareApp) {
    let canvas_bg = app.bg_raised();
    let sep       = app.separator();
    let accent    = app.accent();

    let cw    = ui.available_width();
    let ch    = canvas_h(ui);
    let n     = app.monitors.len();
    let scale = canvas_scale(n);

    let ncw = cw / scale;
    let nch = ch / scale;

    // Auto-center the primary monitor the first time we lay out, and on
    // canvas resize while the user hasn't dragged yet. Once the user starts
    // dragging anything, `primary_placed` latches true and we leave their
    // layout alone.
    let canvas_size = vec2(cw, ch);
    let resized = (canvas_size - app.last_canvas_size).length() > 0.5;
    if !app.monitors.is_empty()
        && app.dragging_id.is_none()
        && (!app.primary_placed || resized)
    {
        let primary_size = app.monitors[0].size;
        let cx = (ncw - primary_size.x) * 0.5;
        let cy = (nch - primary_size.y) * 0.5;
        let centered = vec2(cx.max(0.0), cy.max(0.0));
        let delta = centered - app.monitors[0].pos;
        // Shift all monitors by the same offset so the user's arrangement
        // (peers dropped at specific offsets to primary) doesn't break.
        for mon in app.monitors.iter_mut() {
            mon.pos      += delta;
            mon.anim_pos += delta;
        }
        app.primary_placed   = true;
        app.last_canvas_size = canvas_size;
    }

    for mon in app.monitors.iter_mut() {
        mon.pos.x = mon.pos.x.clamp(0.0, (ncw - mon.size.x).max(0.0));
        mon.pos.y = mon.pos.y.clamp(0.0, (nch - mon.size.y).max(0.0));
    }
    // NOTE: resolve_overlaps() disabled — it fought the user every frame by
    // pushing monitors back along X. Overlaps are now allowed; users can
    // arrange freely.

    let mut animating = false;
    for (i, mon) in app.monitors.iter_mut().enumerate() {
        if Some(i) == app.dragging_id {
            mon.anim_pos = mon.pos;
        } else {
            let d = mon.pos - mon.anim_pos;
            if d.length_sq() > 0.3 {
                mon.anim_pos += d * LERP;
                animating = true;
            } else {
                mon.anim_pos = mon.pos;
            }
        }
        mon.anim_pos.x = mon.anim_pos.x.clamp(0.0, (ncw - mon.size.x).max(0.0));
        mon.anim_pos.y = mon.anim_pos.y.clamp(0.0, (nch - mon.size.y).max(0.0));
    }
    if animating { ui.ctx().request_repaint(); }

    let (canvas_resp, painter) = ui.allocate_painter(vec2(cw, ch), Sense::hover());
    let cr = canvas_resp.rect;

    painter.rect_filled(cr, 10.0, canvas_bg);
    painter.rect_stroke(cr, 10.0, Stroke::new(1.0, sep));

    {
        let dot  = Color32::from_rgba_unmultiplied(sep.r(), sep.g(), sep.b(), 55);
        let step = 22.0_f32;
        let mut gx = cr.left() + step;
        while gx < cr.right() - 4.0 {
            let mut gy = cr.top() + step;
            while gy < cr.bottom() - 4.0 {
                painter.circle_filled(egui::pos2(gx, gy), 0.8, dot);
                gy += step;
            }
            gx += step;
        }
    }
    painter.text(
        cr.right_bottom() - vec2(10.0, 8.0),
        egui::Align2::RIGHT_BOTTOM,
        "Drag to reposition",
        egui::FontId::new(9.5, egui::FontFamily::Proportional),
        app.text_subtle(),
    );

    let dragging_id = app.dragging_id;
    let order: Vec<usize> = (0..n)
        .filter(|&i| Some(i) != dragging_id)
        .chain(dragging_id.into_iter())
        .collect();

    let mut new_dragging = dragging_id;
    let mut snap_out     = SnapGuides::default();

    for &i in &order {
        // Hit-test against the *visible* rect (anim_pos), not the target.
        // Using `pos` here meant that while a monitor was still lerping
        // into place the click target was somewhere the user couldn't see,
        // so grabbing it felt unresponsive.
        let s_pos  = app.monitors[i].anim_pos * scale;
        let s_size = app.monitors[i].size     * scale;
        let hit    = Rect::from_min_size(cr.min + s_pos, s_size);
        let id     = egui::Id::new("mon_drag").with(i);
        let resp   = ui.interact(hit, id, Sense::drag());

        if resp.drag_started() {
            new_dragging = Some(i);
            // User has taken over layout — stop auto-centering on resize.
            app.primary_placed = true;
            // Sync target to visible so the first drag frame doesn't teleport
            // to wherever `pos` was still heading.
            app.monitors[i].pos = app.monitors[i].anim_pos;
        }
        if resp.drag_stopped() && new_dragging == Some(i) {
            new_dragging = None;
            // NOTE: previously teleported to `nearest_adjacent` if the monitor
            // wasn't touching another — that made releasing feel like the
            // monitor jumped away from you. Free placement now.
        }

        if resp.dragged() {
            // Cursor-tracking drag with edge-magnet snap. If one of the
            // dragged monitor's 4 edges gets within MAG_THRESH of another
            // monitor's opposite edge (and the perpendicular extents
            // overlap), gently pull it flush so the user sees the border
            // "connect". Edge endpoints are returned so we can highlight
            // the shared segment.
            let delta  = resp.drag_delta();
            let raw_sx = app.monitors[i].pos.x * scale + delta.x;
            let raw_sy = app.monitors[i].pos.y * scale + delta.y;
            let dw_s   = app.monitors[i].size.x * scale;
            let dh_s   = app.monitors[i].size.y * scale;

            let others_s: Vec<(f32, f32, f32, f32)> = app.monitors.iter().enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, m)| (m.anim_pos.x*scale, m.anim_pos.y*scale,
                               m.size.x*scale,     m.size.y*scale))
                .collect();

            let (sx, sy, edge) = magnetic_snap(raw_sx, raw_sy, dw_s, dh_s, &others_s);

            app.monitors[i].pos      = vec2(sx, sy) / scale;
            app.monitors[i].anim_pos = vec2(sx, sy) / scale;
            snap_out.magnet = edge;
        }
    }

    app.dragging_id = new_dragging;
    app.snap_guides = if app.dragging_id.is_some() { snap_out } else { SnapGuides::default() };

    for &i in &order {
        let mon     = &app.monitors[i];
        let is_drag = Some(i) == app.dragging_id;
        let primary = i == 0;

        let sp = mon.anim_pos * scale;
        let ss = mon.size     * scale;
        let br = Rect::from_min_size(cr.min + sp, ss);

        if is_drag {
            painter.rect_filled(
                br.translate(vec2(3.0, 6.0)), 7.0,
                Color32::from_rgba_unmultiplied(0, 0, 0, 42),
            );
        }

        let (fill, bc, bw) = if mon.connected {
            let g = app.green();
            let f = Color32::from_rgba_unmultiplied(g.r(), g.g(), g.b(), if primary {30} else {18});
            let b = Color32::from_rgba_unmultiplied(g.r(), g.g(), g.b(), if primary {120} else {80});
            (f, b, if primary || is_drag { 1.5_f32 } else { 1.0 })
        } else {
            let bc = if is_drag { accent } else { sep };
            (app.bg_panel(), bc, if is_drag || primary { 1.5 } else { 1.0 })
        };

        painter.rect_filled(br, 7.0, fill);
        painter.rect_stroke(br, 7.0, Stroke::new(bw, bc));

        let screen = br.shrink(4.0);
        if screen.width() > 4.0 && screen.height() > 4.0 {
            painter.rect_filled(screen, 3.0,
                Color32::from_rgba_unmultiplied(0, 0, 0, if app.settings.dark_mode {22} else {10}));
        }

        let stand_c  = Color32::from_rgba_unmultiplied(bc.r(), bc.g(), bc.b(), 65);
        let stem_h   = (ss.y * 0.07).max(3.0);
        let base_w   = ss.x * 0.38;
        let stem_top = br.bottom();
        painter.rect_filled(
            Rect::from_center_size(egui::pos2(br.center().x, stem_top + stem_h * 0.5), vec2(3.0, stem_h)),
            0.0, stand_c,
        );
        painter.rect_filled(
            Rect::from_center_size(egui::pos2(br.center().x, stem_top + stem_h), vec2(base_w, 2.5)),
            1.5, stand_c,
        );

        let dot_c = if mon.connected { app.green() } else { app.text_subtle() };
        painter.circle_filled(br.right_top() + vec2(-6.5, 6.5), 2.5, dot_c);

        let lbl_sz = (ss.y * 0.17).clamp(8.5, 13.0);
        let sub_sz = (ss.y * 0.13).clamp(7.0, 10.5);
        let cy     = br.center().y - lbl_sz * 0.35;

        if primary {
            let chip_col = Color32::from_rgba_unmultiplied(
                accent.r(), accent.g(), accent.b(), 175);
            painter.text(
                egui::pos2(br.center().x, br.top() + lbl_sz * 0.9),
                egui::Align2::CENTER_CENTER,
                "PRIMARY",
                egui::FontId::new((lbl_sz * 0.65).max(7.0), egui::FontFamily::Proportional),
                chip_col,
            );
        }

        let label_y = if primary { cy + lbl_sz * 0.5 } else { cy };
        painter.text(
            egui::pos2(br.center().x, label_y),
            egui::Align2::CENTER_CENTER,
            &mon.label,
            egui::FontId::new(lbl_sz, egui::FontFamily::Proportional),
            app.text(),
        );
        painter.text(
            egui::pos2(br.center().x, label_y + lbl_sz * 1.1),
            egui::Align2::CENTER_CENTER,
            &mon.host,
            egui::FontId::new(sub_sz, egui::FontFamily::Proportional),
            app.text_muted(),
        );
    }

    // Magnet edge highlight: a bright accent segment along the shared
    // border between the dragged monitor and whichever neighbor it's
    // snapping to. Gives the user immediate visual confirmation that
    // "this edge will connect to that edge".
    if let Some((a, b)) = app.snap_guides.magnet {
        let a = cr.min + a.to_vec2();
        let b = cr.min + b.to_vec2();
        let glow = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 80);
        // Soft outer glow, then crisp core line.
        painter.line_segment([a, b], Stroke::new(6.0, glow));
        painter.line_segment([a, b], Stroke::new(2.5, accent));
        // End caps so the "connection" reads clearly even on short edges.
        painter.circle_filled(a, 3.5, accent);
        painter.circle_filled(b, 3.5, accent);
    }
}

/// Edge-to-edge magnetic snap.
///
/// When one of the dragged monitor's four edges lands within MAG_THRESH of
/// another monitor's opposite edge *and* the perpendicular extents overlap,
/// snap flush to that edge and return the shared-edge segment so it can
/// be painted as a "this border will connect" indicator.
///
/// Outside the snap zone, returns the cursor position unchanged — no wall
/// snap, no alignment pulls, no overlap prevention. Mirrors the feel of
/// Windows Display Settings.
fn magnetic_snap(
    raw_x: f32, raw_y: f32,
    dw: f32, dh: f32,
    others: &[(f32, f32, f32, f32)],
) -> (f32, f32, Option<(egui::Pos2, egui::Pos2)>) {
    let a_right  = raw_x + dw;
    let a_bottom = raw_y + dh;

    let mut best_d = MAG_THRESH;
    let mut out_x  = raw_x;
    let mut out_y  = raw_y;
    let mut edge: Option<(egui::Pos2, egui::Pos2)> = None;

    for &(ox, oy, ow, oh) in others {
        let o_right  = ox + ow;
        let o_bottom = oy + oh;

        // Perpendicular overlaps (need >0, not just touching, to avoid
        // snapping a monitor diagonally off another's corner).
        let v_overlap = raw_y < o_bottom && a_bottom > oy;
        let h_overlap = raw_x < o_right  && a_right  > ox;

        if v_overlap {
            // Dragged right edge ↔ other's left edge.
            let d = (a_right - ox).abs();
            if d < best_d {
                best_d = d;
                out_x  = ox - dw;
                out_y  = raw_y;
                let top = out_y.max(oy);
                let bot = (out_y + dh).min(o_bottom);
                edge = Some((egui::pos2(ox, top), egui::pos2(ox, bot)));
            }
            // Dragged left edge ↔ other's right edge.
            let d = (raw_x - o_right).abs();
            if d < best_d {
                best_d = d;
                out_x  = o_right;
                out_y  = raw_y;
                let top = out_y.max(oy);
                let bot = (out_y + dh).min(o_bottom);
                edge = Some((egui::pos2(o_right, top), egui::pos2(o_right, bot)));
            }
        }

        if h_overlap {
            // Dragged bottom edge ↔ other's top edge.
            let d = (a_bottom - oy).abs();
            if d < best_d {
                best_d = d;
                out_x  = raw_x;
                out_y  = oy - dh;
                let left  = out_x.max(ox);
                let right = (out_x + dw).min(o_right);
                edge = Some((egui::pos2(left, oy), egui::pos2(right, oy)));
            }
            // Dragged top edge ↔ other's bottom edge.
            let d = (raw_y - o_bottom).abs();
            if d < best_d {
                best_d = d;
                out_x  = raw_x;
                out_y  = o_bottom;
                let left  = out_x.max(ox);
                let right = (out_x + dw).min(o_right);
                edge = Some((egui::pos2(left, o_bottom), egui::pos2(right, o_bottom)));
            }
        }
    }

    (out_x, out_y, edge)
}

fn draw_connections(ui: &mut egui::Ui, app: &mut LocalShareApp) {
    let n    = app.monitors.len();
    let rows: Vec<_> = app.monitors.iter().map(|m| {
        (m.label.clone(), m.host.clone(), m.resolution.clone(), m.hz, m.connected, m.active)
    }).collect();
    for (i, (label, host, res, hz, connected, active)) in rows.iter().enumerate() {
        draw_conn_card(ui, app, i, label, host, res, *hz, *connected, *active);
        if i < n - 1 { ui.add_space(6.0); }
    }
}

fn draw_conn_card(
    ui: &mut egui::Ui, app: &mut LocalShareApp,
    idx: usize, label: &str, host: &str, res: &str, hz: u32,
    connected: bool, _active: bool,
) {
    let sep     = app.separator();
    let card_bg = app.bg_raised();
    let accent  = app.accent();
    let green   = app.green();
    let text_c  = app.text();
    let muted   = app.text_muted();
    let subtle  = app.text_subtle();
    let primary = idx == 0;

    let border = if connected {
        Color32::from_rgba_unmultiplied(green.r(), green.g(), green.b(), 55)
    } else { sep };

    Frame::none()
        .fill(card_bg)
        .rounding(egui::Rounding::same(9.0))
        .stroke(Stroke::new(1.0, border))
        .inner_margin(Margin { left: 12.0, right: 12.0, top: 10.0, bottom: 10.0 })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let (tr, tp) = ui.allocate_painter(vec2(38.0, 27.0), Sense::hover());
                let tb = if connected {
                    Color32::from_rgba_unmultiplied(green.r(), green.g(), green.b(), 75)
                } else { sep };
                tp.rect_filled(tr.rect, 4.0, app.bg_panel());
                tp.rect_stroke(tr.rect, 4.0, Stroke::new(1.0, tb));
                tp.rect_filled(tr.rect.shrink(3.0), 2.0, Color32::from_rgba_unmultiplied(0,0,0,18));
                tp.rect_filled(
                    Rect::from_center_size(egui::pos2(tr.rect.center().x, tr.rect.bottom()+2.0), vec2(2.0, 4.0)),
                    0.0, tb,
                );
                tp.rect_filled(
                    Rect::from_center_size(egui::pos2(tr.rect.center().x, tr.rect.bottom()+4.0), vec2(12.0, 2.0)),
                    1.0, tb,
                );
                let num: String = label.chars().filter(|c| c.is_ascii_digit()).collect();
                tp.text(
                    tr.rect.center() - vec2(0.0, 1.0),
                    egui::Align2::CENTER_CENTER,
                    if num.is_empty() { &label[..1.min(label.len())] } else { num.as_str() },
                    egui::FontId::new(10.5, egui::FontFamily::Monospace),
                    if connected { green } else { muted },
                );

                ui.add_space(10.0);

                let right_w = 140.0_f32;
                let info_w  = (ui.available_width() - right_w).max(50.0);
                ui.allocate_ui(vec2(info_w, 0.0), |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(host).size(12.0).strong().color(text_c));
                            if primary {
                                ui.add_space(5.0);
                                Frame::none()
                                    .fill(Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 18))
                                    .stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 50)))
                                    .rounding(egui::Rounding::same(3.0))
                                    .inner_margin(Margin { left: 4.0, right: 4.0, top: 1.0, bottom: 1.0 })
                                    .show(ui, |ui| {
                                        ui.label(RichText::new("Primary").size(9.0).color(
                                            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 175)
                                        ));
                                    });
                            }
                        });
                        ui.add_space(2.0);
                        let sub = if connected { format!("{} · {} Hz", res, hz) }
                                  else { res.to_string() };
                        ui.label(RichText::new(sub).size(10.0).color(muted));
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (btn_label, use_accent) = if primary && connected { ("Sharing", true) }
                        else if connected { ("Connected", true) }
                        else { ("Connect", false) };

                    let btn_fill = if use_accent {
                        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 20)
                    } else { app.bg_panel() };

                    let button = egui::Button::new(RichText::new(btn_label).size(10.5)
                            .color(if use_accent { accent } else { muted }))
                            .fill(btn_fill)
                            .stroke(Stroke::new(1.0, if use_accent { accent } else { sep }))
                            .min_size(vec2(72.0, 24.0));
                    if ui.add(button).clicked() && !connected {
                        app.monitors[idx].connected = true;
                        app.monitors[idx].active    = true;
                        if let Some(slot) = app.monitors[idx].slot {
                            app.bridge.switch_to(slot);
                        }
                    }

                    ui.add_space(8.0);

                    // A non-primary card only exists because we discovered
                    // the peer — bridge.rs is already auto-dialing it. So
                    // "not connected" here always means the TCP handshake
                    // is in flight, never "Offline".
                    let (dot_c, lbl_c, status) = if connected {
                        (green, green, "Active")
                    } else if primary {
                        (subtle, muted, "Idle")
                    } else {
                        (accent, accent, "Connecting…")
                    };
                    ui.label(RichText::new(status).size(10.0).color(lbl_c));
                    let (dr, dp) = ui.allocate_painter(vec2(7.0, 7.0), Sense::hover());
                    dp.circle_filled(dr.rect.center(), 3.0, dot_c);
                });
            });
        });
}
