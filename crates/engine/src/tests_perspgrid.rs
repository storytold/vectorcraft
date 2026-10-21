//! Perspective Grid definition, presets and view options (`perspective.grid.get` / `define`,
//! `perspective.presets.*`, View → Perspective Grid toggles, the grid widgets).

use serde_json::{Value, json};
use vectorcraft_tools::distort::perspective::{PerspectiveGrid, Rgb};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn grid(s: &Session) -> PerspectiveGrid {
    PerspectiveGrid::effective(&s.doc().unwrap().doc)
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// `cmd` with `p` fails with the error for `key`, which `p` gives as something other than true or false.
fn refuses_non_boolean(s: &mut Session, cmd: &str, p: &Value, key: &str) {
    let r = s.execute(cmd, p);
    let want = format!("`{key}` must be true or false");
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{cmd} {p}: {r:?}");
}

/// Values for an on/off param that are not true, false or null.
fn not_booleans() -> [Value; 4] {
    [json!("true"), json!(1), json!("false"), json!([true])]
}

// ---------- M8.8: Define Grid ----------

#[test]
fn grid_get_reports_the_model_the_define_fields_and_the_station_point() {
    let mut s = session();
    let r = run(&mut s, "perspective.grid.get", json!({}));
    assert_eq!(r["defined"], json!(false), "the default grid until one is set");
    assert_eq!(r["grid"]["kind"], json!(2));
    assert_eq!(r["define"]["name"], json!("[2P-Normal View]"));
    assert_eq!((r["define"]["angle"].clone(), r["define"]["units"].clone()), (json!(45.0), json!("points")));
    let g = grid(&s);
    assert!(near(r["station"]["x"].as_f64().unwrap(), g.origin[0]), "the normal view looks at the corner");
    assert!(near(r["station"]["distance"].as_f64().unwrap(), (g.vp_right - g.vp_left) / 2.0));
    // A query: nothing to undo.
    assert_eq!(undo_len(&s), 0);
    run(&mut s, "perspective.grid.preset", json!({"kind": 1}));
    assert_eq!(run(&mut s, "perspective.grid.get", json!({}))["defined"], json!(true));
}

#[test]
fn define_sets_the_grid_from_the_dialog_fields_in_one_undo_step() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let before = grid(&s);
    let n = undo_len(&s);
    let r = run(
        &mut s,
        "perspective.grid.define",
        json!({"units": "inches", "gridline": 0.5, "angle": 30, "distance": 4, "horizonHeight": 2, "leftColor": "#00ff00", "opacity": 75}),
    );
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(r["units"], json!("inches"));
    let g = grid(&s);
    assert!(near(g.cell, 36.0) && near(g.origin[1] - g.horizon, 144.0));
    let st = g.station();
    assert!(near(st.distance, 288.0) && near(st.x, before.station().x), "the viewer stays put");
    assert!(near(g.viewing_angle(), 30.0));
    assert_eq!((g.left_color, g.opacity, g.name.as_str()), (Rgb([0, 255, 0]), 75.0, ""), "no longer the preset");
    // Unchanged fields leave the grid alone: no undo step.
    let same = run(&mut s, "perspective.grid.get", json!({}))["define"].clone();
    run(&mut s, "perspective.grid.define", same);
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(grid(&s), before);
    // Bad values are refused and change nothing.
    for bad in [json!({"angle": 0}), json!({"distance": -1}), json!({"units": "parsecs"}), json!({"kind": 5}), json!({"rightColor": "red"})] {
        assert!(s.execute("perspective.grid.define", &bad).is_err(), "{bad}");
    }
    assert_eq!(grid(&s), before);
}

#[test]
fn define_switches_the_type_around_the_station_point() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let st = grid(&s).station();
    run(&mut s, "perspective.grid.define", json!({"kind": 1}));
    let g = grid(&s);
    assert_eq!(g.kind, 1);
    assert!(near(g.vp_left, st.x) && near(g.distance, st.distance), "one-point looks straight at the station's centre of vision");
    run(&mut s, "perspective.grid.define", json!({"kind": 3, "thirdVp": [0, 500]}));
    let g = grid(&s);
    assert!(near(g.vp_vertical[0], g.station().x) && near(g.horizon - g.vp_vertical[1], 500.0));
}

