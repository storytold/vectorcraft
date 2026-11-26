//! Pattern Options panel: active in pattern editing mode (Object → Pattern → Make / Edit
//! Pattern). Name, Tile Type, Brick Offset, Width/Height with the W/H link, Size Tile to Art with
//! H/V spacing, Move Tile with Art, Overlap, Copies, Dim Copies, Show Tile Edge, Show Swatch
//! Bounds; Save a Copy / Done / Cancel. Outside editing mode it lists the document's patterns with
//! an Edit button.

use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Bracket, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::PanelRow;
use craft::ui::notices::{Notice, NoticeIcon, NoticeLayout};
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::LengthUnits;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::text::{NumberField, TextField, link_toggle};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::pattern::{PatternDef, RepeatKind, TileType};
use vectorcraft_doc::{NodeKind, Unit};

use super::attributes::side_name;
use crate::VectorcraftApp;
use crate::canvas::rulers::general_lengths;

const TILE_LABELS: [&str; 5] = ["Grid", "Brick by Row", "Brick by Column", "Hex by Column", "Hex by Row"];
const BRICK: [(&str, f64); 4] = [("1/2", 0.5), ("1/3", 1.0 / 3.0), ("1/4", 0.25), ("1/5", 0.2)];
const COPIES: [(&str, u32); 4] = [("3 x 3", 3), ("5 x 5", 5), ("7 x 7", 7), ("9 x 9", 9)];
/// The engine commands of the footer buttons and the panel menu, in order.
const COMMANDS: [&str; 3] = ["object.pattern.saveCopy", "object.pattern.done", "object.pattern.cancel"];

/// The Pattern Options panel's own state, kept while the app runs.
#[derive(Clone, Debug, Default)]
pub(crate) struct PatternOptionsPanel {
    /// The pattern picked to edit outside pattern editing mode (the first one until one is).
    pub(crate) chosen: String,
}

fn editing(app: &VectorcraftApp) -> Option<(PatternDef, Unit)> {
    let st = app.session.active()?;
    let pe = st.doc.pattern_edit.as_ref()?;
    Some((st.doc.pattern(&pe.pattern)?.clone(), app.session.general_unit()))
}

fn set(app: &mut VectorcraftApp, name: &str, mut p: Value) {
    p["name"] = json!(name);
    if let Err(e) = app.run("pattern.options", p) {
        app.status(e);
    }
}

/// Runs an engine command of the panel; its error shows in the status bar.
fn command(app: &mut VectorcraftApp, id: &str, params: Value) {
    if let Err(e) = app.run(id, params) {
        app.status(e);
    }
}

/// A row of the panel's form: `label` (a catalog string with its colon) names the row and, as
/// `body`'s second argument, its control.
fn row(ui: &mut Ui, label: &str, mut body: impl FnMut(&mut Ui, &str)) {
    let name = side_name(label);
    Field::new(name.clone()).show(ui, |ui, _| body(ui, &name));
}

/// A select over `labels` named `name`, showing item `current`; the index picked this pass. `salt`
/// keeps two selects of one name apart.
fn choose(ui: &mut Ui, name: &str, salt: &'static str, current: usize, labels: &[&'static str], enabled: bool) -> Option<usize> {
    let entries: Vec<Entry<usize>> = labels.iter().enumerate().map(|(i, l)| Entry::new(i, *l)).collect();
    let pick = Select::new(name.to_owned(), Bind::given(current), &entries).id_salt(salt).enabled(enabled).show(ui);
    pick.events.committed().copied()
}

/// A length field in the shown unit, kept in the document's: the length committed this pass.
fn length(ui: &mut Ui, name: &str, value: f64, lengths: LengthUnits, enabled: bool) -> Option<f64> {
    NumberField::length(name.to_owned(), Bind::given(Some(value)), lengths).enabled(enabled).show(ui).events.committed().copied().flatten()
}

