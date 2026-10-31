use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::{ControlRequest, MAX_DIALOG_FIELDS, Outcome, handle};
use crate::state::Dialog;
use crate::{Services, VectorcraftApp};

fn input_request(app: &mut VectorcraftApp, method: &str, params: Value) -> Value {
    let (req, _) = ControlRequest::new(method, params);
    let Outcome::Done(r) = handle(app, &egui::Context::default(), &req) else { panic!("expected immediate reply") };
    r
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

#[test]
fn synthetic_pointer_backquote_keeps_solid_object_in_place() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let tile = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.run("object.pattern.make", json!({"ids": [tile], "name": "Dots", "width": 20, "height": 20})).unwrap();
    app.run("object.pattern.done", json!({})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Dots"})).unwrap();
    let solid = app.run("shape.rectangle", json!({"x": 220, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [solid], "color": "#3366cc"})).unwrap();
    app.run("select.set", json!({"ids": [id, solid]})).unwrap();
    let before = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    let before_solid = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(solid)).unwrap().clone();
    let r = input_request(
        &mut app,
        "ui.pointer",
        json!({"events": [
        {"kind": "down", "x": 125, "y": 125},
        {"kind": "drag", "x": 145, "y": 125},
        {"kind": "up", "x": 145, "y": 125}
    ], "backquote": true}),
    );
    assert_eq!(r["ok"], true, "{r}");
    let after = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
    assert_eq!(after.geometric_bounds(), before.geometric_bounds());
    assert_eq!(app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(solid)).unwrap(), &before_solid);
    let vectorcraft_color::Paint::Pattern { xf, .. } = after.appearance.fill_paint() else { panic!("pattern fill expected") };
    assert_eq!(xf.as_coeffs()[4], 20.0, "synthetic backquote drag changes pattern paint");
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
