//! Object → Transform → Transform Each: scale, move and rotate every selected object about its own
//! reference point, with reflection, randomness, Transform Objects, Transform Patterns, Scale
//! Corners and Scale Strokes & Effects (`object.transformEach`), previewed live on the canvas. Its Scale and Move sliders, Rotate dial
//! and reflection, reference point and Random options are the Transform effect's too.
//!
//! Fields: `scaleH`, `scaleV` (percentages, or with Absolute each object's width and height in pt),
//! `relative` (Relative or Absolute scaling), `moveH`, `moveV` (pt), `rotate` (°), `reflectX`,
//! `reflectY`, `random`, `reference` (0..8, the 9-point grid), `copy` (set by the Copy button, which
//! confirms), `objects`, `patterns`, `corners`, `strokes` (the last two default to the preferences,
//! which OK updates) and `preview`; `__constrain` is the link that keeps the two scales in
//! proportion. It opens on the values it was last confirmed with, Copy aside (#1732).

use serde_json::{Value, json};

use super::transform::{add_object_options, add_scale_options, object_options, scale_options_ordered, with_scale_options};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

/// The dialog kind of Transform Each.
pub const KIND: &str = "transformEach";

const CMD: &str = "object.transformEach";

/// OK transforms each object; Copy, left of Cancel as in the documented dialog, transforms a copy
/// of each ([Scale multiple objects](https://helpx.adobe.com/illustrator/desktop/manage-objects/reshape-transform-objects/scale-multiple-objects.html)).
pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Transform Each").into(),
    body,
    confirm,
    alternate: Some(("Copy", "copy")),
    preview: true,
    min_width: 360.0,
    inputs: |_| &["relative", "absH", "absV", "objects", "patterns", "corners", "strokes"],
    // Preview sits at the left of the button row, level with Copy, Cancel and OK.
    footer_left: Some(form::preview_check),
    ..DialogSpec::FORM
};

/// The fields a fresh dialog starts with.
pub fn fields() -> Value {
    json!({"scaleH": 100, "scaleV": 100, "relative": true, "moveH": 0, "moveV": 0, "rotate": 0, "reflectX": false, "reflectY": false, "random": false, "reference": 4, "copy": false, "__constrain": false})
}

/// The fields kept from the last OK or Copy (`UiState::transform_each`): all but Copy, Transform
/// Patterns (the preference it starts from), the scale options (preferences too) and the Absolute
/// targets (`absH`, `absV`), which the dialog sets from the selection every time it opens.
const REMEMBERED: [&str; 13] =
    ["scaleH", "scaleV", "relative", "moveH", "moveV", "rotate", "reflectX", "reflectY", "random", "reference", "objects", "preview", LINK];

/// The fields the menu item opens with: [`fields`], with the values it was last confirmed with.
/// A kept value of the wrong kind (a hand-edited file) is left out.
pub fn open_fields(app: &VectorcraftApp) -> Value {
    let mut out = fields();
    for k in REMEMBERED {
        match app.ui.transform_each.get(k) {
            Some(v @ Value::Number(_)) if out.get(k).is_some_and(Value::is_number) => out[k] = v.clone(),
            Some(v @ Value::Bool(_)) if out.get(k).is_none_or(Value::is_boolean) => out[k] = v.clone(),
            _ => {}
        }
    }
    let relative = out.get("relative").and_then(Value::as_bool).unwrap_or(true);
    // Absolute shows the size the selected objects share, so a dialog that opens in it has it.
    if !relative {
        let (w, h) = shared_size(app, app.session.general_unit());
        for (key, v) in [("absH", w), ("absV", h)] {
            if let Some(v) = v {
                out[key] = json!(v);
            }
        }
    }
    // A link that opens on keeps the proportions of the scales it opens on, so a scale set before
    // the first frame (`ui.dialog.set`) follows it.
    if out.get(LINK).and_then(Value::as_bool) == Some(true)
        && let Some(ratio) = out.as_object().and_then(|f| link_ratio(f, relative))
    {
        out[RATIO] = json!(ratio);
    }
    out
}

