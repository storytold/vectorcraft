//! Unit Converter's submenu and panel: one field per unit, typing in one updates the others and
//! text that does not read leaves them alone, From Selection reads the selection's size, Apply
//! sets the side picked as one undo step and adds none when nothing changes, and Copy writes the
//! value with or without its unit.

use egui::{Event, Key, Modifiers, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_doc::Unit;
use vectorcraft_engine::Session;

use super::{Converter, ID, STATE, Side, apply, document_now, from_selection, show};
use crate::panels::{pstate, set_pstate};
use crate::tests_labels::{painted_text, shapes_text};
use crate::{VectorcraftApp, menus};

/// An app with a selected 72 × 36 pt rectangle.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 72, "height": 36})).unwrap();
    app
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// The selection's width and height.
fn size(app: &mut VectorcraftApp) -> (f64, f64) {
    let r = app.run("addon.units.selectionSize", json!({})).unwrap();
    (r["width"]["points"].as_f64().unwrap(), r["height"]["points"].as_f64().unwrap())
}

#[test]
fn the_submenu_opens_the_panel() {
    let mut app = app();
    let entries = menus::menu_entries(&app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Unit Converter"] && e.label == "Unit Converter Panel").expect("listed");
    menus::invoke(&mut app, "window.panel", item.params.clone());
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel opens");
}

#[test]
fn the_panel_has_a_field_for_every_unit_and_its_buttons() {
    let mut app = app();
    let text = painted_text(&mut app, |app, ui| {
        set_pstate(ui.ctx(), STATE, Converter { points: Some(720.0 / 25.4), ..Converter::default() });
        show(app, ui);
    });
    for u in Unit::EAST_ASIAN {
        assert!(text.lines().any(|l| l == u.label()), "{} in {text}", u.label());
    }
    for s in ["10 mm", "1 cm", "0.3937 in", "28.346 pt", "40 Q", "40 H"] {
        assert!(text.lines().any(|l| l == s), "{s} in {text}");
    }
    for s in ["Copy with Units", "Selection", "Width", "Height", "From Selection", "Apply"] {
        assert!(text.lines().any(|l| l == s), "{s} in {text}");
    }
}

#[test]
fn typing_in_a_field_updates_the_others_and_text_that_does_not_read_leaves_them() {
    let mut app = app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut time = 0.0;
    let mut frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
        time += 0.05;
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 800.0)));
        let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), screen_rect, focused: true, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        (pstate::<Converter>(&ctx, STATE), shapes_text(&out))
    };
    let tab = Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    // Tab reaches the first field, Points, which selects its text so typing replaces it.
    frame(&mut app, vec![tab]);
    frame(&mut app, vec![]);
    let (st, _) = frame(&mut app, vec![Event::Text("1in/2".into())]);
    assert!(st.points.is_some_and(|pt| near(pt, 36.0)), "{st:?}");
    let (st, text) = frame(&mut app, vec![]);
    assert!(near(st.points.unwrap(), 36.0));
    for s in ["0.5 in", "3 p", "12.7 mm", "1.27 cm"] {
        assert!(text.lines().any(|l| l == s), "{s} in {text}");
    }
    // Text that does not read keeps the length the other fields show.
    let (st, text) = frame(&mut app, vec![Event::Text("x".into())]);
    assert!(near(st.points.unwrap(), 36.0), "{st:?}");
    assert!(text.lines().any(|l| l == "12.7 mm"), "{text}");
    // So does a length too large to show in Q and Ha.
    let select_all = Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
    frame(&mut app, vec![select_all]);
    // 1.7e308 pt: finite in points.
    let (st, _) = frame(&mut app, vec![Event::Text(format!("17{}", "0".repeat(307)))]);
    assert!(near(st.points.unwrap(), 36.0), "{st:?}");
}

#[test]
fn a_number_typed_without_a_unit_in_the_picas_row_is_in_picas() {
    let mut app = app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // Numbers Without Units Are Points is on (its default), and stays on for other fields.
    crate::widgets::set_bare_numbers_are_points(&ctx, true);
    let mut time = 0.0;
    let mut frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
        time += 0.05;
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 800.0)));
        let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), screen_rect, focused: true, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        pstate::<Converter>(&ctx, STATE)
    };
    let tab = Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    // Tab reaches the Points field, then the Picas field (the copy buttons are off with no length).
    for _ in 0..2 {
        frame(&mut app, vec![tab.clone()]);
        frame(&mut app, vec![]);
    }
    let st = frame(&mut app, vec![Event::Text("2".into())]);
    assert!(st.points.is_some_and(|pt| near(pt, 24.0)), "{st:?}");
    assert_eq!(crate::widgets::typed_unit(&ctx, Unit::Picas), Unit::Points, "other fields still read points");
}

