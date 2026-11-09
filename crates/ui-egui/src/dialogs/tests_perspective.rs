//! Perspective Grid dialogs: Define Grid, the presets manager and the tool options.

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_tools::distort::perspective::PerspectiveGrid;

use crate::{VectorcraftApp, theme};

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app
}

fn kind(app: &VectorcraftApp) -> Option<&str> {
    app.ui.dialog.as_ref().map(|d| d.kind.as_str())
}

fn field(app: &VectorcraftApp, k: &str) -> Value {
    app.ui.dialog.as_ref().and_then(|d| d.fields.get(k).cloned()).unwrap_or_default()
}

fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
    app.ui.dialog.as_mut().expect("a dialog is open").fields.insert(k.into(), v);
}

fn grid(app: &VectorcraftApp) -> PerspectiveGrid {
    PerspectiveGrid::effective(&app.session.active().unwrap().doc)
}

#[test]
fn define_grid_opens_on_the_grid_and_applies_its_fields() {
    let mut app = app();
    app.run("perspective.grid.preset", json!({"kind": 3})).unwrap();
    let g = grid(&app);
    app.run("ui.perspectiveGridDialog", json!({})).unwrap();
    assert_eq!(kind(&app), Some(super::perspective_grid::KIND));
    assert_eq!(field(&app, "kind"), json!(3), "prefilled from the grid, not a fixed type");
    assert!((field(&app, "gridline").as_f64().unwrap() - g.cell).abs() < 1e-9);
    frame(&mut app);
    // A new unit converts the lengths shown.
    set(&mut app, "units", json!("inches"));
    frame(&mut app);
    assert!((field(&app, "gridline").as_f64().unwrap() - g.cell / 72.0).abs() < 1e-9);
    // OK with nothing changed leaves the grid as it was.
    super::confirm(&mut app).unwrap();
    assert_eq!(kind(&app), None);
    assert_eq!((grid(&app).cell, grid(&app).vp_left, grid(&app).kind), (g.cell, g.vp_left, 3));
    // A new viewing angle and colour apply in one undo step.
    let undo = app.session.active().unwrap().history.undo.len();
    app.run("ui.perspectiveGridDialog", json!({})).unwrap();
    set(&mut app, "angle", json!(30));
    set(&mut app, "rightColor", json!("#112233"));
    frame(&mut app);
    super::confirm(&mut app).unwrap();
    let n = grid(&app);
    assert!((n.viewing_angle() - 30.0).abs() < 1e-9);
    assert_eq!(n.right_color.hex(), "#112233");
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    // A refused value keeps the dialog open.
    app.run("ui.perspectiveGridDialog", json!({})).unwrap();
    set(&mut app, "distance", json!(-5));
    assert!(super::confirm(&mut app).is_err());
    assert_eq!(kind(&app), Some(super::perspective_grid::KIND));
}

/// A three-digit code pasted or typed into a Define Grid color field sets that grid color.
#[test]
fn define_grid_takes_a_three_digit_color() {
    let mut app = app();
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    // One frame of the dialog layer: every text drawn, with its center.
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::show(app, ui.ctx()));
        out.textures_delta.clear();
        fn collect(s: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match s {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect().center())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let mut texts = vec![];
        out.shapes.iter().for_each(|c| collect(&c.shape, &mut texts));
        texts
    };
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // The context keeps a key down until it is let go, and a dialog ignores a repeat of Enter.
    let enter = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    for (how, input) in [("pasted", egui::Event::Paste("0f0".into())), ("typed", egui::Event::Text("0f0".into()))] {
        app.run("ui.perspectiveGridDialog", json!({})).unwrap();
        set(&mut app, "leftColor", json!("#123456"));
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let texts = frame(&mut app, vec![]);
        let at = texts.iter().find(|(t, _)| t == "123456").unwrap_or_else(|| panic!("no Left Grid field in {texts:?}")).1;
        frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true)]);
        frame(&mut app, vec![button(at, false)]);
        frame(&mut app, vec![]);
        frame(&mut app, vec![input]);
        frame(&mut app, vec![enter(true)]);
        frame(&mut app, vec![enter(false)]);
        assert_eq!(kind(&app), None, "{how}: Enter closes the dialog");
        assert_eq!(grid(&app).left_color.hex(), "#00ff00", "{how}: the grid takes the color");
    }
}