/// The command's parameters without the scale options. The scales are the percentages with
/// Relative and the Absolute targets with Absolute, where a side the objects don't share (Mixed) is
/// left out, so each object keeps that side.
fn params(d: &Dialog) -> Value {
    let f = |k: &str, v: f64| d.f64(k, v);
    let mut p = json!({
        "relative": relative(d),
        "moveH": f("moveH", 0.0), "moveV": f("moveV", 0.0),
        "rotate": f("rotate", 0.0), "reflectX": d.bool("reflectX"), "reflectY": d.bool("reflectY"), "random": d.bool("random"),
        "reference": f("reference", 4.0).clamp(0.0, 8.0) as u64, "copy": d.bool("copy"),
    });
    let (h, v) = scale_keys(d);
    for (key, field) in [("scaleH", h), ("scaleV", v)] {
        // A field the dialog doesn't hold is Mixed; one kept as text ("10 mm") parses as the
        // dialog's own fields read it.
        if d.fields.contains_key(field) {
            p[key] = json!(d.f64(field, 100.0));
        }
    }
    p
}

/// The two Scale fields the dialog edits: the percentages with Relative, the Absolute targets with
/// Absolute (`absH` and `absV`, absent when the objects differ).
fn scale_keys(d: &Dialog) -> (&'static str, &'static str) {
    if relative(d) { ("scaleH", "scaleV") } else { ("absH", "absV") }
}

/// The proportions the link keeps for the scales `fields` show (vertical ÷ horizontal): none while
/// a side is Mixed.
fn link_ratio(fields: &serde_json::Map<String, Value>, relative: bool) -> Option<f64> {
    let (h, v) = if relative { ("scaleH", "scaleV") } else { ("absH", "absV") };
    let ratio = field_number(fields, v)? / field_number(fields, h)?;
    (ratio.is_finite() && ratio != 0.0).then_some(ratio)
}

/// The number `fields[key]` holds: a number, or a distance in text ("10 mm"), as the dialog's own
/// fields read them; none when the field is absent (Mixed).
fn field_number(fields: &serde_json::Map<String, Value>, key: &str) -> Option<f64> {
    match fields.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => vectorcraft_doc::Unit::Points.parse(s),
        _ => None,
    }
}

/// Relative scaling (field `relative`, on until switched off): percentages of each object's size,
/// rather than the width and height it becomes (Absolute).
fn relative(d: &Dialog) -> bool {
    d.fields.get("relative").and_then(Value::as_bool).unwrap_or(true)
}

/// Width of the label column and the value fields of [`sections`].
const LABEL_W: f32 = 72.0;
const FIELD_W: f32 = 78.0;
/// Width of the Angle field: the dial leaves it room to be wider than a Scale or Move field.
const ANGLE_FIELD_W: f32 = 94.0;
/// Width of the Constrain Width and Height Proportions link's column, at the right of the Scale
/// rows.
const LINK_W: f32 = 22.0;
/// Largest scale (%) the fields take (as `object.transformEach` and the Transform effect).
const MAX_SCALE: f64 = 100_000.0;
/// Largest width or height (pt) the Scale fields take with Absolute (as `object.transformEach`).
const MAX_ABSOLUTE: f64 = 1.0e7;
/// The Scale sliders' rails: percentages, and points with Absolute (the fields take values past
/// the rails).
const SCALE_RAIL: std::ops::RangeInclusive<f64> = 0.0..=200.0;
const ABSOLUTE_RAIL: std::ops::RangeInclusive<f64> = 0.0..=1000.0;

/// The dialog fields of the Scale link, as the shape dialogs' Width and Height link: its state, the
/// state the dialog last applied (a change by its button or from outside shows as a difference
/// from it) and the proportions it keeps (vertical ÷ horizontal).
const LINK: &str = "__constrain";
const APPLIED: &str = "__constrainApplied";
const RATIO: &str = "__ratio";

