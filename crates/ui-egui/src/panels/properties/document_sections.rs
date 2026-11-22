//! The Properties panel's Document sections, shown with nothing selected.

use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Sizing};
use craft::ui::form::Field;
use craft::ui::layout_components::{Grid, Row};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::LengthUnits;
use craft::ui::sections::Section;
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::json;
use vectorcraft_doc::Unit;

use super::artboard_sections::rearrange_button;
use super::object_sections::HALVES;
use crate::VectorcraftApp;
use crate::panels::transform;

/// The panel with nothing selected: the document's units, Document Setup and artboards, the
/// Appearance fill and stroke, the view's rulers, grids, guides and snapping, the keyboard
/// increment and Scale Strokes & Effects, and Quick Actions.
pub(super) fn document_sections(app: &mut VectorcraftApp, ui: &mut Ui) {
    let units = app.session.general_unit();
    let artboards = app.session.active().map_or(0, |d| d.doc.artboards.len());
    Section::fixed(tl!("Document")).show(ui, |ui, _| {
        Field::new(tl!("Units")).show(ui, |ui, _| {
            let entries: Vec<Entry<Unit>> = Unit::ALL.iter().map(|u| Entry::new(*u, u.label())).collect();
            let out = Select::new(tl!("Units"), Bind::given(units), &entries).show(ui);
            if let Some(u) = out.value() {
                app.run("document.setUnits", json!({ "units": u.label() })).ok();
            }
        });
        Grid::new(&HALVES).show(ui, |ui, _| {
            Button::command(CommandRef::new("file.documentSetup")).sizing(Sizing::FillWidth).show(ui);
            Button::command(CommandRef::with_params("tool.select", json!({ "tool": "artboard" })))
                .label(tl!("Edit Artboards"))
                .sizing(Sizing::FillWidth)
                .show(ui);
            if artboards > 1 {
                rearrange_button(ui, artboards);
            }
        });
    });
    Section::fixed(tl!("Appearance")).show(ui, |ui, _| super::appearance_section::fill_stroke_rows(app, ui));
    Section::fixed(tl!("Rulers & Grids")).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            // The checked and enabled states and the tooltips come from the commands (Trim View turns the grid off).
            for (command, icon) in [("view.rulers", "ruler"), ("view.grid", "grid-3x3"), ("view.transparencyGrid", "square-dashed")] {
                ToggleButton::command(CommandRef::new(command)).icon(IconId::new(icon)).icon_only().show(ui);
            }
        });
    });
    Section::fixed(tl!("Guides")).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            for (command, icon) in [("view.guides", "layout-grid"), ("view.smartGuides", "sparkles")] {
                ToggleButton::command(CommandRef::new(command)).icon(IconId::new(icon)).icon_only().show(ui);
            }
        });
    });
    Section::fixed(tl!("Snap Options")).show(ui, |ui, _| {
        let (point, grid) = (app.ui.view.snap_to_point, app.ui.view.snap_to_grid);
        for (command, label, on) in [("view.snapToPoint", tl!("Snap to Point"), point), ("view.snapToGrid", tl!("Snap to Grid"), grid)] {
            if Checkbox::new(Bind::given(on)).label(label).show(ui).value().is_some() {
                app.run(command, json!({})).ok();
            }
        }
    });
    Section::fixed(tl!("Preferences")).show(ui, |ui, _| {
        Field::new(tl!("Keyboard Increment")).show(ui, |ui, _| {
            // PLAN: assumed `canvas::rulers::general_lengths(app) -> LengthUnits` (cv.B5b)
            let lengths: LengthUnits = crate::canvas::rulers::general_lengths(app);
            // The engine's own range for the preference (`prefscmds.rs`): a larger value is an error.
            let out = NumberField::length(tl!("Keyboard Increment"), Bind::given(Some(app.session.prefs.keyboard_increment)), lengths)
                .range(0.001..=1296.0)
                .show(ui);
            crate::edits::apply_edits(app, &out.edit, crate::edits::EditPath::Scope, |app, v: &Option<f64>, _| {
                if let Some(value) = *v {
                    app.run("prefs.set", json!({ "key": "keyboardIncrement", "value": value })).ok();
                }
            });
        });
        let scale = app.session.prefs.scale_strokes;
        let out = Checkbox::new(Bind::given(scale)).label(tl!("Scale Strokes & Effects")).show(ui);
        if let Some(on) = out.value().copied() {
            transform::set_pref(app, "scaleStrokes", on);
        }
    });
    Section::fixed(tl!("Quick Actions")).show(ui, |ui, _| {
        Grid::new(&HALVES).show(ui, |ui, _| {
            for command in ["file.documentSetup", "edit.preferences"] {
                Button::command(CommandRef::new(command)).sizing(Sizing::FillWidth).show(ui);
            }
        });
    });
}
