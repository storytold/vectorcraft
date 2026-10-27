//! The cutting tools in the UI: toolbar entries, icons and Mirror & Cut's Control bar options.

use serde_json::json;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, chrome, icons, tests_labels::painted_text};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app
}

#[test]
fn the_cut_tools_have_their_own_icons_and_toolbar_slot() {
    for (tool, icon) in [("mirrorCut", "dc-mirror-cut"), ("lineCut", "dc-line-cut"), ("rectCut", "dc-rect-cut")] {
        let info = vectorcraft_tools::tool_info(tool).unwrap();
        assert_eq!(icons::tool_icon(info.icon), icon);
        assert!(icons::exists(icon), "{icon}");
        assert!(crate::toolbar::BASIC.iter().flat_map(|(_, slots)| slots.iter()).any(|s| s.contains(&tool) && s.contains(&"knife")));
    }
}

#[test]
fn the_control_bar_shows_mirror_and_cut_options() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "lineCut"})).unwrap();
    assert!(!painted_text(&mut app, chrome::control_bar).contains("Axis:"), "only Mirror & Cut has options");
    app.run("tool.select", json!({"tool": "mirrorCut"})).unwrap();
    let text = painted_text(&mut app, chrome::control_bar);
    for s in ["Axis:", "Free", "Keep:", "Left"] {
        assert!(text.lines().any(|l| l == s), "{s} in {text}");
    }
    app.run("tool.setOption", json!({"key": "axis", "value": "horizontal"})).unwrap();
    app.run("tool.setOption", json!({"key": "keep", "value": "bottom"})).unwrap();
    let text = painted_text(&mut app, chrome::control_bar);
    assert!(text.lines().any(|l| l == "Horizontal") && text.lines().any(|l| l == "Bottom"), "{text}");
}