#[test]
fn grid_definition_fields_survive_a_native_round_trip() {
    let mut s = session();
    run(&mut s, "perspective.grid.define", json!({"units": "cm", "scale": [1, 4], "angle": 40, "groundColor": "#123456", "opacity": 20}));
    let doc = s.doc().unwrap().doc.clone();
    vectorcraft_testkit::invariants::check_native_roundtrip_exact(&doc).unwrap();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(PerspectiveGrid::from_doc(&back), PerspectiveGrid::from_doc(&doc));
    let g = PerspectiveGrid::from_doc(&back).unwrap();
    assert_eq!((g.units.as_str(), g.scale, g.ground_color, g.opacity), ("centimeters", [1.0, 4.0], Rgb([0x12, 0x34, 0x56]), 20.0));
    // A grid saved before these fields loads with their defaults.
    let mut old = serde_json::to_value(&g).unwrap();
    for k in ["name", "angle", "units", "scale", "leftColor", "rightColor", "groundColor", "opacity"] {
        old.as_object_mut().unwrap().remove(k);
    }
    let g: PerspectiveGrid = serde_json::from_value(old).unwrap();
    assert_eq!((g.angle, g.units.as_str(), g.scale, g.opacity), (None, "points", [1.0, 1.0], 50.0));
}

// ---------- M8.9: presets ----------

fn preset_names(s: &mut Session) -> Vec<String> {
    let r = run(s, "perspective.presets.list", json!({}));
    r["presets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn built_in_views_apply_by_name_or_type() {
    let mut s = session();
    let names = preset_names(&mut s);
    assert_eq!(&names[..3], ["[1P-Normal View]", "[1P-Low View]", "[1P-High View]"]);
    assert!(names.contains(&"[3P-Normal View]".to_string()));
    let r = run(&mut s, "perspective.presets.list", json!({}));
    assert!(r["presets"].as_array().unwrap().iter().all(|p| p["builtIn"] == json!(true)));
    run(&mut s, "perspective.grid.preset", json!({"name": "[2P-low view]"}));
    let low = grid(&s);
    assert_eq!((low.name.as_str(), low.kind, low.visible), ("[2P-Low View]", 2, true));
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let normal = grid(&s);
    assert_eq!(normal.name, "[2P-Normal View]");
    assert!(low.origin[1] - low.horizon < normal.origin[1] - normal.horizon, "the low view's horizon is nearer the ground");
    assert!(s.execute("perspective.grid.preset", &json!({"name": "[9P-Fisheye]"})).is_err());
    assert!(s.execute("perspective.grid.preset", &json!({})).is_err());
}

#[test]
fn presets_save_rename_delete_and_protect_the_built_in_ones() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 3}));
    run(&mut s, "perspective.grid.define", json!({"angle": 35, "opacity": 30}));
    // Save Grid as Preset: the document's grid under a name, not an undo step.
    let n = undo_len(&s);
    let r = run(&mut s, "perspective.presets.save", json!({"name": "Tower"}));
    assert_eq!((r["name"].clone(), r["created"].clone(), r["definition"]["kind"].clone()), (json!("Tower"), json!(true), json!(3)));
    assert_eq!(undo_len(&s), n);
    let saved = &s.prefs.perspective_presets[0];
    assert!((saved.angle - 35.0).abs() < 1e-9 && saved.opacity == 30.0);
    // A grid with its fields is that preset; changing one makes it custom again.
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    run(&mut s, "perspective.grid.preset", json!({"name": "tower"}));
    assert_eq!(grid(&s).name, "Tower");
    assert!((grid(&s).viewing_angle() - 35.0).abs() < 1e-9);
    run(&mut s, "perspective.grid.define", json!({"opacity": 31}));
    assert_eq!(grid(&s).name, "");
    run(&mut s, "perspective.grid.define", json!({"name": "Tower", "opacity": 30}));
    assert_eq!(grid(&s).name, "Tower");
    // Edit, rename, start from another preset.
    let r = run(&mut s, "perspective.presets.save", json!({"name": "Tower", "newName": "Spire", "gridline": 12}));
    assert_eq!((r["name"].clone(), r["created"].clone()), (json!("Spire"), json!(false)));
    assert_eq!(s.prefs.perspective_presets[0].gridline, 12.0);
    assert_eq!(run(&mut s, "perspective.presets.save", json!({"preset": "[1P-Normal View]"}))["name"], json!("Perspective Preset 1"));
    assert_eq!(s.prefs.perspective_presets[1].kind, 1);
    // Built-in names are protected and names stay unique.
    assert!(s.execute("perspective.presets.save", &json!({"name": "[2P-Normal View]"})).is_err());
    assert!(s.execute("perspective.presets.save", &json!({"name": "Spire", "newName": "[1P-Low View]"})).is_err());
    assert!(s.execute("perspective.presets.save", &json!({"name": "Spire", "newName": "perspective preset 1"})).is_err());
    assert!(s.execute("perspective.presets.save", &json!({"name": "Spire", "angle": 95})).is_err());
    assert!(s.execute("perspective.presets.delete", &json!({"name": "[3P-Low View]"})).is_err());
    assert_eq!(run(&mut s, "perspective.presets.delete", json!({"name": "spire"}))["deleted"], json!("Spire"));
    assert!(s.execute("perspective.presets.delete", &json!({"name": "Spire"})).is_err());
    assert_eq!(preset_names(&mut s).last().unwrap(), "Perspective Preset 1");
}

