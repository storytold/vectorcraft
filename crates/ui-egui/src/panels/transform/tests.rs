use vectorcraft_doc::Unit;

use super::*;
use crate::panels::tests_units::{cancel_dialog, confirm_dialog, set_dialog_field};
use crate::tests::panel_harness::Panel;

fn panel(app: VectorcraftApp) -> Panel {
    Panel::new(app, show)
}

/// The Control bar's X, Y, W and H.
fn bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    object_bar_fields(app, ui);
}

/// A length in points as a length field reads it: its number and the unit painted after it.
fn pt(v: f64) -> String {
    Unit::Points.format(v)
}

/// A shown angle, in degrees.
fn degrees(p: &Panel, name: &str) -> Option<f64> {
    p.value(name)?.trim_end_matches('°').trim_end().parse().ok()
}

/// The state of panel-menu row `id`, as the panel menus' states give it.
fn row_state(app: &VectorcraftApp, id: &str) -> Option<CommandState> {
    let mut states = CommandStates::new();
    crate::panels::panel_menu_states(app, &mut states);
    states.get(&CommandRef::new(id)).cloned()
}

/// The check of panel-menu row `id`.
fn checked(app: &VectorcraftApp, id: &str) -> Option<bool> {
    row_state(app, id)?.is_checked()
}

/// Whether panel-menu row `id` is on.
fn enabled(app: &VectorcraftApp, id: &str) -> bool {
    row_state(app, id).is_some_and(|s| s.is_enabled())
}

fn blank(v: Option<String>) -> bool {
    v.is_none_or(|s| s.is_empty())
}

fn new_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app
}

#[test]
fn use_preview_bounds_shows_the_visual_size_and_checks_the_align_flyout() {
    let mut app = new_app();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
    app.run("stroke.set", json!({"weight": 10})).unwrap();
    let mut p = panel(app);
    assert_eq!(p.value("Width"), Some(pt(100.0)));
    assert_ne!(checked(&p.app(), "ui.align.usePreviewBounds"), Some(true));
    p.app().run("ui.align.usePreviewBounds", json!({})).unwrap();
    p.frame();
    assert_eq!((p.value("Width"), p.value("Height")), (Some(pt(110.0)), Some(pt(60.0))));
    assert_eq!(checked(&p.app(), "ui.align.usePreviewBounds"), Some(true));
}

/// Measure Type by Glyph Bounds is checked while `typeGlyphBounds` is on, and the bounding box of
/// selected point type then measures its glyphs without a change to the document.
#[test]
fn measure_type_by_glyph_bounds_is_checked_and_resizes_the_box() {
    let mut app = new_app();
    app.run("text.create", json!({"x": 20, "y": 200, "size": 100, "text": "Hg"})).unwrap();
    assert_ne!(checked(&app, "ui.transform.typeGlyphBounds"), Some(true));
    let em = app.selection_box().unwrap().rect;
    let revision = app.session.active().unwrap().revision;
    set_pref(&mut app, "typeGlyphBounds", true);
    assert!(app.session.prefs.type_glyph_bounds);
    assert_eq!(checked(&app, "ui.transform.typeGlyphBounds"), Some(true));
    assert_eq!(app.session.active().unwrap().revision, revision);
    let glyphs = app.selection_box().unwrap().rect;
    assert!(glyphs.height() < em.height() - 10.0, "{glyphs:?} inside {em:?}");
    set_pref(&mut app, "typeGlyphBounds", false);
    assert_eq!(app.selection_box().unwrap().rect, em);
}

#[test]
fn use_preview_bounds_shows_the_size_after_geometry_effects() {
    let offset = 10.0;
    let mut app = new_app();
    app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 50})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": offset}, "item": null})).unwrap();
    let mut p = panel(app);
    assert_eq!((p.value("Width"), p.value("Height")), (Some(pt(100.0)), Some(pt(50.0))));
    set_pref(&mut p.app(), "usePreviewBounds", true);
    p.frame();
    assert_eq!((p.value("Width"), p.value("Height")), (Some(pt(100.0 + 2.0 * offset)), Some(pt(50.0 + 2.0 * offset))));
}

#[test]
fn rotation_field_shows_the_angle_the_box_keeps() {
    let mut app = new_app();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
    app.run("object.rotate", json!({"angle": 45, "absolute": true})).unwrap();
    let mut p = panel(app);
    // The angle stays, and W/H are the rectangle's own sides.
    let bx = p.app().selection_box().unwrap();
    assert!((degrees(&p, "Rotate").unwrap() - bx.angle).abs() < 0.05);
    assert_eq!((p.value("Width"), p.value("Height")), (Some(pt(bx.rect.width())), Some(pt(bx.rect.height()))));
    p.app().run("object.resetBoundingBox", json!({})).unwrap();
    p.frame();
    assert_eq!(degrees(&p, "Rotate"), Some(0.0));
}

/// Object › Transform › Rotate Bounding Box… asks for an angle and turns the box, not the art, in
/// one undo step: the panel then shows the box's angle.
#[test]
fn rotate_bounding_box_menu_item_turns_the_box_by_the_dialog_angle() {
    let mut app = new_app();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
    // The menu item as a click runs it: its params ask for the angle in a dialog.
    let entry = crate::menus::menu_entries(&app)
        .into_iter()
        .find(|e| e.path == ["Object", "Transform"] && e.label == "Rotate Bounding Box…")
        .expect("menu item");
    let (id, p) = crate::menus::click_target(&entry.label, entry.command.as_deref().unwrap(), &entry.params);
    app.adopt_context(&crate::testing::ui_context());
    crate::menus::invoke(&mut app, &id, p);
    set_dialog_field(&mut app, "angle", json!(90));
    let bounds = |app: &VectorcraftApp| {
        let st = app.session.doc().unwrap();
        st.doc.bounds_of(&st.selection.objects, false)
    };
    let art = bounds(&app);
    let undo = app.session.doc().unwrap().history.undo.len();
    confirm_dialog(&mut app);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(bounds(&app), art, "the art stays where it is");
    assert_eq!(degrees(&panel(app), "Rotate"), Some(90.0));
}

