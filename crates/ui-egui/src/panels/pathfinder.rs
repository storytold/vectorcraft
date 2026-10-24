//! Pathfinder panel: Shape Modes (+ Expand) and Pathfinders, with icons drawn for VectorCraft.

use egui::Ui;
use serde_json::json;

use super::{pstate, selection_len, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

pub const SHAPE_MODES: [(&str, &str, &str); 4] = [
    ("dc-pf-unite", "Unite", "unite"),
    ("dc-pf-minus-front", "Minus Front", "minusFront"),
    ("dc-pf-intersect", "Intersect", "intersect"),
    ("dc-pf-exclude", "Exclude", "exclude"),
];

pub const PATHFINDERS: [(&str, &str, &str); 6] = [
    ("dc-pf-divide", "Divide", "divide"),
    ("dc-pf-trim", "Trim", "trim"),
    ("dc-pf-merge", "Merge", "merge"),
    ("dc-pf-crop", "Crop", "crop"),
    ("dc-pf-outline", "Outline", "outline"),
    ("dc-pf-minus-back", "Minus Back", "minusBack"),
];

fn run(app: &mut VectorcraftApp, ui: &Ui, op: &str, label: &str) {
    if app.run(&format!("object.pathfinder.{op}"), json!({})).is_ok() {
        set_pstate(ui.ctx(), "pf-last", Some((op.to_string(), label.to_string())));
    }
}

/// How many objects the selection gives an operation: a group selected alone counts as the shapes
/// the operation takes from it, nested groups included (#1450, #1635).
fn operands(app: &VectorcraftApp) -> usize {
    let alone = app.session.active().filter(|d| d.selection.len() == 1).and_then(|d| d.doc.node(*d.selection.objects.first()?));
    match alone.filter(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Group { .. })) {
        Some(group) => vectorcraft_engine::cmd::shape_count(group),
        None => selection_len(app),
    }
}

/// An operation's button, greyed out and saying what it needs unless the selection has `min` operands.
fn button(ui: &mut Ui, icon: &str, tip: &str, n: usize, min: usize) -> bool {
    let resp = widgets::icon_button_enabled(ui, icon, tip, false, n >= min, 34.0);
    if n < min {
        resp.on_hover_text(tl!("Select two or more objects, or a group"));
        return false;
    }
    resp.clicked()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let n = operands(app);
    let t = crate::theme::Tokens::get(ui.ctx());
    widgets::subheader(ui, tl!("Shape Modes:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (icon, tip, op) in SHAPE_MODES {
            if button(ui, icon, tip, n, 2) {
                run(app, ui, op, tip);
            }
        }
        ui.add_enabled_ui(false, |ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(90.0), 26.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 2, t.hover.gamma_multiply(0.5));
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, tl!("Expand"), egui::FontId::proportional(12.5), t.text_disabled);
        })
        .response
        .on_disabled_hover_text(tl!("Expand applies to compound shapes (Alt-click a shape mode) — on the roadmap"));
    });
    widgets::subheader(ui, tl!("Pathfinders:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (icon, tip, op) in PATHFINDERS {
            let min = if op == "outline" || op == "divide" { 1 } else { 2 };
            if button(ui, icon, tip, n, min) {
                run(app, ui, op, tip);
            }
        }
    });
}

