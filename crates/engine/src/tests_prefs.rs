//! Preferences: `prefs.get` / `prefs.set` / `prefs.reset` / `prefs.list`.

use serde_json::{Value, json};

use super::*;
use crate::cmd::prefscmds::{PREF_CATEGORIES, PREF_GROUPS, PREF_SPECS, validate};

#[test]
fn every_spec_matches_a_prefs_field_and_back() {
    let all = Prefs::default().to_json();
    let obj = all.as_object().unwrap();
    for sp in PREF_SPECS {
        assert!(obj.contains_key(sp.key), "spec `{}` has no Prefs field", sp.key);
        assert!(PREF_CATEGORIES.contains(&sp.category), "spec `{}` has unknown category", sp.key);
        // Defaults validate against their own spec.
        assert!(validate(sp.key, &obj[sp.key]).is_ok(), "default of `{}` fails validation", sp.key);
    }
    for k in obj.keys() {
        assert!(PREF_SPECS.iter().any(|s| s.key == k) || PREF_GROUPS.contains(&k.as_str()), "Prefs field `{k}` has no spec");
    }
    for c in PREF_CATEGORIES {
        assert!(PREF_SPECS.iter().any(|s| s.category == *c), "category {c} is empty");
    }
}

#[test]
fn set_and_get_keyboard_increment_drives_nudge() {
    let mut s = Session::new();
    s.execute("prefs.set", &json!({"key": "keyboardIncrement", "value": 5})).unwrap();
    assert_eq!(s.execute("prefs.get", &json!({"key": "keyboardIncrement"})).unwrap(), json!(5.0));
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("object.nudge", &json!({"dx": 1, "dy": 0})).unwrap();
    let b = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    assert!((b.x0 - 15.0).abs() < 1e-9);
}

#[test]
fn set_rejects_out_of_range_and_wrong_types() {
    let mut s = Session::new();
    assert!(s.execute("prefs.set", &json!({"key": "keyboardIncrement", "value": -1})).is_err());
    assert!(s.execute("prefs.set", &json!({"key": "anchorSize", "value": 9})).is_err());
    assert!(s.execute("prefs.set", &json!({"key": "anchorSize", "value": 2.5})).is_err());
    assert!(s.execute("prefs.set", &json!({"key": "scaleStrokes", "value": 3})).is_err());
    assert!(s.execute("prefs.set", &json!({"key": "uiBrightness", "value": "purple"})).is_err());
    assert!(s.execute("prefs.set", &json!({"key": "gridColor", "value": "#12345"})).is_err());
    assert!(s.execute("prefs.set", &json!({"key": "nope", "value": 1})).is_err());
    assert!(s.execute("prefs.set", &json!({"key": "scaleStrokes"})).is_err());
    assert_eq!(s.prefs, Prefs::default());
}

#[test]
fn set_normalizes_strings_labels_and_colours() {
    let mut s = Session::new();
    let r = s
        .execute(
            "prefs.set",
            &json!({"values": {"uiBrightness": "Medium Light", "gridColor": "ABCDEF", "keyboardIncrement": "2.5 pt", "scaleStrokes": "false"}}),
        )
        .unwrap();
    assert_eq!(r["uiBrightness"], json!("mediumLight"));
    assert_eq!(s.prefs.grid_color, "#abcdef");
    assert_eq!(s.prefs.keyboard_increment, 2.5);
    assert!(!s.prefs.scale_strokes);
}

#[test]
fn batch_set_is_atomic() {
    let mut s = Session::new();
    assert!(s.execute("prefs.set", &json!({"values": {"keyboardIncrement": 3, "anchorSize": 99}})).is_err());
    assert_eq!(s.prefs.keyboard_increment, 1.0);
}

#[test]
fn grid_prefs_apply_to_open_documents() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s.execute("prefs.set", &json!({"values": {"gridlineEvery": 36, "gridSubdivisions": 4}})).unwrap();
    let g = &s.doc().unwrap().doc.grid;
    assert_eq!((g.spacing, g.subdivisions), (36.0, 4));
}

#[test]
fn history_states_limit_undo_depth() {
    let mut s = Session::new();
    s.execute("prefs.set", &json!({"key": "historyStates", "value": 5})).unwrap();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    for i in 0..9 {
        s.execute("shape.rectangle", &json!({"x": i * 10, "y": 0, "width": 5, "height": 5})).unwrap();
    }
    assert!(s.doc().unwrap().history.undo.len() <= 5);
}

#[test]
fn reset_category_and_all() {
    let mut s = Session::new();
    s.execute("prefs.set", &json!({"values": {"keyboardIncrement": 7, "gridColor": "#000000"}})).unwrap();
    s.execute("prefs.reset", &json!({"category": "General"})).unwrap();
    assert_eq!(s.prefs.keyboard_increment, 1.0);
    assert_eq!(s.prefs.grid_color, "#000000");
    s.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(s.prefs, Prefs::default());
    assert!(s.execute("prefs.reset", &json!({"category": "Bogus"})).is_err());
}