#[test]
fn presets_export_import_and_live_in_the_preferences() {
    let mut s = session();
    run(&mut s, "perspective.presets.save", json!({"name": "Street", "units": "inches", "gridline": 1, "distance": 8, "horizonHeight": 4}));
    let text = run(&mut s, "perspective.presets.export", json!({"names": ["Street", "[2P-High View]"]}))["data"].as_str().unwrap().to_string();
    assert!(text.contains("vcperspective"));
    let mut t = session();
    let r = run(&mut t, "perspective.presets.import", json!({"data": text}));
    assert_eq!(r["imported"], json!(["Street", "2P-High View"]), "a built-in view comes in as a saved copy");
    assert_eq!(t.prefs.perspective_presets[0], s.prefs.perspective_presets[0]);
    // Again: names in use get a number, or replace.
    assert_eq!(run(&mut t, "perspective.presets.import", json!({"data": text}))["imported"], json!(["Street 2", "2P-High View 2"]));
    assert_eq!(run(&mut t, "perspective.presets.import", json!({"data": text, "replace": true}))["imported"], json!(["Street", "2P-High View"]));
    assert_eq!(t.prefs.perspective_presets.len(), 4);
    // Bad files are refused whole.
    for bad in [
        json!({"data": "{}"}),
        json!({"data": "{\"format\": \"vcprintpresets\", \"presets\": []}"}),
        json!({"data": "{\"format\": \"vcperspective\", \"presets\": [{\"name\": \"ok\"}, {\"name\": \"x\", \"angle\": 120}]}"}),
        json!({"data": "{\"format\": \"vcperspective\", \"presets\": [{\"kind\": \"two\"}]}"}),
        json!({"dataBase64": "%%%"}),
        json!({}),
    ] {
        assert!(t.execute("perspective.presets.import", &bad).is_err(), "{bad}");
    }
    assert_eq!(t.prefs.perspective_presets.len(), 4);
    assert!(Session::new().execute("perspective.presets.export", &json!({})).is_err(), "nothing saved to export");
    // Saved with the preferences; older preferences have none.
    let back: Prefs = serde_json::from_value(serde_json::to_value(&t.prefs).unwrap()).unwrap();
    assert_eq!(back.perspective_presets, t.prefs.perspective_presets);
    assert!(serde_json::from_value::<Prefs>(json!({})).unwrap().perspective_presets.is_empty());
}

