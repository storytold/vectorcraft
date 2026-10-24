//! Preferences: `prefs.get` / `prefs.set` / `prefs.reset` / `prefs.list`.

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_tools::{PointerEvent, PointerKind};

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

/// Guides & Grid › Gridline every and Subdivisions (#394): set with no document open, as the
/// desktop app restores them at launch, they give `file.new`'s document their grid.
#[test]
fn grid_prefs_apply_to_new_documents() {
    let mut s = Session::new();
    s.execute("prefs.set", &json!({"values": {"gridlineEvery": 36, "gridSubdivisions": 4}})).unwrap();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let g = &s.doc().unwrap().doc.grid;
    assert_eq!((g.spacing, g.subdivisions), (36.0, 4));
}

/// An SVG file with no grid in it.
const GRIDLESS_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="10" height="10"/></svg>"#;

/// Gridline every and Subdivisions (#394): an SVG or a plain PDF opens with their grid, while a
/// native file, and the native document a PDF saved with Preserve Editing carries, keep the grid
/// they were saved with.
#[test]
fn grid_prefs_apply_to_opened_files_without_a_grid() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s.execute("prefs.set", &json!({"values": {"gridlineEvery": 18, "gridSubdivisions": 3}})).unwrap();
    let native = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let mut pdf = |editing: bool| {
        let v = s.execute("document.exportPdf", &json!({"preserveEditing": editing})).unwrap();
        vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
    };
    let (editable, plain) = (pdf(true), pdf(false));
    s.execute("prefs.set", &json!({"values": {"gridlineEvery": 36, "gridSubdivisions": 4}})).unwrap();
    let mut open = |name: &str, bytes: &[u8]| {
        let r = s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap();
        let g = &s.doc().unwrap().doc.grid;
        (g.spacing, g.subdivisions, r["restored"].as_bool().unwrap())
    };
    assert_eq!(open("drawing.svg", GRIDLESS_SVG.as_bytes()), (36.0, 4, false));
    assert_eq!(open("plain.pdf", &plain), (36.0, 4, false));
    assert_eq!(open("saved.vectorcraft", &native), (18.0, 3, false));
    assert_eq!(open("editable.pdf", &editable), (18.0, 3, true));
}

