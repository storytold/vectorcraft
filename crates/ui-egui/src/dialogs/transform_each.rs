//! Object → Transform → Transform Each: scale, move and rotate every selected object about its own
//! reference point, with reflection, randomness, Transform Objects, Transform Patterns, Scale
//! Corners and Scale Strokes & Effects (`object.transformEach`), previewed live on the canvas. Its
//! Scale, Move and Rotate rows, the reflection checks and the reference point are the Transform
//! effect's too ([`rows`], [`reflect_checks`], [`reference_point`]).
//!
//! Fields: `scaleH`, `scaleV` (percentages, or with Absolute each object's width and height in pt),
//! `relative` (Relative or Absolute scaling), `moveH`, `moveV` (pt), `rotate` (°), `reflectX`,
//! `reflectY`, `random`, `reference` (0..8, the 9-point grid), `copy` (set by the Copy button, which
//! confirms), `objects`, `patterns`, `corners`, `strokes` (the last two default to the preferences,
//! which OK updates) and `preview`; `__constrain` is the link that keeps the two scales in
//! proportion. It opens on the values it was last confirmed with, Copy aside (#1732).
//!
//! The dialog is a held-OK [`HostedParamDialog`]: a refused OK keeps it open. The link and mode
//! rules run in its build closure ([`apply_links`]), so a click, a typed value and an automation
//! set all follow the same rule.

use craft::behavior::Mixed;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, FieldLabelRole, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft::ui::dialog::{DialogHost, DialogId, FieldRefusal};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::pickers::{Anchor9, ReferencePoint, ReferencePointLook};
use craft::ui::runtime::env::{Label, LengthUnits, UnitKind};
use craft::ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::text::{NumberField, follow_ratio, link_toggle};
use craft::ui::value_drag::{AngleDial, Slider};
use egui::{Response, Ui};
use serde_json::{Value, json};
use std::ops::RangeInclusive;

use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview; // PLAN: assumed path of wg.A20's `LivePreview`
use super::transform::{add_object_options, add_scale_options, with_scale_options}; // PLAN: assumed (app, &DialogFields, ..) forms of od.OS6
use crate::VectorcraftApp;

const CMD: &str = "object.transformEach";
const KEY: &str = "transformEach";
/// The footer action and field of the Copy button.
const COPY: &str = "copy";

/// Where the values OK confirmed last are kept. Not `.remember`: that drops custom rows.
pub(crate) const EACH_KEY: SaveKey = SaveKey::new("vectorcraft.transform_each");

/// The open dialog: its id and the live preview of its values.
pub(crate) struct Open {
    /// The id of the held dialog.
    pub id: DialogId,
    /// The preview OK commits and Cancel rolls back.
    pub preview: LivePreview,
    /// The opening document's uid and revert count.
    document: (u64, u64),
    /// The tool reset count when the current preview record was created.
    preview_tool_resets: u64,
}

impl Open {
    fn in_source(&self, app: &VectorcraftApp) -> bool {
        app.session.active().is_some_and(|doc| (doc.uid, doc.reverts) == self.document)
    }

    /// Whether the active document is the opening one, reverted since the dialog opened.
    fn reverted(&self, app: &VectorcraftApp) -> bool {
        let (uid, reverts) = self.document;
        app.session.active().is_some_and(|doc| doc.uid == uid && doc.reverts != reverts)
    }
}

/// Whether [`rows`] draws the link and the Relative / Absolute choice (Transform Each) or only the
/// two percentage scales (the Transform effect).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScaleLink {
    /// Percentages only.
    None,
    /// The proportion link and the Relative / Absolute choice.
    Toggle,
}

/// The width and height the selected objects share, each none when they differ.
type Shared = (Option<f64>, Option<f64>);

/// The fields a fresh dialog starts with.
pub fn fields() -> DialogFields {
    DialogFields::from_pairs([
        ("scaleH", FieldValue::Number(100.0)),
        ("scaleV", FieldValue::Number(100.0)),
        ("relative", FieldValue::Bool(true)),
        ("moveH", FieldValue::Number(0.0)),
        ("moveV", FieldValue::Number(0.0)),
        ("rotate", FieldValue::Number(0.0)),
        ("reflectX", FieldValue::Bool(false)),
        ("reflectY", FieldValue::Bool(false)),
        ("random", FieldValue::Bool(false)),
        ("reference", FieldValue::Int(4)),
        (COPY, FieldValue::Bool(false)),
        (LINK, FieldValue::Bool(false)),
    ])
}

/// The fields kept from the last OK or Copy: all but Copy, Preview (it opens on), Transform
/// Patterns (the preference it starts from), the scale options (preferences too) and the Absolute
/// targets (`absH`, `absV`), which the dialog sets from the selection every time it opens.
const REMEMBERED: [&str; 12] =
    ["scaleH", "scaleV", "relative", "moveH", "moveV", "rotate", "reflectX", "reflectY", "random", "reference", "objects", LINK];

/// Whether a kept value has the kind of the field it would replace (a hand-edited file may not).
fn same_kind(current: Option<&FieldValue>, kept: &FieldValue) -> bool {
    match kept {
        FieldValue::Bool(_) => current.is_none_or(|c| matches!(c, FieldValue::Bool(_))),
        FieldValue::Number(_) | FieldValue::Int(_) => matches!(current, Some(FieldValue::Number(_) | FieldValue::Int(_))),
        _ => false,
    }
}

