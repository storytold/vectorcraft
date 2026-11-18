//! Tool Options of the freehand tools (double-click Pencil, Paintbrush, Smooth, Blob Brush or
//! Eraser): the options each tool keeps, which last across tool switches and are saved with the
//! preferences. OK sets them with `tool.setOption {tool, values}`. The Pencil also has `liveFit`
//! (Live curve fitting, on: its preview fits the stroke while it is drawn).
//!
//! Fields: `tool`; `fidelity` (screen pixels for the Pencil, Paintbrush and Smooth, points for the
//! Blob Brush), `fill` (Pencil and
//! Paintbrush), `keepSelected` (Pencil, Paintbrush and Blob Brush), `mergeOnlySelection` (Blob
//! Brush), `closeWithin` and `editWithin` (screen pixels, 0 turns it off; Pencil and Paintbrush),
//! `angle` (degrees) and `roundness` (percent; Blob Brush and Eraser), `livePreview` (Pencil: the stroke shows
//! with its fill and stroke while it is drawn, or as a thin line until the release), `size`
//! (points; Blob Brush and Eraser), and `sizeMode` and `sizeVariation` (points; Blob Brush and
//! Eraser: with Pressure the tip is Size − Variation wide at the lightest pen pressure and Size +
//! Variation at the heaviest). The Blob Brush's `sizeMode` is `fixed` or `pressure`. The Eraser's
//! Angle, Roundness and Size each have a menu (`angleMode`, `roundnessMode`, `sizeMode`: `fixed`,
//! `random`, `pressure`, `tilt`, `bearing` or `rotation`) before the field and a Variation field
//! under it (`angleVariation` in degrees, `roundnessVariation` in percent, `sizeVariation`),
//! greyed while the menu is Fixed.

use serde_json::{Map, Value, json};
use vectorcraft_brush::Variation;

use super::document_setup::check;
use super::{DialogSpec, form, set_tool_options_and_close};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the freehand tools' Tool Options.
pub const KIND: &str = "freehandOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: CONTENT_W, ..DialogSpec::FORM };

/// The tools the dialog serves.
pub const TOOLS: [&str; 5] = ["pencil", "paintbrush", "smooth", "blobBrush", "eraser"];

/// The options the dialog edits, in the order it shows them (a tool has those it reports).
const KEYS: [&str; 18] = [
    "fidelity",
    "angle",
    "angleMode",
    "angleVariation",
    "roundness",
    "roundnessMode",
    "roundnessVariation",
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

/// The top of the Fidelity slider, in the tool's unit (screen pixels for the Pencil, Paintbrush
/// and Smooth, points for the Blob Brush); it starts at the lowest Fidelity the tool takes,
/// and the field takes up to 100.
const FIDELITY_MAX: f64 = 20.0;

/// Fidelity `v` stepped `n` steps by the slider's and the field's keys: an arrow steps hundredths up to 0.1,
/// tenths up to 1 and whole units above, in the tool's unit (a step down from 0.1 or 1 takes the finer step below
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

/// The unit of tool `tool`'s Fidelity. The Pencil's, the Paintbrush's and the Smooth tool's
/// Fidelity is on screen, the same at any zoom (#5729); the Blob Brush's is in points.
fn fidelity_unit(tool: &str) -> &'static str {
    if matches!(tool, "pencil" | "paintbrush" | "smooth") { "px" } else { "pt" }
}

/// Width of the Fidelity field.
pub(crate) const FIDELITY_FIELD_W: f32 = FIELD_W;

/// The Fidelity field of tool `tool`, showing `v` to two decimals from the tool's lowest Fidelity to
/// 100 and stepping by [`fidelity_step`], named by the label drawn just before it. Returns the value
/// typed or stepped to. The Contextual Task Bar shows the Smooth tool's.
pub(crate) fn fidelity_field(ui: &mut egui::Ui, id: &str, v: f64, tool: &str) -> Option<f64> {
    let min = vectorcraft_tools::draw2::min_fidelity(tool);
    widgets::range_field_stepped(ui, id, v, min..=100.0, fidelity_unit(tool), 2, FIELD_W, fidelity_step)
}

/// Where Fidelity `v` sits on a slider over `(min, max)`, from 0 to 1. The rail is logarithmic, so
/// the low values, where the stroke is followed closely, get as much of it as the high ones
/// (0.01–1 takes most of a 0.01–20 rail, in the tool's unit).
fn fidelity_at(v: f64, (min, max): (f64, f64)) -> f32 {
    if !(v.is_finite() && min > 0.0 && max > min) {
        return 0.0;
    }
    ((v.clamp(min, max) / min).ln() / (max / min).ln()) as f32
}

