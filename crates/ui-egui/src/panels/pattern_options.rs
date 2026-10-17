//! Pattern Options panel: active in pattern editing mode (Object → Pattern → Make / Edit
//! Pattern). Name, Tile Type, Brick Offset, Width/Height, Size Tile to Art with H/V spacing,
//! Overlap, Copies, Dim Copies, Show Tile Edge, Show Swatch Bounds; Save a Copy / Done / Cancel. Outside editing
//! mode it lists the document's patterns with an Edit button.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::pattern::{PatternDef, RepeatKind, TileType};
use vectorcraft_doc::{NodeKind, Unit};

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

const TILE_LABELS: [&str; 5] = ["Grid", "Brick by Row", "Brick by Column", "Hex by Column", "Hex by Row"];
const BRICK: [(&str, f64); 4] = [("1/2", 0.5), ("1/3", 1.0 / 3.0), ("1/4", 0.25), ("1/5", 0.2)];
const COPIES: [(&str, u32); 4] = [("3 x 3", 3), ("5 x 5", 5), ("7 x 7", 7), ("9 x 9", 9)];

fn editing(app: &VectorcraftApp) -> Option<(PatternDef, Unit)> {
    let st = app.session.active()?;
    let pe = st.doc.pattern_edit.as_ref()?;
    Some((st.doc.pattern(&pe.pattern)?.clone(), app.session.general_unit()))
}

fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    widgets::label_row(ui, label, 96.0, add);
}

