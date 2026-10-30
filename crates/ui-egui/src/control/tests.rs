use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::{ControlRequest, MAX_DIALOG_FIELDS, Outcome, handle};
use crate::state::Dialog;
use crate::{Services, VectorcraftApp};

fn input_request(app: &mut VectorcraftApp, method: &str, params: Value) -> Value {
    let (req, _) = ControlRequest::new(method, params);
    let Outcome::Done(r) = handle(app, &egui::Context::default(), &req) else { panic!("expected immediate reply") };
    r
}

#[test]
fn deferred_pointer_reply_uses_the_gesture_document_after_file_new() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let (reply, received) = std::sync::mpsc::channel();
    let at = app.session.input_count();
    app.pointer_replies.push(super::PendingPointerReply { from: at, to: at, document: app.session.active().map(|d| d.uid), reply });

    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("tool.select", json!({"tool": "rectangle"})).unwrap();
    app.answer_pointer_replies(&egui::Context::default());

    let result = received.try_recv().unwrap();
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["result"]["selection"], json!([id]));
    assert_eq!(result["result"]["key"], Value::Null);
    assert_eq!(result["result"]["tool"], "rectangle");
}

#[test]
fn ui_set_turns_the_image_trace_preview_off_and_on() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    assert!(app.ui.image_trace_preview);
    for on in [false, true] {
        let r = input_request(&mut app, "ui.set", json!({"imageTracePreview": on}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.ui.image_trace_preview, on);
    }
}

#[test]
fn the_image_trace_preview_is_a_command() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    assert!(crate::menus::UI_COMMANDS.iter().any(|c| c.0 == "imageTrace.preview"));
    assert_eq!(app.run("imageTrace.preview", json!({})), Ok(json!(false)), "toggles");
    assert!(!app.ui.image_trace_preview);
    assert_eq!(app.run("imageTrace.preview", json!({"on": false})), Ok(json!(false)), "stays off");
    assert_eq!(app.run("imageTrace.preview", json!({"on": true})), Ok(json!(true)));
    assert!(app.ui.image_trace_preview);
    assert!(app.run("imageTrace.preview", json!({"on": "yes"})).is_err());
    assert!(app.ui.image_trace_preview, "a bad value leaves it");
    let r = input_request(&mut app, "engine.execute", json!({"command": "imageTrace.preview", "params": {"on": false}}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(!app.ui.image_trace_preview, "the control channel reaches it");
}

#[test]
fn control_input_rejects_excessive_counts_before_mutation() {
    // Only modest over-limit values on the baseline: never exercise an unbounded loop.
    for (method, params) in [("ui.click", json!({"count": 17})), ("ui.drag", json!({"steps": 257})), ("ui.drag", json!({"steps": 2, "count": 17}))] {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.synthetic.push(egui::Event::Text("already queued".into()));
        let before = format!("{:?}", app.synthetic);
        let r = input_request(&mut app, method, params);
        assert_eq!(r["ok"], false, "oversized input must be rejected: {r}");
        assert_eq!(format!("{:?}", app.synthetic), before, "rejection must preserve the entire queue");
    }
}

#[test]
fn control_input_respects_existing_queue_budget() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.synthetic = vec![egui::Event::PointerMoved(egui::pos2(1.0, 2.0)); 1023];
    let before = format!("{:?}", app.synthetic);
    let r = input_request(&mut app, "ui.click", json!({}));
    assert_eq!(r["ok"], false, "a whole click must fit before any part is appended");
    assert_eq!(format!("{:?}", app.synthetic), before);
}

