//! Brush Options (double-click a brush in the Brushes panel, or the panel menu's Brush Options…).
//! Calligraphic Brush Options: Name, then Angle, Roundness and Size, each Fixed, Random, Pressure,
//! Tilt, Bearing or Rotation with its Variation, over live previews of the nib and of a stroke
//! (light, heavy, light pressure). Bristle Brush Options: Name (a new bristle brush name that is too
//! long is refused at OK), Shape and the bristle sliders. Scatter, Art and Pattern Brush Options: see
//! [`super::brush_art_options`]. OK runs `brush.options` (strokes painted with the brush update);
//! a name another brush has, or a new bristle brush name that is too long, keeps the dialog open.
//!
//! Fields: `__brush` (the brush's name), `__type` (`calligraphic` | `bristle` | `scatter` | `art` |
//! `pattern`), `name`.
//! Calligraphic: `angle` (−180..180°), `roundness` (0..100 %), `size` (0..1296 pt), and per value
//! `angleMode`, `roundnessMode`, `sizeMode` (`fixed` | `random` | `pressure` | `tilt` | `bearing` |
//! `rotation`) and
//! `angleVariation`, `roundnessVariation`, `sizeVariation`. Bristle: `shape` (`roundPoint`…),
//! `size` (pt), `length` (25..300 %), `density`, `thickness`, `opacity`, `stiffness` (1..100 %).
//!
//! New Brush opens the dialog for a brush that does not exist yet ([`open_new`]), with `__create`
//! and, for dropped art, `__ids`: OK runs `brush.new` instead, which gives a name another brush has
//! a number, and Cancel adds nothing.
//!
//! The panel menu's Options of Selected Object opens the dialog on the brush of the first selected
//! brushed path, with that path's own settings ([`open_object`]), with `__object`, `__targets` (the
//! brushed paths), `__opened` (the values it opened with) and `__doc` (the uid of the document). It
//! has no name, and of the Art and Pattern brush options only those a path can set for itself. OK
//! runs `brush.objectOptions` with the values changed since it opened: the paths change and the
//! brush stays as it is. OK after another document has become the active one changes nothing.

use std::sync::Arc;

use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Picture, PictureSize};
use craft::ui::form::{Field, SliderField, SliderFieldLayout};
use craft::ui::layout_components::Row;
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::pickers::Select;
use craft::ui::primitives::{ContentPainterCx, StrokeWidth};
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::text::NumberField;
use egui::{Color32, Rect, Stroke, pos2, vec2};
use serde_json::{Map, Value, json};
use vectorcraft_brush::{Bristle, Brush, BrushKind, Calligraphic, Variation};
use vectorcraft_doc::{AppearanceItem, Document, NodeId, NodeKind};

use super::{brush_art_options, form, host};
use crate::VectorcraftApp;

// PLAN: the `craft::ui::…` import paths above follow the framework crates' module layout; the app tree
// has no other user of these components yet. `brush_art_options::{params, edited_def, fields,
// heading, rows}` are called with `&DialogFields` where they took `&Dialog`.

/// The dialog key of Brush Options.
pub const KIND: &str = "brushOptions";

/// The field of a Brush Options dialog that New Brush opened.
const CREATE: &str = "__create";

/// The field of such a dialog holding the ids of the art dropped on the Brushes panel.
const IDS: &str = "__ids";

/// The field of a Brush Options dialog that Options of Selected Object opened.
pub(super) const OBJECT: &str = "__object";

/// The field of such a dialog holding the brushed paths it sets.
const TARGETS: &str = "__targets";

/// The field of such a dialog holding the `brush.objectOptions` params it opened with.
const OPENED: &str = "__opened";

/// The field of such a dialog holding the uid of the document it opened in.
const DOC: &str = "__doc";

/// The field of such a dialog holding the option keys a path can set for itself.
const KEYS: &str = "__keys";

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

/// The string in field `key`, empty when it has none.
fn string(d: &DialogFields, key: &str) -> String {
    d.text(key).unwrap_or_default().to_string()
}