#[test]
fn list_describes_every_pref() {
    let mut s = Session::new();
    let l = s.execute("prefs.list", &json!({})).unwrap();
    let a = l.as_array().unwrap();
    assert_eq!(a.len(), PREF_SPECS.len());
    assert!(a.iter().any(|e| e["key"] == "renderThreads" && e["kind"] == "integer" && e["min"] == -1));
}

#[test]
fn prefs_serde_round_trip_and_tolerates_missing_fields() {
    let p = Prefs { ui_scaling: 1.25, units_general: "millimeters".into(), ..Default::default() };
    let back: Prefs = serde_json::from_value(p.to_json()).unwrap();
    assert_eq!(back, p);
    let partial: Prefs = serde_json::from_value(json!({"keyboardIncrement": 4})).unwrap();
    assert_eq!(partial.keyboard_increment, 4.0);
    assert_eq!(partial.corner_radius, 12.0);
}

/// Performance › Graphics Processor (#306, #502): automatic by default, set by value or label,
/// kept through a save/load round trip, and a bad value (0.5.0's `powerSaving` among them) is an
/// error that changes nothing.
#[test]
fn gpu_preference_defaults_to_automatic_and_validates() {
    let mut s = Session::new();
    assert_eq!(s.prefs.gpu_preference, "automatic");
    assert_eq!(s.execute("prefs.get", &json!({"key": "gpuPreference"})).unwrap(), json!("automatic"));
    s.execute("prefs.set", &json!({"key": "gpuPreference", "value": "highPerformance"})).unwrap();
    assert_eq!(s.prefs.gpu_preference, "highPerformance");
    s.execute("prefs.set", &json!({"key": "gpuPreference", "value": "Power Saving (integrated)"})).unwrap();
    assert_eq!(s.prefs.gpu_preference, "lowPower");
    s.execute("prefs.set", &json!({"key": "gpuPreference", "value": "automatic"})).unwrap();
    assert_eq!(s.prefs.gpu_preference, "automatic");
    for bad in [json!("powerSaving"), json!("turbo"), json!(""), json!(1), json!(null), json!(["highPerformance"])] {
        assert!(s.execute("prefs.set", &json!({"key": "gpuPreference", "value": bad})).is_err(), "{bad}");
        assert_eq!(s.prefs.gpu_preference, "automatic");
    }
    let p = Prefs { gpu_preference: "lowPower".into(), ..Default::default() };
    let back: Prefs = serde_json::from_value(p.to_json()).unwrap();
    assert_eq!(back.gpu_preference, "lowPower");
    // Preference files written before the preference existed get the default.
    let old: Prefs = serde_json::from_value(json!({"gpuPerformance": true})).unwrap();
    assert_eq!(old.gpu_preference, "automatic");
    let l = s.execute("prefs.list", &json!({})).unwrap();
    let row = l.as_array().unwrap().iter().find(|e| e["key"] == "gpuPreference").unwrap().clone();
    assert_eq!(row["category"], "Performance");
    assert_eq!(row["options"], json!(["automatic", "lowPower", "highPerformance"]));
    s.execute("prefs.set", &json!({"key": "gpuPreference", "value": "highPerformance"})).unwrap();
    s.execute("prefs.reset", &json!({"category": "Performance"})).unwrap();
    assert_eq!(s.prefs.gpu_preference, "automatic");
}

/// Selection & Anchor Display › Tolerance, Object Selection by Path Only and Command Click to
/// Select Objects Behind, and General › Double Click To Isolate, as the Selection tool sees them
/// (#394).
#[test]
fn selection_preferences_drive_the_selection_tool() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let rect = |s: &mut Session, x: f64| {
        NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap())
    };
    // Two filled squares overlapping between x = 150 and 200, `front` on top.
    let (back, front) = (rect(&mut s, 100.0), rect(&mut s, 150.0));
    s.select_tool("selection", ViewInfo::default()).unwrap();
    let set = |s: &mut Session, key: &str, value: Value| s.execute("prefs.set", &json!({"key": key, "value": value})).unwrap();
    let click = |s: &mut Session, x: f64, mods: Mods| {
        for k in [PointerKind::Down, PointerKind::Up] {
            // Halfway between the bounding box's handles.
            s.pointer(&PointerEvent::new(k, x, 125.0).with_mods(mods), ViewInfo::default()).unwrap();
        }
        s.doc().unwrap().selection.objects.clone()
    };
    let plain = Mods::default();
    // Tolerance: the right edge's stroke ends at x = 250.5.
    assert_eq!(click(&mut s, 252.5, plain), vec![front], "2 px out: within the default 3 px");
    assert!(click(&mut s, 256.5, plain).is_empty(), "6 px out: beyond 3 px");
    set(&mut s, "selectionTolerance", json!(8));
    assert_eq!(click(&mut s, 256.5, plain), vec![front], "6 px out: within 8 px");
    set(&mut s, "selectionTolerance", json!(1));
    assert!(click(&mut s, 252.5, plain).is_empty(), "2 px out: beyond 1 px");
    set(&mut s, "selectionTolerance", json!(3));
    // Object Selection by Path Only: the fill no longer selects, the path does.
    assert_eq!(click(&mut s, 225.0, plain), vec![front]);
    set(&mut s, "objectSelectionByPathOnly", json!(true));
    assert!(click(&mut s, 225.0, plain).is_empty(), "a click inside the fill selects nothing");
    assert_eq!(click(&mut s, 250.0, plain), vec![front], "a click on the path selects it");
    set(&mut s, "objectSelectionByPathOnly", json!(false));
    // Command Click to Select Objects Behind (on by default): each Cmd/Ctrl-click goes one down,
    // then back to the top.
    let cmd = Mods { cmd: true, ..Mods::default() };
    assert_eq!(click(&mut s, 175.0, plain), vec![front]);
    assert_eq!(click(&mut s, 175.0, cmd), vec![back], "the object behind");
    assert_eq!(click(&mut s, 175.0, cmd), vec![front], "back to the topmost");
    set(&mut s, "ctrlClickSelectsBehind", json!(false));
    assert_eq!(click(&mut s, 175.0, cmd), vec![front], "off: a plain click");
    // Double Click To Isolate (on by default) isolates a group; off, it doesn't.
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let double = |s: &mut Session| {
        s.pointer(&PointerEvent::new(PointerKind::DoubleClick, 175.0, 150.0), ViewInfo::default()).unwrap();
        s.doc().unwrap().isolation
    };
    set(&mut s, "doubleClickToIsolate", json!(false));
    assert_eq!(double(&mut s), None, "off: no isolation");
    set(&mut s, "doubleClickToIsolate", json!(true));
    assert!(double(&mut s).is_some(), "on: the group is isolated");
}

