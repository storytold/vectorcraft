//! View → Perspective Grid → Define Grid…: the grid's type, units and scale, gridline spacing,
//! viewing angle and distance, horizon height and third vanishing point, and the planes' gridline
//! colours and opacity, prefilled from the grid (`perspective.grid.get`). OK runs
//! `perspective.grid.define`: one undo step, nothing when nothing changed.
//!
//! The Preset menu loads a preset's fields (it reads [Custom] once they differ); Save Preset…
//! asks for a name and saves the fields as a preset (`perspective.presets.save`), then comes back.
//! The same dialog is View → Perspective Grid → Save Grid as Preset… and the preset editor of
//! Edit → Perspective Grid Presets (New…, Edit…), whose OK returns to the presets.
//!
//! Fields: the keys of `GridDefinition` (lengths are real-world lengths in `units`; `name` the
//! preset). What the dialog is (Define Grid, Save Grid as Preset, the preset editor) is its mode,
//! not a field.

use std::cell::Cell;

use craft::theme::variant::Tone;
use craft::theme::{Rgba, Srgba};
use craft::ui::buttons::Button;
use craft::ui::choice::Swatch;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::{Field, SliderField, SliderFieldLayout};
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::patterns::presets::{PresetPicker, PresetRow, PresetSource, PresetState};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Label, LengthUnits, UnitKind};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::style::TypeRole;
use craft::ui::text::{NumberField, TextField};
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_tools::distort::perspective::define::is_builtin;
use vectorcraft_tools::distort::perspective::{GridDefinition, Rgb};

use crate::VectorcraftApp;
use crate::canvas::rulers::{general_lengths, unit_kind};
use crate::panels::label_or_name;

/// The dialog kind of Define Perspective Grid.
pub const KIND: &str = "perspectiveGrid";

/// The Preset menu's entry for fields no preset has.
const CUSTOM: &str = "[Custom]";

/// The scale choices: (label, [artboard, real world]).
const SCALES: [(&str, [f64; 2]); 9] = [
    ("1:1", [1.0, 1.0]),
    ("1:2", [1.0, 2.0]),
    ("1:4", [1.0, 4.0]),
    ("1:8", [1.0, 8.0]),
    ("1:12", [1.0, 12.0]),
    ("1:24", [1.0, 24.0]),
    ("1:48", [1.0, 48.0]),
    ("1:96", [1.0, 96.0]),
    ("1:100", [1.0, 100.0]),
];

const TYPES: [&str; 3] = ["One Point Perspective", "Two Point Perspective", "Three Point Perspective"];

/// The least and greatest custom scale part, as `GridDefinition::validate` takes them.
const SCALE_RANGE: std::ops::RangeInclusive<f64> = 0.001..=1.0e6;

/// The translated English string `s` as a label.
pub(super) fn word(s: &str) -> Label {
    Label::from(crate::i18n::t(s).to_owned())
}

/// What the dialog is.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum GridMode {
    /// Define Grid: OK defines the document's grid.
    Define,
    /// Save Grid as Preset: OK saves a new preset; `back` returns to Define Grid on it (Save Preset…).
    Save {
        /// Whether the dialog goes back to Define Grid once the preset is saved.
        back: bool,
    },
    /// The preset editor: OK saves `original`'s changes, or a new preset when there is none; the presets
    /// manager opens again on it.
    Edit {
        /// The saved preset the editor changes.
        original: Option<String>,
    },
}

/// The fields the dialog edits.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct GridDraft {
    /// The grid as the dialog shows it.
    def: GridDefinition,
}

impl GridDraft {
    /// The type, 1 to 3.
    fn kind(&self) -> u8 {
        self.def.kind.clamp(1, 3)
    }

    /// Shows the lengths in `unit`: they keep their size, so their numbers convert.
    fn set_units(&mut self, unit: Unit) {
        let before = self.def.unit();
        if unit == before {
            return;
        }
        let f = before.points() / unit.points();
        let d = &mut self.def;
        for v in [&mut d.gridline, &mut d.distance, &mut d.horizon_height].into_iter().chain(d.third_vp.iter_mut()) {
            *v *= f;
        }
        d.units = unit.key().into();
    }

