//! Path dialogs: Average, Average & Join, and Offset Path, Simplify and Split Into Grid with Preview.
//!
//! Every dialog is a held parameter dialog: its OK is the closure given to `open_held`, a refusal keeps it open with
//! the reason, and [`pump`] only runs the live preview of the kinds that have one.

use std::sync::{Arc, Weak};

// PLAN: the framework import paths below are assumed from the contract rows (craft-ui-patterns re-exports contract and
// runtime; the form-dialog items live under `patterns::form_dialogs`; `Field` and `NumberField` under craft-ui-widgets).
use craft::behavior::ColumnWidth;
use craft::ui::contract::{Bind, FieldLabelRole, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout::GridFit;
use craft::ui::layout_components::Grid;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use craft::ui::runtime::measure;
use craft::ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use craft::ui::text::{NumberField, NumberOutput};
use craft::ui::value_drag::SliderScale;
use serde_json::{Map, Value, json};
use vectorcraft_doc::{NodeId, NodeKind, Selection};
use vectorcraft_geom::PathData;

use super::form;
// PLAN: assumed `LivePreview` is exported by `dialogs::form` (wg.A20, widgets-w11).
use super::host::{HeldOptions, open_held, open_held_with};
use super::preview::LivePreview;
use crate::VectorcraftApp;

/// Simplify's Corner Point Angle Threshold when the dialog opens, in degrees. An anchor whose
/// corner is sharper than the threshold stays a corner, so a higher threshold keeps more corners.
/// The command's `cornerAngle` is the turn above which an anchor stays a corner, so the dialog
/// sends 180° less the threshold: 150° sends `object.path.simplify`'s default of 30°.
pub(crate) const ANGLE_THRESHOLD: f64 = 150.0;

/// Split Into Grid's two columns, each as wide as its fields: the rows on the left, the columns on the right. The
/// grid keeps both columns in a narrow dialog.
const AXES: [ColumnWidth; 2] = [ColumnWidth::Content; 2];

/// Simplify's tolerance when the dialog opens from the menu, in points.
const DEFAULT_TOLERANCE: f64 = 1.0;

// PLAN: the save-key prefix is assumed.
/// The Simplify settings kept by "Open this dialog with these settings next time" (`None` when unticked).
pub(crate) const SIMPLIFY_KEY: SaveKey = SaveKey::new("vectorcraft_dialogs.simplify");
/// The Offset Path values confirmed last.
pub(crate) const OFFSET_KEY: SaveKey = SaveKey::new("vectorcraft_dialogs.offset_path");
/// The Average & Join anchor confirmed last.
pub(crate) const JOIN_KEY: SaveKey = SaveKey::new("vectorcraft_dialogs.average_join");

/// The key of Split Into Grid's command and dialog.
const SPLIT_COMMAND: &str = "object.path.splitIntoGrid";
/// The private field that holds Simplify's anchor counts, `{before, after}`.
const COUNTS: &str = "__counts";

/// The path dialogs that share [`open`]; Split Into Grid has [`open_split`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PathOp {
    /// Path › Average.
    Average,
    /// Path › Average & Join.
    AverageJoin,
    /// Object › Path › Offset Path.
    OffsetPath,
    /// Object › Path › Simplify.
    Simplify,
}

impl PathOp {
    fn title(self) -> &'static str {
        match self {
            PathOp::Average => tl!("Average"),
            PathOp::AverageJoin => tl!("Average & Join"),
            PathOp::OffsetPath => tl!("Offset Path"),
            PathOp::Simplify => tl!("Simplify"),
        }
    }

    /// The dialog's key: the command it runs.
    fn key(self) -> &'static str {
        match self {
            PathOp::Average => "path.average",
            PathOp::AverageJoin => "path.averageJoin",
            PathOp::OffsetPath => "object.path.offsetPath",
            PathOp::Simplify => "object.path.simplify",
        }
    }

    /// Whether the dialog previews on the canvas.
    fn previews(self) -> bool {
        matches!(self, PathOp::OffsetPath | PathOp::Simplify)
    }

    /// The fields the dialog opens with, before remembered values.
    fn opening(self) -> Map<String, Value> {
        let v = match self {
            PathOp::Average => json!({"axis": "both"}),
            PathOp::AverageJoin => json!({"to": "corner"}),
            PathOp::OffsetPath => json!({"offset": 10.0, "joins": "miter", "miterLimit": 4.0}),
            PathOp::Simplify => json!({}),
        };
        match v {
            Value::Object(m) => m,
            _ => Map::new(),
        }
    }
}

