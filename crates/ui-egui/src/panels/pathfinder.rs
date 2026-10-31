//! Pathfinder panel: Shape Modes (+ Expand) and Pathfinders, with icons drawn for VectorCraft.
//! Alt-clicking a Shape Mode makes a compound shape; with members of one selected, a Shape Mode
//! sets theirs.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_engine::doc::ShapeMode;

use super::selection_len;
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

/// Returns the minimum number of operands required by a Pathfinder operation.
pub(crate) fn minimum_operands(op: &str) -> usize {
    if matches!(op, "outline" | "divide") { 1 } else { 2 }
}

/// The panel name for one of the ten repeatable Shape Mode or Pathfinder operations.
pub fn operation_label(op: &str) -> Option<&'static str> {
    SHAPE_MODES.into_iter().chain(PATHFINDERS).find(|(_, _, id)| *id == op).map(|(_, label, _)| label)
}

fn run(app: &mut VectorcraftApp, op: &str) {
    // The command path reports a failed operation in the status bar.
    let _ = app.run(&format!("object.pathfinder.{op}"), json!({}));
}

/// Repeat the last successful Shape Mode or Pathfinder operation on the current selection.
pub fn repeat(app: &mut VectorcraftApp) -> Result<Value, String> {
    match app.last_pathfinder.clone() {
        Some(op) => {
            let min = minimum_operands(&op);
            if operands(app) < min {
                return Err("Select two or more objects, or a group".into());
            }
            app.run(&format!("object.pathfinder.{op}"), json!({}))
        }
        None => Err("no Pathfinder operation applied yet".into()),
    }
}

