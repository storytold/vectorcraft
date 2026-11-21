//! The 3D dialog through the app frame: what its controls do to the document.
use egui::{Event, Key, Modifiers, PointerButton, vec2};
use serde_json::{Value, json};
use vectorcraft_effects::{
    CLASSIC_EXTRUDE, CLASSIC_REVOLVE, CLASSIC_ROTATE, EXTRUDE, INFLATE, REVOLVE, ROTATE, ROTATION_PRESETS, merged_params,
};
use vectorcraft_engine::Session;

use std::{cell::{RefCell, RefMut}, rc::Rc};

use craft::ui::testing::{Fixture, Target};

use super::super::{effect, three_d};
use crate::VectorcraftApp;

/// Drives the 3D dialog through the app's host: each frame pumps the session and the dialog host, and every
/// control is reached by its accessible name.
struct ThreeD {
    app: Rc<RefCell<VectorcraftApp>>,
    fixture: Fixture,
}

impl ThreeD {
    fn new(app: VectorcraftApp) -> Self {
        let app = Rc::new(RefCell::new(app));
        let shown = app.clone();
        let fixture = Fixture::new(move |ui| {
            let mut app = shown.borrow_mut();
            three_d::pump(&mut app, ui.ctx());
            // PLAN: assumed `DialogHost::pump(&mut self, &egui::Context)` as the host's own per-frame pump
            app.dialogs().pump(ui.ctx());
        });
        Self { app, fixture }
    }

    fn app(&self) -> RefMut<'_, VectorcraftApp> {
        self.app.borrow_mut()
    }

    fn open(&mut self, params: Value) {
        // PLAN: assumed `effect::open(&mut VectorcraftApp, &Value)`; the result is ignored, a failed open shows as a missing dialog below
        let _ = effect::open(&mut self.app(), &params);
        self.fixture.settle();
        assert!(self.app().three_d_dialog.is_some(), "the dialog opened");
    }

    /// Sets a key the shown page offers, as a command-driven edit does.
    fn set(&mut self, key: &str, value: Value) {
        // PLAN: assumed `HostedParamDialog::set_json_field(&str, Value)`
        three_d::with_fields(&mut self.app(), |d| {
            let _ = d.set_json_field(key, value);
        });
        self.fixture.settle();
    }

    fn fields(&self) -> craft::ui::patterns::form_dialogs::DialogFields {
        // PLAN: assumed `DialogFields: Clone`
        three_d::with_fields(&mut self.app(), |d| d.fields().clone())
    }

    fn json(&self, key: &str) -> Value {
        // PLAN: assumed `json_fields()` returns a `serde_json::Map<String, Value>`
        three_d::with_fields(&mut self.app(), |d| d.json_fields().get(key).cloned().unwrap_or(Value::Null))
    }

    /// The surfaces of the object the Map Art page shows.
    fn surfaces(&self) -> Option<Vec<vectorcraft_effects::Surface>> {
        // PLAN: assumed `Session.data.surfaces: Vec<Surface>` visible to this crate
        self.app().three_d_dialog.as_ref().map(|s| s.data.surfaces.clone())
    }

    fn target(&self, name: &str) -> Target {
        self.fixture.by_label(name).unwrap_or_else(|| panic!("no control named {name}"))
    }

    fn press(&mut self, name: &str) {
        let target = self.target(name);
        self.fixture.click(&target);
        self.fixture.settle();
    }

    fn ok(&mut self) {
        self.finish(true);
    }

    fn cancel(&mut self) {
        self.finish(false);
    }

    fn finish(&mut self, confirm: bool) {
        let ctx = self.fixture.ctx().clone();
        let mut app = self.app();
        let Some(id) = app.three_d_dialog.as_ref().map(|s| s.id) else { return };
        if confirm {
            app.dialogs().confirm_by_id(&ctx, id);
        } else {
            app.dialogs().cancel_by_id(&ctx, id);
        }
        drop(app);
        self.fixture.settle();
    }
}

const SQUARE: &str = "M100 100 L200 100 L200 200 L100 200 Z";

fn app_with(d: &str) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":300,"height":300})).unwrap();
    app.run("path.create", json!({"d": d})).unwrap();
    app.run("paint.setFill", json!({"color":"#338ad6"})).unwrap();
    app
}