/// The open Average-family, Offset Path or Simplify dialog's live state.
pub(crate) struct Open {
    op: PathOp,
    id: DialogId,
    document: u64,
    preview: LivePreview,
    /// A distinct selection snapshot identifies the interaction that ran this preview.
    preview_selection: Option<Weak<Selection>>,
    /// The params the preview last answered for.
    last: Value,
    /// Simplify: the paths the preview ran on.
    original: Vec<NodeId>,
    /// Simplify: whether the canvas outlines `original` (Preview and Show Original Path are on).
    shown: bool,
}

/// The open Split Into Grid dialog's live state.
pub(crate) struct SplitOpen {
    id: DialogId,
    document: u64,
    preview: LivePreview,
    /// An interaction allocates its own selection snapshot, even when the selected objects did not change.
    preview_selection: Option<Weak<Selection>>,
}

impl SplitOpen {
    fn owns_preview(&self, app: &VectorcraftApp) -> bool {
        let Some(selection) = &self.preview_selection else { return false };
        app.session.active().is_some_and(|state| {
            state.uid == self.document
                && state.interaction.as_ref().is_some_and(|interaction| selection.as_ptr() == Arc::as_ptr(&interaction.selection))
        })
    }

    fn update_preview(&mut self, app: &mut VectorcraftApp, on: bool, fields: &DialogFields) {
        let Some(state) = app.session.active().filter(|state| state.uid == self.document) else { return };
        if !self.owns_preview(app) {
            self.preview = LivePreview::default();
            self.preview_selection = None;
            if state.interaction.is_some() {
                return;
            }
        }
        let params = split_params(fields);
        let ran = self.preview.run(app, on, tl!("Split Into Grid"), SPLIT_COMMAND, &params).is_some();
        if !on {
            self.preview_selection = None;
        } else if ran {
            self.preview_selection = app.session.active().and_then(|state| state.interaction.as_ref()).map(|it| Arc::downgrade(&it.selection));
        }
    }

    fn confirm(&mut self, app: &mut VectorcraftApp, fields: &DialogFields) -> Result<(), String> {
        if app.session.active().is_none_or(|state| state.uid != self.document) {
            return Err(tl!("Split Into Grid can only change the document where it was opened.").to_owned());
        }
        if app.session.in_interaction() && !self.owns_preview(app) {
            return Err(tl!("Finish the active edit before applying Split Into Grid.").to_owned());
        }
        self.preview_selection = None;
        std::mem::take(&mut self.preview).commit(app, SPLIT_COMMAND, split_params(fields)).map(drop)
    }

    fn cancel(self, app: &mut VectorcraftApp) {
        if self.owns_preview(app) {
            self.preview.cancel(app);
        }
    }
}

/// The command the dialog runs and its parameters.
fn command(op: PathOp, f: &DialogFields) -> (&'static str, Value) {
    let params = match op {
        PathOp::Average => json!({"axis": f.text("axis").unwrap_or("both")}),
        PathOp::AverageJoin => json!({"to": f.text("to").unwrap_or("corner")}),
        PathOp::OffsetPath => json!({
            "offset": f.number("offset").unwrap_or(10.0),
            "joins": f.text("joins").unwrap_or("miter"),
            "miterLimit": f.number("miterLimit").unwrap_or(4.0),
        }),
        PathOp::Simplify => {
            let turn = 180.0 - f.number("angleThreshold").unwrap_or(ANGLE_THRESHOLD).clamp(0.0, 180.0);
            json!({
                "tolerance": f.number("tolerance").unwrap_or(DEFAULT_TOLERANCE),
                "cornerAngle": turn,
                "straightLines": f.bool("straightLines").unwrap_or(false),
            })
        }
    };
    (op.key(), params)
}

/// Split Into Grid's command parameters.
fn split_params(f: &DialogFields) -> Value {
    let num = |k: &str, d: f64| f.number(k).unwrap_or(d);
    json!({
        "rows": num("rows", 2.0),
        "columns": num("columns", 2.0),
        "rowGutter": num("rowGutter", 0.0),
        "columnGutter": num("columnGutter", 0.0),
        "totalHeight": num("totalHeight", 0.0),
        "totalWidth": num("totalWidth", 0.0),
        "addGuides": f.bool("addGuides").unwrap_or(false),
    })
}

/// The fields Split Into Grid opens with for a selection `width` × `height` points.
pub(crate) fn split_fields(width: f64, height: f64) -> DialogFields {
    let mut f = DialogFields::from_pairs([
        ("rows", 2.0),
        ("columns", 2.0),
        ("rowGutter", 12.0),
        ("columnGutter", 12.0),
        ("totalHeight", height),
        ("totalWidth", width),
    ]);
    f.set("addGuides", false);
    f
}