/// A check box named `label`: the value it was set to this pass.
fn check(ui: &mut Ui, label: &'static str, on: bool) -> Option<bool> {
    Checkbox::new(Bind::given(on)).label(label).show(ui).events.committed().copied()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        Notice::new(Tone::Neutral, tl!("Open a document to edit its patterns."))
            .layout(NoticeLayout::Centred)
            .title(tl!("No document"))
            .icon(NoticeIcon::Custom(IconId::new("swatch-book").into()))
            .show(ui);
        return;
    }
    let Some((def, _)) = editing(app) else {
        idle(app, ui);
        return;
    };
    let mut name = def.name.clone();
    let lengths = general_lengths(app);
    // The patches of this pass, applied in order once the rows are drawn.
    let mut edits: Vec<Value> = Vec::new();
    let mut run_command: Option<&'static str> = None;
    let offset = match def.tile_type {
        TileType::BrickByRow { offset } | TileType::BrickByColumn { offset } => Some(offset),
        _ => None,
    };
    let sized = def.size_tile_to_art;
    // The Constrain Width and Height Proportions link: the preference the Transform panel's link sets.
    let linked = app.session.prefs.constrain_proportions;
    let mut link: Option<bool> = None;
    let canvas = crate::doc_windows::current(app).map(crate::canvas::canvas_id);
    FieldGroup::new(tl!("Pattern Options")).show(ui, |ui, _| {
        // Name commits on Enter or focus loss; Shift+Enter commits too and keeps the focus, so the
        // renamed pattern's name stays selected for the next edit. Enter ends the editing: the keys go
        // to the document canvas, as with nothing focused.
        row(ui, tl!("Name:"), |ui, label| {
            let edit = TextField::new(label.to_owned(), Bind::given(def.name.clone())).submits().select_on_focus().show(ui);
            if let Some(new) = edit.events.committed().map(|s| s.trim()).filter(|s| !s.is_empty() && *s != def.name) {
                edits.push(json!({ "newName": new }));
            }
            let next = if edit.submit_mods.shift { Some(edit.response.id) } else { canvas };
            if let Some(id) = next.filter(|_| edit.submitted) {
                focus::request(ui.ctx(), FocusTarget::Id(id));
            }
        });
        let tile = TileType::IDS.iter().position(|i| *i == def.tile_type.id()).unwrap_or(0);
        row(ui, tl!("Tile Type:"), |ui, label| {
            if let Some(id) = choose(ui, label, "po-tile", tile, &TILE_LABELS, true).and_then(|i| TileType::IDS.get(i)) {
                edits.push(json!({ "tileType": id }));
            }
        });
        let brick = BRICK.iter().position(|(_, v)| offset.is_some_and(|o| (o - v).abs() < 1e-6)).unwrap_or(0);
        row(ui, tl!("Brick Offset:"), |ui, label| {
            if let Some((_, v)) = choose(ui, label, "po-brick", brick, &BRICK.map(|(l, _)| l), offset.is_some()).and_then(|i| BRICK.get(i)) {
                edits.push(json!({ "brickOffset": v }));
            }
        });
        // Width and Height, with the link beside them, halfway between the two, and after them in Tab order. While it
        // is on, or when Ctrl/Cmd+Enter applies the value, a new width or height changes the other in proportion (#8566).
        Row::new().bracket(Bracket::Unlinked).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                for (key, label, value) in [("width", tl!("Width:"), def.tile.width()), ("height", tl!("Height:"), def.tile.height())] {
                    row(ui, label, |ui, label| {
                        let out = NumberField::length(label.to_owned(), Bind::given(Some(value)), lengths).enabled(!sized).show(ui);
                        let keep = linked || out.submit_mods.cmd;
                        edits.extend(out.events.committed().copied().flatten().map(|v| json!({ key: v, "proportional": keep })));
                    });
                }
            });
            link = link_toggle(tl!("Constrain Width and Height Proportions"), Bind::given(linked)).enabled(!sized).show(ui).value().copied();
        });
        edits.extend(check(ui, tl!("Size Tile to Art"), sized).map(|on| json!({ "sizeTileToArt": on })));
        edits.extend(check(ui, tl!("Move Tile with Art"), def.move_tile_with_art).map(|on| json!({ "moveTileWithArt": on })));
        for (key, label, value) in [("hSpacing", tl!("H Spacing:"), def.h_spacing), ("vSpacing", tl!("V Spacing:"), def.v_spacing)] {
            row(ui, label, |ui, label| edits.extend(length(ui, label, value, lengths, sized).map(|v| json!({ key: v }))));
        }
        row(ui, tl!("Overlap:"), |ui, label| {
            Row::new().show(ui, |ui, _| {
                let h = choose(ui, label, "po-oh", usize::from(def.overlap.right_in_front), &["Left in Front", "Right in Front"], true);
                edits.extend(h.map(|i| json!({ "overlap": { "h": if i == 1 { "right" } else { "left" } } })));
                let v = choose(ui, label, "po-ov", usize::from(def.overlap.bottom_in_front), &["Top in Front", "Bottom in Front"], true);
                edits.extend(v.map(|i| json!({ "overlap": { "v": if i == 1 { "bottom" } else { "top" } } })));
            });
        });
        Divider::horizontal().show(ui);
        let copies = COPIES.iter().position(|(_, n)| *n == def.copies).unwrap_or(1);
        row(ui, tl!("Copies:"), |ui, label| {
            if let Some((_, n)) = choose(ui, label, "po-copies", copies, &COPIES.map(|(l, _)| l), true).and_then(|i| COPIES.get(i)) {
                edits.push(json!({ "copies": n }));
            }
        });
        row(ui, tl!("Dim Copies to:"), |ui, label| {
            let edit = NumberField::new(label.to_owned(), Bind::given(Some(def.dim_copies as f64))).unit("%").show(ui);
            edits.extend(edit.events.committed().copied().flatten().map(|v| json!({ "dimCopies": v.round() })));
        });
        edits.extend(check(ui, tl!("Show Tile Edge"), def.show_tile_edge).map(|on| json!({ "showTileEdge": on })));
        edits.extend(check(ui, tl!("Show Swatch Bounds"), def.show_swatch_bounds).map(|on| json!({ "showSwatchBounds": on })));
        Divider::horizontal().show(ui);
        Row::new().show(ui, |ui, _| {
            for (label, id) in [tl!("Save a Copy"), tl!("Done"), tl!("Cancel")].into_iter().zip(COMMANDS) {
                if Button::new(label).show(ui).clicked() {
                    run_command = Some(id);
                }
            }
        });
    });
    for patch in edits {
        let renames = patch.get("newName").is_some();
        set(app, &name, patch);
        // Later patches of the pass (a blur that commits the name also clicks a box) name the pattern as it is now.
        if renames && let Some((now, _)) = editing(app) {
            name = now.name;
        }
    }
    if let Some(on) = link {
        super::transform::set_pref(app, "constrainProportions", on);
    }
    if let Some(id) = run_command {
        command(app, id, json!({}));
    }
}