/// Import Perspective Grid Presets refuses a `replace` given as text or as another value that is not
/// true or false: the saved presets stay as they were. Null is read as the key left out, so the
/// imported presets whose names are taken get a number.
#[test]
fn presets_import_refuses_replace_given_as_text() {
    let mut s = session();
    run(&mut s, "perspective.presets.save", json!({"name": "Street", "gridline": 1}));
    let text = run(&mut s, "perspective.presets.export", json!({}))["data"].as_str().unwrap().to_string();
    let before = s.prefs.perspective_presets.clone();
    for v in not_booleans() {
        refuses_non_boolean(&mut s, "perspective.presets.import", &json!({"data": text, "replace": v}), "replace");
        assert_eq!(s.prefs.perspective_presets, before, "{v}");
    }
    assert_eq!(run(&mut s, "perspective.presets.import", json!({"data": text, "replace": null}))["imported"], json!(["Street 2"]));
    assert_eq!(run(&mut s, "perspective.presets.import", json!({"data": text, "replace": true}))["imported"], json!(["Street"]));
    assert_eq!(s.prefs.perspective_presets.len(), 2);
}

// ---------- M8.10: view options ----------

use vectorcraft_geom::Point;
use vectorcraft_tools::distort::perspective::Plane;
use vectorcraft_tools::{Overlay, PointerEvent, PointerKind};

fn drag(s: &mut Session, tool: &str, from: Point, to: Point) {
    let v = ViewInfo::default();
    s.select_tool(tool, v).unwrap();
    for (k, p) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
        s.pointer(&PointerEvent::new(k, p.x, p.y), v).unwrap();
    }
}

#[test]
fn view_options_toggle_without_undo_steps_and_survive_presets() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let n = undo_len(&s);
    let g = grid(&s);
    assert!(g.snap && !g.locked && !g.lock_station && !g.rulers, "Snap to Grid is on by default");
    for (id, on) in
        [("perspective.grid.lock", true), ("perspective.grid.lockStation", true), ("perspective.grid.snap", false), ("perspective.grid.rulers", true)]
    {
        assert_eq!(run(&mut s, id, json!({})), json!({"on": on}), "{id} toggles");
    }
    assert_eq!(run(&mut s, "perspective.grid.rulers", json!({"on": true})), json!({"on": true}));
    assert_eq!(undo_len(&s), n);
    run(&mut s, "perspective.grid.preset", json!({"name": "[2P-High View]"}));
    let g = grid(&s);
    assert!(g.locked && g.lock_station && !g.snap && g.rulers, "a preset keeps the view options");
    // Saved with the document.
    let doc = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(PerspectiveGrid::from_doc(&back), Some(g));
}

/// Lock Grid, Lock Station Point, Snap to Grid and Show Rulers refuse an `on` given as text or as
/// another value that is not true or false: the grid and the undo history stay as they were. Null is
/// read as the key left out, so it toggles the option.
#[test]
fn view_options_refuse_on_given_as_text() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    run(&mut s, "perspective.grid.lock", json!({"on": true}));
    let before = (grid(&s), undo_len(&s));
    for id in ["perspective.grid.lock", "perspective.grid.lockStation", "perspective.grid.snap", "perspective.grid.rulers"] {
        for v in not_booleans() {
            refuses_non_boolean(&mut s, id, &json!({"on": v}), "on");
            assert_eq!((grid(&s), undo_len(&s)), before, "{id} {v}");
        }
    }
    for (id, on) in [
        ("perspective.grid.lock", false),
        ("perspective.grid.lockStation", true),
        ("perspective.grid.snap", false),
        ("perspective.grid.rulers", true),
    ] {
        assert_eq!(run(&mut s, id, json!({"on": null})), json!({"on": on}), "{id} toggles");
        assert_eq!(run(&mut s, id, json!({"on": on})), json!({"on": on}), "{id} is set");
    }
    assert_eq!(undo_len(&s), before.1);
}