/// The flag in field `key`, false when it has none.
fn flag(d: &DialogFields, key: &str) -> bool {
    d.bool(key).unwrap_or(false)
}

/// The JSON value a private marker or an edited definition is kept as in field `key`.
fn json<'a>(d: &'a DialogFields, key: &str) -> Option<&'a Value> {
    match d.get(key)? {
        FieldValue::Json(value) => Some(value),
        _ => None,
    }
}

/// The uid of the active document, in the form the `__doc` field keeps it.
fn active_uid(app: &VectorcraftApp) -> Option<i64> {
    app.session.active().and_then(|doc| i64::try_from(doc.uid).ok())
}

/// The title of Brush Options for brush type `ty`.
fn title(ty: &str, object: bool) -> String {
    if object {
        return tl!("Options of Selected Object").to_string();
    }
    match ty {
        "bristle" => tl!("Bristle Brush Options").to_string(),
        "calligraphic" => tl!("Calligraphic Brush Options").to_string(),
        ty => brush_art_options::heading(ty).to_string(),
    }
}

/// The key of the field holding the variation mode of value `key`.
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
    if let Some(st) = app.session.active() {
        f.insert("__patterns".into(), serde_json::to_value(&st.doc.patterns).unwrap_or_default());
    }
    open_dialog(app, &f, None)
}

/// Opens the hosted dialog over the fields `f` (`__type` says which brush type's rows it has).
/// `object_keys` are the option keys of Options of Selected Object, which the dialog then records
/// with the values it opened with. OK reaches [`ok`] once the dialog's own rows accept it.
fn open_dialog(app: &mut VectorcraftApp, f: &Map<String, Value>, object_keys: Option<&[&str]>) -> Result<Value, String> {
    let ty = f.get("__type").and_then(Value::as_str).unwrap_or_default();
    let object = object_keys.is_some();
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    // Modal: no edit changes the document while the dialog is open, so the previews read this snapshot.
    let doc = app.session.active().map(|st| st.doc.clone());
    let rows = rows(ty, object, doc, crate::canvas::rulers::unit_kind(app.session.general_unit()));
    let title = Label::from(title(ty, object));
    let mut initial = form::fields_of(&rows, f);
    // The hidden values the rows read whole stay JSON: an untagged read makes an array a list of values.
    for key in ["__def", "__patterns", "__colors"] {
        if let Some(value) = f.get(key) {
            initial.set(key, FieldValue::Json(value.clone()));
        }
    }
    if let Some(keys) = object_keys {
        let opened = object_params(&initial, keys);
        initial.set(OPENED, FieldValue::Json(Value::Object(opened)));
        initial.set(KEYS, FieldValue::Json(json!(keys)));
    }
    // The canvas of the current window: the Key Color dropper samples there while the dialog is open.
    let canvas = crate::doc_windows::current(app).map(crate::canvas::canvas_id);
    let dialog = HostedParamDialog::new(&ctx, KIND, initial, move |fields| {
        let dialog = ParamDialog::new(KIND, title.clone(), rows.clone()).values(Bind::bound(fields));
        match canvas {
            Some(canvas) => dialog.canvas(canvas),
            None => dialog,
        }
    })
    .held_ok();
    app.ui.brush_options = Some(host::open_held(app, dialog, ok).ok_or("no window")?);
    Ok(Value::Null)
}

/// Once a frame: runs the Key Color pick of the open Brush Options, and forgets the dialog once it
/// is gone, ending a pick it left.
pub(super) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(id) = app.ui.brush_options else { return };
    let Some(dialogs) = app.dialogs() else { return };
    let open =
        dialogs.with(|host| host.get_mut_by_id::<HostedParamDialog>(id).map(|dialog| brush_art_options::key_color_pick(app, dialog)).is_some());
    if !open {
        app.ui.brush_options = None;
        if app.ui.brush_key_color_pick.as_ref().is_some_and(|pick| pick.dialog == id) {
            brush_art_options::end_key_color_pick(app, true);
        }
    }
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