/// A live rectangle's Rectangle Properties show one Corner menu for all four corners while Corner
/// Radius is linked, blank when the corners' kinds differ, and one per corner when it is unlinked.
#[test]
fn a_rectangle_shows_linked_and_unlinked_corner_types() {
    let mut app = new_app();
    app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 120, "height": 80, "radius": 10})).unwrap();
    set_pref(&mut app, "singleCornerRadius", true);
    let mut p = panel(app);
    assert!(p.shows("Corner") && !p.shows("Top Left Corner"));
    assert_eq!(p.value("Corner").map(|s| s.is_empty()), Some(false), "one kind: the menu reads it");
    // Kinds that differ leave the linked menu blank.
    p.app().run("object.setLiveShape", json!({"kind": "chamfer", "corners": [2]})).unwrap();
    p.frame();
    assert!(p.shows("Corner") && blank(p.value("Corner")));
    set_pref(&mut p.app(), "singleCornerRadius", false);
    p.frame();
    assert!(!p.shows("Corner") && p.shows("Top Left Corner") && p.shows("Bottom Left Corner"));
}

/// A corner kind set with no live rectangle selected leaves the selected polygon, the document and
/// its undo history as they are.
#[test]
fn a_rectangle_corner_kind_without_a_selected_rectangle_changes_nothing() {
    use vectorcraft_geom::shapes::CornerKind::Chamfer;
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.polygon", json!({"cx": 150, "cy": 150, "radius": 40, "sides": 6})).unwrap();
    let st = app.session.active().unwrap();
    let (before, steps) = (st.doc.clone(), st.history.undo.len());
    crate::panels::set_rectangle_kinds(&mut app, Chamfer, None);
    crate::panels::set_rectangle_kinds(&mut app, Chamfer, Some(2));
    let st = app.session.active().unwrap();
    assert!(st.doc == before, "the document changed");
    assert_eq!(st.history.undo.len(), steps);
}