/// Gridline every and Subdivisions (#394): File › Revert reads the file again, and an SVG gets
/// their grid while a native file gets the grid it was saved with.
#[test]
fn grid_prefs_apply_to_reverted_files_without_a_grid() {
    let dir = std::env::temp_dir().join(format!("vc-grid-revert-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = |name: &str| dir.join(name).to_string_lossy().to_string();
    let (svg, native) = (path("drawing.svg"), path("saved.vectorcraft"));
    std::fs::write(&svg, GRIDLESS_SVG).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s.execute("prefs.set", &json!({"values": {"gridlineEvery": 18, "gridSubdivisions": 3}})).unwrap();
    s.execute("document.save", &json!({"path": native})).unwrap();
    // The preference changes while the file is open, which sets the open documents' grid.
    let mut reverted = |file: &str, every: f64| {
        s.execute("document.open", &json!({"path": file})).unwrap();
        s.execute("prefs.set", &json!({"values": {"gridlineEvery": every, "gridSubdivisions": 4}})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 10, "height": 10})).unwrap();
        s.execute("file.revert", &json!({})).unwrap();
        let g = &s.doc().unwrap().doc.grid;
        (g.spacing, g.subdivisions)
    };
    assert_eq!(reverted(&svg, 36.0), (36.0, 4));
    assert_eq!(reverted(&native, 24.0), (18.0, 3));
    let _ = std::fs::remove_dir_all(dir);
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

/// Performance › Linux Display Server (#1151): automatic by default, X11 by value or label, and
/// anything else is an error that changes nothing.
#[test]
fn linux_display_server_defaults_to_automatic_and_validates() {
    let mut s = Session::new();
    assert_eq!(s.prefs.linux_display_server, "automatic");
    s.execute("prefs.set", &json!({"key": "linuxDisplayServer", "value": "X11 (XWayland)"})).unwrap();
    assert_eq!(s.prefs.linux_display_server, "x11");
    for bad in [json!("wayland"), json!(""), json!(11), json!(null)] {
        assert!(s.execute("prefs.set", &json!({"key": "linuxDisplayServer", "value": bad})).is_err(), "{bad}");
        assert_eq!(s.prefs.linux_display_server, "x11");
    }
    assert_eq!(s.prefs.to_json()["linuxDisplayServer"], "x11");
    s.execute("prefs.reset", &json!({"category": "Performance"})).unwrap();
    assert_eq!(s.prefs.linux_display_server, "automatic");
}

/// Linux Display Server's pen choice (#2028): set by value or by label, saved, and off by default.
#[test]
fn linux_display_server_has_a_choice_for_an_attached_pen() {
    let mut s = Session::new();
    s.execute("prefs.set", &json!({"key": "linuxDisplayServer", "value": "x11WithPen"})).unwrap();
    assert_eq!(s.prefs.linux_display_server, "x11WithPen");
    assert_eq!(s.prefs.to_json()["linuxDisplayServer"], "x11WithPen");
    s.execute("prefs.set", &json!({"key": "linuxDisplayServer", "value": "automatic"})).unwrap();
    s.execute("prefs.set", &json!({"key": "linuxDisplayServer", "value": "X11 (XWayland) When a Pen Is Attached"})).unwrap();
    assert_eq!(s.prefs.linux_display_server, "x11WithPen");
    s.execute("prefs.reset", &json!({"category": "Performance"})).unwrap();
    assert_eq!(s.prefs.linux_display_server, "automatic");
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

/// Double Click To Isolate: a double-click with the Selection tool isolates a single object too,
/// not only a group. Inside, only it is hit, marqueed and selected by Select All; new art goes
/// beside it; a double-click on the empty canvas leaves, with it selected. Type isn't isolated:
/// a double-click on it switches to the Type tool.
#[test]
fn double_clicking_a_single_object_isolates_it() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = |v: Value| NodeId(v["id"].as_u64().unwrap());
    let rect = |s: &mut Session, x: f64| id(s.execute("shape.rectangle", &json!({"x": x, "y": 100, "width": 50, "height": 50})).unwrap());
    let (a, b) = (rect(&mut s, 100.0), rect(&mut s, 200.0));
    s.select_tool("selection", ViewInfo::default()).unwrap();
    let ev = |s: &mut Session, kinds: &[PointerKind], x: f64, y: f64| {
        for k in kinds {
            s.pointer(&PointerEvent::new(*k, x, y), ViewInfo::default()).unwrap();
        }
    };
    let click = [PointerKind::Down, PointerKind::Up];
    let double = [PointerKind::DoubleClick];
    ev(&mut s, &double, 125.0, 125.0);
    assert_eq!(s.doc().unwrap().isolation, Some(a));
    // Only the isolated object is hit: a click on the other selects nothing.
    ev(&mut s, &click, 225.0, 125.0);
    assert!(s.doc().unwrap().selection.objects.is_empty());
    ev(&mut s, &click, 125.0, 125.0);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    // A marquee over both and Select All take it alone.
    s.execute("select.none", &json!({})).unwrap();
    ev(&mut s, &[PointerKind::Down], 90.0, 90.0);
    ev(&mut s, &[PointerKind::Move, PointerKind::Up], 300.0, 300.0);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    s.execute("select.none", &json!({})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    // A second double-click on it stays in isolation; new art lands on its layer.
    ev(&mut s, &double, 125.0, 125.0);
    assert_eq!(s.doc().unwrap().isolation, Some(a));
    let c = rect(&mut s, 300.0);
    let st = s.doc().unwrap();
    assert_eq!(st.doc.parent_of(c), st.doc.parent_of(a));
    // A double-click on the empty canvas leaves isolation, the object selected.
    ev(&mut s, &double, 50.0, 350.0);
    let st = s.doc().unwrap();
    assert_eq!((st.isolation, st.selection.objects.clone()), (None, vec![a]));
    assert!(st.doc.node(b).is_some());
    // Type is not isolated.
    let t = id(s.execute("text.create", &json!({"x": 100, "y": 300, "text": "Type"})).unwrap());
    assert!(s.execute("object.isolate", &json!({"id": t.0})).is_err());
    s.execute("prefs.set", &json!({"key": "doubleClickToIsolate", "value": false})).unwrap();
    ev(&mut s, &double, 225.0, 125.0);
    assert_eq!(s.doc().unwrap().isolation, None, "off: no isolation");
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
    assert_eq!(cursor(&mut s, "eyedropper"), Cursor::Eyedropper);
    s.execute("prefs.set", &json!({"key": "usePreciseCursors", "value": true})).unwrap();
    assert_eq!(cursor(&mut s, "pen"), Cursor::Crosshair);
    assert_eq!(cursor(&mut s, "eyedropper"), Cursor::Crosshair);
    assert_eq!(cursor(&mut s, "selection"), Cursor::Arrow);
    s.execute("prefs.set", &json!({"key": "usePreciseCursors", "value": false})).unwrap();
    assert_eq!(cursor(&mut s, "eyedropper"), Cursor::Eyedropper);
}

/// Use Precise Cursors changes the pointer of the tools that have one of their own: off, each shows its own
/// pointer; on, a crosshair. Every other drawing tool shows a crosshair either way, and the Type tool and the
/// Scissors over no path keep theirs.
#[test]
fn use_precise_cursors_swaps_the_tools_own_pointers_for_a_crosshair() {
    use vectorcraft_tools::{Cursor, Mods};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let at = vectorcraft_geom::Point::new(300.0, 300.0);
    let cursor = |s: &mut Session, tool: &str| {
        s.select_tool(tool, ViewInfo::default()).unwrap();
        s.cursor(at, Mods::default(), ViewInfo::default())
    };
    // Off: the pointer of each tool whose own pointer the preference replaces.
    let own = [
        ("pen", Cursor::Pen),
        ("addAnchor", Cursor::PenAdd),
        ("deleteAnchor", Cursor::PenDelete),
        ("anchorPoint", Cursor::AnchorPoint),
        ("curvature", Cursor::Curvature),
        ("mesh", Cursor::MeshAdd),
        ("eyedropper", Cursor::Eyedropper),
        ("slice", Cursor::Slice),
        ("blend", Cursor::Blend),
        ("shapeBuilder", Cursor::ShapeBuilder),
        ("paintbrush", Cursor::Paintbrush),
        ("blobBrush", Cursor::BlobBrush),
        ("shaper", Cursor::Shaper),
        ("pencil", Cursor::Pencil),
        ("smooth", Cursor::Smooth),
        ("pathEraser", Cursor::PathEraser),
        ("join", Cursor::Join),
        ("eraser", Cursor::Eraser),
        ("knife", Cursor::Knife),
    ];
    for (tool, c) in own {
        assert_eq!(cursor(&mut s, tool), c, "{tool} off");
    }
    // The tools that show a crosshair whatever the preference says.
    let always = [
        "lineSegment",
        "arc",
        "spiral",
        "rectangularGrid",
        "polarGrid",
        "rectangle",
        "roundedRectangle",
        "ellipse",
        "polygon",
        "star",
        "flare",
        "mirrorCut",
        "lineCut",
        "rectCut",
        "gradient",
    ];
    for tool in always {
        assert_eq!(cursor(&mut s, tool), Cursor::Crosshair, "{tool} off");
    }
    s.execute("prefs.set", &json!({"key": "usePreciseCursors", "value": true})).unwrap();
    for (tool, _) in own {
        assert_eq!(cursor(&mut s, tool), Cursor::Crosshair, "{tool} on");
    }
    for tool in always {
        assert_eq!(cursor(&mut s, tool), Cursor::Crosshair, "{tool} on");
    }
    assert_eq!(cursor(&mut s, "type"), Cursor::Text);
    assert_eq!(cursor(&mut s, "scissors"), Cursor::ScissorsNotAllowed);
    s.execute("prefs.set", &json!({"key": "usePreciseCursors", "value": false})).unwrap();
    for (tool, c) in own {
        assert_eq!(cursor(&mut s, tool), c, "{tool} off again");
    }
}

/// With Alt held while drawing, the Pencil and the Paintbrush show their closing cursors, and Use Precise Cursors
/// turns those into a crosshair too.
#[test]
fn alt_shows_the_pencil_and_paintbrush_closing_cursors() {
    use vectorcraft_tools::{Cursor, Mods, PointerEvent, PointerKind};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let at = vectorcraft_geom::Point::new(300.0, 300.0);
    let alt = Mods { alt: true, ..Default::default() };
    for precise in [false, true] {
        s.execute("prefs.set", &json!({"key": "usePreciseCursors", "value": precise})).unwrap();
        for (tool, c) in [("pencil", Cursor::PencilClose), ("paintbrush", Cursor::PaintbrushClose)] {
            s.select_tool(tool, ViewInfo::default()).unwrap();
            s.pointer(&PointerEvent::new(PointerKind::Down, 250.0, 250.0), ViewInfo::default()).unwrap();
            s.pointer(&PointerEvent::new(PointerKind::Drag, 300.0, 300.0), ViewInfo::default()).unwrap();
            assert_eq!(s.cursor(at, alt, ViewInfo::default()), if precise { Cursor::Crosshair } else { c }, "{tool}, precise {precise}");
            s.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 300.0), ViewInfo::default()).unwrap();
        }
    }
}