/// The dialog of `effect` over a path drawn from `d`.
fn drawn(d: &str, effect: &str) -> ThreeD {
    let mut h = ThreeD::new(app_with(d));
    h.open(json!({"effect": effect, "item": null}));
    h
}

/// An app with a one-surface symbol "Mark" and a path drawn from `d`, selected.
fn with_symbol(d: &str) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":300,"height":300})).unwrap();
    let mark = app.run("path.create", json!({"d":"M250 250L260 250L260 270L250 270Z"})).unwrap()["id"].clone();
    app.run("threeD.addGraphic", json!({"name":"Mark","ids":[mark]})).unwrap();
    app.run("path.create", json!({"d": d})).unwrap();
    app
}

fn doc(h: &ThreeD) -> vectorcraft_doc::Document {
    h.app().session.doc().unwrap().doc.clone()
}

/// The effect's stored params with the defaults filled in.
fn stored(h: &ThreeD, effect: &str) -> Value {
    let app = h.app();
    let state = app.session.doc().unwrap();
    merged_params(effect, &state.doc.node(state.selection.objects[0]).unwrap().appearance.effects[0].params)
}

fn undo(h: &ThreeD) {
    h.app().run("edit.undo", json!({})).unwrap();
}

fn maps(h: &ThreeD) -> Vec<vectorcraft_effects::SurfaceMap> {
    vectorcraft_effects::surface_maps(&json!({"mapArt": h.json("mapArt")}))
}

fn number(h: &ThreeD, key: &str) -> f64 {
    h.fields().number(key).unwrap_or(f64::NAN)
}

fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(a), Some(b)) => (a - b).abs() < 1e-6,
        _ => a == b,
    }
}

/// `(effect, page, key, value)`: a setting each 3D effect shows, on the page that shows it.
// PLAN: the page names ("object", "rotation", "lighting") are assumed; `set` fails on a key the page does not show.
fn rows() -> Vec<(&'static str, &'static str, &'static str, Value)> {
    let [x, y, z] = ROTATION_PRESETS[14].1;
    vec![
        (REVOLVE, "object", "angle", json!(90.0)),
        (REVOLVE, "rotation", "rotationX", json!(30.0)),
        (REVOLVE, "rotation", "rotationY", json!(20.0)),
        (REVOLVE, "object", "cap", json!(false)),
        (REVOLVE, "object", "offset", json!(10.0)),
        (EXTRUDE, "lighting", "shadow", json!(true)),
        (EXTRUDE, "object", "bevel", json!("rounded")),
        (INFLATE, "object", "volume", json!(50.0)),
        (ROTATE, "rotation", "rotationX", json!(x)),
        (ROTATE, "rotation", "rotationY", json!(y)),
        (ROTATE, "rotation", "rotationZ", json!(z)),
        (CLASSIC_EXTRUDE, "object", "depth", json!(60.0)),
        (CLASSIC_REVOLVE, "object", "angle", json!(90.0)),
        (CLASSIC_ROTATE, "rotation", "rotationX", json!(30.0)),
        (CLASSIC_ROTATE, "lighting", "surface", json!("none")),
    ]
}

#[test]
fn a_3d_row_sets_its_param_previews_and_applies_in_one_step() {
    for (effect, page, key, value) in rows() {
        let name = format!("{effect}.{key}");
        for confirm in [false, true] {
            let mut h = drawn(SQUARE, effect);
            let before = doc(&h);
            h.set("__page", json!(page));
            h.set(key, value.clone());
            assert!(same(&stored(&h, effect)[key], &value), "{name}: the preview follows the row");
            if confirm {
                h.ok();
                assert!(same(&stored(&h, effect)[key], &value), "{name}: OK keeps it");
                undo(&h);
            } else {
                h.cancel();
            }
            assert_eq!(doc(&h), before, "{name}: {}", if confirm { "one undo step" } else { "Cancel restores" });
        }
    }
}