#[test]
fn lock_grid_keeps_the_widgets_still_but_planes_switch() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    run(&mut s, "perspective.grid.lock", json!({"on": true}));
    let g = grid(&s);
    let n = undo_len(&s);
    drag(&mut s, "perspectiveGrid", Point::new(g.vp_left, g.horizon), Point::new(g.vp_left - 50.0, g.horizon));
    assert_eq!((grid(&s).vp_left, undo_len(&s)), (g.vp_left, n));
    let (faces, _) = g.widget(&s.doc().unwrap().doc, 1.0);
    let q = faces[2].1;
    let c = Point::new((q[0].x + q[2].x) / 2.0, (q[0].y + q[2].y) / 2.0);
    drag(&mut s, "perspectiveGrid", c, c);
    assert_eq!(grid(&s).plane, Plane::Ground);
    run(&mut s, "perspective.grid.lock", json!({"on": false}));
    drag(&mut s, "perspectiveGrid", Point::new(g.vp_left, g.horizon), Point::new(g.vp_left - 50.0, g.horizon));
    assert!((grid(&s).vp_left - (g.vp_left - 50.0)).abs() < 1e-6);
}

#[test]
fn lock_station_point_swings_the_other_vanishing_point() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let g = grid(&s);
    let st = g.station();
    // Unlocked: only the dragged point moves (the viewing angle stays).
    drag(&mut s, "perspectiveGrid", Point::new(g.vp_left, g.horizon), Point::new(g.vp_left - 60.0, g.horizon));
    assert_eq!(grid(&s).vp_right, g.vp_right);
    s.execute("edit.undo", &json!({})).unwrap();
    run(&mut s, "perspective.grid.lockStation", json!({"on": true}));
    drag(&mut s, "perspectiveGrid", Point::new(g.vp_left, g.horizon), Point::new(g.vp_left - 60.0, g.horizon));
    let n = grid(&s);
    assert!((n.vp_left - (g.vp_left - 60.0)).abs() < 1e-6);
    assert!(n.vp_right < g.vp_right, "the right vanishing point comes in");
    let st2 = n.station();
    assert!((st2.x - st.x).abs() < 1e-6 && (st2.distance - st.distance).abs() < 1e-6, "the viewer stays put");
    assert!(n.viewing_angle() < 45.0);
}

#[test]
fn snap_to_grid_lands_drawn_and_moved_art_on_gridlines() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let g = grid(&s);
    let on_line = |v: f64| (v / g.cell - (v / g.cell).round()).abs() < 1e-6;
    // A corner a little off the gridlines (within a quarter cell) is drawn on them.
    let a = g.to_page(Plane::Left, Point::new(2.0 * g.cell + 3.0, g.cell - 2.0)).unwrap();
    let b = g.to_page(Plane::Left, Point::new(4.0 * g.cell - 1.0, 3.0 * g.cell + 4.0)).unwrap();
    let rect = json!({"x": a.x.min(b.x), "y": a.y.min(b.y), "width": (a.x - b.x).abs(), "height": (a.y - b.y).abs()});
    let r = run(&mut s, "perspective.draw", json!({"command": "shape.rectangle", "params": rect, "plane": "left"}));
    let id = NodeId(r["id"].as_u64().unwrap());
    let corners = |s: &Session| -> Vec<Point> {
        let n = s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone();
        n.anchors().map(|(_, _, an)| g.to_plane(Plane::Left, an.p).unwrap()).collect()
    };
    assert!(corners(&s).iter().all(|q| on_line(q.x) && on_line(q.y)), "{:?}", corners(&s));
    // Moved a cell and a bit: it lands a whole cell over.
    let from = g.to_page(Plane::Left, Point::new(3.0 * g.cell, 2.0 * g.cell)).unwrap();
    let to = g.to_page(Plane::Left, Point::new(4.0 * g.cell + 2.0, 2.0 * g.cell)).unwrap();
    run(&mut s, "perspective.move", json!({"ids": [id.0], "from": [from.x, from.y], "to": [to.x, to.y]}));
    assert!(corners(&s).iter().all(|q| on_line(q.x) && on_line(q.y)), "{:?}", corners(&s));
    // Without snapping it moves exactly.
    let before = corners(&s);
    run(&mut s, "perspective.move", json!({"ids": [id.0], "from": [from.x, from.y], "to": [to.x, to.y], "snap": false}));
    for (p, q) in before.iter().zip(corners(&s)) {
        assert!((q.x - p.x - (g.cell + 2.0)).abs() < 1e-6 && (q.y - p.y).abs() < 1e-6);
    }
}