/// The values the dialog opens with: [`fields`] and the preferences' options, with the values it
/// was last confirmed with over them.
fn opening_fields(app: &VectorcraftApp, ctx: &egui::Context) -> DialogFields {
    let mut f = fields();
    let prefs = &app.session.prefs;
    for (key, on) in
        [("objects", true), ("patterns", prefs.transform_pattern_tiles), ("strokes", prefs.scale_strokes), ("corners", prefs.scale_corners)]
    {
        f.set(key, on);
    }
    if let Some(saved) = saved_value::<Option<DialogFields>>(ctx, &EACH_KEY) {
        for k in REMEMBERED {
            if let Some(v) = saved.get(k).filter(|v| same_kind(f.get(k), v)) {
                f.set(k, v.clone());
            }
        }
    }
    f
}

/// The command's parameters without the scale options. The scales are the percentages with
/// Relative and the Absolute targets with Absolute, where a side the objects don't share (Mixed) is
/// left out, so each object keeps that side.
fn params(f: &DialogFields) -> Value {
    let mut p = json!({
        "relative": relative(f),
        "moveH": num(f, "moveH", 0.0), "moveV": num(f, "moveV", 0.0),
        "rotate": num(f, "rotate", 0.0), "reflectX": flag(f, "reflectX"), "reflectY": flag(f, "reflectY"), "random": flag(f, "random"),
        "reference": num(f, "reference", 4.0).clamp(0.0, 8.0) as u64, "copy": flag(f, COPY),
    });
    let (h, v) = scale_keys(f);
    for (key, field) in [("scaleH", h), ("scaleV", v)] {
        if let Some(n) = field_number(f, field) {
            p[key] = json!(n);
        }
    }
    p
}

/// [`params`] with the Transform Objects and Transform Patterns options.
fn command_params(app: &VectorcraftApp, f: &DialogFields) -> Value {
    let mut p = params(f);
    add_object_options(app, f, &mut p);
    p
}

/// The two Scale fields the dialog edits: the percentages with Relative, the Absolute targets with
/// Absolute (`absH` and `absV`, absent when the objects differ).
fn scale_keys(f: &DialogFields) -> (&'static str, &'static str) {
    if relative(f) { ("scaleH", "scaleV") } else { ("absH", "absV") }
}

/// The scales `h` and `v` the link keeps in proportion, as they are now: none while a side is
/// Mixed or zero (the command refuses a scale of 0, and a zero side leaves no proportion).
fn ratio_base(f: &DialogFields, h: &str, v: &str) -> Option<[f64; 2]> {
    let (a, b) = (field_number(f, h)?, field_number(f, v)?);
    let ratio = b / a;
    (ratio.is_finite() && ratio != 0.0).then_some([a, b])
}

/// The number `key` holds: a number, or a distance in text ("10 mm"), as the dialog's own fields
/// read them; none when the field is absent (Mixed).
fn field_number(f: &DialogFields, key: &str) -> Option<f64> {
    f.number(key).or_else(|| f.text(key).and_then(|s| vectorcraft_doc::Unit::Points.parse(s)))
}

/// `key` as a number, or `default` when absent.
fn num(f: &DialogFields, key: &str, default: f64) -> f64 {
    field_number(f, key).unwrap_or(default)
}

/// `key` as a check box, off when absent.
fn flag(f: &DialogFields, key: &str) -> bool {
    f.bool(key).unwrap_or(false)
}

/// Relative scaling (field `relative`, on until switched off): percentages of each object's size,
/// rather than the width and height it becomes (Absolute).
fn relative(f: &DialogFields) -> bool {
    f.bool("relative").unwrap_or(true)
}

/// Largest scale (%) the fields take (as `object.transformEach` and the Transform effect).
const MAX_SCALE: f64 = 100_000.0;
/// Largest width or height (pt) the Scale fields take with Absolute (as `object.transformEach`).
const MAX_ABSOLUTE: f64 = 1.0e7;
/// The Scale sliders' rails: percentages, and points with Absolute (the fields take values past
/// the rails).
const SCALE_RAIL: RangeInclusive<f64> = 0.0..=200.0;
const ABSOLUTE_RAIL: RangeInclusive<f64> = 0.0..=1000.0;
/// The Move sliders' rails (pt).
const MOVE_RAIL: RangeInclusive<f64> = -1000.0..=1000.0;

/// The dialog fields of the Scale link: its state, the state last applied (a change by its button
/// or from outside shows as a difference from it), the proportions it keeps, the mode last applied
/// and the scales last seen (a difference from them is an edit the link follows).
const LINK: &str = "__constrain";
const APPLIED: &str = "__constrainApplied";
const BASE: &str = "__ratioBase";
const MODE: &str = "__mode";
const SEEN: &str = "__seen";
/// The width and height the selected objects share ([`shared_size`]), kept for the set-time rule.
const SHARED: [&str; 2] = ["__sharedH", "__sharedV"];

/// The link and mode rules, run before every pass and before a confirm or an automation set: a
/// switch to Absolute shows the size the objects share, a turned link or a mode switch takes the
/// proportions the scales show then, and while the link is on an edited scale sets the other from
/// those proportions (the base is the values when the link turned on, so nothing drifts).
fn apply_links(f: &mut DialogFields, shared: Shared) {
    let relative = relative(f);
    let linked = flag(f, LINK);
    let switched = f.bool(MODE) != Some(relative);
    if switched {
        f.set(MODE, relative);
        if !relative {
            set_absolute_targets(f, shared);
        }
    }
    let (h, v) = scale_keys(f);
    let turned = f.bool(APPLIED) != Some(linked);
    if turned {
        f.set(APPLIED, linked);
    }
    let pair = |f: &DialogFields| Some([field_number(f, h)?, field_number(f, v)?]);
    if switched || turned {
        match ratio_base(f, h, v).filter(|_| linked) {
            Some(base) => f.set(BASE, FieldValue::Point(base)),
            None => {
                f.remove(BASE);
            }
        }
    } else if linked
        && let (Some(base), Some(seen), Some(now)) = (f.point(BASE), f.point(SEEN), pair(f))
        && let Some(changed) = (0..2).find(|&i| now[i] != seen[i])
    {
        let limit = if relative { MAX_SCALE } else { MAX_ABSOLUTE };
        let out = follow_ratio(base, changed, now[changed], &[-limit..=limit, -limit..=limit]);
        f.set(h, out[0]);
        f.set(v, out[1]);
    }
    match pair(f) {
        Some(now) => f.set(SEEN, FieldValue::Point(now)),
        None => {
            f.remove(SEEN);
        }
    }
}

