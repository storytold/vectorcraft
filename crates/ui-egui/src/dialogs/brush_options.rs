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

/// OK of a new brush's Brush Options: `brush.new` with the type, the name, the values set and the
/// dropped art's ids. The dialog closes once the brush is made.
fn create(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = json!({ "type": d.str("__type"), "name": d.str("name").trim(), "params": params(d) });
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
        "calligraphic" => calligraphic_body(ui, d),
        _ => brush_art_options::body(app, ui, d),
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
    for (key, text, (lo, hi)) in BRISTLE {
        widgets::label_row(ui, tl!(text), LABEL_W, |ui| form::slider_field_named(ui, d, key, tl!(text), lo..=hi, "%", &|_| rail));
    }
    ui.add_space(8.0);
    let p = params(d);
    stroke_preview(ui, "bristle", p, vec2(STROKE_W, 64.0));
}

/// A stroke painted with the brush the fields describe (type `ty`, definition fields `def`).
pub(super) fn stroke_preview(ui: &mut egui::Ui, ty: &str, def: Value, size: egui::Vec2) {
    let mut def = match def {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    def.insert("name".into(), json!("preview"));
    def.insert("type".into(), json!(ty));
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    crate::panels::brushes::chip(ui, r, &Value::Object(def));
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
    fn bristle_options_and_an_unknown_brush() {
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
        // No such brush: an error, nothing opens.
        app.ui.dialog = None;
        assert!(app.run("ui.brushOptions", json!({"name": "Nope"})).is_err());
        assert!(app.ui.dialog.is_none());
    }

    /// Bristle Brush Options' Name field takes at most 31 characters as they are typed; the
    /// Calligraphic Brush Options' Name field takes more.
    #[test]
    fn bristle_brush_options_name_field_takes_at_most_31_characters() {
        let mut app = app();
        let typed = "é".repeat(40);
        for (brush, kept) in [("Bristle Round", 31), ("6 pt. Flat", 40)] {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
                let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
                out.textures_delta.clear();
            };
            app.run("ui.brushOptions", json!({ "name": brush })).unwrap();
            // The Name field takes the focus with its text selected; typing replaces it.
            frame(&mut app, vec![]);
            frame(&mut app, vec![]);
            frame(&mut app, vec![egui::Event::Text(typed.clone())]);
            assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "é".repeat(kept), "{brush}");
            super::super::confirm(&mut app).unwrap();
            assert!(app.run("brush.get", json!({ "name": "é".repeat(kept) })).is_ok(), "{brush}");
        }
        // A new name of more than 31 characters in the field (a longer saved name with some of its
        // characters deleted) is refused, and the dialog stays open.
        app.run("ui.brushOptions", json!({ "name": "é".repeat(31) })).unwrap();
        set(&mut app, json!({ "name": "é".repeat(35) }));
        assert!(super::super::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
    }

    /// A path painted with brush `name`, and the height of the art its stroke draws.
    fn brushed(app: &mut VectorcraftApp, name: &str) -> (vectorcraft_doc::NodeId, f64) {
        let id = app.run("shape.line", json!({"x1": 20, "y1": 100, "x2": 280, "y2": 100})).unwrap()["id"].as_u64().unwrap();
        let id = vectorcraft_doc::NodeId(id);
        app.run("brush.apply", json!({ "name": name, "ids": [id.0] })).unwrap();
        (id, drawn_height(app, id))
    }

    fn drawn_height(app: &VectorcraftApp, id: vectorcraft_doc::NodeId) -> f64 {
        let st = app.session.active().unwrap();
        let pieces = vectorcraft_brush::node_pieces(&st.doc, st.doc.node(id).unwrap()).unwrap();
        vectorcraft_brush::pieces_bounds(&pieces).unwrap().height()
    }

    #[test]
    fn scatter_options_show_each_value_with_its_mode_and_ok_applies_them() {
        let mut app = app();
        let (id, before) = brushed(&mut app, "Confetti");
        app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
        let text = shown(&mut app);
        for label in [
            "Scatter Brush Options",
            "Name:",
            "Size:",
            "Spacing:",
            "Scatter:",
            "Rotation:",
            "Random",
            "Rotation relative to:",
            "Page",
            "Colorization",
            "Method:",
            "Tints",
            "Key Color:",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.f64("sizeMin", 0.0), d.f64("sizeMax", 0.0), d.str("sizeMode")), (60.0, 140.0, "random".into()));
        assert_eq!((d.str("rotationRelativeTo"), d.str("colorization")), ("page".into(), "tints".into()));
        // Values are kept within their ranges (rotation here); Fixed takes the first value.
        set(
            &mut app,
            json!({"sizeMode": "fixed", "sizeMin": 5000, "spacingMode": "pressure", "rotationMin": -400, "rotationRelativeTo": "path", "colorization": "hueShift"}),
        );
        assert!(shown(&mut app).contains("Hue Shift"));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let def = app.run("brush.get", json!({"name": "Confetti"})).unwrap();
        assert_eq!(
            (&def["size"], &def["rotation"][0], &def["modes"]),
            (&json!([5000.0, 140.0]), &json!(-180.0), &json!(["fixed", "pressure", "random", "random"]))
        );
        assert_eq!((&def["rotation_relative_to_path"], &def["colorization"]["method"]), (&json!(true), &json!("hueShift")));
        assert!(def["colorization"]["key"].is_object(), "the art's colour as the key: {def}");
        assert!(drawn_height(&app, id) > before, "the stroke redraws with the bigger confetti");
        // Opened again, it shows what was set.
        app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("sizeMode"), d.str("spacingMode"), d.str("colorization")), ("fixed".into(), "pressure".into(), "hueShift".into()));
    }

    #[test]
    fn brush_key_color_eyedropper_updates_the_open_dialog() {
        use vectorcraft_tools::{PointerEvent, PointerKind};
        let mut app = app();
        app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
        app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap();
        app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
        set(&mut app, json!({"colorization": "hueShift"}));
        let back = app.session.tool_id();
        app.select_tool("eyedropper");
        app.session.set_tool_option("brushKeyColor", &json!(true));
        app.session.set_tool_option("stop", &json!(back));
        let view = app.view_info();
        crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Down, 80.0, 70.0), view);
        let key = app.ui.dialog.as_ref().unwrap().fields.get("keyColor").cloned().unwrap();
        assert_eq!(vectorcraft_engine::cmd::color_value(&key).unwrap().to_hex(), "#336699");
        assert_eq!(app.session.tool_id(), back);
    }

    #[test]
    fn art_options_set_width_scale_direction_flips_and_colorization() {
        let mut app = app();
        let (id, before) = brushed(&mut app, "Arrow");
        app.run("ui.brushOptions", json!({"name": "Arrow"})).unwrap();
        let text = shown(&mut app);
        for label in [
            "Art Brush Options",
            "Width:",
            "Brush Scale Options",
            "Scale Proportionately",
            "Stretch to Fit Stroke Length",
            "Stretch Between Guides",
            "Start:",
            "End:",
            "Direction:",
            "Flip Along",
            "Flip Across",
            "Colorization",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(
            (d.f64("width", 0.0), d.str("scaleMode"), d.f64("guideEnd", 0.0), d.str("direction")),
            (100.0, "betweenGuides".into(), 78.0, "leftToRight".into())
        );
        set(
            &mut app,
            json!({"width": 250, "guideStart": 10, "guideEnd": 140, "direction": "bottomToTop", "flipAcross": true, "colorization": "none"}),
        );
        super::super::confirm(&mut app).unwrap();
        let def = app.run("brush.get", json!({"name": "Arrow"})).unwrap();
        assert_eq!(
            (&def["width"], &def["scale"], &def["direction"]),
            (&json!(250.0), &json!({"mode": "betweenGuides", "start": 0.1, "end": 1.0}), &json!("bottomToTop"))
        );
        assert_eq!((&def["flip_across"], &def["flip_along"], &def["colorization"]), (&json!(true), &json!(false), &json!({"method": "none"})));
        assert!(drawn_height(&app, id) != before, "the stroke redraws");
        // Stretch to Fit drops the guides.
        app.run("ui.brushOptions", json!({"name": "Arrow"})).unwrap();
        set(&mut app, json!({"scaleMode": "stretch"}));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.run("brush.get", json!({"name": "Arrow"})).unwrap()["scale"], json!({"mode": "stretch"}));
    }

    /// A context with the app's fonts and theme, for [`frame`].
    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        ctx
    }

    /// Runs one 1200 × 900 frame with `events` through the app's key handling and the dialogs, and
    /// returns the center of the text `label` if the frame shows it.
    fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>, label: &str) -> Option<egui::Pos2> {
        use egui::{Pos2, Rect};
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            super::super::show(app, ui.ctx());
        });
        out.textures_delta.clear();
        out.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label => Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center()),
            _ => None,
        })
    }

    /// Clicks the text `label` the open dialog shows.
    fn click_text(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str) {
        use egui::{Event, PointerButton};
        frame(ctx, app, vec![], label);
        let at = frame(ctx, app, vec![], label).unwrap_or_else(|| panic!("no {label:?} shown"));
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(ctx, app, vec![Event::PointerMoved(at), button(true)], label);
        frame(ctx, app, vec![button(false)], label);
        frame(ctx, app, vec![], label);
    }

    /// Presses Escape.
    fn escape(ctx: &egui::Context, app: &mut VectorcraftApp) {
        let key = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(ctx, app, vec![key], "");
        frame(ctx, app, vec![], "");
    }

    /// Scatter, Art and Pattern Brush Options have a Tips button beside Method. A click on it opens
    /// Colorization Tips, which names each method and what it does beside a sample of the brush,
    /// and a second click closes it. Screen readers name the button, the popover and each sample.
    #[test]
    fn tips_names_each_colorization_method_beside_a_sample() {
        use crate::widgets::accessibility_tests::{role_and_state, tree};
        use egui::accesskit::Role;
        let mut app = app();
        for name in ["Confetti", "Arrow", "Chain"] {
            app.run("ui.brushOptions", json!({ "name": name })).unwrap();
            let text = shown(&mut app);
            assert!(text.lines().any(|l| l == "Tips") && !text.contains("Colorization Tips"), "{name}: {text}");
        }
        app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
        let ctx = ctx();
        click_text(&ctx, &mut app, "Tips");
        assert!(app.ui.dialog.as_ref().unwrap().bool("tips"), "Tips opens Colorization Tips");
        let text = shown(&mut app);
        for label in [
            "Colorization Tips",
            "The samples show this brush on a blue stroke.",
            "None",
            "The art keeps its own colors.",
            "Tints",
            "Black becomes the stroke color and white stays white. Other colors become tints of the stroke color by their lightness.",
            "Tints and Shades",
            "Hue Shift",
            "The key color becomes the stroke color. The other colors shift their hue by the same amount and keep their saturation and brightness.",
            "Black and white stay as they are. Dark colors become shades of the stroke color and light colors become tints of it.",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let nodes = tree(|ui| super::super::show(&mut app, ui.ctx()));
        assert_eq!(role_and_state(&nodes, "Tips").0, Role::Button);
        assert_eq!(role_and_state(&nodes, "Colorization Tips").0, Role::Dialog);
        for method in ["None", "Tints", "Tints and Shades", "Hue Shift"] {
            assert!(nodes.iter().any(|n| n.role() == Role::Image && n.label() == Some(method)), "a sample named {method}");
        }
        click_text(&ctx, &mut app, "Tips");
        assert!(!app.ui.dialog.as_ref().unwrap().bool("tips"), "a second click closes it");
        assert!(!shown(&mut app).contains("Colorization Tips"));
        // `ui.dialog.set {field: "tips", value: true}` opens it too.
        set(&mut app, json!({"tips": true}));
        assert!(shown(&mut app).contains("Colorization Tips"));
    }

    /// Escape with Colorization Tips or the Method list open closes only that: Brush Options stays
    /// open with its edits, and the next Escape cancels it.
    #[test]
    fn escape_closes_tips_or_the_method_list_and_leaves_the_dialog_open() {
        let (mut app, ctx) = (app(), ctx());
        app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
        set(&mut app, json!({"colorization": "tints"}));
        click_text(&ctx, &mut app, "Tips");
        assert!(app.ui.dialog.as_ref().unwrap().bool("tips"));
        escape(&ctx, &mut app);
        let d = app.ui.dialog.as_ref().expect("Escape closes Tips, not Brush Options");
        assert!(!d.bool("tips") && d.str("colorization") == "tints");
        assert!(frame(&ctx, &mut app, vec![], "Colorization Tips").is_none());
        click_text(&ctx, &mut app, "Tints");
        assert!(egui::Popup::is_any_open(&ctx), "the Method list is open");
        escape(&ctx, &mut app);
        assert!(app.ui.dialog.as_ref().is_some_and(|d| d.str("colorization") == "tints"), "Escape closes the list, not Brush Options");
        assert!(!egui::Popup::is_any_open(&ctx));
        escape(&ctx, &mut app);
        assert!(app.ui.dialog.is_none(), "the next Escape cancels Brush Options");
        // Calligraphic Brush Options has no Tips: with `tips` set, Escape still cancels it.
        app.run("ui.brushOptions", json!({"name": "6 pt. Flat"})).unwrap();
        set(&mut app, json!({"tips": true}));
        escape(&ctx, &mut app);
        assert!(app.ui.dialog.is_none(), "Escape cancels Calligraphic Brush Options");
    }

    /// Each Colorization Tips sample paints the brush with its own method on the blue stroke:
    /// Confetti's black and gray art keeps its colors with None, its black becomes the stroke color
    /// with Tints and with Hue Shift (black is its key color), and stays black with Tints and Shades.
    #[test]
    fn each_tips_sample_paints_the_brush_with_its_method() {
        let mut app = app();
        app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        let key: vectorcraft_color::Color = serde_json::from_value(d.fields["keyColor"].clone()).unwrap();
        let colors = |id: &str| -> Vec<String> {
            let method = vectorcraft_brush::Colorization::parse(id, key).unwrap();
            let doc = brush_art_options::tip_doc(&brush_art_options::edited_def(d), "scatter", method, 120.0, 44.0).unwrap();
            let mut hex: Vec<String> = vectorcraft_brush::art_colors(doc.layers.iter().map(|l| l.as_ref())).iter().map(|c| c.to_hex()).collect();
            hex.sort();
            hex
        };
        let (black, blue) = ("#000000".to_string(), brush_art_options::TIP_STROKE.to_hex());
        let confetti = vectorcraft_brush::defaults().iter().find(|b| b.name == "Confetti").unwrap();
        let art: Vec<String> = vectorcraft_brush::art_colors(confetti.art()).iter().map(|c| c.to_hex()).collect();
        let none = colors("none");
        assert!(none.len() == 2 && art.iter().all(|c| none.contains(c)), "None keeps the art's colors {art:?}: {none:?}");
        let (tints, shades, hue) = (colors("tints"), colors("tintsAndShades"), colors("hueShift"));
        assert!(tints.contains(&blue) && !tints.contains(&black), "{tints:?}");
        assert!(shades.contains(&black) && !shades.contains(&blue), "{shades:?}");
        assert!(hue.contains(&blue) && !hue.contains(&black), "{hue:?}");
        assert!(tints != hue && tints != shades && none != shades, "four different samples");
    }

    /// `ui.dialog.set` writes any field, `__def` included. With a brush definition that is not
    /// an object, Brush Options and Colorization Tips still draw.
    #[test]
    fn a_brush_definition_that_is_not_an_object_still_draws() {
        let (mut app, ctx) = (app(), ctx());
        app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
        crate::tests_synthetic::control(&mut app, &ctx, "ui.dialog.set", json!({"field": "__def", "value": 5}));
        crate::tests_synthetic::control(&mut app, &ctx, "ui.dialog.set", json!({"field": "tips", "value": true}));
        let text = shown(&mut app);
        assert!(text.contains("Scatter Brush Options") && text.contains("Colorization Tips"), "{text}");
    }

    #[test]
    fn pattern_options_show_the_tiles_and_set_scale_spacing_fit() {
        let mut app = app();
        app.run("ui.brushOptions", json!({"name": "Chain"})).unwrap();
        let text = shown(&mut app);
        for label in [
            "Pattern Brush Options",
            "Scale:",
            "Spacing:",
            "Outer Corner Tile",
            "Side Tile",
            "Inner Corner Tile",
            "Start Tile",
            "End Tile",
            "None",
            "Fit",
            "Stretch to fit",
            "Add space to fit",
            "Approximate path",
            "Flip Along",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_ref().unwrap();
        let mut tiles: Vec<&String> = d.fields["__tiles"].as_object().unwrap().keys().collect();
        tiles.sort();
        assert_eq!(tiles, ["innerCorner", "outerCorner", "side"], "Chain has side and corner tiles, no start or end");
        // Diamonds (straight-edged tiles, so sizes compare exactly).
        let (id, before) = brushed(&mut app, "Diamonds");
        app.run("ui.brushOptions", json!({"name": "Diamonds"})).unwrap();
        assert_eq!((app.ui.dialog.as_ref().unwrap().f64("spacing", 0.0), app.ui.dialog.as_ref().unwrap().str("fit")), (25.0, "addSpace".into()));
        set(&mut app, json!({"scale": 200, "spacing": 30, "fit": "approximate", "flipAlong": true}));
        super::super::confirm(&mut app).unwrap();
        let def = app.run("brush.get", json!({"name": "Diamonds"})).unwrap();
        assert_eq!(
            (&def["scale"], &def["spacing"], &def["fit"], &def["flip_along"]),
            (&json!(200.0), &json!(30.0), &json!("approximate"), &json!(true))
        );
        assert!((drawn_height(&app, id) / before - 2.0).abs() < 1e-6, "tiles twice the size");
    }

    /// Art Brush Options' Width and Pattern Brush Options' Scale each have a slider beside their
    /// field, which screen readers name as the row, with the field's number and text over the
    /// value's whole range.
    #[test]
    fn art_width_and_pattern_scale_have_sliders() {
        use crate::widgets::accessibility_tests::{slider_value, tree};
        let mut app = app();
        app.run("ui.brushOptions", json!({"name": "Arrow"})).unwrap();
        set(&mut app, json!({"width": 250}));
        let shown = tree(|ui| super::super::show(&mut app, ui.ctx()));
        assert_eq!(slider_value(&shown, "Width:"), (Some(250.0), Some(1.0), Some(1000.0), Some("250%".into())));
        app.ui.dialog = None;
        app.run("ui.brushOptions", json!({"name": "Diamonds"})).unwrap();
        let shown = tree(|ui| super::super::show(&mut app, ui.ctx()));
        assert_eq!(slider_value(&shown, "Scale:"), (Some(100.0), Some(1.0), Some(10000.0), Some("100%".into())));
    }
}