/// Every state of the Pen's pointer, and the other drawing pointers, turn into a crosshair; the rest stay.
#[test]
fn precise_cursors_replace_every_pen_state() {
    use vectorcraft_tools::Cursor;
    for c in [
        Cursor::Pen,
        Cursor::PenDrawing,
        Cursor::PenAdd,
        Cursor::PenDelete,
        Cursor::PenClose,
        Cursor::PenContinue,
        Cursor::PenJoin,
        Cursor::PenConvert,
        Cursor::Eyedropper,
        Cursor::Slice,
        Cursor::Blend,
        Cursor::BlendObject,
        Cursor::BlendAnchor,
        Cursor::ShapeBuilder,
        Cursor::ShapeBuilderErase,
        Cursor::AnchorPoint,
        Cursor::Curvature,
    ] {
        assert_eq!(c.precise(), Cursor::Crosshair, "{c:?}");
    }
    for c in [Cursor::Arrow, Cursor::Move, Cursor::Text, Cursor::Hand, Cursor::NotAllowed, Cursor::SliceSelect] {
        assert_eq!(c.precise(), c, "{c:?}");
    }
}

/// Switching the preference is not an edit: no undo step, no unsaved changes.
#[test]
fn use_precise_cursors_is_not_an_edit() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let before = (s.active().unwrap().history.undo.len(), s.active().unwrap().history.redo.len());
    assert!(!s.active().unwrap().is_dirty());
    s.execute("prefs.set", &json!({"key": "usePreciseCursors", "value": true})).unwrap();
    assert!(s.prefs.use_precise_cursors);
    let st = s.active().unwrap();
    assert_eq!((st.history.undo.len(), st.history.redo.len()), before, "no undo step");
    assert!(!st.is_dirty(), "the document stays saved");
}