/// General › Use Precise Cursors (#394): the Pen's pointer becomes a crosshair; the Selection
/// tool's arrow stays.
#[test]
fn use_precise_cursors_makes_drawing_cursors_crosshairs() {
    use vectorcraft_tools::{Cursor, Mods};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let cursor = |s: &mut Session, tool: &str| {
        s.select_tool(tool, ViewInfo::default()).unwrap();
        s.cursor(vectorcraft_geom::Point::new(300.0, 300.0), Mods::default(), ViewInfo::default())
    };
    assert_eq!(cursor(&mut s, "pen"), Cursor::Pen);
    s.execute("prefs.set", &json!({"key": "usePreciseCursors", "value": true})).unwrap();
    assert_eq!(cursor(&mut s, "pen"), Cursor::Crosshair);
    assert_eq!(cursor(&mut s, "selection"), Cursor::Arrow);
}

// ---------- #394, second batch ----------

fn new_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn square(s: &mut Session, x: f64, y: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap())
}

fn set_pref(s: &mut Session, key: &str, value: Value) {
    s.execute("prefs.set", &json!({"key": key, "value": value})).unwrap();
}

fn gesture(s: &mut Session, v: ViewInfo, events: &[(vectorcraft_tools::PointerKind, f64, f64)]) {
    for (k, x, y) in events {
        s.pointer(&vectorcraft_tools::PointerEvent::new(*k, *x, *y), v).unwrap();
    }
}

fn top_left(s: &Session, id: NodeId) -> (f64, f64) {
    let b = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    (b.x0, b.y0)
}

/// View › Snap to Point with Selection & Anchor Display › Snap to Point's distance (#394): with
/// Smart Guides off, the point a selection is dragged by lands on an anchor within 2 px.
#[test]
fn snap_to_point_lands_a_dragged_selection_on_anchors() {
    use vectorcraft_tools::PointerKind::{Down, Drag, Up};
    let mut s = new_doc();
    let a = square(&mut s, 100.0, 100.0);
    square(&mut s, 300.0, 100.0);
    let v = ViewInfo { smart_guides: false, ..ViewInfo::default() };
    s.select_tool("selection", v).unwrap();
    // Grabbed by its centre and dropped 1.5, 1 px from the other square's top-left anchor.
    let drag = |s: &mut Session, v: ViewInfo| {
        gesture(s, v, &[(Down, 125.0, 125.0), (Drag, 200.0, 110.0), (Drag, 301.5, 101.0), (Up, 301.5, 101.0)]);
        let at = top_left(s, a);
        s.execute("edit.undo", &json!({})).unwrap();
        at
    };
    assert_eq!(drag(&mut s, v), (275.0, 75.0), "the centre lands on the anchor");
    set_pref(&mut s, "snapToPointTolerance", json!(1));
    assert_eq!(drag(&mut s, v), (276.5, 76.0), "1.8 px is beyond 1 px");
    set_pref(&mut s, "snapToPointTolerance", json!(2));
    assert_eq!(drag(&mut s, ViewInfo { snap_to_point: false, ..v }), (276.5, 76.0), "View › Snap to Point off");
}