/// The Scale section: a slider and a field for `scaleH` and for `scaleV`, percentages of each
/// object's size, or with `absolute` the width and height it becomes, in `units` (Transform Each
/// alone: the Transform effect scales in percentages), and the link that keeps them in proportion.
fn scale_section(ui: &mut egui::Ui, d: &mut Dialog, units: vectorcraft_doc::Unit, absolute: bool) {
    widgets::subheader(ui, tl!("Scale"));
    link_changed(d);
    if let Some(rows) = scale_rows(ui, d, units, false, absolute, true) {
        scale_link(ui, d, rows);
    }
}

/// The two Scale rows, and with `link` the column at their right that [`scale_link`] fills. Returns
/// the two rows' bounds. While the link is on, a scale that changes sets the other.
fn scale_rows(ui: &mut egui::Ui, d: &mut Dialog, units: vectorcraft_doc::Unit, live: bool, absolute: bool, link: bool) -> Option<egui::Rect> {
    let rail = Tokens::get(ui.ctx()).input_border;
    let track = move |_| rail;
    let axes = |h, v| [(h, tl!("Horizontal:")), (v, tl!("Vertical:"))];
    let mut rows: Option<egui::Rect> = None;
    for (key, label) in axes(if absolute { "absH" } else { "scaleH" }, if absolute { "absV" } else { "scaleV" }) {
        let before = d.f64(key, 100.0);
        let row = ui
            .scope(|ui| {
                widgets::label_row(ui, label, LABEL_W, |ui| {
                    if absolute {
                        // A side the objects don't share reads "Mixed" until a size is typed.
                        let shown = form::length_value(d, key, units, ABSOLUTE_RAIL);
                        form::slider_rail_valued(ui, d, key, label, ABSOLUTE_RAIL, shown, &track);
                        form::length_mixed(ui, d, key, units, FIELD_W, Some(-MAX_ABSOLUTE..=MAX_ABSOLUTE), tl!("Mixed"));
                    } else {
                        let shown = widgets::SliderValue::field(d.f64(key, 100.0).clamp(-MAX_SCALE, MAX_SCALE), SCALE_RAIL, "%", 2);
                        form::slider_rail_valued(ui, d, key, label, SCALE_RAIL, Some(shown), &track);
                        let value = if live {
                            widgets::range_field_live(ui, ("te", key), d.f64(key, 100.0), -MAX_SCALE..=MAX_SCALE, "%", 2, FIELD_W)
                        } else {
                            widgets::range_field(ui, ("te", key), d.f64(key, 100.0), -MAX_SCALE..=MAX_SCALE, "%", 2, FIELD_W)
                        };
                        if let Some(n) = value {
                            d.fields.insert(key.into(), json!(n));
                        }
                    }
                    if link {
                        ui.add_space(LINK_W + ui.spacing().item_spacing.x);
                    }
                });
            })
            .response
            .rect;
        if d.f64(key, 100.0) != before {
            follow_link(d, key);
        }
        rows = Some(rows.map_or(row, |rows| rows.union(row)));
    }
    rows
}

/// The Constrain Width and Height Proportions link, drawn over the two Scale rows' right end as the
/// shape dialogs' link is over theirs (field [`LINK`]).
fn scale_link(ui: &mut egui::Ui, d: &mut Dialog, rows: egui::Rect) {
    let rect = egui::Rect::from_center_size(egui::pos2(rows.right() - LINK_W / 2.0, rows.center().y), egui::vec2(LINK_W, LINK_W));
    let resp = ui.interact(rect, ui.scope_id().with("te-scale-link"), egui::Sense::click());
    let on = d.bool(LINK);
    let icon = if on { "link" } else { "link-2-off" };
    if widgets::paint_icon_button(ui, resp, icon, "Constrain Width and Height Proportions", on).clicked() {
        d.fields.insert(LINK.into(), json!(!on));
        link_changed(d);
    }
}