/// The preference is saved with the others and reads back as it was.
#[test]
fn use_precise_cursors_is_saved_and_restored() {
    let mut s = Session::new();
    assert!(!s.prefs.use_precise_cursors, "off by default");
    s.execute("prefs.set", &json!({"key": "usePreciseCursors", "value": true})).unwrap();
    let saved = s.prefs.to_json();
    assert_eq!(saved["usePreciseCursors"], json!(true));
    let mut t = Session::new();
    t.apply_prefs(serde_json::from_value(saved).unwrap());
    assert!(t.prefs.use_precise_cursors);
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

/// Smart Guides › Invoke Distance Guides: with a square selected, Alt held over another square to
/// its right shows the gap between them in the document's General unit; with the preference or
/// View › Smart Guides off, nothing.
#[test]
fn invoke_distance_guides_measures_the_selection_to_the_object_under_alt() {
    use vectorcraft_tools::{Mods, Overlay, PointerKind::Move};
    let mut s = new_doc();
    let a = square(&mut s, 100.0, 100.0);
    square(&mut s, 250.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let shown = |s: &mut Session, v: ViewInfo| {
        s.pointer(&PointerEvent::new(Move, 275.0, 125.0).with_mods(Mods { alt: true, ..Mods::default() }), v).unwrap();
        s.overlays(v).into_iter().filter_map(|o| if let Overlay::Distance { text, .. } = o { Some(text) } else { None }).collect::<Vec<_>>()
    };
    assert_eq!(shown(&mut s, v), ["100.00 pt"], "the 100 pt gap; the tops and bottoms are level");
    s.execute("document.setUnits", &json!({"units": "Millimeters"})).unwrap();
    assert_eq!(shown(&mut s, v), ["35.28 mm"]);
    assert!(shown(&mut s, ViewInfo { smart_guides: false, ..v }).is_empty(), "View › Smart Guides off");
    set_pref(&mut s, "distanceGuides", json!(false));
    assert!(shown(&mut s, v).is_empty(), "Invoke Distance Guides off");
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

/// A 50 pt square at (100, 100) with a 1 pt stroke, filled with a 20 pt pattern, selected.
fn patterned_square(s: &mut Session) -> NodeId {
    let tile = square(s, 0.0, 0.0);
    s.execute("select.set", &json!({"ids": [tile.0]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    let id = square(s, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [id.0], "swatch": "Dots"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    id
}

/// The tile transform of the pattern fill of `id`.
fn pattern_tiles(s: &Session, id: NodeId) -> [f64; 6] {
    match s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint() {
        vectorcraft_color::Paint::Pattern { xf, .. } => xf.as_coeffs(),
        p => panic!("not a pattern: {p:?}"),
    }
}

/// Transform Again repeats the options of the transform it repeats, whatever the preferences say:
/// a Move with Transform Patterns off while Transform Pattern Tiles is on, and a Scale with Scale
/// Strokes & Effects on while its preference is off.
#[test]
fn transform_again_repeats_the_options_of_the_transform_it_repeats() {
    let mut s = new_doc();
    let id = patterned_square(&mut s);
    set_pref(&mut s, "transformPatternTiles", json!(true));
    s.execute("object.move", &json!({"dx": 10, "dy": 0, "patterns": false})).unwrap();
    s.execute("object.transformAgain", &json!({})).unwrap();
    assert_eq!(top_left(&s, id), (120.0, 100.0), "the square moved twice");
    assert_eq!(pattern_tiles(&s, id), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], "the tiles stay");
    assert_eq!(s.journal.last().unwrap().1, json!({"patterns": false}), "the journal keeps the choice");
    s.execute("object.scale", &json!({"sx": 200, "strokes": true})).unwrap();
    s.execute("object.transformAgain", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().appearance.stroke_width(), 4.0, "the 1 pt stroke scaled twice");
}

/// A recorded Transform Again plays back with the Transform Patterns its step records, whatever the
/// transform it repeats used.
#[test]
fn a_recorded_transform_again_plays_back_with_its_transform_patterns() {
    let mut s = new_doc();
    let id = patterned_square(&mut s);
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    let start = s.journal.len();
    s.execute("object.transformAgain", &json!({})).unwrap();
    let action = s.journal_for_action(start);
    assert_eq!(action, [("object.transformAgain".to_string(), json!({"patterns": false}))], "recorded with the preference off");
    // With the preference on, a Move moves the tiles, and the recorded Transform Again leaves them.
    set_pref(&mut s, "transformPatternTiles", json!(true));
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    assert_eq!(pattern_tiles(&s, id), [1.0, 0.0, 0.0, 1.0, 10.0, 0.0]);
    for (cmd, p) in &action {
        s.execute(cmd, p).unwrap();
    }
    assert_eq!(top_left(&s, id), (140.0, 100.0), "the square moved");
    assert_eq!(pattern_tiles(&s, id), [1.0, 0.0, 0.0, 1.0, 10.0, 0.0], "the tiles stay");
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

/// General › Transform Pattern Tiles (#394) applies to art moved or duplicated with its artboard
/// as it does to the transforms: off, the tiles of a pattern-filled square stay in place; on, they
/// move with the square or its copy, and `patterns: false` keeps them in place for one call.
#[test]
fn art_moved_with_its_artboard_follows_transform_pattern_tiles() {
    use vectorcraft_color::Paint;
    let mut s = new_doc();
    let plain = square(&mut s, 500.0, 0.0);
    s.execute("select.set", &json!({"ids": [plain.0]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 400, "height": 400})).unwrap();
    let filled = square(&mut s, 1100.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [filled.0], "swatch": "Dots"})).unwrap();
    let tiles = |s: &Session, id: NodeId| match s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint() {
        Paint::Pattern { xf, .. } => xf.as_coeffs(),
        p => panic!("not a pattern: {p:?}"),
    };
    // Artboard 1 starts at x 1000 and holds the square; artboard 0 holds no art, and `plain` is on
    // no artboard. Duplicate Artboard puts the copy 20 pt right of the last artboard, artboard 1, so
    // the art's copy is 420 pt right of the square. Rearranged in a row, artboard 1 moves to x 420,
    // 20 pt right of artboard 0, and `artboard.setProps` puts it at x 1030. `moved` and `ids` name
    // the art moved or its copies.
    for (cmd, p, key, d) in [
        ("artboard.move", json!({"index": 1, "dx": 30, "dy": 0, "moveArt": true}), Some("moved"), [30.0, 0.0]),
        ("artboard.move", json!({"indices": [0, 1], "dx": 30, "dy": 0, "moveArt": true}), Some("moved"), [30.0, 0.0]),
        ("artboard.move", json!({"index": 1, "dx": 0, "dy": 500, "moveArt": true, "copy": true}), Some("moved"), [0.0, 500.0]),
        ("artboard.duplicate", json!({"index": 1, "art": true}), Some("ids"), [420.0, 0.0]),
        ("artboard.rearrange", json!({"layout": "row"}), None, [-580.0, 0.0]),
        ("artboard.setProps", json!({"index": 1, "x": 1030, "moveArt": true}), None, [30.0, 0.0]),
    ] {
        let run = |s: &mut Session, p: &Value| {
            let r = s.execute(cmd, p).unwrap();
            let ids = key.and_then(|k| r[k].as_array().cloned()).unwrap_or_else(|| vec![json!(filled.0)]);
            assert_eq!(ids.len(), 1, "{cmd}: the square alone goes with the artboard");
            NodeId(ids[0].as_u64().unwrap())
        };
        let id = run(&mut s, &p);
        assert_eq!(tiles(&s, id), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], "{cmd}: off, the tiles stay");
        assert_eq!(s.journal.last().unwrap().1["patterns"], json!(false), "{cmd}: the journal keeps the choice");
        s.execute("edit.undo", &json!({})).unwrap();
        set_pref(&mut s, "transformPatternTiles", json!(true));
        let id = run(&mut s, &p);
        assert_eq!(tiles(&s, id), [1.0, 0.0, 0.0, 1.0, d[0], d[1]], "{cmd}: on, they move with the square");
        assert_eq!(s.journal.last().unwrap().1["patterns"], json!(true), "{cmd}: the journal keeps the choice");
        s.execute("edit.undo", &json!({})).unwrap();
        let mut p = p;
        p["patterns"] = json!(false);
        let id = run(&mut s, &p);
        assert_eq!(tiles(&s, id), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], "{cmd}: the param wins");
        s.execute("edit.undo", &json!({})).unwrap();
        set_pref(&mut s, "transformPatternTiles", json!(false));
    }
}

/// General › Transform Pattern Tiles (#394) applies to the objects selected whole that move with
/// direct-selected anchors (an arrow key, `path.moveAnchors` or a Direct Selection drag of an
/// anchor): off, the tiles of a pattern-filled square stay in place; on, they move with it, and
/// `patterns: false` keeps them in place for one call.
#[test]
fn whole_objects_moved_with_selected_anchors_follow_transform_pattern_tiles() {
    use vectorcraft_tools::PointerKind::{Down, Drag, Up};
    let mut s = new_doc();
    let filled = patterned_square(&mut s);
    let other = square(&mut s, 300.0, 100.0);
    let v = ViewInfo { smart_guides: false, snap_to_point: false, corner_widgets: false, ..ViewInfo::default() };
    s.select_tool("directSelection", v).unwrap();
    // The square is selected whole with the top-left anchor of `other`, as a click on the square's
    // fill and a Shift-click on that anchor select them. Each move takes both 10 pt right: Shift
    // with an arrow key, `path.moveAnchors`, and a drag of the anchor (no params).
    for (cmd, p) in [
        ("object.nudge", Some(json!({"dx": 1, "dy": 0, "big": true}))),
        ("path.moveAnchors", Some(json!({"dx": 10, "dy": 0}))),
        ("path.moveAnchors", None),
    ] {
        let run = |s: &mut Session, p: Option<&Value>| {
            s.execute("select.set", &json!({"ids": [filled.0]})).unwrap();
            s.execute("select.anchors", &json!({"id": other.0, "anchors": [[0, 0]], "mode": "add"})).unwrap();
            match p {
                Some(p) => {
                    s.execute(cmd, p).unwrap();
                }
                None => gesture(s, v, &[(Down, 300.0, 100.0), (Drag, 305.0, 100.0), (Drag, 310.0, 100.0), (Up, 310.0, 100.0)]),
            }
            assert_eq!(top_left(s, filled), (110.0, 100.0), "{cmd}: the square moves whole");
            let anchors = &s.doc().unwrap().doc.node(other).unwrap().path_data().unwrap().subpaths[0].anchors;
            assert_eq!((anchors[0].p.x, anchors[1].p.x), (310.0, 350.0), "{cmd}: the selected anchor moves alone");
            assert_eq!(s.journal.last().unwrap().0, cmd);
        };
        run(&mut s, p.as_ref());
        assert_eq!(pattern_tiles(&s, filled), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], "{cmd}: off, the tiles stay");
        assert_eq!(s.journal.last().unwrap().1["patterns"], json!(false), "{cmd}: the journal keeps the choice");
        s.execute("edit.undo", &json!({})).unwrap();
        set_pref(&mut s, "transformPatternTiles", json!(true));
        run(&mut s, p.as_ref());
        assert_eq!(pattern_tiles(&s, filled), [1.0, 0.0, 0.0, 1.0, 10.0, 0.0], "{cmd}: on, they move with the square");
        assert_eq!(s.journal.last().unwrap().1["patterns"], json!(true), "{cmd}: the journal keeps the choice");
        s.execute("edit.undo", &json!({})).unwrap();
        if let Some(mut p) = p {
            p["patterns"] = json!(false);
            run(&mut s, Some(&p));
            assert_eq!(pattern_tiles(&s, filled), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], "{cmd}: the param wins");
            s.execute("edit.undo", &json!({})).unwrap();
        }
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
    // The type objects given in `ids` step in full while the tool edits.
    s.execute("type.step", &json!({"attribute": "size", "ids": [id.0]})).unwrap();
    assert_eq!((style_at(&s, id, 0).size, style_at(&s, id, 6).size), (22.0, 25.0));
}

/// Auto leading steps from the leading its paragraph's Justification › Auto Leading sets, which
/// is what the layout shows; a run spanning paragraphs with different Auto Leading is split.
/// Fill with Placeholder Text counts lines at that leading too.
#[test]
fn auto_leading_steps_and_fills_from_the_paragraph_auto_leading() {
    let mut s = new_doc();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "One\nTwo", "size": 20})).unwrap()["id"].as_u64().unwrap());
    s.execute("text.setFormat", &json!({"ids": [id.0], "autoLeading": 200, "start": 5, "end": 5})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("type.step", &json!({"attribute": "leading"})).unwrap();
    let lead = |s: &Session, byte| style_at(s, id, byte).leading.unwrap();
    assert!((lead(&s, 0) - (20.0 * 1.2 + 2.0)).abs() < 1e-9, "default Auto Leading 120%");
    assert!((lead(&s, 3) - (20.0 * 1.2 + 2.0)).abs() < 1e-9, "the line break is the first paragraph's");
    assert!((lead(&s, 5) - (20.0 * 2.0 + 2.0)).abs() < 1e-9, "Auto Leading 200%");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((style_at(&s, id, 0).leading, style_at(&s, id, 5).leading), (None, None), "one undo step");

    let area = |s: &mut Session, x: f64| {
        let r = s.execute("text.create", &json!({"x": x, "y": 10, "text": "x", "size": 10, "area": {"width": 100, "height": 200}})).unwrap();
        NodeId(r["id"].as_u64().unwrap())
    };
    let (normal, airy) = (area(&mut s, 10.0), area(&mut s, 200.0));
    s.execute("text.setFormat", &json!({"ids": [airy.0], "autoLeading": 240})).unwrap();
    s.execute("type.fillPlaceholder", &json!({"ids": [normal.0, airy.0]})).unwrap();
    let len = |s: &Session, id: NodeId| match s.doc().unwrap().doc.node(id).map(|n| &n.kind) {
        Some(vectorcraft_doc::NodeKind::Text(t)) => t.plain_text().len(),
        _ => panic!("not text"),
    };
    let (n, a) = (len(&s, normal), len(&s, airy));
    assert!(a * 3 < n * 2 && a * 3 > n, "half as many lines at twice the leading: {a} vs {n}");
}

