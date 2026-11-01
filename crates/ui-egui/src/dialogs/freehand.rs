//! Tool Options of the freehand tools (double-click Pencil, Paintbrush, Smooth, Blob Brush or
//! Eraser): the options each tool keeps, which last across tool switches and are saved with the
//! preferences. OK sets them with `tool.setOption {tool, values}`. The Pencil also has `liveFit`
//! (Live curve fitting, on: its preview fits the stroke while it is drawn).
//!
//! Fields: `tool`; `fidelity` (points; Pencil, Paintbrush, Smooth and Blob Brush), `fill` (Pencil and
//! Paintbrush), `keepSelected` (Pencil, Paintbrush and Blob Brush), `mergeOnlySelection` (Blob
//! Brush), `closeWithin` and `editWithin` (screen pixels, 0 turns it off; Pencil and Paintbrush),
//! `angle` (degrees) and `roundness` (percent; Blob Brush and Eraser), `livePreview` (Pencil: the stroke shows
//! with its fill and stroke while it is drawn, or as a thin line until the release), `size`
//! (points; Blob Brush and Eraser), and `sizeMode` (`fixed` or `pressure`) and `sizeVariation`
//! (points; Blob Brush and Eraser: with Pressure the tip is Size − Variation wide at the lightest
//! pen pressure and Size + Variation at the heaviest).

use serde_json::{Map, Value, json};

use super::document_setup::check;
use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the freehand tools' Tool Options.
pub const KIND: &str = "freehandOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: CONTENT_W, ..DialogSpec::FORM };

/// The tools the dialog serves.
pub const TOOLS: [&str; 5] = ["pencil", "paintbrush", "smooth", "blobBrush", "eraser"];

/// The options the dialog edits, in the order it shows them (a tool has those it reports).
const KEYS: [&str; 14] = [
    "fidelity",
    "angle",
    "roundness",
    "size",
    "sizeMode",
    "sizeVariation",
    "fill",
    "altSmooth",
    "keepSelected",
    "mergeOnlySelection",
    "closeWithin",
    "editWithin",
    "livePreview",
    "liveFit",
];

/// Width of the dialog's rows: every number field ends at its right edge.
const CONTENT_W: f32 = 330.0;

/// Width of the number fields, Fidelity's among them.
const FIELD_W: f32 = 72.0;

/// Width of the label column of a row with a number field: what the field leaves of the row. The
/// Fidelity slider takes the same width under its label.
fn label_w(ui: &egui::Ui) -> f32 {
    CONTENT_W - FIELD_W - ui.spacing().item_spacing.x
}

/// The top of the Fidelity slider (points); it starts at the lowest Fidelity the tool takes, and
/// the field takes up to 100.
const FIDELITY_MAX: f64 = 20.0;

/// Fidelity `v` stepped `n` steps by the slider's and the field's keys: an arrow steps hundredths up to 0.1 pt,
/// tenths up to 1 pt and whole points above (a step down from 0.1 or 1 takes the finer step below
/// it); ten steps at once (Shift with an arrow, Page Up/Down) multiply or divide by ten, so they go
/// 0.01 → 0.1 → 1 → 10.
fn fidelity_step(v: f64, n: f64) -> f64 {
    if n.abs() >= 10.0 {
        let times = 10f64.powf((n / 10.0).round());
        return ((v * times) * 100.0).round() / 100.0;
    }
    // The step at `v` going up, or going down from it.
    let unit = |v: f64, up: bool| {
        let at = |edge: f64| if up { v < edge - 1e-9 } else { v <= edge + 1e-9 };
        if at(0.1) {
            0.01
        } else if at(1.0) {
            0.1
        } else {
            1.0
        }
    };
    let up = n > 0.0;
    let mut v = v;
    for _ in 0..(n.abs().min(1000.0) as usize) {
        let u = unit(v, up);
        let q = v / u;
        let k = if up { (q + 1e-6).floor() + 1.0 } else { (q - 1e-6).ceil() - 1.0 };
        v = ((k * u) * 100.0).round() / 100.0;
    }
    v
}

/// Where Fidelity `v` sits on a slider over `(min, max)`, from 0 to 1. The rail is logarithmic, so
/// the low values, where the stroke is followed closely, get as much of it as the high ones
/// (0.01–1 pt takes most of a 0.01–20 rail).
fn fidelity_at(v: f64, (min, max): (f64, f64)) -> f32 {
    if !(v.is_finite() && min > 0.0 && max > min) {
        return 0.0;
    }
    ((v.clamp(min, max) / min).ln() / (max / min).ln()) as f32
}

/// The Fidelity at `x` (0 to 1) on a slider over `(min, max)`: hundredths below 1 pt, tenths above.
fn fidelity_from(x: f32, (min, max): (f64, f64)) -> f64 {
    let v = min * (max / min).powf(f64::from(x.clamp(0.0, 1.0)));
    let step = if v < 1.0 { 100.0 } else { 10.0 };
    ((v * step).round() / step).clamp(min, max)
}