/// A corner kind picked while a Shape Builder merge is computed on another thread applies the
/// merge first, and sets the kind on the live rectangles selected after it.
#[test]
fn a_rectangle_corner_kind_applies_a_merge_on_another_thread_first() {
    use vectorcraft_geom::shapes::CornerKind::Chamfer;
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
    for k in 0..40 {
        app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
    app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
    let view = app.view_info();
    let at = |kind| PointerEvent {
        kind,
        pos: vectorcraft_geom::Point::new(15.0, 10.0),
        mods: Mods::default(),
        pressure: 1.0,
        backquote: false,
        pen: None,
    };
    crate::canvas::dispatch(&mut app, &at(PointerKind::Move), view);
    app.session.tool_poll(view, true).unwrap();
    crate::canvas::dispatch(&mut app, &at(PointerKind::Down), view);
    crate::canvas::dispatch(&mut app, &at(PointerKind::Up), view);
    assert!(app.session.tool_editing());
    crate::panels::set_rectangle_kinds(&mut app, Chamfer, None);
    assert!(!app.session.tool_editing());
    let st = app.session.active().unwrap();
    let labels: Vec<_> = st.history.undo.iter().rev().take(2).map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["Live Shape", "Shape Builder"]);
    let kinds: Vec<_> = st
        .selection
        .objects
        .iter()
        .filter_map(|id| match &st.doc.node(*id)?.kind {
            NodeKind::Path { live: Some(l), .. } => match &**l {
                LiveShape::Rectangle { kinds, .. } => Some(*kinds),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert!(!kinds.is_empty() && kinds.iter().all(|k| *k == [Chamfer; 4]), "{kinds:?}");
}

/// A live rectangle and a live ellipse show their own angle apart from the Rotate field's box
/// angle, which Reset Bounding Box squares to the page. Selected shapes of one kind at different
/// angles show a blank angle. The shared angle is kept for the revision.
#[test]
fn a_rectangle_and_an_ellipse_show_their_angle() {
    let mut p = panel(new_app());
    for shape in ["shape.rectangle", "shape.ellipse"] {
        let mut ids = vec![];
        for (y, angle) in [(20, 30), (200, 60)] {
            let mut app = p.app();
            let id = app.run(shape, json!({"x": 50, "y": y, "width": 100, "height": 50})).unwrap()["id"].as_u64().unwrap();
            app.run("object.rotate", json!({"angle": angle, "absolute": true})).unwrap();
            app.run("object.resetBoundingBox", json!({})).unwrap();
            ids.push(id);
        }
        p.app().run("select.set", json!({"ids": [ids[0]]})).unwrap();
        p.frame();
        assert_eq!((degrees(&p, "Rotate"), degrees(&p, "Angle")), (Some(0.0), Some(30.0)), "{shape}");
        let key = p.app().canvas.shape_angle.clone();
        assert!(key.is_some(), "{shape}: the angle is kept");
        p.frame();
        assert_eq!(p.app().canvas.shape_angle, key, "{shape}: not measured again");
        p.app().run("select.set", json!({"ids": ids})).unwrap();
        p.frame();
        assert!(blank(p.value("Angle")), "{shape}: mixed angles");
        p.app().run("select.set", json!({"ids": [ids[1]]})).unwrap();
        p.frame();
        assert_eq!(degrees(&p, "Angle"), Some(60.0), "{shape}");
    }
}

/// Ellipse Properties' Constrain Pie Angle link: off at first, a click or Space on it toggles it, and
/// while it is on a typed Pie Start or End Angle moves the other angle by as much, keeping the span,
/// in one undo step. Off, each angle changes on its own.
#[test]
fn the_constrain_pie_angle_link_keeps_the_span() {
    const LINK: &str = "Constrain Pie Angle";
    let pie = |app: &VectorcraftApp| match &first_selected(app).unwrap().kind {
        NodeKind::Path { live: Some(l), .. } => match **l {
            LiveShape::Ellipse { pie, .. } => pie,
            ref k => panic!("{k:?}"),
        },
        k => panic!("{k:?}"),
    };
    let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let mut app = new_app();
    app.run("shape.ellipse", json!({"x": 50, "y": 50, "width": 120, "height": 80})).unwrap();
    app.run("object.setLiveShape", json!({"pieStart": 30, "pieEnd": 120})).unwrap();
    assert!(!app.ui.constrain_pie_angle, "off at first");
    let mut p = panel(app);
    p.click(LINK);
    assert!(p.app().ui.constrain_pie_angle, "a click turns it on");
    // A click leaves the focus where it was, so the link takes it first, as production's test gave it.
    p.focus(LINK);
    p.key(egui::Key::Space, egui::Modifiers::NONE);
    assert!(!p.app().ui.constrain_pie_angle, "Space turns it off");
    p.click(LINK);
    assert!(p.app().ui.constrain_pie_angle);
    let n = undo(&p.app());
    p.type_into("Pie Start Angle", "60");
    assert_eq!(pie(&p.app()), (60.0, 150.0));
    assert_eq!(undo(&p.app()), n + 1, "one undo step");
    p.type_into("Pie End Angle", "200");
    assert_eq!(pie(&p.app()), (110.0, 200.0));
    p.click(LINK);
    assert!(!p.app().ui.constrain_pie_angle);
    p.type_into("Pie Start Angle", "100");
    assert_eq!(pie(&p.app()), (100.0, 200.0), "off, the end stays");
}

/// A live line shows its Line Properties: a typed length keeps its first end in place and a typed
/// angle turns it about its midpoint, each one undo step. A line with other objects selected, and a
/// line after Expand Shape, show none.
#[test]
fn a_line_shows_and_takes_its_line_properties() {
    let ends = |app: &VectorcraftApp| match &first_selected(app).unwrap().kind {
        NodeKind::Path { live: Some(l), .. } => match **l {
            LiveShape::Line { a, b } => (a, b),
            ref k => panic!("{k:?}"),
        },
        k => panic!("{k:?}"),
    };
    let near = |p: Point, x: f64, y: f64| (p.x - x).abs() < 1e-6 && (p.y - y).abs() < 1e-6;
    let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let mut app = new_app();
    // 100 pt long, going down the page to the right: 53.13° clockwise, 306.87° counterclockwise.
    let id = app.run("shape.line", json!({"x1": 50, "y1": 50, "x2": 110, "y2": 130})).unwrap()["id"].as_u64().unwrap();
    let mut p = panel(app);
    assert_eq!(p.value("Line Length"), Some(pt(100.0)));
    assert!((degrees(&p, "Line Angle").unwrap() - 306.87).abs() < 0.05);
    let n = undo(&p.app());
    p.type_into("Line Length", "50");
    let (a, b) = ends(&p.app());
    assert!(near(a, 50.0, 50.0) && near(b, 80.0, 90.0), "{a:?} {b:?}");
    assert_eq!(undo(&p.app()), n + 1, "one undo step per edit");
    p.type_into("Line Angle", "90");
    let (a, b) = ends(&p.app());
    assert!(near(a, 65.0, 95.0) && near(b, 65.0, 45.0), "{a:?} {b:?}");
    assert_eq!(undo(&p.app()), n + 2, "one undo step per edit");
    assert_eq!(degrees(&p, "Line Angle"), Some(90.0));
    p.app().run("edit.undo", json!({})).unwrap();
    let (a, b) = ends(&p.app());
    assert!(near(a, 50.0, 50.0) && near(b, 80.0, 90.0), "one undo step: {a:?} {b:?}");
    // With a rectangle selected too, the fields would set the line alone: they don't show.
    let rect = p.app().run("shape.rectangle", json!({"x": 200, "y": 200, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    p.app().run("select.set", json!({"ids": [id, rect]})).unwrap();
    p.frame();
    assert!(!p.shows("Line Length"));
    p.app().run("select.set", json!({"ids": [id]})).unwrap();
    p.frame();
    assert!(p.shows("Line Length"));
    p.app().run("object.expandShape", json!({})).unwrap();
    p.frame();
    assert!(!p.shows("Line Length"));
}

#[test]
fn the_artboard_tool_shows_the_active_artboard() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
    let mut p = panel(app);
    assert_eq!(p.value("Width"), Some(pt(40.0)), "the selection first");
    p.app().run("tool.select", json!({"tool": "artboard"})).unwrap();
    p.frame();
    // Centre reference point: X 150, Y 100, W 300, H 200; no rotate/shear for artboards.
    let r = p.app().session.active().unwrap().doc.artboards[0].rect;
    assert_eq!((p.value("X position"), p.value("Y position")), (Some(pt(r.center().x)), Some(pt(r.center().y))));
    assert_eq!((p.value("Width"), p.value("Height")), (Some(pt(r.width())), Some(pt(r.height()))));
    assert!(!p.shows("Rotate"));
}

/// #1597: with only ruler guides selected the panel shows their X or Y, and nothing of the art.
#[test]
fn selected_guides_show_their_position() {
    let mut app = new_app();
    app.run("guide.add", json!({"vertical": true, "pos": 120})).unwrap();
    app.run("guide.add", json!({"vertical": false, "pos": 45})).unwrap();
    app.run("guide.select", json!({"indexes": [0]})).unwrap();
    assert_eq!(selected_guides(&app), Some((Some(120.0), None)));
    let mut p = panel(app);
    assert_eq!(p.value("X"), Some(pt(120.0)));
    assert!(p.enabled("X") && !p.enabled("Y"));
    p.app().run("guide.select", json!({"indexes": [0, 1]})).unwrap();
    p.frame();
    assert_eq!((p.value("X"), p.value("Y")), (Some(pt(120.0)), Some(pt(45.0))));
    // The field's edit: the vertical guide moves, the horizontal one stays.
    p.app().run("guide.move", json!({"dx": 30})).unwrap();
    assert_eq!(selected_guides(&p.app()), Some((Some(150.0), Some(45.0))));
    // Art selected with them: the art's box, as before.
    p.app().run("shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
    assert_eq!(selected_guides(&p.app()), None);
}

/// #1824: with an anchor selected with Direct Selection the panel shows that anchor's position,
/// and a new X moves it alone.
#[test]
fn an_anchor_selection_shows_and_moves_its_own_box() {
    let mut app = new_app();
    let square = json!({"anchors": [{"x": 20, "y": 30}, {"x": 120, "y": 30}, {"x": 120, "y": 130}, {"x": 20, "y": 130}], "closed": true});
    let id = app.run("path.create", square).unwrap()["id"].as_u64().unwrap();
    let mut p = panel(app);
    assert_eq!((p.value("X position"), p.value("Width")), (Some(pt(70.0)), Some(pt(100.0))), "the whole path");
    p.app().run("select.anchors", json!({"id": id, "anchors": [[0, 1]]})).unwrap();
    p.frame();
    assert_eq!((p.value("X position"), p.value("Y position"), p.value("Width")), (Some(pt(120.0)), Some(pt(30.0)), Some(pt(0.0))));
    p.type_into("X position", "150");
    let n = first_selected(&p.app()).unwrap();
    let Some(path) = n.path_data() else { panic!("a path") };
    let xs: Vec<f64> = path.subpaths[0].anchors.iter().map(|a| a.p.x).collect();
    assert_eq!(xs, [20.0, 150.0, 120.0, 20.0]);
}

/// #2398: the panel menu's Flip Horizontal and Flip Vertical mirror the selection across the
/// reference point, not across its centre.
#[test]
fn menu_flips_keep_the_reference_point() {
    let mut app = new_app();
    app.run("shape.rectangle", json!({"x": 20, "y": 30, "width": 100, "height": 50})).unwrap();
    // Top-right reference point: (120, 30).
    app.ui.transform_panel.reference_point = 2;
    app.run("ui.transform.flipHorizontal", json!({})).unwrap();
    assert_eq!(app.selection_box().unwrap().rect, Rect::new(120.0, 30.0, 220.0, 80.0));
    app.run("ui.transform.flipVertical", json!({})).unwrap();
    assert_eq!(app.selection_box().unwrap().rect, Rect::new(120.0, -20.0, 220.0, 30.0));
}

/// A rectangle filled with a pattern, selected, in a new document.
fn pattern_app() -> (VectorcraftApp, u64) {
    let mut app = new_app();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.run("object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    app.run("object.pattern.done", json!({})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 40, "width": 100, "height": 60})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Dots"})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    (app, id)
}

/// The matrix coefficients of the tiles of the pattern fill of node `id`.
fn tiles(app: &VectorcraftApp, id: u64) -> [f64; 6] {
    let n = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    let vectorcraft_color::Paint::Pattern { xf, .. } = n.appearance.fill_paint() else { panic!("a pattern fill") };
    xf.as_coeffs()
}

const PATTERN_MODES: [&str; 3] = ["ui.transform.objectOnly", "ui.transform.patternOnly", "ui.transform.both"];

/// The Transform Object Only, Transform Pattern Only and Transform Both commands, one checked: a new
/// X typed in the panel (or the Control bar) moves the pattern-filled rectangle and leaves its
/// tiles, moves its tiles alone, or moves both. Until one is chosen the panels do as General ›
/// Transform Pattern Tiles says; a choice overrides it, and resetting the preferences returns to it.
#[test]
fn the_menu_chooses_whether_the_object_its_pattern_or_both_transform() {
    let checked_modes = |app: &VectorcraftApp| -> Vec<usize> { (0..3).filter(|i| checked(app, PATTERN_MODES[*i]) == Some(true)).collect() };
    let modes = ["object", "pattern", "both"];
    for draw in [show as fn(&mut VectorcraftApp, &mut Ui), bar] {
        for general in [false, true] {
            for pick in [None, Some(0), Some(1), Some(2)] {
                let case = format!("General › Transform Pattern Tiles {general}, menu choice {pick:?}");
                let (mut app, id) = pattern_app();
                app.run("prefs.set", json!({"key": "transformPatternTiles", "value": general})).unwrap();
                let following = if general { 2 } else { 0 };
                assert_eq!(checked_modes(&app), [following], "{case}: before a choice");
                if let Some(item) = pick {
                    app.run(PATTERN_MODES[item], json!({})).unwrap();
                }
                let item = pick.unwrap_or(following);
                assert_eq!(app.session.prefs.transform_panel_patterns, pick.map_or("default", |i| modes[i]), "{case}");
                assert_eq!(checked_modes(&app), [item], "{case}");
                // The centre reference point: X 150 becomes 170.
                let mut p = Panel::new(app, draw);
                p.type_into("X position", "170");
                let mut guard = p.app();
                let app = &mut *guard;
                let x0 = app.selection_box().unwrap().rect.x0;
                assert_eq!(x0, if item == 1 { 100.0 } else { 120.0 }, "{case}: the object");
                assert_eq!(tiles(app, id), [1.0, 0.0, 0.0, 1.0, if item == 0 { 0.0 } else { 20.0 }, 0.0], "{case}: the tiles");
                // Reset (the General category): the choice follows General › Transform Pattern Tiles again.
                app.run("prefs.reset", json!({"category": "General"})).unwrap();
                assert_eq!(app.session.prefs.transform_panel_patterns, "default", "{case}: reset");
                let following = if app.session.prefs.transform_pattern_tiles { 2 } else { 0 };
                assert_eq!(checked_modes(app), [following], "{case}: reset");
            }
        }
    }
}

/// The three commands act on the rotation field and on the Transform panel's shear field as they do
/// on X: the pattern-filled rectangle turns or shears with its tiles left as they are, its tiles
/// alone, or both.
#[test]
fn the_menu_choice_also_applies_to_rotation_and_shear() {
    for field in ["Rotate", "Shear"] {
        for (item, command) in PATTERN_MODES.iter().enumerate() {
            let case = format!("{command}, {field}");
            let (mut app, id) = pattern_app();
            app.run(command, json!({})).unwrap();
            let before = (app.selection_box().unwrap(), tiles(&app, id));
            let mut p = panel(app);
            p.type_into(field, "30");
            let after = {
                let mut app = p.app();
                (app.selection_box().unwrap(), tiles(&app, id))
            };
            assert_eq!(after.0 != before.0, item != 1, "{case}: the object {:?} → {:?}", before.0, after.0);
            // Turned or sheared tiles change their matrix's linear part; moved ones only its offset.
            assert_eq!(after.1[..4] != before.1[..4], item != 0, "{case}: the tiles {:?} → {:?}", before.1, after.1);
        }
    }
}

/// Use Registration Point for Symbol: with one symbol instance selected, the panel and the Control
/// bar show and set X and Y at its symbol's registration point (here the top-left one) instead of
/// the reference point (the center).
#[test]
fn use_registration_point_for_symbol_measures_an_instance_from_its_registration_point() {
    for draw in [show as fn(&mut VectorcraftApp, &mut Ui), bar] {
        let mut app = new_app();
        app.run("shape.rectangle", json!({"x": 100, "y": 50, "width": 80, "height": 40})).unwrap();
        let id = app.run("symbol.new", json!({"name": "Corner", "registration": 0})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        let mut p = Panel::new(app, draw);
        assert_eq!((p.value("X position"), p.value("Y position")), (Some(pt(140.0)), Some(pt(70.0))), "the centre");
        assert_ne!(checked(&p.app(), "ui.transform.registrationPoint"), Some(true));
        p.app().run("ui.transform.registrationPoint", json!({})).unwrap();
        assert!(p.app().session.prefs.use_registration_point_for_symbol);
        assert_eq!(checked(&p.app(), "ui.transform.registrationPoint"), Some(true));
        p.frame();
        assert_eq!((p.value("X position"), p.value("Y position")), (Some(pt(100.0)), Some(pt(50.0))), "the top-left");
        p.type_into("X position", "30");
        assert_eq!(p.app().selection_box().unwrap().rect, Rect::new(30.0, 50.0, 110.0, 90.0));
        // Any other selection: the reference point.
        p.app().run("shape.rectangle", json!({"x": 200, "y": 200, "width": 20, "height": 20})).unwrap();
        p.frame();
        assert_eq!(p.value("X position"), Some(pt(210.0)), "the centre");
    }
}

/// The Control bar's X, Y, W and H use the reference point the Transform panel's locator chose:
/// X and Y show that point of the selection's box and a new X or Y moves it there, a new W or H
/// keeps it in place, and choosing another point changes what the fields show without editing the
/// art. Each field edit is one undo step.
#[test]
fn the_control_bar_fields_use_the_shared_reference_point() {
    // The reference point (top-left, center, bottom-right): X/Y shown for the rectangle at
    // (10, 20), 100 × 50; the box after X becomes `new_x`, and after W becomes 200 and H 100.
    let cases = [
        (0usize, (10.0, 20.0), "30", Rect::new(30.0, 20.0, 130.0, 70.0), Rect::new(30.0, 20.0, 230.0, 120.0)),
        (4, (60.0, 45.0), "80", Rect::new(30.0, 20.0, 130.0, 70.0), Rect::new(-20.0, -5.0, 180.0, 95.0)),
        (8, (110.0, 70.0), "130", Rect::new(30.0, 20.0, 130.0, 70.0), Rect::new(-70.0, -30.0, 130.0, 70.0)),
    ];
    let revision = |p: &Panel| p.app().session.active().unwrap().revision;
    let steps = |p: &Panel| p.app().session.active().unwrap().history.undo.len();
    for (refi, (x, y), new_x, moved, sized) in cases {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 100, "height": 50})).unwrap();
        app.ui.transform_panel.reference_point = refi;
        let mut p = Panel::new(app, bar);
        // The Control bar spans the window.
        p.room(egui::vec2(1600.0, 200.0));
        let (before, undo) = (revision(&p), steps(&p));
        assert_eq!((p.value("X position"), p.value("Y position")), (Some(pt(x)), Some(pt(y))), "reference {refi}");
        assert_eq!(revision(&p), before, "reference {refi}: choosing the point leaves the art");
        p.type_into("X position", new_x);
        assert_eq!(p.app().selection_box().unwrap().rect, moved, "reference {refi}: X");
        // A typed value previews as it is typed (contract: a plain number applies as typed); the edit is one step.
        assert_eq!(steps(&p), undo + 1, "reference {refi}: one undo step");
        p.type_into("Width", "200");
        p.type_into("Height", "100");
        assert_eq!(p.app().selection_box().unwrap().rect, sized, "reference {refi}: W and H");
        p.app().run("edit.undo", json!({})).unwrap();
        p.app().run("edit.undo", json!({})).unwrap();
        assert_eq!(p.app().selection_box().unwrap().rect, moved, "reference {refi}: undo");
    }
}

/// Use Registration Point for Symbol on a reflected, a rotated and a sheared symbol instance:
/// the panel shows X and Y at the registration point where the instance's transform put it (not at
/// the box's point of the same index), a new X moves it there, and the rotation field and the flip
/// commands turn and mirror the instance about it.
#[test]
fn use_registration_point_for_symbol_follows_a_transformed_instance() {
    // How each instance is transformed after it is made: (command, params).
    let shapes = [
        ("object.reflect", json!({"axis": "vertical"})),
        ("object.rotate", json!({"angle": 90})),
        // About the registration point (the top-left corner), which then lies on the box's
        // top edge but on none of its nine reference points.
        ("object.shear", json!({"angle": 30, "axis": "horizontal", "origin": [100, 50]})),
    ];
    // The registration point (the art's top-left) where the instance's transform puts it.
    let registration = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::SymbolInstance { xf, .. } => *xf * vectorcraft_geom::Point::new(-vectorcraft_doc::SYMBOL_HALF, -vectorcraft_doc::SYMBOL_HALF),
        k => panic!("not an instance: {k:?}"),
    };
    let near = |a: vectorcraft_geom::Point, b: vectorcraft_geom::Point| (a - b).hypot() < 1e-6;
    for (cmd, params) in &shapes {
        let case = format!("{cmd} {params}");
        let mut app = new_app();
        app.run("shape.rectangle", json!({"x": 100, "y": 50, "width": 80, "height": 40})).unwrap();
        let id = app.run("symbol.new", json!({"name": "Corner", "registration": 0})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run(cmd, params.clone()).unwrap();
        app.run("prefs.set", json!({"key": "useRegistrationPointForSymbol", "value": true})).unwrap();
        let q = registration(&app, id);
        let mut p = panel(app);
        assert_eq!((p.value("X position"), p.value("Y position")), (Some(pt(q.x)), Some(pt(q.y))), "{case}: X/Y at {q:?}");
        // A new X moves the registration point there.
        p.type_into("X position", &format!("{}", q.x - 40.0));
        let q = vectorcraft_geom::Point::new(q.x - 40.0, q.y);
        assert!(near(registration(&p.app(), id), q), "{case}: moved to {q:?}, now {:?}", registration(&p.app(), id));
        // The rotation field turns it about the registration point.
        let angle = p.app().selection_box().unwrap().angle;
        let before = p.app().session.active().unwrap().revision;
        p.type_into("Rotate", &format!("{}", angle + 30.0));
        assert_ne!(p.app().session.active().unwrap().revision, before, "{case}: turned");
        assert!(near(registration(&p.app(), id), q), "{case}: rotated about {q:?}, now {:?}", registration(&p.app(), id));
        // The flips mirror it about the registration point.
        for flip in ["ui.transform.flipHorizontal", "ui.transform.flipVertical"] {
            let before = p.app().session.active().unwrap().revision;
            p.app().run(flip, json!({})).unwrap();
            assert_ne!(p.app().session.active().unwrap().revision, before, "{case}: {flip} flipped");
            assert!(near(registration(&p.app(), id), q), "{case}: {flip} about {q:?}, now {:?}", registration(&p.app(), id));
        }
    }
}

#[test]
fn artboard_fields_keep_the_reference_point() {
    let r = Rect::new(0.0, 0.0, 300.0, 200.0);
    // Centre: a new X centres it there, a new W grows it both ways.
    assert_eq!(artboard_rect(r, 4, Some(200.0), None, None, None, false), Rect::new(50.0, 0.0, 350.0, 200.0));
    assert_eq!(artboard_rect(r, 4, None, None, Some(400.0), None, false), Rect::new(-50.0, 0.0, 350.0, 200.0));
    // Top-left: a new W keeps the left edge; linked, H follows.
    assert_eq!(artboard_rect(r, 0, None, None, Some(600.0), None, true), Rect::new(0.0, 0.0, 600.0, 400.0));
    // Bottom-right: a new Y puts the bottom edge there.
    assert_eq!(artboard_rect(r, 8, None, Some(500.0), None, None, false), Rect::new(0.0, 300.0, 300.0, 500.0));
}

/// A 100 × 60 rectangle at (100, 40), selected, in a new app.
fn shear_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 40, "width": 100, "height": 60})).unwrap();
    app
}