/// The Fidelity at `x` (0 to 1) on a slider over `(min, max)`: hundredths below 1, tenths above, in the tool's unit.
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

/// Width of the Blob Brush's and the Eraser's variation menus.
const MODE_W: f32 = 96.0;

/// A tip value with a variation menu: its field.
struct Varied<'a> {
    /// The value's key; the menu's is `<key>Mode` and the Variation's `<key>Variation`.
    key: &'a str,
    label: &'a str,
    unit: &'a str,
    decimals: usize,
    range: (f64, f64),
    /// The value's default.
    default: f64,
    /// The Variation field's range.
    variation: (f64, f64),
}

/// The rows of a tip value with a variation menu: the value with its menu of `modes` before the
/// field, then the Variation field the menu's modes other than Fixed take (greyed while the
/// menu is Fixed). The Blob Brush's Size offers Fixed and Pressure, and the Eraser's Angle,
/// Roundness and Size every mode.
fn varied(ui: &mut egui::Ui, d: &mut Dialog, v: &Varied, modes: &[Variation]) {
    let (mode_key, var_key) = (format!("{}Mode", v.key), format!("{}Variation", v.key));
    let mode = Variation::parse(&d.str(&mode_key)).filter(|m| modes.contains(m)).unwrap_or_default();
    let labels: Vec<&str> = modes.iter().map(|m| m.label()).collect();
    let menu_label_w = label_w(ui) - MODE_W - ui.spacing().item_spacing.x;
    let value = d.f64(v.key, v.default);
    widgets::label_row(ui, v.label, menu_label_w, |ui| {
        if let Some(m) = widgets::dropdown(ui, ("freehand-mode", v.key), mode.label(), &labels, MODE_W).and_then(|i| modes.get(i)) {
            d.fields.insert(mode_key.clone(), json!(m.id()));
        }
        if let Some(x) = widgets::range_field(ui, ("freehand-field", v.key), value, v.range.0..=v.range.1, v.unit, v.decimals, FIELD_W) {
            d.fields.insert(v.key.into(), json!(x));
        }
    });
    ui.add_enabled_ui(mode != Variation::Fixed, |ui| {
        let label_w = label_w(ui);
        let x = d.f64(&var_key, 0.0);
        widgets::label_row(ui, tl!("Variation:"), label_w, |ui| {
            if let Some(x) =
                widgets::range_field(ui, ("freehand-field", var_key.as_str()), x, v.variation.0..=v.variation.1, v.unit, v.decimals, FIELD_W)
            {
                d.fields.insert(var_key.clone(), json!(x));
            }
        });
    });
}

/// The Blob Brush's and the Eraser's Size: [`varied`] in points.
fn size_rows(ui: &mut egui::Ui, d: &mut Dialog, modes: &[Variation]) {
    let size = Varied { key: "size", label: tl!("Size:"), unit: "pt", decimals: 1, range: (0.1, 1000.0), default: 10.0, variation: (0.0, 1000.0) };
    varied(ui, d, &size, modes);
}