fn heading(d: &Dialog) -> String {
    let label = vectorcraft_tools::tool_info(&d.str("tool")).map_or("", |t| t.label);
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))])
}

/// Open the options of freehand tool `tool` with their current values.
pub fn open(app: &mut VectorcraftApp, tool: &str) -> Result<Value, String> {
    if !TOOLS.contains(&tool) {
        return Err(format!("`{tool}` isn't a freehand tool"));
    }
    let o = app.session.tool_options_of(tool);
    let mut fields: Map<String, Value> = KEYS.iter().filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    fields.insert("tool".into(), json!(tool));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(json!({ "dialog": KIND }))
}

/// A row with a number field for `d.fields[key]` (`suffix` is its unit), kept within `range`.
fn number(ui: &mut egui::Ui, d: &mut Dialog, (key, label): (&str, &str), suffix: &str, decimals: usize, range: (f64, f64)) {
    let label_w = label_w(ui);
    let v = d.f64(key, 0.0);
    widgets::label_row(ui, label, label_w, |ui| {
        if let Some(x) = widgets::range_field(ui, ("freehand-field", key), v, range.0..=range.1, suffix, decimals, FIELD_W) {
            d.fields.insert(key.into(), json!(x));
        }
    });
}

/// Width of the Blob Brush's and the Eraser's Size mode menu.
const MODE_W: f32 = 96.0;

/// The Blob Brush's and the Eraser's Size rows: Size with its Fixed or Pressure menu before the
/// field, then the Variation that Pressure adds and takes (greyed while Size is Fixed).
fn blob_size(ui: &mut egui::Ui, d: &mut Dialog) {
    const MODES: [&str; 2] = ["fixed", "pressure"];
    let pressure = d.str("sizeMode") == "pressure";
    let labels = ["Fixed", "Pressure"];
    let label_w = label_w(ui) - MODE_W - ui.spacing().item_spacing.x;
    let size = d.f64("size", 10.0);
    widgets::label_row(ui, tl!("Size:"), label_w, |ui| {
        if let Some(m) = widgets::dropdown(ui, "freehand-size-mode", labels[usize::from(pressure)], &labels, MODE_W).and_then(|i| MODES.get(i)) {
            d.fields.insert("sizeMode".into(), json!(m));
        }
        if let Some(x) = widgets::range_field(ui, ("freehand-field", "size"), size, 0.1..=1000.0, "pt", 1, FIELD_W) {
            d.fields.insert("size".into(), json!(x));
        }
    });
    ui.add_enabled_ui(pressure, |ui| number(ui, d, ("sizeVariation", tl!("Variation:")), "pt", 1, (0.0, 1000.0)));
}

/// The Fidelity rows: the label on a line of its own, then a logarithmic slider from the tool's
/// lowest Fidelity to [`FIDELITY_MAX`] across the row with the field at its end, both to two
/// decimals, and Accurate and Smooth under the slider's ends.
fn fidelity(ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let v = d.f64("fidelity", 1.5);
    let rail = (vectorcraft_tools::draw2::min_fidelity(&d.str("tool")), FIDELITY_MAX);
    let rail_w = label_w(ui);
    let mut new = None;
    ui.label(egui::RichText::new(tl!("Fidelity:")).size(12.5).color(t.text));
    ui.horizontal(|ui| {
        let shown = widgets::SliderValue::field(v, rail.0..=rail.1, "pt", 2).with_step(fidelity_step);
        let (x, _, keyed) =
            widgets::color_slider_stepped(ui, "freehand-fidelity", tl!("Fidelity:"), Some(shown), fidelity_at(v, rail), rail_w, &|_| t.input_border);
        new = x.map(|x| fidelity_from(x, rail)).or(keyed);
        if let Some(x) = widgets::range_field_stepped(ui, "freehand-fidelity-field", v, rail.0..=100.0, "pt", 2, FIELD_W, fidelity_step) {
            new = Some(x);
        }
    });
    if let Some(n) = new.filter(|n| *n != v) {
        d.fields.insert("fidelity".into(), json!(n));
    }
    form::slider_ends_below(ui, rail_w, (tl!("Accurate"), tl!("Smooth")));
    ui.add_space(4.0);
    // Wrapped to the rows' width, so the sentence doesn't widen the dialog.
    ui.allocate_ui(egui::vec2(CONTENT_W, 0.0), |ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(tl!("A lower value follows the stroke closely; a higher one gives fewer points and smoother curves."))
                    .size(11.5)
                    .color(t.text_dim),
            )
            .wrap(),
        );
    });
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    if d.fields.contains_key("fidelity") {
        fidelity(ui, d);
        ui.add_space(10.0);
    }
    if d.fields.contains_key("angle") {
        number(ui, d, ("angle", tl!("Angle:")), "°", 0, (-180.0, 180.0));
    }
    if d.fields.contains_key("roundness") {
        let min = if d.str("tool") == "blobBrush" { 0.0 } else { 1.0 };
        number(ui, d, ("roundness", tl!("Roundness:")), "%", 0, (min, 100.0));
    }
    if d.str("tool") == "blobBrush" {
        blob_tip_preview(ui, d);
    }
    if d.fields.contains_key("sizeMode") {
        blob_size(ui, d);
    } else if d.fields.contains_key("size") {
        number(ui, d, ("size", tl!("Size:")), "pt", 1, (0.1, 1000.0));
    }
    if d.fields.contains_key("fill") {
        check(ui, d, "fill", tl!("Fill new strokes"));
    }
    if d.fields.contains_key("altSmooth") {
        check(ui, d, "altSmooth", tl!("Alt key toggles to Smooth Tool"));
    }
    if d.fields.contains_key("keepSelected") {
        check(ui, d, "keepSelected", tl!("Keep selected"));
        if d.fields.contains_key("mergeOnlySelection") {
            check(ui, d, "mergeOnlySelection", tl!("Merge only with selection"));
        }
        ui.add_space(6.0);
    }
    if d.fields.contains_key("closeWithin") {
        number(ui, d, ("closeWithin", tl!("Close paths when ends are within:")), "px", 0, (0.0, 100.0));
    }
    if d.fields.contains_key("editWithin") {
        number(ui, d, ("editWithin", tl!("Edit selected paths within:")), "px", 0, (0.0, 100.0));
    }
    if d.fields.contains_key("livePreview") {
        ui.add_space(6.0);
        check(ui, d, "livePreview", tl!("Live preview"));
    }
    if d.fields.contains_key("liveFit") {
        if !d.fields.contains_key("livePreview") {
            ui.add_space(6.0);
        }
        check(ui, d, "liveFit", tl!("Live curve fitting"));
    }
    false
}

