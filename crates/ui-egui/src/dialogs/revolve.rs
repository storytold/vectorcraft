//! Compact Revolve controls. The object and rotation gizmo live on the artboard.
//! All document changes run through the effect dialog's existing preview/commit commands.

use egui::{Align2, FontId, Id, Pos2, Sense, Stroke, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_effects::REVOLVE;

use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, scrub, widgets};

const LABEL_W: f32 = 102.0;
const VALUE_W: f32 = 74.0;
const SIDE_W: f32 = 354.0;

/// Sized to the screen, with scrolling inside the body so OK and Cancel stay reachable.
pub(super) fn body(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    // Values start in drag mode; clicking or Tab enters their text editor.
    ui.data_mut(|m| m.remove::<bool>(widgets::dialog_focus_flag()));
    let screen = ui.ctx().content_rect();
    let width = (screen.width() - 64.0).clamp(240.0, SIDE_W);
    let height = (screen.height() - 240.0).clamp(120.0, 340.0);
    ui.set_width(width);
    egui::ScrollArea::vertical().id_salt("revolve-body").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
        controls(app, ui, d);
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let t = Tokens::get(ui.ctx());
        ui.allocate_ui_with_layout(vec2((width - 100.0).max(80.0), 32.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(egui::RichText::new(super::revolve_gizmo::hint(d)).size(11.0).color(t.text_dim)).wrap());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let defaults = vectorcraft_effects::default_params(REVOLVE).unwrap_or(json!({}));
            let changed = defaults.as_object().is_some_and(|m| {
                m.iter().any(|(k, v)| v.as_f64().map_or_else(|| d.fields.get(k) != Some(v), |default| (d.f64(k, default) - default).abs() > 1e-6))
            });
            let (r, _) = ui.allocate_exact_size(vec2(88.0, 24.0), Sense::hover());
            let response = ui.interact(r, Id::new("revolve-reset-all"), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!("Reset all")));
            if response.hovered() {
                ui.painter().rect_filled(r, 3.0, t.hover);
            }
            ui.painter().text(r.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, tl!("Reset all"), FontId::proportional(11.5), t.text);
            dot(ui, r.right_center() - vec2(10.0, 0.0), changed);
            if response.on_hover_text(tl!("Reset all Revolve settings to their defaults.")).clicked()
                && let Some(defaults) = defaults.as_object()
            {
                d.fields.extend(defaults.clone());
            }
        });
    });
}

fn controls(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    ui.spacing_mut().item_spacing.y = 4.0;
    let lighting = super::revolve_gizmo::lighting(d);
    if let Some(tab) = widgets::tab_bar(ui, &[tl!("Rotation"), tl!("Lighting")], usize::from(lighting)) {
        d.fields.insert("__gizmo".into(), json!(if tab == 1 { "light" } else { "rotation" }));
    }
    ui.add_space(8.0);
    if super::revolve_gizmo::lighting(d) {
        lighting_controls(ui, d);
        return;
    }
    let unit = app.session.general_unit();
    heading(ui, tl!("Shape"));
    number(ui, d, "angle", tl!("Revolve angle"), (0.0, 360.0), "°", 1, None);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Axis side"));
        let edge = d.str("edge");
        let current = if edge == "right" { tl!("Right") } else { tl!("Left") };
        let width = VALUE_W + rail_width(ui) + 6.0;
        if let Some(k) = widgets::dropdown(ui, "revolve-edge", current, &[tl!("Left"), tl!("Right")], width) {
            d.fields.insert("edge".into(), json!(if k == 0 { "left" } else { "right" }));
        }
        if reset_dot(ui, "edge", edge != "left", tl!("Left")) {
            d.fields.insert("edge".into(), json!("left"));
        }
    });
    number(ui, d, "offset", tl!("Offset"), (0.0, 100000.0), "", 1, Some(unit));
    ui.add_space(10.0);
    heading(ui, tl!("View"));
    for (key, name) in [("rotationX", tl!("Rotation X")), ("rotationY", tl!("Rotation Y")), ("rotationZ", tl!("Rotation Z"))] {
        number(ui, d, key, name, (-360.0, 360.0), "°", 1, None);
    }
    number(ui, d, "perspective", tl!("Perspective"), (0.0, 100.0), "%", 1, None);
    ui.add_space(6.0);
    egui::CollapsingHeader::new(tl!("Advanced")).id_salt("revolve-advanced").show(ui, |ui| {
        number(ui, d, "segments", tl!("Segments"), (8.0, 128.0), "", 0, None);
        super::form::check(ui, d, "__showAxis", tl!("Show axis"));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let mut visible = d.fields.get("expandVisibleOnly").and_then(Value::as_bool).unwrap_or(true);
            let response = ui.allocate_ui_with_layout(
                vec2((ui.available_width() - 30.0).max(100.0), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.checkbox(&mut visible, egui::RichText::new(tl!("Keep visible surfaces only when expanding")).size(12.0))
                },
            ).inner;
            if response.on_hover_text(tl!("Trim covered geometry on Expand Appearance. Live preview keeps the full surface. Transparent paints and surfaces exceeding visibility limits retain all faces.")).changed() {
                d.fields.insert("expandVisibleOnly".into(), json!(visible));
            }
            if reset_dot(ui, "expandVisibleOnly", !visible, "true") {
                d.fields.insert("expandVisibleOnly".into(), json!(true));
            }
        });
    });
}