#[test]
fn map_art_maps_a_symbol_onto_a_surface_of_extrude_and_bevel_classic() {
    let app = with_symbol("M50 50L150 50L150 150L50 150Z");
    let mut h = ThreeD::new(app);
    h.open(json!({"effect":CLASSIC_EXTRUDE,"item":null}));
    h.set("__page", json!("mapArt"));
    let count = h.surfaces().map_or(0, |s| s.len());
    assert!(count >= 6, "a square's solid has at least six surfaces: {count}");
    // The Symbol menu maps the symbol onto the surface shown, the first.
    h.press("None");
    h.press("Mark");
    assert_eq!(maps(&h).iter().map(|m| (m.surface, m.symbol.as_str(), m.scale_x)).collect::<Vec<_>>(), [(1, "Mark", 100.0)]);
    h.set("__mapSurface", json!(2));
    assert_eq!(maps(&h).len(), 1, "the next surface has no map until it is given one");
    h.set("__mapSurface", json!(1));
    h.press("Scale to Fit");
    assert!(maps(&h)[0].scale_x > 100.0, "the 10-point art is scaled up to the 100-point cap: {:?}", maps(&h));
    h.press("Done");
    assert_ne!(h.fields().text("__page"), Some("mapArt"), "Done goes back to the effect's options");
    h.ok();
    let app = h.app();
    let state = app.session.doc().unwrap();
    let params = &state.doc.node(state.selection.objects[0]).unwrap().appearance.effects[0].params;
    assert_eq!(vectorcraft_effects::surface_maps(params).len(), 1, "{params}");
    assert!(params.get("__mapArt").is_none() && params.get("__mapSurface").is_none(), "the dialog's own state stays out of the effect");
}

#[test]
fn map_art_maps_no_more_surfaces_than_an_object_takes() {
    let max = vectorcraft_effects::MAX_MAPS;
    let n = max + 1;
    let corners: Vec<String> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            format!("{:.2} {:.2}", 150.0 + 100.0 * a.cos(), 150.0 + 100.0 * a.sin())
        })
        .collect();
    let mut h = ThreeD::new(with_symbol(&format!("M{}Z", corners.join(" L"))));
    h.open(json!({"effect":CLASSIC_EXTRUDE,"item":null}));
    h.set("__page", json!("mapArt"));
    assert!(h.surfaces().map_or(0, |s| s.len()) > max);
    // Every map the object takes is used, and the surface shown has none.
    let full: Vec<Value> = (1..=max).map(|surface| json!({"surface": surface, "symbol": "Mark"})).collect();
    h.set("mapArt", json!(full));
    h.set("__mapSurface", json!(max + 1));
    h.press("None");
    if let Some(mark) = h.fixture.by_label("Mark") {
        h.fixture.click(&mark);
    }
    assert_eq!(maps(&h).len(), max, "no map past the limit is written");
}

#[test]
fn map_art_surfaces_follow_the_active_document() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    for d in ["M50 50L150 50L150 150L50 150Z", "M50 50L80 50L80 200L50 200Z"] {
        app.run("file.new", json!({"width":300,"height":300})).unwrap();
        app.run("path.create", json!({"d": d})).unwrap();
    }
    let key = |h: &ThreeD| {
        let app = h.app();
        let st = app.session.active().unwrap();
        (st.selection.objects.first().copied(), st.revision)
    };
    let mut h = ThreeD::new(app);
    h.open(json!({"effect":CLASSIC_EXTRUDE,"item":null}));
    h.set("__page", json!("mapArt"));
    let second = h.surfaces().unwrap();
    let at_second = key(&h);
    assert!(h.app().session.set_active(0));
    h.fixture.settle();
    assert_eq!(key(&h), at_second, "both documents' objects have the same id and revision");
    let first = h.surfaces().unwrap();
    assert_ne!(first, second, "the two objects' solids differ in size");
    assert!(h.app().session.set_active(1));
    h.fixture.settle();
    assert_eq!(h.surfaces().unwrap(), second, "back on the second document its own surfaces");
}