/// The brush that the first brushed stroke of path `id` paints with, in library `lib`.
fn stroke_brush<'a>(app: &VectorcraftApp, lib: &'a [Brush], id: NodeId) -> Option<&'a Brush> {
    let n = app.session.active()?.doc.node(id)?;
    n.appearance.items.iter().find_map(|i| match i {
        AppearanceItem::Stroke(st) => st.brush.as_deref().and_then(|name| lib.iter().find(|b| b.name == name)),
        AppearanceItem::Fill(_) => None,
    })
}

/// Can Options of Selected Object open: does a selected path or compound path (or one in a
/// selected group, but not a path inside a compound path, as `brush.objectOptions` takes them)
/// paint with a brush, are all their brushes of one type, and can a path set options of its own
/// for that type? These are the selections `brush.objectOptions` takes.
pub fn object_available(app: &VectorcraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    let ids = st.selection.subjects();
    if ids.is_empty() {
        return false;
    }
    let lib = vectorcraft_brush::library(&st.doc);
    let doc = &st.doc;
    // The brush of every brushed stroke of the paths the command sets.
    let mut brushes: Vec<&Brush> = vec![];
    for id in ids {
        let Some(n) = doc.node(*id) else { continue };
        n.walk(&mut |c| {
            let own = matches!(c.kind, NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. });
            let in_compound = doc.parent_of(c.id).and_then(|p| doc.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. }));
            if !own || in_compound {
                return;
            }
            for i in &c.appearance.items {
                if let AppearanceItem::Stroke(s) = i
                    && let Some(b) = s.brush.as_deref().and_then(|name| lib.iter().find(|b| b.name == name))
                {
                    brushes.push(b);
                }
            }
        });
    }
    let Some(first) = brushes.first() else { return false };
    !first.kind.stroke_option_keys().is_empty() && brushes.iter().all(|b| b.kind.type_id() == first.kind.type_id())
}

/// Open Options of Selected Object: the Brush Options of the brush the first selected brushed path
/// paints with, at that path's own settings, without the name and with only the options a path can
/// set for itself.
pub fn open_object(app: &mut VectorcraftApp) -> Result<Value, String> {
    let r = app.run("brush.objectOptions", json!({}))?;
    let targets = r["ids"].clone();
    let first = targets.get(0).and_then(Value::as_u64).map(NodeId).ok_or("no path painted with a brush is selected")?;
    let st = app.session.active().ok_or("no document open")?;
    let (lib, doc) = (vectorcraft_brush::library(&st.doc), st.uid);
    let b = stroke_brush(app, &lib, first).ok_or("no path painted with a brush is selected")?;
    let own: Map<String, Value> = serde_json::from_value(r["options"].clone()).unwrap_or_default();
    let edited = b.with_stroke_options(&own)?;
    let def = serde_json::to_value(&edited).map_err(|e| e.to_string())?;
    let mut f = fields(&edited, def)?;
    f.insert("__brush".into(), json!(b.name));
    f.insert(OBJECT.into(), json!(true));
    f.insert(TARGETS.into(), targets);
    f.insert(DOC.into(), json!(doc));
    open_dialog(app, &f, Some(b.kind.stroke_option_keys()))
}

/// The fields' `brush.options` params that a path can set for itself (`keys`).
fn object_params(d: &DialogFields, keys: &[&str]) -> Map<String, Value> {
    match params(d) {
        Value::Object(p) => p.into_iter().filter(|(k, _)| keys.contains(&k.as_str())).collect(),
        _ => Map::new(),
    }
}

/// What of option value `now` changed from `was`, the value it opened with: `None` when nothing
/// did; of a range or a list of modes, the items that changed with `null` for the others, and of a
/// colorization, the fields that changed. The paths keep their own values for the rest.
fn changed_part(was: Option<&Value>, now: Value) -> Option<Value> {
    match (was, now) {
        (Some(w), n) if *w == n => None,
        (Some(Value::Array(w)), Value::Array(n)) if w.len() == n.len() => {
            Some(Value::Array(w.iter().zip(n).map(|(w, n)| if *w == n { Value::Null } else { n }).collect()))
        }
        (Some(Value::Object(w)), Value::Object(n)) => Some(Value::Object(n.into_iter().filter(|(f, v)| w.get(f) != Some(v)).collect())),
        (_, n) => Some(n),
    }
}