#[test]
fn define_grid_is_a_view_menu_item() {
    let app = app();
    let entries = crate::menus::menu_entries(&app);
    let e = entries.iter().find(|e| e.command.as_deref() == Some("ui.perspectiveGridDialog")).expect("View › Perspective Grid › Define Grid…");
    assert!(e.path.iter().any(|p| p == "Perspective Grid"), "{:?}", e.path);
    assert!(e.enabled);
}

#[test]
fn define_grid_loads_and_saves_presets() {
    let mut app = app();
    app.run("ui.perspectiveGridDialog", json!({})).unwrap();
    frame(&mut app);
    assert_eq!(field(&app, "name"), json!("[2P-Normal View]"));
    // Choosing a preset (an agent sets its fields; the menu loads them) keeps its name while the
    // fields are its; a changed field makes the fields [Custom].
    set(&mut app, "angle", json!(20));
    frame(&mut app);
    assert_eq!(field(&app, "name"), json!(""));
    // Save Preset… asks for a name; OK saves the fields and comes back to Define Grid on it.
    app.ui.dialog.as_mut().unwrap().fields.insert("__mode".into(), json!("save"));
    set(&mut app, "__from", json!("define"));
    set(&mut app, "name", json!("Shallow"));
    frame(&mut app);
    super::confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some(super::perspective_grid::KIND));
    assert_eq!((field(&app, "name"), field(&app, "__mode")), (json!("Shallow"), Value::Null));
    assert_eq!(app.session.prefs.perspective_presets[0].angle, 20.0);
    frame(&mut app);
    assert_eq!(field(&app, "name"), json!("Shallow"), "the fields are the saved preset's");
    super::confirm(&mut app).unwrap();
    assert_eq!(grid(&app).name, "Shallow");
    // View › Perspective Grid › Save Grid as Preset…: the grid under a new name.
    app.run("ui.savePerspectivePreset", json!({})).unwrap();
    assert_eq!(field(&app, "name"), json!("Perspective Preset 1"));
    frame(&mut app);
    super::confirm(&mut app).unwrap();
    assert_eq!(kind(&app), None);
    assert_eq!(app.session.prefs.perspective_presets.len(), 2);
}

