//! Brush Options (double-click a brush in the Brushes panel, or the panel menu's Brush Options…).
//! Calligraphic Brush Options: Name, then Angle, Roundness and Size, each Fixed, Random or
//! Pressure with its Variation, over live previews of the nib and of a stroke (light, heavy, light
//! pressure). Bristle Brush Options: Name, Shape and the bristle sliders. OK runs `brush.options`
//! (strokes painted with the brush update); a name another brush has keeps the dialog open.
//!
//! Fields: `__brush` (the brush's name), `__type` (`calligraphic` | `bristle`), `name`.
//! Calligraphic: `angle` (−180..180°), `roundness` (0..100 %), `size` (0..1296 pt), and per value
//! `angleMode`, `roundnessMode`, `sizeMode` (`fixed` | `random` | `pressure`) and
//! `angleVariation`, `roundnessVariation`, `sizeVariation`. Bristle: `shape` (`roundPoint`…),
//! `size` (pt), `length` (25..300 %), `density`, `thickness`, `opacity`, `stiffness` (1..100 %).

use egui::{Color32, Sense, Stroke, pos2, vec2};
use serde_json::{Map, Value, json};
use vectorcraft_brush::{Brush, BrushKind, Calligraphic, Variation};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Brush Options.
pub const KIND: &str = "brushOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 440.0, ..DialogSpec::FORM };

/// The brush types that have an options dialog.
pub const TYPES: [&str; 2] = ["calligraphic", "bristle"];

/// Calligraphic values: (field, label, suffix, decimals), in [`Calligraphic::RANGES`] order.
const CALLI: [(&str, &str, &str, usize); 3] = [("angle", "Angle:", "°", 0), ("roundness", "Roundness:", "%", 0), ("size", "Size:", " pt", 1)];

/// Bristle sliders: (field, label, min, max), in percent.
const BRISTLE: [(&str, &str, f64, f64); 5] = [
    ("length", "Bristle Length:", 25.0, 300.0),
    ("density", "Bristle Density:", 1.0, 100.0),
    ("thickness", "Bristle Thickness:", 1.0, 100.0),
    ("opacity", "Paint Opacity:", 1.0, 100.0),
    ("stiffness", "Stiffness:", 1.0, 100.0),
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
        _ => tl!("Calligraphic Brush Options").into(),
    }
}

fn mode_key(key: &str) -> String {
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
    let b: Brush = serde_json::from_value(def).map_err(|e| e.to_string())?;
    let mut f = Map::new();
    f.insert("__brush".into(), json!(b.name));
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
        _ => return Err("this kind of brush has no options dialog yet".into()),
    }
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields: f });
    Ok(Value::Null)
}

/// Can Brush Options open without a name (its brush has a type with a dialog)?
pub fn available(app: &VectorcraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    target(app).and_then(|n| vectorcraft_brush::find(&st.doc, &n)).is_some_and(|b| TYPES.contains(&b.kind.type_id()))
}

/// The brush Brush Options opens on without a name: the selected path's brush, else the current one.
fn target(app: &VectorcraftApp) -> Option<String> {
    let st = app.session.active()?;
    let selected = st.selection.subjects().first().and_then(|id| st.doc.node(*id)).and_then(|n| n.appearance.stroke()?.brush.clone());
    selected.or_else(|| vectorcraft_brush::current(&st.doc))
}

/// The value of field `key` within `range`.
fn clamped(d: &Dialog, key: &str, (lo, hi): (f64, f64)) -> f64 {
    d.f64(key, lo).clamp(lo, hi)
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
            let mut p = json!({ "shape": d.str("shape"), "size": clamped(d, "size", (0.1, 1296.0)) });
            for (key, _, lo, hi) in BRISTLE {
                p[key] = json!(clamped(d, key, (lo, hi)));
            }
            p
        }
        _ => serde_json::to_value(calligraphic(d)).unwrap_or_default(),
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    grid(ui, |ui| {
        label(ui, tl!("Name:"));
        form::text(ui, d, "name", 240.0);
        ui.end_row();
    });
    ui.add_space(10.0);
    match d.str("__type").as_str() {
        "bristle" => bristle_body(app, ui, d),
        _ => calligraphic_body(ui, d),
    }
    false
}