/// [`shared_size`] as the fields keep it under [`SHARED`].
fn shared_of(f: &DialogFields) -> Shared {
    (f.number(SHARED[0]), f.number(SHARED[1]))
}

/// The Transform row's set-time rule: automation's value is written as the row keeps it (a size may come as text in a
/// unit, "10 mm"), then the link and mode rules run as before a pass, so a switch to Absolute shows the shared size and
/// a scale set with the link on sets the other.
fn set_from_outside(f: &mut DialogFields, key: &str, value: Value) -> Result<(), String> {
    let refused = || FieldRefusal::Rejects(&value).message(key);
    match (&value, matches!(key, "relative" | LINK)) {
        (Value::Bool(on), true) => f.set(key, *on),
        (Value::Number(n), false) => f.set(key, n.as_f64().ok_or_else(refused)?),
        (Value::String(text), false) => f.set(key, vectorcraft_doc::Unit::Points.parse(text).ok_or_else(refused)?),
        _ => return Err(refused()),
    }
    apply_links(f, shared_of(f));
    Ok(())
}

/// The Absolute targets (`absH`, `absV`) for the selection's [`shared_size`]: the width and height
/// its objects share, and none (the fields read "Mixed") when they differ.
fn set_absolute_targets(f: &mut DialogFields, (w, h): Shared) {
    for (key, v) in [("absH", w), ("absV", h)] {
        match v {
            Some(v) => f.set(key, v),
            None => {
                f.remove(key);
            }
        }
    }
}

/// What a Scale or Move field counts: percentages, or lengths kept in points and shown in the
/// document's unit.
#[derive(Clone, Copy)]
enum Measure {
    Percent,
    Length(LengthUnits),
}

/// One captioned side: a slider over `rail` and a number field in ±`limit`, `value` being none
/// while the objects differ (the field reads Mixed). `caption` is the row (Scale or Move) and the
/// visible label; the label repeats in each row, so it only describes the controls, and their
/// names start with the row. Returns the value the user set this pass.
fn side(
    ui: &mut Ui,
    key: &'static str,
    (row, label): (&'static str, &'static str),
    value: Option<f64>,
    measure: Measure,
    rail: &RangeInclusive<f64>,
    limit: f64,
) -> Option<f64> {
    let name: Label = format!("{row} {label}").into();
    let mut edit = None;
    Field::new(label).side(SideLabel::new().role(FieldLabelRole::Describes)).id_salt(key).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let shown = value.map_or(Mixed::Mixed, |v| Mixed::Same(Some(v)));
            let dragged = Slider::new(Bind::given(value.map_or(Mixed::Mixed, |v| Mixed::Same(v.clamp(*rail.start(), *rail.end())))), rail.clone())
                .label(name.clone())
                .id_salt("slider")
                .show(ui)
                .value()
                .copied();
            let field = match measure {
                Measure::Percent => NumberField::new(name.clone(), Bind::given(shown)).unit_kind(UnitKind::Percent),
                Measure::Length(lengths) => NumberField::length(name.clone(), Bind::given(shown), lengths),
            };
            let typed = field.range(-limit..=limit).id_salt("field").show(ui).edit.value().cloned().flatten();
            edit = typed.or(dragged);
        });
    });
    edit
}

/// The Relative and Absolute radio buttons; the value picked this pass.
fn mode_radios(ui: &mut Ui, relative: bool) -> Option<bool> {
    let group = RadioGroup::new(tl!("Scale mode"), Bind::given(relative)).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            Radio::new(true).label(tl!("Relative")).show(ui);
            Radio::new(false).label(tl!("Absolute")).show(ui);
        });
    });
    group.output.value().copied()
}

/// The Angle row: a dial and a field in degrees; the angle set this pass.
fn rotate_row(ui: &mut Ui, angle: f64) -> Option<f64> {
    let mut edit = None;
    Field::new(tl!("Angle:")).id_salt("rotate").show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let dialed = AngleDial::new(Bind::given(angle)).label(tl!("Angle:")).range(-360.0..=360.0).show(ui).value().copied();
            let typed =
                NumberField::new(tl!("Angle:"), Bind::given(Some(angle))).unit_kind(UnitKind::Degrees).show(ui).edit.value().cloned().flatten();
            edit = typed.or(dialed);
        });
    });
    edit
}