/// Smart Guides › Spacing Guides (#394): a square dragged 21 pt to the right of a row of two
/// squares 20 pt apart lands 20 pt from it, with the gaps marked; off, it stays at 21.
#[test]
fn spacing_guides_space_a_dragged_square_like_its_row() {
    use vectorcraft_tools::PointerKind::{Down, Drag, Up};
    let mut s = new_doc();
    square(&mut s, 20.0, 300.0);
    square(&mut s, 90.0, 300.0);
    let m = square(&mut s, 200.0, 305.0);
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    // `m` grabbed by its centre and dragged so its left edge is 21 pt from the second square.
    let drag = |s: &mut Session| {
        gesture(s, v, &[(Down, 225.0, 330.0), (Drag, 200.0, 330.0), (Drag, 186.0, 330.0)]);
        let marks = s.overlays(v).iter().filter(|o| matches!(o, vectorcraft_tools::Overlay::Line { a, b, .. } if a.y == b.y)).count();
        gesture(s, v, &[(Up, 186.0, 330.0)]);
        let at = top_left(s, m).0;
        s.execute("edit.undo", &json!({})).unwrap();
        (at, marks)
    };
    let (at, marks) = drag(&mut s);
    assert_eq!(at, 160.0, "20 pt from it");
    assert!(marks >= 2, "both gaps marked");
    set_pref(&mut s, "spacingGuides", json!(false));
    assert_eq!(drag(&mut s), (161.0, 0), "off: where it was dragged");
}

/// Preferences › Smart Guides (#394): Color, Alignment Guides, Anchor/Path Labels, Measurement
/// Labels, Transform Tools and Snapping Tolerance change what the Selection tool shows while a
/// square is dragged into line with another, and how far the pull reaches; a hidden guide still
/// snaps.
#[test]
fn smart_guide_display_preferences_filter_the_overlays() {
    use vectorcraft_tools::Overlay;
    use vectorcraft_tools::PointerKind::{Down, Drag, Up};
    let mut s = new_doc();
    square(&mut s, 100.0, 100.0);
    let b = square(&mut s, 200.0, 100.0);
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let count = |s: &mut Session, f: &dyn Fn(&Overlay) -> bool| s.overlays(v).iter().filter(|o| f(o)).count();
    let line = |o: &Overlay| matches!(o, Overlay::Line { .. });
    let label = |o: &Overlay| matches!(o, Overlay::Label { .. });
    let measure = |o: &Overlay| matches!(o, Overlay::Measure { .. });
    // `b` grabbed by its centre and dragged so its left edge comes `off` px from the first
    // square's right edge (x = 150): the x of its left edge mid-drag, before the pointer is let go.
    let drag_to = |s: &mut Session, off: f64| {
        gesture(s, v, &[(Down, 225.0, 125.0), (Drag, 200.0, 125.0), (Drag, 175.0 + off, 125.0)]);
        top_left(s, b).0
    };
    let release = |s: &mut Session| {
        gesture(s, v, &[(Up, 175.0, 125.0)]);
        s.execute("edit.undo", &json!({})).unwrap();
    };
    assert_eq!(drag_to(&mut s, 3.0), 150.0, "3 px off: into line");
    assert!(count(&mut s, &line) > 0 && count(&mut s, &measure) == 1);
    let magenta = s.overlays(v).iter().find_map(|o| if let Overlay::Line { color, .. } = o { Some(*color) } else { None });
    assert_eq!(magenta, Some(vectorcraft_tools::guides::MAGENTA), "the default colour");
    release(&mut s);
    set_pref(&mut s, "smartGuideColor", json!("#00ff00"));
    drag_to(&mut s, 3.0);
    assert!(s.overlays(v).iter().all(|o| !matches!(o, Overlay::Line { color, .. } | Overlay::Label { color, .. } if *color != [0, 255, 0])));
    release(&mut s);
    set_pref(&mut s, "alignmentGuides", json!(false));
    assert_eq!(drag_to(&mut s, 3.0), 150.0, "still into line");
    assert_eq!(count(&mut s, &line), 0, "no line");
    release(&mut s);
    set_pref(&mut s, "measurementLabels", json!(false));
    drag_to(&mut s, 3.0);
    assert_eq!(count(&mut s, &measure), 0);
    release(&mut s);
    // Snapping Tolerance: 6 px is beyond the default 4, within 8.
    assert_eq!(drag_to(&mut s, 6.0), 156.0);
    release(&mut s);
    set_pref(&mut s, "snappingTolerance", json!(8));
    assert_eq!(drag_to(&mut s, 6.0), 150.0);
    release(&mut s);
    // Transform Tools: the size readout while a bounding-box handle is dragged.
    let scale = |s: &mut Session| {
        gesture(s, v, &[(Down, 225.0, 150.0), (Drag, 225.0, 170.0)]);
        let n = count(s, &measure);
        gesture(s, v, &[(Up, 225.0, 170.0)]);
        s.execute("edit.undo", &json!({})).unwrap();
        n
    };
    assert_eq!(scale(&mut s), 1);
    set_pref(&mut s, "transformToolsGuides", json!(false));
    assert_eq!(scale(&mut s), 0);
    // Anchor/Path Labels: a drawn corner pulled onto an anchor says "anchor" (and still lands there).
    s.select_tool("rectangle", v).unwrap();
    let draw = |s: &mut Session| {
        gesture(s, v, &[(Down, 300.0, 300.0), (Drag, 153.0, 151.0)]);
        let n = count(s, &label);
        gesture(s, v, &[(Up, 153.0, 151.0)]);
        let at = top_left(s, s.doc().unwrap().selection.objects[0]);
        s.execute("edit.undo", &json!({})).unwrap();
        (n, at)
    };
    assert_eq!(draw(&mut s), (1, (150.0, 150.0)));
    set_pref(&mut s, "anchorPathLabels", json!(false));
    assert_eq!(draw(&mut s), (0, (150.0, 150.0)));
}