/// Types `value` into the panel's Shear field of a [`shear_app`] and returns the panel.
fn type_shear(value: &str) -> Panel {
    let mut p = panel(shear_app());
    p.type_into("Shear", value);
    p
}

/// The Shear field takes -359° to 359°: a typed 120, 359 and -359 shear about the reference point
/// (the centre) as `object.shear` does with that angle, and 120 slants as -60 does.
#[test]
fn the_shear_field_takes_angles_from_minus_359_to_359() {
    for (typed, angle, like) in [("120", 120.0, Some(-60.0)), ("359", 359.0, None), ("-359", -359.0, None), ("45", 45.0, None)] {
        let before = shear_app().selection_box().unwrap();
        let p = type_shear(typed);
        let mut app = p.app();
        assert_eq!(app.ui.status, "", "{typed}: no error");
        let got = app.selection_box().unwrap();
        assert_ne!(got, before, "{typed}: sheared");
        let mut want = shear_app();
        want.run("object.shear", json!({"angle": angle, "axis": "horizontal", "origin": [150, 70]})).unwrap();
        assert_eq!(got, want.selection_box().unwrap(), "{typed}");
        if let Some(like) = like {
            let mut same = shear_app();
            same.run("object.shear", json!({"angle": like, "axis": "horizontal", "origin": [150, 70]})).unwrap();
            let (a, b) = (got.rect, same.selection_box().unwrap().rect);
            assert!(
                (a.x0 - b.x0).abs() < 1e-6 && (a.x1 - b.x1).abs() < 1e-6 && (a.y0 - b.y0).abs() < 1e-6 && (a.y1 - b.y1).abs() < 1e-6,
                "{typed}: {a:?} like {b:?}"
            );
        }
    }
}