/// One cell of Split Into Grid's grid: a labelled number field. A count (`lengths` is `None`) is a whole number;
/// the others are lengths. A cell whose visible label repeats another's carries its axis in its name, so the visible
/// label only describes it. Returns the value typed this pass.
fn cell(ui: &mut egui::Ui, label: &'static str, name: String, describes: bool, value: f64, lengths: Option<LengthUnits>) -> Option<f64> {
    let role = if describes { FieldLabelRole::Describes } else { FieldLabelRole::Names };
    let mut edited = None;
    Field::new(label).side(SideLabel::new().role(role)).id_salt(name.as_str()).show(ui, |ui, _| {
        let bind = Bind::given(Some(value));
        let out: NumberOutput = match lengths {
            Some(l) => NumberField::length(name.clone(), bind, l).show(ui),
            None => NumberField::new(name.clone(), bind).range(1.0..=500.0).commit_decimals(0).show(ui),
        };
        edited = out.value().copied().flatten();
    });
    edited
}

/// Split Into Grid's fields: the rows on the left and the columns on the right, each with its number, size, gutter
/// and total. The size of a row or column is what the total leaves each of them after the gutters, so it follows the
/// other three; typing it sets the total.
fn split_grid(ui: &mut egui::Ui, f: &mut DialogFields, lengths: LengthUnits) -> egui::Response {
    let axes = [
        ("rows", tl!("Rows:"), tl!("Height:"), "rowGutter", "totalHeight", 2.0),
        ("columns", tl!("Columns:"), tl!("Width:"), "columnGutter", "totalWidth", 2.0),
    ];
    // The grid can run its body twice in a pass, so the body edits a copy, kept only from the run that takes effect.
    let before = f.clone();
    let mut edited = None;
    let shown = Grid::new(&AXES).fit(GridFit::Fixed).id_salt("split-grid").show(ui, |ui, _| {
        let mut f = before.clone();
        for (number, label, _, _, _, default) in axes {
            let n = f.number(number).unwrap_or(default);
            if let Some(v) = cell(ui, label, label.to_string(), false, n, None) {
                f.set(number, v);
            }
        }
        for (number, _, label, gutter, total, default) in axes {
            let n = f.number(number).unwrap_or(default).max(1.0);
            let gaps = f.number(gutter).unwrap_or(0.0) * (n - 1.0);
            let size = (f.number(total).unwrap_or(0.0) - gaps) / n;
            if let Some(v) = cell(ui, label, label.to_string(), false, size, Some(lengths)) {
                f.set(total, n * v + gaps);
            }
        }
        for (_, axis, _, gutter, _, _) in axes {
            let label = tl!("Gutter:");
            if let Some(v) = cell(ui, label, format!("{axis} {label}"), true, f.number(gutter).unwrap_or(0.0), Some(lengths)) {
                f.set(gutter, v);
            }
        }
        for (_, axis, _, _, total, _) in axes {
            let label = tl!("Total:");
            if let Some(v) = cell(ui, label, format!("{axis} {label}"), true, f.number(total).unwrap_or(0.0), Some(lengths)) {
                f.set(total, v);
            }
        }
        if measure::effect(ui.ctx(), "Split Into Grid's fields edit their values") {
            edited = Some(f);
        }
    });
    if let Some(edited) = edited {
        *f = edited;
    }
    shown.output
}

/// Split Into Grid's rows: the grid and Add Guides.
fn split_rows(lengths: LengthUnits) -> Vec<Param> {
    vec![
        // PLAN: `companions` is assumed to take the keys of the values a custom row owns.
        Param::custom("grid", tl!("Grid"), move |ui, f, _| split_grid(ui, f, lengths))
            .companions(["rows", "columns", "rowGutter", "columnGutter", "totalHeight", "totalWidth"])
            .full_width(),
        Param::new("addGuides", tl!("Add Guides"), ParamKind::bool(false)),
    ]
}

/// Simplify's rows: Tolerance, Corner Point Angle Threshold (0°, the smoothest, to 180°, the sharpest), Convert to
/// Straight Lines, Show Original Path and Open this dialog with these settings next time.
fn simplify_rows(lengths: LengthUnits, tolerance: f64) -> Vec<Param> {
    let check = |key: &str, label: &'static str| Param::new(key, label, ParamKind::bool(false));
    vec![
        Param::number("tolerance", tl!("Tolerance:"), tolerance, 0.1, 3).bounds(Some(0.0), None).length(lengths),
        Param::new("angleThreshold", tl!("Corner Point Angle Threshold:"), ParamKind::range(0.0, 180.0, ANGLE_THRESHOLD, SliderScale::Linear))
            .unit(UnitKind::Degrees),
        check("straightLines", tl!("Convert to Straight Lines")),
        check("showOriginal", tl!("Show Original Path")),
        check("retain", tl!("Open this dialog with these settings next time")),
    ]
}

