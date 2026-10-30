//! Pathfinder panel: Shape Modes (+ Expand) and Pathfinders, with icons drawn for VectorCraft.
//! Alt-clicking a Shape Mode makes a compound shape; with members of one selected, a Shape Mode
//! sets theirs.

use egui::Ui;
use serde_json::json;
use vectorcraft_engine::doc::ShapeMode;

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

/// An operation's button, lit while `selected`, greyed out and saying what it needs unless the
/// selection has `min` operands.
fn button(ui: &mut Ui, icon: &str, tip: &str, selected: bool, n: usize, min: usize) -> bool {
    let resp = widgets::icon_button_enabled(ui, icon, tip, selected, n >= min, 34.0);
    if n < min {
        resp.on_hover_text(tl!("Select two or more objects, or a group"));
        return false;
    }
    resp.clicked()
}

/// A Shape Mode button clicked: Alt makes a compound shape of the selection in `mode`; with
/// members of compound shapes selected they take `mode`; otherwise operation `op` runs.
fn shape_mode(app: &mut VectorcraftApp, ui: &Ui, op: &str, label: &str, mode: ShapeMode, members: bool) {
    if ui.input(|i| i.modifiers.alt) {
        crate::menus::invoke(app, "object.compoundShape.make", json!({"mode": mode.name()}));
    } else if members {
        crate::menus::invoke(app, "object.compoundShape.setMode", json!({"mode": mode.name()}));
    } else {
        run(app, ui, op, label);
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let n = operands(app);
    // The modes of the compound-shape members selected: their Shape Mode button lights when they
    // share one.
    let modes = vectorcraft_engine::cmd::selected_member_modes(&app.session);
    let shared = modes.first().copied().filter(|m| modes.iter().all(|x| x == m));
    let expand = crate::menus::enabled(app, "object.compoundShape.expand");
    widgets::subheader(ui, tl!("Shape Modes:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        // Selected members can always take a mode.
        let count = if modes.is_empty() { n } else { n.max(2) };
        for ((icon, tip, op), mode) in SHAPE_MODES.into_iter().zip(ShapeMode::ALL) {
            if button(ui, icon, tip, shared == Some(mode), count, 2) {
                shape_mode(app, ui, op, tip, mode, !modes.is_empty());
            }
        }
        let resp = ui.add_enabled_ui(expand, |ui| widgets::flat_button_fit(ui, "Expand", 90.0)).inner;
        if resp.on_hover_text(tl!("Expand Compound Shape")).on_disabled_hover_text(tl!("Expand Compound Shape")).clicked() {
            crate::menus::invoke(app, "object.compoundShape.expand", json!({}));
        }
    });
    widgets::subheader(ui, tl!("Pathfinders:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (icon, tip, op) in PATHFINDERS {
            let min = if op == "outline" || op == "divide" { 1 } else { 2 };
            if button(ui, icon, tip, false, n, min) {
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
    if menu_item(ui, tl!("Pathfinder Options…"), true, false) {
        crate::menus::invoke(app, "object.pathfinder.options", json!({}));
    }
    ui.separator();
    for (label, id) in [
        (tl!("Make Compound Shape"), "object.compoundShape.make"),
        (tl!("Release Compound Shape"), "object.compoundShape.release"),
        (tl!("Expand Compound Shape"), "object.compoundShape.expand"),
    ] {
        if menu_item(ui, label, crate::menus::enabled(app, id), false) {
            crate::menus::invoke(app, id, json!({}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::accessibility_tests::{named, role_and_state, tree};
    use serde_json::Value;
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

    /// The panel menu's compound shape items: Make with two or more objects selected, Release and
    /// Expand with a compound shape (or a member of one); the Expand button follows Expand, and
    /// with a member selected its Shape Mode button is lit.
    #[test]
    fn compound_shape_items_follow_the_selection() {
        use egui::accesskit::{Role, Toggled};
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 50, "height": 50})).unwrap()["id"].clone();
        let state = |app: &mut VectorcraftApp| {
            let items = tree(|ui| menu(app, ui));
            let on = |label: &str| !named(&items, &format!("   {label}")).is_disabled();
            let panel = tree(|ui| show(app, ui));
            (on("Make Compound Shape"), on("Release Compound Shape"), on("Expand Compound Shape"), !named(&panel, "Expand").is_disabled())
        };
        app.run("select.set", json!({"ids": [a]})).unwrap();
        assert_eq!(state(&mut app), (false, false, false, false), "one object");
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        assert_eq!(state(&mut app), (true, false, false, false), "two objects");
        app.run("object.compoundShape.make", json!({"mode": "subtract"})).unwrap();
        assert_eq!(state(&mut app), (false, true, true, true), "a compound shape");
        app.run("select.set", json!({"ids": [b]})).unwrap();
        assert_eq!(state(&mut app), (false, true, true, true), "a member");
        let panel = tree(|ui| show(&mut app, ui));
        assert_eq!(role_and_state(&panel, "Minus Front"), (Role::Button, Some(Toggled::True)), "the member's mode");
        assert!(!named(&panel, "Unite").is_disabled(), "a member takes another mode");
        assert_ne!(role_and_state(&panel, "Unite").1, Some(Toggled::True));
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

    /// The panel menu's Pathfinder Options… is enabled and opens the options as they are; OK keeps
    /// them, and the next Divide leaves out the pieces of the object with no fill.
    #[test]
    fn pathfinder_options_open_and_ok_sets_what_the_next_divide_uses() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 100})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        let b = app.run("shape.rectangle", json!({"x": 60, "y": 30, "width": 100, "height": 50})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"none": true})).unwrap();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let items = tree(|ui| menu(&mut app, ui));
        assert!(!named(&items, "   Pathfinder Options…").is_disabled());
        crate::menus::invoke(&mut app, "object.pathfinder.options", json!({}));
        let d = app.ui.dialog.as_ref().expect("the Pathfinder Options");
        assert_eq!(d.kind, crate::dialogs::pathfinder_options::KIND);
        assert_eq!(Value::Object(d.fields.clone()), json!({"precision": 0.01, "removeRedundantPoints": false, "removeUnpainted": false}));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("removeUnpainted".into(), json!(true));
        d.fields.insert("precision".into(), json!(0.1));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let o = app.session.prefs.pathfinder;
        assert!(o.remove_unpainted && !o.remove_redundant_points && o.precision == 0.1, "{o:?}");
        let id = app.run("object.pathfinder.divide", json!({})).unwrap()["ids"][0].as_u64().unwrap();
        let group = app.session.active().unwrap().doc.node(NodeId(id)).unwrap();
        let pieces = group.children().unwrap();
        assert_eq!(pieces.len(), 1, "only the square's piece outside the unfilled rectangle");
        assert!(pieces.iter().all(|p| !p.appearance.fill_paint().is_none()));
    }
}