/// When the link's state differs from the one the dialog last applied (or none was applied yet),
/// apply it and return it: turned on, the link keeps the proportions of the scales the dialog shows
/// then; turned off, it forgets them. A scale of zero when it turns on leaves nothing to keep, so
/// typing changes only the side typed until the link is turned off and on again.
fn link_changed(d: &mut Dialog) -> Option<bool> {
    let on = d.bool(LINK);
    if d.fields.get(APPLIED).and_then(Value::as_bool) == Some(on) {
        return None;
    }
    d.fields.insert(APPLIED.into(), json!(on));
    keep_link_proportions(d);
    Some(on)
}

/// The link keeps the proportions of the scales the dialog shows now (none while a side is Mixed).
fn keep_link_proportions(d: &mut Dialog) {
    match link_ratio(&d.fields, relative(d)).filter(|_| d.bool(LINK)) {
        Some(ratio) => {
            d.fields.insert(RATIO.into(), json!(ratio));
        }
        None => {
            d.fields.remove(RATIO);
        }
    }
}

/// After the scale `typed` ("scaleH" or "scaleV") changed with the link on, set the other one from
/// the proportions the link keeps, within the range its field takes (so the field shows what the
/// command applies at the limits).
fn follow_link(d: &mut Dialog, typed: &str) {
    let ratio = d.f64(RATIO, 0.0);
    let (h, v) = scale_keys(d);
    if !d.bool(LINK) || !ratio.is_finite() || ratio == 0.0 || (typed != h && typed != v) {
        return;
    }
    let (other, value) = if typed == h { (v, d.f64(h, 100.0) * ratio) } else { (h, d.f64(v, 100.0) / ratio) };
    let limit = if relative(d) { MAX_SCALE } else { MAX_ABSOLUTE };
    if value.is_finite() {
        d.fields.insert(other.into(), json!(value.clamp(-limit, limit)));
    }
}

/// The Scale section's Relative and Absolute radio buttons, under its sliders (field `relative`).
/// Absolute shows the size the selected objects share, set again each time it is chosen.
fn scale_mode_radios(ui: &mut egui::Ui, d: &mut Dialog, app: &VectorcraftApp, units: vectorcraft_doc::Unit) {
    let relative = relative(d);
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
        for (label, on) in [(tl!("Relative"), true), (tl!("Absolute"), false)] {
            if widgets::radio(ui, label, relative == on, true) && relative != on {
                d.fields.insert("relative".into(), json!(on));
                if on {
                    keep_link_proportions(d);
                } else {
                    let size = shared_size(app, units);
                    set_absolute_targets(d, size);
                }
            }
        }
    });
}

/// `ui.dialog.set` changed a field the Scale section reads: a switch to Absolute shows the size the
/// objects share, and the link (`__constrain`) and its two scales follow as a click or a typed
/// value does.
pub(super) fn field_set(app: &mut VectorcraftApp, field: &str, old: Option<&Value>) {
    match field {
        "relative" => {
            let size = shared_size(app, app.session.general_unit());
            let Some(d) = app.ui.dialog.as_mut() else { return };
            let mode = relative(d);
            if old.and_then(Value::as_bool) != Some(mode) {
                if mode {
                    keep_link_proportions(d);
                } else {
                    set_absolute_targets(d, size);
                }
            }
        }
        LINK => {
            if let Some(d) = app.ui.dialog.as_mut() {
                link_changed(d);
            }
        }
        "scaleH" | "scaleV" | "absH" | "absV" => {
            if let Some(d) = app.ui.dialog.as_mut() {
                follow_link(d, field);
            }
        }
        _ => {}
    }
}