/// Not editing: the document's patterns, one to pick and edit.
fn idle(app: &mut VectorcraftApp, ui: &mut Ui) {
    let names: Vec<String> = app.session.active().map(|st| st.doc.patterns.iter().map(|p| p.name.clone()).collect()).unwrap_or_default();
    Text::new(tl!("Pattern options are available in pattern editing mode.")).shade(Shade::Muted).show(ui);
    if names.is_empty() {
        Text::new(tl!("Select art and choose Object › Pattern › Make.")).shade(Shade::Muted).show(ui);
        return;
    }
    // A pick that is no longer in the list falls back to the first pattern.
    let current = names.iter().position(|n| *n == app.ui.pattern_options_panel.chosen).unwrap_or(0);
    let entries: Vec<Entry<usize>> = names.iter().enumerate().map(|(i, n)| Entry::new(i, n.clone())).collect();
    let mut edit = false;
    let mut picked = None;
    Row::new().show(ui, |ui, _| {
        let pick = Select::new(side_name(tl!("Name:")), Bind::given(current), &entries).show(ui);
        picked = pick.events.committed().and_then(|i| names.get(*i)).cloned();
        edit = Button::new(tl!("Edit Pattern")).show(ui).clicked();
    });
    if let Some(name) = picked {
        app.ui.pattern_options_panel.chosen = name;
    }
    if edit && let Some(name) = names.get(current) {
        command(app, "object.pattern.edit", json!({ "name": name }));
    }
}

/// The panel menu's rows: Save a Copy, Done and Cancel.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    COMMANDS.into_iter().map(|id| PanelRow::Command(CommandRef::new(id))).collect()
}

/// The rows of [`menu_rows`] work only while a pattern is being edited.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let editing = editing(app).is_some();
    for id in COMMANDS {
        states.set(CommandRef::new(id), CommandState::new().enabled(editing));
    }
}

/// The panel has no UI commands of its own: its menu rows are engine commands.
pub(crate) fn run(_app: &mut VectorcraftApp, _id: &str, _params: &Value) -> Option<Result<Value, String>> {
    None
}