/// The Scale, Move and Rotate rows of Transform Each and the Transform effect: sliders and fields
/// for `scaleH` and `scaleV` (rails 0–200 %; with `link` also the proportion link and the Relative
/// and Absolute choice, whose Absolute edits `absH` and `absV`), `moveH` and `moveV` (rails
/// ±1000 pt), and a dial for `rotate`. The fields take values past the rails.
pub(crate) fn rows(ui: &mut Ui, fields: &mut DialogFields, lengths: LengthUnits, link: ScaleLink) -> Response {
    let absolute = link == ScaleLink::Toggle && !relative(fields);
    let (hk, vk) = if absolute { ("absH", "absV") } else { ("scaleH", "scaleV") };
    let (measure, rail, limit) =
        if absolute { (Measure::Length(lengths), ABSOLUTE_RAIL, MAX_ABSOLUTE) } else { (Measure::Percent, SCALE_RAIL, MAX_SCALE) };
    let linked = flag(fields, LINK);
    let (mut scales, mut moves) = ([None; 2], [None; 2]);
    let (mut angle, mut toggled, mut mode) = (None, None, None);
    let read: &DialogFields = fields;
    let shown = Stack::new().show(ui, |ui, _| {
        let scale = tl!("Scale");
        Text::new(scale).show(ui);
        Row::new().show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                for (at, (key, label)) in [(hk, tl!("Horizontal:")), (vk, tl!("Vertical:"))].into_iter().enumerate() {
                    scales[at] = side(ui, key, (scale, label), field_number(read, key), measure, &rail, limit);
                }
            });
            if link == ScaleLink::Toggle {
                toggled = link_toggle(tl!("Constrain Width and Height Proportions"), Bind::given(linked)).show(ui).value().copied();
            }
        });
        if link == ScaleLink::Toggle {
            mode = mode_radios(ui, !absolute);
        }
        let shift = tl!("Move");
        Text::new(shift).show(ui);
        for (at, (key, label)) in [("moveH", tl!("Horizontal:")), ("moveV", tl!("Vertical:"))].into_iter().enumerate() {
            moves[at] = side(ui, key, (shift, label), Some(num(read, key, 0.0)), Measure::Length(lengths), &MOVE_RAIL, MAX_ABSOLUTE);
        }
        Text::new(tl!("Rotate")).show(ui);
        angle = rotate_row(ui, num(read, "rotate", 0.0));
    });
    for (key, edit) in [(hk, scales[0]), (vk, scales[1]), ("moveH", moves[0]), ("moveV", moves[1]), ("rotate", angle)] {
        if let Some(n) = edit {
            fields.set(key, n);
        }
    }
    if let Some(on) = toggled {
        fields.set(LINK, on);
    }
    if let Some(on) = mode {
        fields.set("relative", on);
    }
    shown.output
}

/// The Reflect X, Reflect Y and Random options of Transform Each and the Transform effect.
pub(crate) fn reflect_checks(ui: &mut Ui, fields: &mut DialogFields) -> Response {
    const KEYS: [&str; 3] = ["reflectX", "reflectY", "random"];
    let mut edits = [None; 3];
    let read: &DialogFields = fields;
    let shown = Stack::new().show(ui, |ui, _| {
        for (at, label) in [tl!("Reflect X"), tl!("Reflect Y"), tl!("Random")].into_iter().enumerate() {
            edits[at] = Checkbox::new(Bind::given(flag(read, KEYS[at]))).label(label).show(ui).value().copied();
        }
    });
    for (key, edit) in KEYS.into_iter().zip(edits) {
        if let Some(on) = edit {
            fields.set(key, on);
        }
    }
    shown.output
}

/// The reference point picker (field `reference`, 0..8 over the 9-point grid).
pub(crate) fn reference_point(ui: &mut Ui, fields: &mut DialogFields) -> Response {
    let at = num(fields, "reference", 4.0).clamp(0.0, 8.0) as usize;
    let anchor = Anchor9::ALL.get(at).copied().unwrap_or(Anchor9::Centre);
    let shown = ReferencePoint::new(tl!("Reference Point"), Bind::given(Some(anchor))).look(ReferencePointLook::Squares).show(ui);
    if let Some(i) = shown.value().and_then(|picked| Anchor9::ALL.iter().position(|a| Some(*a) == *picked)) {
        fields.set("reference", i as i64);
    }
    shown.response
}

/// The dialog's rows: Scale, Move and Rotate in one row, then the options.
fn dialog_params(lengths: LengthUnits) -> Vec<Param> {
    vec![
        Param::custom("scaleH", tl!("Transform"), move |ui, f, _| rows(ui, f, lengths, ScaleLink::Toggle))
            .companions(["scaleV", "absH", "absV", "relative", LINK, "moveH", "moveV", "rotate"])
            .on_set(set_from_outside)
            .full_width(),
        Param::section(tl!("Options")),
        Param::new("objects", tl!("Transform Objects"), ParamKind::bool(true)),
        Param::new("patterns", tl!("Transform Patterns"), ParamKind::bool(true)),
        Param::new("strokes", tl!("Scale Strokes & Effects"), ParamKind::bool(true)),
        Param::new("corners", tl!("Scale Corners"), ParamKind::bool(true)),
        Param::custom("reflectX", tl!("Reflect"), |ui, f, _| reflect_checks(ui, f))
            .companions(["reflectY", "random"])
            .on_set(|f, key, value| {
                f.set(key, value.as_bool().ok_or_else(|| FieldRefusal::Rejects(&value).message(key))?);
                Ok(())
            })
            .full_width(),
        Param::custom("reference", tl!("Reference Point"), |ui, f, _| reference_point(ui, f))
            .on_set(|f, key, value| {
                let at = value.as_i64().filter(|at| (0..=8).contains(at)).ok_or_else(|| FieldRefusal::Rejects(&value).message(key))?;
                f.set(key, at);
                Ok(())
            })
            .full_width(),
    ]
}