/// The Absolute targets (`absH`, `absV`) for the selection's [`shared_size`]: the width and height
/// its objects share, and none (the fields read "Mixed") when they differ.
fn set_absolute_targets(d: &mut Dialog, (w, h): (Option<f64>, Option<f64>)) {
    for (key, v) in [("absH", w), ("absV", h)] {
        match v {
            Some(v) => {
                d.fields.insert(key.into(), json!(v));
            }
            None => {
                d.fields.remove(key);
            }
        }
    }
    keep_link_proportions(d);
}

/// The objects a transform of `ids` acts on, as the command's own selection roots: a compound
/// path's member stands for its compound, and layers and objects a listed object holds are left
/// out.
fn transform_roots(doc: &vectorcraft_doc::Document, ids: &[vectorcraft_doc::NodeId]) -> Vec<vectorcraft_doc::NodeId> {
    let roots: Vec<vectorcraft_doc::NodeId> = ids
        .iter()
        .map(|id| match doc.parent_of(*id).and_then(|p| doc.node(p)) {
            Some(parent) if matches!(parent.kind, vectorcraft_doc::NodeKind::Compound { .. }) => parent.id,
            _ => *id,
        })
        .collect();
    let listed: std::collections::HashSet<vectorcraft_doc::NodeId> = roots.iter().copied().collect();
    roots.iter().copied().filter(|id| doc.node(*id).is_some_and(|n| !n.is_layer()) && !held_by_listed(doc, *id, &listed)).collect()
}

/// Whether an object `listed` holds is above `id` (one walk up, not one per pair of objects: a
/// selection can be tens of thousands of objects).
fn held_by_listed(doc: &vectorcraft_doc::Document, id: vectorcraft_doc::NodeId, listed: &std::collections::HashSet<vectorcraft_doc::NodeId>) -> bool {
    let mut cur = id;
    while let Some(parent) = doc.parent_of(cur) {
        if listed.contains(&parent) {
            return true;
        }
        cur = parent;
    }
    false
}

/// The size `sizes` share, or none when they differ: sizes the panels show alike
/// ([`Unit::number`], four decimals, trailing zeros trimmed) count as the same, and the first is
/// the one shown, so the fields read the same number the Transform panel and the Control bar give
/// each object. A side of 0 (a line's height) is none too: the command refuses a scale of 0, and
/// the objects keep that side anyway.
fn shared_side(unit: vectorcraft_doc::Unit, sizes: impl Iterator<Item = f64>) -> Option<f64> {
    let mut sizes = sizes;
    let first = sizes.next()?;
    if first.abs() <= 1.0e-9 {
        return None;
    }
    let shown = unit.number(first);
    sizes.all(|size| unit.number(size) == shown).then_some(first)
}

/// The width and height the selected objects share (each `None` when they differ), measured on
/// their own geometric bounds and without a preview: with Preview on, the document holds the
/// transform the dialog has shown so far.
fn shared_size(app: &VectorcraftApp, unit: vectorcraft_doc::Unit) -> (Option<f64>, Option<f64>) {
    // The objects the command transforms (a compound path's member stands for its compound), each
    // measured by its own box, rotated with it, as the Transform panel and the Control bar show it:
    // a rotated circle's axis-aligned bounds run a hair larger than the circle.
    let measure = |doc: &vectorcraft_doc::Document, ids: &[vectorcraft_doc::NodeId]| -> Vec<(f64, f64)> {
        // One symbol cache for all of them: many instances of one symbol measure its art once.
        let symbols = doc.symbol_boxes();
        transform_roots(doc, ids)
            .iter()
            .filter_map(|id| doc.oriented_bounds_with(&[*id], false, &symbols))
            .map(|b| (b.rect.width(), b.rect.height()))
            .collect()
    };
    let bounds = match app.session.active().and_then(|st| st.interaction.as_ref()) {
        Some(it) => measure(&it.doc, &it.selection.objects),
        None => app.session.doc().ok().map(|st| measure(&st.doc, &st.selection.objects)).unwrap_or_default(),
    };
    let shared = |side: fn(&(f64, f64)) -> f64| shared_side(unit, bounds.iter().map(side));
    (shared(|b| b.0), shared(|b| b.1))
}