/// Current values of the selected repeat as Repeat Options dialog fields.
pub fn repeat_fields(app: &VectorcraftApp) -> Option<Value> {
    let st = app.session.active()?;
    let spec = st.selection.objects.iter().find_map(|id| {
        st.doc.ancestry(*id)?.into_iter().rev().find_map(|a| match &st.doc.node(a)?.kind {
            NodeKind::Repeat(r) => Some(r.clone()),
            _ => None,
        })
    })?;
    Some(match spec.kind {
        RepeatKind::Radial { instances, radius, reverse_overlap, start_angle, end_angle, .. } => {
            json!({"instances": instances, "radius": radius, "startAngle": start_angle, "endAngle": end_angle, "reverseOverlap": reverse_overlap})
        }
        RepeatKind::Grid { h_spacing, v_spacing, rows, cols, grid_type, flip_rows, flip_cols } => {
            json!({"hSpacing": h_spacing, "vSpacing": v_spacing, "rows": rows, "cols": cols, "gridType": grid_type.id(), "flipRows": flip_rows, "flipCols": flip_cols})
        }
        RepeatKind::Mirror { angle, .. } => json!({"angle": angle}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use vectorcraft_engine::Session;

    /// A panel over a document with one pattern, "Dots", in editing mode.
    fn editing_panel() -> Panel {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("object.pattern.make", json!({"name": "Dots"})).unwrap();
        Panel::new(app, show)
    }

    /// Shift+Enter in the Name field renames the pattern and keeps the field active; Enter then
    /// renames it again and ends the editing.
    #[test]
    fn shift_enter_in_the_name_field_renames_the_pattern_and_keeps_the_focus() {
        let mut panel = editing_panel();
        let named = |panel: &Panel, name: &str| panel.app().session.active().unwrap().doc.pattern(name).is_some();
        panel.frame();
        panel.type_into_with("Name", "Stripes", egui::Modifiers::SHIFT);
        assert!(named(&panel, "Stripes"), "Shift+Enter applies the name");
        assert!(panel.focused("Name"), "the field keeps the focus");
        panel.type_into("Name", "Waves");
        assert!(named(&panel, "Waves"), "Enter applies the name");
        assert!(!panel.focused("Name"), "Enter ends the editing");
    }

    /// The Constrain Width and Height Proportions link sits right of the Width and Height fields,
    /// between them. A click turns it on (the preference the Transform panel's link sets), and then
    /// a new width changes the height in proportion (#8566). With the link off, Ctrl/Cmd+Enter in a
    /// field keeps the proportions too. While Size Tile to Art is on, the link is disabled with the
    /// fields.
    #[test]
    fn the_size_link_keeps_the_tile_proportions() {
        const LINK: &str = "Constrain Width and Height Proportions";
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("object.pattern.make", json!({"name": "Dots", "width": 20, "height": 10})).unwrap();
        let mut panel = Panel::new(app, show);
        let size = |panel: &Panel| {
            let tile = panel.app().session.active().unwrap().doc.pattern("Dots").unwrap().tile;
            (tile.width(), tile.height())
        };
        panel.frame();
        let (w, h, link) = (panel.rect("Width"), panel.rect("Height"), panel.rect(LINK));
        assert!(panel.enabled(LINK));
        assert!(link.left() >= w.right() && link.left() >= h.right(), "right of the fields: {link:?}, {w:?}, {h:?}");
        assert!(w.center().y < link.center().y && link.center().y < h.center().y, "between them: {link:?}, {w:?}, {h:?}");
        assert!(!panel.app().session.prefs.constrain_proportions);
        panel.click(LINK);
        assert!(panel.app().session.prefs.constrain_proportions, "the click turned the link on");
        panel.type_into("Width", "40");
        assert_eq!(size(&panel), (40.0, 20.0), "a new width changed the height in proportion");
        panel.app().run("prefs.set", json!({"key": "constrainProportions", "value": false})).unwrap();
        panel.type_into("Height", "30");
        assert_eq!(size(&panel), (40.0, 30.0), "with the link off a new height leaves the width");
        panel.type_into_with("Height", "15", egui::Modifiers::COMMAND);
        assert_eq!(size(&panel), (20.0, 15.0), "Ctrl/Cmd+Enter kept the proportions with the link off");
        panel.app().run("pattern.options", json!({"sizeTileToArt": true})).unwrap();
        panel.frame();
        assert!(!panel.enabled(LINK) && !panel.enabled("Width"), "disabled with Size Tile to Art");
    }
}