/// OK of Options of Selected Object: `brush.objectOptions` on the paths it opened for, with the
/// values changed since it opened ([`changed_part`]). When another document has become the active
/// one, it closes as Cancel does and changes nothing.
fn confirm_object(app: &mut VectorcraftApp, d: &DialogFields) -> Result<(), String> {
    if d.int(DOC) != active_uid(app) {
        return Ok(());
    }
    let keys: Vec<String> = json(d, KEYS).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let opened = json(d, OPENED).and_then(Value::as_object).cloned().unwrap_or_default();
    let changed: Map<String, Value> =
        object_params(d, &keys).into_iter().filter_map(|(k, v)| changed_part(opened.get(&k), v).map(|v| (k, v))).collect();
    let targets = json(d, TARGETS).cloned().unwrap_or_default();
    app.run("brush.objectOptions", json!({ "ids": targets, "params": changed })).map(|_| ())
}

/// Open the Brush Options of a new brush of type `ty` (`calligraphic` | `scatter` | `art` |
/// `bristle` | `pattern`), the second step of New Brush. The fields start from the type's default
/// values and the name "New <Type> Brush", numbered when another brush has it. A scatter or art
/// brush takes the art `brush.selectionArt` gives for `ids` (else for the selection) as its art,
/// and a pattern brush takes it as its side tile; the preview draws it. Without that art nothing
/// opens.
pub(super) fn open_new(app: &mut VectorcraftApp, ty: &str, ids: Option<Value>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let patterns = serde_json::to_value(&st.doc.patterns).unwrap_or_default();
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
    f.insert("__patterns".into(), patterns);
    f.insert(CREATE.into(), json!(true));
    if let Some(ids) = ids {
        f.insert(IDS.into(), ids);
    }
    open_dialog(app, &f, None)
}

/// OK of a new brush's Brush Options: `brush.new` with the type, the name, the values set, the art
/// the dialog opened with (`art` or `side` of `__def`, which the preview draws) and the dropped
/// art's ids. A selection changed or cleared while the dialog is open does not change the art.
fn create(app: &mut VectorcraftApp, d: &DialogFields) -> Result<(), String> {
    let mut brush_params = params(d);
    if string(d, "__type") == "pattern" {
        if string(d, "sideTile") == "none" {
            return Err("You must choose a side pattern.".into());
        }
        let preview = brush_art_options::edited_def(d);
        for key in [
            "side",
            "side_frame",
            "side_swatch",
            "outer_corner",
            "outer_corner_frame",
            "outer_corner_swatch",
            "outer_source",
            "inner_corner",
            "inner_corner_frame",
            "inner_corner_swatch",
            "inner_source",
            "start",
            "start_frame",
            "start_swatch",
            "end",
            "end_frame",
            "end_swatch",
        ] {
            if let Some(value) = preview.get(key) {
                brush_params[key] = value.clone();
            }
        }
    }
    if let Some(def) = json(d, "__def") {
        for key in ["art", "side"] {
            if let Some(art) = def.get(key).filter(|v| !v.is_null() && brush_params.get(key).is_none_or(Value::is_null)) {
                brush_params[key] = art.clone();
            }
        }
    }
    let mut p = json!({ "type": string(d, "__type"), "name": string(d, "name").trim(), "params": brush_params });
    if let Some(ids) = json(d, IDS) {
        p["ids"] = ids.clone();
    }
    app.run("brush.new", p).map(|_| ())
}

/// OK: make the new brush when New Brush opened the dialog, else edit the brush. A refusal keeps the
/// dialog open with the reason under its rows.
fn ok(app: &mut VectorcraftApp, answer: &ParamResult) -> Result<(), String> {
    let d = &answer.fields;
    if flag(d, CREATE) {
        create(app, d)
    } else if flag(d, OBJECT) {
        confirm_object(app, d)
    } else {
        confirm(app, d)
    }
}