    /// The name of the preset among `presets` whose fields these are; a draft that is no preset's loses its name
    /// (the Preset row reads [Custom]).
    fn matched(&mut self, presets: &[GridDefinition]) -> Option<String> {
        let found = presets.iter().find(|p| p.name == self.def.name && self.def.same(p)).map(|p| p.name.clone());
        if found.is_none() {
            self.def.name.clear();
        }
        found
    }
}

/// The active document's grid as Define Grid fields.
fn grid_fields(app: &mut VectorcraftApp) -> Result<Value, String> {
    Ok(app.session.execute("perspective.grid.get", &json!({})).map_err(|e| e.to_string())?["define"].take())
}

/// Opens the dialog on `def` as `mode`.
fn open_on(app: &mut VectorcraftApp, def: GridDefinition, mode: GridMode) -> Result<Value, String> {
    let dialog = GridDialog::new(
        def,
        mode,
        app.session.active().map(|st| (st.uid, st.reverts)),
        (general_lengths(app).pixels_per_inch, app.session.prefs.numbers_without_units_are_points),
        app.session.perspective_presets(),
        app.session.new_perspective_preset_name(),
    );
    app.dialogs().ok_or("no window")?.with(|host| host.open(dialog));
    Ok(Value::Null)
}

/// Open Define Grid on the active document's grid.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let def = serde_json::from_value(grid_fields(app)?).map_err(|e| e.to_string())?;
    open_on(app, def, GridMode::Define)
}

/// View → Perspective Grid → Save Grid as Preset…: the grid's fields under a new preset name.
pub fn open_save(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut def: GridDefinition = serde_json::from_value(grid_fields(app)?).map_err(|e| e.to_string())?;
    def.name = app.session.new_perspective_preset_name();
    open_on(app, def, GridMode::Save { back: false })
}

/// The preset editor: the saved preset `original` (Edit…), or a new preset starting from `base`
/// (New…). Its OK returns to Edit → Perspective Grid Presets.
pub fn open_edit(app: &mut VectorcraftApp, original: Option<&str>, base: &str) -> Result<(), String> {
    let name = original.unwrap_or(base);
    let mut def = app.session.perspective_preset(name).ok_or_else(|| format!("no perspective grid preset named `{name}`"))?;
    if original.is_none() {
        def.name = app.session.new_perspective_preset_name();
    }
    open_on(app, def, GridMode::Edit { original: original.map(str::to_owned) }).map(|_| ())
}

/// Define Perspective Grid, Save Grid as Preset and the preset editor, on the window's dialog host. OK files the
/// fields for [`pump`], which runs the command and answers it.
pub(super) struct GridDialog {
    id: DialogId,
    mode: GridMode,
    /// The document and loaded contents whose grid Define Grid edits.
    source_document: Option<(u64, u64)>,
    draft: GridDraft,
    /// The presets the Preset row offers, as the document and the preferences have them.
    presets: Vec<GridDefinition>,
    /// The Preset row's rows, for `presets`.
    rows: Vec<PresetRow<String>>,
    /// The name Save Preset… starts from.
    new_name: String,
    /// The document's pixels per inch, which sizes lengths shown in pixels.
    ppi: f64,
    /// Numbers Without Units Are Points.
    bare_points: bool,
    picker: PresetState<String>,
    held: HeldAnswer<GridDefinition>,
    /// The draft the filed answer was refused for: the reason shows until the draft changes.
    refused_at: Option<GridDraft>,
    open: bool,
}

impl GridDialog {
    fn new(
        def: GridDefinition,
        mode: GridMode,
        source_document: Option<(u64, u64)>,
        (ppi, bare_points): (f64, bool),
        presets: Vec<GridDefinition>,
        new_name: String,
    ) -> Self {
        let mut dialog = GridDialog {
            id: dialog_id(KIND),
            mode,
            source_document,
            draft: GridDraft { def },
            presets: Vec::new(),
            rows: Vec::new(),
            new_name,
            ppi,
            bare_points,
            picker: PresetState::default(),
            held: HeldAnswer::new(),
            refused_at: None,
            open: true,
        };
        dialog.set_presets(presets, dialog.new_name.clone());
        dialog
    }