/// Cmd/Ctrl+Alt+Shift+. and , step the font size by five times the Size/Leading increment, on
/// the selected type objects and on the Type tool's selected text.
#[test]
fn font_size_steps_by_five_increments() {
    use vectorcraft_tools::PointerKind::{Down, Up};
    let mut s = new_doc();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Hello world", "size": 12})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    assert_eq!(find_command("type.size.increaseByFive").unwrap().shortcut, Some("Cmd+Alt+Shift+."));
    assert_eq!(find_command("type.size.decreaseByFive").unwrap().shortcut, Some("Cmd+Alt+Shift+,"));
    s.execute("type.size.increaseByFive", &json!({})).unwrap();
    assert_eq!(style_at(&s, id, 0).size, 22.0, "five steps of the 2 pt Size/Leading increment");
    s.execute("type.size.decreaseByFive", &json!({})).unwrap();
    assert_eq!(style_at(&s, id, 0).size, 12.0);
    set_pref(&mut s, "typeSizeIncrement", json!(3));
    s.execute("type.size.increaseByFive", &json!({})).unwrap();
    assert_eq!(style_at(&s, id, 10).size, 27.0, "the step follows the preference");
    // While the Type tool edits, only its selected text steps.
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    gesture(&mut s, v, &[(Down, 102.0, 95.0), (Up, 102.0, 95.0)]);
    assert_eq!(s.tool_options()["editing"], json!(id.0), "editing the text");
    s.set_tool_option("select", &json!({"start": 6, "end": 11}));
    s.execute("type.size.decreaseByFive", &json!({})).unwrap();
    assert_eq!((style_at(&s, id, 0).size, style_at(&s, id, 6).size), (27.0, 12.0));
}