#[test]
fn automatic_smooth_shading_previews_saves_and_respects_cancel_and_one_step_undo() {
    let gradients = |h: &ThreeD| {
        let app = h.app();
        let state = app.session.doc().unwrap();
        let art = vectorcraft_effects::revolve_art(state.doc.node(state.selection.objects[0]).unwrap()).unwrap();
        let mut count = 0;
        art.walk(&mut |face| {
            count += face
                .appearance
                .items
                .iter()
                .filter(|item| matches!(item, vectorcraft_doc::AppearanceItem::Fill(f) if matches!(f.paint, vectorcraft_doc::color::Paint::Gradient(_))))
                .count();
        });
        count
    };
    for confirm in [false, true] {
        let mut h = drawn("M150 40 L180 60 L180 140 L150 160", REVOLVE);
        let before = doc(&h);
        h.set("__page", json!("lighting"));
        assert!(!h.fields().contains("smoothShading"));
        assert!(gradients(&h) > 0, "default rendering uses vector gradients");
        h.set("shade", json!(false));
        assert_eq!(gradients(&h), 0, "Shading still controls all lighting");
        h.set("shade", json!(true));
        assert!(gradients(&h) > 0);
        h.set("preview", json!(false));
        assert_eq!(doc(&h), before, "no preview while Preview is off");
        h.set("preview", json!(true));
        if confirm {
            h.ok();
            let app = h.app();
            let state = app.session.doc().unwrap();
            let node = state.doc.node(state.selection.objects[0]).unwrap();
            let loaded = vectorcraft_format::load(&vectorcraft_format::save(&state.doc, false)).unwrap();
            assert!(!node.appearance.effects[0].params.as_object().unwrap().contains_key("smoothShading"));
            assert_eq!(vectorcraft_effects::revolve_art(loaded.node(node.id).unwrap()), vectorcraft_effects::revolve_art(node));
            drop(app);
            undo(&h);
        } else {
            h.cancel();
        }
        assert_eq!(doc(&h), before);
    }
}

#[test]
fn advanced_visibility_option_is_saved_resettable_and_leaves_preview_geometry_complete() {
    let mut h = drawn("M150 40 L180 60 L180 140 L150 160", REVOLVE);
    let art = |h: &ThreeD| {
        let app = h.app();
        let state = app.session.doc().unwrap();
        vectorcraft_effects::revolve_art(state.doc.node(state.selection.objects[0]).unwrap()).unwrap()
    };
    let before = art(&h);
    h.set("expandVisibleOnly", json!(false));
    assert_eq!(stored(&h, REVOLVE)["expandVisibleOnly"], json!(false));
    assert_eq!(art(&h), before, "the option does not change the preview geometry");
    h.press("Reset all");
    assert_eq!(h.fields().bool("expandVisibleOnly"), Some(true));
    h.ok();
    let app = h.app();
    let state = app.session.doc().unwrap();
    let loaded = vectorcraft_format::load(&vectorcraft_format::save(&state.doc, false)).unwrap();
    assert_eq!(loaded.node(state.selection.objects[0]).unwrap().appearance.effects[0].params["expandVisibleOnly"], json!(true));
}