/// The Move and Rotate sections of Transform Each and the Transform effect: sliders for `moveH`
/// and `moveV` (rails ±1000 pt, fields in `units`), and an angle dial for `rotate`
/// (counter-clockwise). The fields take values past the rails.
fn move_and_rotate(ui: &mut egui::Ui, d: &mut Dialog, units: vectorcraft_doc::Unit, live: bool) {
    let rail = Tokens::get(ui.ctx()).input_border;
    let track = move |_| rail;
    let axes = |h, v| [(h, tl!("Horizontal:")), (v, tl!("Vertical:"))];
    widgets::subheader(ui, tl!("Move"));
    for (key, label) in axes("moveH", "moveV") {
        widgets::label_row(ui, label, LABEL_W, |ui| {
            let shown = form::length_value(d, key, units, -1000.0..=1000.0);
            form::slider_rail_valued(ui, d, key, label, -1000.0..=1000.0, shown, &track);
            if live {
                if let Some(n) = widgets::num_field_live(ui, ("dlg-len", key), Some(d.f64(key, 0.0)), units, FIELD_W) {
                    d.fields.insert(key.into(), json!(n));
                }
            } else {
                form::length(ui, d, key, units, FIELD_W);
            }
        });
    }
    widgets::subheader(ui, tl!("Rotate"));
    widgets::label_row(ui, tl!("Angle:"), LABEL_W, |ui| {
        let angle = d.f64("rotate", 0.0);
        let dialed = widgets::angle_dial(ui, "te-dial", angle, 28.0);
        let typed = if live {
            widgets::plain_field_live(ui, ("te", "rotate"), angle, "°", 2, ANGLE_FIELD_W)
        } else {
            widgets::plain_field(ui, ("te", "rotate"), angle, "°", 2, ANGLE_FIELD_W)
        };
        if let Some(a) = typed.or(dialed) {
            d.fields.insert("rotate".into(), json!(a));
        }
    });
}

/// The Scale, Move and Rotate sections of Transform Each and the Transform effect: sliders for
/// `scaleH` and `scaleV` (rails 0–200 %), sliders for `moveH` and `moveV` (rails ±1000 pt, fields
/// in `units`), and an angle dial for `rotate` (counter-clockwise). The Transform effect has no
/// link between its two scales.
pub(super) fn sections(ui: &mut egui::Ui, d: &mut Dialog, units: vectorcraft_doc::Unit, live: bool) {
    widgets::subheader(ui, tl!("Scale"));
    scale_rows(ui, d, units, live, false, false);
    move_and_rotate(ui, d, units, live);
}

/// The Reflect X, Reflect Y and Random options of Transform Each and the Transform effect.
pub(super) fn reflect_checks(ui: &mut egui::Ui, d: &mut Dialog) {
    form::check(ui, d, "reflectX", tl!("Reflect X"));
    form::check(ui, d, "reflectY", tl!("Reflect Y"));
    form::check(ui, d, "random", tl!("Random"));
}

/// The reference point locator (field `reference`) and its label.
pub(super) fn reference_point_row(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        let cur = d.f64("reference", 4.0).clamp(0.0, 8.0) as usize;
        if let Some(i) = widgets::reference_point(ui, cur) {
            d.fields.insert("reference".into(), json!(i));
        }
        widgets::dim_label(ui, tl!("Reference Point"));
    });
}

