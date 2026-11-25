//! Unit Converter's state and commands: a typed length that shows in every unit replaces the
//! length, From Selection reads the selection's size, Apply sets the side picked as one undo step
//! and adds none when nothing changes, and Copy writes the value with or without its unit.

use serde_json::json;
use vectorcraft_doc::Unit;
use vectorcraft_engine::Session;

use super::{Converter, Side, apply, document_now, from_selection};
use crate::VectorcraftApp;

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

/// A readable length replaces the length; one too large for some unit (Q and Ha) leaves it.
#[test]
fn a_typed_length_that_shows_in_every_unit_replaces_the_length() {
    let mut st = Converter::default();
    st.typed(Some(36.0));
    assert_eq!(st.points, Some(36.0));
    // 1.7e308 pt is finite in points but not in Q and Ha.
    st.typed(Some(1.7e308));
    assert_eq!(st.points, Some(36.0), "too large to show in every unit");
    st.typed(None);
    assert_eq!(st.points, Some(36.0), "a cleared field leaves the length");
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

/// Apply `st` with the Transform panel's shared reference point at `reference`.
fn apply_at(app: &mut VectorcraftApp, st: &Converter, reference: usize) {
    try_apply_at(app, st, reference).unwrap();
}

/// [`apply_at`], with its result.
fn try_apply_at(app: &mut VectorcraftApp, st: &Converter, reference: usize) -> Result<(), String> {
    app.ui.transform_panel.reference_point = reference;
    apply(app, st)
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
    let mut st = Converter::default();
    from_selection(&mut app, &mut st).unwrap();
    app.addon_panels.units = st;
    // Nothing changed: the size stays.
    let now = document_now(&app);
    app.addon_panels.units.follow(now);
    assert_eq!(app.addon_panels.units.size, Some((72.0, 36.0)));
    // Another object selected.
    app.run("shape.rectangle", json!({"x": 200, "y": 100, "width": 20, "height": 50})).unwrap();
    let now = document_now(&app);
    app.addon_panels.units.follow(now);
    let mut st = app.addon_panels.units.clone();
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

#[test]
fn from_selection_with_nothing_selected_reports_it_and_keeps_the_length() {
    let mut app = app();
    app.run("select.none", json!({})).unwrap();
    let mut st = Converter { points: Some(10.0), ..Converter::default() };
    assert!(from_selection(&mut app, &mut st).is_err());
    assert_eq!((st.points, st.size), (Some(10.0), None));
}
