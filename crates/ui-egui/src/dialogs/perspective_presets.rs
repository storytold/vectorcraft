//! Edit → Perspective Grid Presets…: every preset in a list (the built-in views, protected, then
//! the saved ones) with the selected one's settings beside it; New… (a preset starting from the
//! selected one), Edit… (saved presets), Delete, Import… and Export… (the selected preset) under
//! the list. New… and Edit… open the preset editor (the Define Grid dialog in its editor mode),
//! whose OK comes back here.
//!
//! Also View → Perspective Grid → One/Two/Three Point Perspective: the built-in views of the type
//! and slots for the saved presets of that type ([`SLOT`]).
//!
//! Field: `selected` (a preset's name).

use std::sync::Arc;

use craft::behavior::{DisplayOrder, ItemSource, Selection};
use craft::host::PickRequest;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use craft::ui::text::SelectableText;
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::perspgrid::{PRESET_EXTS, PRESET_FORMAT};
use vectorcraft_tools::distort::perspective::GridDefinition;
use vectorcraft_tools::distort::perspective::define::{BUILTINS, is_builtin};

use super::perspective_grid::word;
use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};
use crate::menus::{Node, c, cp};
use crate::panels::label_or_name;

/// The dialog kind of Perspective Grid Presets.
pub const KIND: &str = "perspectiveGridPresets";

/// The id prefix of the saved-preset slots in the One/Two/Three Point Perspective menus:
/// `ui.perspectiveUserPreset<kind>.<n>` is the n-th saved preset of that type.
pub const SLOT: &str = "ui.perspectiveUserPreset";

/// Saved-preset slots per type.
pub const SLOTS: usize = 5;

/// The rows the preset list shows before it scrolls.
const LIST_ROWS: u16 = 10;

/// What a button asked for, done once the list and the details are drawn.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Action {
    Select(String),
    New,
    Edit,
    Delete,
    Import,
    Export,
}

/// The settings rows of a preset: (label, value).
fn details(p: &GridDefinition) -> Vec<(&'static str, String)> {
    let u = p.unit();
    let len = |v: f64| u.format(u.to_pt(v));
    let mut rows = vec![
        (
            tl!("Type:"),
            [tl!("One Point"), tl!("Two Point"), tl!("Three Point")]
                .get(usize::from(p.kind.clamp(1, 3) - 1))
                .copied()
                .unwrap_or_default()
                .to_string(),
        ),
        (tl!("Units:"), u.label().to_string()),
        (tl!("Scale:"), format!("{}:{}", Unit::Points.number(p.scale[0]), Unit::Points.number(p.scale[1]))),
        (tl!("Gridline every:"), len(p.gridline)),
    ];
    if p.kind != 1 {
        rows.push((tl!("Viewing Angle:"), format!("{:.1}°", p.angle)));
    }
    rows.extend([(tl!("Viewing Distance:"), len(p.distance)), (tl!("Horizon Height:"), len(p.horizon_height))]);
    if p.kind == 3 {
        rows.push((tl!("Third Vanishing Point:"), format!("{}, {}", len(p.third_vp[0]), len(p.third_vp[1]))));
    }
    rows.push((tl!("Opacity:"), format!("{:.0}%", p.opacity)));
    rows
}

/// The shown names of the presets, as the list's rows.
struct Names<'a>(&'a [String]);

impl DisplayOrder<usize> for Names<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        self.0.get(at).map(|name| std::borrow::Cow::Borrowed(name.as_str()))
    }
}

impl ItemSource<usize> for Names<'_> {}

/// The presets manager on the window's dialog host.
pub(super) struct PresetsDialog {
    id: DialogId,
    selected: String,
    presets: Vec<GridDefinition>,
    /// The names the list shows for `presets`: the built-in views translated, the saved ones as named.
    names: Vec<String>,
    /// The button pressed in the last pass, for [`pump`] to run outside the host's lock.
    action: Option<Action>,
    /// Identifies the manager that opened an Import picker.
    import_owner: Arc<()>,
    open: bool,
}

impl PresetsDialog {
    fn new(selected: &str, presets: Vec<GridDefinition>) -> Self {
        let mut dialog = PresetsDialog {
            id: dialog_id(KIND),
            selected: selected.to_owned(),
            presets: Vec::new(),
            names: Vec::new(),
            action: None,
            import_owner: Arc::new(()),
            open: true,
        };
        dialog.set_presets(presets);
        dialog
    }