#[test]
fn the_presets_manager_creates_edits_and_deletes_presets() {
    use super::perspective_presets::{self, Action};
    let mut app = app();
    app.run("ui.perspectivePresetsDialog", json!({"selected": "[3P-Normal View]"})).unwrap();
    frame(&mut app);
    let press = |app: &mut VectorcraftApp, act: Action, current: &str| {
        let mut d = app.ui.dialog.take().unwrap();
        perspective_presets::run(app, &mut d, act, current).unwrap();
        app.ui.dialog = Some(d);
        frame(app);
    };
    // New… opens the editor on a copy of the selected preset; its OK comes back here.
    press(&mut app, Action::New, "[3P-Normal View]");
    assert_eq!((kind(&app), field(&app, "kind")), (Some(super::perspective_grid::KIND), json!(3)));
    set(&mut app, "name", json!("Canyon"));
    set(&mut app, "opacity", json!(90));
    super::confirm(&mut app).unwrap();
    assert_eq!((kind(&app), field(&app, "selected")), (Some(perspective_presets::KIND), json!("Canyon")));
    // Edit… renames it.
    press(&mut app, Action::Edit, "Canyon");
    set(&mut app, "name", json!("Gorge"));
    super::confirm(&mut app).unwrap();
    assert_eq!(field(&app, "selected"), json!("Gorge"));
    assert_eq!(app.session.prefs.perspective_presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Gorge"]);
    assert_eq!(app.session.prefs.perspective_presets[0].opacity, 90.0);
    // Its type's menu lists it in a slot, which applies it.
    assert_eq!(crate::menus::dynamic_label(&app, "ui.perspectiveUserPreset3.1", ""), "Gorge");
    assert!(
        ["ui.perspectiveUserPreset3.2", "ui.perspectiveUserPreset2.1"]
            .iter()
            .all(|id| crate::menus::shown_state(&app, id, &serde_json::Value::Null).is_none())
    );
    app.run("ui.perspectiveUserPreset3.1", json!({})).unwrap();
    assert_eq!(grid(&app).name, "Gorge");
    // Delete selects the one above it.
    press(&mut app, Action::Delete, "Gorge");
    assert!(app.session.prefs.perspective_presets.is_empty());
    assert_eq!(field(&app, "selected"), json!("[3P-Low View]"));
    assert!(app.run("ui.perspectiveUserPreset3.1", json!({})).is_err());
}

#[test]
fn perspective_preset_menus_list_the_views_of_each_type() {
    let app = app();
    let entries = crate::menus::menu_entries(&app);
    let under = |sub: &str| entries.iter().filter(|e| e.path.iter().any(|p| p == sub)).map(|e| e.label.clone()).collect::<Vec<_>>();
    assert_eq!(under("One Point Perspective"), ["[1P-Normal View]", "[1P-Low View]", "[1P-High View]"]);
    assert_eq!(under("Three Point Perspective"), ["[3P-Normal View]", "[3P-Low View]"]);
    let presets = entries.iter().find(|e| e.command.as_deref() == Some("ui.perspectivePresetsDialog")).unwrap();
    assert_eq!((presets.label.as_str(), presets.path.first().map(String::as_str)), ("Perspective Grid Presets…", Some("Edit")));
}

#[test]
fn perspective_grid_view_toggles_flip_their_labels_and_checks() {
    use crate::menus::{checked, dynamic_label};
    let mut app = app();
    assert_eq!(dynamic_label(&app, "perspective.grid.show", ""), "Show Grid");
    assert_eq!(checked(&app, "perspective.grid.snap", &Value::Null), Some(true), "Snap to Grid is on by default");
    app.run("perspective.grid.show", json!({})).unwrap();
    app.run("perspective.grid.rulers", json!({})).unwrap();
    app.run("perspective.grid.lock", json!({})).unwrap();
    app.run("perspective.grid.lockStation", json!({})).unwrap();
    app.run("perspective.grid.snap", json!({})).unwrap();
    assert_eq!(
        ["perspective.grid.show", "perspective.grid.rulers", "perspective.grid.lock"].map(|id| dynamic_label(&app, id, "")),
        ["Hide Grid", "Hide Rulers", "Unlock Grid"]
    );
    assert_eq!(checked(&app, "perspective.grid.lockStation", &Value::Null), Some(true));
    assert_eq!(checked(&app, "perspective.grid.snap", &Value::Null), Some(false));
    let entries = crate::menus::menu_entries(&app);
    let labels: Vec<&str> = entries.iter().filter(|e| e.path.last().is_some_and(|p| p == "Perspective Grid")).map(|e| e.label.as_str()).collect();
    assert_eq!(&labels[..6], ["Hide Grid", "Hide Rulers", "Snap to Grid", "Unlock Grid", "Lock Station Point", "Define Grid…"]);
}

#[test]
fn double_clicking_the_tool_opens_perspective_grid_options() {
    let mut app = app();
    let r = app.run("tool.options", json!({"tool": "perspectiveGrid"})).unwrap();
    assert_eq!(r["dialog"], json!(super::perspective_options::KIND));
    assert_eq!((field(&app, "show"), field(&app, "position")), (json!(true), json!("topLeft")));
    frame(&mut app);
    set(&mut app, "position", json!("bottomLeft"));
    frame(&mut app);
    super::confirm(&mut app).unwrap();
    assert_eq!(kind(&app), None);
    assert_eq!(app.session.prefs.perspective_widget.position.id(), "bottomLeft");
    app.run("tool.options", json!({"tool": "perspectiveGrid"})).unwrap();
    set(&mut app, "show", json!(false));
    super::confirm(&mut app).unwrap();
    assert!(!app.session.prefs.perspective_widget.show);
}

#[test]
fn digit_keys_switch_planes_and_the_widget_follows_the_window() {
    let mut app = app();
    app.run("perspective.grid.preset", json!({"kind": 2})).unwrap();
    let key = |app: &mut VectorcraftApp, k: egui::Key| {
        let ctx = egui::Context::default();
        let press = egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        let input = egui::RawInput { events: vec![press], ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| crate::shortcuts::handle(app, ui.ctx()));
        out.textures_delta.clear();
    };
    key(&mut app, egui::Key::Num3);
    assert_eq!(grid(&app).plane.id(), "right");
    key(&mut app, egui::Key::Num2);
    assert_eq!(grid(&app).plane.id(), "ground");
    // The window the widget stays in: none before the canvas is laid out.
    app.canvas_rect = Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0)));
    let f = app.screen_frame().unwrap();
    assert_eq!(f.size, (800.0, 600.0));
    let z = app.view().unwrap().zoom;
    assert!((f.right.x - 1.0 / z).abs() < 1e-9 && f.right.y.abs() < 1e-9);
    assert_eq!(app.view_info().screen, Some(f));
}