#[test]
fn control_input_yields_control_channel_after_frame_budget() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Services::default()).with_control(rx);
    let mut replies = vec![];
    for _ in 0..65 {
        let (req, reply) = ControlRequest::new("ui.move", json!({"x": 1, "y": 2}));
        tx.send(req).unwrap();
        replies.push(reply);
    }
    app.drain_control(&egui::Context::default());
    assert_eq!(replies.iter().filter(|r| r.try_recv().is_ok()).count(), 64);
    assert_eq!(app.synthetic.len(), 64);
    app.drain_control(&egui::Context::default());
    assert_eq!(replies.last().unwrap().try_recv().unwrap()["ok"], true);
    assert_eq!(app.synthetic.len(), 65);
}

#[test]
fn control_input_rejects_invalid_and_extreme_counts() {
    for value in [json!(u64::MAX), json!(-1), json!(1.5), json!("2"), Value::Null] {
        for (method, key) in [("ui.click", "count"), ("ui.drag", "count"), ("ui.drag", "steps")] {
            let mut app = VectorcraftApp::new(Session::new(), Services::default());
            app.synthetic.push(egui::Event::Text("pending".into()));
            let before = format!("{:?}", app.synthetic);
            let mut params = json!({});
            params[key] = value.clone();
            let r = input_request(&mut app, method, params);
            assert_eq!(r["ok"], false, "{method}/{key}: {r}");
            assert!(r["error"].as_str().unwrap().contains(key));
            assert_eq!(format!("{:?}", app.synthetic), before);
        }
    }
}

#[test]
fn control_input_accepts_boundary_gestures_and_preserves_event_order() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let r = input_request(
        &mut app,
        "ui.drag",
        json!({"x": 2, "y": 3, "toX": 20, "toY": 30,
        "steps": 256, "count": 16, "button": "right", "shift": true}),
    );
    assert_eq!(r["ok"], true);
    assert_eq!(app.synthetic.len(), 289);
    assert!(matches!(app.synthetic.first(), Some(egui::Event::PointerMoved(p)) if *p == egui::pos2(2.0, 3.0)));
    assert!(
        matches!(app.synthetic.get(1), Some(egui::Event::PointerButton { pressed: true, button: egui::PointerButton::Secondary, modifiers, .. }) if modifiers.shift)
    );
    assert!(matches!(app.synthetic.get(257), Some(egui::Event::PointerMoved(p)) if *p == egui::pos2(20.0, 30.0)));
    assert!(matches!(app.synthetic.get(258), Some(egui::Event::PointerButton { pressed: false, .. })));
    assert!(matches!(app.synthetic.last(), Some(egui::Event::PointerButton { pressed: false, pos, .. }) if *pos == egui::pos2(20.0, 30.0)));
    for (method, params, events) in [
        ("ui.click", json!({}), 3),
        ("ui.click", json!({"count": 2}), 5),
        ("ui.drag", json!({}), 11),
        ("ui.drag", json!({"steps": 0, "count": 0}), 4),
    ] {
        app.synthetic.clear();
        assert_eq!(input_request(&mut app, method, params)["ok"], true);
        assert_eq!(app.synthetic.len(), events);
    }
    app.synthetic = vec![egui::Event::PointerMoved(egui::pos2(0.0, 0.0)); 1021];
    assert_eq!(input_request(&mut app, "ui.click", json!({}))["ok"], true);
    assert_eq!(app.synthetic.len(), 1024, "the exact queue boundary is permitted");
}

#[test]
fn control_input_budget_covers_every_synthetic_method() {
    for (method, params) in [
        ("ui.key", json!({"key": "Enter", "text": "x"})),
        ("ui.move", json!({})),
        ("ui.wheel", json!({"dy": 1})),
        ("ui.text", json!({"text": "hello"})),
        ("ui.click", json!({})),
        ("ui.drag", json!({"steps": 1})),
    ] {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.synthetic = vec![egui::Event::PointerMoved(egui::pos2(1.0, 2.0)); 1024];
        let before = format!("{:?}", app.synthetic);
        let r = input_request(&mut app, method, params);
        assert_eq!(r["ok"], false, "{method}: {r}");
        assert_eq!(format!("{:?}", app.synthetic), before);
        app.synthetic.clear();
        assert_eq!(input_request(&mut app, method, json!({"key": "Enter", "steps": 1}))["ok"], true, "queue space becomes usable after draining");
    }
}