/// Enable Rubber Band for Pen Tool / Curvature Tool (#394): off, no segment follows the pointer.
#[test]
fn rubber_band_preferences_hide_the_preview_to_the_pointer() {
    use vectorcraft_tools::Overlay;
    use vectorcraft_tools::PointerKind::{Down, Move, Up};
    let v = ViewInfo::default();
    for (tool, key) in [("pen", "penRubberBand"), ("curvature", "curvatureRubberBand")] {
        let mut s = new_doc();
        s.select_tool(tool, v).unwrap();
        gesture(&mut s, v, &[(Down, 50.0, 50.0), (Up, 50.0, 50.0), (Down, 150.0, 50.0), (Up, 150.0, 50.0), (Move, 200.0, 150.0)]);
        let bands = |s: &mut Session| s.overlays(v).iter().filter(|o| matches!(o, Overlay::Path { .. })).count();
        assert_eq!(bands(&mut s), 1, "{tool}: the rubber band");
        set_pref(&mut s, key, json!(false));
        assert_eq!(bands(&mut s), 0, "{tool}: off");
    }
}

/// Move Locked and Hidden Artwork with Artboard (#394): off, an artboard moved with its art
/// leaves locked and hidden objects where they are; on, they move too.
#[test]
fn move_locked_and_hidden_artwork_with_artboard() {
    let mut s = new_doc();
    let (free, locked, hidden) = (square(&mut s, 10.0, 10.0), square(&mut s, 100.0, 10.0), square(&mut s, 200.0, 10.0));
    for (id, cmd) in [(locked, "object.lock"), (hidden, "object.hide")] {
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute(cmd, &json!({})).unwrap();
    }
    let moved = |s: &mut Session| {
        s.execute("artboard.move", &json!({"index": 0, "dx": 30, "dy": 0, "moveArt": true})).unwrap();
        let xs = [free, locked, hidden].map(|id| top_left(s, id).0);
        s.execute("edit.undo", &json!({})).unwrap();
        xs
    };
    assert_eq!(moved(&mut s), [40.0, 100.0, 200.0]);
    set_pref(&mut s, "moveLockedWithArtboard", json!(true));
    assert_eq!(moved(&mut s), [40.0, 130.0, 230.0]);
}