/// A typed ±90 or ±270, which have no shear, and a typed 360 or ±400, outside -359 to 359, leave the
/// rectangle as it is, record no undo step and put an error in the status bar.
#[test]
fn the_shear_field_shows_the_error_for_an_angle_with_no_shear() {
    for typed in ["90", "-90", "270", "-270", "360", "400", "-400"] {
        let (before, revision) = (shear_app().selection_box().unwrap(), 0);
        let _ = revision;
        let mut p = panel(shear_app());
        let (rev, undo) = {
            let app = p.app();
            (app.session.active().unwrap().revision, app.session.doc().unwrap().history.undo.len())
        };
        p.type_into("Shear", typed);
        let mut app = p.app();
        assert!(!app.ui.status.is_empty(), "{typed}: an error shows");
        assert_eq!(app.selection_box().unwrap(), before, "{typed}: nothing moved");
        assert_eq!(app.session.active().unwrap().revision, rev, "{typed}: unchanged");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "{typed}: no undo step");
    }
}

/// The Align to Pixel Grid checkbox turns the selected paths' pixel-aligned property on, shows a
/// dash when they differ (a click then turns it on for both), turns it off when it is on for all,
/// and is unavailable with type selected.
#[test]
fn align_to_pixel_grid_shows_and_sets_the_property() {
    const BOX: &str = "Align to Pixel Grid";
    let mut app = new_app();
    let a = app.run("shape.rectangle", json!({"x": 10.3, "y": 20.6, "width": 30.4, "height": 9.7})).unwrap()["id"].as_u64().unwrap();
    let b = app.run("shape.rectangle", json!({"x": 60.3, "y": 20.6, "width": 30.4, "height": 9.7})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let aligned = |app: &VectorcraftApp, id: u64| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().extras().pixel_aligned;
    let mut p = panel(app);
    assert_eq!(pixel_aligned_state(&p.app()).map(|s| (s.0, s.1)), Some((Some(false), true)));
    assert!(p.enabled(BOX));
    p.click(BOX);
    {
        let app = p.app();
        assert!(aligned(&app, a) && aligned(&app, b));
    }
    let x0 = p.app().session.active().unwrap().doc.node(vectorcraft_doc::NodeId(a)).unwrap().geometric_bounds().unwrap().x0;
    // The default 1 pt stroke is odd and centered: the edges go to half points.
    assert_eq!(x0, 10.5);
    p.app().run("object.setPixelAligned", json!({"ids": [b], "on": false})).unwrap();
    p.frame();
    assert_eq!(pixel_aligned_state(&p.app()).map(|s| s.0), Some(None));
    p.click(BOX);
    {
        let app = p.app();
        assert!(aligned(&app, a) && aligned(&app, b));
    }
    p.click(BOX);
    {
        let app = p.app();
        assert!(!aligned(&app, a) && !aligned(&app, b));
    }
    let t = p.app().run("text.create", json!({"x": 10, "y": 100, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    p.app().run("select.set", json!({"ids": [a, t]})).unwrap();
    p.frame();
    assert_eq!(pixel_aligned_state(&p.app()).map(|s| s.1), Some(false));
    assert!(!p.enabled(BOX));
    // A path inside a compound path snaps with the compound path, so it cannot be aligned alone.
    p.app().run("select.set", json!({"ids": [a, b]})).unwrap();
    let c = p.app().run("object.compoundPath.make", json!({})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(pixel_aligned_state(&p.app()).map(|s| s.1), Some(true));
    let member = p.app().session.active().unwrap().doc.node(vectorcraft_doc::NodeId(c)).unwrap().children().unwrap()[0].id.0;
    p.app().run("select.set", json!({"ids": [member]})).unwrap();
    assert_eq!(pixel_aligned_state(&p.app()).map(|s| s.1), Some(false));
}

/// The Align to Pixel Grid state measured from scratch in one pass over the selection, with the ids
/// a click sends: what [`pixel_aligned_shown`] and [`pixel_aligned_ids`] are compared with.
fn full_pixel_aligned_state(app: &VectorcraftApp) -> Option<(Option<bool>, bool, Vec<u64>)> {
    let st = app.session.active()?;
    let nodes: Vec<&Node> = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).collect();
    let first = nodes.first()?.extras().pixel_aligned;
    let value = nodes.iter().all(|n| n.extras().pixel_aligned == first).then_some(first);
    let in_compound = |n: &Node| st.doc.parent_of(n.id).and_then(|p| st.doc.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. }));
    let alignable = nodes.iter().all(|n| matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. }) && !in_compound(n));
    Some((value, alignable, nodes.iter().map(|n| n.id.0).collect()))
}