/// The dialog over `fields`, after the link and mode rules.
fn form(fields: &mut DialogFields, shared: Shared, lengths: LengthUnits) -> ParamDialog<'_> {
    apply_links(fields, shared);
    ParamDialog::new(KEY, tl!("Transform Each"), dialog_params(lengths))
        .values(Bind::bound(fields))
        .previews(true)
        .extra_buttons(vec![(COPY.to_string(), FooterButton::new(tl!("Copy"), DialogRole::Other))])
}

/// Opens Transform Each on the values it was last confirmed with.
pub(crate) fn open(app: &mut VectorcraftApp) {
    let Some(ctx) = app.ui_ctx.clone() else { return };
    let Some(document) = app.session.active().map(|doc| (doc.uid, doc.reverts)) else { return };
    let shared = shared_size(app, app.session.general_unit());
    let lengths = crate::canvas::general_lengths(app); // PLAN: assumed path of cv.B5b
    let mut f = opening_fields(app, &ctx);
    for (key, side) in SHARED.into_iter().zip([shared.0, shared.1]) {
        if let Some(side) = side {
            f.set(key, side);
        }
    }
    apply_links(&mut f, shared);
    let dialog = HostedParamDialog::new(&ctx, KEY, f, move |fields| form(fields, shared, lengths));
    let options = HeldOptions { cancel: Some(Box::new(cancel)), ..Default::default() };
    if let Some(id) = open_held_with(app, dialog, move |app, result| ok(app, &ctx, result), options) {
        app.object_dialogs.each = Some(Open { id, preview: LivePreview::default(), document, preview_tool_resets: app.session.tool_resets() });
    }
}

/// Cancel rolls the preview back.
fn cancel(app: &mut VectorcraftApp) {
    if let Some(open) = app.object_dialogs.each.take()
        && open.in_source(app)
        && app.session.tool_resets() == open.preview_tool_resets
    {
        open.preview.cancel(app);
    }
}

/// Applies the confirmed values as one undo step.
fn commit(app: &mut VectorcraftApp, f: &DialogFields) -> Result<(), String> {
    let open = app.object_dialogs.each.as_ref().ok_or("the Transform Each dialog is closed")?;
    let (in_source, reverted, (uid, _), preview_tool_resets) = (open.in_source(app), open.reverted(app), open.document, open.preview_tool_resets);
    if !in_source {
        let reason = if reverted {
            let reason = tl!("The document was reverted. Reopen this dialog to continue.");
            app.status(reason);
            reason
        } else if app.session.documents().iter().any(|doc| doc.uid == uid) {
            "switch back to the document where Transform Each was opened"
        } else {
            "the document where Transform Each was opened has closed"
        };
        return Err(reason.into());
    }
    if app.session.tool_resets() != preview_tool_resets && app.session.in_interaction() {
        return Err("finish the active edit before applying Transform Each".into());
    }
    let p = with_scale_options(app, f, command_params(app, f))?;
    let preview = app.object_dialogs.each.as_mut().map(|o| std::mem::take(&mut o.preview)).unwrap_or_default();
    preview.commit(app, CMD, p).map(|_| ())
}

/// OK: transforms each object and keeps the values for the next open. A refusal turns Copy off, or
/// the next plain OK would copy; a refused plain OK keeps its values, so the dialog shows the reason.
fn ok(app: &mut VectorcraftApp, ctx: &egui::Context, result: &ParamResult) -> Result<(), String> {
    if let Err(reason) = commit(app, &result.fields) {
        if let (Some(id), Some(dialogs)) = (app.object_dialogs.each.as_ref().map(|o| o.id), app.dialogs()) {
            dialogs.with(|host| {
                if let Some(d) = host.get_mut_by_id::<HostedParamDialog>(id) {
                    d.set_field(COPY, FieldValue::Bool(false));
                }
            });
        }
        return Err(reason);
    }
    let mut kept = DialogFields::new();
    for k in REMEMBERED {
        if let Some(v) = result.fields.get(k) {
            kept.set(k, v.clone());
        }
    }
    set_saved_value(ctx, &EACH_KEY, Some(kept));
    app.object_dialogs.each = None;
    Ok(())
}