/// Side of the Blob Brush tip preview's square, in points.
const TIP_PREVIEW: f32 = 64.0;

/// The Blob Brush's tip preview: a square with the tip at its Angle and Roundness and a line along
/// its long axis, and beside it three dabs of the tip, the middle one darker. A click or a drag in
/// the square points the Angle at the pointer and gives the square the keyboard focus; screen
/// readers get it as the Angle slider, and its keys step the Angle as a dialog slider's do
/// ([`widgets::stepped_area`]) within -180° to 180°.
fn blob_tip_preview(ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(CONTENT_W, TIP_PREVIEW), egui::Sense::hover());
    let square = egui::Rect::from_min_size(rect.min, egui::vec2(TIP_PREVIEW, TIP_PREVIEW));
    let response = ui.interact(square, ui.scope_id().with("freehand-tip-angle"), egui::Sense::click_and_drag());
    if let Some((p, _)) = widgets::pointer_phase(&response) {
        let delta = p - square.center();
        if delta.length_sq() > 1.0 {
            d.fields.insert("angle".into(), json!(f64::from((-delta.y).atan2(delta.x)).to_degrees().round()));
        }
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(square, 2.0, t.input);
    painter.rect_stroke(square, 2.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    // After the square's frame, so the focus outline draws over it.
    let shown = widgets::SliderValue::field(d.f64("angle", 0.0), -180.0..=180.0, "°", 0);
    if let Some(a) = widgets::stepped_area(ui, &response, tl!("Angle"), Some(shown)) {
        d.fields.insert("angle".into(), json!(a));
    }
    let a = d.f64("angle", 0.0).clamp(-180.0, 180.0).to_radians();
    // The long axis on screen (y down: a positive angle tilts it up to the right) and the short one.
    let u = egui::vec2(a.cos() as f32, -a.sin() as f32);
    let v = egui::vec2(-u.y, u.x);
    let long = 18.0;
    let short = long * blob_tip_short_ratio(d.f64("roundness", 100.0));
    let dab = |center: egui::Pos2, color: egui::Color32| {
        let points = (0..32)
            .map(|i| {
                let s = std::f32::consts::TAU * i as f32 / 32.0;
                center + u * (long * s.cos()) + v * (short * s.sin())
            })
            .collect();
        painter.add(egui::Shape::convex_polygon(points, color, egui::Stroke::NONE));
    };
    let center = square.center();
    dab(center, t.text);
    painter.line_segment([center, center + u * 25.0], egui::Stroke::new(1.0, t.accent));
    let strip = square.right() + 2.5 * long;
    for (i, color) in [t.text_dim, t.text, t.text_dim].into_iter().enumerate() {
        dab(egui::pos2(strip + i as f32 * 1.5 * long, center.y), color);
    }
}

/// The preview tip's short axis as a fraction of its long axis: Roundness in percent, with 0
/// percent drawn as 0.1 percent, the minimum the brush draws with.
fn blob_tip_short_ratio(roundness: f64) -> f32 {
    (roundness.clamp(0.0, 100.0) as f32 / 100.0).max(0.001)
}

/// OK: set the tool's options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let values: Map<String, Value> = KEYS.iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect();
    run_and_close(app, "tool.setOption", json!({ "tool": d.str("tool"), "values": values }))
}

#[cfg(test)]
mod tests;