fn set(app: &mut VectorcraftApp, name: &str, mut p: Value) {
    p["name"] = json!(name);
    if let Err(e) = app.run("pattern.options", p) {
        app.status(e);
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        super::empty_state(ui, "swatch-book", tl!("No document"), tl!("Open a document to edit its patterns."));
        return;
    }
    let Some((def, unit)) = editing(app) else {
        idle(app, ui);
        return;
    };
    let name = def.name.clone();
    // Name (commit on Enter / focus loss; Shift+Enter commits and keeps the focus).
    row(ui, tl!("Name:"), |ui| {
        let id = ui.id().with("po-name");
        let mut buf: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| name.clone());
        if !ui.memory(|m| m.has_focus(id)) {
            buf = name.clone();
        }
        let kept = widgets::own_field_shift_enter(ui, id, &mut buf, &name);
        let r = ui.add(egui::TextEdit::singleline(&mut buf).id(id).desired_width(150.0));
        ui.data_mut(|d| d.insert_temp(id, buf.clone()));
        if (r.lost_focus() || kept) && !buf.trim().is_empty() && buf != name {
            set(app, &name, json!({"newName": buf.trim()}));
        }
    });
    let tt_index = TileType::IDS.iter().position(|i| *i == def.tile_type.id()).unwrap_or(0);
    row(ui, tl!("Tile Type:"), |ui| {
        if let Some(i) = widgets::dropdown(ui, "po-tile", TILE_LABELS[tt_index], &TILE_LABELS, 150.0) {
            set(app, &name, json!({"tileType": TileType::IDS[i]}));
        }
    });
    let offset = match def.tile_type {
        TileType::BrickByRow { offset } | TileType::BrickByColumn { offset } => Some(offset),
        _ => None,
    };
    row(ui, tl!("Brick Offset:"), |ui| {
        ui.add_enabled_ui(offset.is_some(), |ui| {
            let cur = BRICK.iter().find(|(_, v)| offset.is_some_and(|o| (o - v).abs() < 1e-6)).map(|(l, _)| *l).unwrap_or("1/2");
            let labels: Vec<&str> = BRICK.iter().map(|(l, _)| *l).collect();
            if let Some(i) = widgets::dropdown(ui, "po-brick", cur, &labels, 70.0) {
                set(app, &name, json!({"brickOffset": BRICK[i].1}));
            }
        });
    });
    let sized = def.size_tile_to_art;
    row(ui, tl!("Width:"), |ui| {
        ui.add_enabled_ui(!sized, |ui| {
            if let Some(v) = widgets::num_field(ui, "po-w", Some(def.tile.width()), unit, 90.0) {
                set(app, &name, json!({"width": v}));
            }
        });
    });
    row(ui, tl!("Height:"), |ui| {
        ui.add_enabled_ui(!sized, |ui| {
            if let Some(v) = widgets::num_field(ui, "po-h", Some(def.tile.height()), unit, 90.0) {
                set(app, &name, json!({"height": v}));
            }
        });
    });
    if widgets::check(ui, tl!("Size Tile to Art"), sized, true) {
        set(app, &name, json!({"sizeTileToArt": !sized}));
    }
    row(ui, tl!("H Spacing:"), |ui| {
        ui.add_enabled_ui(sized, |ui| {
            if let Some(v) = widgets::num_field(ui, "po-hs", Some(def.h_spacing), unit, 90.0) {
                set(app, &name, json!({"hSpacing": v}));
            }
        });
    });
    row(ui, tl!("V Spacing:"), |ui| {
        ui.add_enabled_ui(sized, |ui| {
            if let Some(v) = widgets::num_field(ui, "po-vs", Some(def.v_spacing), unit, 90.0) {
                set(app, &name, json!({"vSpacing": v}));
            }
        });
    });
    row(ui, tl!("Overlap:"), |ui| {
        let h = if def.overlap.right_in_front { "Right in Front" } else { "Left in Front" };
        if let Some(i) = widgets::dropdown(ui, "po-oh", h, &["Left in Front", "Right in Front"], 110.0) {
            set(app, &name, json!({"overlap": {"h": if i == 1 { "right" } else { "left" }}}));
        }
    });
    row(ui, "", |ui| {
        let v = if def.overlap.bottom_in_front { "Bottom in Front" } else { "Top in Front" };
        if let Some(i) = widgets::dropdown(ui, "po-ov", v, &["Top in Front", "Bottom in Front"], 110.0) {
            set(app, &name, json!({"overlap": {"v": if i == 1 { "bottom" } else { "top" }}}));
        }
    });
    widgets::divider(ui);
    row(ui, tl!("Copies:"), |ui| {
        let cur = COPIES.iter().find(|(_, n)| *n == def.copies).map(|(l, _)| *l).unwrap_or("5 x 5");
        let labels: Vec<&str> = COPIES.iter().map(|(l, _)| *l).collect();
        if let Some(i) = widgets::dropdown(ui, "po-copies", cur, &labels, 80.0) {
            set(app, &name, json!({"copies": COPIES[i].1}));
        }
    });
    row(ui, tl!("Dim Copies to:"), |ui| {
        if let Some(v) = widgets::plain_field(ui, "po-dim", def.dim_copies as f64, "%", 0, 60.0) {
            set(app, &name, json!({"dimCopies": v}));
        }
    });
    if widgets::check(ui, tl!("Show Tile Edge"), def.show_tile_edge, true) {
        set(app, &name, json!({"showTileEdge": !def.show_tile_edge}));
    }
    if widgets::check(ui, tl!("Show Swatch Bounds"), def.show_swatch_bounds, true) {
        set(app, &name, json!({"showSwatchBounds": !def.show_swatch_bounds}));
    }
    widgets::divider(ui);
    ui.horizontal(|ui| {
        if widgets::flat_button(ui, tl!("Save a Copy"), 90.0).clicked()
            && let Err(e) = app.run("object.pattern.saveCopy", json!({}))
        {
            app.status(e);
        }
        if widgets::flat_button(ui, tl!("Done"), 60.0).clicked() {
            app.run("object.pattern.done", json!({})).ok();
        }
        if widgets::flat_button(ui, tl!("Cancel"), 60.0).clicked() {
            app.run("object.pattern.cancel", json!({})).ok();
        }
    });
}

