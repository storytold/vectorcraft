//! Effect > Warp controls. The style remains an effect ID; Envelope Warp has a different schema.
use egui::{Pos2, Rect, Stroke, vec2};
use serde_json::{Map, Value, json};
use vectorcraft_effects::{WARP_STYLES, WarpStyle, warp_point};

use super::form;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

const REMEMBERED: [&str; 4] = ["bend", "horizontal", "vertical", "orientation"];

pub(super) fn is_warp(id: &str) -> bool {
    id.strip_prefix("warp.").and_then(WarpStyle::from_id).is_some()
}

/// Slider and orientation values to seed the next new Warp Options dialog.
pub(super) fn remembered_params(d: &Dialog) -> Value {
    let kept = REMEMBERED.iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone())));
    Value::Object(kept.collect())
}

/// Last confirmed warp settings over the catalog defaults for a new dialog.
pub(super) fn merge_remembered(fields: &mut Map<String, Value>, remembered: Option<&Value>) {
    let Some(obj) = remembered.and_then(Value::as_object) else { return };
    for key in REMEMBERED {
        if let Some(v) = obj.get(key) {
            fields.insert(key.into(), v.clone());
        }
    }
}

fn shown(label: &str) -> String {
    tl!(label).trim_end_matches('…').to_string()
}

/// An original three-by-three grid, bent by VectorCraft's own warp map. No reference icon artwork.
fn style_preview(ui: &mut egui::Ui, suffix: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(30.0, 22.0), egui::Sense::hover());
    let Some(style) = WarpStyle::from_id(suffix) else { return };
    let mut lines = vec![];
    let mut bounds = Rect::NOTHING;
    for vertical in [false, true] {
        for fixed in [-1.0, 0.0, 1.0] {
            let line: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let along = -1.0 + f64::from(i) / 8.0;
                    let (x, y) = if vertical { (fixed, along) } else { (along, fixed) };
                    let (x, y) = warp_point(style, 0.45, 0.0, 0.0, 0.55, x, y);
                    let p = Pos2::new(x as f32, y as f32);
                    bounds.extend_with(p);
                    p
                })
                .collect();
            lines.push(line);
        }
    }
    let room = rect.shrink(2.0);
    let scale = (room.width() / bounds.width().max(0.001)).min(room.height() / bounds.height().max(0.001));
    let stroke = Stroke::new(1.0, Tokens::get(ui.ctx()).text);
    for line in lines {
        let points = line.into_iter().map(|p| room.center() + (p - bounds.center()) * scale).collect();
        ui.painter().add(egui::Shape::line(points, stroke));
    }
}

pub(super) fn body(ui: &mut egui::Ui, d: &mut Dialog) {
    const LABEL_W: f32 = 84.0;
    let id = d.str("__effect");
    let suffix = id.strip_prefix("warp.").unwrap_or_default();
    let current = WARP_STYLES.iter().find(|(style, _)| *style == suffix).map_or_else(|| suffix.to_string(), |(_, label)| shown(label));
    widgets::label_row(ui, tl!("Style:"), LABEL_W, |ui| {
        style_preview(ui, suffix);
        if let Some((style, label)) = widgets::combo(ui, "warp-style", &current, 170.0, |ui| {
            let mut chosen = None;
            for (style, label) in WARP_STYLES {
                ui.horizontal(|ui| {
                    style_preview(ui, style);
                    if ui.add(egui::Button::selectable(style == suffix, shown(label))).clicked() {
                        chosen = Some((style, label));
                        ui.close();
                    }
                });
            }
            chosen
        }) {
            d.fields.insert("__effect".into(), json!(format!("warp.{style}")));
            d.fields.insert("__warp_style_changed".into(), json!(true));
            d.fields.insert("__label".into(), json!(label.trim_end_matches('…')));
        }
    });
    let orientation = d.str("orientation");
    widgets::label_row(ui, "", LABEL_W, |ui| {
        for (label, value) in super::effect::own_choices(&id, "orientation", false).unwrap_or_default() {
            if widgets::radio(ui, label, orientation == value, true) {
                d.fields.insert("orientation".into(), json!(value));
            }
        }
    });
    let rail = Tokens::get(ui.ctx()).input_border;
    form::slider_w(ui, d, ("bend", tl!("Bend:"), LABEL_W), -100.0..=100.0, "%", &move |_| rail);
    ui.add_space(6.0);
    widgets::subheader(ui, tl!("Distortion"));
    for (key, label) in [("horizontal", tl!("Horizontal:")), ("vertical", tl!("Vertical:"))] {
        form::slider_w(ui, d, (key, label, LABEL_W), -100.0..=100.0, "%", &move |_| rail);
    }
}