#[test]
fn grid_overlays_use_the_colours_opacity_and_rulers() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    run(&mut s, "perspective.grid.define", json!({"leftColor": "#102030", "opacity": 20}));
    let ov = s.overlays(ViewInfo::default());
    assert!(ov.iter().any(|o| matches!(o, Overlay::GridLine { color: [0x10, 0x20, 0x30, 51], .. })));
    let labels = |ov: &[Overlay]| ov.iter().filter(|o| matches!(o, Overlay::Label { .. })).count();
    assert_eq!(labels(&ov), 0);
    run(&mut s, "perspective.grid.rulers", json!({"on": true}));
    assert!(labels(&s.overlays(ViewInfo::default())) > 2);
}

#[test]
fn snapped_moves_keep_objects_on_a_plane_together() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let g = grid(&s);
    let rect_at = |s: &mut Session, u: f64, v: f64| {
        let (a, b) = (g.to_page(Plane::Left, Point::new(u, v)).unwrap(), g.to_page(Plane::Left, Point::new(u + g.cell, v + g.cell)).unwrap());
        let p = json!({"x": a.x.min(b.x), "y": a.y.min(b.y), "width": (a.x - b.x).abs(), "height": (a.y - b.y).abs()});
        NodeId(run(s, "perspective.draw", json!({"command": "shape.rectangle", "params": p, "plane": "left"}))["id"].as_u64().unwrap())
    };
    let (a, b) = (rect_at(&mut s, 2.0 * g.cell, g.cell), rect_at(&mut s, 4.0 * g.cell + 7.0, g.cell));
    let left = |s: &Session, id: NodeId| {
        let p = s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone();
        p.anchors().map(|(_, _, an)| g.to_plane(Plane::Left, an.p).unwrap().x).fold(f64::MAX, f64::min)
    };
    let gap = left(&s, b) - left(&s, a);
    let from = g.to_page(Plane::Left, Point::new(3.0 * g.cell, 2.0 * g.cell)).unwrap();
    let to = g.to_page(Plane::Left, Point::new(4.0 * g.cell + 3.0, 2.0 * g.cell)).unwrap();
    run(&mut s, "perspective.move", json!({"ids": [a.0, b.0], "from": [from.x, from.y], "to": [to.x, to.y]}));
    assert!(near(left(&s, b) - left(&s, a), gap), "they move by the same amount");
    assert!(near(left(&s, a), 3.0 * g.cell), "the joint bounds land on a gridline");
}

// ---------- M8.11: grid widgets and the Plane Switching Widget ----------

use vectorcraft_tools::distort::perspective::widget::{WidgetCorner, WidgetPlace};
use vectorcraft_tools::{Mods, ScreenFrame, ToolKey};

/// A 1000 × 700 window at zoom 2 showing the document from (100, 50).
fn screen() -> ScreenFrame {
    ScreenFrame {
        origin: Point::new(100.0, 50.0),
        right: vectorcraft_geom::Vec2::new(0.5, 0.0),
        down: vectorcraft_geom::Vec2::new(0.0, 0.5),
        size: (1000.0, 700.0),
    }
}

fn windowed() -> ViewInfo {
    ViewInfo { zoom: 2.0, screen: Some(screen()), ..Default::default() }
}

/// The centre of the widget's `plane` face in `view`.
fn face(s: &Session, view: ViewInfo, corner: WidgetCorner, plane: Plane) -> Point {
    let d = &s.doc().unwrap().doc;
    let (faces, (c, _)) = grid(s).widget_at(d, 1.0 / view.zoom, WidgetPlace { screen: view.screen.as_ref(), corner });
    match faces.into_iter().find(|f| f.0 == plane) {
        Some((_, q)) => Point::new((q[0].x + q[1].x + q[2].x + q[3].x) / 4.0, (q[0].y + q[1].y + q[2].y + q[3].y) / 4.0),
        None => c,
    }
}