#[test]
fn picking_a_side_shows_that_side_of_the_size_read() {
    let mut st = Converter { size: Some((72.0, 36.0)), ..Converter::default() };
    st.pick(Side::Height);
    assert_eq!((st.side, st.points), (Side::Height, Some(36.0)));
    st.pick(Side::Width);
    assert_eq!(st.points, Some(72.0));
    // A length typed after From Selection replaces the side shown, and picking the other side and
    // back keeps it.
    st.set(100.0);
    st.pick(Side::Height);
    assert_eq!(st.points, Some(36.0));
    st.pick(Side::Width);
    assert_eq!((st.points, st.size), (Some(100.0), Some((100.0, 36.0))));
    // Before From Selection a side keeps the length typed.
    let mut st = Converter::default();
    st.set(5.0);
    st.pick(Side::Height);
    assert_eq!((st.points, st.size), (Some(5.0), None));
}

#[test]
fn copy_writes_the_value_with_or_without_its_unit() {
    let mut st = Converter { points: Some(72.0), ..Converter::default() };
    assert_eq!(st.copied(Unit::Inches).as_deref(), Some("1"));
    st.with_units = true;
    assert_eq!(st.copied(Unit::Inches).as_deref(), Some("1 in"));
    assert_eq!(st.copied(Unit::Millimeters), Some(Unit::Millimeters.format(72.0)));
    assert_eq!(Converter::default().copied(Unit::Points), None, "nothing to copy before a length");
}

/// Apply `st` in a frame whose shared reference point (the Transform panel's) is `reference`.
fn apply_at(app: &mut VectorcraftApp, st: &Converter, reference: usize) {
    try_apply_at(app, st, reference).unwrap();
}

/// [`apply_at`], with its result.
fn try_apply_at(app: &mut VectorcraftApp, st: &Converter, reference: usize) -> Result<(), String> {
    let ctx = egui::Context::default();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("refpt"), reference));
    let mut result = Ok(());
    ctx.run_ui(Default::default(), |ui| result = apply(app, ui, st)).textures_delta.clear();
    result
}

/// The selection's bounding box.
fn bounds(app: &mut VectorcraftApp) -> vectorcraft_geom::Rect {
    app.selection_box().unwrap().rect
}

#[test]
fn from_selection_reads_the_size_and_apply_sets_the_side_picked() {
    let mut app = app();
    let mut st = Converter::default();
    from_selection(&mut app, &mut st).unwrap();
    assert_eq!((st.size, st.points), (Some((72.0, 36.0)), Some(72.0)));
    st.pick(Side::Height);
    assert_eq!(st.points, Some(36.0));
    // Apply: 1 in high, from the centre, as one undo step.
    let steps = undo_steps(&app);
    st.points = Some(72.0);
    apply_at(&mut app, &st, 4);
    let (w, h) = size(&mut app);
    assert!(near(w, 72.0) && near(h, 72.0), "{w} × {h}");
    assert_eq!(undo_steps(&app), steps + 1);
    // The same length again changes nothing and adds no step.
    apply_at(&mut app, &st, 4);
    assert_eq!(undo_steps(&app), steps + 1);
    // Without a length Apply does nothing.
    apply_at(&mut app, &Converter::default(), 4);
    assert_eq!(undo_steps(&app), steps + 1);
    // Width, kept at the left edge (reference 3).
    let x0 = bounds(&mut app).x0;
    let st = Converter { points: Some(144.0), side: Side::Width, ..Converter::default() };
    apply_at(&mut app, &st, 3);
    let b = bounds(&mut app);
    assert!(near(b.width(), 144.0) && near(b.x0, x0), "{b:?}");
    assert_eq!(undo_steps(&app), steps + 2);
}

/// Apply follows the Transform panel's W and H fields: Constrain Width and Height Proportions
/// scales the other side too, and Transform Pattern Only leaves the object's size alone.
#[test]
fn apply_follows_the_transform_panels_proportions_and_pattern_choice() {
    let mut app = app();
    app.run("prefs.set", json!({"key": "constrainProportions", "value": true})).unwrap();
    apply_at(&mut app, &Converter { points: Some(144.0), side: Side::Width, ..Converter::default() }, 4);
    let (w, h) = size(&mut app);
    assert!(near(w, 144.0) && near(h, 72.0), "{w} × {h}");
    app.run("prefs.set", json!({"key": "constrainProportions", "value": false})).unwrap();
    app.run("prefs.set", json!({"key": "transformPanelPatterns", "value": "pattern"})).unwrap();
    // The object has no pattern to transform: whether the command reports that or not, the
    // object keeps its size.
    let _ = try_apply_at(&mut app, &Converter { points: Some(10.0), side: Side::Width, ..Converter::default() }, 4);
    let (w, h) = size(&mut app);
    assert!(near(w, 144.0) && near(h, 72.0), "the object keeps its size: {w} × {h}");
    app.run("prefs.set", json!({"key": "transformPanelPatterns", "value": "both"})).unwrap();
    apply_at(&mut app, &Converter { points: Some(10.0), side: Side::Width, ..Converter::default() }, 4);
    assert!(near(size(&mut app).0, 10.0));
}