/// Pathfinder panel › Trap…: the Trap dialog (`object.pathfinder.trap`'s options) for two or more
/// objects.
pub fn open_trap(app: &mut VectorcraftApp) {
    let params = vectorcraft_effects::trap::command_defaults();
    let _ = app.run("ui.paramDialog", json!({"command": "object.pathfinder.trap", "label": "Trap", "params": params}));
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    if menu_item(ui, tl!("Trap…"), operands(app) >= 2, false) {
        open_trap(app);
    }
    let last: Option<(String, String)> = pstate(ui.ctx(), "pf-last");
    let label =
        last.as_ref().map(|(_, l)| crate::i18n::fmt(tl!("Repeat {name}"), &[("name", tl!(l))])).unwrap_or_else(|| tl!("Repeat Pathfinder").into());
    if menu_item(ui, &label, last.is_some() && selection_len(app) >= 1, false)
        && let Some((op, l)) = last
    {
        run(app, ui, &op, &l);
    }
    menu_item(ui, tl!("Pathfinder Options…"), false, false);
    ui.separator();
    menu_item(ui, tl!("Make Compound Shape"), false, false);
    menu_item(ui, tl!("Release Compound Shape"), false, false);
    menu_item(ui, tl!("Expand Compound Shape"), false, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::accessibility_tests::{named, tree};
    use vectorcraft_engine::doc::{NodeId, NodeKind};

    /// #1635: a stroked object expanded inside a group leaves a group holding one group (its fill
    /// and its outlined stroke). The Shape Modes take the group's shapes, so they stay enabled, and
    /// Unite merges the fill and the stroke into one path.
    #[test]
    fn shape_modes_take_the_shapes_of_an_expanded_stroke_in_a_group() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        app.run("stroke.set", json!({"weight": 10})).unwrap();
        app.run("object.group", json!({})).unwrap();
        app.run("object.expand", json!({})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        let group = doc.node(*app.session.active().unwrap().selection.objects.first().unwrap()).unwrap();
        let members = group.children().unwrap();
        assert_eq!(members.len(), 1, "one member: the expanded rectangle");
        assert!(matches!(members[0].kind, NodeKind::Group { .. }), "the fill and the outlined stroke, grouped");
        assert_eq!(operands(&app), 2, "the fill and the outlined stroke");
        let id = app.run("object.pathfinder.unite", json!({})).unwrap()["ids"][0].as_u64().unwrap();
        let united = app.session.active().unwrap().doc.node(NodeId(id)).unwrap();
        assert!(matches!(united.kind, NodeKind::Path { .. }), "one path, the stroke's inner edge gone: {:?}", united.kind);
        let b = united.geometric_bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (5.0, 5.0, 65.0, 65.0), "the stroke's outer edge");
    }

    /// Screen readers name each Shape Mode and Pathfinder button, enabled for two selected objects.
    #[test]
    fn screen_readers_name_the_buttons() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 50, "height": 50})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let tree = tree(|ui| show(&mut app, ui));
        for (_, label, _) in SHAPE_MODES.into_iter().chain(PATHFINDERS) {
            let node = named(&tree, label);
            assert_eq!(node.role(), egui::accesskit::Role::Button, "{label}");
            assert!(!node.is_disabled(), "{label}");
        }
    }

    /// The panel menu's Trap… is enabled for two objects and opens the Trap options; OK adds the
    /// trap above them as one selected group in one undo step.
    #[test]
    fn trap_opens_its_options_and_ok_traps_the_selection() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let mut rect = |x: f64, color: &str| {
            let id = app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
            app.run("paint.setFill", json!({"color": color})).unwrap();
            id
        };
        let (a, b) = (rect(10.0, "#ffff00"), rect(60.0, "#0000ff"));
        app.run("select.set", json!({"ids": [a]})).unwrap();
        // Menu rows pad their label where a check mark would go.
        let one = tree(|ui| menu(&mut app, ui));
        assert!(named(&one, "   Trap…").is_disabled(), "one object has nothing to trap with");
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let two = tree(|ui| menu(&mut app, ui));
        assert!(!named(&two, "   Trap…").is_disabled());
        open_trap(&mut app);
        let d = app.ui.dialog.as_mut().expect("the Trap options");
        assert_eq!(d.str("__command"), "object.pathfinder.trap");
        for key in ["thickness", "heightWidth", "tintReduction", "trapsWithProcessColor", "reverseTraps"] {
            assert!(d.fields.contains_key(key), "{key}");
        }
        d.fields.insert("thickness".into(), json!(2));
        let undo = app.session.doc().unwrap().history.undo.len();
        crate::dialogs::confirm(&mut app).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let group = st.doc.node(*st.selection.objects.first().unwrap()).unwrap();
        let trap = &group.children().unwrap()[0];
        let bb = trap.geometric_bounds().unwrap();
        assert!((bb.x0 - 60.0).abs() < 0.02 && (bb.x1 - 62.0).abs() < 0.02, "2 pt into the blue: {bb:?}");
        assert!(trap.appearance.fill().is_some_and(|f| f.overprint));
    }
}