/// The value of field `key` within `range`.
pub(super) fn clamped(fields: &DialogFields, key: &str, (lo, hi): (f64, f64)) -> f64 {
    fields.number(key).unwrap_or(lo).clamp(lo, hi)
}

/// The Calligraphic brush the fields describe.
fn calligraphic(d: &DialogFields) -> Calligraphic {
    let value = |i: usize| CALLI.get(i).zip(Calligraphic::RANGES.get(i)).map_or(0.0, |((key, ..), r)| clamped(d, key, *r));
    let variation =
        |i: usize| CALLI.get(i).zip(Calligraphic::MAX_VARIATION.get(i)).map_or(0.0, |((key, ..), max)| clamped(d, &variation_key(key), (0.0, *max)));
    let mode = |i: usize| CALLI.get(i).and_then(|(key, ..)| Variation::parse(&string(d, &mode_key(key)))).unwrap_or_default();
    Calligraphic { angle: value(0), roundness: value(1), size: value(2), variation: [0, 1, 2].map(variation), modes: Some([0, 1, 2].map(mode)) }
}

/// `brush.options` params from the fields.
fn params(d: &DialogFields) -> Value {
    match string(d, "__type").as_str() {
        "bristle" => {
            let mut p = json!({ "shape": string(d, "shape"), "size": clamped(d, "size", Bristle::SIZE_RANGE) });
            for (key, _, range) in BRISTLE {
                p[key] = json!(clamped(d, key, range));
            }
            p
        }
        "calligraphic" => serde_json::to_value(calligraphic(d)).unwrap_or_default(),
        _ => brush_art_options::params(d),
    }
}

/// The rows of Brush Options for brush type `ty`: the Name (not for Options of Selected Object, which
/// sets a path's settings, not the brush) and the type's own rows. `doc` gives the previews the
/// resources (patterns, symbols…) the art uses.
fn rows(ty: &str, object: bool, doc: Option<Arc<Document>>, unit: UnitKind) -> Vec<Param> {
    let mut rows = Vec::new();
    if !object {
        rows.push(Param::new("name", tl!("Name:"), ParamKind::text("")));
    }
    match ty {
        "bristle" => rows.extend(bristle_rows(doc, unit)),
        "calligraphic" => rows.extend(calligraphic_rows(doc)),
        _ => rows.extend(brush_art_options::rows(ty, object)),
    }
    rows
}

/// The Calligraphic rows: the nib and stroke previews, then Angle, Roundness and Size.
fn calligraphic_rows(doc: Option<Arc<Document>>) -> Vec<Param> {
    let values = CALLI.into_iter().enumerate().filter_map(|(i, (key, label, suffix, decimals))| {
        let (range, max) = (*Calligraphic::RANGES.get(i)?, *Calligraphic::MAX_VARIATION.get(i)?);
        Some(variation_row(key, tl!(label).into(), suffix, decimals, range, max))
    });
    std::iter::once(preview_row(doc, "calligraphic")).chain(values).collect()
}

/// The Bristle rows: Shape, Size in the document's unit, the five sliders, then the stroke preview.
fn bristle_rows(doc: Option<Arc<Document>>, unit: UnitKind) -> Vec<Param> {
    let shapes = SHAPES.into_iter().map(|(id, label)| (id.to_string(), Label::from(tl!(label)))).collect();
    let (lo, hi) = Bristle::SIZE_RANGE;
    let mut rows = vec![
        Param::new("shape", tl!("Shape:"), ParamKind::choice(shapes, Some("roundPoint".into()))),
        Param::new("size", tl!("Size:"), ParamKind::number(Some(lo), Some(hi), lo, 1.0, 1)).unit(unit),
    ];
    rows.extend(BRISTLE.into_iter().map(|(key, label, range)| bristle_slider(key, tl!(label), range)));
    rows.push(preview_row(doc, "bristle"));
    rows
}