/// General › Transform Pattern Tiles (#394) is what transforms do with pattern fills unless their
/// `patterns` param says otherwise (the dialogs' Transform Patterns).
#[test]
fn transform_pattern_tiles_is_the_transforms_default() {
    use vectorcraft_color::Paint;
    let mut s = new_doc();
    let tile = square(&mut s, 0.0, 0.0);
    s.execute("select.set", &json!({"ids": [tile.0]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    let big = square(&mut s, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    s.execute("select.set", &json!({"ids": [big.0]})).unwrap();
    let tiles = |s: &Session| match s.doc().unwrap().doc.node(big).unwrap().appearance.fill_paint() {
        Paint::Pattern { xf, .. } => xf.as_coeffs(),
        p => panic!("not a pattern: {p:?}"),
    };
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    assert_eq!(tiles(&s), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], "off: the tiles stay");
    set_pref(&mut s, "transformPatternTiles", json!(true));
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    assert_eq!(tiles(&s), [1.0, 0.0, 0.0, 1.0, 10.0, 0.0], "on: they move with the art");
    assert_eq!(s.journal.last().unwrap().1["patterns"], json!(true), "the journal keeps the choice");
    s.execute("object.scale", &json!({"sx": 200, "origin": [0, 0], "patterns": false})).unwrap();
    assert_eq!(tiles(&s), [1.0, 0.0, 0.0, 1.0, 10.0, 0.0], "the param wins");
    s.execute("object.scale", &json!({"sx": 50, "origin": [0, 0]})).unwrap();
    assert_eq!(tiles(&s), [0.5, 0.0, 0.0, 0.5, 5.0, 0.0]);
}

/// General › Transform Pattern Tiles (#394) applies to Align and Distribute as it does to the
/// transforms: off, the tiles of a pattern-filled square stay in place when the square moves;
/// on, they move with it, and `patterns: false` keeps them in place for one call.
#[test]
fn align_and_distribute_follow_transform_pattern_tiles() {
    use vectorcraft_color::Paint;
    let mut s = new_doc();
    let plain = square(&mut s, 0.0, 0.0);
    s.execute("select.set", &json!({"ids": [plain.0]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    let far = square(&mut s, 300.0, 0.0);
    let filled = square(&mut s, 100.0, 0.0);
    s.execute("paint.setFill", &json!({"ids": [filled.0], "swatch": "Dots"})).unwrap();
    s.execute("select.set", &json!({"ids": [plain.0, filled.0, far.0]})).unwrap();
    let tiles = |s: &Session| match s.doc().unwrap().doc.node(filled).unwrap().appearance.fill_paint() {
        Paint::Pattern { xf, .. } => xf.as_coeffs(),
        p => panic!("not a pattern: {p:?}"),
    };
    // Align left moves the square 100 pt left, to the left edge of `plain`. Both Distribute
    // commands move it 50 pt right, halfway between the other two.
    for (cmd, p, dx) in [
        ("object.align", json!({"horizontal": "left"}), -100.0),
        ("object.distribute", json!({"horizontal": "left"}), 50.0),
        ("object.distributeSpacing", json!({"axis": "horizontal"}), 50.0),
    ] {
        s.execute(cmd, &p).unwrap();
        assert_eq!(tiles(&s), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], "{cmd}: off, the tiles stay");
        assert_eq!(s.journal.last().unwrap().1["patterns"], json!(false), "{cmd}: the journal keeps the choice");
        s.execute("edit.undo", &json!({})).unwrap();
        set_pref(&mut s, "transformPatternTiles", json!(true));
        s.execute(cmd, &p).unwrap();
        assert_eq!(tiles(&s), [1.0, 0.0, 0.0, 1.0, dx, 0.0], "{cmd}: on, they move with the square");
        assert_eq!(s.journal.last().unwrap().1["patterns"], json!(true), "{cmd}: the journal keeps the choice");
        s.execute("edit.undo", &json!({})).unwrap();
        let mut p = p;
        p["patterns"] = json!(false);
        s.execute(cmd, &p).unwrap();
        assert_eq!(tiles(&s), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], "{cmd}: the param wins");
        s.execute("edit.undo", &json!({})).unwrap();
        set_pref(&mut s, "transformPatternTiles", json!(false));
    }
}

/// General › Select Same Tint % (#394): off, Select › Same › Fill Color takes every tint of the
/// swatch; on, only the same tint.
#[test]
fn select_same_tint_percent() {
    let mut s = new_doc();
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#cc0066", "spot": true})).unwrap();
    // Before the others: new art takes the last fill applied.
    square(&mut s, 300.0, 10.0);
    let ids: Vec<NodeId> = [(10.0, 40), (100.0, 40), (200.0, 80)]
        .into_iter()
        .map(|(x, tint)| {
            let id = square(&mut s, x, 10.0);
            s.execute("paint.setFill", &json!({"ids": [id.0], "swatch": "Ink", "tint": tint})).unwrap();
            id
        })
        .collect();
    let same = |s: &mut Session| {
        s.execute("select.set", &json!({"ids": [ids[0].0]})).unwrap();
        s.execute("select.same.fillColor", &json!({})).unwrap();
        s.doc().unwrap().selection.objects.len()
    };
    assert_eq!(same(&mut s), 3, "every tint of Ink");
    set_pref(&mut s, "selectSameTintPercent", json!(true));
    assert_eq!(same(&mut s), 2, "only Ink 40%");
}

fn style_at(s: &Session, id: NodeId, byte: usize) -> vectorcraft_doc::CharStyle {
    let Some(vectorcraft_doc::NodeKind::Text(t)) = s.doc().unwrap().doc.node(id).map(|n| &n.kind) else { panic!("not text") };
    let mut at = 0;
    for r in &t.runs {
        if byte < at + r.text.len() {
            return r.style.clone();
        }
        at += r.text.len();
    }
    panic!("byte {byte} past the text")
}

/// Type › Size/Leading, Tracking and Baseline Shift (#394): the increments `type.step` and the
/// font size shortcuts step by, on selected type objects and on the Type tool's selected text
/// (Alt+arrows; Cmd/Ctrl too: five steps).
#[test]
fn type_increments_step_the_type() {
    use vectorcraft_tools::PointerKind::{Down, Up};
    use vectorcraft_tools::{Mods, ToolKey};
    let mut s = new_doc();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Hello world", "size": 20})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("type.size.increase", &json!({})).unwrap();
    assert_eq!(style_at(&s, id, 0).size, 22.0, "Size/Leading is 2 pt");
    set_pref(&mut s, "typeSizeIncrement", json!(3));
    s.execute("type.size.decrease", &json!({})).unwrap();
    assert_eq!(style_at(&s, id, 0).size, 19.0);
    s.execute("type.step", &json!({"attribute": "leading"})).unwrap();
    assert!((style_at(&s, id, 0).leading.unwrap() - (19.0 * 1.2 + 3.0)).abs() < 1e-9, "from Auto");
    set_pref(&mut s, "baselineShiftIncrement", json!(1.5));
    s.execute("type.step", &json!({"attribute": "baselineShift", "by": -2})).unwrap();
    assert_eq!(style_at(&s, id, 10).baseline_shift, -3.0);
    // The Type tool: Alt+→ tracks the selected text by Tracking (20/1000 em), Cmd-Alt five times.
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    gesture(&mut s, v, &[(Down, 102.0, 95.0), (Up, 102.0, 95.0)]);
    assert_eq!(s.tool_options()["editing"], json!(id.0), "editing the text");
    let alt = Mods { alt: true, ..Mods::default() };
    let select = |s: &mut Session, a: usize, b: usize| s.set_tool_option("select", &json!({"start": a, "end": b}));
    select(&mut s, 0, 5);
    s.tool_key(ToolKey::Right, alt, v).unwrap();
    assert_eq!((style_at(&s, id, 0).tracking, style_at(&s, id, 6).tracking), (20.0, 0.0));
    s.tool_key(ToolKey::Left, Mods { cmd: true, ..alt }, v).unwrap();
    assert_eq!(style_at(&s, id, 4).tracking, -80.0);
    // At a caret: Alt+← kerns the character before it, Alt+↓ opens up the paragraph's leading.
    select(&mut s, 3, 3);
    s.tool_key(ToolKey::Left, alt, v).unwrap();
    assert_eq!((style_at(&s, id, 2).kerning, style_at(&s, id, 3).kerning), (Some(-20.0), None));
    let leading = style_at(&s, id, 0).leading.unwrap();
    s.tool_key(ToolKey::Down, alt, v).unwrap();
    assert_eq!(style_at(&s, id, 8).leading, Some(leading + 3.0));
    // Alt+Shift+↑ raises the selected text's baseline.
    select(&mut s, 6, 11);
    s.tool_key(ToolKey::Up, Mods { shift: true, ..alt }, v).unwrap();
    assert_eq!((style_at(&s, id, 0).baseline_shift, style_at(&s, id, 6).baseline_shift), (-3.0, -1.5));
    // The font size shortcut steps the selected text while the tool edits.
    s.execute("type.size.increase", &json!({})).unwrap();
    assert_eq!((style_at(&s, id, 0).size, style_at(&s, id, 6).size), (19.0, 22.0));
}

/// Type › Fill New Type Objects With Placeholder Text (#394, on by default): type the Type tool
/// places starts with placeholder text, selected, so typing replaces it; off, it starts empty.
#[test]
fn new_type_starts_with_placeholder_text() {
    use vectorcraft_tools::PointerKind::{Down, Drag, Up};
    let v = ViewInfo::default();
    let mut s = new_doc();
    s.select_tool("type", v).unwrap();
    let text = |s: &Session| {
        let st = s.doc().unwrap();
        match &st.doc.node(st.selection.objects[0]).unwrap().kind {
            vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
            _ => panic!("not text"),
        }
    };
    gesture(&mut s, v, &[(Down, 50.0, 50.0), (Up, 50.0, 50.0)]);
    let placeholder = text(&s);
    assert!(placeholder.len() > 10, "{placeholder}");
    assert_eq!((s.tool_options()["start"].clone(), s.tool_options()["end"].clone()), (json!(0), json!(placeholder.len())), "selected");
    s.tool_text("Hi", v).unwrap();
    assert_eq!(text(&s), "Hi");
    // Area type is filled to its frame.
    gesture(&mut s, v, &[(Down, 100.0, 150.0), (Drag, 300.0, 300.0), (Up, 300.0, 300.0)]);
    assert!(text(&s).len() > placeholder.len());
    set_pref(&mut s, "placeholderText", json!(false));
    gesture(&mut s, v, &[(Down, 50.0, 350.0), (Up, 50.0, 350.0)]);
    assert_eq!(text(&s), "");
}

/// Type › Enable Missing Glyph Protection (#394, on by default): characters a new font has no
/// glyph for keep the font that has one; off, they take the new font.
#[test]
fn missing_glyph_protection_keeps_glyphs_a_new_font_lacks() {
    let (from, to, c) = ("Source Sans 3", "JetBrains Mono", '\u{180}');
    let db = vectorcraft_text::FontDb::global();
    let covers = |f: &str| db.face(f, "Regular").is_some_and(|face| face.covers(c));
    if !covers(from) || covers(to) {
        return; // Installed fonts of these names cover other characters.
    }
    let mut s = new_doc();
    let id = NodeId(s.execute("text.create", &json!({"x": 10, "y": 50, "text": "a\u{180}c", "font": from})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let fonts = |s: &Session| [0, 1, 3].map(|b| style_at(s, id, b).font_family);
    s.execute("text.setStyle", &json!({"font": to})).unwrap();
    assert_eq!(fonts(&s), [to, from, to], "the character keeps its font");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 1, "end": 4, "font": to})).unwrap();
    assert_eq!(fonts(&s), [from, from, to]);
    s.execute("edit.undo", &json!({})).unwrap();
    set_pref(&mut s, "missingGlyphProtection", json!(false));
    s.execute("text.setStyle", &json!({"font": to})).unwrap();
    assert_eq!(fonts(&s), [to, to, to]);
}

// ---------- #394, fourth batch ----------

/// Type › Type Object Selection by Path Only (#394): on, a click among point type's glyphs no
/// longer selects it with the Selection tool, a click on its baseline does; off (the default),
/// anywhere in its bounds selects it.
#[test]
fn type_selection_by_path_only_picks_type_on_its_baseline() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = new_doc();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Hello world", "size": 20})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.none", &json!({})).unwrap();
    let b = s.doc().unwrap().doc.node(id).and_then(|n| n.geometric_bounds()).unwrap();
    assert!(b.y0 < 95.0 && b.x1 > 130.0, "the glyphs rise above the baseline at y = 100: {b:?}");
    s.select_tool("selection", ViewInfo::default()).unwrap();
    let click = |s: &mut Session, x: f64, y: f64| {
        for k in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(k, x, y), ViewInfo::default()).unwrap();
        }
        s.doc().unwrap().selection.objects.clone()
    };
    // Clear of the bounding box's handles: 15 px in from the left, halfway up the glyphs.
    let (x, glyphs) = (b.x0 + 15.0, (b.y0 + 100.0) / 2.0);
    assert_eq!(click(&mut s, x, glyphs), vec![id], "off: a click among the glyphs selects the type");
    assert!(click(&mut s, x, 300.0).is_empty(), "empty canvas deselects");
    set_pref(&mut s, "typeSelectionByPathOnly", json!(true));
    assert!(click(&mut s, x, glyphs).is_empty(), "on: a click among the glyphs selects nothing");
    assert_eq!(click(&mut s, x, 101.0), vec![id], "on: a click on the baseline selects it");
    assert!(click(&mut s, x, 300.0).is_empty());
    set_pref(&mut s, "typeSelectionByPathOnly", json!(false));
    assert_eq!(click(&mut s, x, glyphs), vec![id], "off again: the glyphs select it");
}