/// The Align to Pixel Grid state the panel shows, and the ids a click sends, equal a full measure
/// of the selection: with no document, nothing selected, mixed values, a guide, a compound path,
/// a path inside a compound path and a selection that holds both.
#[test]
fn align_to_pixel_grid_kept_state_equals_a_full_measure() {
    fn same(app: &mut VectorcraftApp, why: &str) -> Option<PixelAligned> {
        let full = full_pixel_aligned_state(app);
        let shown = pixel_aligned_shown(app);
        assert_eq!(shown, full.as_ref().map(|f| (f.0, f.1)), "{why}");
        assert_eq!(pixel_aligned_ids(app), full.map_or(vec![], |f| f.2), "{why}");
        shown
    }
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    assert_eq!(same(&mut app, "no document"), None);
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    assert_eq!(same(&mut app, "nothing selected"), None);
    let rect = |app: &mut VectorcraftApp, x: f64| {
        app.run("shape.rectangle", json!({"x": x, "y": 20, "width": 30, "height": 10})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b, g) = (rect(&mut app, 10.0), rect(&mut app, 60.0), rect(&mut app, 110.0));
    app.run("object.setPixelAligned", json!({"ids": [a], "on": true})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    assert_eq!(same(&mut app, "mixed"), Some((None, true)));
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(same(&mut app, "on"), Some((Some(true), true)));
    app.run("select.set", json!({"ids": [g]})).unwrap();
    app.run("view.guides.make", json!({})).unwrap();
    app.run("select.set", json!({"ids": [g, b]})).unwrap();
    assert_eq!(same(&mut app, "a guide"), Some((Some(false), false)));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let c = app.run("object.compoundPath.make", json!({})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(same(&mut app, "a compound path").map(|s| s.1), Some(true));
    let member = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(c)).unwrap().children().unwrap()[0].id.0;
    app.run("select.set", json!({"ids": [member]})).unwrap();
    assert_eq!(same(&mut app, "a path inside a compound path").map(|s| s.1), Some(false));
    app.run("select.set", json!({"ids": [c, member]})).unwrap();
    assert_eq!(same(&mut app, "a compound path and its member").map(|s| s.1), Some(false));
    app.run("select.none", json!({})).unwrap();
    assert_eq!(same(&mut app, "deselected"), None);
}

/// A frame of the panel measures the Align to Pixel Grid state once and keeps it for the document's
/// revision: later frames read it. An edit, Undo, Redo, a new selection, another document and a
/// document read back from its file measure it again.
#[test]
fn align_to_pixel_grid_state_is_measured_once_per_revision() {
    const PLANTED: Option<PixelAligned> = Some((None, false));
    fn key(app: &VectorcraftApp) -> (u64, u64) {
        app.session.active().map(|st| (st.uid, st.revision)).unwrap()
    }
    fn measured_again(app: &mut VectorcraftApp, why: &str, change: impl FnOnce(&mut VectorcraftApp)) {
        app.canvas.pixel_aligned = Some((key(app), PLANTED));
        change(app);
        let shown = pixel_aligned_shown(app);
        assert_ne!(shown, PLANTED, "{why}");
        assert_eq!(shown, pixel_aligned_state(app), "{why}");
    }
    let mut app = new_app();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 10})).unwrap()["id"].as_u64().unwrap();
    let b = app.run("shape.rectangle", json!({"x": 60, "y": 20, "width": 30, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let saved = (*app.session.active().unwrap().doc).clone();
    let mut p = panel(app);
    p.frame();
    let k = key(&p.app());
    assert_eq!(p.app().canvas.pixel_aligned, Some((k, Some((Some(false), true)))));
    // A wrong answer planted for this revision shows: the state is not measured again.
    p.app().canvas.pixel_aligned = Some((k, PLANTED));
    assert_eq!(pixel_aligned_shown(&mut p.app()), PLANTED);
    p.frame();
    assert_eq!(p.app().canvas.pixel_aligned, Some((k, PLANTED)));
    measured_again(&mut p.app(), "an edit", |app| {
        app.run("object.setPixelAligned", json!({"ids": [a], "on": true})).unwrap();
    });
    measured_again(&mut p.app(), "Undo", |app| {
        app.run("edit.undo", json!({})).unwrap();
    });
    measured_again(&mut p.app(), "Redo", |app| {
        app.run("edit.redo", json!({})).unwrap();
    });
    measured_again(&mut p.app(), "a new selection", |app| {
        app.run("select.set", json!({"ids": [a]})).unwrap();
    });
    measured_again(&mut p.app(), "a new document", |app| {
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 10})).unwrap();
    });
    measured_again(&mut p.app(), "back to the first document", |app| {
        app.session.set_active(0);
    });
    assert_eq!(pixel_aligned_shown(&mut p.app()), Some((Some(true), true)));
    measured_again(&mut p.app(), "the document read back", |app| {
        assert!(app.session.replace_document(0, saved));
    });
}

