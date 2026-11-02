//! Duotone's dialog (Effect › Color Effects › Duotone…): a row per tone (on or off, its swatch or
//! process colour), the curve of the tone picked under Curve (the graph, its points as text and
//! the Reset buttons) and Skip Objects with Spot Colors.
//!
//! Fields: `tones` (the effect's, always [`MAX_TONES`] while the dialog is open, the ones after
//! the first off), `skipSpot`, and `__tone`, the tone whose curve shows.

use serde_json::{Map, Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_effects::addons::duotone::{MAX_TONES, tone_curve_points};

use super::effect::{curve_graph, points_text};
use super::form;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// A curve's text: linear, inverse.
const LINEAR: &str = "0,0 100,100";
const INVERSE: &str = "0,100 100,0";

/// Width of the label column.
const LABEL_W: f32 = 84.0;

/// A tone's colour (black when it has none that reads).
fn tone_color(tone: &Value) -> Color {
    match tone.get("color") {
        Some(Value::String(s)) => Color::from_hex(s),
        Some(v @ Value::Object(_)) => serde_json::from_value(v.clone()).ok(),
        _ => None,
    }
    .unwrap_or(Color::BLACK)
}

/// The dialog's fields; returns nothing, the caller compares the values with those previewed.
pub(super) fn fields(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let mut tones: Vec<Value> = d.fields.get("tones").and_then(Value::as_array).cloned().unwrap_or_default();
    tones.truncate(MAX_TONES);
    while tones.len() < MAX_TONES {
        tones.push(json!({"color": "#000000", "curve": LINEAR, "active": false}));
    }
    // The global and spot swatches a tone can be: (name, colour).
    let swatches: Vec<(String, Color)> = app
        .session
        .active()
        .map(|st| st.doc.swatches_iter().filter_map(|w| Some((w.name.clone(), st.doc.global_color(&w.name)?))).collect())
        .unwrap_or_default();
    let labels = [tl!("Tone 1"), tl!("Tone 2"), tl!("Tone 3"), tl!("Tone 4")];
    let process = tl!("Process Color");
    let names: Vec<&str> = std::iter::once(process).chain(swatches.iter().map(|(n, _)| n.as_str())).collect();
    for (i, tone) in tones.iter_mut().enumerate() {
        let Some(t) = tone.as_object_mut() else { continue };
        ui.horizontal(|ui| {
            let on = t.get("active").and_then(Value::as_bool).unwrap_or(true);
            if widgets::check(ui, labels.get(i).copied().unwrap_or_default(), on, true) {
                t.insert("active".into(), json!(!on));
            }
            tone_row(ui, (i, labels.get(i).copied().unwrap_or_default()), t, &swatches, &names);
        });
    }
    ui.add_space(6.0);
    widgets::subheader(ui, tl!("Curve"));
    let picked = (d.f64("__tone", 0.0).max(0.0) as usize).min(MAX_TONES - 1);
    ui.horizontal(|ui| {
        let current = labels.get(picked).copied().unwrap_or_default();
        if let Some(i) = widgets::dropdown_named(ui, "duotone-curve-tone", Some(tl!("Curve")), current, &labels, 120.0) {
            d.fields.insert("__tone".into(), json!(i));
        }
    });
    if let Some(t) = tones.get_mut(picked).and_then(Value::as_object_mut) {
        curve_fields(ui, t);
    }
    d.fields.insert("tones".into(), Value::Array(tones));
    ui.add_space(4.0);
    form::check(ui, d, "skipSpot", tl!("Skip Objects with Spot Colors"));
}

/// Tone `i`'s (named `name`) colour chip, its swatch (or Process Color) and, as a process colour,
/// its hex value.
fn tone_row(ui: &mut egui::Ui, (i, name): (usize, &str), t: &mut Map<String, Value>, swatches: &[(String, Color)], names: &[&str]) {
    let color = tone_color(&Value::Object(t.clone()));
    let (chip, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    widgets::paint_chip(ui, chip, &Paint::solid(color));
    let swatch = t.get("swatch").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let current = swatch.as_deref().unwrap_or(names.first().copied().unwrap_or_default());
    if let Some(k) = widgets::dropdown_names_named(ui, ("duotone-swatch", i), Some(name), current, names, 130.0) {
        match k.checked_sub(1).and_then(|k| swatches.get(k)) {
            Some((name, c)) => {
                t.insert("swatch".into(), json!(name));
                t.insert("color".into(), serde_json::to_value(c).unwrap_or(Value::Null));
            }
            None => {
                t.remove("swatch");
            }
        }
    }
    if swatch.is_none()
        && let Some(hex) = widgets::hex_field_named(ui, ("duotone-hex", i), Some(name), color.to_hex().trim_start_matches('#'))
        && let Some(c) = Color::from_hex(&format!("#{hex}"))
    {
        t.insert("color".into(), json!(c.to_hex()));
    }
}

/// The picked tone's curve: the graph, its points as text, and Reset to Linear or to Inverse.
fn curve_fields(ui: &mut egui::Ui, t: &mut Map<String, Value>) {
    let mut pts = tone_curve_points(t.get("curve"));
    if curve_graph(ui, &mut pts, 100.0) {
        t.insert("curve".into(), json!(points_text(&pts, 100.0)));
    }
    widgets::label_row(ui, tl!("Points"), LABEL_W, |ui| {
        let mut text = t.get("curve").and_then(Value::as_str).unwrap_or(LINEAR).to_string();
        if ui.add(egui::TextEdit::singleline(&mut text).desired_width(200.0)).changed() {
            t.insert("curve".into(), json!(text));
        }
    });
    ui.horizontal(|ui| {
        for (label, curve) in [(tl!("Reset to Linear"), LINEAR), (tl!("Reset to Inverse"), INVERSE)] {
            if widgets::flat_button(ui, label, 120.0).clicked() {
                t.insert("curve".into(), json!(curve));
            }
        }
    });
}
