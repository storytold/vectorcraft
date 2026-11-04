//! The Contour bar: a floating bar above the selection while the Contour tool is active. It has the
//! Contour's distance in a number box, its corner style (Sharp, Round or Square) and Bake, which
//! turns the live Contour into plain paths. It acts through `object.contour` and
//! `effect.expandAppearance`, so everything it does is a command with its own undo step.

use serde_json::Value;
use vectorcraft_doc::{Node, NodeKind};

use super::{CornerRadius, Stroke, Tokens, Ui, VectorcraftApp, Xf, json, pos2, vec2};

/// The live effect a Contour is made of.
const EFFECT: &str = "path.offsetPath";

fn bar_id() -> egui::Id {
    egui::Id::unique("contour-bar")
}

/// Where the Contour bar was last drawn (none while it isn't shown).
#[cfg(test)]
pub(super) fn bar_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    let id = bar_id();
    ctx.memory(|m| m.areas().is_visible(&egui::LayerId::new(egui::Order::Middle, id)).then(|| m.area_rect(id)).flatten())
}

/// A selected Contour's distance and corner style, read from its Offset Path effect.
struct Standing {
    offset: f64,
    corners: &'static str,
}

fn standing(n: &Node) -> Option<Standing> {
    let e = n.appearance.effects.iter().rev().find(|e| e.id == EFFECT)?;
    let offset = e.params.get("offset").and_then(Value::as_f64).unwrap_or(0.0);
    let corners = match e.params.get("joins").and_then(Value::as_str) {
        Some("round") => "round",
        Some("bevel") => "square",
        _ => "sharp",
    };
    Some(Standing { offset, corners })
}

/// The Contour bar, over the selection's top edge (under it when the top is off the canvas).
pub(super) fn contour_bar(app: &mut VectorcraftApp, ui: &mut Ui, xf: &Xf) {
    if app.session.tool_id() != "contour" {
        return;
    }
    let Some(b) = app.selection_bounds() else { return };
    let Some(st) = app.session.active() else { return };
    let shapes: Vec<&Node> = st
        .selection
        .objects
        .iter()
        .filter_map(|id| st.doc.node(*id))
        .filter(|n| matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }))
        .collect();
    if shapes.is_empty() {
        return;
    }
    let stand = shapes.first().and_then(|n| standing(n));
    drop(shapes);
    let tool_corners = app.session.tool_options().get("corners").and_then(Value::as_str).unwrap_or("sharp").to_string();
    let corners = stand.as_ref().map_or(tool_corners.as_str(), |s| s.corners);
    let shown = stand.as_ref().map_or(0.0, |s| s.offset);
    // A grown Contour reaches past the outline it was made from: keep the bar clear of it.
    let b = b.inflate(shown.max(0.0), shown.max(0.0));

    let t = Tokens::get(ui.ctx());
    let quad = xf.quad(b);
    let top = quad.iter().map(|c| c.y).fold(f32::INFINITY, f32::min);
    let bottom = quad.iter().map(|c| c.y).fold(f32::NEG_INFINITY, f32::max);
    let size = egui::AreaState::load(ui.ctx(), bar_id()).and_then(|s| s.size).unwrap_or(vec2(330.0, 40.0));
    let area = xf.rect.shrink(8.0);
    let above = top - size.y - 12.0;
    let y = if above >= area.top() { above } else { bottom + 12.0 };
    let at = pos2(xf.to_screen(b.center()).x - size.x / 2.0, y);
    let at = at.clamp(area.min, (area.max - size).max(area.min));

    let mut run: Option<(&str, Value)> = None;
    let mut corner_pick: Option<&str> = None;
    egui::Area::new(bar_id()).order(egui::Order::Middle).fixed_pos(at).show(ui.ctx(), |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .stroke(Stroke::new(1.0, t.tool_active))
            .corner_radius(CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(8, 6))
            .shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: egui::Color32::from_black_alpha(70) })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(egui::RichText::new(tl!("Contour")).color(t.text_strong));
                    // The distance: a drag or typed value is applied when it ends, as one step.
                    let key = egui::Id::unique("contour-bar-offset");
                    let mut v = ui.data(|d| d.get_temp::<f64>(key)).unwrap_or(shown);
                    let resp = ui.add(egui::DragValue::new(&mut v).speed(0.5).range(-1e5..=1e5).suffix(" pt"));
                    if resp.dragged() || resp.has_focus() {
                        ui.data_mut(|d| d.insert_temp(key, v));
                    }
                    if resp.drag_stopped() || resp.lost_focus() {
                        ui.data_mut(|d| d.remove_temp::<f64>(key));
                        if (v - shown).abs() > 1e-9 {
                            run = Some(("object.contour", json!({"offset": v, "corners": corners})));
                        }
                    }
                    for (label, id) in [(tl!("Sharp"), "sharp"), (tl!("Round"), "round"), (tl!("Square"), "square")] {
                        if ui.selectable_label(corners == id, label).clicked() {
                            corner_pick = Some(id);
                        }
                    }
                    let bake = ui.add_enabled(stand.is_some(), egui::Button::new(tl!("Bake")));
                    if bake.on_hover_text(tl!("Turn the Contour into plain paths")).clicked() {
                        run = Some(("effect.expandAppearance", json!({})));
                    }
                });
            });
    });
    if let Some(id) = corner_pick {
        // The tool keeps the style for the next Contour; a selected Contour changes now.
        let mut result = app.run("tool.setOption", json!({"key": "corners", "value": id}));
        if result.is_ok()
            && let Some(s) = &stand
        {
            result = app.run("object.contour", json!({"offset": s.offset, "corners": id}));
        }
        if let Err(e) = result {
            app.ui.status = e;
        }
    }
    if let Some((cmd, params)) = run
        && let Err(e) = app.run(cmd, params)
    {
        app.ui.status = e;
    }
}