/// With Use Registration Point for Symbol on and one symbol instance selected, Apply keeps the
/// symbol's registration point (here its top-left corner) in place, as the Transform panel does,
/// instead of the shared reference point (the centre).
#[test]
fn apply_keeps_a_symbol_instances_registration_point() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 50, "width": 80, "height": 40})).unwrap();
    let id = app.run("symbol.new", json!({"name": "Corner", "registration": 0})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("prefs.set", json!({"key": "useRegistrationPointForSymbol", "value": true})).unwrap();
    apply_at(&mut app, &Converter { points: Some(160.0), side: Side::Width, ..Converter::default() }, 4);
    assert_eq!(bounds(&mut app), vectorcraft_geom::Rect::new(100.0, 50.0, 260.0, 90.0));
}

/// The size From Selection read belongs to the selection it was read from: after the selection
/// changes, picking a side keeps the length shown instead of showing the old object's side, and
/// Apply sets that length.
#[test]
fn a_size_read_from_another_selection_is_dropped() {
    let mut app = app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp| {
        ctx.run_ui(Default::default(), |ui| show(app, ui)).textures_delta.clear();
        pstate::<Converter>(&ctx, STATE)
    };
    let mut st = Converter::default();
    from_selection(&mut app, &mut st).unwrap();
    set_pstate(&ctx, STATE, st);
    // Drawing the panel again with nothing changed keeps it.
    assert_eq!(frame(&mut app).size, Some((72.0, 36.0)));
    // Another object selected.
    app.run("shape.rectangle", json!({"x": 200, "y": 100, "width": 20, "height": 50})).unwrap();
    let mut st = frame(&mut app);
    assert_eq!((st.size, st.points), (None, Some(72.0)), "{st:?}");
    st.pick(Side::Height);
    assert_eq!(st.points, Some(72.0), "not the first object's height");
    apply_at(&mut app, &st, 4);
    let (w, h) = size(&mut app);
    assert!(near(w, 20.0) && near(h, 72.0), "{w} × {h}");
    // So does a change to the document.
    let mut st = Converter::default();
    from_selection(&mut app, &mut st).unwrap();
    app.run("object.rotate", json!({"angle": 30})).unwrap();
    st.follow(document_now(&app));
    assert_eq!(st.size, None);
    // And a change to how the selection is measured (Use Preview Bounds), which leaves the
    // document as it is.
    let mut st = Converter::default();
    from_selection(&mut app, &mut st).unwrap();
    st.follow(document_now(&app));
    assert!(st.size.is_some());
    app.run("prefs.set", json!({"key": "usePreviewBounds", "value": true})).unwrap();
    st.follow(document_now(&app));
    assert_eq!(st.size, None);
}

/// Each row's copy button reads as copying that row's unit, so a screen reader tells the twelve
/// apart, and the fields are named by their rows.
#[test]
fn each_copy_button_and_field_is_named_with_its_unit() {
    let mut app = app();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    set_pstate(&ctx, STATE, Converter { points: Some(72.0), ..Converter::default() });
    ctx.run_ui(Default::default(), |ui| show(&mut app, ui)).textures_delta.clear();
    let mut out = ctx.run_ui(Default::default(), |ui| show(&mut app, ui));
    out.textures_delta.clear();
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    // A label node carries its text as its value.
    let label_of = |id: egui::accesskit::NodeId| nodes.iter().find(|(n, _)| *n == id).and_then(|(_, n)| n.label().or(n.value())).map(str::to_string);
    let buttons: Vec<String> =
        nodes.iter().filter(|(_, n)| n.role() == egui::accesskit::Role::Button).filter_map(|(_, n)| n.label().map(str::to_string)).collect();
    for u in Unit::EAST_ASIAN {
        let name = format!("Copy {}", u.label());
        assert_eq!(buttons.iter().filter(|b| **b == name).count(), 1, "{name} in {buttons:?}");
        // The row fits the 240 pt body of the default 260 pt floating panel, with 10 pt left for
        // its scroll bar.
        let button = nodes.iter().find(|(_, n)| n.label() == Some(name.as_str())).and_then(|(_, n)| n.bounds()).unwrap();
        assert!(button.x1 <= 230.0, "{name} ends at {}", button.x1);
        // The field of the row is labeled by the row's label.
        let field = nodes
            .iter()
            .find(|(_, n)| n.role() != egui::accesskit::Role::Button && n.labelled_by().iter().any(|l| label_of(*l).as_deref() == Some(u.label())));
        assert!(field.is_some(), "a field labeled {}", u.label());
    }
}

#[test]
fn from_selection_with_nothing_selected_reports_it_and_keeps_the_length() {
    let mut app = app();
    app.run("select.none", json!({})).unwrap();
    let mut st = Converter { points: Some(10.0), ..Converter::default() };
    assert!(from_selection(&mut app, &mut st).is_err());
    assert_eq!((st.points, st.size), (Some(10.0), None));
}