/// `ui.dialog.set {fields}` stores the whole list in order, or nothing when an item is refused.
#[test]
fn dialog_set_stores_a_field_list_whole_or_not_at_all() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.ui.dialog = Some(crate::state::Dialog::new("move", json!({"dx": "0 pt", "dy": "0 pt"})));
    let fields = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| serde_json::to_value(&d.fields).unwrap());
    let before = fields(&app);
    for params in [
        json!({"fields": [{"field": "copy", "value": true}, {"value": 1}]}),
        json!({"fields": [{"field": "copy", "value": true}, 7]}),
        json!({"fields": {"field": "copy"}}),
        json!({"field": "dx", "fields": [{"field": "copy", "value": true}]}),
        json!({"fields": [{"field": "copy", "value": true}, {"field": "note"}]}),
    ] {
        let r = input_request(&mut app, "ui.dialog.set", params.clone());
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert_eq!(fields(&app), before, "{params}: a refused list stores no field");
    }
    let list = json!([{"field": "copy", "value": true}, {"field": "dx", "value": "3 pt"}, {"field": "dx", "value": "5 pt"}, {"field": "dy"}]);
    let r = input_request(&mut app, "ui.dialog.set", json!({"fields": list}));
    assert_eq!(r["ok"], true, "{r}");
    let after = fields(&app).unwrap();
    assert_eq!(after, json!({"copy": true, "dx": "5 pt", "dy": null}));
    assert_eq!(r["result"]["fields"], after);
    let r = input_request(&mut app, "ui.dialog.set", json!({"field": "dy", "value": "2 pt"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(fields(&app).unwrap()["dy"], "2 pt");
    app.ui.dialog = None;
    assert_eq!(input_request(&mut app, "ui.dialog.set", json!({"fields": []}))["error"], "no dialog open");
}

/// `ui.dialog.set {fields}` stores at most `MAX_DIALOG_FIELDS` items; a longer list stores none.
#[test]
fn dialog_set_refuses_a_field_list_over_the_limit() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.ui.dialog = Some(crate::state::Dialog::new("move", json!({"dx": "0 pt"})));
    let list = |n: usize| Value::Array((0..n).map(|i| json!({"field": "dx", "value": format!("{i} pt")})).collect());
    let r = input_request(&mut app, "ui.dialog.set", json!({"fields": list(MAX_DIALOG_FIELDS + 1)}));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["error"], format!("`fields` holds {} items; at most {MAX_DIALOG_FIELDS}", MAX_DIALOG_FIELDS + 1));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.fields["dx"].clone()), Some(json!("0 pt")), "a refused list stores no field");
    let r = input_request(&mut app, "ui.dialog.set", json!({"fields": list(MAX_DIALOG_FIELDS)}));
    assert_eq!(r["ok"], true, "{r}");
    let last = format!("{} pt", MAX_DIALOG_FIELDS - 1);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.fields["dx"].clone()), Some(json!(last)), "the whole list is stored, in order");
}