/// How many objects the selection gives an operation: a group selected alone counts as the shapes
/// the operation takes from it, nested groups included (#1450, #1635).
pub(crate) fn operands(app: &VectorcraftApp) -> usize {
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
fn shape_mode(app: &mut VectorcraftApp, ui: &Ui, op: &str, mode: ShapeMode, members: bool) {
    if ui.input(|i| i.modifiers.alt) {
        crate::menus::invoke(app, "object.compoundShape.make", json!({"mode": mode.name()}));
    } else if members {
        crate::menus::invoke(app, "object.compoundShape.setMode", json!({"mode": mode.name()}));
    } else {
        run(app, op);
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
                shape_mode(app, ui, op, mode, !modes.is_empty());
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
            let min = minimum_operands(op);
            if button(ui, icon, tip, false, n, min) {
                run(app, op);
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
    let label = app
        .last_pathfinder
        .as_ref()
        .and_then(|op| operation_label(op))
        .map(|label| crate::i18n::fmt(tl!("Repeat {name}"), &[("name", tl!(label))]))
        .unwrap_or_else(|| tl!("Repeat Pathfinder").into());
    if menu_item(ui, &label, crate::menus::enabled(app, "pathfinder.repeat"), false) {
        crate::menus::invoke(app, "pathfinder.repeat", json!({}));
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

    /// A document with two overlapping 50 pt squares, both selected.
    fn two_squares() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        select_two_squares(&mut app, 10.0);
        app
    }

    /// Draw two overlapping 50 pt squares, the first at (`x`, 10), and select only them.
    fn select_two_squares(app: &mut VectorcraftApp, x: f64) {
        let a = app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": x + 30.0, "y": 40, "width": 50, "height": 50})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
    }

    /// The kind and geometric bounds of each object on the first layer.
    fn objects(app: &VectorcraftApp) -> Vec<(&'static str, [f64; 4])> {
        let doc = &app.session.active().unwrap().doc;
        let kind = |n: &vectorcraft_engine::doc::Node| if matches!(n.kind, NodeKind::Path { .. }) { "path" } else { "other" };
        let bounds = |n: &vectorcraft_engine::doc::Node| n.geometric_bounds().map_or([0.0; 4], |b| [b.x0, b.y0, b.x1, b.y1]);
        doc.layers[0].children().unwrap().iter().map(|n| (kind(n), bounds(n))).collect()
    }

    /// Cmd/Ctrl+4 pressed through the shortcut handler.
    fn press_cmd_4(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        let ev = egui::Event::Key { key: egui::Key::Num4, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
        out.textures_delta.clear();
    }

    /// A click on the panel button that screen readers name `label`. egui finds what the pointer
    /// is over from the frame before, so the move gets a frame of its own.
    fn click_button(app: &mut VectorcraftApp, label: &str) {
        click_button_with_modifiers(app, label, egui::Modifiers::NONE);
    }

    /// Click a panel button with a modifier held across the input frames.
    fn click_button_with_modifiers(app: &mut VectorcraftApp, label: &str, modifiers: egui::Modifiers) {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install_fonts(&ctx);
        let mut time = 0.0;
        let mut frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            time += 0.1;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0));
            let mut events = events;
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            let input = egui::RawInput { events, time: Some(time), screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| show(app, ui));
            out.textures_delta.clear();
            out
        };
        frame(app, vec![]);
        let out = frame(app, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        let node = nodes.iter().find(|(_, n)| n.label() == Some(label) && n.role() == egui::accesskit::Role::Button);
        let b = node.unwrap_or_else(|| panic!("no button named {label:?}")).1.bounds().expect("bounds");
        let at = egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
        frame(app, vec![egui::Event::PointerMoved(at)]);
        frame(app, vec![button(true)]);
        frame(app, vec![button(false)]);
    }

    /// A click on the panel menu item labeled `label`.
    fn click_menu_item(app: &mut VectorcraftApp, label: &str) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(236.0, 400.0));
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| menu(app, ui));
            out.textures_delta.clear();
            crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
        };
        let labels = frame(app, vec![]);
        let at = labels.iter().find(|(t, _)| t.trim() == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {label:?} in {labels:?}"));
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(app, vec![egui::Event::PointerMoved(at), button(true)]);
        frame(app, vec![button(false)]);
    }

    /// Cmd/Ctrl+4 is Repeat Pathfinder's default key. Keyboard Shortcuts lists the command with
    /// it, and no other default has the key.
    #[test]
    fn cmd_4_is_repeat_pathfinder() {
        use crate::shortcut_editor::{command_for_key, default_command_shortcut};
        assert_eq!(default_command_shortcut("pathfinder.repeat"), Some("Cmd+4"));
        assert_eq!(default_command_shortcut("effect.applyLast"), Some("Cmd+Shift+E"));
        assert_eq!(command_for_key("Cmd+4"), Some("pathfinder.repeat"));
        let chord = crate::shortcuts::parse("Cmd+4").expect("parses");
        assert_eq!((chord.modifiers, chord.logical_key), (egui::Modifiers::COMMAND, egui::Key::Num4));
        assert_eq!(crate::menus::pretty_shortcut("Cmd+4"), if cfg!(target_os = "macos") { "⌘4" } else { "Ctrl+4" });
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let listed = app.run("shortcuts.list", json!({"query": "pathfinder.repeat"})).unwrap();
        let rows: Vec<_> = listed.as_array().expect("rows").iter().map(|r| (r["label"].clone(), r["default"].clone())).collect();
        assert_eq!(rows, [(json!("Repeat Pathfinder"), json!("Cmd+4"))], "Keyboard Shortcuts lists it");
        assert_eq!(app.run("shortcuts.conflicts", json!({})).unwrap(), json!([]), "no other default has the key");
        assert_eq!(vectorcraft_engine::command_specs().iter().filter(|c| c.shortcut == Some("Cmd+4")).count(), 0);
        assert_eq!(crate::menus::UI_COMMANDS.iter().filter(|c| c.2 == "Cmd+4").map(|c| c.0).collect::<Vec<_>>(), ["pathfinder.repeat"]);
    }

    /// After Unite from the panel, Cmd/Ctrl+4 unites the two squares selected next, and the panel
    /// menu's Repeat Unite item, which runs the same command, unites a third pair.
    #[test]
    fn cmd_4_repeats_the_operation_a_panel_button_ran_last() {
        let mut app = two_squares();
        click_button(&mut app, "Unite");
        let first = ("path", [10.0, 10.0, 90.0, 90.0]);
        assert_eq!(objects(&app), [first], "the panel's Unite merged the first pair");
        select_two_squares(&mut app, 110.0);
        assert!(crate::menus::enabled(&app, "pathfinder.repeat"));
        press_cmd_4(&mut app);
        let second = ("path", [110.0, 10.0, 190.0, 90.0]);
        assert_eq!(objects(&app), [first, second], "Cmd+4 merged the second pair");
        select_two_squares(&mut app, 210.0);
        click_menu_item(&mut app, "Repeat Unite");
        assert_eq!(objects(&app), [first, second, ("path", [210.0, 10.0, 290.0, 90.0])], "the menu item merged the third pair");
    }

    /// Before a repeatable operation has run, Repeat Pathfinder is disabled and Cmd/Ctrl+4 leaves
    /// the selected squares as they are, with a message in the status bar. After an operation has
    /// run, the command is disabled again while nothing is selected.
    #[test]
    fn cmd_4_does_nothing_before_an_operation_has_run() {
        let mut app = two_squares();
        let before = objects(&app);
        assert_eq!(before.len(), 2);
        assert!(!crate::menus::enabled(&app, "pathfinder.repeat"));
        press_cmd_4(&mut app);
        assert_eq!(objects(&app), before, "Cmd+4 changed nothing");
        assert_eq!(app.ui.status, "no Pathfinder operation applied yet");
        click_button(&mut app, "Unite");
        app.run("select.none", json!({})).unwrap();
        assert!(!crate::menus::enabled(&app, "pathfinder.repeat"), "nothing selected");
        assert!(app.run("pathfinder.repeat", json!({})).is_err());
    }

    /// One selected path cannot supply the operands for Unite; the shortcut reports the failure.
    #[test]
    fn repeat_with_one_path_keeps_art_and_history() {
        let mut app = two_squares();
        click_button(&mut app, "Unite");
        let before = objects(&app);
        assert_eq!(before.len(), 1);
        assert!(!crate::menus::enabled(&app, "pathfinder.repeat"));
        press_cmd_4(&mut app);
        assert_eq!(objects(&app), before);
        assert_eq!(app.ui.status, "Select two or more objects, or a group");
        assert_eq!(app.last_pathfinder.as_deref(), Some("unite"));
    }

    /// A direct command replaces the panel's previous operation and supplies the menu name.
    #[test]
    fn direct_operation_becomes_the_one_repeated() {
        let mut app = two_squares();
        click_button(&mut app, "Unite");
        select_two_squares(&mut app, 110.0);
        app.run("object.pathfinder.intersect", json!({})).unwrap();
        assert_eq!(app.last_pathfinder.as_deref(), Some("intersect"));
        select_two_squares(&mut app, 210.0);
        click_menu_item(&mut app, "Repeat Intersect");
        assert_eq!(objects(&app).last(), Some(&("path", [240.0, 40.0, 260.0, 60.0])));
        assert_eq!(app.last_pathfinder.as_deref(), Some("intersect"));
    }

    /// A successful batch records its last repeatable Pathfinder step.
    #[test]
    fn batch_pathfinder_step_becomes_the_one_repeated() {
        let mut app = two_squares();
        click_button(&mut app, "Intersect");
        select_two_squares(&mut app, 110.0);
        app.run(
            "command.batch",
            json!({"commands": [
                {"command": "object.pathfinder.unite", "params": {}},
                {"command": "select.none", "params": {}}
            ]}),
        )
        .unwrap();
        assert_eq!(app.last_pathfinder.as_deref(), Some("unite"));
        select_two_squares(&mut app, 210.0);
        press_cmd_4(&mut app);
        assert_eq!(objects(&app).last(), Some(&("path", [210.0, 10.0, 290.0, 90.0])));
    }

    /// A batch rollback leaves the previous Pathfinder operation available.
    #[test]
    fn failed_batch_pathfinder_step_keeps_history() {
        let mut app = two_squares();
        click_button(&mut app, "Unite");
        select_two_squares(&mut app, 110.0);
        let before = objects(&app);
        assert!(
            app.run(
                "command.batch",
                json!({"commands": [
                    {"command": "object.pathfinder.intersect", "params": {}},
                    {"command": "unknown.command", "params": {}}
                ]})
            )
            .is_err()
        );
        assert_eq!(app.last_pathfinder.as_deref(), Some("unite"));
        assert_eq!(objects(&app), before);
        press_cmd_4(&mut app);
        assert_eq!(objects(&app).last(), Some(&("path", [110.0, 10.0, 190.0, 90.0])));
    }

    /// Playing an action records its Pathfinder step for the repeat shortcut.
    #[test]
    fn actions_panel_play_records_pathfinder_step() {
        let mut app = two_squares();
        click_button(&mut app, "Intersect");
        app.ui.action_sets[0]
            .actions
            .push(crate::panels::actions::Action { name: "Unite".into(), steps: vec![("object.pathfinder.unite".into(), json!({}))] });
        let idx = app.ui.action_sets[0].actions.len() - 1;
        select_two_squares(&mut app, 110.0);
        crate::panels::actions::play(&mut app, 0, idx).unwrap();
        assert_eq!(app.last_pathfinder.as_deref(), Some("unite"));
        select_two_squares(&mut app, 210.0);
        press_cmd_4(&mut app);
        assert_eq!(objects(&app).last(), Some(&("path", [210.0, 10.0, 290.0, 90.0])));
    }

    /// A rejected operation leaves the prior successful operation available to repeat.
    #[test]
    fn failed_operation_keeps_history() {
        let mut app = two_squares();
        click_button(&mut app, "Unite");
        assert!(app.run("object.pathfinder.intersect", json!({"precision": -1.0})).is_err());
        assert_eq!(app.last_pathfinder.as_deref(), Some("unite"));
        select_two_squares(&mut app, 110.0);
        click_menu_item(&mut app, "Repeat Unite");
        assert_eq!(objects(&app).last(), Some(&("path", [110.0, 10.0, 190.0, 90.0])));
    }

    /// A successful Trap keeps the last repeatable operation in history.
    #[test]
    fn successful_trap_keeps_history() {
        let mut app = two_squares();
        click_button(&mut app, "Unite");
        let mut rect = |x: f64, color: &str| {
            let id = app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
            app.run("paint.setFill", json!({"color": color})).unwrap();
            id
        };
        let (a, b) = (rect(110.0, "#ffff00"), rect(160.0, "#0000ff"));
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        app.run("object.pathfinder.trap", json!({"thickness": 2, "tintReduction": 30})).unwrap();
        assert_eq!(app.last_pathfinder.as_deref(), Some("unite"));
    }

    /// Repeat reads the current options instead of replaying the earlier command's settings.
    #[test]
    fn repeat_uses_current_pathfinder_options() {
        let mut app = two_squares();
        app.run("object.pathfinder.divide", json!({})).unwrap();
        assert_eq!(app.last_pathfinder.as_deref(), Some("divide"));
        app.run("object.pathfinder.options", json!({"precision": 1.0})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": 0, "y": 99.5, "width": 100, "height": 100})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let id = app.run("pathfinder.repeat", json!({})).unwrap()["ids"][0].as_u64().unwrap();
        let group = app.session.active().unwrap().doc.node(NodeId(id)).unwrap();
        assert_eq!(group.children().unwrap().len(), 2);
        assert_eq!(app.last_pathfinder.as_deref(), Some("divide"));
    }

    /// Compound-shape creation and member edits leave destructive Pathfinder history alone.
    #[test]
    fn compound_shape_commands_do_not_set_history() {
        let mut app = two_squares();
        let a = app.session.active().unwrap().selection.objects[0];
        click_button_with_modifiers(&mut app, "Minus Front", egui::Modifiers::ALT);
        assert_eq!(app.last_pathfinder, None);
        app.run("select.set", json!({"ids": [a.0]})).unwrap();
        click_button(&mut app, "Unite");
        assert_eq!(app.last_pathfinder, None);
        assert!(!crate::menus::enabled(&app, "pathfinder.repeat"));
    }
}
