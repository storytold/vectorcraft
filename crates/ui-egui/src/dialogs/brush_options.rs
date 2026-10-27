//! Brush Options (double-click a brush in the Brushes panel, or the panel menu's Brush Options…).
//! Calligraphic Brush Options: Name, then Angle, Roundness and Size, each Fixed, Random or
//! Pressure with its Variation, over live previews of the nib and of a stroke (light, heavy, light
//! pressure). Bristle Brush Options: Name (typing stops at [`Bristle::MAX_NAME_CHARS`]
//! characters), Shape and the bristle sliders. Scatter, Art and Pattern Brush Options: see
//! [`super::brush_art_options`]. OK runs `brush.options` (strokes painted with the brush update);
//! a name another brush has, or a new bristle brush name that is too long, keeps the dialog open.
//!
//! Fields: `__brush` (the brush's name), `__type` (`calligraphic` | `bristle` | `scatter` | `art` |
//! `pattern`), `name`.
//! Calligraphic: `angle` (−180..180°), `roundness` (0..100 %), `size` (0..1296 pt), and per value
//! `angleMode`, `roundnessMode`, `sizeMode` (`fixed` | `random` | `pressure`) and
//! `angleVariation`, `roundnessVariation`, `sizeVariation`. Bristle: `shape` (`roundPoint`…),
//! `size` (pt), `length` (25..300 %), `density`, `thickness`, `opacity`, `stiffness` (1..100 %).
//!
//! New Brush opens the dialog for a brush that does not exist yet ([`open_new`]), with `__create`
//! and, for dropped art, `__ids`: OK runs `brush.new` instead, which gives a name another brush has
//! a number, and Cancel adds nothing.

use egui::{Color32, Sense, Stroke, pos2, vec2};
use serde_json::{Map, Value, json};
use vectorcraft_brush::{Bristle, Brush, BrushKind, Calligraphic, Variation};

use super::swatch_options::{grid, label};
use super::{DialogSpec, brush_art_options, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Brush Options.
pub const KIND: &str = "brushOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm: ok, min_width: 440.0, inputs: |_| &["tips"], ..DialogSpec::FORM };

/// The field of a Brush Options dialog that New Brush opened.
const CREATE: &str = "__create";

/// The field of such a dialog holding the ids of the art dropped on the Brushes panel.
const IDS: &str = "__ids";

/// Calligraphic values: (field, label, suffix, decimals), in [`Calligraphic::RANGES`] order.
const CALLI: [(&str, &str, &str, usize); 3] = [("angle", "Angle:", "°", 0), ("roundness", "Roundness:", "%", 0), ("size", "Size:", " pt", 1)];

/// Bristle sliders: (field, label, range), in percent.
const BRISTLE: [(&str, &str, (f64, f64)); 5] = [
    ("length", "Bristle Length:", Bristle::LENGTH_RANGE),
    ("density", "Bristle Density:", Bristle::PERCENT_RANGE),
    ("thickness", "Bristle Thickness:", Bristle::PERCENT_RANGE),
    ("opacity", "Paint Opacity:", Bristle::PERCENT_RANGE),
    ("stiffness", "Stiffness:", Bristle::PERCENT_RANGE),
];

/// Bristle tip shapes: (value, label).
const SHAPES: [(&str, &str); 10] = [
    ("roundPoint", "Round Point"),
    ("flatPoint", "Flat Point"),
    ("roundBlunt", "Round Blunt"),
    ("flatBlunt", "Flat Blunt"),
    ("roundCurve", "Round Curve"),
    ("flatCurve", "Flat Curve"),
    ("roundAngle", "Round Angle"),
    ("flatAngle", "Flat Angle"),
    ("roundFan", "Round Fan"),
    ("flatFan", "Flat Fan"),
];

/// Width of the bristle dialog's label column.
const LABEL_W: f32 = 112.0;

/// Widths of the nib preview and of the whole preview row.
const NIB_W: f32 = 120.0;
const STROKE_W: f32 = 400.0;

fn heading(d: &Dialog) -> String {
    match d.str("__type").as_str() {
        "bristle" => tl!("Bristle Brush Options").into(),
        "calligraphic" => tl!("Calligraphic Brush Options").into(),
        ty => brush_art_options::heading(ty).into(),
    }
}

pub(super) fn mode_key(key: &str) -> String {
    format!("{key}Mode")
}

fn variation_key(key: &str) -> String {
    format!("{key}Variation")
}

/// Open the options of brush `name` (default: the selected path's brush, else the current one).
pub fn open(app: &mut VectorcraftApp, name: Option<&str>) -> Result<Value, String> {
    let name = match name {
        Some(n) => n.to_string(),
        None => target(app).ok_or("no brush selected")?,
    };
    let def = app.run("brush.get", json!({ "name": name }))?;
    let b: Brush = serde_json::from_value(def.clone()).map_err(|e| e.to_string())?;
    let mut f = fields(&b, def)?;
    f.insert("__brush".into(), json!(b.name));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields: f });
    Ok(Value::Null)
}

