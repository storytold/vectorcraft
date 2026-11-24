//! The Blend panel drawn headless: its controls do nothing until the selection allows them. Make
//! needs two or more objects, Spacing and Orientation need a blend or two or more objects, and the
//! other buttons need a blend. Make blends the selection, Create Spine sets whether new blends get a
//! spine, and Spacing, Orientation, Type, Ramp and Shift edit the selected blend through
//! `object.blend.options`.

use serde_json::{Value, json};
use vectorcraft_doc::live::{BlendOrientation, BlendSpacing};
use vectorcraft_doc::{BlendSpec, NodeId, NodeKind};
use vectorcraft_engine::Session;

use super::blend::show;
use crate::VectorcraftApp;
use crate::tests::panel_harness::Panel;

/// A document with two 20 pt squares 200 pt apart, both selected.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("shape.rectangle", json!({"x": 200, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app
}

/// [`app`] with the squares blended at Smooth Color; the blend is selected.
fn blended() -> (VectorcraftApp, NodeId) {
    let mut app = app();
    let id = app.run("object.blend.make", json!({"smooth": true})).unwrap()["id"].as_u64().unwrap();
    (app, NodeId(id))
}

/// [`app`] with an Align to Path blend, which has a spine for Type and Ramp.
fn pathed() -> (VectorcraftApp, NodeId) {
    let mut app = app();
    let id = app.run("object.blend.make", json!({"smooth": true, "orientation": "path"})).unwrap()["id"].as_u64().unwrap();
    (app, NodeId(id))
}

/// Blends `a` (with a spine, Align to Path) and `b` (with `b_params`), both selected.
fn two_blends(b_params: Value) -> (VectorcraftApp, NodeId, NodeId) {
    let mut app = app();
    let a = NodeId(app.run("object.blend.make", json!({"smooth": true, "orientation": "path"})).unwrap()["id"].as_u64().unwrap());
    let ids: Vec<Value> =
        [0, 200].map(|x| app.run("shape.rectangle", json!({"x": x, "y": 100, "width": 20, "height": 20})).unwrap()["id"].clone()).into();
    app.run("select.set", json!({"ids": ids})).unwrap();
    let b = NodeId(app.run("object.blend.make", b_params).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    (app, a, b)
}

fn spec(app: &VectorcraftApp, id: NodeId) -> BlendSpec {
    match &app.session.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Blend { spec, .. } => (**spec).clone(),
        k => panic!("not a blend: {k:?}"),
    }
}

