//! Edit → Perspective Grid Presets…: every preset in a list (the built-in views, protected, then
//! the saved ones) with the selected one's settings beside it; New… (a preset starting from the
//! selected one), Edit… (saved presets), Delete, Import… and Export… (the selected preset) under
//! the list. New… and Edit… open the preset editor (the Define Grid dialog, mode `edit`), whose OK
//! comes back here.
//!
//! Also View → Perspective Grid → One/Two/Three Point Perspective: the built-in views of the type
//! and slots for the saved presets of that type ([`SLOT`]).
//!
//! Field: `selected` (a preset's name).

use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::perspgrid::{PRESET_EXTS, PRESET_FORMAT};
use vectorcraft_tools::distort::perspective::GridDefinition;
use vectorcraft_tools::distort::perspective::define::{BUILTINS, is_builtin};

use super::DialogSpec;
use crate::menus::Item;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Perspective Grid Presets.
pub const KIND: &str = "perspectiveGridPresets";

/// The id prefix of the saved-preset slots in the One/Two/Three Point Perspective menus:
/// `ui.perspectiveUserPreset<kind>.<n>` is the n-th saved preset of that type.
pub const SLOT: &str = "ui.perspectiveUserPreset";

/// Saved-preset slots per type.
pub const SLOTS: usize = 5;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Perspective Grid Presets").into(),
    body,
    confirm: |app, _| {
        app.ui.dialog = None;
        Ok(Value::Null)
    },
    ok: None,
    min_width: 600.0,
    max_width: Some(600.0),
    ..DialogSpec::FORM
};

/// Open the presets manager on `selected` (default: the first preset).
pub fn open(app: &mut VectorcraftApp, selected: Option<&str>) {
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "selected": selected.unwrap_or(BUILTINS[0].0) })));
}

/// What a button asked for, done once the list and the details are drawn.
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

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let presets = app.session.perspective_presets();
    let selected = d.str("selected");
    let i = presets.iter().position(|p| p.name.eq_ignore_ascii_case(selected.trim())).unwrap_or(0);
    let Some(preset) = presets.get(i) else { return false };
    let builtin = is_builtin(&preset.name);
    let mut act = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(260.0);
            widgets::dim_label(ui, tl!("Presets:"));
            widgets::list_box(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("perspective-presets").min_scrolled_height(240.0).max_height(240.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(240.0);
                    for (k, p) in presets.iter().enumerate() {
                        let shown = if is_builtin(&p.name) { tl!(&p.name) } else { p.name.as_str() };
                        if ui.selectable_label(k == i, shown).clicked() {
                            act = Some(Action::Select(p.name.clone()));
                        }
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if widgets::flat_button(ui, tl!("New…"), 48.0).on_hover_text(tl!("A new preset starting from the selected one")).clicked() {
                    act = Some(Action::New);
                }
                if ui.add_enabled_ui(!builtin, |ui| widgets::flat_button(ui, tl!("Edit…"), 48.0)).inner.clicked() {
                    act = Some(Action::Edit);
                }
                if ui.add_enabled_ui(!builtin, |ui| widgets::flat_button(ui, tl!("Delete"), 52.0)).inner.clicked() {
                    act = Some(Action::Delete);
                }
                if widgets::flat_button(ui, tl!("Import…"), 60.0).clicked() {
                    act = Some(Action::Import);
                }
                if widgets::flat_button(ui, tl!("Export…"), 60.0).on_hover_text(tl!("Save the selected preset to a file")).clicked() {
                    act = Some(Action::Export);
                }
            });
        });
        ui.add_space(18.0);
        ui.vertical(|ui| {
            ui.set_width(280.0);
            ui.label(egui::RichText::new(if builtin { tl!(&preset.name) } else { preset.name.as_str() }).color(t.text_strong).strong());
            ui.add_space(8.0);
            egui::Grid::new("perspective-preset-details").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                for (label, value) in details(preset) {
                    ui.label(egui::RichText::new(label).color(t.text_dim));
                    ui.label(egui::RichText::new(value).color(t.text));
                    ui.end_row();
                }
            });
            if builtin {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tl!("Built-in presets don't change: New… starts an editable copy.")).color(t.text_dim).size(11.5));
            }
        });
    });
    let name = preset.name.clone();
    if let Some(a) = act {
        let r = match a {
            // Their file dialogs, shown off the UI thread, import into or export from the dialog
            // as it is then.
            Action::Import | Action::Export => {
                let (current, export) = (name.clone(), matches!(a, Action::Export));
                crate::picks::in_dialog(app, d, move |app, d| run(app, d, if export { Action::Export } else { Action::Import }, &current))
            }
            a => run(app, d, a, &name),
        };
        if let Err(e) = r {
            app.status(e);
        }
    }
    false
}

fn select(d: &mut Dialog, name: &str) {
    d.fields.insert("selected".into(), json!(name));
}

pub(super) fn run(app: &mut VectorcraftApp, d: &mut Dialog, act: Action, current: &str) -> Result<(), String> {
    match act {
        Action::Select(name) => select(d, &name),
        // The editor replaces this dialog; its OK comes back here.
        Action::New | Action::Edit => {
            let original = matches!(act, Action::Edit).then_some(current);
            super::perspective_grid::open_edit(app, original, current)?;
            *d = app.ui.dialog.take().ok_or("no preset editor")?;
        }
        Action::Delete => {
            let i = app.session.prefs.perspective_presets.iter().position(|p| p.name == current);
            app.run("perspective.presets.delete", json!({ "name": current }))?;
            // The one above it takes its place (the last built-in view above the first saved one).
            let saved = &app.session.prefs.perspective_presets;
            let next =
                i.and_then(|i| i.checked_sub(1)).and_then(|i| saved.get(i)).map_or(BUILTINS[BUILTINS.len() - 1].0, |p| p.name.as_str()).to_string();
            select(d, &next);
        }
        Action::Import => {
            // On the web the picked file arrives later and imports through `io::open_bytes`.
            if let Some(f) = app.services.open_async.as_mut() {
                f();
                return Ok(());
            }
            let pick = crate::FilePick { filters: vec![("Perspective grid presets", PRESET_EXTS)], ..Default::default() };
            let path = crate::picks::open(app, &pick).ok_or("cancelled")?;
            let r = app.run("perspective.presets.import", json!({ "path": path }))?;
            if let Some(first) = r["imported"].get(0).and_then(Value::as_str) {
                select(d, first);
            }
        }
        Action::Export => {
            crate::io::save_command_output(app, "perspective.presets.export", PRESET_FORMAT, json!({ "names": [current] }))?;
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
pub fn menu(kind: u8, slots: [&'static str; SLOTS]) -> Vec<Item> {
    let mut items: Vec<Item> = BUILTINS
        .iter()
        .filter(|b| b.1 == kind)
        .enumerate()
        .map(|(i, b)| {
            // The normal view is the type's preset (`kind`), as before named presets.
            let p = if i == 0 { json!({ "kind": kind }) } else { json!({ "name": b.0 }) };
            Item::Cmd(b.0, "perspective.grid.preset", p)
        })
        .collect();
    items.extend(slots.map(|id| Item::Cmd("User Preset", id, Value::Null)));
    items
}
