//! Tool options that persist across tool switches, documents and launches (`Prefs::tool_settings`).

use serde_json::json;

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