fn button(pos: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// The classic effects' track cube: a drag on its face turns the object live, Escape during a drag puts the angles
/// back, a key turns Z by a degree, and OK keeps it all as one undo step.
#[test]
fn the_track_cube_turns_the_classic_effects_live_in_one_undo_step() {
    let mut h = drawn(SQUARE, CLASSIC_ROTATE);
    let before = doc(&h);
    h.set("__page", json!("rotation"));
    let angles = |h: &ThreeD| ["rotationX", "rotationY", "rotationZ"].map(|k| number(h, k));
    let start = angles(&h);
    // PLAN: assumed the cube's accessible name "Track cube"
    let cube = h.target("Track cube");
    h.fixture.drag(&cube, vec2(30.0, 0.0));
    let turned = angles(&h);
    assert_ne!(turned, start, "the drag turned the object");
    assert_eq!(stored(&h, CLASSIC_ROTATE)["rotationY"].as_f64(), Some(turned[1]), "the preview follows the cube");
    // Escape during a drag puts the angles back and keeps the dialog open.
    let p = cube.center();
    h.fixture.frame(vec![Event::PointerMoved(p), button(p, true)]);
    h.fixture.frame(vec![Event::PointerMoved(p + vec2(0.0, 30.0))]);
    h.fixture.frame(vec![Event::PointerMoved(p + vec2(0.0, 40.0))]);
    assert_ne!(number(&h, "rotationX"), turned[0]);
    h.fixture.key(Key::Escape, Modifiers::NONE);
    h.fixture.frame(vec![button(p + vec2(0.0, 40.0), false)]);
    h.fixture.settle();
    assert!(h.app().three_d_dialog.is_some(), "Escape ends the drag, not the dialog");
    assert_eq!(angles(&h), turned);
    h.fixture.key(Key::PageUp, Modifiers::NONE);
    h.fixture.settle();
    assert!((number(&h, "rotationZ") - (turned[2] - 1.0)).abs() < 0.011, "Page Up turns 1° counterclockwise");
    let last = number(&h, "rotationZ");
    h.ok();
    assert_eq!(stored(&h, CLASSIC_ROTATE)["rotationZ"].as_f64(), Some(last));
    undo(&h);
    assert_eq!(doc(&h), before);
}

/// A mixed rotation (the 3D and Materials panel's selected effects differ) leaves the cube off. It is tested at the
/// row: `effect::open` never marks a value mixed, only the panel does.
#[test]
fn the_track_cube_is_off_while_a_rotation_is_mixed() {
    use std::{cell::RefCell, rc::Rc};
    let fields = Rc::new(RefCell::new(craft::ui::patterns::form_dialogs::DialogFields::from_pairs([
        ("rotationX", 0.0),
        ("rotationY", 10.0),
        ("rotationZ", 0.0),
    ])));
    fields.borrow_mut().set_mixed("rotationX");
    let shown = fields.clone();
    let params = [super::super::three_d_classic::track_cube::row()];
    // PLAN: assumed `ParamRows::new(label, &[Param], Bind<DialogFields>)` and `Bind::new(&mut T)`
    let mut fixture = craft::ui::testing::Fixture::new(move |ui| {
        craft::ui::patterns::form_dialogs::ParamRows::new("Rotation", &params, craft::ui::contract::Bind::new(&mut shown.borrow_mut())).show(ui);
    });
    fixture.settle();
    let cube = fixture.by_label("Track cube").expect("the cube is shown");
    fixture.drag(&cube, vec2(30.0, 0.0));
    let f = fields.borrow();
    assert_eq!(["rotationX", "rotationY", "rotationZ"].map(|k| f.number(k)), [Some(0.0), Some(10.0), Some(0.0)]);
    assert!(f.is_mixed("rotationX"), "the drag leaves the mixed rotation mixed");
}

#[test]
fn the_bevel_reset_restores_the_bevel_settings_and_keeps_the_others() {
    let mut h = drawn(SQUARE, EXTRUDE);
    let before = doc(&h);
    h.set("__page", json!("object"));
    let changed = json!({"bevel": "step", "bevelWidth": 12.0, "bevelHeight": 3.0, "bevelRepeat": 4, "bevelSpace": 30.0, "bevelInside": true, "bevelBothSides": true, "depth": 90.0, "twist": 45.0});
    for (key, value) in changed.as_object().unwrap() {
        h.set(key, value.clone());
    }
    h.set("__page", json!("rotation"));
    h.set("rotationX", json!(10.0));
    h.set("__page", json!("object"));
    // PLAN: assumed the button's name "Reset" (the bevel group's)
    h.press("Reset");
    let defaults = vectorcraft_effects::default_params(EXTRUDE).unwrap();
    let fields = h.fields();
    for key in ["bevel", "bevelWidth", "bevelHeight", "bevelRepeat", "bevelSpace", "bevelInside", "bevelBothSides"] {
        assert!(same(&h.json(key), &defaults[key]), "reset: {key}");
    }
    for (key, want) in [("depth", 90.0), ("twist", 45.0), ("rotationX", 10.0)] {
        assert_eq!(fields.number(key), Some(want), "kept: {key}");
    }
    h.ok();
    let params = stored(&h, EXTRUDE);
    assert_eq!((params["depth"].clone(), params["bevelBothSides"].clone(), params["bevel"].clone()), (json!(90.0), json!(false), json!("none")));
    undo(&h);
    assert_eq!(doc(&h), before);
}

#[test]
fn reset_all_keeps_map_art_and_its_options() {
    let mut h = drawn(SQUARE, CLASSIC_EXTRUDE);
    let maps = json!([{"surface": 1, "symbol": "Mark"}]);
    h.set("mapArt", maps.clone());
    h.set("mapShade", json!(true));
    h.set("mapInvisible", json!(true));
    h.set("__page", json!("rotation"));
    h.set("rotationX", json!(40.0));
    h.press("Reset all");
    assert_eq!(number(&h, "rotationX"), -21.88, "the view is reset");
    assert_eq!(h.json("mapArt"), maps, "the maps stay");
    let fields = h.fields();
    assert!(fields.bool("mapShade") == Some(true) && fields.bool("mapInvisible") == Some(true), "Shade Artwork and Invisible Geometry stay");
    h.cancel();
}

#[test]
fn light_list_add_select_edit_delete_and_reset_keep_each_light_independent() {
    for effect in [REVOLVE, EXTRUDE] {
        let mut h = drawn(SQUARE, effect);
        let before = doc(&h);
        h.set("__page", json!("lighting"));
        let one = doc(&h);
        h.press("Delete Light");
        assert_eq!(doc(&h), one, "retain at least one light");
        h.press("Add Light");
        let lights = h.json("lights");
        let first = lights[0].clone();
        assert_eq!(lights.as_array().map(Vec::len), Some(2));
        assert_eq!(h.json("__light"), json!(1));
        assert_eq!(number(&h, "lightHeight"), 90.0, "a new light starts at Height 90");
        h.set("lightHeight", json!(55.0));
        let lights = h.json("lights");
        assert_eq!(lights[1]["height"], json!(55.0));
        assert_eq!(lights[0], first, "an edit changes only the selected light");
        let two = doc(&h);
        h.press("Light 2");
        h.press("Light 1");
        assert_eq!(doc(&h), two, "selection changes no effect params");
        assert_eq!(h.json("__light"), json!(0));
        h.press("Delete Light");
        assert_eq!(h.json("lights").as_array().map(Vec::len), Some(1));
        h.press("Reset all");
        assert!(h.json("lights").is_null());
        h.press("Add Light");
        h.ok();
        undo(&h);
        assert_eq!(doc(&h), before, "whole dialog is one undo step");
    }
}

#[test]
fn light_list_stops_at_ten_and_cancel_restores_the_source() {
    let mut h = drawn("M150 40 L180 60 L180 140 L150 160", REVOLVE);
    let before = doc(&h);
    h.set("__page", json!("lighting"));
    for _ in 0..11 {
        h.press("Add Light");
    }
    assert_eq!(h.json("lights").as_array().map(Vec::len), Some(10));
    assert_eq!(h.json("__light"), json!(9));
    h.cancel();
    assert_eq!(doc(&h), before);
}

#[test]
fn lighting_presets_preview_custom_edits_reset_and_commit_as_one_undo_step() {
    let mut h = drawn(SQUARE, REVOLVE);
    let before = doc(&h);
    h.set("__page", json!("lighting"));
    let view = h.json("rotationX");
    let pick = |h: &mut ThreeD, from: &str, to: &str| {
        h.press(from);
        h.press(to);
    };
    pick(&mut h, "Standard", "Right");
    assert_eq!(number(&h, "lightRotation"), 0.0);
    assert_eq!(h.json("rotationX"), view, "a preset keeps the view");
    h.set("lightHeight", json!(70.0));
    h.target("Custom");
    pick(&mut h, "Custom", "Diffuse");
    assert_eq!((number(&h, "lightSoftness"), number(&h, "ambient")), (100.0, 35.0));
    h.press("Add Light");
    h.target("Custom");
    pick(&mut h, "Custom", "Top Left");
    assert!(h.json("lights").is_null());
    assert_eq!((h.json("__light"), number(&h, "lightRotation")), (json!(0), 135.0));
    h.press("Reset all");
    h.target("Standard");
    pick(&mut h, "Standard", "Right");
    h.ok();
    undo(&h);
    assert_eq!(doc(&h), before);
}

#[test]
fn lighting_presets_cancel_restores_an_existing_custom_light_rig() {
    let mut h = drawn("M150 40 L180 60 L180 140 L150 160", REVOLVE);
    h.ok();
    h.app()
        .run("effect.setParams", json!({"index":0,"item":null,"params":{"ambient":19,"lights":[{"color":"#ff8040"},{"color":"#4080ff"}]}}))
        .unwrap();
    let before = doc(&h);
    h.open(json!({"effect":REVOLVE,"index":0,"item":null}));
    h.set("__page", json!("lighting"));
    h.press("Custom");
    h.press("Right");
    assert_ne!(doc(&h), before);
    h.cancel();
    assert_eq!(doc(&h), before);
}
