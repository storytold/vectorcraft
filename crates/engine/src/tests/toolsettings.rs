//! Tool options that persist across tool switches, documents and launches (`Prefs::tool_settings`).

use serde_json::json;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn select(s: &mut Session, tool: &str) {
    s.select_tool(tool, ViewInfo::default()).unwrap();
}

#[test]
fn blob_tip_options_report_and_persist() {
    let mut s = session();
    select(&mut s, "blobBrush");
    assert_eq!((s.tool_options()["angle"].as_f64(), s.tool_options()["roundness"].as_f64()), (Some(0.0), Some(100.0)));
    s.set_tool_option("angle", &json!(45));
    s.set_tool_option("roundness", &json!(0));
    select(&mut s, "selection");
    let saved = serde_json::to_string(&s.prefs).unwrap();
    let mut next = session();
    next.apply_prefs(serde_json::from_str(&saved).unwrap());
    select(&mut next, "blobBrush");
    assert_eq!((next.tool_options()["angle"].as_f64(), next.tool_options()["roundness"].as_f64()), (Some(45.0), Some(0.0)));
}

#[test]
fn options_survive_a_tool_switch() {
    let mut s = session();
    select(&mut s, "mirrorCut");
    s.set_tool_option("axis", &json!("horizontal"));
    s.set_tool_option("keep", &json!("bottom"));
    select(&mut s, "selection");
    select(&mut s, "mirrorCut");
    assert_eq!(s.tool_options(), json!({"axis": "horizontal", "keep": "bottom"}));
    // Interaction state isn't kept: the Rotate tool's reference point starts afresh.
    select(&mut s, "rotate");
    s.set_tool_option("origin", &json!([10, 20]));
    select(&mut s, "selection");
    select(&mut s, "rotate");
    assert_eq!(s.tool_options(), json!({"origin": null}));
}

#[test]
fn liquify_tools_share_the_global_brush_but_not_their_own_options() {
    let mut s = session();
    select(&mut s, "warp");
    s.set_tool_option("width", &json!(60));
    s.set_tool_option("intensity", &json!(80));
    s.set_tool_option("detail", &json!(7));
    select(&mut s, "twirl");
    let o = s.tool_options();
    assert_eq!((o["width"].as_f64(), o["intensity"].as_f64(), o["detail"].as_f64()), (Some(60.0), Some(0.8), Some(2.0)));
    s.set_tool_option("rate", &json!(-90));
    select(&mut s, "warp");
    assert_eq!(s.tool_options()["detail"], json!(7.0));
    select(&mut s, "twirl");
    assert_eq!(s.tool_options()["rate"], json!(-90.0));
}

#[test]
fn another_tools_options_are_stored_and_shared_ones_reach_the_active_tool() {
    let mut s = session();
    select(&mut s, "warp");
    let o = s.set_tool_option_cmd(&json!({"tool": "pucker", "values": {"width": 40, "height": 30, "detail": 5}})).unwrap();
    assert_eq!((o["tool"].as_str(), o["detail"].as_f64()), (Some("pucker"), Some(5.0)));
    assert_eq!((s.tool_id(), s.tool_options()["width"].as_f64(), s.tool_options()["detail"].as_f64()), ("warp", Some(40.0), Some(2.0)));
    assert_eq!(s.tool_options_of("pucker")["height"], json!(30.0));
    select(&mut s, "pucker");
    assert_eq!(s.tool_options()["detail"], json!(5.0));
    // `{}` reads the options; an unknown tool is an error.
    assert_eq!(s.set_tool_option_cmd(&json!({})).unwrap(), s.tool_options());
    assert!(s.set_tool_option_cmd(&json!({"tool": "nope", "key": "width", "value": 1})).is_err());
}

#[test]
fn a_gesture_keeps_what_it_changed_and_a_document_switch_keeps_options() {
    let mut s = session();
    select(&mut s, "spiral");
    s.set_tool_option("segments", &json!(14));
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    assert_eq!(s.tool_id(), "spiral");
    assert_eq!(s.tool_options()["segments"], json!(14));
    s.set_active(0);
    assert_eq!(s.tool_options()["segments"], json!(14));
}