    /// The presets as the document and the preferences have them now.
    fn set_presets(&mut self, presets: Vec<GridDefinition>) {
        if self.presets != presets {
            self.names = presets.iter().map(|p| label_or_name(&p.name, is_builtin(&p.name)).to_owned()).collect();
            self.presets = presets;
        }
    }

    /// The place of the preset named `name`, trimmed and in any case.
    fn index_of(&self, name: &str) -> Option<usize> {
        self.presets.iter().position(|p| p.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The place of the selected preset: the first one when `selected` names none.
    fn index(&self) -> usize {
        self.index_of(&self.selected).unwrap_or(0)
    }
}

/// The list of presets with its buttons, and the settings of the preset at `index`. A button sets `action`; the
/// built-in views take neither Edit… nor Delete.
fn body(ui: &mut egui::Ui, presets: &[GridDefinition], names: &[String], index: usize, action: &mut Option<Action>) {
    let Some(preset) = presets.get(index) else { return };
    let builtin = is_builtin(&preset.name);
    Row::new().id_salt("presets-body").show(ui, |ui, _| {
        Stack::new().id_salt("presets-list").show(ui, |ui, _| {
            Text::new(word("Presets:")).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            let mut chosen = Selection::single(index);
            let _ = List::<usize>::new(word("Presets:"))
                .header(false)
                .select(SelectMode::Single)
                .selection(Bind::bound(&mut chosen))
                .max_rows(LIST_ROWS)
                .show_items(ui, &Names(names), &mut |key, _| ListRow::new(names.get(*key).cloned().unwrap_or_default()));
            if let Some(picked) = chosen.items.first().filter(|k| **k != index).and_then(|k| presets.get(*k)) {
                *action = Some(Action::Select(picked.name.clone()));
            }
            Row::new().id_salt("presets-actions").show(ui, |ui, _| {
                for (act, label, tip, enabled) in [
                    (Action::New, "New…", Some("A new preset starting from the selected one"), true),
                    (Action::Edit, "Edit…", None, !builtin),
                    (Action::Delete, "Delete", None, !builtin),
                    (Action::Import, "Import…", None, true),
                    (Action::Export, "Export…", Some("Save the selected preset to a file"), true),
                ] {
                    let mut button = Button::new(word(label)).enabled(enabled);
                    if let Some(tip) = tip {
                        button = button.tooltip(word(tip));
                    }
                    if clicked(&button.show(ui)) {
                        *action = Some(act);
                    }
                }
            });
        });
        Stack::new().id_salt("presets-details").show(ui, |ui, _| {
            Text::new(names.get(index).cloned().unwrap_or_default()).role(TypeRole::BodyStrong).show(ui);
            for (at, (label, value)) in details(preset).into_iter().enumerate() {
                Field::new(label).id_salt(("detail", at)).show(ui, |ui, _| {
                    SelectableText::new(label, &value).show(ui);
                });
            }
            if builtin {
                Text::new(word("Built-in presets don't change: New… starts an editable copy.")).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            }
        });
    });
}

impl StackDialog for PresetsDialog {
    type Result = ();

    /// The selected preset, `selected`.
    fn json_fields(&self) -> Option<Value> {
        Some(json!({ "selected": self.selected }))
    }

    /// Selects the preset `selected` names, in any case.
    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if field != "selected" {
            return Err(FieldRefusal::Unknown.message(field));
        }
        let at = value.as_str().and_then(|name| self.index_of(name)).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
        self.selected = self.presets.get(at).map(|p| p.name.clone()).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
        Ok(())
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let mut action = None;
        let index = self.index();
        let (presets, names) = (&self.presets, &self.names);
        let shown = Dialog::new(word("Perspective Grid Presets"))
            .buttons([FooterButton::new(word("Close"), DialogRole::Cancel)])
            .opened(Bind::bound(&mut self.open))
            .show(ui, |ui, _| body(ui, presets, names, index, &mut action));
        if action.is_some() {
            self.action = action;
        }
        shown.output.closed.map(|_| DialogResult::Cancel)
    }
}

/// Open the presets manager on `selected` (default: the first preset).
pub fn open(app: &mut VectorcraftApp, selected: Option<&str>) {
    let dialog = PresetsDialog::new(selected.unwrap_or(BUILTINS[0].0), app.session.perspective_presets());
    match app.dialogs() {
        Some(dialogs) => {
            dialogs.with(|host| host.open(dialog));
        }
        None => app.status("no window"),
    }
}

/// Runs `f` on the open presets manager.
fn with_dialog<R>(app: &VectorcraftApp, f: impl FnOnce(&mut PresetsDialog) -> R) -> Option<R> {
    app.dialogs()?.with(|host| host.get_mut::<PresetsDialog>().map(f))
}

fn select(app: &VectorcraftApp, name: &str) {
    with_dialog(app, |d| d.selected = name.to_owned());
}

/// Once a frame: gives the open manager the app's presets and runs the button it was pressed with, outside the
/// host's lock.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    if app.dialogs().is_none_or(|dialogs| dialogs.with(|host| host.get::<PresetsDialog>().is_none())) {
        return;
    }
    let presets = app.session.perspective_presets();
    let pressed = with_dialog(app, |d| {
        d.set_presets(presets);
        d.action.take().zip(d.presets.get(d.index()).map(|p| p.name.clone()))
    });
    if let Some((action, current)) = pressed.flatten()
        && let Err(e) = run(app, action, &current)
    {
        app.status(e);
    }
}