fn undo_depth(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// The top-level objects of the first layer.
fn top(app: &VectorcraftApp) -> Vec<(NodeId, bool)> {
    let doc = &app.session.active().unwrap().doc;
    doc.layers[0].children().unwrap().iter().map(|n| (n.id, matches!(n.kind, NodeKind::Blend { .. }))).collect()
}

fn info(p: &Panel) -> Value {
    p.app().session.execute("object.blend.info", &json!({})).unwrap()
}

#[test]
fn the_controls_are_dimmed_until_a_blend_or_two_objects_are_selected() {
    let mut app = app();
    app.run("select.none", json!({})).unwrap();
    let mut p = Panel::new(app, show);
    p.frame();
    for name in ["Make", "Spacing:", "Reverse Front to Back"] {
        assert!(!p.enabled(name), "{name} with nothing selected");
    }
    p.app().run("select.all", json!({})).unwrap();
    p.frame();
    assert!(p.enabled("Make") && p.enabled("Spacing:"), "two objects");
    assert!(!p.enabled("Reverse Front to Back"), "no blend");
    p.click("Make");
    p.frame();
    assert!(p.enabled("Reverse Front to Back") && p.enabled("Spacing:"), "a blend");
    assert!(!p.enabled("Make"), "one object");
}

#[test]
fn make_blends_the_selected_objects_in_one_undo_step() {
    let app = app();
    let depth = undo_depth(&app);
    let mut p = Panel::new(app, show);
    p.click("Make");
    let app = p.app();
    let objects = top(&app);
    assert_eq!(objects.len(), 1, "{objects:?}");
    assert!(objects[0].1, "a blend");
    assert_eq!(undo_depth(&app), depth + 1);
}

#[test]
fn spacing_and_orientation_edit_the_selected_blend() {
    let (app, id) = blended();
    let depth = undo_depth(&app);
    let mut p = Panel::new(app, show);
    p.click("Spacing:");
    p.click("Specified Steps");
    assert_eq!(spec(&p.app(), id).spacing, BlendSpacing::Steps(5));
    assert_eq!(undo_depth(&p.app()), depth + 1, "one undo step");
    p.click("Spacing:");
    p.click("Specified Distance");
    assert_eq!(spec(&p.app(), id).spacing, BlendSpacing::Distance(10.0));
    p.click("Align to Path");
    assert_eq!(spec(&p.app(), id).orientation, BlendOrientation::AlignToPath);
    assert_eq!(spec(&p.app(), id).spacing, BlendSpacing::Distance(10.0), "Orientation keeps the spacing");
    p.app().run("edit.undo", json!({})).unwrap();
    assert_eq!(spec(&p.app(), id).orientation, BlendOrientation::AlignToPage);
}

#[test]
fn create_spine_sets_what_new_blends_get_whatever_is_selected() {
    let (app, id) = blended();
    let (before, depth) = (spec(&app, id), undo_depth(&app));
    let mut p = Panel::new(app, show);
    let mode = |p: &Panel| p.app().session.execute("object.blend.createSpine", &json!({})).unwrap()["mode"].clone();
    p.click("Create Spine:");
    p.click("Never");
    assert_eq!(mode(&p), "never");
    assert_eq!(
        {
            let app = p.app();
            (spec(&app, id), undo_depth(&app))
        },
        (before, depth),
        "the selected blend and the undo history stay as they were"
    );
    // Nothing selected: the select still sets the setting.
    p.app().run("select.none", json!({})).unwrap();
    p.click("Create Spine:");
    p.click("Always");
    assert_eq!(mode(&p), "always");
    p.click("Create Spine:");
    p.click("Never");
    // A blend made now has no spine: Reverse Spine and Ramp are dimmed for it.
    p.app().run("edit.undo", json!({})).unwrap();
    p.app().run("select.all", json!({})).unwrap();
    p.click("Make");
    let (g, blend) = top(&p.app())[0];
    assert!(blend && spec(&p.app(), g).no_spine, "{:?}", top(&p.app()));
    p.frame();
    assert!(!p.enabled("Reverse Spine") && !p.enabled("Type:") && !p.enabled("Ramp:"));
}

#[test]
fn panel_easing_type_ramp_and_shift_edit_the_blend_and_undo() {
    // Type and Ramp need a spine: Align to Path gives the blend one.
    let (app, id) = pathed();
    let mut p = Panel::new(app, show);
    p.click("Type:");
    p.click("Ease Out In");
    assert_eq!(spec(&p.app(), id).ease.kind, vectorcraft_doc::BlendEaseType::OutIn);
    for (name, key) in [("Ramp:", "ramp"), ("Shift:", "colorShift")] {
        let (before, depth) = (info(&p)[key].clone(), undo_depth(&p.app()));
        p.type_into(name, "30");
        assert_ne!(info(&p)[key], before, "{key}");
        assert_eq!(undo_depth(&p.app()), depth + 1, "{key} is one undo step");
        p.app().run("edit.undo", json!({})).unwrap();
        assert_eq!(info(&p)[key], before);
    }
}

#[test]
fn a_ramp_drag_is_one_undo_step() {
    let (app, id) = pathed();
    let (before, depth) = (spec(&app, id), undo_depth(&app));
    let mut p = Panel::new(app, show);
    // A drag along the slider from its middle to the right, the button held.
    p.drag_hold_role("Ramp:", craft::ui::contract::Role::SLIDER, egui::vec2(60.0, 0.0));
    // While the button is down the blend shows the new value, and no undo step is recorded yet.
    assert!(p.app().session.in_interaction(), "the drag is in progress");
    assert_ne!(spec(&p.app(), id), before, "the drag previews on the blend");
    assert_eq!(undo_depth(&p.app()), depth);
    p.release();
    assert!(!p.app().session.in_interaction(), "the release ends the drag");
    assert!(spec(&p.app(), id).ease.ramp > before.ease.ramp);
    assert_eq!(undo_depth(&p.app()), depth + 1, "the whole drag is one undo step");
    p.app().run("edit.undo", json!({})).unwrap();
    assert_eq!(spec(&p.app(), id), before);
}

#[test]
fn ramp_and_shift_edit_every_selected_blend_and_keep_their_other_options() {
    let mut app = app();
    let a = NodeId(app.run("object.blend.make", json!({"spacing": "steps", "steps": 3, "orientation": "path"})).unwrap()["id"].as_u64().unwrap());
    let ids: Vec<Value> =
        [0, 200].map(|x| app.run("shape.rectangle", json!({"x": x, "y": 100, "width": 20, "height": 20})).unwrap()["id"].clone()).into();
    app.run("select.set", json!({"ids": ids})).unwrap();
    let b =
        NodeId(app.run("object.blend.make", json!({"spacing": "distance", "distance": 12, "orientation": "path"})).unwrap()["id"].as_u64().unwrap());
    app.run("object.blend.setEasing", json!({"id": b.0, "easingType": "easeInOut", "colorShift": -40})).unwrap();
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    let mut p = Panel::new(app, show);
    for name in ["Ramp:", "Shift:"] {
        let (before_a, before_b, depth) = {
            let app = p.app();
            (spec(&app, a), spec(&app, b), undo_depth(&app))
        };
        p.type_into(name, "75");
        let (after_a, after_b) = {
            let app = p.app();
            (spec(&app, a), spec(&app, b))
        };
        assert_eq!(undo_depth(&p.app()), depth + 1, "one undo step for both blends");
        // The value typed goes to both; each keeps its own spacing, type and the other value.
        if name == "Ramp:" {
            assert!(after_a.ease.ramp > 0.0 && after_a.ease.ramp == after_b.ease.ramp, "{after_a:?} {after_b:?}");
            assert_eq!((after_a.color_shift, after_b.color_shift), (before_a.color_shift, before_b.color_shift));
        } else {
            assert!(after_a.color_shift > 0.0 && after_a.color_shift == after_b.color_shift, "{after_a:?} {after_b:?}");
            assert_eq!((after_a.ease.ramp, after_b.ease.ramp), (before_a.ease.ramp, before_b.ease.ramp));
        }
        assert_eq!((after_a.spacing, after_b.spacing), (BlendSpacing::Steps(3), BlendSpacing::Distance(12.0)));
        assert_eq!((after_a.ease.kind, after_b.ease.kind), (before_a.ease.kind, before_b.ease.kind));
    }
}

#[test]
fn ramp_is_dimmed_while_a_selected_blend_has_no_spine() {
    // The first selected blend has a spine and the second has none: the engine refuses easing for
    // the pair, so Ramp is dimmed; Shift still edits both.
    let (app, a, b) = two_blends(json!({"smooth": true, "createSpine": "never"}));
    let mut p = Panel::new(app, show);
    p.frame();
    assert!(!p.enabled("Ramp:") && p.enabled("Shift:"));
    p.type_into("Shift:", "75");
    let app = p.app();
    assert!(spec(&app, a).color_shift > 0.0 && spec(&app, b).color_shift == spec(&app, a).color_shift);
}

#[test]
fn ramp_and_reverse_spine_are_dimmed_while_a_selected_key_belongs_to_a_blend_with_no_spine() {
    // A key of each blend is selected, as Direct Selection does: the commands act on the keys'
    // blends, and the second has no spine, so Ramp and Reverse Spine are dimmed.
    let (mut app, a, b) = two_blends(json!({"smooth": true, "createSpine": "never"}));
    let key = |app: &VectorcraftApp, blend: NodeId| match &app.session.active().unwrap().doc.node(blend).unwrap().kind {
        NodeKind::Blend { children, .. } => children[0].id,
        k => panic!("not a blend: {k:?}"),
    };
    let keys = [key(&app, a).0, key(&app, b).0];
    app.run("select.set", json!({"ids": keys})).unwrap();
    assert_eq!(app.session.execute("object.blend.info", &json!({})).unwrap()["target"], "blend");
    let mut p = Panel::new(app, show);
    p.frame();
    assert!(!p.enabled("Ramp:") && !p.enabled("Reverse Spine"));
}