/// Escape on a grid drag puts back the dividers its arrows changed, in the saved options too,
/// while an option set during the drag stays.
#[test]
fn escape_on_a_drag_saves_the_options_it_put_back() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};
    let mut s = session();
    select(&mut s, "rectangularGrid");
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 180.0), v).unwrap();
    s.tool_key(ToolKey::Up, Mods::default(), v).unwrap();
    // Saves every option, the rows the arrow changed included.
    s.set_tool_option("columns", &json!(8));
    assert_eq!(s.prefs.tool_settings["rectangularGrid"]["rows"], json!(6));
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    let saved = &s.prefs.tool_settings["rectangularGrid"];
    assert_eq!((saved["rows"].clone(), saved["columns"].clone()), (json!(5), json!(8)));
}

/// A dropped press (a long touch, a canceled touch, a second finger) keeps nothing it began: the
/// tool ends at rest with the options it had before the press, and the options the drag changed
/// are not stored. A release stores them.
#[test]
fn a_dropped_press_does_not_keep_what_its_drag_changed() {
    let mut s = session();
    select(&mut s, "polygon");
    let v = ViewInfo::default();
    let drag_with_one_more_side = |s: &mut Session| {
        s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 150.0), v).unwrap();
        s.tool_key(ToolKey::Up, Mods::default(), v).unwrap();
    };
    assert_eq!(s.tool_options()["sides"], json!(6));
    drag_with_one_more_side(&mut s);
    assert_eq!(s.tool_options()["sides"], json!(7));
    s.drop_tool_press(v).unwrap();
    assert!(!s.tool_busy(), "the tool is at rest");
    let st = s.active().unwrap();
    assert!(st.interaction.is_none() && st.doc.art_bounds().is_none(), "the polygon is not drawn");
    assert_eq!(s.tool_options()["sides"], json!(6), "the tool has its sides from before the press");
    assert_eq!(s.prefs.tool_settings.get("polygon").and_then(|o| o.get("sides")), None, "the dropped drag's sides are not stored");
    drag_with_one_more_side(&mut s);
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 150.0), v).unwrap();
    assert!(s.active().unwrap().doc.art_bounds().is_some(), "a release draws the polygon");
    assert_eq!(s.prefs.tool_settings["polygon"]["sides"], json!(7), "and stores its sides");
}

/// A dropped Pen press drops only the anchor it was placing: the path being drawn goes on with
/// the next click.
#[test]
fn a_dropped_pen_press_keeps_the_path_being_drawn() {
    let mut s = session();
    select(&mut s, "pen");
    let v = ViewInfo::default();
    let click = |s: &mut Session, x: f64, y: f64| {
        s.pointer(&PointerEvent::new(PointerKind::Down, x, y), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, x, y), v).unwrap();
    };
    click(&mut s, 10.0, 10.0);
    click(&mut s, 100.0, 10.0);
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 140.0, 120.0), v).unwrap();
    s.drop_tool_press(v).unwrap();
    click(&mut s, 10.0, 100.0);
    let d = &s.active().unwrap().doc;
    let paths = d.layers[0].children().unwrap();
    assert_eq!(paths.len(), 1, "no new path starts");
    assert_eq!(paths[0].path_data().unwrap().anchor_count(), 3, "the dropped anchor is not placed");
}

/// A dropped Pen press on the first anchor does not close the path or end it.
#[test]
fn a_dropped_pen_close_keeps_the_path_open_and_being_drawn() {
    let mut s = session();
    select(&mut s, "pen");
    let v = ViewInfo::default();
    let click = |s: &mut Session, x: f64, y: f64| {
        s.pointer(&PointerEvent::new(PointerKind::Down, x, y), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, x, y), v).unwrap();
    };
    click(&mut s, 10.0, 10.0);
    click(&mut s, 100.0, 10.0);
    click(&mut s, 100.0, 100.0);
    s.pointer(&PointerEvent::new(PointerKind::Down, 10.0, 10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 40.0, 30.0), v).unwrap();
    s.drop_tool_press(v).unwrap();
    click(&mut s, 10.0, 100.0);
    let d = &s.active().unwrap().doc;
    let paths = d.layers[0].children().unwrap();
    assert_eq!(paths.len(), 1, "no new path starts");
    let pd = paths[0].path_data().unwrap();
    assert_eq!(pd.anchor_count(), 4, "the next click goes on the path");
    assert!(!pd.subpaths[0].closed, "the path is not closed");
}