/// Not editing: the document's patterns, one to pick and edit.
fn idle(app: &mut VectorcraftApp, ui: &mut Ui) {
    let names: Vec<String> = app.session.active().map(|st| st.doc.patterns.iter().map(|p| p.name.clone()).collect()).unwrap_or_default();
    widgets::dim_label(ui, tl!("Pattern options are available in pattern editing mode."));
    ui.add_space(4.0);
    if names.is_empty() {
        widgets::dim_label(ui, tl!("Select art and choose Object › Pattern › Make."));
        return;
    }
    let mut chosen: String = pstate(ui.ctx(), "po-chosen");
    if !names.contains(&chosen) {
        chosen = names[0].clone();
    }
    ui.horizontal(|ui| {
        let labels: Vec<&str> = names.iter().map(String::as_str).collect();
        if let Some(i) = widgets::dropdown_names(ui, "po-pick", &chosen, &labels, 150.0) {
            set_pstate(ui.ctx(), "po-chosen", names[i].clone());
        }
        if widgets::flat_button(ui, tl!("Edit Pattern"), 90.0).clicked()
            && let Err(e) = app.run("object.pattern.edit", json!({"name": chosen}))
        {
            app.status(e);
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let editing = editing(app).is_some();
    if menu_item(ui, tl!("Save a Copy"), editing, false) {
        app.run("object.pattern.saveCopy", json!({})).ok();
    }
    if menu_item(ui, tl!("Done"), editing, false) {
        app.run("object.pattern.done", json!({})).ok();
    }
    if menu_item(ui, tl!("Cancel"), editing, false) {
        app.run("object.pattern.cancel", json!({})).ok();
    }
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
    use vectorcraft_engine::Session;

    /// One headless frame of the panel with `events`: where each text was drawn.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        let mut v = vec![];
        for c in &out.shapes {
            if let egui::Shape::Text(t) = &c.shape {
                v.push((t.galley.text().to_string(), t.visual_bounding_rect()));
            }
        }
        v
    }

    /// Shift+Enter in the Name field renames the pattern and keeps the field active, the new name
    /// selected; Enter then renames it again and ends the editing.
    #[test]
    fn shift_enter_in_the_name_field_renames_the_pattern_and_keeps_the_focus() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("object.pattern.make", json!({"name": "Dots"})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let named = |app: &VectorcraftApp, name: &str| app.session.active().unwrap().doc.pattern(name).is_some();
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let texts = frame(&mut app, &ctx, vec![]);
        let at = texts.iter().find(|(t, _)| t == "Dots").expect("the Name field").1.center();
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), click(true)]);
        frame(&mut app, &ctx, vec![click(false)]);
        frame(&mut app, &ctx, vec![key(egui::Key::A, egui::Modifiers::COMMAND), egui::Event::Text("Stripes".into())]);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)]);
        assert!(named(&app, "Stripes"), "Shift+Enter applies the name");
        let field = ctx.memory(|m| m.focused()).expect("the field keeps the focus");
        frame(&mut app, &ctx, vec![]);
        let selected = egui::TextEdit::load_state(&ctx, field).and_then(|st| st.cursor.char_range()).map(|r| r.as_sorted_char_range());
        assert_eq!(selected.map(|r| (r.start.0, r.end.0)), Some((0, 7)), "the name shown is selected");
        frame(&mut app, &ctx, vec![egui::Event::Text("Waves".into())]);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(named(&app, "Waves"), "typing replaced it, and Enter applies that");
        assert!(ctx.memory(|m| m.focused()).is_none(), "Enter ends the editing");
    }

    #[test]
    fn show_swatch_bounds_toggles_the_pattern_option() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("object.pattern.make", json!({"name": "Dots"})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let texts = frame(&mut app, &ctx, vec![]);
        let at = texts.iter().find(|(t, _)| t == "Show Swatch Bounds").expect("the checkbox").1.center();
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), click(true)]);
        frame(&mut app, &ctx, vec![click(false)]);
        assert!(app.session.active().unwrap().doc.pattern("Dots").unwrap().show_swatch_bounds);
    }
}