    /// The presets and the name a new preset starts from, as the app has them now.
    fn set_presets(&mut self, presets: Vec<GridDefinition>, new_name: String) {
        self.new_name = new_name;
        if self.presets != presets {
            self.rows = presets.iter().map(|p| PresetRow::item(p.name.clone(), label_or_name(&p.name, is_builtin(&p.name)).to_owned(), 0)).collect();
            self.presets = presets;
        }
    }

    /// Save Preset…: the fields under a new name, back to Define Grid once saved.
    pub(super) fn start_save(&mut self) {
        self.mode = GridMode::Save { back: true };
        self.draft.def.name = self.new_name.clone();
    }

    /// The saved preset `name` is the fields now: Define Grid again, on it.
    fn back_to_define(&mut self, name: String) {
        self.mode = GridMode::Define;
        self.draft.def.name = name;
        self.held = HeldAnswer::new();
        self.refused_at = None;
    }

    /// The app refused the filed answer for `reason`.
    fn refuse(&mut self, reason: Label) {
        self.refused_at = Some(self.draft.clone());
        self.held.answer(Err(reason));
    }

    fn title(&self) -> Label {
        match self.mode {
            GridMode::Define => word("Define Perspective Grid"),
            GridMode::Save { .. } => word("Save Grid as Preset"),
            GridMode::Edit { .. } => word("Perspective Grid Preset"),
        }
    }
}

/// What a pass reads besides the draft.
struct Pass<'p> {
    mode: &'p GridMode,
    presets: &'p [GridDefinition],
    rows: &'p [PresetRow<String>],
    ppi: f64,
    bare_points: bool,
    /// Why the app refused the last OK.
    refusal: Option<&'p Label>,
}