/// A dropped marquee selects nothing: the selection stays as it was before the press.
#[test]
fn a_dropped_marquee_leaves_the_selection_alone() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 600, "y": 400, "width": 100, "height": 100})).unwrap();
    let before = s.doc().unwrap().selection.objects.clone();
    assert_eq!(before.len(), 1);
    select(&mut s, "selection");
    let v = ViewInfo::default();
    s.pointer(&PointerEvent::new(PointerKind::Down, 300.0, 300.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 350.0, 350.0), v).unwrap();
    s.drop_tool_press(v).unwrap();
    assert!(!s.tool_busy(), "the tool is at rest");
    assert_eq!(s.doc().unwrap().selection.objects, before);
}

/// A dropped Shift-drag marquee of the Artboard tool toggles no artboard: the tool takes Escape,
/// as during a drag, before its release.
#[test]
fn a_dropped_artboard_marquee_toggles_nothing() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 400, "height": 400})).unwrap();
    select(&mut s, "artboard");
    let before = s.tool_options()["selected"].clone();
    let v = ViewInfo::default();
    let shift = Mods { shift: true, ..Mods::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, 1200.0, 500.0).with_mods(shift), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 1250.0, 550.0).with_mods(shift), v).unwrap();
    s.drop_tool_press(v).unwrap();
    assert!(!s.tool_busy(), "the tool is at rest");
    assert_eq!(s.tool_options()["selected"], before, "no artboard is toggled");
}

#[test]
fn options_are_saved_with_the_preferences() {
    let mut s = session();
    select(&mut s, "polygon");
    s.set_tool_option("sides", &json!(9));
    select(&mut s, "selection");
    let saved = serde_json::to_string(&s.prefs).unwrap();
    assert!(saved.contains("toolSettings"), "{saved}");
    // Next launch.
    let mut t = session();
    t.apply_prefs(serde_json::from_str(&saved).unwrap());
    select(&mut t, "polygon");
    assert_eq!(t.tool_options()["sides"], json!(9));
    // Older preference files have none; none are written until a tool keeps something.
    assert!(Prefs::default().to_json().get("toolSettings").is_none());
    let old: Prefs = serde_json::from_value(json!({"keyboardIncrement": 2})).unwrap();
    assert!(old.tool_settings.is_empty());
    // Resetting the preferences keeps them.
    t.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(t.prefs.tool_settings["polygon"]["sides"], json!(9));
}

/// Escape on a polygon or rounded rectangle drag puts back the side count or corner radius its
/// arrows changed, in the saved options and the Corner Radius preference too.
#[test]
fn escape_on_a_shape_drag_saves_the_option_it_put_back() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};
    let mut s = session();
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    let drag = |s: &mut Session| {
        s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 180.0), v).unwrap();
        s.tool_key(ToolKey::Up, Mods::default(), v).unwrap();
        // Saves every option, the one the arrow changed included.
        s.set_tool_option("length", &json!(50));
    };
    select(&mut s, "polygon");
    drag(&mut s);
    assert_eq!(s.prefs.tool_settings["polygon"]["sides"], json!(7));
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert_eq!(s.prefs.tool_settings["polygon"]["sides"], json!(6));
    select(&mut s, "roundedRectangle");
    let radius = s.prefs.corner_radius;
    drag(&mut s);
    assert_eq!(s.prefs.corner_radius, radius + 1.0);
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert_eq!(s.prefs.corner_radius, radius);
}

/// Setting another tool's option during a polygon drag leaves the drag's options alone: Escape
/// still puts back the side count the arrow changed.
#[test]
fn escape_puts_back_the_sides_after_another_tool_s_option_is_set() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};
    let mut s = session();
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    select(&mut s, "polygon");
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 180.0), v).unwrap();
    s.tool_key(ToolKey::Up, Mods::default(), v).unwrap();
    // Saves every option, the one the arrow changed included.
    s.set_tool_option("length", &json!(50));
    let values = json!({"points": 9});
    s.set_tool_options(Some("star"), values.as_object().unwrap());
    assert_eq!(s.tool_options()["sides"], json!(7), "the drag keeps its side count");
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert_eq!(s.tool_options()["sides"], json!(6));
    assert_eq!(s.prefs.tool_settings["polygon"]["sides"], json!(6));
    assert_eq!(s.prefs.tool_settings["star"]["points"], json!(9));
}