/// Average & Join's Points: Corner or Smooth, the joined anchor. "Points:" in the "anchor" context: the plain row is
/// a star's number of points.
fn join_rows() -> Vec<Param> {
    let options = vec![("corner".to_string(), tl!("Corner").into()), ("smooth".to_string(), tl!("Smooth").into())];
    vec![Param::new("to", crate::i18n::tr_ctx(crate::i18n::current(), "anchor", "Points:"), ParamKind::radio(options, Some("corner".into())))]
}

fn rows(op: PathOp, opening: &Map<String, Value>, lengths: LengthUnits) -> Vec<Param> {
    match op {
        PathOp::Average => form::grid_params("average", opening, lengths),
        PathOp::AverageJoin => join_rows(),
        PathOp::OffsetPath => form::grid_params("offsetPath", opening, lengths),
        PathOp::Simplify => simplify_rows(lengths, DEFAULT_TOLERANCE),
    }
}

/// The dialog of Simplify's fields at `tolerance` (points): the slider's tolerance when its More Options opens the
/// dialog.
pub(crate) fn simplify_dialog(tolerance: f64) -> DialogFields {
    let mut f = DialogFields::from_pairs([("tolerance", tolerance), ("angleThreshold", ANGLE_THRESHOLD)]);
    for key in ["straightLines", "showOriginal", "retain"] {
        f.set(key, false);
    }
    f
}

/// The anchor counts line under Simplify's rows.
fn counts_row(ui: &mut egui::Ui, f: &DialogFields) {
    if let Some(FieldValue::Json(c)) = f.get(COUNTS)
        && let (Some(before), Some(after)) = (c["before"].as_u64(), c["after"].as_u64())
    {
        Text::new(format!("{} {before} → {after}", crate::i18n::colon(tl!("Anchor Points")))).show(ui);
    }
}

/// The record of the open dialog of `op`, taken out of the app.
fn take_open(app: &mut VectorcraftApp, op: PathOp) -> Option<Open> {
    let i = app.object_dialogs.path.iter().position(|r| r.op == op)?;
    Some(app.object_dialogs.path.remove(i))
}

impl Open {
    fn owns_preview(&self, app: &VectorcraftApp) -> bool {
        let Some(selection) = &self.preview_selection else { return false };
        app.session.active().is_some_and(|state| {
            state.uid == self.document
                && state.interaction.as_ref().is_some_and(|interaction| selection.as_ptr() == Arc::as_ptr(&interaction.selection))
        })
    }

    fn cancel(self, app: &mut VectorcraftApp) {
        if self.owns_preview(app) {
            self.preview.cancel(app);
        }
    }

    fn commit(&mut self, app: &mut VectorcraftApp, cmd: &str, params: Value) -> Result<(), String> {
        if app.session.active().is_none_or(|state| state.uid != self.document) {
            return Err(tl!("Return to the document where this dialog opened.").to_owned());
        }
        if app.session.in_interaction() && !self.owns_preview(app) {
            return Err(tl!("Finish the active edit before confirming.").to_owned());
        }
        self.preview_selection = None;
        self.last = Value::Null;
        self.original.clear();
        self.shown = false;
        std::mem::take(&mut self.preview).commit(app, cmd, params).map(drop)
    }
}

/// Whether Object › Path › Simplify opens its dialog (the user kept its settings) instead of the slider.
pub(crate) fn retained_simplify(app: &VectorcraftApp) -> bool {
    let Some(ctx) = app.ui_ctx.as_ref() else { return false };
    saved_value::<Option<DialogFields>>(ctx, &SIMPLIFY_KEY).is_some()
}

/// Opens the dialog of `op` on its remembered values (Simplify: the ones kept for next time).
pub(crate) fn open(app: &mut VectorcraftApp, op: PathOp) {
    let opening = op.opening();
    let fields = if op == PathOp::Simplify {
        simplify_dialog(DEFAULT_TOLERANCE)
    } else {
        let lengths = crate::canvas::general_lengths(app);
        form::fields_of(&rows(op, &opening, lengths), &opening)
    };
    let remember = match op {
        PathOp::Average => None,
        PathOp::AverageJoin => Some(JOIN_KEY),
        PathOp::OffsetPath => Some(OFFSET_KEY),
        PathOp::Simplify => Some(SIMPLIFY_KEY),
    };
    open_dialog(app, op, fields, remember);
}

/// Simplify's More Options: the dialog on its defaults at the slider's `tolerance`. It does not lay the kept values
/// over the opening, which would replace the slider's tolerance.
pub(crate) fn open_simplify(app: &mut VectorcraftApp, tolerance: f64) {
    open_dialog(app, PathOp::Simplify, simplify_dialog(tolerance), None);
}