/// The Fidelity rows: the label on a line of its own, then a logarithmic slider from the tool's
/// lowest Fidelity to [`FIDELITY_MAX`] across the row with the field at its end, both to two
/// decimals, and Accurate and Smooth under the slider's ends.
fn fidelity(ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let v = d.f64("fidelity", 1.5);
    let tool = d.str("tool");
    let rail = (vectorcraft_tools::draw2::min_fidelity(&tool), FIDELITY_MAX);
    let unit = fidelity_unit(&tool);
    let rail_w = label_w(ui);
    let mut new = None;
    ui.label(egui::RichText::new(tl!("Fidelity:")).size(12.5).color(t.text));
    ui.horizontal(|ui| {
        let shown = widgets::SliderValue::field(v, rail.0..=rail.1, unit, 2).with_step(fidelity_step);
        let (x, _, keyed) =
            widgets::color_slider_stepped(ui, "freehand-fidelity", tl!("Fidelity:"), Some(shown), fidelity_at(v, rail), rail_w, &|_| t.input_border);
        new = x.map(|x| fidelity_from(x, rail)).or(keyed);
        if let Some(x) = fidelity_field(ui, "freehand-fidelity-field", v, &tool) {
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
    let eraser = d.fields.contains_key("angleMode");
    if eraser {
        // The Eraser: Angle, Roundness and Size each with every variation mode.
        let angle =
            Varied { key: "angle", label: tl!("Angle:"), unit: "°", decimals: 0, range: (-180.0, 180.0), default: 0.0, variation: (0.0, 180.0) };
        varied(ui, d, &angle, &Variation::ALL);
        let roundness = Varied {
            key: "roundness",
            label: tl!("Roundness:"),
            unit: "%",
            decimals: 0,
            range: (1.0, 100.0),
            default: 100.0,
            variation: (0.0, 100.0),
        };
        varied(ui, d, &roundness, &Variation::ALL);
        size_rows(ui, d, &Variation::ALL);
    } else {
        if d.fields.contains_key("angle") {
            number(ui, d, ("angle", tl!("Angle:")), "°", 0, (-180.0, 180.0));
        }
        if d.fields.contains_key("roundness") {
            let min = if d.str("tool") == "blobBrush" { 0.0 } else { 1.0 };
            number(ui, d, ("roundness", tl!("Roundness:")), "%", 0, (min, 100.0));
        }
    }
    if d.str("tool") == "blobBrush" {
        blob_tip_preview(ui, d);
    }
    // The Eraser's Size rows are above.
    if !eraser && d.fields.contains_key("sizeMode") {
        size_rows(ui, d, &[Variation::Fixed, Variation::Pressure]);
    } else if !eraser && d.fields.contains_key("size") {
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

/// Half the length of the Blob Brush tip in its preview, in points.
const TIP_LONG: f32 = 18.0;

/// How far from a Roundness dot of the tip preview a press still takes the dot, in points.
const TIP_DOT_REACH: f32 = 8.0;

/// The Blob Brush's tip preview: a square with the tip at its Angle and Roundness, a line with an
/// arrowhead along its long axis and a black dot at each end of its short axis, and beside it three
/// dabs of the tip, the middle one darker. A click or a drag that starts on a dot sets the
/// Roundness from the pointer's distance from the center along the short axis
/// ([`blob_tip_roundness_at`]) until the release, inside the square or out of it, and gives the
/// Roundness slider the keyboard focus. A click or a drag that starts anywhere else in the square
/// points the Angle at the pointer and gives the Angle slider the focus. Screen readers get the
/// square as the Angle slider and then the Roundness slider, and their keys step the Angle within
/// -180° to 180° and the Roundness within 0% to 100% as a dialog slider's do
/// ([`widgets::stepped_area`]).
fn blob_tip_preview(ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(CONTENT_W, TIP_PREVIEW), egui::Sense::hover());
    let square = egui::Rect::from_min_size(rect.min, egui::vec2(TIP_PREVIEW, TIP_PREVIEW));
    let center = square.center();
    let response = ui.interact(square, ui.scope_id().with("freehand-tip-angle"), egui::Sense::click_and_drag());
    // Whether the press that started this click or drag was on a dot: it is kept until the release.
    let on_dot = response.id.with("on-dot");
    let near_dot = |p: egui::Pos2, d: &Dialog| {
        blob_tip_dots(center, d.f64("angle", 0.0), d.f64("roundness", 100.0)).iter().any(|dot| dot.distance(p) <= TIP_DOT_REACH)
    };
    let mut started = ui.data(|m| m.get_temp::<bool>(on_dot));
    if started.is_none() && response.is_pointer_button_down_on() {
        let pressed = ui.input(|i| i.pointer.press_origin()).is_some_and(|p| near_dot(p, d));
        ui.data_mut(|m| m.insert_temp(on_dot, pressed));
        started = Some(pressed);
    }
    // A click whose press and release came in the same frame: its press is among this frame's events.
    let dot_drag = started.unwrap_or_else(|| {
        let press = ui.input(|i| {
            i.events.iter().rev().find_map(|e| match e {
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => Some(*pos),
                _ => None,
            })
        });
        response.clicked() && press.is_some_and(|p| near_dot(p, d))
    });
    if !response.is_pointer_button_down_on() {
        ui.data_mut(|m| m.remove_temp::<bool>(on_dot));
    }
    if let Some((p, _)) = widgets::pointer_phase(&response) {
        let delta = p - center;
        if dot_drag {
            d.fields.insert("roundness".into(), json!(blob_tip_roundness_at(delta, d.f64("angle", 0.0))));
        } else if delta.length_sq() > 1.0 {
            d.fields.insert("angle".into(), json!(f64::from((-delta.y).atan2(delta.x)).to_degrees().round()));
        }
    }
    if dot_drag && response.is_pointer_button_down_on() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if response.hover_pos().is_some_and(|p| near_dot(p, d)) && !response.is_pointer_button_down_on() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(square, 2.0, t.input);
    painter.rect_stroke(square, 2.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    // After the square's frame, so the focus outline draws over it.
    let shown = widgets::SliderValue::field(d.f64("angle", 0.0), -180.0..=180.0, "°", 0);
    if let Some(a) = widgets::stepped_area(ui, &response, tl!("Angle"), Some(shown)) {
        d.fields.insert("angle".into(), json!(a));
    }
    // The Roundness slider takes the focus and the keys only: the square's response reads the pointer.
    let roundness = ui.interact(square, ui.scope_id().with("freehand-tip-roundness"), egui::Sense::focusable_noninteractive());
    if dot_drag && (response.drag_started() || response.clicked()) {
        widgets::focus_by_pointer(ui, roundness.id);
    }
    let shown = widgets::SliderValue::field(d.f64("roundness", 100.0), 0.0..=100.0, "%", 0);
    if let Some(r) = widgets::stepped_area(ui, &roundness, tl!("Roundness"), Some(shown)) {
        d.fields.insert("roundness".into(), json!(r));
    }
    let (u, v) = blob_tip_axes(d.f64("angle", 0.0));
    let long = TIP_LONG;
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
    dab(center, t.text);
    painter.line_segment([center, center + u * 22.0], egui::Stroke::new(1.0, t.accent));
    let arrowhead = vec![center + u * 28.0, center + u * 22.0 + v * 3.0, center + u * 22.0 - v * 3.0];
    painter.add(egui::Shape::convex_polygon(arrowhead, t.accent, egui::Stroke::NONE));
    // Black with a white rim, so a dot shows on the tip and on the square in either theme.
    for dot in blob_tip_dots(center, d.f64("angle", 0.0), d.f64("roundness", 100.0)) {
        painter.circle(dot, 3.0, egui::Color32::BLACK, egui::Stroke::new(1.0, egui::Color32::WHITE));
    }
    let strip = square.right() + 2.5 * long;
    for (i, color) in [t.text_dim, t.text, t.text_dim].into_iter().enumerate() {
        dab(egui::pos2(strip + i as f32 * 1.5 * long, center.y), color);
    }
}

/// The tip preview's long axis on screen at Angle `angle` (degrees, y down: a positive angle tilts
/// it up to the right), and its short axis.
fn blob_tip_axes(angle: f64) -> (egui::Vec2, egui::Vec2) {
    let a = angle.clamp(-180.0, 180.0).to_radians();
    let u = egui::vec2(a.cos() as f32, -a.sin() as f32);
    (u, egui::vec2(-u.y, u.x))
}

/// The two Roundness dots of the tip preview centered on `center`: the ends of the tip's short
/// axis at Angle `angle` and Roundness `roundness`.
fn blob_tip_dots(center: egui::Pos2, angle: f64, roundness: f64) -> [egui::Pos2; 2] {
    let (_, v) = blob_tip_axes(angle);
    let short = v * (TIP_LONG * blob_tip_short_ratio(roundness));
    [center + short, center - short]
}

/// The Roundness, in whole percent from 0 to 100, that a dot of the tip preview dragged to `offset`
/// from the preview's center gives at Angle `angle`: the distance along the tip's short axis, on
/// either side of the center, as a percentage of half the tip's length.
fn blob_tip_roundness_at(offset: egui::Vec2, angle: f64) -> f64 {
    let (_, v) = blob_tip_axes(angle);
    (f64::from(offset.dot(v).abs() / TIP_LONG) * 100.0).round().clamp(0.0, 100.0)
}

/// The preview tip's short axis as a fraction of its long axis: Roundness in percent, with 0
/// percent drawn as 0.1 percent, the minimum the brush draws with.
fn blob_tip_short_ratio(roundness: f64) -> f32 {
    (roundness.clamp(0.0, 100.0) as f32 / 100.0).max(0.001)
}

/// OK: set the tool's options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let values: Map<String, Value> = KEYS.iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect();
    set_tool_options_and_close(app, json!({ "tool": d.str("tool"), "values": values }))
}

#[cfg(test)]
mod tests;