impl Pass<'_> {
    /// The rows; whether the draft can be accepted. `save` is set when Save Preset… was pressed.
    fn body(&self, ui: &mut egui::Ui, draft: &mut GridDraft, picker: &mut PresetState<String>, save: &mut bool) -> bool {
        self.name(ui, draft, picker, save);
        crate::panels::section(word("Perspective Grid Settings")).id_salt("settings").show(ui, |ui, _| self.settings(ui, draft));
        crate::panels::section(word("Grid Color & Opacity")).id_salt("colors").show(ui, |ui, _| colors(ui, draft));
        let valid = draft.def.validate();
        if let Err(why) = &valid {
            Text::new(why.clone()).role(TypeRole::Caption).tone(Tone::Bad).show(ui);
        } else if let Some(reason) = self.refusal {
            Text::new(reason.clone()).role(TypeRole::Caption).tone(Tone::Bad).show(ui);
        }
        valid.is_ok()
    }

    /// The Name row: in Define Grid the Preset menu (choosing a preset loads its fields) and Save Preset…, else the
    /// preset's name.
    fn name(&self, ui: &mut egui::Ui, draft: &mut GridDraft, picker: &mut PresetState<String>, save: &mut bool) {
        if *self.mode != GridMode::Define {
            let label = word("Name:");
            row(ui, &label, "name", |ui| {
                if let Some(name) = TextField::new(label.clone(), Bind::given(draft.def.name.clone())).show(ui).value().cloned() {
                    draft.def.name = name;
                }
            });
            return;
        }
        let current = draft.matched(self.presets);
        Row::new().id_salt("preset").show(ui, |ui, _| {
            let shown = PresetPicker::new(word("Preset:"), Bind::given(current.clone()), &PresetSource::listed(self.rows), Bind::bound(picker))
                .placeholder(CUSTOM)
                .show(ui);
            if let Some(preset) = shown.picked.and_then(|name| self.presets.iter().find(|p| p.name == name)) {
                draft.def = preset.clone();
            }
            let button = Button::new(word("Save Preset…")).tooltip(word("Save these settings as a preset")).show(ui);
            *save |= clicked(&button);
        });
    }

    fn settings(&self, ui: &mut egui::Ui, draft: &mut GridDraft) {
        let kind = draft.kind();
        let types: Vec<Entry<u8>> = (1..=3).zip(TYPES).map(|(k, name)| Entry::new(k, word(name))).collect();
        choose(ui, "Type:", "type", &types, kind, |k| draft.def.kind = k);
        let units: Vec<Entry<Unit>> = Unit::ALL.iter().filter(|u| **u != Unit::FeetInches).map(|u| Entry::new(*u, u.label())).collect();
        choose(ui, "Units:", "units", &units, draft.def.unit(), |u| draft.set_units(u));

        let unit = draft.def.unit();
        let lengths = crate::fields::bare_points(LengthUnits::new(unit_kind(unit), unit_kind(unit), self.ppi), self.bare_points);
        let scale = draft.def.scale;
        let named = SCALES.iter().position(|(_, s)| *s == scale);
        let mut scales: Vec<Entry<usize>> = SCALES.iter().enumerate().map(|(i, (label, _))| Entry::new(i, *label)).collect();
        scales.push(Entry::new(SCALES.len(), word("Custom")));
        // Custom starts from the scale shown.
        choose(ui, "Scale:", "scale", &scales, named.unwrap_or(SCALES.len()), |i| draft.def.scale = SCALES.get(i).map_or(scale, |(_, s)| *s));
        if named.is_none() {
            Row::new().id_salt("custom-scale").show(ui, |ui, _| {
                for (i, label) in [word("Artboard:"), word("Real World:")].into_iter().enumerate() {
                    let field = NumberField::new(label, Bind::given(Some(scale[i]))).range(SCALE_RANGE).show(ui);
                    if let Some(v) = field.edit.value().copied().flatten() {
                        draft.def.scale[i] = v;
                    }
                }
            });
        }
        length(ui, "Gridline every:", "gridline", &mut draft.def.gridline, lengths);
        if scale[0] != scale[1] {
            let real = unit.number(unit.to_pt(scale[1] / scale[0]));
            let note = crate::i18n::fmt(
                tl!("1 {unit} on the artboard stands for {real} {unit} in the scene."),
                &[("unit", unit.suffix()), ("real", real.as_str())],
            );
            Text::new(note).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        }
        let label = word("Viewing Angle:");
        row(ui, &label, "angle", |ui| {
            let field = NumberField::new(label.clone(), Bind::given(Some(draft.def.angle))).unit_kind(UnitKind::Degrees);
            if let Some(v) = field.range(0.01..=89.99).enabled(kind != 1).show(ui).edit.value().copied().flatten() {
                draft.def.angle = v;
            }
        });
        length(ui, "Viewing Distance:", "distance", &mut draft.def.distance, lengths);
        length(ui, "Horizon Height:", "horizonHeight", &mut draft.def.horizon_height, lengths);
        let label = word("Third Vanishing Point:");
        row(ui, &label, "third-vp", |ui| {
            Row::new().id_salt("third-vp-row").show(ui, |ui, _| {
                for (i, axis) in ["X:", "Y:"].into_iter().enumerate() {
                    let field = NumberField::length(word(axis), Bind::given(Some(draft.def.third_vp[i])), lengths).enabled(kind == 3);
                    if let Some(v) = field.show(ui).edit.value().copied().flatten() {
                        draft.def.third_vp[i] = v;
                    }
                }
            });
        });
    }
}

/// Shows `body` in a field named `name`.
fn row(ui: &mut egui::Ui, name: &Label, salt: &str, mut body: impl FnMut(&mut egui::Ui)) {
    Field::new(name.clone()).id_salt(salt).show(ui, |ui, _| body(ui));
}