fn calligraphic_body(ui: &mut egui::Ui, d: &mut Dialog) {
    let c = calligraphic(d);
    ui.horizontal(|ui| {
        nib_preview(ui, &c, vec2(NIB_W, 84.0));
        ui.add_space(8.0);
        stroke_preview(ui, "calligraphic", serde_json::to_value(&c).unwrap_or_default(), vec2(STROKE_W - NIB_W - 8.0, 84.0));
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
    for (key, text, lo, hi) in BRISTLE {
        widgets::label_row(ui, tl!(text), LABEL_W, |ui| form::slider_field(ui, d, key, lo..=hi, "%", &|_| rail));
    }
    ui.add_space(8.0);
    let p = params(d);
    stroke_preview(ui, "bristle", p, vec2(STROKE_W, 64.0));
}

/// A stroke painted with the brush the fields describe (type `ty`, definition fields `def`).
fn stroke_preview(ui: &mut egui::Ui, ty: &str, mut def: Value, size: egui::Vec2) {
    def["name"] = json!("preview");
    def["type"] = json!(ty);
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    crate::panels::brushes::chip(ui, r, &def);
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

/// OK: edit the brush (and rename it). A name another brush has keeps the dialog open.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let name = d.str("name").trim().to_string();
    let r = app.run("brush.options", json!({ "name": d.str("__brush"), "newName": name, "params": params(d) }));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app
    }

    fn set(app: &mut VectorcraftApp, fields: Value) {
        let d = app.ui.dialog.as_mut().unwrap();
        for (k, v) in fields.as_object().unwrap() {
            d.fields.insert(k.clone(), v.clone());
        }
    }

    fn shown(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    #[test]
    fn calligraphic_options_show_each_value_with_its_variation_and_ok_applies_them() {
        let mut app = app();
        app.run("ui.brushOptions", json!({"name": "6 pt. Flat"})).unwrap();
        let text = shown(&mut app);
        for label in ["Calligraphic Brush Options", "Name:", "Angle:", "Roundness:", "Size:", "Variation:", "Fixed", "OK", "Cancel"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.f64("angle", 0.0), d.f64("roundness", 0.0), d.f64("size", 0.0)), (45.0, 15.0, 6.0));
        // Out-of-range values are kept within their ranges.
        set(&mut app, json!({"angleMode": "random", "angleVariation": 500, "roundness": 140, "sizeMode": "pressure", "sizeVariation": 4}));
        assert!(shown(&mut app).contains("Pressure"));
        super::super::confirm(&mut app).unwrap();
        let def = app.run("brush.get", json!({"name": "6 pt. Flat"})).unwrap();
        assert_eq!(def["modes"], json!(["random", "fixed", "pressure"]));
        assert_eq!((def["variation"].clone(), def["roundness"].clone()), (json!([180.0, 0.0, 4.0]), json!(100.0)));
        // Opened again, it shows what was set.
        app.run("ui.brushOptions", json!({"name": "6 pt. Flat"})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("angleMode"), d.f64("sizeVariation", 0.0)), ("random".into(), 4.0));
    }

    #[test]
    fn a_taken_name_keeps_the_dialog_open() {
        let mut app = app();
        app.run("ui.brushOptions", json!({"name": "6 pt. Flat"})).unwrap();
        set(&mut app, json!({"name": "3 pt. Round"}));
        assert!(super::super::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
    }

    #[test]
    fn bristle_options_and_brushes_without_a_dialog() {
        let mut app = app();
        app.run("ui.brushOptions", json!({"name": "Bristle Round"})).unwrap();
        let text = shown(&mut app);
        for label in ["Bristle Brush Options", "Shape:", "Bristle Length:", "Stiffness:", "Paint Opacity:"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        set(&mut app, json!({"shape": "flatFan", "density": 0, "length": 120}));
        super::super::confirm(&mut app).unwrap();
        let def = app.run("brush.get", json!({"name": "Bristle Round"})).unwrap();
        assert_eq!((def["shape"].clone(), def["density"].clone(), def["length"].clone()), (json!("flatFan"), json!(1.0), json!(120.0)));
        // Art, scatter and pattern brushes have no dialog yet: an error, nothing opens.
        assert!(app.run("ui.brushOptions", json!({"name": "Arrow"})).is_err());
        assert!(app.ui.dialog.is_none());
        assert!(app.run("ui.brushOptions", json!({"name": "Nope"})).is_err());
    }
}