fn lighting_controls(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        let mut shade = d.bool("shade");
        if ui.checkbox(&mut shade, tl!("Shading")).changed() {
            d.fields.insert("shade".into(), json!(shade));
        }
        ui.add_space((ui.available_width() - 24.0).max(0.0));
        if reset_dot(ui, "shade", !shade, "true") {
            d.fields.insert("shade".into(), json!(true));
        }
    });
    ui.add_enabled_ui(d.bool("shade"), |ui| {
        number(ui, d, "lightAzimuth", tl!("Direction"), (-360.0, 360.0), "°", 1, None);
        number(ui, d, "lightElevation", tl!("Elevation"), (-90.0, 90.0), "°", 1, None);
        number(ui, d, "lightIntensity", tl!("Intensity"), (0.0, 100.0), "%", 1, None);
        number(ui, d, "ambient", tl!("Ambient"), (0.0, 100.0), "%", 1, None);
    });
}

fn heading(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).font(crate::theme::semibold(13.0)).color(t.text));
}

fn label(ui: &mut egui::Ui, name: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, 24.0), Sense::hover());
    ui.put(r, egui::Label::new(egui::RichText::new(name).size(12.5).color(t.text)).halign(egui::Align::Min).truncate()).on_hover_text(name);
    scrub::note_label(ui, r);
}

fn rail_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - VALUE_W - 24.0 - 18.0).clamp(40.0, 134.0)
}

/// A value can be typed, dragged, scrubbed by its label, or moved on its slider.
#[allow(clippy::too_many_arguments)]
fn number(ui: &mut egui::Ui, d: &mut Dialog, key: &str, name: &str, range: (f64, f64), suffix: &str, decimals: usize, unit: Option<Unit>) {
    let default = vectorcraft_effects::default_params(REVOLVE).and_then(|p| p.get(key).and_then(Value::as_f64)).unwrap_or(0.0);
    let from = |n| unit.map_or(n, |u| u.from_pt(n));
    let to = |n| unit.map_or(n, |u| u.to_pt(n));
    let typed = unit.map(|u| widgets::typed_unit(ui.ctx(), u));
    let mut value = from(d.f64(key, default)).clamp(from(range.0), from(range.1));
    let suffix = unit.map_or_else(|| suffix.to_string(), |u| format!(" {}", u.suffix()));
    let mut next = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, name);
        let t = Tokens::get(ui.ctx());
        let response = ui
            .scope(|ui| {
                ui.visuals_mut().widgets.inactive.weak_bg_fill = t.input;
                ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, t.input_border);
                ui.add_sized(
                    [VALUE_W, 24.0],
                    egui::DragValue::new(&mut value)
                        .range(from(range.0)..=from(range.1))
                        .speed(if decimals == 0 { 1.0 } else { 0.25 })
                        .max_decimals(decimals)
                        .suffix(&suffix)
                        .update_while_editing(false)
                        .custom_parser(|s| {
                            unit.map_or_else(
                                || vectorcraft_doc::parse_number(&s.replace(['°', '%'], "")),
                                |u| typed.unwrap_or(u).parse(s).map(|v| u.from_pt(v)),
                            )
                        }),
                )
            })
            .inner;
        response.clone().on_hover_text(tl!("Drag values or labels. Double-click a value to type."));
        if response.changed() {
            next = Some(value);
        }
        if let Some(n) = scrub::field(ui, response.id, response.rect, &value.to_string(), Some(value), decimals as i32) {
            next = Some(n);
        }
        // Offset's rail covers a useful working range; larger values can still be typed.
        let rail_max = if key == "offset" { from(500.0) } else { from(range.1) };
        let rail_min = from(range.0);
        let rail = rail_width(ui) + VALUE_W;
        let slider =
            widgets::color_slider(ui, ("revolve-slider", key), ((value - rail_min) / (rail_max - rail_min)).clamp(0.0, 1.0) as f32, rail, &|_| {
                t.input_border
            });
        if let (Some(n), _) = slider {
            next = Some(rail_min + f64::from(n) * (rail_max - rail_min));
        }
        if reset_dot(ui, key, (to(value) - default).abs() > 1e-6, &format!("{}{}", from(default), suffix)) {
            next = Some(from(default));
        }
    });
    if let Some(n) = next.filter(|n| n.is_finite()) {
        let factor = 10f64.powi(decimals as i32);
        let n = (n * factor).round() / factor;
        d.fields.insert(key.into(), json!(to(n.clamp(from(range.0), from(range.1)))));
    }
}

fn dot(ui: &egui::Ui, p: Pos2, changed: bool) {
    let t = Tokens::get(ui.ctx());
    if changed {
        ui.painter().circle_filled(p, 3.0, if ui.is_enabled() { t.accent } else { t.text_disabled });
    } else {
        ui.painter().circle_stroke(p, 3.0, Stroke::new(1.0, t.text_disabled));
    }
}

fn reset_dot(ui: &mut egui::Ui, key: &str, changed: bool, default: &str) -> bool {
    let (r, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
    let response = ui.interact(r, Id::new(("revolve-reset", key)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!("Reset to default")));
    if response.hovered() {
        ui.painter().rect_filled(r, 3.0, Tokens::get(ui.ctx()).hover);
    }
    dot(ui, r.center(), changed);
    response.on_hover_text(format!("{}: {default}", tl!("Reset to default"))).clicked()
}

#[cfg(test)]
#[path = "tests_revolve.rs"]
mod tests;