/// #1004: `ui.render` (the MCP `screenshot` of a connected app) renders the artboard asked
/// for, at the scale asked for; the first without one.
#[test]
fn render_takes_the_artboard_asked_for() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Services::default()).with_control(rx);
    let mut request = |method: &str, params: Value| {
        let (req, reply) = ControlRequest::new(method, params);
        tx.send(req).unwrap();
        app.drain_control(&egui::Context::default());
        reply.try_recv().unwrap()
    };
    let new = json!({"width": 64, "height": 64, "units": "Points", "artboards": 2});
    request("engine.execute", json!({"command": "file.new", "params": new}));
    let board = json!({"index": 1, "x": 100, "y": 0, "width": 96, "height": 48});
    let r = request("engine.execute", json!({"command": "artboard.setProps", "params": board}));
    assert_eq!(r["ok"], true, "{r}");
    for (params, size) in [
        (json!({}), [64, 64]),
        (json!({"artboard": 0}), [64, 64]),
        (json!({"artboard": 1}), [96, 48]),
        (json!({"artboard": 1, "scale": 0.5}), [48, 24]),
    ] {
        let r = request("ui.render", params.clone());
        let got = [&r["result"]["width"], &r["result"]["height"]].map(Value::as_u64);
        assert_eq!(got, size.map(Some), "{params}: {r}");
    }
    assert!(request("ui.render", json!({"artboard": 2}))["error"].is_string());
}

/// `ui.pointer` replies with the key object: a click on one of two selected objects makes it
/// the key, and a click on the key again lets it go.
#[test]
fn pointer_replies_name_the_key_object() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    let b = app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let ctx = egui::Context::default();
    let click = |app: &mut VectorcraftApp| {
        let events = json!([{"kind": "down", "x": 110, "y": 20}, {"kind": "up", "x": 110, "y": 20}]);
        let (req, _) = ControlRequest::new("ui.pointer", json!({"events": events}));
        let Outcome::Done(r) = handle(app, &ctx, &req) else { panic!("not done") };
        assert_eq!(r["ok"], true, "{r}");
        r["result"].get("key").cloned()
    };
    assert_eq!(click(&mut app), Some(b), "a click on a selected object makes it the key");
    assert_eq!(click(&mut app), Some(Value::Null), "a click on the key again lets it go");
}

/// A Shape Builder click on a large selection merges on another thread: the frames go on, and
/// the `ui.pointer` reply comes once the merge is applied, with the selection after it.
#[test]
fn pointer_reply_waits_for_a_merge_on_another_thread() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Services::default()).with_control(rx);
    app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
    for k in 0..40 {
        app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
    app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
    let ctx = egui::Context::default();
    let send = |app: &mut VectorcraftApp, events: Value| {
        let (req, reply) = ControlRequest::new("ui.pointer", json!({"events": events}));
        tx.send(req).unwrap();
        app.drain_control(&ctx);
        reply
    };
    let hover = send(&mut app, json!([{"kind": "move", "x": 15, "y": 10}]));
    assert_eq!(hover.try_recv().unwrap()["ok"], true, "a hover is answered at once");
    app.session.tool_poll(app.view_info(), true).unwrap();
    let revision = app.session.active().unwrap().revision;
    let click = send(&mut app, json!([{"kind": "down", "x": 15, "y": 10}, {"kind": "up", "x": 15, "y": 10}]));
    assert!(click.try_recv().is_err() && app.session.tool_editing(), "the click is not answered before the merge is applied");
    // A hover sent meanwhile is answered at once; a click is made after the merge, and its
    // reply waits until then.
    let hover = send(&mut app, json!([{"kind": "move", "x": 35, "y": 10}]));
    assert_eq!(hover.try_recv().map(|r| r["ok"].clone()), Ok(json!(true)), "a hover during the merge");
    let cmd = json!({"cmd": true});
    let press = send(&mut app, json!([{"kind": "down", "x": 500, "y": 150, "mods": cmd}, {"kind": "up", "x": 500, "y": 150, "mods": cmd}]));
    assert!(press.try_recv().is_err(), "a Cmd-click during the merge waits for it");
    let hover = send(&mut app, json!([{"kind": "move", "x": 45, "y": 10}]));
    assert_eq!(hover.try_recv().map(|r| r["ok"].clone()), Ok(json!(true)), "a hover after the held Cmd-click");
    let start = std::time::Instant::now();
    let reply = loop {
        if let Ok(r) = click.try_recv() {
            break r;
        }
        assert!(app.session.tool_editing(), "the merge is being computed until the reply goes");
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert!(start.elapsed() < std::time::Duration::from_secs(60), "no reply");
        std::thread::sleep(std::time::Duration::from_millis(2));
        // A frame: the canvas checks on the tool, then the replies are answered.
        let r = app.session.tool_poll(app.view_info(), false);
        crate::canvas::apply_requests(&mut app, r);
        app.answer_pointer_replies(&ctx);
    };
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["selection"].as_array().map(Vec::len), Some(41), "{reply}: the selection after the merge");
    assert_ne!(app.session.active().unwrap().revision, revision);
    let press = loop {
        if let Ok(r) = press.try_recv() {
            break r;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(60), "no reply to the Cmd-click");
        let r = app.session.tool_poll(app.view_info(), false);
        crate::canvas::apply_requests(&mut app, r);
        app.answer_pointer_replies(&ctx);
    };
    assert_eq!(press["result"]["selection"].as_array().map(Vec::len), Some(0), "{press}: the lent selection tool deselected after the merge");
    assert_eq!(press["result"]["tool"], "shapeBuilder", "{press}");
}