fn open_dialog(app: &mut VectorcraftApp, op: PathOp, fields: DialogFields, remember: Option<SaveKey>) {
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status("There is no window to show the dialog in.");
        return;
    };
    let drawing = app.session.active().map(|state| (state.uid, state.reverts));
    let document = drawing.map(|(uid, _)| uid);
    if op.previews() && document.is_none() {
        app.status("Open a document before editing paths.");
        return;
    }
    let lengths = crate::canvas::general_lengths(app);
    let opening = op.opening();
    let key = op.key();
    let dialog = HostedParamDialog::new(&ctx, key, fields, move |f| {
        let mut d = ParamDialog::new(key, op.title(), rows(op, &opening, lengths)).values(Bind::bound(f));
        if let Some(k) = remember.clone() {
            d = d.remember(k);
        }
        if op.previews() {
            d = d.previews(true);
        }
        if op == PathOp::Simplify { d.below(counts_row) } else { d }
    });
    let ok = move |app: &mut VectorcraftApp, result: &craft::ui::patterns::form_dialogs::ParamResult| {
        let (cmd, params) = command(op, &result.fields);
        if op.previews() {
            let Some(mut record) = take_open(app, op) else { return Err(tl!("The path dialog is closed.").to_owned()) };
            record.commit(app, cmd, params).inspect_err(|_| app.object_dialogs.path.push(record))?;
        } else {
            if app.session.active().map(|state| (state.uid, state.reverts)) != drawing {
                return Err(tl!("Return to the opening drawing, or reopen this dialog after File Revert.").to_owned());
            }
            app.run(cmd, params).map(drop)?;
        }
        match op {
            PathOp::Simplify => {
                let kept =
                    result.fields.bool("retain").unwrap_or(false).then(|| ParamDialog::remembered(&rows(op, &Map::new(), lengths), &result.fields));
                set_saved_value(&ctx, &SIMPLIFY_KEY, kept);
            }
            PathOp::AverageJoin | PathOp::OffsetPath => result.remember(&ctx),
            PathOp::Average => {}
        }
        Ok(())
    };
    if let Some(old) = take_open(app, op) {
        old.cancel(app);
    }
    let id = if op.previews() {
        let cancel = Box::new(move |app: &mut VectorcraftApp| {
            // A cancelled opening can leave a replacement with the same dialog ID.
            let id = app.object_dialogs.path.iter().find(|record| record.op == op).map(|record| record.id);
            if let Some(id) = id
                && app.dialogs().is_some_and(|dialogs| dialogs.with(|host| host.get_by_id::<HostedParamDialog>(id).is_some()))
            {
                return;
            }
            if let Some(record) = take_open(app, op) {
                record.cancel(app);
            }
        });
        open_held_with(app, dialog, ok, HeldOptions { parent: None, cancel: Some(cancel) })
    } else {
        open_held(app, dialog, ok)
    };
    match (id, document) {
        (Some(id), Some(document)) if op.previews() => {
            app.object_dialogs.path.push(Open {
                op,
                id,
                document,
                preview: LivePreview::default(),
                preview_selection: None,
                last: Value::Null,
                original: vec![],
                shown: false,
            });
        }
        (Some(_), _) => {}
        (None, _) => app.status("There is no window to show the dialog in."),
    }
}

/// Opens Split Into Grid on a grid the size of the selection.
pub(crate) fn open_split(app: &mut VectorcraftApp) {
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status("There is no window to show the dialog in.");
        return;
    };
    let Some(state) = app.session.active() else {
        app.status("Open a document before using Split Into Grid.");
        return;
    };
    let document = state.uid;
    let (width, height) = state.doc.bounds_of(&state.selection.objects, false).map_or((0.0, 0.0), |b| (b.width(), b.height()));
    let lengths = crate::canvas::general_lengths(app);
    let dialog = HostedParamDialog::new(&ctx, SPLIT_COMMAND, split_fields(width, height), move |f| {
        ParamDialog::new(SPLIT_COMMAND, tl!("Split Into Grid"), split_rows(lengths)).values(Bind::bound(f)).previews(true)
    });
    let ok = |app: &mut VectorcraftApp, result: &craft::ui::patterns::form_dialogs::ParamResult| {
        let Some(mut record) = app.object_dialogs.split.take() else { return Err(tl!("The Split Into Grid dialog is closed.").to_owned()) };
        record.confirm(app, &result.fields).inspect_err(|_| app.object_dialogs.split = Some(record))
    };
    if let Some(old) = app.object_dialogs.split.take() {
        old.cancel(app);
    }
    match open_held(app, dialog, ok) {
        Some(id) => app.object_dialogs.split = Some(SplitOpen { id, document, preview: LivePreview::default(), preview_selection: None }),
        None => app.status("There is no window to show the dialog in."),
    }
}

/// Whether the dialog `id` previews, and its values.
fn dialog_state(app: &VectorcraftApp, id: DialogId) -> Option<(bool, DialogFields)> {
    app.dialogs()?.with(|h| h.get_by_id::<HostedParamDialog>(id).map(|d| (d.is_previewing(), d.fields().clone())))
}