/// A select of `entries` named `name` (an English label), over `value`; `set` takes the entry chosen, not one only
/// highlighted.
fn choose<T: craft::behavior::Value>(ui: &mut egui::Ui, name: &str, salt: &str, entries: &[Entry<T>], value: T, mut set: impl FnMut(T)) {
    let label = word(name);
    row(ui, &label, salt, |ui| {
        if let Some(picked) = Select::new(label.clone(), Bind::given(value.clone()), entries).show(ui).events.committed().cloned() {
            set(picked);
        }
    });
}

/// A length field named `name` (an English label) over `value`, in the dialog's unit.
fn length(ui: &mut egui::Ui, name: &str, salt: &str, value: &mut f64, lengths: LengthUnits) {
    let label = word(name);
    row(ui, &label, salt, |ui| {
        if let Some(v) = NumberField::length(label.clone(), Bind::given(Some(*value)), lengths).show(ui).edit.value().copied().flatten() {
            *value = v;
        }
    });
}

/// The three planes' gridline colours, and the opacity.
fn colors(ui: &mut egui::Ui, draft: &mut GridDraft) {
    let d = &mut draft.def;
    for (name, salt, rgb) in
        [("Left Grid:", "left", &mut d.left_color), ("Right Grid:", "right", &mut d.right_color), ("Horizontal Grid:", "ground", &mut d.ground_color)]
    {
        let label = word(name);
        let shown = Rgba::rgb(rgb.0[0], rgb.0[1], rgb.0[2]);
        row(ui, &label, salt, |ui| {
            Row::new().id_salt((salt, "color")).show(ui, |ui, _| {
                Swatch::new(label.clone(), Srgba::from(shown)).show(ui);
                if let Some(Rgba([r, g, b, _])) = TextField::hex(label.clone(), Bind::given(shown)).show(ui).value().copied() {
                    *rgb = Rgb([r, g, b]);
                }
            });
        });
    }
    let label = word("Opacity:");
    row(ui, &label, "opacity", |ui| {
        let field = SliderField::new(label.clone(), Bind::given(d.opacity), 0.0..=100.0).step(1.0).layout(SliderFieldLayout::Inline).show(ui);
        if let Some(v) = field.value().map(|v| v.round().clamp(0.0, 100.0)) {
            d.opacity = v;
        }
    });
}

impl StackDialog for GridDialog {
    type Result = ();

    /// Files the fields as OK does, for the app to run; answers none until the app accepted them.
    fn confirm(&mut self, _ctx: &egui::Context) -> Option<()> {
        if self.held.is_accepted() {
            return self.held.finish().map(|_| ());
        }
        if self.held.is_waiting() || self.draft.def.validate().is_err() {
            return None;
        }
        self.held.file(self.draft.def.clone());
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    /// The keys of `GridDefinition`.
    fn json_fields(&self) -> Option<Value> {
        serde_json::to_value(&self.draft.def).ok()
    }

    /// Sets a field as `GridDefinition::merged` reads it; `units` converts the lengths. The angle takes no input
    /// in a one-point grid, nor the third vanishing point outside a three-point one.
    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        let known = serde_json::to_value(&self.draft.def).map_err(|e| e.to_string())?;
        if known.get(field).is_none() {
            return Err(FieldRefusal::Unknown.message(field));
        }
        let kind = self.draft.kind();
        if (field == "angle" && kind == 1) || (field == "thirdVp" && kind != 3) {
            return Err(FieldRefusal::Inactive.message(field));
        }
        if field == "units" {
            let unit = value.as_str().and_then(Unit::named).filter(|u| *u != Unit::FeetInches).ok_or_else(rejects)?;
            self.draft.set_units(unit);
            return Ok(());
        }
        self.draft.def = self.draft.def.merged(&json!({ field: value.clone() })).map_err(|_| rejects())?;
        Ok(())
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let finish = self.held.is_accepted();
        if self.refused_at.as_ref().is_some_and(|at| *at != self.draft) {
            self.refused_at = None;
            self.held.clear_refusal();
        }
        let mut save = false;
        let title = self.title();
        let pass = Pass {
            mode: &self.mode,
            presets: &self.presets,
            rows: &self.rows,
            ppi: self.ppi,
            bare_points: self.bare_points,
            refusal: self.held.refusal().map(|(reason, _)| reason),
        };
        let picker = &mut self.picker;
        let accepted = Cell::new(false);
        // OK files the answer and the dialog stays; it closes once the app accepted it.
        let shown = FormDialog::new(title, Bind::bound(&mut self.draft))
            .opened(Bind::bound(&mut self.open))
            .ok_enabled(finish || !self.held.is_waiting())
            .accept_stays_open(true)
            .show(ui, |ui, form, draft| {
                if finish {
                    form.request_close_as(CloseReason::Activated);
                }
                let valid = pass.body(ui, draft, picker, &mut save);
                accepted.set(valid);
                valid
            });
        if save {
            self.start_save();
            ui.ctx().request_repaint();
        }
        if shown.picked == Some(0) && accepted.get() && !self.held.is_waiting() {
            self.held.file(self.draft.def.clone());
            ui.ctx().request_repaint();
        }
        if finish && shown.accepted.is_some() {
            self.held.finish();
            return Some(DialogResult::Ok(()));
        }
        shown.closed.map(|_| DialogResult::Cancel)
    }
}