fn click(s: &mut Session, p: Point, view: ViewInfo) {
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, p.x, p.y), view).unwrap();
    }
}

#[test]
fn the_plane_widget_stays_on_screen_and_takes_clicks_with_any_tool() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let view = windowed();
    // In the window's top-left corner, whatever part of the document shows.
    let p = face(&s, view, WidgetCorner::TopLeft, Plane::Right);
    assert!(p.x > 100.0 && p.x < 100.0 + 40.0 && p.y > 50.0 && p.y < 50.0 + 40.0, "{p:?}");
    // The Selection tool's click on it picks the plane and selects nothing.
    s.select_tool("selection", view).unwrap();
    let n = undo_len(&s);
    click(&mut s, p, view);
    assert_eq!(grid(&s).plane, Plane::Right);
    assert!(s.doc().unwrap().selection.is_empty());
    assert_eq!(undo_len(&s), n);
    // A hidden grid takes no clicks; choosing a perspective tool shows it.
    run(&mut s, "perspective.grid.show", json!({"visible": false}));
    let q = face(&s, view, WidgetCorner::TopLeft, Plane::Ground);
    click(&mut s, q, view);
    assert_eq!(grid(&s).plane, Plane::Right);
    s.select_tool("perspectiveSelection", view).unwrap();
    click(&mut s, q, view);
    assert_eq!(grid(&s).plane, Plane::Ground);
    // Perspective Grid Options move it to another corner or hide it.
    assert_eq!(run(&mut s, "perspective.widget.options", json!({"position": "bottomRight"})), json!({"show": true, "position": "bottomRight"}));
    let r = face(&s, view, WidgetCorner::BottomRight, Plane::Left);
    assert!(r.x > 100.0 + 450.0 && r.y > 50.0 + 300.0, "{r:?}");
    click(&mut s, r, view);
    assert_eq!(grid(&s).plane, Plane::Left);
    run(&mut s, "perspective.widget.options", json!({"show": false}));
    let hidden = face(&s, view, WidgetCorner::BottomRight, Plane::Right);
    click(&mut s, hidden, view);
    assert_eq!(grid(&s).plane, Plane::Left, "a hidden widget takes no clicks");
    assert!(s.execute("perspective.widget.options", &json!({"position": "middle"})).is_err());
    assert!(s.execute("perspective.widget.options", &json!({"show": "yes"})).is_err());
}

#[test]
fn digit_keys_pick_the_plane_while_the_grid_shows() {
    let mut s = session();
    let view = ViewInfo::default();
    assert!(!s.tool_claims_key(ToolKey::Digit(2), view), "no grid shown: the digit is free");
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    for (n, plane) in [(1, Plane::Left), (2, Plane::Ground), (3, Plane::Right), (4, Plane::None)] {
        assert!(s.tool_claims_key(ToolKey::Digit(n), view));
        s.tool_key(ToolKey::Digit(n), Mods::default(), view).unwrap();
        assert_eq!(grid(&s).plane, plane, "key {n}");
    }
    assert!(!s.tool_claims_key(ToolKey::Digit(5), view));
}

#[test]
fn ground_level_points_move_the_whole_grid() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let g = grid(&s);
    let o = Point::new(g.origin[0], g.origin[1]);
    // The left ground-level point sits 24 px from the origin along the left wall's ground line.
    let towards = g.to_page(Plane::Left, Point::new(g.cell, 0.0)).unwrap();
    let gl = o + (towards - o).normalize() * 24.0;
    drag(&mut s, "perspectiveGrid", gl, gl + vectorcraft_geom::Vec2::new(30.0, -20.0));
    let n = grid(&s);
    let moved = [n.origin[0] - o.x, n.origin[1] - o.y, n.horizon - g.horizon, n.vp_left - g.vp_left, n.vp_right - g.vp_right];
    for (got, want) in moved.into_iter().zip([30.0, -20.0, -20.0, 30.0, 30.0]) {
        assert!(near(got, want), "{moved:?}");
    }
    assert_eq!((n.cell, n.viewing_angle()), (g.cell, g.viewing_angle()), "the same grid, elsewhere");
    // Shift keeps it on one axis.
    s.execute("edit.undo", &json!({})).unwrap();
    gesture_mods(&mut s, gl, gl + vectorcraft_geom::Vec2::new(30.0, -8.0), Mods { shift: true, ..Default::default() });
    let n = grid(&s);
    assert!(near(n.origin[0], o.x + 30.0) && n.origin[1] == o.y && n.horizon == g.horizon);
}