/// Cmd/Ctrl+Alt+Q resets tracking to 0 and kerning to Auto (0 in the Character panel) in every
/// selected type object and in the Type tool's selected text. At a caret it resets only the
/// kerning of the character before it. It is one undo step. Type objects given in `ids` are reset
/// in full while the Type tool edits, and the typing in progress ends first.
#[test]
fn reset_tracking_sets_tracking_and_kerning_to_zero() {
    use vectorcraft_tools::PointerKind::{Down, Up};
    let mut s = new_doc();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Hello world", "size": 20})).unwrap()["id"].as_u64().unwrap());
    let spaced = |s: &mut Session| s.execute("text.setRangeStyle", &json!({"id": id.0, "tracking": 200, "kerning": -40})).unwrap();
    let spacing = |s: &Session, byte| {
        let st = style_at(s, id, byte);
        (st.tracking, st.kerning)
    };
    spaced(&mut s);
    let other = NodeId(s.execute("text.create", &json!({"x": 100, "y": 300, "text": "Other", "size": 20})).unwrap()["id"].as_u64().unwrap());
    s.execute("text.setRangeStyle", &json!({"id": other.0, "tracking": 200})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    assert_eq!(find_command("type.resetTracking").unwrap().shortcut, Some("Cmd+Alt+Q"));
    assert_eq!(s.execute("type.resetTracking", &json!({})).unwrap()["ids"], json!([id.0]));
    assert_eq!((spacing(&s, 0), spacing(&s, 10)), ((0.0, None), (0.0, None)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spacing(&s, 0), (200.0, Some(-40.0)), "one undo step");
    // With the Type tool, it resets only the selected text.
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    gesture(&mut s, v, &[(Down, 102.0, 95.0), (Up, 102.0, 95.0)]);
    assert_eq!(s.tool_options()["editing"], json!(id.0), "editing the text");
    s.set_tool_option("select", &json!({"start": 0, "end": 5}));
    s.execute("type.resetTracking", &json!({})).unwrap();
    assert_eq!((spacing(&s, 4), spacing(&s, 6)), ((0.0, None), (200.0, Some(-40.0))));
    // At a caret it resets only the kerning of the character before it, and the tracking does not change.
    s.set_tool_option("select", &json!({"start": 8, "end": 8}));
    s.execute("type.resetTracking", &json!({})).unwrap();
    assert_eq!((spacing(&s, 6), spacing(&s, 7), spacing(&s, 8)), ((200.0, Some(-40.0)), (200.0, None), (200.0, Some(-40.0))));
    // While the Type tool edits, the type objects in `ids` are reset in full.
    s.execute("type.resetTracking", &json!({"ids": [id.0]})).unwrap();
    assert_eq!((spacing(&s, 6), spacing(&s, 10)), ((0.0, None), (0.0, None)));
    // Typing in progress ends first, so the reset of another object stays when typing goes on.
    s.tool_text("!", v).unwrap();
    s.execute("type.resetTracking", &json!({"ids": [other.0]})).unwrap();
    s.tool_text("?", v).unwrap();
    assert_eq!(style_at(&s, other, 0).tracking, 0.0, "the reset stays");
}

/// Cmd/Ctrl+Shift+X resets the horizontal scale to 100% in every selected type object and in the
/// Type tool's selected text, as one undo step, and leaves the vertical scale as it is. At a caret
/// it leaves the text as it is. On a stretched object the scale the Character panel shows, which includes the
/// object's own stretch, goes to 100%. Type objects given in `ids` are reset in full while the
/// Type tool edits.
#[test]
fn reset_horizontal_scale_sets_it_to_100_percent() {
    use vectorcraft_tools::PointerKind::{Down, Up};
    let mut s = new_doc();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Hello world", "size": 20})).unwrap()["id"].as_u64().unwrap());
    let scales = |s: &Session, byte| {
        let st = style_at(s, id, byte);
        (st.h_scale, st.v_scale)
    };
    s.execute("text.setRangeStyle", &json!({"id": id.0, "hScale": 150, "vScale": 80})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    assert_eq!(find_command("type.resetHorizontalScale").unwrap().shortcut, Some("Cmd+Shift+X"));
    assert_eq!(s.execute("type.resetHorizontalScale", &json!({})).unwrap()["ids"], json!([id.0]));
    assert_eq!((scales(&s, 0), scales(&s, 10)), ((100.0, 80.0), (100.0, 80.0)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(scales(&s, 0), (150.0, 80.0), "one undo step");
    // A type object stretched to twice its width shows 200% and keeps half of it stored.
    let wide = NodeId(s.execute("text.create", &json!({"x": 100, "y": 300, "text": "Wide", "size": 20})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [wide.0]})).unwrap();
    s.execute("object.scale", &json!({"sx": 200, "sy": 100})).unwrap();
    s.execute("type.resetHorizontalScale", &json!({})).unwrap();
    assert_eq!(style_at(&s, wide, 0).h_scale, 50.0);
    // With the Type tool, it resets only the selected text.
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    gesture(&mut s, v, &[(Down, 102.0, 95.0), (Up, 102.0, 95.0)]);
    assert_eq!(s.tool_options()["editing"], json!(id.0), "editing the text");
    s.set_tool_option("select", &json!({"start": 0, "end": 5}));
    s.execute("type.resetHorizontalScale", &json!({})).unwrap();
    assert_eq!((scales(&s, 4), scales(&s, 6)), ((100.0, 80.0), (150.0, 80.0)));
    // At a caret it leaves the text as it is.
    s.set_tool_option("select", &json!({"start": 8, "end": 8}));
    assert_eq!(s.execute("type.resetHorizontalScale", &json!({})).unwrap()["ids"], json!([]));
    assert_eq!((scales(&s, 7), scales(&s, 8)), ((150.0, 80.0), (150.0, 80.0)));
    // While the Type tool edits, the type objects in `ids` are reset in full.
    s.execute("type.resetHorizontalScale", &json!({"ids": [id.0]})).unwrap();
    assert_eq!((scales(&s, 6), scales(&s, 10)), ((100.0, 80.0), (100.0, 80.0)));
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
    // The first click ends editing the area type (#2871); the second places point type.
    for _ in 0..2 {
        gesture(&mut s, v, &[(Down, 50.0, 350.0), (Up, 50.0, 350.0)]);
    }
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

/// A character that keeps its font through Missing Glyph Protection keeps that font's Variable
/// Font settings too; the characters that take the new font lose them.
#[test]
fn missing_glyph_protection_keeps_the_variable_font_settings_of_the_font_kept() {
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY as FROM, two_axis_font};
    let (to, c) = ("JetBrains Mono", '\u{180}');
    let db = vectorcraft_text::FontDb::global();
    db.add_font(two_axis_font().unwrap());
    if !db.face(FROM, "Regular").is_some_and(|f| f.covers(c)) || db.face(to, "Regular").is_some_and(|f| f.covers(c)) {
        return; // An installed font of that name covers the character.
    }
    let mut s = new_doc();
    let id = NodeId(s.execute("text.create", &json!({"x": 10, "y": 50, "text": "a\u{180}c", "font": FROM})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("text.setFormat", &json!({"fontAxes": {"wght": 650}})).unwrap();
    let fonts = |s: &Session| [0, 1, 3].map(|b| style_at(s, id, b)).map(|st| (st.font_family, st.font_axes.get("wght").copied()));
    let (kept, moved) = ((FROM.to_string(), Some(650.0)), (to.to_string(), None));
    for change in [json!({"font": to}), json!({"id": id.0, "start": 0, "end": 4, "font": to})] {
        let cmd = if change.get("id").is_some() { "text.setRangeStyle" } else { "text.setStyle" };
        s.execute(cmd, &change).unwrap();
        assert_eq!(fonts(&s), [moved.clone(), kept.clone(), moved.clone()], "{cmd}");
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

/// A character that keeps its font through Missing Glyph Protection keeps the version of it the
/// type names.
#[test]
fn missing_glyph_protection_keeps_the_version_of_the_font_kept() {
    use vectorcraft_text::test_fonts::{TWIN_FAMILY as FROM, TWIN_VERSIONS, twin_font};
    let (to, c) = ("JetBrains Mono", '\u{180}');
    let db = vectorcraft_text::FontDb::global();
    db.add_font(twin_font(false).unwrap());
    let older = db.face_version(FROM, "Regular", Some(TWIN_VERSIONS[0]));
    if !older.is_some_and(|f| f.version == TWIN_VERSIONS[0] && f.covers(c)) || db.face(to, "Regular").is_some_and(|f| f.covers(c)) {
        return; // An installed font of that name covers the character.
    }
    let mut s = new_doc();
    let id = NodeId(s.execute("text.create", &json!({"x": 10, "y": 50, "text": "a\u{180}c", "font": FROM})).unwrap()["id"].as_u64().unwrap());
    if let Some(NodeKind::Text(t)) = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).map(|n| &mut n.kind) {
        t.runs[0].style.font_version = Some(TWIN_VERSIONS[0].into());
    }
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("text.setStyle", &json!({"font": to})).unwrap();
    let versions = [0, 1, 3].map(|b| (style_at(&s, id, b).font_family, style_at(&s, id, b).font_version));
    let (kept, moved) = ((FROM.to_string(), Some(TWIN_VERSIONS[0].to_string())), (to.to_string(), None));
    assert_eq!(versions, [moved.clone(), kept, moved]);
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

/// Preferences › Devices › Enable Touch Workspace (#394): stored and restored through `prefs.set`.
#[test]
fn touch_workspace_preference_is_stored() {
    let mut s = Session::new();
    assert!(s.prefs.touch_workspace, "on by default");
    set_pref(&mut s, "touchWorkspace", json!(false));
    assert!(!s.prefs.touch_workspace);
    set_pref(&mut s, "touchWorkspace", json!(true));
    assert!(s.prefs.touch_workspace);
}

/// Preferences › Type › Highlight Alternate Glyphs (#394): off by default so OpenType
/// substitutions are not pink on every canvas until the user turns the preference on.
#[test]
fn highlight_alternate_glyphs_preference_is_off_by_default() {
    let mut s = Session::new();
    assert!(!s.prefs.highlight_alternate_glyphs, "off by default");
    set_pref(&mut s, "highlightAlternateGlyphs", json!(true));
    assert!(s.prefs.highlight_alternate_glyphs);
    set_pref(&mut s, "highlightAlternateGlyphs", json!(false));
    assert!(!s.prefs.highlight_alternate_glyphs);
}

/// Preferences › Performance › Real-time Drawing and Editing (#394): off, the document isn't
/// previewed while dragging but the pending shape still shows as a canvas overlay until commit.
#[test]
fn real_time_drawing_off_skips_tool_previews_until_commit() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("rectangle", v).unwrap();
    set_pref(&mut s, "realTimeDrawing", json!(false));
    let paths = |s: &Session| {
        s.doc()
            .unwrap()
            .doc
            .layers
            .first()
            .and_then(|l| l.children())
            .map_or(0, |c| c.iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).count())
    };
    let rev = s.doc().unwrap().revision;
    assert_eq!(paths(&s), 0);
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 150.0, 150.0), v).unwrap();
    assert_eq!(s.doc().unwrap().revision, rev, "no preview steps");
    assert_eq!(paths(&s), 0, "the rectangle isn't in the document yet");
    assert!(s.overlays(v).iter().any(|o| matches!(o, vectorcraft_tools::Overlay::Path { .. })), "the deferred rectangle shows as an outline");
    s.pointer(&PointerEvent::new(PointerKind::Up, 150.0, 150.0), v).unwrap();
    assert!(s.doc().unwrap().revision > rev, "commit applies the shape");
    assert_eq!(paths(&s), 1);
    set_pref(&mut s, "realTimeDrawing", json!(true));
    let rev = s.doc().unwrap().revision;
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 280.0, 120.0), v).unwrap();
    assert!(s.doc().unwrap().revision > rev, "on again: previews run while dragging");
}

/// Real-time Drawing turned on mid-drag still applies the deferred preview on commit.
#[test]
fn real_time_drawing_turned_on_mid_drag_still_commits_deferred_preview() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("rectangle", v).unwrap();
    set_pref(&mut s, "realTimeDrawing", json!(false));
    let paths = |s: &Session| {
        s.doc()
            .unwrap()
            .doc
            .layers
            .first()
            .and_then(|l| l.children())
            .map_or(0, |c| c.iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).count())
    };
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 150.0, 150.0), v).unwrap();
    assert_eq!(paths(&s), 0);
    set_pref(&mut s, "realTimeDrawing", json!(true));
    s.pointer(&PointerEvent::new(PointerKind::Up, 150.0, 150.0), v).unwrap();
    assert_eq!(paths(&s), 1, "the deferred rectangle is committed");
}