/// The Fidelity an Alt smoothing drag uses on a selected path.
fn alt_smooth_fidelity(s: &mut Session) -> Option<f64> {
    use vectorcraft_tools::{Action, Mods, PointerEvent, PointerKind};
    let mods = Mods { alt: true, ..Default::default() };
    let mut last = vec![];
    for (kind, x, y) in [(PointerKind::Down, 50.0, 150.0), (PointerKind::Drag, 150.0, 150.0), (PointerKind::Up, 250.0, 160.0)] {
        let ev = PointerEvent { mods, ..PointerEvent::new(kind, x, y) };
        last = s.with_tool_cx(ViewInfo::default(), |t, cx| t.pointer(cx, &ev));
    }
    last.iter().find_map(|a| match a {
        Action::Exec(c, v) if c == "path.smoothRegion" => v["fidelity"].as_f64(),
        _ => None,
    })
}

/// Alt switches Pencil, Paintbrush and Blob Brush to Smooth, using Smooth Tool Options' stored Fidelity.
#[test]
fn alt_smoothing_takes_the_smooth_tools_fidelity() {
    for id in ["pencil", "paintbrush", "blobBrush"] {
        let mut s = session();
        s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        select(&mut s, "smooth");
        s.set_tool_option("fidelity", &json!(6));
        select(&mut s, id);
        if id == "pencil" {
            s.set_tool_option("altSmooth", &json!(true));
        }
        s.set_tool_option("fidelity", &json!(3));
        assert_eq!(alt_smooth_fidelity(&mut s), Some(6.0), "{id}");
        s.set_tool_option_cmd(&json!({"tool": "smooth", "key": "fidelity", "value": 9})).unwrap();
        assert_eq!(alt_smooth_fidelity(&mut s), Some(9.0), "{id}");
        assert_eq!(s.tool_options()["fidelity"], json!(3.0), "{id}");
    }
}

/// `tool.setOption` with a value of the wrong type, a number out of range, an unknown value or an
/// option the tool doesn't have is an error naming the tool and the option. Nothing is set, also
/// when another option given with it is good, and nothing is saved or recorded for undo.
#[test]
fn bad_option_values_are_errors_that_set_nothing() {
    let mut s = session();
    select(&mut s, "smooth");
    let (options, saved, undo) = (s.tool_options(), s.prefs.tool_settings.clone(), s.doc().unwrap().history.undo.len());
    let fidelity = "tool `smooth`: `fidelity` must be a number from 0.05 to 100";
    for bad in [json!("abc"), json!(true), json!(-3), json!(1e9), json!(null), json!([2])] {
        assert_eq!(s.set_tool_option_cmd(&json!({"key": "fidelity", "value": bad})).unwrap_err(), fidelity, "{bad}");
    }
    let cases = [
        (json!({"key": "bogus", "value": 1}), "tool `smooth`: unknown option `bogus`"),
        (json!({"tool": "pencil", "values": {"closeWithin": 5, "fidelity": "x"}}), "tool `pencil`: `fidelity` must be a number from 0.01 to 100"),
        (json!({"tool": "pencil", "key": "fill", "value": "yes"}), "tool `pencil`: `fill` must be true or false"),
        (json!({"tool": "blobBrush", "key": "sizeMode", "value": "huge"}), "tool `blobBrush`: `sizeMode` must be one of `fixed`, `pressure`"),
        (json!({"tool": "eraser", "key": "fidelity", "value": 4}), "tool `eraser`: unknown option `fidelity`"),
        (json!({"tool": "polygon", "key": "sides", "value": 2}), "tool `polygon`: `sides` must be a whole number from 3 to 1000"),
        (
            json!({"tool": "mirrorCut", "key": "axis", "value": "diagonal"}),
            "tool `mirrorCut`: `axis` must be one of `vertical`, `horizontal`, `free`",
        ),
        (json!({"tool": "selection", "key": "fidelity", "value": 4}), "tool `selection`: unknown option `fidelity`"),
        (json!({"values": [1]}), "`values` must be an object"),
        (json!({"tool": 3, "key": "fidelity", "value": 4}), "`tool` must be a tool id"),
        (json!({"key": "fidelity"}), "`value` is missing for `fidelity`"),
    ];
    for (p, want) in cases {
        assert_eq!(s.set_tool_option_cmd(&p).unwrap_err(), want, "{p}");
    }
    assert_eq!(s.tool_options_of("pencil")["closeWithin"], session().tool_options_of("pencil")["closeWithin"]);
    assert_eq!((s.tool_options(), &s.prefs.tool_settings, s.doc().unwrap().history.undo.len()), (options, &saved, undo));
    // A good value after the errors is set and read back.
    assert_eq!(s.set_tool_option_cmd(&json!({"key": "fidelity", "value": 9})).unwrap()["fidelity"], json!(9.0));
}

