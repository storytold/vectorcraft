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

/// An option set with `tool.setOption` while a press is held survives the dropped press, in the
/// tool and in the stored options, while what the drag itself changed is put back in both.
#[test]
fn an_option_set_during_a_dropped_press_is_kept() {
    let mut s = session();
    select(&mut s, "rectangularGrid");
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 180.0), v).unwrap();
    s.tool_key(ToolKey::Up, Mods::default(), v).unwrap();
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!((s.tool_options()["rows"].clone(), s.tool_options()["columns"].clone()), (json!(6), json!(6)));
    s.set_tool_option_cmd(&json!({"key": "columns", "value": 9})).unwrap();
    s.drop_tool_press(v).unwrap();
    let o = s.tool_options();
    assert_eq!((o["rows"].clone(), o["columns"].clone()), (json!(5), json!(9)), "the drag's rows go back; the set columns stay");
    let saved = &s.prefs.tool_settings["rectangularGrid"];
    assert_eq!((saved["rows"].clone(), saved["columns"].clone()), (json!(5), json!(9)), "the drag's rows are not stored; the set columns are");
    select(&mut s, "selection");
    select(&mut s, "rectangularGrid");
    let o = s.tool_options();
    assert_eq!((o["rows"].clone(), o["columns"].clone()), (json!(5), json!(9)), "a tool switch keeps the set columns");
    // The same through the desktop app's setter.
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 180.0), v).unwrap();
    s.tool_key(ToolKey::Up, Mods::default(), v).unwrap();
    s.set_tool_option("columns", &json!(3));
    s.drop_tool_press(v).unwrap();
    let o = s.tool_options();
    assert_eq!((o["rows"].clone(), o["columns"].clone()), (json!(5), json!(3)));
    let saved = &s.prefs.tool_settings["rectangularGrid"];
    assert_eq!((saved["rows"].clone(), saved["columns"].clone()), (json!(5), json!(3)));
}

/// A Corner Radius preference set while a Rounded Rectangle press is held survives the dropped
/// press, in the tool and in the preference.
#[test]
fn a_corner_radius_set_during_a_dropped_press_is_kept() {
    let mut s = session();
    select(&mut s, "roundedRectangle");
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 180.0), v).unwrap();
    let mut p = s.prefs.clone();
    p.corner_radius = 30.0;
    s.apply_prefs(p);
    s.drop_tool_press(v).unwrap();
    assert_eq!(s.tool_options()["cornerRadius"].as_f64(), Some(30.0), "the tool keeps the new radius");
    assert_eq!(s.prefs.corner_radius, 30.0, "the preference keeps it");
}

/// An option set with `tool.setOption` on a tool that lent a selection tool for a Cmd-drag
/// survives the dropped press, in the tool and in the stored options. So does a Corner Radius
/// preference set while the Rounded Rectangle lends one.
#[test]
fn an_option_set_on_a_lending_tool_during_a_dropped_press_is_kept() {
    let mut s = session();
    select(&mut s, "rectangularGrid");
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    let cmd = Mods { cmd: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0).with_mods(cmd), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 180.0).with_mods(cmd), v).unwrap();
    assert_eq!(s.lender.as_ref().map(|t| t.id()), Some("rectangularGrid"), "the press lent a selection tool");
    s.set_tool_option_cmd(&json!({"tool": "rectangularGrid", "key": "columns", "value": 9})).unwrap();
    s.drop_tool_press(v).unwrap();
    assert_eq!(s.tool_id(), "rectangularGrid", "the lending tool is back");
    assert_eq!(s.tool_options()["columns"], json!(9), "the tool keeps the set columns");
    assert_eq!(s.prefs.tool_settings["rectangularGrid"]["columns"], json!(9), "and stores them");
    select(&mut s, "roundedRectangle");
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0).with_mods(cmd), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 180.0).with_mods(cmd), v).unwrap();
    assert_eq!(s.lender.as_ref().map(|t| t.id()), Some("roundedRectangle"), "the press lent a selection tool");
    let mut p = s.prefs.clone();
    p.corner_radius = 30.0;
    s.apply_prefs(p);
    s.drop_tool_press(v).unwrap();
    assert_eq!(s.tool_options()["cornerRadius"].as_f64(), Some(30.0), "the tool keeps the new radius");
    assert_eq!(s.prefs.corner_radius, 30.0, "the preference keeps it");
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

#[test]
fn spiral_decay_is_normalized_for_active_and_inactive_tools_and_preferences() {
    let mut s = session();
    for active in [false, true] {
        select(&mut s, if active { "spiral" } else { "selection" });
        // A decay past the 99.99 % a spiral draws is refused, and nothing changes.
        let before = s.tool_options_of("spiral");
        let refused = s.set_tool_option_cmd(&json!({"tool": "spiral", "values": {"decay": 150, "segments": 4, "clockwise": false}}));
        assert_eq!(refused.err(), Some("tool `spiral`: `decay` must be a number from 5 to 99.99".to_string()));
        assert_eq!(s.tool_options_of("spiral"), before);
        let options = s.set_tool_option_cmd(&json!({"tool": "spiral", "values": {"decay": 99.99, "segments": 4, "clockwise": false}})).unwrap();
        assert_eq!(options["decay"], json!(99.99));
        assert_eq!(s.tool_options_of("spiral")["decay"], json!(99.99));
        assert_eq!(s.prefs.tool_settings["spiral"]["decay"], json!(99.99));
        assert_eq!(s.tool_id(), if active { "spiral" } else { "selection" });
        select(&mut s, "selection");
        select(&mut s, "spiral");
        assert_eq!(s.tool_options()["decay"], json!(99.99));
    }
    let saved = serde_json::to_string(&s.prefs).unwrap();
    let mut restored = session();
    restored.apply_prefs(serde_json::from_str(&saved).unwrap());
    select(&mut restored, "spiral");
    assert_eq!(restored.tool_options(), json!({"decay": 99.99, "segments": 4, "clockwise": false}));
}

