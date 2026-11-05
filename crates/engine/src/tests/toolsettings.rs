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