/// The fields of brush `b`, whose definition is `def`.
fn fields(b: &Brush, def: Value) -> Result<Map<String, Value>, String> {
    let mut f = Map::new();
    f.insert("name".into(), json!(b.name));
    f.insert("__type".into(), json!(b.kind.type_id()));
    match &b.kind {
        BrushKind::Calligraphic(c) => {
            let values = [c.angle, c.roundness, c.size];
            for (i, (key, ..)) in CALLI.iter().enumerate() {
                let (Some(v), Some(var), Some(mode)) = (values.get(i), c.variation.get(i), c.modes().get(i).copied()) else { continue };
                f.insert((*key).into(), json!(v));
                f.insert(variation_key(key), json!(var));
                f.insert(mode_key(key), json!(mode.id()));
            }
        }
        BrushKind::Bristle(b) => {
            let v = serde_json::to_value(b).map_err(|e| e.to_string())?;
            for key in ["shape", "size"].into_iter().chain(BRISTLE.map(|r| r.0)) {
                f.insert(key.into(), v.get(key).cloned().unwrap_or(Value::Null));
            }
        }
        // The definition goes along, for the preview (and the art it carries).
        BrushKind::Scatter(_) | BrushKind::Art(_) | BrushKind::Pattern(_) => brush_art_options::fields(b, def, &mut f),
    }
    Ok(f)
}

/// Can Brush Options open without a name (the selected path's brush, else the current one, exists)?
pub fn available(app: &VectorcraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    target(app).is_some_and(|n| vectorcraft_brush::find(&st.doc, &n).is_some())
}

/// The brush Brush Options opens on without a name: the selected path's brush, else the current one.
fn target(app: &VectorcraftApp) -> Option<String> {
    let st = app.session.active()?;
    let selected = st.selection.subjects().first().and_then(|id| st.doc.node(*id)).and_then(|n| n.appearance.stroke()?.brush.clone());
    selected.or_else(|| vectorcraft_brush::current(&st.doc))
}

/// Open the Brush Options of a new brush of type `ty` (`calligraphic` | `scatter` | `art` |
/// `bristle` | `pattern`), the second step of New Brush. The fields start from the type's default
/// values and the name "New <Type> Brush", numbered when another brush has it. A scatter or art
/// brush takes the art `brush.selectionArt` gives for `ids` (else for the selection) as its art,
/// and a pattern brush takes it as its side tile; the preview draws it. Without that art nothing
/// opens.
pub(super) fn open_new(app: &mut VectorcraftApp, ty: &str, ids: Option<Value>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let title: String = ty.chars().take(1).flat_map(char::to_uppercase).chain(ty.chars().skip(1)).collect();
    let name = vectorcraft_brush::unique_name(&vectorcraft_brush::library(&st.doc), &format!("New {title} Brush"));
    let mut def = json!({ "name": name, "type": ty });
    let art_key = match ty {
        "scatter" | "art" => Some("art"),
        "pattern" => Some("side"),
        _ => None,
    };
    if let Some(key) = art_key {
        let mut r = app.run("brush.selectionArt", ids.clone().map_or_else(|| json!({}), |ids| json!({ "ids": ids })))?;
        let art = r["art"].take();
        if art.is_null() {
            return Err("nothing selected".into());
        }
        def[key] = art;
    }
    let mut b: Brush = serde_json::from_value(def).map_err(|e| e.to_string())?;
    b.sanitize();
    let def = serde_json::to_value(&b).map_err(|e| e.to_string())?;
    let mut f = fields(&b, def)?;
    f.insert(CREATE.into(), json!(true));
    if let Some(ids) = ids {
        f.insert(IDS.into(), ids);
    }
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields: f });
    Ok(Value::Null)
}