/// A Bristle slider, kept within `range` while typed or dragged. A custom row: a value set by command
/// outside the range is kept and clamped on OK ([`params`]), not refused.
fn bristle_slider(key: &'static str, label: &'static str, range: (f64, f64)) -> Param {
    Param::custom(key, label, move |ui, d, _| {
        let shown =
            SliderField::new(label, Bind::given(clamped(d, key, range)), range.0..=range.1).layout(SliderFieldLayout::Inline).id_salt(key).show(ui);
        if let Some(v) = shown.edit.value() {
            d.set(key, *v);
        }
        shown.edit.response.clone()
    })
}

/// One Calligraphic value: its number, the mode of its variation, and (enabled while the mode is not
/// Fixed) the variation's amount. Automation sets the three fields through the one row.
fn variation_row(key: &'static str, label: Label, suffix: &'static str, decimals: usize, range: (f64, f64), max: f64) -> Param {
    let (mode_id, variation_id) = (mode_key(key), variation_key(key));
    let entries: Vec<Entry<String>> = Variation::offered(key == "size").iter().map(|m| Entry::new(m.id().to_string(), m.label())).collect();
    let companions = [mode_id.clone(), variation_id.clone()];
    let name = label.clone();
    Param::custom(key, label, move |ui, d, _| {
        let mode = string(d, &mode_id);
        let mut edits: Vec<(String, FieldValue)> = Vec::new();
        let shown = Row::new().show(ui, |ui, _| {
            let typed = NumberField::new(name.clone(), Bind::given(Some(clamped(d, key, range))))
                .range(range.0..=range.1)
                .unit(suffix)
                .fixed_decimals(decimals as _)
                .id_salt("value")
                .show(ui);
            if let Some(Some(v)) = typed.edit.value() {
                edits.push((key.to_string(), FieldValue::Number(v.clamp(range.0, range.1))));
            }
            if let Some(picked) = Select::new(name.clone(), Bind::given(mode.clone()), &entries).id_salt("mode").show(ui).value() {
                edits.push((mode_id.clone(), FieldValue::Text(picked.clone())));
            }
            Field::new(tl!("Variation:")).id_salt("variation").show(ui, |ui, _| {
                let typed = NumberField::new(tl!("Variation:"), Bind::given(Some(clamped(d, &variation_id, (0.0, max)))))
                    .range(0.0..=max)
                    .unit(suffix)
                    .fixed_decimals(decimals as _)
                    .enabled(mode != Variation::Fixed.id())
                    .id_salt("amount")
                    .show(ui);
                if let Some(Some(v)) = typed.edit.value() {
                    edits.push((variation_id.clone(), FieldValue::Number(v.clamp(0.0, max))));
                }
            });
        });
        for (field, value) in edits {
            d.set(field, value);
        }
        shown.output
    })
    .companions(companions)
    .full_width()
}

/// The previews: for a Calligraphic brush the nib, and for both types a stroke painted with the brush
/// the fields describe (light, heavy, light pressure), with the resources of `doc`.
fn preview_row(doc: Option<Arc<Document>>, ty: &'static str) -> Param {
    Param::custom("preview", "", move |ui, d, _| {
        let nib = (ty == "calligraphic").then(|| calligraphic(d));
        let def = match &nib {
            Some(c) => serde_json::to_value(c).unwrap_or_default(),
            None => params(d),
        };
        let (doc, extent) = (doc.as_deref(), PictureSize::Sample(ThumbSize::Large));
        Row::new()
            .show(ui, |ui, _| {
                if let Some(c) = &nib {
                    Picture::painter("", ThumbAspect::new(10, 7), |cx, rect| nib_preview(cx, rect, c)).extent(extent).show(ui);
                }
                let wide = if nib.is_some() { ThumbAspect::new(34, 10) } else { ThumbAspect::new(48, 10) };
                Picture::painter("", wide, |cx, rect| stroke_preview(cx, rect, doc, ty, def.clone())).extent(extent).show(ui);
            })
            .output
    })
    .full_width()
}