/// Runs the OK the open dialog filed: Define Grid defines the grid, the other modes save the preset.
enum Next {
    /// The dialog closes.
    Close,
    /// Save Preset… saved `name`: back to Define Grid on it.
    Define(String),
    /// The preset editor saved `name`: the presets manager opens on it.
    Presets(String),
}

fn file(app: &mut VectorcraftApp, mode: &GridMode, source_document: Option<(u64, u64)>, def: &GridDefinition) -> Result<Next, String> {
    let mut params = serde_json::to_value(def).map_err(|e| e.to_string())?;
    if *mode == GridMode::Define {
        let active = app.session.active().map(|st| (st.uid, st.reverts));
        if active.map(|(uid, _)| uid) != source_document.map(|(uid, _)| uid) {
            return Err(tl!("Define Grid can only change the document where it was opened.").to_owned());
        }
        if active != source_document {
            return Err(tl!("The document was reverted. Reopen this dialog to continue.").to_owned());
        }
        app.run("perspective.grid.define", params)?;
        return Ok(Next::Close);
    }
    if let GridMode::Edit { original: Some(original) } = mode {
        params["newName"] = params["name"].take();
        params["name"] = json!(original);
    }
    let saved = app.run("perspective.presets.save", params)?;
    let name = saved["name"].as_str().unwrap_or_default().to_string();
    Ok(match mode {
        GridMode::Edit { .. } => Next::Presets(name),
        GridMode::Save { back: true } => Next::Define(name),
        _ => Next::Close,
    })
}

/// Once a frame: gives the open dialog the app's presets and runs the OK it filed. A failure shows under the rows
/// and on the status line; the dialog stays.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    if dialogs.with(|host| host.get::<GridDialog>().is_none()) {
        return;
    }
    let (presets, new_name) = (app.session.perspective_presets(), app.session.new_perspective_preset_name());
    let filed = dialogs.with(|host| {
        let dialog = host.get_mut::<GridDialog>()?;
        dialog.set_presets(presets, new_name);
        dialog.held.take().map(|def| (dialog.mode.clone(), dialog.source_document, def))
    });
    let Some((mode, source_document, def)) = filed else { return };
    let next = file(app, &mode, source_document, &def);
    if let Err(why) = &next {
        app.status(why.clone());
    }
    // The new preset is in the list the dialog reads next.
    let (presets, new_name) = (app.session.perspective_presets(), app.session.new_perspective_preset_name());
    dialogs.with(|host| {
        let Some(dialog) = host.get_mut::<GridDialog>() else { return };
        dialog.set_presets(presets, new_name);
        match &next {
            Ok(Next::Define(name)) => dialog.back_to_define(name.clone()),
            Ok(_) => {
                dialog.held.answer(Ok(()));
            }
            Err(why) => dialog.refuse(Label::from(why.clone())),
        }
    });
    if let Ok(Next::Presets(name)) = next {
        super::perspective_presets::open(app, Some(&name));
    }
}