/// A Cmd-click held while a Shape Builder merge is computed on another thread makes the artboard
/// it lands in active once it is made, as a Cmd-click made at once does: by the frames' checks,
/// by choosing another tool first, or before a command that acts on the active artboard takes
/// it.
#[test]
fn a_held_cmd_click_makes_its_artboard_active() {
    for then in ["frames", "another tool", "All on Active Artboard", "another document"] {
        held_cmd_click(then);
    }
}

fn held_cmd_click(then: &str) {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    if then == "another document" {
        // A document with an artboard where the press is, whose view keeps its own.
        app.run("file.new", json!({"width": 1400, "height": 200})).unwrap();
        app.run("artboard.new", json!({"x": 1000, "y": 0, "width": 300, "height": 200})).unwrap();
        app.view_mut().unwrap().artboard = 0;
    }
    app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
    app.run("artboard.new", json!({"x": 1000, "y": 0, "width": 300, "height": 200})).unwrap();
    for k in 0..40 {
        app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
    app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
    app.view_mut().unwrap().artboard = 0;
    let view = app.view_info();
    let at = |kind, x, mods| PointerEvent { kind, pos: vectorcraft_geom::Point::new(x, 10.0), mods, pressure: 1.0 };
    crate::canvas::dispatch(&mut app, &at(PointerKind::Move, 15.0, Mods::default()), view);
    app.session.tool_poll(view, true).unwrap();
    crate::canvas::dispatch(&mut app, &at(PointerKind::Down, 15.0, Mods::default()), view);
    crate::canvas::dispatch(&mut app, &at(PointerKind::Up, 15.0, Mods::default()), view);
    let cmd = Mods { cmd: true, ..Default::default() };
    crate::canvas::dispatch(&mut app, &at(PointerKind::Down, 1100.0, cmd), view);
    crate::canvas::dispatch(&mut app, &at(PointerKind::Up, 1100.0, cmd), view);
    assert_eq!(app.view().unwrap().artboard, 0, "{then}: the Cmd-click is held");
    if then == "another tool" {
        app.select_tool("rectangle");
        assert!(!app.session.tool_editing(), "{then}");
    }
    if then == "All on Active Artboard" {
        app.run("select.allOnArtboard", json!({})).unwrap();
        assert!(app.session.doc().unwrap().selection.is_empty(), "{then}: the second artboard, which holds no art");
    }
    let doc = app.session.active_index().unwrap();
    if then == "another document" {
        // A document tab chosen.
        app.session.set_active(0);
    }
    while app.session.tool_waiting() {
        let r = app.session.tool_poll(view, true);
        crate::canvas::apply_requests(&mut app, r);
    }
    app.sync_views();
    assert_eq!(app.views[doc].artboard, 1, "{then}: the Cmd-click made the artboard it landed in active");
    if then == "another document" {
        assert_eq!(app.views[0].artboard, 0, "{then}: the other document's view keeps its artboard");
    }
}

/// A dialog chosen from the menu, or with `ui.menuDialog`, while a Shape Builder merge is
/// computed on another thread opens after the merge is applied, on the merged art. So do the
/// app's own dialogs (Recolor Artwork), from the menu or a panel's button.
#[test]
fn menu_dialogs_open_after_a_merge_on_another_thread() {
    for route in ["menu", "ui.menuDialog", "app dialog from the menu", "app dialog from a panel"] {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
        for k in 0..40 {
            app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
        }
        app.run("select.all", json!({})).unwrap();
        app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
        let view = app.view_info();
        let at = |kind| PointerEvent { kind, pos: vectorcraft_geom::Point::new(15.0, 10.0), mods: Mods::default(), pressure: 1.0 };
        crate::canvas::dispatch(&mut app, &at(PointerKind::Move), view);
        app.session.tool_poll(view, true).unwrap();
        crate::canvas::dispatch(&mut app, &at(PointerKind::Down), view);
        crate::canvas::dispatch(&mut app, &at(PointerKind::Up), view);
        assert!(app.session.tool_editing(), "{route}");
        match route {
            "menu" => crate::menus::invoke(&mut app, "object.move", json!({})),
            "ui.menuDialog" => {
                app.run("ui.menuDialog", json!({"command": "object.move"})).unwrap();
            }
            "app dialog from the menu" => crate::menus::invoke(&mut app, "ui.recolorDialog", json!({})),
            _ => {
                app.run("ui.recolorDialog", json!({})).unwrap();
            }
        }
        assert!(app.ui.dialog.is_some(), "{route}: the dialog is open");
        assert!(!app.session.tool_editing(), "{route}");
        let merged = app.session.doc().unwrap().history.undo.last().map(|e| e.label.clone());
        assert_eq!(merged.as_deref(), Some("Shape Builder"), "{route}");
        assert_eq!(app.session.doc().unwrap().selection.objects.len(), 41, "{route}: the dialog acts on the merged art");
    }
}

/// The Separations panel's checkboxes and a preference set while a Shape Builder merge is
/// computed on another thread give the documents new revisions: the merge is applied first,
/// and the document keeps it.
#[test]
fn panels_that_redraw_apply_a_merge_on_another_thread_first() {
    let cases = [
        ("view.separationsPreview", json!({"on": false})),
        ("view.overprintPreview", json!({"on": false})),
        ("prefs.set", json!({"values": {"gridlineEvery": 90}})),
    ];
    for (id, params) in cases {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
        for k in 0..40 {
            app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
        }
        app.run("select.all", json!({})).unwrap();
        app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
        let view = app.view_info();
        let at = |kind| PointerEvent { kind, pos: vectorcraft_geom::Point::new(15.0, 10.0), mods: Mods::default(), pressure: 1.0 };
        crate::canvas::dispatch(&mut app, &at(PointerKind::Move), view);
        app.session.tool_poll(view, true).unwrap();
        crate::canvas::dispatch(&mut app, &at(PointerKind::Down), view);
        crate::canvas::dispatch(&mut app, &at(PointerKind::Up), view);
        assert!(app.session.tool_editing(), "{id}");
        app.run(id, params).unwrap();
        assert!(!app.session.tool_editing(), "{id}");
        for _ in 0..3 {
            app.session.tool_poll(view, false).unwrap();
        }
        let merged = app.session.doc().unwrap().history.undo.last().map(|e| e.label.clone());
        assert_eq!(merged.as_deref(), Some("Shape Builder"), "{id}");
        assert_eq!(app.session.doc().unwrap().selection.objects.len(), 41, "{id}");
    }
}

/// Saving, exporting or closing while a Shape Builder merge is computed on another thread
/// applies the merge first: the file holds it, and closing asks to save it.
#[test]
fn save_and_close_apply_a_merge_on_another_thread_first() {
    let dir = vectorcraft_testkit::temp_dir("control-merge-save");
    for then in ["file.saveAs", "file.exportAs", "file.close"] {
        let services = Services { write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))), ..Default::default() };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 800, "height": 200})).unwrap();
        for k in 0..40 {
            app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
        }
        app.run("select.all", json!({})).unwrap();
        app.run("tool.select", json!({"tool": "shapeBuilder"})).unwrap();
        // As if just saved: only the merge modifies it.
        app.session.doc_mut().unwrap().mark_saved();
        let view = app.view_info();
        let at = |kind| PointerEvent { kind, pos: vectorcraft_geom::Point::new(15.0, 10.0), mods: Mods::default(), pressure: 1.0 };
        crate::canvas::dispatch(&mut app, &at(PointerKind::Move), view);
        app.session.tool_poll(view, true).unwrap();
        crate::canvas::dispatch(&mut app, &at(PointerKind::Down), view);
        crate::canvas::dispatch(&mut app, &at(PointerKind::Up), view);
        assert!(app.session.tool_editing() && !app.session.doc().unwrap().is_dirty(), "{then}");
        let path = dir.join(if then == "file.exportAs" { "a.svg" } else { "a.vectorcraft" });
        let p = if then == "file.close" { json!({}) } else { json!({"path": path.to_string_lossy()}) };
        let r = app.run(then, p).unwrap();
        assert!(!app.session.tool_editing(), "{then}");
        let merged = app.session.doc().unwrap().history.undo.last().map(|e| e.label.clone());
        assert_eq!(merged.as_deref(), Some("Shape Builder"), "{then}");
        match then {
            "file.saveAs" => assert!(!app.session.doc().unwrap().is_dirty(), "the file holds the merge"),
            "file.exportAs" => assert!(app.session.doc().unwrap().is_dirty(), "an export leaves the document modified"),
            _ => assert_eq!(r["pending"], "saveChanges", "closing asks to save the merge"),
        }
    }
}