/// A stroke painted with the brush the fields describe (type `ty`, definition fields `def`), with
/// the resources (the patterns, symbols… its art uses) of `doc`.
pub(super) fn stroke_preview(cx: &ContentPainterCx, rect: Rect, doc: Option<&Document>, ty: &str, def: Value) {
    let mut def = match def {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    def.insert("name".into(), json!("preview"));
    def.insert("type".into(), json!(ty));
    crate::panels::brushes::paint_definition(cx, rect, doc, &Value::Object(def));
}

/// The nib preview's paper, white like the brush previews'.
const NIB_PAPER: Color32 = Color32::WHITE;
/// The nib at the brush's values, black on the paper.
const NIB_INK: Color32 = Color32::BLACK;
/// The nib preview's center lines, a light gray on its white paper.
const NIB_GUIDE: Color32 = Color32::from_gray(220);
/// The nib preview's lightest and heaviest nibs, in a gray behind the black nib at the brush's values.
const NIB_RANGE: Color32 = Color32::from_gray(200);
/// The room the nib preview keeps between its largest nib and each edge, in points.
const NIB_MARGIN: f32 = 6.0;

/// The nib of `c` at its values (black), and — for values that vary — at their lightest and
/// heaviest (grey), on white like the brush previews, its largest nib filling the box.
fn nib_preview(cx: &ContentPainterCx, r: Rect, c: &Calligraphic) {
    let guide = Stroke::new(cx.stroke_width(StrokeWidth::Thin), NIB_GUIDE);
    cx.data_shapes([
        egui::Shape::rect_filled(r, 0.0, NIB_PAPER),
        egui::Shape::line_segment([pos2(r.left(), r.center().y), pos2(r.right(), r.center().y)], guide),
        egui::Shape::line_segment([pos2(r.center().x, r.top()), pos2(r.center().x, r.bottom())], guide),
    ]);
    let values = [c.angle, c.roundness, c.size];
    let at = |sign: f64| -> [f64; 3] {
        let mut v = values;
        for (i, (x, mode)) in v.iter_mut().zip(c.modes()).enumerate() {
            let Some(&var) = c.variation.get(i) else { continue };
            if mode != Variation::Fixed {
                *x = Calligraphic::moved(i, *x, sign * var);
            }
        }
        v
    };
    let nibs = [at(-1.0), at(1.0), values];
    let largest = nibs.iter().map(|n| n[2]).fold(0.0, f64::max).max(1e-6);
    let scale = (r.height().min(r.width()) - 2.0 * NIB_MARGIN) / largest as f32;
    let mut shapes = Vec::with_capacity(nibs.len());
    for (k, [angle, round, size]) in nibs.into_iter().enumerate() {
        let fill = if k < 2 { NIB_RANGE } else { NIB_INK };
        let (a, b) = ((size as f32 * scale / 2.0).max(0.5), (size * round.clamp(1.0, 100.0) / 100.0) as f32 * scale / 2.0);
        let th = (angle as f32).to_radians();
        let (u, v) = (vec2(th.cos(), -th.sin()), vec2(th.sin(), th.cos()));
        let pts: Vec<egui::Pos2> = (0..48)
            .map(|i| {
                let s = i as f32 / 48.0 * std::f32::consts::TAU;
                r.center() + u * (a * s.cos()) + v * (b.max(0.5) * s.sin())
            })
            .collect();
        shapes.push(egui::Shape::convex_polygon(pts, fill, Stroke::NONE));
    }
    cx.data_shapes(shapes);
}

/// OK: edit the brush (and rename it). A name another brush has, or a new bristle brush name that
/// is too long, keeps the dialog open.
fn confirm(app: &mut VectorcraftApp, d: &DialogFields) -> Result<(), String> {
    let name = string(d, "name").trim().to_string();
    app.run("brush.options", json!({ "name": string(d, "__brush"), "newName": name, "params": params(d) })).map(|_| ())
}

#[cfg(test)]
mod tests;