/// Runs the previews of the open dialogs. Each pass applies only the dialog's own values, so a second pass in one
/// frame finds the same params and runs nothing.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let mut records = std::mem::take(&mut app.object_dialogs.path);
    for r in &mut records {
        step(app, r);
    }
    records.append(&mut app.object_dialogs.path);
    app.object_dialogs.path = records;
    if let Some(mut split) = app.object_dialogs.split.take() {
        if let Some((on, fields)) = dialog_state(app, split.id) {
            split.update_preview(app, on, &fields);
            app.object_dialogs.split = Some(split);
        } else {
            split.cancel(app);
        }
    }
}

fn step(app: &mut VectorcraftApp, r: &mut Open) {
    let Some((on, fields)) = dialog_state(app, r.id) else { return };
    let Some(state) = app.session.active().filter(|state| state.uid == r.document) else { return };
    if !r.owns_preview(app) {
        r.preview = LivePreview::default();
        r.preview_selection = None;
        r.last = Value::Null;
        r.original.clear();
        r.shown = false;
        if state.interaction.is_some() {
            return;
        }
    }
    let (cmd, params) = command(r.op, &fields);
    let stale = !on || r.last != params;
    if stale {
        r.original.clear();
    }
    if !on {
        r.last = Value::Null;
        r.preview_selection = None;
    }
    let mut counts = stale.then_some(None);
    if let Some(answer) = r.preview.run(app, on, r.op.title(), cmd, &params) {
        r.preview_selection = app.session.active().and_then(|state| state.interaction.as_ref()).map(|it| Arc::downgrade(&it.selection));
        if let Ok(answer) = answer {
            r.last = params;
            if r.op == PathOp::Simplify {
                r.original = answer["ids"].as_array().into_iter().flatten().filter_map(|id| id.as_u64().map(NodeId)).collect();
                counts = Some(Some(json!({"before": answer["before"], "after": answer["after"]})));
            }
        }
    }
    r.shown = on && fields.bool("showOriginal").unwrap_or(false);
    let Some(counts) = counts.filter(|_| r.op == PathOp::Simplify) else { return };
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|h| {
            let Some(d) = h.get_mut_by_id::<HostedParamDialog>(r.id) else { return };
            match counts {
                Some(v) => d.set_field(COUNTS, FieldValue::Json(v)),
                None if d.fields().contains(COUNTS) => d.clear_field(COUNTS),
                None => {}
            }
        });
    }
}