/// A Perspective Grid tool drag with modifiers held.
fn gesture_mods(s: &mut Session, from: Point, to: Point, mods: Mods) {
    let v = ViewInfo::default();
    s.select_tool("perspectiveGrid", v).unwrap();
    for (k, p) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
        s.pointer(&PointerEvent::new(k, p.x, p.y).with_mods(mods), v).unwrap();
    }
}

#[test]
fn extents_cell_size_and_vanishing_points_follow_their_widgets() {
    let mut s = session();
    run(&mut s, "perspective.grid.preset", json!({"kind": 2}));
    let g = grid(&s);
    // The right extent alone.
    let er = g.to_page(Plane::Right, Point::new(g.extent, 0.0)).unwrap();
    let to = g.to_page(Plane::Right, Point::new(g.extent * 0.5, 0.0)).unwrap();
    drag(&mut s, "perspectiveGrid", er, to);
    let n = grid(&s);
    assert!((n.right_extent() - g.extent * 0.5).abs() < 1e-6 && n.extent == g.extent, "{:?} {}", n.extent_right, n.extent);
    assert_eq!(n.domain(Plane::Right).width(), n.right_extent());
    // Alt drags both; Shift goes by whole cells.
    let el = g.to_page(Plane::Left, Point::new(g.extent, 0.0)).unwrap();
    let to = g.to_page(Plane::Left, Point::new(5.3 * g.cell, 0.0)).unwrap();
    gesture_mods(&mut s, el, to, Mods { alt: true, shift: true, ..Default::default() });
    let n = grid(&s);
    assert!((n.extent - 5.0 * g.cell).abs() < 1e-6 && n.extent_right == Some(n.extent));
    // The cell widget: a cell up the line where the planes meet (more cells when they're small).
    let c = g.to_page(Plane::Left, Point::new(0.0, g.cell)).unwrap();
    drag(&mut s, "perspectiveGrid", c, g.to_page(Plane::Left, Point::new(0.0, 40.0)).unwrap());
    assert!((grid(&s).cell - 40.0).abs() < 1e-6, "{}", grid(&s).cell);
    // Shift-dragging a vanishing point keeps the horizon.
    gesture_mods(
        &mut s,
        Point::new(g.vp_right, g.horizon),
        Point::new(g.vp_right + 20.0, g.horizon + 30.0),
        Mods { shift: true, ..Default::default() },
    );
    assert_eq!((grid(&s).vp_right, grid(&s).horizon), (g.vp_right + 20.0, g.horizon));
}

#[test]
fn widget_options_and_the_right_extent_are_saved() {
    let mut s = session();
    run(&mut s, "perspective.widget.options", json!({"position": "Top Right"}));
    let back: Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.perspective_widget.position, WidgetCorner::TopRight);
    run(&mut s, "perspective.grid.set", json!({"extentRight": 123}));
    let doc = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(PerspectiveGrid::from_doc(&back).unwrap().extent_right, Some(123.0));
    assert!(s.execute("perspective.grid.set", &json!({"extentRight": -1})).is_err());
}

#[test]
fn perspective_grid_options_are_a_preference_group() {
    let mut s = session();
    run(&mut s, "prefs.set", json!({"key": "perspectiveWidget", "value": {"position": "bottomLeft"}}));
    assert_eq!(s.prefs.perspective_widget.position, WidgetCorner::BottomLeft);
    assert!(s.prefs.perspective_widget.show, "a partial object keeps the rest");
    assert!(s.execute("prefs.set", &json!({"key": "perspectiveWidget", "value": {"position": "nowhere"}})).is_err());
    run(&mut s, "prefs.reset", json!({}));
    assert_eq!(s.prefs.perspective_widget, Default::default());
}