/// OK of a new brush's Brush Options: `brush.new` with the type, the name, the values set, the art
/// the dialog opened with (`art` or `side` of `__def`, which the preview draws) and the dropped
/// art's ids. A selection changed or cleared while the dialog is open does not change the art. The
/// dialog closes once the brush is made.
fn create(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut brush_params = params(d);
    if let Some(def) = d.fields.get("__def") {
        for key in ["art", "side"] {
            if let Some(art) = def.get(key).filter(|v| !v.is_null()) {
                brush_params[key] = art.clone();
            }
        }
    }
    let mut p = json!({ "type": d.str("__type"), "name": d.str("name").trim(), "params": brush_params });
    if let Some(ids) = d.fields.get(IDS) {
        p["ids"] = ids.clone();
    }
    let r = app.run("brush.new", p);
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

/// OK: make the new brush when New Brush opened the dialog, else edit the brush.
fn ok(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.bool(CREATE) { create(app, d) } else { confirm(app, d) }
}

/// The value of field `key` within `range`.
pub(super) fn clamped(d: &Dialog, key: &str, (lo, hi): (f64, f64)) -> f64 {
    d.f64(key, lo).clamp(lo, hi)
}

/// Is `d` Scatter, Art or Pattern Brush Options with Colorization Tips open? Escape then closes
/// Tips, not the dialog.
pub(crate) fn tips_open(d: &Dialog) -> bool {
    d.kind == KIND && d.bool("tips") && !matches!(d.str("__type").as_str(), "bristle" | "calligraphic")
}

/// The Calligraphic brush the fields describe.
fn calligraphic(d: &Dialog) -> Calligraphic {
    let value = |i: usize| CALLI.get(i).zip(Calligraphic::RANGES.get(i)).map_or(0.0, |((key, ..), r)| clamped(d, key, *r));
    let variation =
        |i: usize| CALLI.get(i).zip(Calligraphic::MAX_VARIATION.get(i)).map_or(0.0, |((key, ..), max)| clamped(d, &variation_key(key), (0.0, *max)));
    let mode = |i: usize| CALLI.get(i).and_then(|(key, ..)| Variation::parse(&d.str(&mode_key(key)))).unwrap_or_default();
    Calligraphic { angle: value(0), roundness: value(1), size: value(2), variation: [0, 1, 2].map(variation), modes: Some([0, 1, 2].map(mode)) }
}

/// `brush.options` params from the fields.
fn params(d: &Dialog) -> Value {
    match d.str("__type").as_str() {
        "bristle" => {
            let mut p = json!({ "shape": d.str("shape"), "size": clamped(d, "size", Bristle::SIZE_RANGE) });
            for (key, _, range) in BRISTLE {
                p[key] = json!(clamped(d, key, range));
            }
            p
        }
        "calligraphic" => serde_json::to_value(calligraphic(d)).unwrap_or_default(),
        _ => brush_art_options::params(d),
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    grid(ui, |ui| {
        label(ui, tl!("Name:"));
        let max = if d.str("__type") == "bristle" { Bristle::MAX_NAME_CHARS } else { usize::MAX };
        form::text_edit_limited(ui, d, "name", 240.0, max);
        ui.end_row();
    });
    ui.add_space(10.0);
    match d.str("__type").as_str() {
        "bristle" => bristle_body(app, ui, d),
        "calligraphic" => calligraphic_body(app, ui, d),
        _ => brush_art_options::body(app, ui, d),
    }
    false
}

fn calligraphic_body(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let c = calligraphic(d);
    ui.horizontal(|ui| {
        nib_preview(ui, &c, vec2(NIB_W, 84.0));
        ui.add_space(8.0);
        stroke_preview(app, ui, "calligraphic", serde_json::to_value(&c).unwrap_or_default(), vec2(STROKE_W - NIB_W - 8.0, 84.0));
    });
    ui.add_space(12.0);
    egui::Grid::new("brush-options-calli").num_columns(5).spacing([8.0, 8.0]).show(ui, |ui| {
        for (i, (key, text, suffix, decimals)) in CALLI.into_iter().enumerate() {
            let (Some(&range), Some(&max)) = (Calligraphic::RANGES.get(i), Calligraphic::MAX_VARIATION.get(i)) else { continue };
            label(ui, tl!(text));
            if let Some(v) = widgets::range_field(ui, ("brush-value", key), clamped(d, key, range), range.0..=range.1, suffix, decimals, 64.0) {
                d.fields.insert(key.into(), json!(v));
            }
            let mode = Variation::parse(&d.str(&mode_key(key))).unwrap_or_default();
            let labels = Variation::ALL.map(Variation::label);
            if let Some(m) = widgets::dropdown(ui, ("brush-mode", key), mode.label(), &labels, 104.0).and_then(|i| Variation::ALL.get(i)) {
                d.fields.insert(mode_key(key), json!(m.id()));
            }
            label(ui, tl!("Variation:"));
            let vkey = variation_key(key);
            ui.add_enabled_ui(mode != Variation::Fixed, |ui| {
                if let Some(v) = widgets::range_field(ui, ("brush-var", key), clamped(d, &vkey, (0.0, max)), 0.0..=max, suffix, decimals, 64.0) {
                    d.fields.insert(vkey.clone(), json!(v));
                }
            });
            ui.end_row();
        }
    });
}

fn bristle_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    form::choice(ui, d, "shape", tl!("Shape:"), (LABEL_W, 150.0), &SHAPES);
    widgets::label_row(ui, tl!("Size:"), LABEL_W, |ui| {
        form::length(ui, d, "size", app.session.general_unit(), 80.0);
    });
    let rail = Tokens::get(ui.ctx()).input_border;
    for (key, text, (lo, hi)) in BRISTLE {
        widgets::label_row(ui, tl!(text), LABEL_W, |ui| form::slider_field_named(ui, d, key, tl!(text), lo..=hi, "%", &|_| rail));
    }
    ui.add_space(8.0);
    let p = params(d);
    stroke_preview(app, ui, "bristle", p, vec2(STROKE_W, 64.0));
}

/// A stroke painted with the brush the fields describe (type `ty`, definition fields `def`), with
/// the active document's resources (the patterns, symbols… its art uses).
pub(super) fn stroke_preview(app: &VectorcraftApp, ui: &mut egui::Ui, ty: &str, def: Value, size: egui::Vec2) {
    let mut def = match def {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    def.insert("name".into(), json!("preview"));
    def.insert("type".into(), json!(ty));
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    crate::panels::brushes::definition_chip(ui, r, app.session.active(), &Value::Object(def));
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, Tokens::get(ui.ctx()).input_border), egui::StrokeKind::Inside);
}