/// Align New Objects to Pixel Grid shows and toggles the active document's setting, follows a
/// switch to another document, and is unavailable with no document open.
#[test]
fn align_new_objects_menu_item_toggles_the_active_document() {
    const ID: &str = "ui.transform.alignNewObjects";
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    assert!(!enabled(&app, ID), "no document");
    // The command refuses with no document; the refusal is what is checked next.
    let _ = app.run(ID, json!({}));
    assert!(app.session.active().is_none());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("file.new", json!({"preset": "Web 1920×1080"})).unwrap();
    assert!(enabled(&app, ID));
    let on = |app: &VectorcraftApp, i: usize| app.session.documents()[i].doc.setup.align_new_objects_to_pixel_grid;
    assert_eq!((on(&app, 0), on(&app, 1)), (false, true));
    assert_eq!(checked(&app, ID), Some(true), "the Web document");
    app.session.set_active(0);
    assert_ne!(checked(&app, ID), Some(true), "the Letter-sized document");
    app.run(ID, json!({})).unwrap();
    assert_eq!((on(&app, 0), on(&app, 1)), (true, true));
    assert_eq!(checked(&app, ID), Some(true));
    let r = app.run("shape.rectangle", json!({"x": 10.3, "y": 20.6, "width": 30.4, "height": 9.7})).unwrap();
    let id = vectorcraft_doc::NodeId(r["id"].as_u64().unwrap());
    assert!(app.session.active().unwrap().doc.node(id).unwrap().extras().pixel_aligned);
    app.session.set_active(1);
    app.run(ID, json!({})).unwrap();
    assert_eq!((on(&app, 0), on(&app, 1)), (true, false));
}