/// `tool.setOption {}` reads the options, of the active tool or of the tool named, and saves nothing.
#[test]
fn an_empty_set_option_reads_and_saves_nothing() {
    let mut s = session();
    select(&mut s, "smooth");
    let saved = s.prefs.tool_settings.clone();
    assert_eq!(s.set_tool_option_cmd(&json!({})).unwrap(), s.tool_options());
    assert_eq!(s.set_tool_option_cmd(&json!({"tool": "flare"})).unwrap(), s.tool_options_of("flare"));
    assert_eq!(s.prefs.tool_settings, saved);
}

/// A Place queue with a malformed entry is an error, and the queue held stays as it was.
#[test]
fn a_malformed_place_queue_is_an_error() {
    let mut s = session();
    select(&mut s, "place");
    let entry = json!({"params": {"svg": "<svg/>"}, "name": "a", "width": 10, "height": 10});
    s.set_tool_option_cmd(&json!({"key": "queue", "value": {"entries": [entry]}})).unwrap();
    let bad = json!({"params": {}, "name": "b", "width": "wide", "height": 10});
    let e = s.set_tool_option_cmd(&json!({"key": "queue", "value": {"entries": [entry, bad]}})).unwrap_err();
    assert!(e.starts_with("tool `place`: `queue` must be"), "{e}");
    assert_eq!(s.tool_options()["names"], json!(["a"]));
    assert_eq!(
        s.set_tool_option_cmd(&json!({"key": "current", "value": 1})).unwrap_err(),
        "tool `place`: `current` must be the index of a queued file, from 0 to 0"
    );
}

/// A `current` given with a new Place queue picks among the files of that queue.
#[test]
fn a_place_queue_and_its_current_file_set_together() {
    let mut s = session();
    select(&mut s, "place");
    let entry = |name: &str| json!({"params": {"svg": "<svg/>"}, "name": name, "width": 10, "height": 10});
    let queue = json!({"entries": [entry("a"), entry("b"), entry("c")]});
    let out = s.set_tool_option_cmd(&json!({"values": {"queue": queue, "current": 2}})).unwrap();
    assert_eq!((out["names"].clone(), out["current"].clone()), (json!(["a", "b", "c"]), json!(2)));
    let short = json!({"entries": [entry("d")]});
    let e = s.set_tool_option_cmd(&json!({"values": {"queue": short, "current": 2}})).unwrap_err();
    assert_eq!(e, "tool `place`: `current` must be the index of a queued file, from 0 to 0");
    assert_eq!(s.tool_options()["names"], json!(["a", "b", "c"]), "a refused call keeps the queue held");
}

/// A value given with a parametric shape's new `kind` is checked against that kind's range and
/// set on it, whether the tool is chosen or not.
#[test]
fn a_parametric_shape_kind_and_its_values_set_together() {
    for active in [false, true] {
        let mut s = session();
        if active {
            select(&mut s, "parametricShape");
        }
        let set = |s: &mut Session, values: serde_json::Value| s.set_tool_option_cmd(&json!({"tool": "parametricShape", "values": values}));
        set(&mut s, json!({"kind": "polygon"})).unwrap();
        let out = set(&mut s, json!({"kind": "star", "a": 0.8})).unwrap();
        assert_eq!((out["kind"].clone(), out["a"].clone()), (json!("star"), json!(0.8)), "active {active}");
        set(&mut s, json!({"kind": "ring"})).unwrap();
        let out = set(&mut s, json!({"kind": "star", "a": 1.0})).unwrap();
        assert_eq!((out["kind"].clone(), out["a"].clone()), (json!("star"), json!(1.0)), "active {active}");
        let e = set(&mut s, json!({"kind": "ring", "a": 1.0})).unwrap_err();
        assert!(e.contains("`a`"), "{e}");
        assert_eq!(s.tool_options_of("parametricShape")["kind"], json!("star"), "a refused call sets nothing");
    }
}