/// The Reflect X, Reflect Y, reference point and Random options of the Transform effect.
pub(super) fn reflect_options(ui: &mut egui::Ui, d: &mut Dialog) {
    form::check(ui, d, "reflectX", tl!("Reflect X"));
    form::check(ui, d, "reflectY", tl!("Reflect Y"));
    reference_point_row(ui, d);
    form::check(ui, d, "random", tl!("Random"));
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let units = app.session.general_unit();
    // The rows scroll when the window is shorter than they are, so the heading, Preview and the
    // buttons stay in view.
    super::modal::scrolled_rows(ui, "transform-each-rows", |ui| {
        scale_section(ui, d, units, !relative(d));
        scale_mode_radios(ui, d, app, units);
        move_and_rotate(ui, d, units, false);
        widgets::subheader(ui, tl!("Options"));
        ui.add_space(4.0);
        // The options in two columns: the object and scale options, then the reflections and
        // Random, which start where the Scale and Move fields do.
        let fields_x = LABEL_W + 2.0 * ui.spacing().item_spacing.x + form::SLIDER_WIDTH;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(fields_x - ui.spacing().item_spacing.x);
                object_options(app, ui, d);
                scale_options_ordered(app, ui, d, true);
            });
            ui.vertical(|ui| reflect_checks(ui, d));
        });
        ui.add_space(6.0);
        reference_point_row(ui, d);
    });
    let mut p = params(d);
    add_object_options(app, d, &mut p);
    add_scale_options(app, d, &mut p);
    form::preview_without_check(app, d, "Transform Each", CMD, p);
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = params(d);
    add_object_options(app, d, &mut p);
    let p = with_scale_options(app, d, p)?;
    let mut kept = serde_json::Map::new();
    for k in REMEMBERED {
        let v = match k {
            "preview" | LINK => json!(d.bool(k)),
            // The percentages stay the dialog's own, whichever mode the scales were applied in.
            "scaleH" | "scaleV" => json!(d.f64(k, 100.0)),
            _ => p[k].clone(),
        };
        kept.insert(k.into(), v);
    }
    let r = form::commit_preview(app, CMD, p)?;
    app.ui.transform_each = Value::Object(kept);
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::NodeId;
    use vectorcraft_engine::Session;

    #[test]
    fn ok_transforms_each_and_keeps_the_scale_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("prefs.set", json!({"key": "scaleStrokes", "value": true})).unwrap();
        app.run("ui.menuDialog", json!({ "command": CMD })).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        // One drawn frame: the checkboxes start from the preferences.
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(d.fields["strokes"], json!(true));
        d.fields.insert("scaleH".into(), json!(200));
        d.fields.insert("scaleV".into(), json!(200));
        d.fields.insert("strokes".into(), json!(false));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert!(!app.session.prefs.scale_strokes, "OK keeps the option as the preference");
        let n = app.session.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
        assert!((n.geometric_bounds().unwrap().width() - 100.0).abs() < 1e-6);
        assert_eq!(n.appearance.stroke_width(), 1.0, "strokes kept their weight");
        let (cmd, p) = app.session.journal.last().unwrap();
        assert_eq!((cmd.as_str(), &p["strokes"], &p["objects"]), (CMD, &json!(false), &json!(true)), "#1448: Transform Objects goes along");
    }

    /// An app with a selected 50 × 50 square at (20, 20).
    fn square_app() -> (VectorcraftApp, NodeId) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        (app, NodeId(id))
    }

    fn open(app: &mut VectorcraftApp) -> &mut Dialog {
        crate::menus::invoke(app, CMD, json!({}));
        let d = app.ui.dialog.as_mut().expect("Transform Each is open");
        assert_eq!(d.kind, KIND);
        d
    }

    #[test]
    fn a_fresh_app_opens_on_the_defaults() {
        let (mut app, _) = square_app();
        let d = open(&mut app);
        assert_eq!(Value::Object(d.fields.clone()), fields());
    }

    /// A dialog that opens with the link on keeps the proportions of the scales it opens on, so a
    /// scale set before the first frame (`ui.dialog.set`) follows it.
    #[test]
    fn a_link_that_opens_on_keeps_the_proportions_it_opens_on() {
        let (mut app, _) = square_app();
        app.ui.transform_each = json!({"scaleH": 100, "scaleV": 200, "__constrain": true});
        assert_eq!(open(&mut app).f64("__ratio", 0.0), 2.0);
        app.ui.transform_each = json!({"scaleH": 100, "scaleV": 200, "__constrain": false});
        assert_eq!(open(&mut app).fields.get("__ratio"), None);
    }

    /// #1732: the dialog opens on the values it was last confirmed with (as Move and Offset Path
    /// do), but never on Copy.
    #[test]
    fn it_reopens_on_the_values_confirmed_last() {
        let (mut app, _) = square_app();
        let d = open(&mut app);
        for (k, v) in [
            ("scaleH", json!(150)),
            ("relative", json!(false)),
            ("moveH", json!(20)),
            ("rotate", json!(15)),
            ("reflectX", json!(true)),
            ("random", json!(true)),
            ("reference", json!(0)),
            ("objects", json!(false)),
            ("patterns", json!(true)),
            ("copy", json!(true)),
        ] {
            d.fields.insert(k.into(), v);
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let d = open(&mut app);
        for (k, v) in [
            ("scaleH", json!(150.0)),
            ("scaleV", json!(100.0)),
            ("relative", json!(false)),
            ("moveH", json!(20.0)),
            ("rotate", json!(15.0)),
            ("reflectX", json!(true)),
            ("reflectY", json!(false)),
            ("random", json!(true)),
            ("reference", json!(0)),
            ("objects", json!(false)),
            ("preview", json!(false)),
            ("copy", json!(false)),
        ] {
            assert_eq!(d.fields.get(k), Some(&v), "{k}");
        }
        // Cancel keeps nothing new.
        d.fields.insert("scaleH".into(), json!(300));
        super::super::cancel(&mut app);
        assert_eq!(open(&mut app).fields["scaleH"], json!(150.0));
        // The remembered values go with the UI state; one saved before they existed loads, and
        // opens on the defaults.
        app.ui.dialog = None;
        app.ui = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert_eq!(open(&mut app).fields["scaleH"], json!(150.0));
        app.ui = serde_json::from_value(json!({"move_options": {"copy": true}})).unwrap();
        assert_eq!(Value::Object(open(&mut app).fields.clone()), fields());
    }

    /// #1732: the Copy button transforms a copy of each object, in one undo step, and Transform
    /// Again makes a further copy; there is no Copy checkbox.
    #[test]
    fn the_copy_button_transforms_a_copy_of_each_object() {
        use egui_kittest::kittest::Queryable;
        let (mut app, id) = square_app();
        let d = open(&mut app);
        d.fields.insert("moveH".into(), json!(100));
        let undo = app.session.doc().unwrap().history.undo.len();
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                super::super::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(4);
        assert!(h.query_by_role_and_label(egui::accesskit::Role::CheckBox, "Copy").is_none(), "no Copy checkbox");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Copy").click();
        h.run_steps(3);
        let app = h.state_mut();
        assert!(app.ui.dialog.is_none(), "Copy closes the dialog");
        let xs = |app: &VectorcraftApp| {
            let st = app.session.doc().unwrap();
            let mut xs: Vec<f64> = st.doc.layers[0].children().unwrap().iter().map(|n| n.geometric_bounds().unwrap().x0.round()).collect();
            xs.sort_by(f64::total_cmp);
            xs
        };
        assert_eq!(xs(app), vec![20.0, 120.0], "the square stays, its copy moved 100 pt");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        assert!(app.session.doc().unwrap().doc.node(id).is_some());
        assert_eq!(open(app).fields["copy"], json!(false), "Copy isn't remembered");
        app.ui.dialog = None;
        app.run("object.transformAgain", json!({})).unwrap();
        assert_eq!(xs(app), vec![20.0, 120.0, 220.0], "Transform Again copies once more");
        h.ctx.forget_all_images();
    }
}