/// Simplify's Show Original Path: while the dialog previews with it on, every path the preview ran
/// on, with its id, as it was before the preview (from the interaction's snapshot). Empty
/// otherwise.
pub(crate) fn original_paths(app: &VectorcraftApp) -> Vec<(NodeId, &PathData)> {
    let Some(r) = app.object_dialogs.path.iter().find(|r| r.op == PathOp::Simplify && r.shown && r.owns_preview(app)) else { return vec![] };
    let Some(it) = app.session.active().and_then(|st| st.interaction.as_ref()) else { return vec![] };
    r.original
        .iter()
        .filter_map(|&id| match &it.doc.node(id)?.kind {
            NodeKind::Path { path, .. } => Some((id, path)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod average_document_tests {
    use super::*;
    use crate::dialogs::tests::{app, cancel, confirm, is_open, set};

    fn lines(app: &mut VectorcraftApp, top: i32) -> [u64; 2] {
        let a = app.run("shape.line", json!({"x1": 10, "y1": top, "x2": 50, "y2": top})).unwrap()["id"].as_u64().unwrap();
        let b = app.run("shape.line", json!({"x1": 60, "y1": top + 20, "x2": 100, "y2": top + 20})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        [a, b]
    }

    fn opening_owns_approval(op: PathOp) {
        for revert in [false, true] {
            let mut app = app();
            let source_paths = lines(&mut app, 20);
            let uid = app.session.active().unwrap().uid;
            let generation = app.session.active().unwrap().reverts;
            let dir = vectorcraft_testkit::TempDir::new("average-path-dialog-drawing");
            if revert {
                let path = dir.join("saved-paths.vectorcraft");
                app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
                app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
                app.run("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
                app.run("object.move", json!({"ids": source_paths, "dx": 12, "dy": 0})).unwrap();
            } else {
                app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
                assert_eq!(lines(&mut app, 80), source_paths);
                app.run("document.activate", json!({"index": 0})).unwrap();
            }
            let open = |app: &mut VectorcraftApp| {
                crate::menus::invoke(app, op.key(), json!({}));
                assert!(is_open(app));
                if op == PathOp::Average {
                    set(app, "axis", json!("both"));
                } else {
                    set(app, "to", json!("smooth"));
                }
            };
            open(&mut app);
            if revert {
                app.run("file.revert", json!({"confirmed": true})).unwrap();
                app.run("select.set", json!({"ids": source_paths})).unwrap();
                let restored = app.session.active().unwrap();
                assert_eq!((restored.uid, restored.reverts), (uid, generation + 1));
            } else {
                app.run("document.activate", json!({"index": 1})).unwrap();
                assert_ne!(app.session.active().unwrap().uid, uid);
            }
            let wrong = app.session.active().unwrap();
            let (before, undo) = (wrong.doc.clone(), wrong.history.undo.len());
            let accepted = confirm(&mut app);
            let after = app.session.active().unwrap();
            assert!(after.doc == before, "the old path approval must leave the other drawing contents intact");
            assert_eq!(after.history.undo.len(), undo);
            assert!(!accepted && is_open(&app));

            if revert {
                assert!(cancel(&mut app));
                open(&mut app);
            } else {
                app.run("document.activate", json!({"index": 0})).unwrap();
            }
            let before = app.session.active().unwrap();
            let (geometry, undo) = (before.doc.clone(), before.history.undo.len());
            assert!(confirm(&mut app));
            let after = app.session.active().unwrap();
            assert!(after.doc != geometry, "a current approval edits the intended paths");
            assert_eq!(after.history.undo.len(), undo + 1);
        }
    }

    #[test]
    fn average_path_approval_stays_with_its_drawing() {
        opening_owns_approval(PathOp::Average);
    }

    #[test]
    fn average_join_approval_stays_with_its_drawing() {
        opening_owns_approval(PathOp::AverageJoin);
    }
}

#[cfg(test)]
mod split_tests {
    use serde_json::json;

    use super::*;
    use crate::dialogs::tests::{app, cancel, confirm, frame, is_open, set};

    fn selected_rectangle(app: &mut VectorcraftApp, width: u32, height: u32) {
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": width, "height": height})).unwrap();
        app.run("select.all", json!({})).unwrap();
    }

    fn other_document(app: &mut VectorcraftApp) {
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        selected_rectangle(app, 150, 90);
    }

    #[test]
    fn split_into_grid_skips_preview_in_the_other_drawing() {
        let mut app = app();
        selected_rectangle(&mut app, 112, 62);
        let original = app.session.doc().unwrap().doc.clone();
        open_split(&mut app);
        set(&mut app, "columns", json!(3));
        other_document(&mut app);
        let other = app.session.doc().unwrap().doc.clone();
        set(&mut app, "preview", json!(true));

        frame(&mut app, vec![]);

        assert!(app.session.doc().unwrap().doc == other, "the other drawing is unchanged by Preview");
        assert!(!app.session.in_interaction());
        app.run("document.activate", json!({"index": 0})).unwrap();
        frame(&mut app, vec![]);
        assert_ne!(app.session.doc().unwrap().doc, original);
        assert!(app.session.in_interaction());
        assert!(confirm(&mut app));
        assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 6);
        assert!(!app.session.in_interaction());
    }

    #[test]
    fn split_into_grid_keeps_the_draft_when_ok_is_in_another_drawing() {
        let mut app = app();
        selected_rectangle(&mut app, 112, 62);
        open_split(&mut app);
        set(&mut app, "columns", json!(3));
        other_document(&mut app);
        let other = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();

        let closed = confirm(&mut app);

        assert!(app.session.doc().unwrap().doc == other, "the other drawing is unchanged by OK");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
        assert!(!closed && is_open(&app), "the user can return to confirm");
        app.run("document.activate", json!({"index": 0})).unwrap();
        assert!(confirm(&mut app));
        assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 6);
    }

    #[test]
    fn split_into_grid_cancel_preserves_the_other_drawings_edit() {
        let mut app = app();
        selected_rectangle(&mut app, 112, 62);
        other_document(&mut app);
        app.run("document.activate", json!({"index": 0})).unwrap();
        let opening = app.session.doc().unwrap().doc.clone();
        open_split(&mut app);
        set(&mut app, "preview", json!(true));
        frame(&mut app, vec![]);
        assert!(app.session.in_interaction(), "the opening drawing is previewing");

        app.run("document.activate", json!({"index": 1})).unwrap();
        app.session.begin_interaction("Other edit").unwrap();
        app.session.preview("paint.setFill", &json!({"color": "#00a000"})).unwrap();
        let other = app.session.doc().unwrap().doc.clone();
        assert!(cancel(&mut app));

        assert!(app.session.in_interaction(), "Cancel preserves the other drawing's pending edit");
        assert_eq!(app.session.doc().unwrap().doc, other);
        app.session.commit_interaction().unwrap();
        app.run("document.activate", json!({"index": 0})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, opening);
    }

    #[test]
    fn split_into_grid_does_not_adopt_a_new_edit_on_return() {
        let mut app = app();
        selected_rectangle(&mut app, 112, 62);
        other_document(&mut app);
        app.run("document.activate", json!({"index": 0})).unwrap();
        open_split(&mut app);
        set(&mut app, "preview", json!(true));
        frame(&mut app, vec![]);
        assert!(app.session.in_interaction());
        for index in [1, 0] {
            app.run("document.activate", json!({"index": index})).unwrap();
        }
        app.session.begin_interaction("New edit").unwrap();
        app.session.preview("paint.setFill", &json!({"color": "#00a000"})).unwrap();
        let editing = app.session.doc().unwrap().doc.clone();

        frame(&mut app, vec![]);
        assert!(!confirm(&mut app));
        assert!(cancel(&mut app));

        assert!(app.session.in_interaction());
        assert_eq!(app.session.doc().unwrap().doc, editing);
    }
}

#[cfg(test)]
mod preview_document_tests {
    use super::*;
    use crate::dialogs::tests::{app, cancel, confirm, frame, set};

    fn dense_path(app: &mut VectorcraftApp, top: f64) -> u64 {
        let anchors: Vec<_> = (0..20).map(|i| json!({"x": 30 + i * 15, "y": top + (i % 2) as f64 * 0.1})).collect();
        let id = app.run("path.create", json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap();
        app.run("select.anchors", json!({"id": id, "anchors": [[0, 19]]})).unwrap();
        id
    }

    fn drawings() -> (VectorcraftApp, NodeId) {
        let mut app = app();
        let id = dense_path(&mut app, 50.0);
        app.run("file.new", json!({"width": 500, "height": 400})).unwrap();
        let other = dense_path(&mut app, 180.0);
        assert_eq!(id, other, "the two drawings use the same path ID");
        app.run("document.activate", json!({"index": 0})).unwrap();
        (app, NodeId(id))
    }

    fn more_options(app: &mut VectorcraftApp) {
        crate::menus::invoke(app, "object.path.simplify", json!({}));
        crate::dialogs::simplify_slider::more_options(app).unwrap();
    }

    #[test]
    fn simplify_options_skip_the_other_drawings_artwork() {
        for from_slider in [false, true] {
            let (mut app, _) = drawings();
            if from_slider {
                more_options(&mut app);
            } else {
                let ctx = app.ui_ctx.clone().unwrap();
                set_saved_value(&ctx, &SIMPLIFY_KEY, Some(simplify_dialog(DEFAULT_TOLERANCE)));
                crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
            }
            set(&mut app, "preview", json!(true));
            app.run("document.activate", json!({"index": 1})).unwrap();
            let other = app.session.doc().unwrap().doc.clone();
            let undo = app.session.doc().unwrap().history.undo.len();

            frame(&mut app, vec![]);

            assert!(app.session.doc().unwrap().doc == other, "Simplify preview leaves the other drawing unchanged");
            assert!(!app.session.in_interaction());
            assert!(!confirm(&mut app), "the draft stays with the opening drawing");
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
            app.run("document.activate", json!({"index": 0})).unwrap();
            frame(&mut app, vec![]);
            assert!(app.session.in_interaction());
            assert!(confirm(&mut app));
        }
    }

    #[test]
    fn simplify_options_never_outline_another_drawings_same_id() {
        let (mut app, id) = drawings();
        more_options(&mut app);
        set(&mut app, "showOriginal", json!(true));
        set(&mut app, "preview", json!(true));
        frame(&mut app, vec![]);
        assert_eq!(original_paths(&app).iter().map(|(path, _)| *path).collect::<Vec<_>>(), [id]);

        app.run("document.activate", json!({"index": 1})).unwrap();
        app.session.begin_interaction("Other edit").unwrap();

        assert!(original_paths(&app).is_empty(), "Show Original leaves the other drawing's canvas alone");
        assert!(cancel(&mut app));
        assert!(app.session.in_interaction(), "Cancel leaves the other drawing's edit in progress");
    }

    #[test]
    fn offset_path_skips_the_other_drawings_artwork() {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 100, "height": 60})).unwrap();
        app.run("select.all", json!({})).unwrap();
        crate::menus::invoke(&mut app, "object.path.offsetPath", json!({}));
        set(&mut app, "offset", json!(12.0));
        set(&mut app, "preview", json!(true));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 140, "y": 30, "width": 80, "height": 100})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let other = app.session.doc().unwrap().doc.clone();

        frame(&mut app, vec![]);

        assert!(app.session.doc().unwrap().doc == other, "Offset Path preview leaves the other drawing unchanged");
        assert!(!confirm(&mut app));
        app.run("document.activate", json!({"index": 0})).unwrap();
        assert!(confirm(&mut app));
        assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
    }
}