/// The nib of `c` at its values (black), and — for values that vary — at their lightest and
/// heaviest (grey), on white like the brush previews, its largest nib filling the box.
fn nib_preview(ui: &mut egui::Ui, c: &Calligraphic, size: egui::Vec2) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(r);
    p.rect_filled(r, 0.0, Color32::WHITE);
    let guide = Stroke::new(1.0, Color32::from_gray(220));
    p.line_segment([pos2(r.left(), r.center().y), pos2(r.right(), r.center().y)], guide);
    p.line_segment([pos2(r.center().x, r.top()), pos2(r.center().x, r.bottom())], guide);
    let values = [c.angle, c.roundness, c.size];
    let at = |sign: f64| -> [f64; 3] {
        let mut v = values;
        for (i, (x, mode)) in v.iter_mut().zip(c.modes()).enumerate() {
            let (Some(&var), Some(&(lo, hi))) = (c.variation.get(i), Calligraphic::RANGES.get(i)) else { continue };
            if mode != Variation::Fixed {
                *x = (*x + sign * var).clamp(lo, hi);
            }
        }
        v
    };
    let nibs = [at(-1.0), at(1.0), values];
    let largest = nibs.iter().map(|n| n[2]).fold(0.0, f64::max).max(1e-6);
    let scale = (r.height().min(r.width()) - 12.0) / largest as f32;
    for (k, [angle, round, size]) in nibs.into_iter().enumerate() {
        let fill = if k < 2 { Color32::from_gray(200) } else { Color32::BLACK };
        let (a, b) = ((size as f32 * scale / 2.0).max(0.5), (size * round.clamp(1.0, 100.0) / 100.0) as f32 * scale / 2.0);
        let th = (angle as f32).to_radians();
        let (u, v) = (vec2(th.cos(), -th.sin()), vec2(th.sin(), th.cos()));
        let pts: Vec<egui::Pos2> = (0..48)
            .map(|i| {
                let s = i as f32 / 48.0 * std::f32::consts::TAU;
                r.center() + u * (a * s.cos()) + v * (b.max(0.5) * s.sin())
            })
            .collect();
        p.add(egui::Shape::convex_polygon(pts, fill, Stroke::NONE));
    }
    p.rect_stroke(r, 0.0, Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
}

/// OK: edit the brush (and rename it). A name another brush has, or a new bristle brush name that
/// is too long, keeps the dialog open.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let name = d.str("name").trim().to_string();
    let r = app.run("brush.options", json!({ "name": d.str("__brush"), "newName": name, "params": params(d) }));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

#[cfg(test)]
mod tests;