/// Each frame: previews the dialog's values while Preview is on, and turns a press of Copy into a
/// confirm with `copy` set.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some((id, in_source)) = app.object_dialogs.each.as_ref().map(|o| (o.id, o.in_source(app))) else { return };
    let Some(dialogs) = app.dialogs() else { return };
    let Some((previewing, fields, action)) =
        dialogs.with(|host| host.get_mut_by_id::<HostedParamDialog>(id).map(|d| (d.is_previewing(), d.fields().clone(), d.take_action())))
    else {
        return;
    };
    if in_source && let Some(mut open) = app.object_dialogs.each.take() {
        let tool_resets = app.session.tool_resets();
        if open.preview_tool_resets == tool_resets || !app.session.in_interaction() {
            if open.preview_tool_resets != tool_resets {
                // A switch already ended the earlier preview.
                open.preview = LivePreview::default();
                open.preview_tool_resets = tool_resets;
            }
            let mut p = command_params(app, &fields);
            add_scale_options(app, &fields, &mut p);
            // The preview reports its own status; the answer of a run is not needed here.
            let _ = open.preview.run(app, previewing, "Transform Each", CMD, &p);
        }
        app.object_dialogs.each = Some(open);
    }
    if action.as_deref() == Some(COPY) {
        dialogs.with(|host: &mut DialogHost| {
            if let Some(d) = host.get_mut_by_id::<HostedParamDialog>(id) {
                d.set_field(COPY, FieldValue::Bool(true));
            }
            // A refused confirm leaves the dialog as it is: the held OK reports why.
            let _ = host.confirm_by_id(ctx, id);
        });
    }
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
fn shared_size(app: &VectorcraftApp, unit: vectorcraft_doc::Unit) -> Shared {
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use vectorcraft_doc::NodeId;
    use vectorcraft_engine::Session;

    type Harness = egui_kittest::Harness<'static, VectorcraftApp>;

    /// An app with a selected 50 × 50 square at (20, 20).
    fn square_app() -> (VectorcraftApp, NodeId) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("select.all", json!({})).unwrap();
        (app, NodeId(id))
    }

    /// A harness that shows the dialog host and runs the module pumps each frame.
    // PLAN: assumed `adopt_context(&Context)` and that `DialogHost::run` over the window's host stands for the shell pass.
    fn start(app: VectorcraftApp) -> Harness {
        let mut ready = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut ready, true) {
                    crate::testing::install(ui.ctx());
                    app.adopt_context(ui.ctx());
                    return;
                }
                if let Some(dialogs) = app.dialogs() {
                    dialogs.with(|host| host.run(ui));
                }
                pump(app, ui.ctx());
                crate::dialogs::host::pump(app, ui.ctx());
            },
            app,
        );
        h.run_steps(2);
        h
    }

    fn id(h: &Harness) -> DialogId {
        h.state().object_dialogs.each.as_ref().expect("Transform Each is open").id
    }

    fn set(h: &mut Harness, key: &str, value: Value) {
        let (ctx, id) = (h.ctx.clone(), id(h));
        h.state().dialogs().unwrap().with(|host| host.set_json_field(&ctx, id, key, value)).unwrap();
    }

    fn field(h: &Harness, key: &str) -> Option<FieldValue> {
        let id = id(h);
        h.state().dialogs().unwrap().with(|host| host.get_by_id::<HostedParamDialog>(id).and_then(|d| d.fields().get(key).cloned()))
    }

    fn number(h: &Harness, key: &str) -> f64 {
        match field(h, key) {
            Some(FieldValue::Number(n)) => n,
            Some(FieldValue::Int(n)) => n as f64,
            other => panic!("{key} is {other:?}"),
        }
    }

    fn confirm(h: &mut Harness) {
        let (ctx, id) = (h.ctx.clone(), id(h));
        assert!(h.state().dialogs().unwrap().with(|host| host.confirm_by_id(&ctx, id)) || h.state().object_dialogs.each.is_some());
        h.run_steps(3);
    }

    fn open_with_two_documents() -> (Harness, NodeId) {
        let (mut app, first) = square_app();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 70, "y": 50, "width": 30, "height": 65})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("document.activate", json!({"index": 0})).unwrap();
        let mut h = start(app);
        open(h.state_mut());
        h.run_steps(2);
        (h, first)
    }

    #[test]
    fn preview_and_ok_stay_on_the_opening_document() {
        let (mut h, first) = open_with_two_documents();
        set(&mut h, "moveH", json!(45));
        h.run_steps(2);
        assert!(h.state().session.in_interaction(), "the opening document has a preview");

        h.state_mut().run("document.activate", json!({"index": 1})).unwrap();
        let other = h.state().session.doc().unwrap().doc.clone();
        set(&mut h, "moveH", json!(100));
        h.run_steps(2);
        assert_eq!(h.state().session.doc().unwrap().doc, other, "Preview must leave the other document unchanged");
        assert!(!h.state().session.in_interaction());
        confirm(&mut h);
        assert_eq!(h.state().session.doc().unwrap().doc, other, "OK must leave the other document unchanged");
        assert!(h.state().object_dialogs.each.is_some(), "the draft is still open");

        h.state_mut().run("document.activate", json!({"index": 0})).unwrap();
        confirm(&mut h);
        let doc = &h.state().session.doc().unwrap().doc;
        assert!((doc.node(first).unwrap().geometric_bounds().unwrap().x0 - 120.0).abs() < 1e-6);
    }

    #[test]
    fn copy_stays_on_the_opening_document() {
        let (mut h, _) = open_with_two_documents();
        set(&mut h, "preview", json!(false));
        set(&mut h, "moveH", json!(100));
        h.run_steps(2);
        h.state_mut().run("document.activate", json!({"index": 1})).unwrap();
        let other = h.state().session.doc().unwrap().doc.clone();
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Copy").click();
        h.run_steps(4);
        assert_eq!(h.state().session.doc().unwrap().doc, other, "Copy must leave the other document unchanged");
        assert_ne!(field(&h, COPY), Some(FieldValue::Bool(true)), "a refused Copy clears its flag");

        h.state_mut().run("document.activate", json!({"index": 0})).unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Copy").click();
        h.run_steps(4);
        assert_eq!(h.state().session.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
    }

    #[test]
    fn cancel_keeps_the_other_documents_gesture_active() {
        let (mut h, _) = open_with_two_documents();
        set(&mut h, "moveH", json!(45));
        h.run_steps(2);
        assert!(h.state().session.in_interaction());
        h.state_mut().run("document.activate", json!({"index": 1})).unwrap();
        let app = h.state_mut();
        app.session.begin_interaction("Another gesture").unwrap();
        app.session.preview("object.move", &json!({"dx": 20, "dy": 0})).unwrap();
        let other = app.session.doc().unwrap().doc.clone();

        let (ctx, id) = (h.ctx.clone(), id(&h));
        assert!(h.state().dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id)));
        h.run_steps(3);
        assert_eq!(h.state().session.doc().unwrap().doc, other, "Cancel preserves the other document's gesture");
        assert!(h.state().session.in_interaction());
    }

    #[test]
    fn switching_back_preserves_a_gesture_that_started_after_the_preview() {
        let (mut h, _) = open_with_two_documents();
        set(&mut h, "moveH", json!(45));
        h.run_steps(2);
        assert!(h.state().session.in_interaction());
        let app = h.state_mut();
        app.run("document.activate", json!({"index": 1})).unwrap();
        app.run("document.activate", json!({"index": 0})).unwrap();
        app.session.begin_interaction("Another gesture").unwrap();
        app.session.preview("object.move", &json!({"dx": 20, "dy": 0})).unwrap();
        let gesture = app.session.doc().unwrap().doc.clone();

        set(&mut h, "moveH", json!(100));
        h.run_steps(2);
        confirm(&mut h);

        assert_eq!(h.state().session.doc().unwrap().doc, gesture, "Preview and OK leave the newer gesture alone");
        assert!(h.state().session.in_interaction());
        let (ctx, id) = (h.ctx.clone(), id(&h));
        assert!(h.state().dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id)));
        h.run_steps(2);
        assert_eq!(h.state().session.doc().unwrap().doc, gesture, "Cancel also leaves the newer gesture alone");
        assert!(h.state().session.in_interaction());
    }

    /// A harness on a saved copy of [`square_app`], with Transform Each open on it. The directory
    /// holds the saved file.
    fn open_on_a_saved_document(tag: &str) -> (Harness, NodeId, vectorcraft_testkit::TempDir) {
        let (mut app, id) = square_app();
        let dir = vectorcraft_testkit::TempDir::new(tag);
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        let mut h = start(app);
        open(h.state_mut());
        h.run_steps(2);
        (h, id, dir)
    }

    /// Reverts the document to its saved copy and selects the restored square again.
    fn revert_and_reselect(h: &mut Harness) {
        h.state_mut().run("file.revert", json!({"confirmed": true})).unwrap();
        h.state_mut().run("select.all", json!({})).unwrap();
        h.run_steps(2);
    }

    const REVERTED: &str = "The document was reverted. Reopen this dialog to continue.";

    /// Asserts that the held OK was refused because the document was reverted, under the rows and
    /// on the status line.
    fn assert_refused_for_the_revert(h: &Harness) {
        assert_eq!(h.state().ui.status, REVERTED, "the status line reports the revert");
        let id = id(h);
        let held = h.state().dialogs().unwrap().with(|host| host.held_state(id));
        assert!(
            matches!(&held, Some(craft::ui::dialog::HeldOk::Refused(reason)) if reason.as_str() == REVERTED),
            "OK is refused for the revert, the held state is {held:?}"
        );
    }

    /// After File > Revert the draft neither previews on nor commits to the restored square, and OK
    /// is refused; reopened, the dialog applies again.
    #[test]
    fn transform_each_refuses_its_draft_after_file_revert() {
        let (mut h, square, _dir) = open_on_a_saved_document("transform-each-revert");
        set(&mut h, "moveH", json!(45));
        h.run_steps(2);
        assert!(h.state().session.in_interaction(), "the draft previews before the revert");

        revert_and_reselect(&mut h);
        let restored = h.state().session.doc().unwrap().doc.clone();
        let undo = h.state().session.doc().unwrap().history.undo.len();
        set(&mut h, "moveH", json!(60));
        h.run_steps(2);
        assert_eq!(h.state().session.doc().unwrap().doc, restored, "Preview must leave the restored square unchanged");
        confirm(&mut h);
        assert_eq!(h.state().session.doc().unwrap().doc, restored, "OK must leave the restored square unchanged");
        assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo, "OK must record no undo step");
        assert!(h.state().object_dialogs.each.is_some(), "the dialog stays open with the refusal");
        assert_refused_for_the_revert(&h);

        let (ctx, dialog) = (h.ctx.clone(), id(&h));
        assert!(h.state().dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, dialog)));
        h.run_steps(3);
        assert_eq!(h.state().session.doc().unwrap().doc, restored, "Cancel must leave the restored square unchanged");
        open(h.state_mut());
        h.run_steps(2);
        set(&mut h, "moveH", json!(30));
        confirm(&mut h);
        assert!(h.state().object_dialogs.each.is_none(), "the reopened dialog applies");
        let x0 = h.state().session.doc().unwrap().doc.node(square).unwrap().geometric_bounds().unwrap().x0;
        assert!((x0 - 50.0).abs() < 1e-6, "the reopened dialog moves the restored square, x0 is {x0}");
        assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo + 1);
    }

    /// After File > Revert, Copy does not copy the restored square either.
    #[test]
    fn copy_refuses_its_draft_after_file_revert() {
        let (mut h, _, _dir) = open_on_a_saved_document("transform-each-copy-revert");
        set(&mut h, "preview", json!(false));
        set(&mut h, "moveH", json!(100));
        h.run_steps(2);
        // With Preview off the document has no unsaved change to revert yet.
        h.state_mut().run("object.move", json!({"dx": 5, "dy": 0})).unwrap();

        revert_and_reselect(&mut h);
        let restored = h.state().session.doc().unwrap().doc.clone();
        let undo = h.state().session.doc().unwrap().history.undo.len();
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Copy").click();
        h.run_steps(4);
        assert_eq!(h.state().session.doc().unwrap().doc, restored, "Copy must leave the restored document unchanged");
        assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo, "Copy must record no undo step");
        assert_ne!(field(&h, COPY), Some(FieldValue::Bool(true)), "a refused Copy clears its flag");
        assert_eq!(h.state().ui.status, REVERTED, "the status line reports the revert");
        assert!(h.state().object_dialogs.each.is_some(), "the dialog stays open");
    }

    #[test]
    fn ok_transforms_each_and_keeps_the_scale_options() {
        let (mut app, node) = square_app();
        app.run("prefs.set", json!({"key": "scaleStrokes", "value": true})).unwrap();
        let mut h = start(app);
        open(h.state_mut());
        h.run_steps(2);
        assert_eq!(field(&h, "strokes"), Some(FieldValue::Bool(true)), "the checkboxes start from the preferences");
        set(&mut h, "scaleH", json!(200));
        set(&mut h, "scaleV", json!(200));
        set(&mut h, "strokes", json!(false));
        confirm(&mut h);
        let app = h.state_mut();
        assert!(app.object_dialogs.each.is_none());
        assert!(!app.session.prefs.scale_strokes, "OK keeps the option as the preference");
        let n = app.session.doc().unwrap().doc.node(node).unwrap().clone();
        assert!((n.geometric_bounds().unwrap().width() - 100.0).abs() < 1e-6);
        assert_eq!(n.appearance.stroke_width(), 1.0, "strokes kept their weight");
        let (cmd, p) = app.session.journal.last().unwrap();
        assert_eq!((cmd.as_str(), &p["strokes"], &p["objects"]), (CMD, &json!(false), &json!(true)), "#1448: Transform Objects goes along");
    }

    /// A dialog that opens with the link on keeps the proportions of the scales it opens on, so a
    /// scale set before the first frame (`ui.dialog.set`) follows it.
    #[test]
    fn a_link_that_opens_on_keeps_the_proportions_it_opens_on() {
        let (app, _) = square_app();
        let mut h = start(app);
        let kept =
            DialogFields::from_pairs([("scaleH", FieldValue::Number(100.0)), ("scaleV", FieldValue::Number(200.0)), (LINK, FieldValue::Bool(true))]);
        set_saved_value(&h.ctx, &EACH_KEY, Some(kept));
        open(h.state_mut());
        set(&mut h, "scaleH", json!(50));
        h.run_steps(2);
        assert_eq!(number(&h, "scaleV"), 100.0, "the other scale followed at the proportions it opened on");
    }

    /// #1732: the dialog opens on the values it was last confirmed with (as Move and Offset Path
    /// do), but never on Copy.
    #[test]
    fn it_reopens_on_the_values_confirmed_last() {
        let (app, _) = square_app();
        let mut h = start(app);
        open(h.state_mut());
        h.run_steps(2);
        for (k, v) in [
            ("scaleH", json!(150)),
            ("moveH", json!(20)),
            ("rotate", json!(15)),
            ("reflectX", json!(true)),
            ("random", json!(true)),
            ("reference", json!(0)),
            ("objects", json!(false)),
        ] {
            set(&mut h, k, v);
        }
        // Copy is a footer button, not a row: the fields hold it as the app's Copy action sets it.
        let dialog = id(&h);
        h.state().dialogs().unwrap().with(|host| host.get_mut_by_id::<HostedParamDialog>(dialog).map(|d| d.set_field(COPY, FieldValue::Bool(true))));
        confirm(&mut h);
        assert!(h.state().object_dialogs.each.is_none());
        open(h.state_mut());
        h.run_steps(2);
        for (k, v) in [("scaleH", 150.0), ("scaleV", 100.0), ("moveH", 20.0), ("rotate", 15.0), ("reference", 0.0)] {
            assert_eq!(number(&h, k), v, "{k}");
        }
        for (k, v) in [("reflectX", true), ("reflectY", false), ("random", true), ("objects", false), (COPY, false)] {
            assert_eq!(field(&h, k), Some(FieldValue::Bool(v)), "{k}");
        }
        // Cancel keeps nothing new.
        set(&mut h, "scaleH", json!(300));
        let (ctx, id) = (h.ctx.clone(), id(&h));
        h.state().dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id));
        h.run_steps(3);
        open(h.state_mut());
        h.run_steps(2);
        assert_eq!(number(&h, "scaleH"), 150.0);
    }

    /// #1732: the Copy button transforms a copy of each object, in one undo step, and Transform
    /// Again makes a further copy; there is no Copy checkbox.
    #[test]
    fn the_copy_button_transforms_a_copy_of_each_object() {
        let (app, id) = square_app();
        let undo = app.session.doc().unwrap().history.undo.len();
        let mut h = start(app);
        open(h.state_mut());
        h.run_steps(2);
        set(&mut h, "moveH", json!(100));
        h.run_steps(2);
        assert!(h.query_by_role_and_label(egui::accesskit::Role::CheckBox, "Copy").is_none(), "no Copy checkbox");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Copy").click();
        h.run_steps(4);
        let app = h.state_mut();
        assert!(app.object_dialogs.each.is_none(), "Copy closes the dialog");
        let xs = |app: &VectorcraftApp| {
            let st = app.session.doc().unwrap();
            let mut xs: Vec<f64> = st.doc.layers[0].children().unwrap().iter().map(|n| n.geometric_bounds().unwrap().x0.round()).collect();
            xs.sort_by(f64::total_cmp);
            xs
        };
        assert_eq!(xs(app), vec![20.0, 120.0], "the square stays, its copy moved 100 pt");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        assert!(app.session.doc().unwrap().doc.node(id).is_some());
        app.run("object.transformAgain", json!({})).unwrap();
        assert_eq!(xs(app), vec![20.0, 120.0, 220.0], "Transform Again copies once more");
        open(h.state_mut());
        h.run_steps(2);
        assert_eq!(field(&h, COPY), Some(FieldValue::Bool(false)), "Copy isn't remembered");
        h.ctx.forget_all_images();
    }
}
