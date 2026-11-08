//! The Puppet Warp tool in the UI: its Control bar (Expand, Show Mesh, Select All Pins) and hint.

use serde_json::json;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, chrome, tests_labels::painted_text};

#[test]
fn the_control_bar_shows_puppet_warp_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 200, "height": 100})).unwrap();
    app.run("tool.select", json!({"tool": "puppetWarp"})).unwrap();
    let text = painted_text(&mut app, chrome::control_bar);
    for s in ["Expand:", "3 pt", "Show Mesh", "Select All Pins"] {
        assert!(text.lines().any(|l| l == s), "{s} in {text}");
    }
    // The options it sets: Expand may be 0, Show Mesh turns off.
    let opts = app.run("tool.setOption", json!({"key": "expand", "value": 0})).unwrap();
    assert_eq!(opts["expand"], json!(0.0));
    let opts = app.run("tool.setOption", json!({"key": "showMesh", "value": false})).unwrap();
    assert_eq!(opts["showMesh"], json!(false));
    assert!(painted_text(&mut app, chrome::control_bar).lines().any(|l| l == "0 pt"));
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    assert!(!painted_text(&mut app, chrome::control_bar).contains("Show Mesh"), "only with Puppet Warp");
}

#[test]
fn the_hint_bar_explains_puppet_warp() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("tool.select", json!({"tool": "puppetWarp"})).unwrap();
    let text = painted_text(&mut app, chrome::hint_bar).replace('\n', "");
    assert!(text.contains("add a pin") && text.contains("rotate") && text.contains("Delete"), "{text}");
}
