//! The app's locate rules: where a tool is found, what the app shows for it, which rail wears the
//! mark, and when the locate ends. The mark's drawing and the reveal are the rails' own.

use craft::ui::runtime::keys::{self, Intent};
use serde_json::json;
use vectorcraft_engine::Session;

use super::locate::{Home, LIFETIME, Target, take_reveal, tidy, wanted};
use crate::VectorcraftApp;
use crate::toolbar::{self, Owner};

fn app() -> VectorcraftApp {
    VectorcraftApp::new(Session::new(), Default::default())
}

fn ctx() -> egui::Context {
    crate::testing::ui_context()
}

fn locate(app: &mut VectorcraftApp, tool: &str) -> serde_json::Value {
    app.run("learn.locateTool", json!({"tool": tool})).unwrap()
}

/// One pass at `time`, then `tidy`.
fn pass(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) {
    ctx.run_ui(egui::RawInput { time: Some(time), events, ..Default::default() }, |ui| tidy(app, ui.ctx())).textures_delta.clear();
}

#[test]
fn a_learn_link_finds_reveals_and_marks_the_tool() {
    // (1) A tool on a Tools button.
    let mut a = app();
    let c = ctx();
    assert_eq!(locate(&mut a, "pen"), json!({"tool": "pen", "location": "toolbar", "toolbar": null}));
    let home = Home::Rail(&Owner::Tools);
    assert_eq!(wanted(&a.ui, &home).as_deref(), Some("pen"));
    assert_eq!(take_reveal(&mut a.ui, &home, |_| true).as_deref(), Some("pen"));
    assert_eq!(take_reveal(&mut a.ui, &home, |_| true), None, "the reveal is taken once");
    assert_eq!(wanted(&a.ui, &home).as_deref(), Some("pen"), "the mark stays");
    assert_eq!(wanted(&a.ui, &Home::Drawer), None);
    assert_eq!(a.session.tool_id(), "selection", "no tool chosen");
    pass(&mut a, &c, 1.0, vec![]);
    pass(&mut a, &c, 1.0 + LIFETIME - 0.1, vec![]);
    assert!(a.ui.learn_locate.is_some(), "still marked");
    pass(&mut a, &c, 1.0 + LIFETIME + 0.01, vec![]);
    assert!(a.ui.learn_locate.is_none(), "the mark has gone");

    // (2) A tool of a collapsed group leaves the shown tool as it was.
    let mut a = app();
    a.ui.slot_tool.insert("pencil".into(), "pathEraser".into());
    for tool in ["smooth", "pencil"] {
        assert_eq!(locate(&mut a, tool)["location"], "flyout", "{tool}");
    }
    assert_eq!(a.ui.slot_tool.get("pencil").map(String::as_str), Some("pathEraser"));

    // (3) A group torn off into a strip.
    let mut a = app();
    a.run("window.floatTools", json!({"tool": "pen"})).unwrap();
    assert_eq!(locate(&mut a, "addAnchor")["location"], "strip");
    let strip = Owner::Strip(Box::new(Owner::Tools), "pen".into());
    assert!(crate::panel_docking::showing(&a.ui, &toolbar::strip_id(&Owner::Tools, "pen")));
    assert_eq!(wanted(&a.ui, &Home::Rail(&strip)).as_deref(), Some("addAnchor"));
    assert_eq!(take_reveal(&mut a.ui, &Home::Rail(&Owner::Tools), |_| false), None, "a rail without the tool leaves the request");
    assert_eq!(take_reveal(&mut a.ui, &Home::Rail(&strip), |_| true).as_deref(), Some("addAnchor"));

    // (4) A tool no toolbar holds: the drawer, until it closes.
    let mut a = app();
    a.run("toolbar.removeTool", json!({"tool": "rectangle"})).unwrap();
    let before = toolbar::slots(&a);
    assert_eq!(locate(&mut a, "rectangle")["location"], "allTools");
    assert!(a.ui.tools_drawer);
    assert_eq!(wanted(&a.ui, &Home::Drawer).as_deref(), Some("rectangle"));
    assert_eq!(toolbar::slots(&a), before, "no tool added to the toolbar");
    pass(&mut a, &c, 1.0, vec![]);
    assert!(a.ui.learn_locate.is_some());
    a.ui.tools_drawer = false;
    pass(&mut a, &c, 1.1, vec![]);
    assert!(a.ui.learn_locate.is_none(), "the locate ends once the drawer is closed");

    // (5) A toolbar made with New Toolbar.
    let mut a = app();
    a.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    a.run("toolbar.addTool", json!({"tool": "artboard", "toolbar": "Mine"})).unwrap();
    assert_eq!(locate(&mut a, "artboard"), json!({"tool": "artboard", "location": "toolbar", "toolbar": "Mine"}));
    let id = crate::custom_toolbars::find(&a.ui, "Mine").unwrap().id;
    assert_eq!(wanted(&a.ui, &Home::Rail(&Owner::Toolbar(id))).as_deref(), Some("artboard"));
    assert_eq!(wanted(&a.ui, &Home::Rail(&Owner::Tools)), None);
    a.run("toolbar.addTool", json!({"tool": "slice", "toolbar": "Mine", "slot": 0, "place": "group"})).unwrap();
    assert_eq!(locate(&mut a, "slice"), json!({"tool": "slice", "location": "flyout", "toolbar": "Mine"}));

    // (6) Tools hidden shows again.
    let mut a = app();
    toolbar::set_shown(&mut a, false);
    assert_eq!(locate(&mut a, "gradient")["location"], "toolbar");
    assert!(toolbar::shown(&a.ui));

    // (7) A toolbar made with New Toolbar under another window shows its panel.
    let mut a = app();
    a.run("window.toolbar.new", json!({"name": "Mine"})).unwrap();
    a.run("toolbar.addTool", json!({"tool": "artboard", "toolbar": "Mine"})).unwrap();
    locate(&mut a, "artboard");
    let id = crate::custom_toolbars::find(&a.ui, "Mine").unwrap().id;
    assert!(crate::panel_docking::showing(&a.ui, &crate::custom_toolbars::panel_id(id)));
    assert_eq!(a.ui.learn_locate.as_ref().map(|l| l.target.clone()), Some(Target::Custom("Mine".into())));
}

#[test]
fn a_second_learn_link_replaces_the_mark_and_escape_removes_it() {
    let mut a = app();
    let c = ctx();
    locate(&mut a, "pen");
    locate(&mut a, "curvature");
    let home = Home::Rail(&Owner::Tools);
    assert_eq!(wanted(&a.ui, &home).as_deref(), Some("curvature"), "only the second tool is wanted");
    pass(&mut a, &c, 0.5, vec![]);
    assert!(a.ui.learn_locate.is_some());
    let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    pass(&mut a, &c, 1.0, vec![escape]);
    assert!(a.ui.learn_locate.is_none(), "an Escape no component took ends the locate");
    let _ = (keys::with_window_intents, Intent::Cancel);
}