/// `ui.dialog.set` of a field the open dialog does not have is an error that lists its fields,
/// and the dialog stays as it was. Fields a dialog may not hold yet are still set: `discard`
/// with a discard button, `preview` with a live preview, `__` fields and the spec's `inputs`.
#[test]
fn dialog_set_refuses_a_field_the_dialog_does_not_have() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let fields = json!({"index": 0, "uid": 1, "name": "Untitled-1", "then": "close"});
    app.ui.dialog = Some(Dialog::new(crate::unsaved::KIND, fields.clone()));
    let r = input_request(&mut app, "ui.dialog.set", json!({"field": "discrad", "value": true}));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["error"], "the `saveChanges` dialog has no field `discrad`; its fields: discard, index, name, then, uid");
    assert_eq!(app.ui.dialog.as_ref().map(|d| Value::Object(d.fields.clone())), Some(fields));
    for field in ["name", "discard", "__note"] {
        let r = input_request(&mut app, "ui.dialog.set", json!({"field": field, "value": true}));
        assert_eq!(r["ok"], true, "{field}: {r}");
    }
    // Move opens without Copy, which its checkbox adds once clicked; Scale's options and the
    // reference point belong to the other transform dialogs.
    let set = |app: &mut VectorcraftApp, kind: &str, fields: Value, cases: &[(&str, bool)]| {
        app.ui.dialog = Some(Dialog::new(kind, fields));
        for (field, ok) in cases {
            let r = input_request(app, "ui.dialog.set", json!({"field": field, "value": true}));
            assert_eq!(r["ok"], *ok, "{kind} {field}: {r}");
        }
    };
    let moved = [("copy", true), ("preview", true), ("distance", true), ("discard", false), ("copies", false), ("corners", false), ("origin", false)];
    set(&mut app, "move", json!({"dx": "0 pt", "dy": "0 pt"}), &moved);
    set(
        &mut app,
        "scale",
        json!({"sx": 100, "sy": 100, "uniform": true}),
        &[("corners", true), ("strokes", true), ("origin", true), ("distance", false)],
    );
    set(&mut app, "rotate", json!({"angle": 0}), &[("origin", true), ("objects", true), ("corners", false)]);
    set(&mut app, "rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}), &[("radius", true), ("sides", false)]);
    set(&mut app, "ellipse", json!({"x": 0, "y": 0, "width": 10, "height": 10}), &[("radius", false)]);
    // Corners that differ in kind and radius open without either.
    let mixed = json!({"id": 1, "corners": [0, 1, 2, 3], "relativeRadii": false});
    set(&mut app, vectorcraft_tools::corners::DIALOG, mixed, &[("kind", true), ("radius", true), ("radii", false)]);
    set(&mut app, "saveForOffice", json!({"ppi": 150, "transparent": false, "artboard": 0}), &[("path", true)]);
    set(&mut app, "brushOptions", json!({"name": "Dots"}), &[("tips", true), ("tip", false)]);
    set(&mut app, "justification", json!({"wordDesired": 100}), &[("wordMin", true), ("singleWord", true), ("wordMid", false)]);
    set(&mut app, "hyphenation", json!({"hyphenate": true}), &[("afterFirst", true), ("hyphenationZone", true), ("afterfirst", false)]);
    set(&mut app, "effectGallery", json!({"stack": []}), &[("browseEffect", true), ("browseParams", true), ("browse", false)]);
    // A new artboard's dialog opens without a background color or display marks.
    let new_board = json!({"new": true, "name": "Artboard 2", "x": 0, "y": 0, "width": 10, "height": 10});
    let marks = [
        ("color", true),
        ("showCenterMark", true),
        ("showCrossHairs", true),
        ("showVideoSafeAreas", true),
        ("videoRulerPixelAspectRatio", true),
        ("showCenterMarks", false),
    ];
    set(&mut app, "artboardOptions", new_board, &marks);
    // Glass and Texturizer take the texture Load Texture… reads; other effects don't.
    set(&mut app, "effect", json!({"__effect": "distort.glass"}), &[("textureImage", true)]);
    set(&mut app, "effect", json!({"__effect": "blur.gaussian"}), &[("textureImage", false)]);
    // The generic dialog takes every parameter of its command.
    set(&mut app, "command", json!({"__command": "shape.rectangle", "x": 0, "y": 0}), &[("width", true), ("widht", false)]);
    // Transparency Flattener Presets holds the selected preset's options before its first frame.
    crate::dialogs::flattener_presets::open(&mut app, None);
    let r = input_request(&mut app, "ui.dialog.set", json!({"field": "textToOutlines", "value": true}));
    assert_eq!(r["ok"], true, "{r}");
    // Edit Selection takes `deleted<i>` for each of its rows only.
    app.run("file.new", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    for name in ["A", "B"] {
        app.run("select.save", json!({ "name": name })).unwrap();
    }
    crate::dialogs::edit_selection::open(&mut app).unwrap();
    for (field, ok) in [("deleted1", true), ("deleted", false), ("deletedx", false), ("deleted01", false), ("deleted2", false)] {
        let r = input_request(&mut app, "ui.dialog.set", json!({"field": field, "value": true}));
        assert_eq!(r["ok"], ok, "{field}: {r}");
    }
    let r = input_request(&mut app, "ui.dialog.set", json!({"field": "name", "value": "A"}));
    assert_eq!(
        r["error"],
        "the `editSelection` dialog has no field `name`; its fields: count, deleted0, deleted1, name0, name1, orig0, orig1, selected"
    );
    app.ui.dialog = None;
    assert_eq!(input_request(&mut app, "ui.dialog.set", json!({"field": "name", "value": "A"}))["error"], "no dialog open");
}