#[test]
fn legacy_spiral_decay_preferences_report_the_ratio_drawn_after_reopening() {
    use vectorcraft_geom::Point;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    // Older versions saved 100–150, although their geometry already capped the ratio at 0.9999.
    for clockwise in [false, true] {
        let mut s = session();
        let mut prefs = Prefs::default();
        prefs.tool_settings.insert("spiral".into(), json!({"decay": 150, "segments": 4, "clockwise": clockwise}).as_object().unwrap().clone());
        s.apply_prefs(serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap());
        let view = ViewInfo { smart_guides: false, ..Default::default() };
        s.select_tool("spiral", view).unwrap();
        let decay = s.tool_options()["decay"].as_f64().unwrap();
        assert_eq!(decay, 99.99);
        let before = s.doc().unwrap().history.undo.len();
        for (kind, x) in [(PointerKind::Down, 200.0), (PointerKind::Drag, 300.0), (PointerKind::Up, 300.0)] {
            s.pointer(&PointerEvent::new(kind, x, 200.0), view).unwrap();
        }
        let d = s.doc().unwrap();
        assert_eq!(d.history.undo.len(), before + 1);
        assert_eq!(d.selection.objects.len(), 1);
        let id = d.selection.objects[0];
        let path = d.doc.node(id).unwrap().path_data().unwrap().clone();
        let anchors = &path.subpaths[0].anchors.to_vec();
        assert_eq!(anchors.len(), 5);
        assert!(!path.is_closed());
        let center = Point::new(200.0, 200.0);
        // The path runs from the centre end outwards, and decay is per full wind: four quarter-turn
        // segments give the decay.
        for pair in anchors.windows(2) {
            let ratio = pair[0].p.distance(center) / pair[1].p.distance(center);
            assert!((100.0 * ratio.powi(4) - decay).abs() < 1e-9);
        }
        assert_eq!(s.prefs.tool_settings["spiral"]["decay"], json!(99.99));
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.node(id).is_none());
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap(), &path);
    }
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

/// The Smoothness an Alt smoothing drag sends with `path.smoothRegion` on a selected path.
fn alt_smoothness(s: &mut Session) -> Option<f64> {
    use vectorcraft_tools::Action;
    let mods = Mods { alt: true, ..Default::default() };
    let mut last = vec![];
    for (kind, x, y) in [(PointerKind::Down, 50.0, 150.0), (PointerKind::Drag, 150.0, 150.0), (PointerKind::Up, 250.0, 160.0)] {
        let ev = PointerEvent { mods, ..PointerEvent::new(kind, x, y) };
        last = s.with_tool_cx(ViewInfo::default(), |t, cx| t.pointer(cx, &ev));
    }
    last.iter().find_map(|a| match a {
        Action::Exec(c, v) if c == "path.smoothRegion" => v["smoothness"].as_f64(),
        _ => None,
    })
}

/// Smooth Tool Options' Smoothness is kept with the preferences (50% until it is set), and Alt
/// smoothing with the Pencil, Paintbrush and Blob Brush uses it. A Smoothness outside 0 to 100 is
/// an error that sets nothing.
#[test]
fn the_smooth_tools_smoothness_is_kept_and_alt_smoothing_takes_it() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    select(&mut s, "smooth");
    assert_eq!(s.tool_options()["smoothness"], json!(50.0));
    for bad in [json!(-1), json!(100.5), json!("30"), json!(null)] {
        let e = s.set_tool_option_cmd(&json!({"key": "smoothness", "value": bad})).unwrap_err();
        assert_eq!(e, "tool `smooth`: `smoothness` must be a number from 0 to 100", "{bad}");
    }
    assert_eq!(s.tool_options()["smoothness"], json!(50.0));
    s.set_tool_option("smoothness", &json!(80));
    assert_eq!(s.prefs.tool_settings["smooth"]["smoothness"], json!(80.0));
    for id in ["pencil", "paintbrush", "blobBrush"] {
        select(&mut s, id);
        if id == "pencil" {
            s.set_tool_option("altSmooth", &json!(true));
        }
        assert_eq!(alt_smoothness(&mut s), Some(80.0), "{id}");
        assert!(s.tool_options().get("smoothness").is_none(), "{id} has no Smoothness of its own");
    }
    s.set_tool_option_cmd(&json!({"tool": "smooth", "key": "smoothness", "value": 10})).unwrap();
    assert_eq!(alt_smoothness(&mut s), Some(10.0), "the Blob Brush takes the new Smoothness");
    let saved = serde_json::to_string(&s.prefs).unwrap();
    let mut next = session();
    next.apply_prefs(serde_json::from_str(&saved).unwrap());
    select(&mut next, "smooth");
    assert_eq!(next.tool_options()["smoothness"], json!(10.0));
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
    // No params (null) read too; params that are not an object are an error.
    assert_eq!(s.set_tool_option_cmd(&serde_json::Value::Null).unwrap(), s.tool_options());
    for (bad, got) in [(json!(true), "true"), (json!([]), "[]")] {
        assert_eq!(s.set_tool_option_cmd(&bad).unwrap_err(), format!("the params must be an object, not {got}"));
    }
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