/// Type Object Selection by Path Only (#394) with real layout: every line's baseline picks point
/// type, between the lines nothing does; the Type tool still edits type clicked among its
/// characters, and the Eyedropper still samples it there.
#[test]
fn type_selection_by_path_only_takes_every_line_and_spares_the_type_tool_and_eyedropper() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let v = ViewInfo::default();
    let mut s = new_doc();
    let created = s
        .execute(
            "text.create",
            &json!({"x": 100, "y": 100, "text": "Hello
world", "size": 20}),
        )
        .unwrap();
    let src = NodeId(created["id"].as_u64().unwrap());
    s.execute("text.setStyle", &json!({"fill": "#00ff00"})).unwrap();
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(src).unwrap().kind else { panic!("not type") };
    let [(a, _), (b, _)] = t.cached_baselines[..] else { panic!("two baselines: {:?}", t.cached_baselines) };
    let (first, second) = (t.xf * a, t.xf * b);
    assert!(first.y == 100.0 && second.y > 115.0, "{first:?} {second:?}");
    let click = |s: &mut Session, tool: &str, x: f64, y: f64| {
        s.select_tool(tool, v).unwrap();
        for k in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(k, x, y), v).unwrap();
        }
        s.doc().unwrap().selection.objects.clone()
    };
    set_pref(&mut s, "typeSelectionByPathOnly", json!(true));
    let x = first.x + 15.0;
    assert!(click(&mut s, "selection", x, 300.0).is_empty());
    assert_eq!(click(&mut s, "selection", x, second.y + 1.0), vec![src], "the second line's baseline");
    assert!(click(&mut s, "selection", x, (first.y + second.y) / 2.0).is_empty(), "between the lines");
    // The Type tool: a click among the glyphs puts the caret in that type, it adds none.
    assert_eq!(click(&mut s, "type", x, first.y - 7.0), vec![src]);
    assert!(s.tool_wants_text(), "editing");
    let texts =
        |s: &Session| s.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.iter().filter(|n| matches!(n.kind, NodeKind::Text(_))).count());
    assert_eq!(texts(&s), 1, "no new type");
    // The Eyedropper: a click among the glyphs samples the type into the selected type.
    s.select_tool("selection", v).unwrap();
    let dst = NodeId(s.execute("text.create", &json!({"x": 100, "y": 300, "text": "Target"})).unwrap()["id"].as_u64().unwrap());
    click(&mut s, "eyedropper", x, first.y - 7.0);
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(dst).unwrap().kind else { panic!("not type") };
    assert_eq!((t.first_style().size, t.first_style().fill.color().map(|c| c.to_hex())), (20.0, Some("#00ff00".into())));
}