/// Repeated overlay redraws with Real-time Drawing off reuse the cached deferred feedback.
#[test]
fn deferred_preview_overlays_are_cached_across_redraws() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("rectangle", v).unwrap();
    set_pref(&mut s, "realTimeDrawing", json!(false));
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 150.0, 150.0), v).unwrap();
    let rev = s.doc().unwrap().revision;
    let count = |s: &mut Session| s.overlays(v).iter().filter(|o| matches!(o, vectorcraft_tools::Overlay::Path { .. })).count();
    let first = count(&mut s);
    assert!(first > 0);
    assert_eq!(s.doc().unwrap().revision, rev);
    let second = count(&mut s);
    assert_eq!(first, second);
    assert_eq!(s.doc().unwrap().revision, rev, "the preview command is not run again");
}

/// A deferred commit that fails its preview cancels the interaction instead of leaving it open.
#[test]
fn deferred_commit_preview_failure_cancels_interaction() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    set_pref(&mut s, "realTimeDrawing", json!(false));
    s.begin_interaction("Bad").unwrap();
    s.doc_mut().unwrap().interaction.as_mut().unwrap().preview = Some(("no.such.command".into(), json!({})));
    s.apply_actions(vec![vectorcraft_tools::Action::Commit]).unwrap();
    assert!(s.doc().unwrap().interaction.is_none());
}

/// A deferred commit applies preview warnings to the UI, as live previews do.
#[test]
fn deferred_commit_forwards_preview_warnings() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let rect = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    let text = s.execute("text.create", &json!({"x": 300, "y": 200, "text": "Hi"})).unwrap();
    s.execute("select.set", &json!({"ids": [rect["id"], text["id"]]})).unwrap();
    set_pref(&mut s, "realTimeDrawing", json!(false));
    s.begin_interaction("Bloat").unwrap();
    let p = json!({"tool": "bloat", "points": [[290.0, 200.0]], "width": 200.0, "height": 200.0});
    s.doc_mut().unwrap().interaction.as_mut().unwrap().preview = Some(("object.liquify".into(), p));
    let ui = s.apply_actions(vec![vectorcraft_tools::Action::Commit]).unwrap();
    assert!(ui.iter().any(|r| matches!(r, UiRequest::Status(m) if m.contains("type"))), "{ui:?}");
}