pub(super) fn run(app: &mut VectorcraftApp, act: Action, current: &str) -> Result<(), String> {
    match act {
        Action::Select(name) => select(app, &name),
        // The editor replaces this dialog; its OK comes back here.
        Action::New | Action::Edit => {
            let original = matches!(act, Action::Edit).then_some(current);
            super::perspective_grid::open_edit(app, original, current)?;
            let ctx = app.ui_ctx.clone().ok_or("no window")?;
            app.dialogs().ok_or("no window")?.with(|host| host.cancel::<PresetsDialog>(&ctx));
        }
        Action::Delete => {
            let i = app.session.prefs.perspective_presets.iter().position(|p| p.name == current);
            app.run("perspective.presets.delete", json!({ "name": current }))?;
            // The one above it takes its place (the last built-in view above the first saved one).
            let saved = &app.session.prefs.perspective_presets;
            let next =
                i.and_then(|i| i.checked_sub(1)).and_then(|i| saved.get(i)).map_or(BUILTINS[BUILTINS.len() - 1].0, |p| p.name.as_str()).to_string();
            select(app, &next);
        }
        Action::Import => {
            let Some(owner) = with_dialog(app, |manager| Arc::clone(&manager.import_owner)) else { return Ok(()) };
            let request = PickRequest::open(KIND).filter("Perspective grid presets", PRESET_EXTS);
            file_asks::ask(app, request, Owner::Free, move |app, picked| {
                let mut params = file_asks::open_params(&picked).ok_or("cancelled")?;
                params.remove("name");
                let r = app.run("perspective.presets.import", Value::Object(params))?;
                if let Some(first) = r["imported"].get(0).and_then(Value::as_str) {
                    with_dialog(app, |manager| {
                        if Arc::ptr_eq(&manager.import_owner, &owner) {
                            manager.selected = first.to_owned();
                        }
                    });
                }
                Ok(r)
            });
        }
        Action::Export => {
            super::preset_manager::export(app, "perspective.presets.export", PRESET_FORMAT, current)?;
        }
    }
    Ok(())
}

// ---------- the One/Two/Three Point Perspective menus ----------

/// The saved preset a slot id (`ui.perspectiveUserPreset2.1`) stands for.
pub fn slot_preset(app: &VectorcraftApp, id: &str) -> Option<String> {
    let (kind, n) = id.strip_prefix(SLOT)?.split_once('.')?;
    let (kind, n): (u8, usize) = (kind.parse().ok()?, n.parse().ok()?);
    app.session.prefs.perspective_presets.iter().filter(|p| p.kind == kind).nth(n.checked_sub(1)?).map(|p| p.name.clone())
}

/// View → Perspective Grid → One/Two/Three Point Perspective (`kind` 1–3): the built-in views of
/// the type, then the saved presets' slots (`slots`, the type's ids).
pub fn menu(kind: u8, slots: [&'static str; SLOTS]) -> Vec<Node> {
    let mut items: Vec<Node> = BUILTINS
        .iter()
        .filter(|b| b.1 == kind)
        .enumerate()
        .map(|(i, b)| {
            // The normal view is the type's preset (`kind`), as before named presets.
            let p = if i == 0 { json!({ "kind": kind }) } else { json!({ "name": b.0 }) };
            cp(b.0, "perspective.grid.preset", p)
        })
        .collect();
    items.extend(slots.map(|id| c("User Preset", id)));
    items
}
