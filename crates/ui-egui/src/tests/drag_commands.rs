//! A command sent through the control channel (`engine.execute`, which MCP's `run_command` sends)
//! or run as a menu item while a tool's drag is in progress is a step of its own, before the drag's.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

/// Send a control request → its reply.
fn control(app: &mut VectorcraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (request, _) = craft::automation::ControlRequest::new(method, params);
    let craft::automation::Reply::Done(reply) = crate::control::handle(app, ctx, &request) else { panic!("{method}: not done") };
    reply
}

#[test]
fn a_command_from_the_control_channel_or_a_menu_during_a_drag_is_a_step_of_its_own() {
    for through_control in [true, false] {
        let at = format!("control channel {through_control}");
        let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let other = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("select.none", json!({})).unwrap();
        app.select_tool("rectangle");
        let ctx = egui::Context::default();
        let objects = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().map_or(0, Vec::len);
        let hidden = |app: &VectorcraftApp| !app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(other)).unwrap().visible;
        let steps = app.session.doc().unwrap().history.undo.len();
        let down = json!({"events": [{"kind": "down", "x": 100, "y": 100}, {"kind": "drag", "x": 150, "y": 150}]});
        assert_eq!(control(&mut app, &ctx, "ui.pointer", down)["ok"], true, "{at}");
        if through_control {
            let r = control(&mut app, &ctx, "engine.execute", json!({"command": "object.hide", "params": {"ids": [other]}}));
            assert_eq!(r["ok"], true, "{at}: {r}");
        } else {
            app.run("object.hide", json!({"ids": [other]})).unwrap();
        }
        let up = json!({"events": [{"kind": "drag", "x": 200, "y": 180}, {"kind": "up", "x": 200, "y": 180}]});
        assert_eq!(control(&mut app, &ctx, "ui.pointer", up)["ok"], true, "{at}");
        assert!(hidden(&app), "{at}: the drag keeps the command's edit");
        assert_eq!(objects(&app), 2, "{at}: the drag draws its rectangle");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 2, "{at}: a step each");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(objects(&app), 1, "{at}: Undo takes back the drag first");
        assert!(hidden(&app), "{at}");
        app.run("edit.undo", json!({})).unwrap();
        assert!(!hidden(&app), "{at}: then the command");
    }
}