/// File Handling › Use Low Resolution Proxy for Linked EPS (#394): off by default; toggling it does
/// not replace full-resolution pixels already in the document.
#[test]
fn low_res_proxy_eps_preference_does_not_swap_full_resolution_pixels() {
    use crate::tests_links::{Folder, RED, place, png, save, write};
    let dir = Folder::new("eps-proxy-pref");
    let pic = dir.file("photo.eps");
    write(&pic, &png(600, 300, RED));
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    assert!(!s.prefs.low_res_proxy_eps);
    let id = place(&mut s, &pic);
    let doc_path = dir.file("doc.vectorcraft");
    save(&mut s, &doc_path);
    s.execute("links.update", &json!({})).unwrap();
    let key = {
        let st = s.doc().unwrap();
        let NodeKind::Image(im) = &st.doc.node(id).unwrap().kind else { panic!("image") };
        im.key.clone()
    };
    assert!(!s.doc().unwrap().doc.images[&key].is_proxy());
    set_pref(&mut s, "lowResProxyEps", json!(true));
    assert!(!s.doc().unwrap().doc.images[&key].is_proxy(), "full pixels stay in the document");
    set_pref(&mut s, "lowResProxyEps", json!(false));
    assert!(!s.prefs.low_res_proxy_eps);
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

/// #1274: View › Snap to Point lands the anchor a selection is grabbed by (a press within a few
/// pixels of it) on another object's anchor, with Smart Guides on or off, not the pressed spot
/// beside it.
#[test]
fn snap_to_point_lands_the_grabbed_anchor_on_another_anchor() {
    use vectorcraft_tools::PointerKind::{Down, Drag, Up};
    let mut s = new_doc();
    let a = square(&mut s, 100.0, 100.0);
    square(&mut s, 300.0, 100.0);
    for v in [ViewInfo { smart_guides: false, ..ViewInfo::default() }, ViewInfo::default()] {
        s.select_tool("selection", v).unwrap();
        // Nothing selected, so no bounding box handle sits on the corner.
        s.execute("select.none", &json!({})).unwrap();
        // Pressed 2 px left of and below the top-right anchor (150, 100), released with the
        // pointer 1.4 px from the other square's top-left anchor (300, 100).
        gesture(&mut s, v, &[(Down, 148.0, 102.0), (Drag, 200.0, 110.0), (Drag, 299.0, 101.0), (Up, 299.0, 101.0)]);
        assert_eq!(top_left(&s, a), (250.0, 100.0), "the grabbed anchor lands on the anchor ({v:?})");
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

/// #1274: with Smart Guides off, View › Snap to Point turns the selection arrow hollow over an
/// anchor a press would grab, and lands a Pen click within 2 px of an anchor on it.
#[test]
fn snap_to_point_cues_anchors_and_lands_pen_clicks_on_them() {
    use vectorcraft_tools::PointerKind::{Down, Up};
    use vectorcraft_tools::{Cursor, Mods};
    let mut s = new_doc();
    square(&mut s, 300.0, 100.0);
    s.execute("select.none", &json!({})).unwrap();
    let on = ViewInfo { smart_guides: false, ..ViewInfo::default() };
    let off = ViewInfo { snap_to_point: false, ..on };
    s.select_tool("selection", on).unwrap();
    let near = vectorcraft_geom::Point::new(302.0, 102.0);
    assert_eq!(s.cursor(near, Mods::default(), on), Cursor::ArrowHollow);
    assert_eq!(s.cursor(vectorcraft_geom::Point::new(325.0, 125.0), Mods::default(), on), Cursor::Arrow, "away from the anchors");
    assert_eq!(s.cursor(near, Mods::default(), off), Cursor::Arrow, "Snap to Point off");
    for (v, first) in [(on, (300.0, 100.0)), (off, (301.0, 101.0))] {
        s.select_tool("pen", v).unwrap();
        gesture(&mut s, v, &[(Down, 301.0, 101.0), (Up, 301.0, 101.0), (Down, 200.0, 250.0), (Up, 200.0, 250.0)]);
        let id = *s.doc().unwrap().selection.objects.first().unwrap();
        let p = s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].anchors[0].p;
        assert_eq!((p.x, p.y), first, "{v:?}");
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("select.none", &json!({})).unwrap();
    }
}

/// A millimetre document with a rectangle (100, 100)–(300, 250) stroked 1 pt and a vertical
/// ruler guide at no whole number of millimetres or points (62.31 mm): the rectangle and the
/// guide's x.
fn stroked_rect_and_guide() -> (Session, NodeId, f64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 600, "units": "Millimeters"})).unwrap();
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap()["id"].as_u64().unwrap());
    s.execute("stroke.set", &json!({"weight": 1})).unwrap();
    let g = 62.31 / 25.4 * 72.0;
    s.execute("guide.add", &json!({"vertical": true, "pos": g})).unwrap();
    (s, id, g)
}

/// #3701: at 3,891%, the Selection tool pressed on a rectangle's 1 pt stroke inside its left edge,
/// or on its fill 3 px inside it, and dragged until the edge is 1 px from a ruler guide lands the
/// edge on the guide exactly, with "align". The pressed spot used to be what snapped, so the edge
/// stopped that far beside the guide however far one zoomed in.
#[test]
fn a_move_held_by_a_paths_stroke_lands_the_path_on_a_ruler_guide() {
    use vectorcraft_tools::Overlay;
    use vectorcraft_tools::PointerKind::{Down, Drag, Up};
    let (mut s, id, g) = stroked_rect_and_guide();
    let v = ViewInfo { zoom: 38.91, ..ViewInfo::default() };
    s.select_tool("selection", v).unwrap();
    // 0.3 pt (12 px) into the stroke, and 3 px into the fill.
    for x in [100.3, 100.0 + 3.0 / v.zoom] {
        s.execute("select.none", &json!({})).unwrap();
        let to = x + g + 1.0 / v.zoom - 100.0;
        gesture(&mut s, v, &[(Down, x, 175.0), (Drag, (x + to) / 2.0, 175.0), (Drag, to, 175.0)]);
        let align = s.overlays(v).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "align"));
        gesture(&mut s, v, &[(Up, to, 175.0)]);
        let edge = top_left(&s, id).0;
        assert!((edge - g).abs() < 1e-9 && align, "pressed at {x}: the edge is {} pt off the guide (align: {align})", edge - g);
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

/// #3701: at 28,757%, Direct Selection pressed 1.5 px beside a rectangle's straight left side and
/// dragged until the side is 1 px from a ruler guide lands both its anchors on the guide; so does
/// the same press and drag on the rectangle selected whole, which moves it.
#[test]
fn a_straight_segment_dragged_by_direct_selection_lands_on_a_ruler_guide() {
    use vectorcraft_tools::PointerKind::{Down, Drag, Up};
    let (mut s, id, g) = stroked_rect_and_guide();
    let v = ViewInfo { zoom: 287.57, ..ViewInfo::default() };
    s.select_tool("directSelection", v).unwrap();
    for whole in [false, true] {
        s.execute("select.set", &json!({"ids": if whole { vec![id.0] } else { vec![] }})).unwrap();
        let x = 100.0 + 1.5 / v.zoom;
        let to = x + g + 1.0 / v.zoom - 100.0;
        gesture(&mut s, v, &[(Down, x, 175.0), (Drag, (x + to) / 2.0, 175.0), (Drag, to, 175.0), (Up, to, 175.0)]);
        let mut xs: Vec<f64> = s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].anchors.iter().map(|a| a.p.x).collect();
        xs.sort_by(f64::total_cmp);
        assert!(
            xs[..2].iter().all(|x| (x - g).abs() < 1e-9),
            "whole: {whole}, the side's anchors are {:?} pt off the guide",
            xs[..2].iter().map(|x| x - g).collect::<Vec<_>>()
        );
        s.execute("edit.undo", &json!({})).unwrap();
    }
}