/// An app with a document and the Transform panel closed.
fn app_without_transform_panel() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    if crate::panel_docking::visible(&app, "transform") {
        app.run("window.panel.close", json!({"panel": "transform"})).unwrap();
    }
    assert!(!crate::panel_docking::visible(&app, "transform"));
    app.session.take_shape_drawn();
    app
}

/// With Show on Shape Creation on, a shape drawn by a command or with OK in a shape tool's dialog
/// shows the closed Transform panel; with it off, after a canceled dialog, for other edits, for
/// saved steps and in Presentation Mode the panel stays closed.
#[test]
fn show_on_shape_creation_shows_the_transform_panel_after_a_shape() {
    let ctx = crate::testing::ui_context();
    let visible = |app: &VectorcraftApp| crate::panel_docking::visible(&app, "transform");
    let rect = json!({"x": 10, "y": 10, "width": 40, "height": 30});

    let mut app = app_without_transform_panel();
    app.run("shape.rectangle", rect.clone()).unwrap();
    show_on_shape_creation(&mut app, &ctx);
    assert!(!visible(&app), "off: the panel stays closed");

    let mut app = app_without_transform_panel();
    // A selected rectangle to move, drawn while the preference is off.
    let id = app.run("shape.rectangle", rect.clone()).unwrap()["id"].as_u64().unwrap();
    app.session.take_shape_drawn();
    set_pref(&mut app, "showOnShapeCreation", true);
    let x0 = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0;
    let before = x0(&app);
    app.run("object.move", json!({"dx": 5, "dy": 0})).unwrap();
    assert_eq!(x0(&app), before + 5.0, "the move moved the rectangle");
    show_on_shape_creation(&mut app, &ctx);
    assert!(!visible(&app), "an edit that draws no shape");
    app.replay("shape.rectangle", rect.clone()).unwrap();
    show_on_shape_creation(&mut app, &ctx);
    assert!(!visible(&app), "saved steps");
    // A dialog that is canceled draws nothing; OK draws the shape. The dialogs open in the window.
    app.adopt_context(&ctx);
    app.run("tool.select", json!({"tool": "ellipse"})).unwrap();
    crate::dialogs::open_tool_dialog(&mut app, "ellipse", json!({"x": 10.0, "y": 10.0}));
    cancel_dialog(&mut app);
    show_on_shape_creation(&mut app, &ctx);
    assert!(!visible(&app), "a canceled dialog");
    crate::dialogs::open_tool_dialog(&mut app, "ellipse", json!({"x": 10.0, "y": 10.0}));
    confirm_dialog(&mut app);
    show_on_shape_creation(&mut app, &ctx);
    assert!(visible(&app), "OK in the Ellipse dialog");
    // Shown already: another shape leaves it showing.
    app.run("shape.polygon", json!({"cx": 100, "cy": 100, "radius": 30})).unwrap();
    show_on_shape_creation(&mut app, &ctx);
    assert!(visible(&app), "a shape with the panel showing");

    // Presentation Mode keeps the panels hidden, also once it ends.
    let mut app = app_without_transform_panel();
    set_pref(&mut app, "showOnShapeCreation", true);
    app.ui.screen_mode = 3;
    app.run("shape.star", json!({"cx": 100, "cy": 100, "radius1": 30, "radius2": 15, "points": 5})).unwrap();
    show_on_shape_creation(&mut app, &ctx);
    app.ui.screen_mode = 0;
    show_on_shape_creation(&mut app, &ctx);
    assert!(!visible(&app), "Presentation Mode");

    // A floating Transform panel in front stays showing (Window › Transform would close it).
    let mut app = app_without_transform_panel();
    set_pref(&mut app, "showOnShapeCreation", true);
    app.run("window.panel.float", json!({"panel": "transform"})).unwrap();
    assert!(visible(&app));
    app.run("shape.line", json!({"x1": 0, "y1": 0, "x2": 50, "y2": 50})).unwrap();
    show_on_shape_creation(&mut app, &ctx);
    assert!(visible(&app), "a floating panel");
}

/// The app's frame shows the Transform panel after a shape is drawn, with Show on Shape Creation
/// on, and leaves it closed with it off.
#[test]
fn a_frame_after_a_shape_shows_the_transform_panel() {
    let ctx = crate::testing::ui_context();
    let rect = json!({"x": 10, "y": 10, "width": 40, "height": 30});
    for on in [false, true] {
        let mut app = app_without_transform_panel();
        set_pref(&mut app, "showOnShapeCreation", on);
        crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
        assert!(!crate::panel_docking::visible(&app, "transform"));
        app.run("shape.rectangle", rect.clone()).unwrap();
        crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
        assert_eq!(crate::panel_docking::visible(&app, "transform"), on, "on: {on}");
    }
}