/// Preferences › Type › Show Font Names in English (#394): stored and read by the font menus.
#[test]
fn show_font_names_in_english_preference_is_stored() {
    let mut s = Session::new();
    assert!(s.prefs.font_names_in_english, "on by default");
    set_pref(&mut s, "fontNamesInEnglish", json!(false));
    assert!(!s.prefs.font_names_in_english);
    set_pref(&mut s, "fontNamesInEnglish", json!(true));
    assert!(s.prefs.font_names_in_english);
}

/// Preferences › Hyphenation › Exceptions (#394): `prefs.set` pushes the list into the hyphenator
/// and bumps open documents so type reflows. Parsing of the list is covered in `vectorcraft-text`.
#[test]
fn hyphenation_exceptions_preference_reaches_the_hyphenator() {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            vectorcraft_text::set_hyphenation_exceptions("");
        }
    }
    let _clear = Clear;
    let mut s = Session::new();
    set_pref(&mut s, "hyphenationExceptions", json!(""));
    assert!(vectorcraft_text::hyphenation_exceptions().is_empty());
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let before = s.doc().unwrap().revision;
    set_pref(&mut s, "hyphenationExceptions", json!("typography, hap-pen"));
    assert_eq!(s.prefs.hyphenation_exceptions, "typography, hap-pen");
    assert_eq!(vectorcraft_text::hyphenation_exceptions(), "typography, hap-pen");
    assert!(s.doc().unwrap().revision > before, "type reflows");
    set_pref(&mut s, "hyphenationExceptions", json!(""));
    assert!(vectorcraft_text::hyphenation_exceptions().is_empty());
}
