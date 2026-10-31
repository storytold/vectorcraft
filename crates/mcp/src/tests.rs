use std::io::{BufRead, BufReader, Write};

use serde_json::{Value, json};

use crate::{Backend, Headless, PROTOCOL_VERSION, Remote, Server, tool_definitions};

mod batchdraw;
mod concurrency;
mod conventions;
mod distortkeys;
mod drawopts;
mod exportas;
mod fileio;
mod freeform;
mod gradient;
mod limits;
mod links;
mod liquify;
mod pen;
mod persp;
mod place;
mod protocol;
mod svg;

fn server() -> Server {
    Server::new(Box::new(Headless::with_document()))
}

fn rpc(s: &mut Server, id: u64, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    let reply = s.handle_line(&line).expect("reply");
    let v: Value = serde_json::from_str(&reply).expect("reply is JSON");
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], id);
    v
}

fn call(s: &mut Server, id: u64, name: &str, args: Value) -> Value {
    let v = rpc(s, id, "tools/call", json!({"name": name, "arguments": args}));
    assert!(v.get("error").is_none(), "{v}");
    v["result"].clone()
}

fn text_of(result: &Value) -> String {
    result["content"].as_array().unwrap().iter().filter(|c| c["type"] == "text").map(|c| c["text"].as_str().unwrap().to_string()).collect()
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vectorcraft-mcp-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

// ---------- framing ----------

#[test]
fn framing_basics() {
    let mut s = server();
    // Blank lines and notifications produce no output.
    assert_eq!(s.handle_line("   "), None);
    assert_eq!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#), None);
    assert!(s.is_initialized());
    // Parse error → -32700 with null id.
    let v: Value = serde_json::from_str(&s.handle_line("{not json").unwrap()).unwrap();
    assert_eq!(v["error"]["code"], -32700);
    assert_eq!(v["id"], Value::Null);
    // Unknown method → -32601, id echoed (string ids too).
    let v: Value = serde_json::from_str(&s.handle_line(r#"{"jsonrpc":"2.0","id":"abc","method":"nope"}"#).unwrap()).unwrap();
    assert_eq!(v["error"]["code"], -32601);
    assert_eq!(v["id"], "abc");
    // Missing method → invalid request.
    let v: Value = serde_json::from_str(&s.handle_line(r#"{"jsonrpc":"2.0","id":3}"#).unwrap()).unwrap();
    assert_eq!(v["error"]["code"], -32600);
    // Responses from the client are ignored.
    assert_eq!(s.handle_line(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#), None);
    // ping
    assert_eq!(rpc(&mut s, 4, "ping", json!({}))["result"], json!({}));
    // Batch (legacy clients).
    let v: Value = serde_json::from_str(
        &s.handle_line(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"}]"#).unwrap(),
    )
    .unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
}

#[test]
fn framing_rejects_invalid_versions_before_dispatch() {
    use std::cell::Cell;
    use std::rc::Rc;

    struct CountingBackend(Rc<Cell<usize>>);
    impl Backend for CountingBackend {
        fn call(&mut self, _: &str, _: Value) -> Result<Value, String> {
            self.0.set(self.0.get() + 1);
            Ok(json!({}))
        }
        fn has_ui(&self) -> bool {
            false
        }
        fn describe(&self) -> String {
            "test".into()
        }
    }

    let calls = Rc::new(Cell::new(0));
    let mut s = Server::new(Box::new(CountingBackend(calls.clone())));
    let mut request = json!({"id": 7, "method": "tools/call", "params": {"name": "run_command", "arguments": {"command": "edit.clear"}}});
    for version in [None, Some(json!("1.0")), Some(json!("3.0")), Some(json!(2.0)), Some(Value::Null)] {
        if let Some(version) = version {
            request["jsonrpc"] = version;
        }
        let reply: Value = serde_json::from_str(&s.handle_line(&request.to_string()).unwrap()).unwrap();
        assert_eq!(calls.get(), 0, "invalid envelope dispatched: {request}");
        assert_eq!(reply["error"]["code"], -32600, "{reply}");
        assert_eq!(reply["id"], 7);
    }
    request["jsonrpc"] = json!("2.0");
    let reply = s.handle(request).unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    assert_eq!(calls.get(), 1);
}

#[test]
fn framing_rejects_invalid_notification_versions_in_batches() {
    let mut s = server();
    for version in [None, Some(json!("1.0")), Some(json!(2.0)), Some(Value::Null)] {
        let mut notification = json!({"method": "notifications/initialized"});
        if let Some(version) = version {
            notification["jsonrpc"] = version;
        }
        let batch = json!([notification, {"jsonrpc": "2.0", "id": "ping", "method": "ping"}]);
        let replies: Value = serde_json::from_str(&s.handle_line(&batch.to_string()).unwrap()).unwrap();
        assert!(!s.is_initialized(), "invalid notification changed state: {batch}");
        assert_eq!(replies.as_array().unwrap().len(), 1, "{replies}");
        assert_eq!(replies[0]["id"], "ping");
        assert_eq!(replies[0]["result"], json!({}));
    }
    assert_eq!(s.handle(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})), None);
    assert!(s.is_initialized());
}

#[test]
fn serve_loop_writes_one_line_per_request() {
    let mut s = server();
    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":PROTOCOL_VERSION,"capabilities":{},"clientInfo":{"name":"t","version":"0"}}}).to_string(),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string(),
    ]
    .join("\n");
    let mut out = Vec::new();
    s.serve(input.as_bytes(), &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(lines[0]["result"]["serverInfo"]["name"], "vectorcraft");
    assert!(lines[0]["result"]["capabilities"]["tools"].is_object());
    assert!(lines[0]["result"]["capabilities"]["resources"].is_object());
    assert!(lines[1]["result"]["tools"].as_array().unwrap().len() >= 19);
}

#[test]
fn initialize_negotiates_version() {
    let mut s = server();
    let v = rpc(&mut s, 1, "initialize", json!({"protocolVersion": "2025-03-26"}));
    assert_eq!(v["result"]["protocolVersion"], "2025-03-26");
    let v = rpc(&mut s, 2, "initialize", json!({"protocolVersion": "1999-01-01"}));
    assert_eq!(v["result"]["protocolVersion"], PROTOCOL_VERSION);
}

// ---------- tools/list ----------

#[test]
fn tool_schemas_are_valid() {
    let tools = tool_definitions();
    let mut names = std::collections::HashSet::new();
    for t in &tools {
        let name = t["name"].as_str().expect("name");
        assert!(!name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'), "{name}");
        assert!(names.insert(name), "duplicate tool {name}");
        assert!(t["description"].as_str().is_some_and(|d| d.len() > 10), "{name} description");
        let schema = &t["inputSchema"];
        assert_eq!(schema["type"], "object", "{name}");
        let props = schema["properties"].as_object().unwrap_or_else(|| panic!("{name} properties"));
        for r in schema.get("required").and_then(Value::as_array).into_iter().flatten() {
            assert!(props.contains_key(r.as_str().unwrap()), "{name}: required {r} not in properties");
        }
        for (k, p) in props {
            assert!(p.is_object(), "{name}.{k}");
            assert!(p.get("type").is_some() || p.get("anyOf").is_some(), "{name}.{k} has no type");
        }
    }
    for want in [
        "list_commands",
        "run_command",
        "inspect_document",
        "inspect_ui",
        "select_tool",
        "pointer_gesture",
        "draw_path",
        "draw_shape",
        "set_paint",
        "press_key",
        "invoke_menu",
        "open_panel",
        "screenshot",
        "open_file",
        "save_file",
        "export",
        "undo",
        "redo",
    ] {
        assert!(names.contains(want), "missing tool {want}");
    }
}

// ---------- headless end-to-end ----------

#[test]
fn headless_end_to_end() {
    let mut s = server();
    rpc(&mut s, 1, "initialize", json!({"protocolVersion": PROTOCOL_VERSION}));
    s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    let r = call(
        &mut s,
        2,
        "draw_shape",
        json!({"shape": "rectangle", "x": 100, "y": 100, "width": 200, "height": 120, "fill": "#ff0000", "stroke": "none"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let created: Value = serde_json::from_str(&text_of(&r)).unwrap();
    let id = created["id"].as_u64().expect("id");

    let r = call(
        &mut s,
        3,
        "draw_shape",
        json!({"shape": "star", "cx": 400, "cy": 400, "radius1": 80, "radius2": 40, "fill": [0, 0, 1], "strokeWidth": 3}),
    );
    assert_eq!(r["isError"], false, "{r}");

    let r = call(&mut s, 4, "inspect_document", json!({}));
    let doc: Value = serde_json::from_str(&text_of(&r)).unwrap();
    assert!(doc["objects"].as_u64().unwrap() >= 3, "{doc}");
    let layer = &doc["layers"][0];
    let rect = layer["children"].as_array().unwrap().iter().find(|c| c["id"] == id).expect("rect in layer");
    assert_eq!(rect["bounds"]["width"], 200.0);
    assert_eq!(rect["fill"], "#ff0000", "{rect}");
    assert_eq!(rect["stroke"], "None");

    // A depth-0 inspect is the skeleton: top layers with counts, nothing below them;
    // a bad slice is a tool error, not a full dump.
    let r = call(&mut s, 41, "inspect_document", json!({"depth": 0}));
    let skel: Value = serde_json::from_str(&text_of(&r)).unwrap();
    assert!(skel["layers"][0].get("children").is_none(), "{skel}");
    assert_eq!(skel["layers"][0]["childCount"], layer["children"].as_array().unwrap().len());
    assert_eq!(skel["artboards"], doc["artboards"]);
    assert_eq!(call(&mut s, 42, "inspect_document", json!({"depth": -1}))["isError"], true);

    // Scoping reaches the command: one node by id equals its part of the full tree;
    // an unknown id is a tool error.
    let r = call(&mut s, 43, "doc_inspect", json!({"ids": [id]}));
    let one: Value = serde_json::from_str(&text_of(&r)).unwrap();
    assert_eq!(one["nodes"], json!([rect]), "{one}");
    assert_eq!(call(&mut s, 44, "doc_inspect", json!({"ids": [999_999]}))["isError"], true);

    // Screenshot returns image content (base64 PNG) and text.
    let shot_path = tmp("shot.png");
    let r = call(&mut s, 5, "screenshot", json!({"path": shot_path.to_str().unwrap()}));
    assert_eq!(r["isError"], false, "{r}");
    let img = r["content"].as_array().unwrap().iter().find(|c| c["type"] == "image").expect("image content");
    assert_eq!(img["mimeType"], "image/png");
    let png = vectorcraft_format::base64_decode(img["data"].as_str().unwrap()).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(std::fs::read(&shot_path).unwrap(), png);

    // Export SVG.
    let svg_path = tmp("out.svg");
    let r = call(&mut s, 6, "export", json!({"path": svg_path.to_str().unwrap()}));
    assert_eq!(r["isError"], false, "{r}");
    let svg = std::fs::read_to_string(&svg_path).unwrap();
    assert!(svg.contains("<svg") && (svg.contains("<rect") || svg.contains("<path")), "{svg}");

    // Save native, open it back.
    let dc = tmp("out.vectorcraft");
    assert_eq!(call(&mut s, 7, "save_file", json!({"path": dc.to_str().unwrap()}))["isError"], false);
    let r = call(&mut s, 8, "open_file", json!({"path": dc.to_str().unwrap()}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 9, "open_file", json!({"path": svg_path.to_str().unwrap()}));
    assert_eq!(r["isError"], false, "{r}");

    // PDF and PDF-compatible .ai open headless too.
    let pdf_path = tmp("out.pdf");
    assert_eq!(call(&mut s, 20, "export", json!({"path": pdf_path.to_str().unwrap()}))["isError"], false);
    let ai_path = tmp("out.ai");
    std::fs::copy(&pdf_path, &ai_path).unwrap();
    for (id, p) in [(21, &pdf_path), (22, &ai_path)] {
        let r = call(&mut s, id, "open_file", json!({"path": p.to_str().unwrap()}));
        assert_eq!(r["isError"], false, "{r}");
        let doc: Value = serde_json::from_str(&text_of(&call(&mut s, id + 10, "inspect_document", json!({})))).unwrap();
        assert!(doc["objects"].as_u64().unwrap() >= 2, "{doc}");
    }

    // Resources.
    let v = rpc(&mut s, 10, "resources/list", json!({}));
    assert_eq!(v["result"]["resources"].as_array().unwrap().len(), 3);
    let v = rpc(&mut s, 11, "resources/read", json!({"uri": "vectorcraft://document"}));
    let text = v["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(serde_json::from_str::<Value>(text).unwrap()["layers"].is_array());
    let v = rpc(&mut s, 12, "resources/read", json!({"uri": "vectorcraft://document/json"}));
    assert!(v["result"]["contents"][0]["text"].as_str().unwrap().len() > 10);
    let v = rpc(&mut s, 13, "resources/read", json!({"uri": "vectorcraft://nope"}));
    assert_eq!(v["error"]["code"], -32002);
}

/// The trace-and-style prompt's steps, run in its order on an image drawn here: each one
/// succeeds, the expanded trace holds no image, and its paths keep the image's colors.
#[test]
fn trace_and_style_prompt_steps_run_in_order() {
    let mut s = server();
    // 60 × 40 pixels: a red and a blue block on white.
    let png = tmp("trace-and-style.png");
    image::RgbImage::from_fn(60, 40, |x, y| match (x, y) {
        (5..25, 5..35) => image::Rgb([220, 30, 30]),
        (35..55, 5..35) => image::Rgb([30, 60, 200]),
        _ => image::Rgb([255, 255, 255]),
    })
    .save(&png)
    .unwrap();
    let path = png.to_str().unwrap();
    let v = rpc(&mut s, 1, "prompts/get", json!({"name": "trace-and-style", "arguments": {"path": path}}));
    let prompt = v["result"]["messages"][0]["content"]["text"].as_str().unwrap().to_string();

    let ok = |s: &mut Server, id: u64, name: &str, args: Value| {
        let r = call(s, id, name, args);
        assert_eq!(r["isError"], false, "{name}: {r}");
        serde_json::from_str::<Value>(&text_of(&r)).unwrap_or(Value::Null)
    };
    let command = |s: &mut Server, id: u64, command: &str, params: Value| {
        assert!(prompt.contains(command), "the prompt names {command}: {prompt}");
        ok(s, id, "run_command", json!({"command": command, "params": params}))
    };
    ok(&mut s, 2, "open_file", json!({"path": path}));
    command(&mut s, 3, "select.all", json!({}));
    let traced = command(&mut s, 4, "imageTrace.make", json!({"preset": "6 Colors"}));
    assert!(traced["colors"].as_u64().unwrap() > 1, "{traced}");
    command(&mut s, 5, "imageTrace.expand", json!({}));
    ok(&mut s, 6, "apply_effect", json!({"effect": "stylize.scribble"}));
    let colors = command(&mut s, 7, "recolor.colors", json!({}));
    let from = colors["colors"].as_array().unwrap().iter().filter_map(|c| c["hex"].as_str()).find(|h| *h != "#ffffff").unwrap().to_string();
    command(&mut s, 8, "recolor.apply", json!({"map": {from.as_str(): "#00aa00"}}));
    let r = call(&mut s, 9, "render_preview", json!({}));
    assert_eq!(r["isError"], false, "{r}");

    // No image is left, and the paths carry more than one fill color, the new one among them.
    let doc = ok(&mut s, 10, "inspect_document", json!({}));
    let (mut kinds, mut fills) = (vec![], std::collections::BTreeSet::new());
    let mut stack: Vec<&Value> = doc["layers"].as_array().unwrap().iter().collect();
    while let Some(n) = stack.pop() {
        kinds.push(n["kind"].as_str().unwrap_or_default().to_string());
        if let Some(fill) = n["fill"].as_str().filter(|f| f.starts_with('#')) {
            fills.insert(fill.to_string());
        }
        stack.extend(n["children"].as_array().into_iter().flatten());
    }
    assert!(!kinds.iter().any(|k| k == "Image"), "{kinds:?}");
    assert!(fills.len() > 1 && fills.contains("#00aa00") && !fills.contains(&from), "{fills:?}");
}

/// `screenshot` and `render_preview` with `traceViews: true` draw an Image Trace object in the
/// view set with imageTrace.setView, as the app's window does; without it they draw the tracing
/// result, as exports do. The image is a dark disc (r = 30 px) on blue, traced with Ignore White.
#[test]
fn screenshot_and_render_preview_draw_the_trace_view_with_trace_views() {
    let mut s = server();
    let png = tmp("trace-views.png");
    image::RgbImage::from_fn(100, 100, |x, y| {
        let (dx, dy) = (f64::from(x) + 0.5 - 50.0, f64::from(y) + 0.5 - 50.0);
        if dx * dx + dy * dy <= 900.0 { image::Rgb([0, 0, 0]) } else { image::Rgb([170, 210, 255]) }
    })
    .save(&png)
    .unwrap();
    let ok = |s: &mut Server, id: u64, name: &str, args: Value| {
        let r = call(s, id, name, args);
        assert_eq!(r["isError"], false, "{name}: {r}");
        r
    };
    ok(&mut s, 1, "open_file", json!({"path": png.to_str().unwrap()}));
    ok(&mut s, 2, "run_command", json!({"command": "select.all"}));
    ok(&mut s, 3, "run_command", json!({"command": "imageTrace.make", "params": {"params": {"ignoreWhite": true}}}));
    let pixels = |r: &Value| {
        let data = r["content"].as_array().unwrap().iter().find(|c| c["type"] == "image").unwrap()["data"].as_str().unwrap();
        image::load_from_memory(&vectorcraft_format::base64_decode(data).unwrap()).unwrap().to_rgba8()
    };
    let dark = |p: &image::Rgba<u8>| p[0] < 60 && p[1] < 60 && p[2] < 60;
    let white = |p: &image::Rgba<u8>| p[0] > 245 && p[1] > 245 && p[2] > 245;
    let blue = |p: &image::Rgba<u8>| p[0] < 200 && p[2] > 245;
    let (middle, beside) = ((50, 50), (10, 10));
    let renders = [("screenshot", json!({})), ("render_preview", json!({"max_side": 100}))];
    for (id, (name, args)) in (10..).zip(renders) {
        let with = |mut a: Value| {
            a["traceViews"] = json!(true);
            a
        };
        ok(&mut s, id, "run_command", json!({"command": "imageTrace.setView", "params": {"view": "sourceImage"}}));
        let (view, result) = (pixels(&ok(&mut s, id, name, with(args.clone()))), pixels(&ok(&mut s, id, name, args.clone())));
        assert_eq!((view.width(), result.width()), (100, 100), "{name}");
        assert!(blue(view.get_pixel(beside.0, beside.1)), "{name}: the source image beside the disc {:?}", view.get_pixel(beside.0, beside.1));
        assert!(white(result.get_pixel(beside.0, beside.1)), "{name}: the tracing result {:?}", result.get_pixel(beside.0, beside.1));
        assert!(dark(view.get_pixel(middle.0, middle.1)) && dark(result.get_pixel(middle.0, middle.1)), "{name}: the disc's middle");
        // Outlines: a line at the disc's right side (x = 80) and no fill in its middle.
        ok(&mut s, id, "run_command", json!({"command": "imageTrace.setView", "params": {"view": "outlines"}}));
        let (view, result) = (pixels(&ok(&mut s, id, name, with(args.clone()))), pixels(&ok(&mut s, id, name, args.clone())));
        let row: Vec<u8> = (75..=85).map(|x| view.get_pixel(x, 50)[0]).collect();
        assert!(row.iter().any(|&r| r < 128), "{name}: the disc's outline {row:?}");
        assert!(white(view.get_pixel(middle.0, middle.1)), "{name}: no fill in outlines {:?}", view.get_pixel(middle.0, middle.1));
        assert!(dark(result.get_pixel(middle.0, middle.1)), "{name}: the tracing result's fill");
        assert_eq!(call(&mut s, id, name, json!({"traceViews": "yes"}))["isError"], true, "{name}: a non-boolean traceViews");
    }
}

#[test]
fn headless_path_gesture_undo() {
    let mut s = server();
    let count = |s: &mut Server| -> u64 {
        let r = call(s, 90, "inspect_document", json!({}));
        serde_json::from_str::<Value>(&text_of(&r)).unwrap()["objects"].as_u64().unwrap()
    };
    let base = count(&mut s);
    let r = call(&mut s, 1, "draw_path", json!({"points": [[10, 10], [100, 10], [50, 90]], "closed": true, "fill": "#00ff00"}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 2, "draw_path", json!({"d": "M0 0 C 10 10 20 10 30 0", "stroke": "#000000", "strokeWidth": 2}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(count(&mut s), base + 2);

    // Drag out an ellipse with the ellipse tool.
    let r = call(
        &mut s,
        3,
        "pointer_gesture",
        json!({"tool": "ellipse", "events": [{"kind": "down", "x": 200, "y": 200}, {"kind": "drag", "x": 260, "y": 240}, {"kind": "up", "x": 300, "y": 280}]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(count(&mut s), base + 3);

    assert_eq!(call(&mut s, 4, "undo", json!({}))["isError"], false);
    assert_eq!(count(&mut s), base + 2);
    assert_eq!(call(&mut s, 5, "redo", json!({}))["isError"], false);
    assert_eq!(count(&mut s), base + 3);

    // Keyboard shortcut headless: Cmd+Z undoes.
    let r = call(&mut s, 6, "press_key", json!({"key": "Z", "mods": {"cmd": true}}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(count(&mut s), base + 2);

    // Long tail through run_command + list_commands filter.
    let r = call(&mut s, 7, "run_command", json!({"command": "select.all"}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 8, "run_command", json!({"command": "object.group"}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 9, "list_commands", json!({"filter": "group"}));
    let v: Value = serde_json::from_str(&text_of(&r)).unwrap();
    assert!(v["commands"].as_array().unwrap().iter().any(|c| c["id"] == "object.group"));
    assert!(
        v["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"].as_str().unwrap().contains("group") || c["label"].as_str().unwrap().to_lowercase().contains("group"))
    );

    // set_paint on the selection.
    let r = call(&mut s, 10, "set_paint", json!({"fill": "#123456", "strokeWidth": 4}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 11, "select_tool", json!({"tool": "pen"}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 12, "export", json!({"path": tmp("x.png").to_str().unwrap(), "scale": 0.5}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(&std::fs::read(tmp("x.png")).unwrap()[..4], b"\x89PNG");
}

/// A hover's reply comes once the tool's highlight is ready, also when the tool finds it on another
/// thread (the Shape Builder's map of a large selection).
#[test]
fn headless_hover_waits_for_the_shape_builder_map() {
    let mut h = Headless::with_document();
    let run = |h: &mut Headless, command: &str, params: Value| {
        let r = crate::call_tool(h, "run_command", &json!({"command": command, "params": params}));
        assert!(!r.is_error, "{command}: {r:?}");
    };
    for k in 0..40 {
        run(&mut h, "shape.rectangle", json!({"x": 100 + k * 10, "y": 100, "width": 20, "height": 20}));
    }
    run(&mut h, "select.all", json!({}));
    run(&mut h, "tool.select", json!({"tool": "shapeBuilder"}));
    let r = crate::call_tool(&mut h, "pointer_gesture", &json!({"events": [{"kind": "move", "x": 115, "y": 110}]}));
    assert!(!r.is_error, "{r:?}");
    assert!(!h.session.tool_waiting());
    assert!(!h.session.overlays(h.view).is_empty(), "the region under the pointer is highlighted");
}

/// inspect_document reads back the paint type's characters show, as add_text and set_paint give it.
#[test]
fn inspect_document_reports_the_paint_of_type() {
    let mut s = server();
    let r = call(&mut s, 2, "add_text", json!({"text": "HI", "x": 50, "y": 60, "size": 24, "color": "#ff0000"}));
    assert_eq!(r["isError"], false, "{r}");
    let id = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["id"].clone();
    let node = |s: &mut Server, n: u64| {
        let v: Value = serde_json::from_str(&text_of(&call(s, n, "inspect_document", json!({})))).unwrap();
        v["layers"][0]["children"].as_array().unwrap().iter().find(|c| c["id"] == id).cloned().unwrap()
    };
    let t = node(&mut s, 3);
    assert_eq!((t["fill"].as_str(), t["stroke"].as_str()), (Some("#ff0000"), Some("None")), "{t}");
    let r = call(&mut s, 4, "set_paint", json!({"fill": "#0000ff", "stroke": "#00ff00", "strokeWidth": 3, "ids": [id]}));
    assert_eq!(r["isError"], false, "{r}");
    let t = node(&mut s, 5);
    assert_eq!((t["fill"].as_str(), t["stroke"].as_str(), t["strokeWidth"].as_f64()), (Some("#0000ff"), Some("#00ff00"), Some(3.0)), "{t}");
}

/// set_paint says how many objects it painted, and says so when it only set the paint for new art.
#[test]
fn set_paint_reports_how_many_objects_it_painted() {
    let mut s = server();
    let r = call(&mut s, 2, "draw_shape", json!({"shape": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10, "fill": "#ffffff"}));
    let id = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["id"].clone();
    let fill = |s: &mut Server, n: u64| {
        let v: Value = serde_json::from_str(&text_of(&call(s, n, "inspect_document", json!({})))).unwrap();
        v["layers"][0]["children"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap()["fill"].clone()
    };
    let paint = |s: &mut Server, n: u64, args: Value| {
        let r = call(s, n, "set_paint", args);
        assert_eq!(r["isError"], false, "{r}");
        serde_json::from_str::<Value>(&text_of(&r)).unwrap()
    };

    // Nothing selected and no ids: only the paint for new art changes.
    let r = call(&mut s, 3, "run_command", json!({"command": "select.set", "params": {"ids": []}}));
    assert_eq!(r["isError"], false, "{r}");
    let v = paint(&mut s, 4, json!({"fill": "#ff0000"}));
    assert_eq!(v["applied"], json!({"fill": "#ff0000"}), "{v}");
    assert_eq!(v["objects"], 0, "{v}");
    assert!(v["note"].as_str().is_some_and(|n| n.contains("new art")), "{v}");
    assert_eq!(fill(&mut s, 5), "#ffffff");

    // The ids given, counted once each.
    let v = paint(&mut s, 6, json!({"fill": "#00ff00", "ids": [id, id]}));
    assert_eq!(v["objects"], 1, "{v}");
    assert!(v.get("note").is_none(), "{v}");
    assert_eq!(fill(&mut s, 7), "#00ff00");

    // The selection.
    let r = call(&mut s, 8, "run_command", json!({"command": "select.set", "params": {"ids": [id]}}));
    assert_eq!(r["isError"], false, "{r}");
    let v = paint(&mut s, 9, json!({"fill": "#0000ff"}));
    assert_eq!(v["objects"], 1, "{v}");
    assert!(v.get("note").is_none(), "{v}");
    assert_eq!(fill(&mut s, 10), "#0000ff");

    // Empty `ids` paint no object: only the paint for new art changes.
    let v = paint(&mut s, 11, json!({"strokeWidth": 3, "ids": []}));
    assert_eq!(v["objects"], 0, "{v}");
    assert!(v["note"].as_str().is_some_and(|n| n.contains("new art")), "{v}");

    // A selected group counts the objects inside it.
    let r = call(&mut s, 12, "draw_shape", json!({"shape": "ellipse", "x": 20, "y": 0, "width": 10, "height": 10}));
    assert_eq!(r["isError"], false, "{r}");
    for (n, command) in [(13, "select.all"), (14, "object.group")] {
        let r = call(&mut s, n, "run_command", json!({"command": command}));
        assert_eq!(r["isError"], false, "{r}");
    }
    let v = paint(&mut s, 15, json!({"stroke": "#000000"}));
    assert_eq!(v["objects"], 2, "{v}");
    let v = paint(&mut s, 16, json!({"strokeWidth": 2}));
    assert_eq!(v["objects"], 2, "{v}");
}

/// #1003: type made on or in a path takes add_text's `color` too, over the current fill.
#[test]
fn add_text_on_or_in_a_path_takes_its_color() {
    for mode in ["onPath", "area"] {
        let mut s = server();
        let r = call(&mut s, 2, "draw_shape", json!({"shape": "rectangle", "x": 40, "y": 40, "width": 180, "height": 60, "fill": "#0000ff"}));
        let path = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["id"].clone();
        let r = call(&mut s, 3, "add_text", json!({"text": "Hello", "path": path, "mode": mode, "color": "#ff0000"}));
        assert_eq!(r["isError"], false, "{r}");
        let id = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["id"].clone();
        let v: Value = serde_json::from_str(&text_of(&call(&mut s, 4, "inspect_document", json!({})))).unwrap();
        let t = v["layers"][0]["children"].as_array().unwrap().iter().find(|c| c["id"] == id).cloned().unwrap();
        assert_eq!(t["fill"].as_str(), Some("#ff0000"), "{mode}: {t}");
    }
}

/// #1748: add_text's `justify` centres point type on (x, y) or ends it there.
#[test]
fn add_text_justify_anchors_point_type_on_x() {
    let mut s = server();
    let mut ids = Vec::new();
    for (n, justify) in [(2, "left"), (3, "center"), (4, "right")] {
        let r = call(&mut s, n, "add_text", json!({"text": "Title", "x": 300, "y": 100, "size": 24, "justify": justify}));
        assert_eq!(r["isError"], false, "{r}");
        ids.push(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["id"].clone());
    }
    let doc: Value = serde_json::from_str(&text_of(&call(&mut s, 5, "inspect_document", json!({})))).unwrap();
    let b = |id: &Value| {
        let t = doc["layers"][0]["children"].as_array().unwrap().iter().find(|c| &c["id"] == id).unwrap();
        (t["bounds"]["x"].as_f64().unwrap(), t["bounds"]["width"].as_f64().unwrap())
    };
    let ((l, lw), (c, cw), (r, rw)) = (b(&ids[0]), b(&ids[1]), b(&ids[2]));
    assert!(lw > 10.0 && (l - 300.0).abs() < 1.0, "{l} {lw}");
    assert!((c + cw / 2.0 - 300.0).abs() < 0.5, "{c} {cw}");
    assert!((r + rw - 300.0).abs() < 1.0, "{r} {rw}");
}

/// add_text forwards writing direction to both text-creation commands, keeping horizontal as the default.
#[test]
fn add_text_sets_writing_direction_for_every_kind() {
    let tools = tool_definitions();
    let definition = tools.iter().find(|t| t["name"] == "add_text").unwrap();
    assert_eq!(definition["inputSchema"]["properties"]["vertical"]["type"], "boolean");
    for mode in ["point", "area", "pathArea", "onPath"] {
        for vertical in [None, Some(false), Some(true)] {
            let mut s = server();
            let mut args = json!({"text": "AB", "size": 24});
            if mode == "point" || mode == "area" {
                args["x"] = json!(100);
                args["y"] = json!(100);
                if mode == "area" {
                    args["width"] = json!(180);
                    args["height"] = json!(160);
                }
            } else {
                let path = call(&mut s, 1, "draw_shape", json!({"shape": "rectangle", "x": 40, "y": 40, "width": 180, "height": 160}));
                assert_eq!(path["isError"], false, "{path}");
                args["path"] = serde_json::from_str::<Value>(&text_of(&path)).unwrap()["id"].clone();
                args["mode"] = json!(if mode == "pathArea" { "area" } else { "onPath" });
            }
            if let Some(value) = vertical {
                args["vertical"] = json!(value);
            }
            let made = call(&mut s, 2, "add_text", args);
            assert_eq!(made["isError"], false, "{mode} {vertical:?}: {made}");
            let id = serde_json::from_str::<Value>(&text_of(&made)).unwrap()["id"].clone();
            let node = call(&mut s, 3, "run_command", json!({"command": "document.node", "params": {"id": id}}));
            assert_eq!(node["isError"], false, "{node}");
            let node: Value = serde_json::from_str(&text_of(&node)).unwrap();
            assert_eq!(node["kind"]["type"], "text", "{node}");
            assert_eq!(node["kind"]["vertical"].as_bool().unwrap_or(false), vertical.unwrap_or(false), "{mode}: {node}");
        }
    }
}

/// save_file says it saves in the document's own format or the one the path's extension picks.
#[test]
fn save_file_describes_the_formats_it_writes() {
    let tools = tool_definitions();
    let d = tools.iter().find(|t| t["name"] == "save_file").and_then(|t| t["description"].as_str()).unwrap();
    for ext in [".vectorcraft", ".vctemplate", ".pdf", ".svg", ".svgz", ".ai"] {
        assert!(d.contains(ext), "{ext}: {d}");
    }
    assert!(d.contains("its own format"), "{d}");
}

#[test]
fn errors_are_tool_results_not_crashes() {
    let mut s = server();
    let cases = [
        ("no_such_tool", json!({})),
        ("draw_shape", json!({"shape": "hexagon"})),
        ("draw_shape", json!({"shape": "rectangle", "x": "left"})),
        ("draw_shape", json!({})),
        ("draw_path", json!({"points": [[1]]})),
        ("draw_path", json!({})),
        ("run_command", json!({"command": "does.not.exist"})),
        ("run_command", json!({"command": "object.move", "params": 5})),
        ("run_command", json!({})),
        ("pointer_gesture", json!({"events": [{"kind": "wiggle", "x": 0, "y": 0}]})),
        ("pointer_gesture", json!({"events": []})),
        ("select_tool", json!({"tool": "laser"})),
        ("set_paint", json!({})),
        ("set_paint", json!({"fill": "#zzzzzz"})),
        ("inspect_ui", json!({})),
        ("open_panel", json!({"panel": "Layers"})),
        ("type_text", json!({"text": "hi"})),
        ("screenshot", json!({"window": true})),
        ("open_file", json!({"path": "/definitely/not/here.svg"})),
        ("export", json!({"path": "/tmp/out.dwg"})),
        ("redo", json!({})),
        ("press_key", json!({"key": "F13"})),
    ];
    for (i, (name, args)) in cases.iter().enumerate() {
        let r = call(&mut s, i as u64, name, args.clone());
        assert_eq!(r["isError"], true, "{name} {args} → {r}");
        assert!(!text_of(&r).is_empty());
    }
    // Arguments that aren't an object.
    let v = rpc(&mut s, 100, "tools/call", json!({"name": "undo", "arguments": [1, 2]}));
    assert_eq!(v["result"]["isError"], true);
    // Missing name is a protocol error.
    let v = rpc(&mut s, 101, "tools/call", json!({}));
    assert_eq!(v["error"]["code"], -32602);
    // The server still works afterwards.
    let r = call(&mut s, 102, "inspect_document", json!({}));
    assert_eq!(r["isError"], false);
}

// ---------- remote ----------

/// A fake control server: answers `document.inspect`, echoes `engine.execute`, errors otherwise.
fn fake_app() -> (String, std::thread::JoinHandle<Vec<Value>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let h = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut seen = vec![];
        // A dialog is open until `ui.dialog.cancel` closes it; then `ui.dialog.set` is refused, as in the app.
        let mut dialog_open = true;
        for line in BufReader::new(stream).lines() {
            let msg: Value = serde_json::from_str(&line.unwrap()).unwrap();
            let reply = match msg["method"].as_str().unwrap() {
                "document.inspect" => json!({"ok": true, "result": {"title": "Remote", "layers": []}}),
                "engine.execute" => json!({"ok": true, "result": {"echo": msg["params"]}}),
                "ui.inspect" => json!({"ok": true, "result": {"tool": "selection"}}),
                "ui.dialog.set" if !dialog_open => json!({"ok": false, "error": "no dialog open"}),
                m if m.starts_with("ui.dialog.") => {
                    dialog_open &= m != "ui.dialog.cancel";
                    json!({"ok": true, "result": {"echo": msg["params"]}})
                }
                _ => json!({"ok": false, "error": "nope"}),
            };
            let mut reply = reply;
            reply["id"] = msg["id"].clone();
            writeln!(out, "{reply}").unwrap();
            seen.push(msg);
        }
        seen
    });
    (addr, h)
}

#[test]
fn remote_forwards_methods() {
    let (addr, h) = fake_app();
    let mut s = Server::new(Box::new(Remote::connect(&addr).unwrap()));
    let r = call(&mut s, 1, "inspect_document", json!({}));
    assert!(text_of(&r).contains("Remote"));
    let r = call(&mut s, 2, "run_command", json!({"command": "object.group"}));
    assert!(text_of(&r).contains("object.group"), "{r}");
    let r = call(&mut s, 3, "inspect_ui", json!({}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 4, "open_panel", json!({"panel": "Layers"}));
    assert_eq!(r["isError"], true);
    assert!(text_of(&r).contains("nope"));
    drop(s);
    let seen = h.join().unwrap();
    let methods: Vec<&str> = seen.iter().map(|m| m["method"].as_str().unwrap()).collect();
    assert_eq!(methods, ["document.inspect", "engine.execute", "ui.inspect", "ui.set"]);
    assert_eq!(seen[1]["params"], json!({"command": "object.group", "params": {}}));
}

#[test]
fn answer_dialog_sets_fields_in_order_then_confirms() {
    let (addr, h) = fake_app();
    let mut s = Server::new(Box::new(Remote::connect(&addr).unwrap()));
    let set = json!([{"field": "discard", "value": true}, {"field": "note"}]);
    let r = call(&mut s, 1, "answer_dialog", json!({"set": set, "action": "confirm"}));
    assert_eq!(r["isError"], false, "{r}");
    let reply: Value = serde_json::from_str(&text_of(&r)).unwrap();
    let fields = json!([{"field": "discard", "value": true}, {"field": "note", "value": null}]);
    assert_eq!(reply, json!({"dialog": {"echo": {"fields": fields}}, "result": {"echo": {}}}));
    let r = call(&mut s, 2, "answer_dialog", json!({"action": "cancel"}));
    assert_eq!(r["isError"], false, "{r}");
    // Bad arguments are refused before anything reaches the app.
    for (i, args) in [
        json!({}),
        json!({"set": []}),
        json!({"set": {"field": "copies"}}),
        json!({"set": [{"value": 1}]}),
        json!({"set": [{"field": "copies", "value": 1}, 7]}),
        json!({"set": [{"field": "copies", "valu": 1}], "action": "confirm"}),
        json!({"set": vec![json!({"field": "copies"}); 257]}),
        json!({"action": "ok"}),
    ]
    .into_iter()
    .enumerate()
    {
        let r = call(&mut s, 10 + i as u64, "answer_dialog", args.clone());
        assert_eq!(r["isError"], true, "{args} → {r}");
    }
    drop(s);
    let seen = h.join().unwrap();
    let methods: Vec<&str> = seen.iter().map(|m| m["method"].as_str().unwrap()).collect();
    assert_eq!(methods, ["ui.dialog.set", "ui.dialog.confirm", "ui.dialog.cancel"]);
    assert_eq!(seen[0]["params"], json!({"fields": fields}));
}

/// When the app refuses the `ui.dialog.set`, the call is an error and nothing is confirmed.
#[test]
fn answer_dialog_does_not_confirm_when_the_app_refuses_the_set() {
    let (addr, h) = fake_app();
    let mut s = Server::new(Box::new(Remote::connect(&addr).unwrap()));
    let r = call(&mut s, 1, "answer_dialog", json!({"action": "cancel"}));
    assert_eq!(r["isError"], false, "{r}");
    let set = json!([{"field": "copy", "value": true}, {"field": "dy", "value": 2}]);
    let r = call(&mut s, 2, "answer_dialog", json!({"set": set, "action": "confirm"}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(text_of(&r).contains("no dialog open"), "{r}");
    drop(s);
    let seen = h.join().unwrap();
    let methods: Vec<&str> = seen.iter().map(|m| m["method"].as_str().unwrap()).collect();
    assert_eq!(methods, ["ui.dialog.cancel", "ui.dialog.set"], "one set call, refused, and no confirm");
    assert_eq!(seen[1]["params"], json!({"fields": set}));
}

#[test]
fn answer_dialog_needs_the_desktop_app() {
    let mut s = server();
    let r = call(&mut s, 1, "answer_dialog", json!({"action": "confirm"}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(text_of(&r).contains("needs the desktop app"), "{r}");
}

/// How long a fake app waits for [`Remote`] to connect or to send, and how long a test waits for
/// a connection to end or a line to arrive. The wait only ends a test whose client or app never
/// does. A machine busy with other builds can hold a thread for seconds.
const WAIT: std::time::Duration = std::time::Duration::from_secs(20);

#[test]
fn remote_does_not_replay_when_edit_reply_is_lost() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let h = std::thread::spawn(move || {
        let timeout = WAIT;
        let mut backend = Headless::with_document();
        let base = backend.call("document.inspect", json!({})).unwrap()["objects"].as_u64().unwrap();
        for connection in 0..2 {
            let deadline = std::time::Instant::now() + timeout;
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::yield_now(),
                    Err(e) => panic!("accept failed: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(timeout)).unwrap();
            stream.set_write_timeout(Some(timeout)).unwrap();
            let mut out = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            for _ in 0..2 {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let msg: Value = serde_json::from_str(&line).unwrap();
                let method = msg["method"].as_str().unwrap();
                let result = backend.call(method, msg["params"].clone()).unwrap();
                if connection == 0 {
                    assert_eq!(method, "engine.execute");
                    break;
                }
                let reply = json!({"id": msg["id"], "ok": true, "result": result});
                writeln!(out, "{reply}").unwrap();
                if method == "document.inspect" {
                    return result["objects"].as_u64().unwrap() - base;
                }
                assert_eq!(method, "engine.execute");
            }
        }
        panic!("document.inspect was not received");
    });

    let mut remote = Remote::connect(&addr).unwrap();
    let first = remote.call("engine.execute", json!({"command": "shape.rectangle", "params": {"x": 10, "y": 10, "width": 20, "height": 20}}));
    let next = remote.call("document.inspect", json!({}));
    let edits = h.join().unwrap();

    assert_eq!(edits, 1, "a lost reply must not duplicate the edit");
    let error = first.expect_err("the lost reply must report uncertainty");
    assert!(error.contains("operation may have completed"), "{error}");
    assert!(next.is_ok(), "the next call must reconnect: {next:?}");
}

#[test]
fn remote_file_tools_send_absolute_paths() {
    let (addr, h) = fake_app();
    let mut s = Server::new(Box::new(Remote::connect(&addr).unwrap()));
    let given = std::env::temp_dir().join("given.svg");
    let given = given.to_str().unwrap();
    call(&mut s, 1, "open_file", json!({"path": "rel/in.svg"}));
    call(&mut s, 2, "open_file", json!({"path": given}));
    call(&mut s, 3, "save_file", json!({"path": "rel/out.vectorcraft"}));
    call(&mut s, 4, "export", json!({"path": "rel/out.png", "scale": 2}));
    call(&mut s, 5, "export", json!({"format": "svg", "options": {"path": "rel/opt.svg"}}));
    call(&mut s, 6, "screenshot", json!({"path": "rel/shot.png"}));
    call(&mut s, 7, "run_command", json!({"command": "document.export", "params": {"path": "rel/cmd.svg"}}));
    drop(s);
    let seen = h.join().unwrap();
    let abs = |p: &str| std::path::absolute(p).unwrap().to_str().unwrap().to_string();
    let methods: Vec<&str> = seen.iter().map(|m| m["method"].as_str().unwrap()).collect();
    assert_eq!(methods, ["app.open", "app.open", "app.save", "engine.execute", "engine.execute", "ui.render", "engine.execute"]);
    assert_eq!(seen[0]["params"]["path"], abs("rel/in.svg"));
    assert_eq!(seen[1]["params"]["path"], given);
    assert_eq!(seen[2]["params"]["path"], abs("rel/out.vectorcraft"));
    assert_eq!(seen[3]["params"]["params"]["path"], abs("rel/out.png"));
    // The format still comes from the extension the agent gave: a raster export keeps `scale`.
    assert_eq!(seen[3]["params"]["params"]["scale"], 2);
    assert_eq!(seen[4]["params"]["params"]["path"], abs("rel/opt.svg"));
    assert_eq!(seen[5]["params"]["path"], abs("rel/shot.png"));
    // Command params go to the app as given.
    assert_eq!(seen[6]["params"]["params"]["path"], "rel/cmd.svg");
}

/// `p` written relative to this process's working directory, or None when the two are on
/// different drives.
fn relative_to_cwd(p: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let cwd = std::env::current_dir().unwrap();
    if cwd.components().next() != p.components().next() {
        return None;
    }
    let mut rel = std::path::PathBuf::new();
    for _ in cwd.components().filter(|c| matches!(c, Component::Normal(_))) {
        rel.push("..");
    }
    for c in p.components().filter(|c| matches!(c, Component::Normal(_))) {
        rel.push(c);
    }
    Some(rel)
}

#[test]
fn headless_file_tools_take_relative_paths() {
    let mut s = server();
    let file = tmp("relative.vectorcraft");
    let Some(rel) = relative_to_cwd(&file) else { return };
    let rel = rel.to_str().unwrap();
    assert!(std::path::Path::new(rel).is_relative());
    let r = call(&mut s, 1, "save_file", json!({"path": rel}));
    assert_eq!(r["isError"], false, "{r}");
    assert!(file.is_file());
    let r = call(&mut s, 2, "open_file", json!({"path": rel}));
    assert_eq!(r["isError"], false, "{r}");
}

#[test]
fn headless_file_tools_send_paths_as_given() {
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Recording(Rc<RefCell<Vec<Value>>>);
    impl Backend for Recording {
        fn call(&mut self, _: &str, params: Value) -> Result<Value, String> {
            self.0.borrow_mut().push(params);
            Ok(json!({}))
        }
        fn has_ui(&self) -> bool {
            false
        }
        fn describe(&self) -> String {
            "test".into()
        }
    }

    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut s = Server::new(Box::new(Recording(seen.clone())));
    call(&mut s, 1, "open_file", json!({"path": "rel/in.svg"}));
    call(&mut s, 2, "save_file", json!({"path": "rel/out.vectorcraft"}));
    call(&mut s, 3, "export", json!({"path": "rel/out.png"}));
    call(&mut s, 4, "export", json!({"format": "svg", "options": {"path": "rel/opt.svg"}}));
    call(&mut s, 5, "screenshot", json!({"path": "rel/shot.png"}));
    let seen = seen.borrow();
    assert_eq!(seen.len(), 5, "{seen:?}");
    assert_eq!(seen[0]["path"], "rel/in.svg");
    assert_eq!(seen[1]["path"], "rel/out.vectorcraft");
    assert_eq!(seen[2]["params"]["path"], "rel/out.png");
    assert_eq!(seen[3]["params"]["path"], "rel/opt.svg");
    assert_eq!(seen[4]["path"], "rel/shot.png");
}

#[test]
fn remote_rejects_uncorrelated_replies_without_replaying() {
    for reply in [
        json!({"ok": true, "result": {}}).to_string(),
        json!({"id": null, "ok": true, "result": {}}).to_string(),
        json!({"id": "1", "ok": true, "result": {}}).to_string(),
        json!({"id": 2, "ok": true, "result": {}}).to_string(),
        json!({"id": 2, "ok": false, "error": "unrelated error"}).to_string(),
        "not json".into(),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let h = std::thread::spawn(move || {
            let timeout = WAIT;
            let mut seen = vec![];
            for connection in 0..2 {
                let deadline = std::time::Instant::now() + timeout;
                let stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::yield_now(),
                        Err(e) => panic!("accept failed: {e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(timeout)).unwrap();
                stream.set_write_timeout(Some(timeout)).unwrap();
                let mut out = stream.try_clone().unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let msg: Value = serde_json::from_str(&line).unwrap();
                seen.push(msg.clone());
                if connection == 0 {
                    writeln!(out, "{reply}").unwrap();
                    line.clear();
                    assert_eq!(reader.read_line(&mut line).unwrap(), 0, "invalid reply must close the connection without another request");
                } else {
                    writeln!(out, "{}", json!({"id": msg["id"], "ok": true, "result": {"objects": 1}})).unwrap();
                }
            }
            seen
        });
        let mut remote = Remote::connect(&addr).unwrap();
        let first = remote.call("engine.execute", json!({"command": "shape.rectangle", "params": {"width": 20, "height": 20}}));
        let next = remote.call("document.inspect", json!({}));
        drop(remote);
        let seen = h.join().unwrap();
        let error = first.expect_err("an uncorrelated reply cannot confirm the edit");
        assert!(error.contains("operation may have completed"), "{error}");
        assert_eq!(next.unwrap(), json!({"objects": 1}));
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["method"], "engine.execute");
        assert_eq!(seen[1]["method"], "document.inspect", "the edit must not be replayed");
        assert_eq!(seen[0]["id"], 1);
        assert_eq!(seen[1]["id"], 2);
    }
}

#[test]
fn remote_rejects_a_reply_with_a_mismatched_id_and_reconnects() {
    // A fake control server that answers the first request with another call's id, then serves
    // correctly: the client must reject the stray reply, drop the connection, and reconnect.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let h = std::thread::spawn(move || {
        let mut seen = vec![];
        for stream in listener.incoming().flatten().take(2) {
            stream.set_read_timeout(Some(WAIT)).unwrap();
            let mut out = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                continue;
            }
            let msg: Value = serde_json::from_str(&line).unwrap();
            // First connection: somebody else's answer. Later ones: the right echo.
            let id = if seen.is_empty() { json!(msg["id"].as_u64().unwrap_or(0) + 1000) } else { msg["id"].clone() };
            let reply = json!({"ok": true, "result": {"echo": msg["params"]}, "id": id});
            writeln!(out, "{reply}").unwrap();
            seen.push(msg);
        }
        seen
    });
    let mut remote = Remote::connect(&addr).unwrap();
    let err = remote.call("engine.execute", json!({"command": "shape.rectangle", "params": {}})).expect_err("a stray reply must not be trusted");
    assert!(err.contains("mismatch"), "{err}");
    let good = remote.call("engine.execute", json!({"command": "shape.rectangle", "params": {}}));
    assert!(good.is_ok(), "the next call reconnects: {good:?}");
    drop(remote);
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 2, "one request per connection: {seen:?}");
    assert_ne!(seen[0]["id"], seen[1]["id"]);
}

/// The app's refusals carry no id (a 17th connection gets one and is closed). The call reports the
/// app's reason, not a mismatch, says nothing ran, and the next call starts a fresh connection.
#[test]
fn remote_reports_the_reason_of_a_refusal_without_an_id() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let h = std::thread::spawn(move || {
        let (mut refused, _) = listener.accept().unwrap();
        // The request is read first, so the refusal answers it and the close is orderly.
        refused.set_read_timeout(Some(WAIT)).unwrap();
        let _ = BufReader::new(refused.try_clone().unwrap()).read_line(&mut String::new());
        writeln!(refused, "{}", json!({"ok": false, "error": "too many control connections"})).unwrap();
        drop(refused);
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(WAIT)).unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let msg: Value = serde_json::from_str(&line).unwrap();
        writeln!(out, "{}", json!({"ok": true, "result": 7, "id": msg["id"]})).unwrap();
    });
    let mut remote = Remote::connect(&addr).unwrap();
    let err = remote.call("document.inspect", json!({})).expect_err("a refusal is an error");
    assert!(err.contains("too many control connections") && err.contains("nothing was run"), "{err}");
    assert!(!err.contains("mismatch") && !err.contains("may have completed"), "{err}");
    assert_eq!(remote.call("document.inspect", json!({})), Ok(json!(7)), "the next call reconnects");
    h.join().unwrap();
}

/// How long a refused connect to the app may take: longer than the 800 ms connect timeout of
/// [`Remote`], and shorter than the 2 s a refused local connect takes on Windows without one.
const FAILED_CONNECT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Runs `attempt` until one run takes less than `bound`, at most five times, and returns how long
/// each run took. A machine busy with other builds can hold the test's thread for seconds in one
/// run; a connect that waits longer than its own timeout is slow in every run.
fn runs_until_faster_than(bound: std::time::Duration, mut attempt: impl FnMut()) -> Vec<std::time::Duration> {
    let mut took = Vec::new();
    while took.len() < 5 {
        let start = std::time::Instant::now();
        attempt();
        took.push(start.elapsed());
        if took.last() < Some(&bound) {
            break;
        }
    }
    took
}

#[test]
fn remote_connect_fails_fast() {
    // A port this test holds, bound but not listening, so connecting to it is refused. (A port
    // bound and dropped could be taken by another process or test before the connect.)
    let held = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    held.bind(&std::net::SocketAddr::from(([127, 0, 0, 1], 0)).into()).unwrap();
    let port = held.local_addr().unwrap().as_socket().unwrap().port();
    let took = runs_until_faster_than(FAILED_CONNECT, || assert!(Remote::connect(&format!("127.0.0.1:{port}")).is_err()));
    assert!(took.last() < Some(&FAILED_CONNECT), "took {took:?}");
    // Nothing else could listen there meanwhile.
    assert!(std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_err());
    drop(held);
}

fn reusable_listener(addr: std::net::SocketAddr) -> std::net::TcpListener {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    socket.set_reuse_address(true).unwrap();
    socket.bind(&addr.into()).unwrap();
    socket.listen(1).unwrap();
    socket.into()
}

fn wait_for_remote_eof(remote: &Remote) {
    let probe = remote.connection_probe();
    probe.set_read_timeout(Some(WAIT)).unwrap();
    assert_eq!(probe.peek(&mut [0]).unwrap(), 0);
}

#[test]
fn remote_edit_succeeds_once_when_app_restarts() {
    let listener = reusable_listener(std::net::SocketAddr::from(([127, 0, 0, 1], 0)));
    let addr = listener.local_addr().unwrap();
    let mut remote = Remote::connect(&addr.to_string()).unwrap();
    let (old_connection, _) = listener.accept().unwrap();
    let old_handle = std::thread::spawn(move || {
        let timeout = WAIT;
        old_connection.set_read_timeout(Some(timeout)).unwrap();
        old_connection.set_write_timeout(Some(timeout)).unwrap();
        let mut reader = BufReader::new(old_connection.try_clone().unwrap());
        let mut writer = old_connection;
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let message: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(message["method"], "document.inspect");
        let result = Headless::with_document().call("document.inspect", message["params"].clone()).unwrap();
        writeln!(writer, "{}", json!({"id": message["id"], "ok": true, "result": result})).unwrap();
        writer
    });
    let inspected = remote.call("document.inspect", json!({})).unwrap();
    assert!(inspected["objects"].as_u64().unwrap() > 0);
    drop(old_handle.join().unwrap());
    drop(listener);
    wait_for_remote_eof(&remote);

    let listener = reusable_listener(addr);
    listener.set_nonblocking(true).unwrap();
    let handle = std::thread::spawn(move || {
        let timeout = WAIT;
        let deadline = std::time::Instant::now() + timeout;
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return (0, 0);
                    }
                    std::thread::yield_now();
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream.set_read_timeout(Some(timeout)).unwrap();
        stream.set_write_timeout(Some(timeout)).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut backend = Headless::with_document();
        let base = backend.call("document.inspect", json!({})).unwrap()["objects"].as_u64().unwrap();
        let mut requests = 0;
        for line in BufReader::new(stream).lines() {
            let message: Value = serde_json::from_str(&line.unwrap()).unwrap();
            let result = backend.call(message["method"].as_str().unwrap(), message["params"].clone()).unwrap();
            requests += 1;
            writeln!(writer, "{}", json!({"id": message["id"], "ok": true, "result": result})).unwrap();
        }
        let objects = backend.call("document.inspect", json!({})).unwrap()["objects"].as_u64().unwrap();
        (requests, objects - base)
    });

    let edit = remote.call("engine.execute", json!({"command": "shape.rectangle", "params": {"x": 10, "y": 10, "width": 20, "height": 20}}));
    drop(remote);
    let (requests, edits) = handle.join().unwrap();

    assert!(edit.is_ok(), "the first edit after restart must succeed: {edit:?}");
    assert_eq!((requests, edits), (1, 1), "the edit must be sent and applied exactly once");
}

/// A connection holding a line nobody asked for (an app that says why it closes an idle
/// connection) is not used: the call goes out once, on a fresh connection, and succeeds.
#[test]
fn remote_does_not_send_on_a_connection_with_a_stray_line() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let mut remote = Remote::connect(&addr).unwrap();
    let (mut stale, _) = listener.accept().unwrap();
    stale.set_read_timeout(Some(WAIT)).unwrap();
    writeln!(stale, "{}", json!({"ok": false, "error": "idle"})).unwrap();
    let probe = remote.connection_probe();
    probe.set_read_timeout(Some(WAIT)).unwrap();
    assert_eq!(probe.peek(&mut [0]).unwrap(), 1, "the stray line arrived");
    drop(probe);
    let h = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(WAIT)).unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let msg: Value = serde_json::from_str(&line).unwrap();
        writeln!(out, "{}", json!({"ok": true, "result": 7, "id": msg["id"]})).unwrap();
    });
    assert_eq!(remote.call("document.inspect", json!({})), Ok(json!(7)));
    h.join().unwrap();
    // Nothing was written to the stale connection: it ends (or is reset, its line unread) without a byte.
    let mut rest = String::new();
    assert_eq!(BufReader::new(stale).read_line(&mut rest).unwrap_or(0), 0, "{rest}");
}

#[test]
fn remote_call_reports_nothing_sent_when_reconnect_is_refused() {
    // A remote whose connection the app closed, and the app's port bound but not listening.
    let closed_remote = || {
        let listener = reusable_listener(std::net::SocketAddr::from(([127, 0, 0, 1], 0)));
        let addr = listener.local_addr().unwrap();
        let remote = Remote::connect(&addr.to_string()).unwrap();
        let (old_connection, _) = listener.accept().unwrap();
        drop(old_connection);
        drop(listener);
        wait_for_remote_eof(&remote);
        let held = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
        held.set_reuse_address(true).unwrap();
        held.bind(&addr.into()).unwrap();
        (remote, held)
    };

    // Each run finds the old connection closed and connects again; only the call is timed.
    let mut errors = Vec::new();
    let mut took = Vec::new();
    while took.len() < 5 {
        let (mut remote, _held) = closed_remote();
        let start = std::time::Instant::now();
        errors.push(remote.call("engine.execute", json!({"command": "shape.rectangle", "params": {"width": 20, "height": 20}})).unwrap_err());
        took.push(start.elapsed());
        if took.last() < Some(&FAILED_CONNECT) {
            break;
        }
    }

    assert!(took.last() < Some(&FAILED_CONNECT), "took {took:?}");
    for error in errors {
        assert!(error.contains("not reachable"), "{error}");
        assert!(error.contains("nothing was sent"), "{error}");
        assert!(!error.contains("operation may have completed"), "{error}");
    }
}

#[test]
fn headless_backend_methods() {
    let mut h = Headless::new();
    assert!(h.call("document.inspect", json!({})).is_err(), "no document yet");
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 300, "height": 200}})).unwrap();
    let r = h.call("ui.render", json!({"scale": 2})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(600), Some(400)));
    let cmds = h.call("engine.commands", json!({})).unwrap();
    assert!(cmds.as_array().unwrap().iter().any(|c| c["id"] == "file.export"));
    assert!(h.call("ui.tool.list", json!({})).unwrap().is_array());
    assert!(h.call("ui.resize", json!({})).is_err());
}

// ---------- workflow tools ----------

fn id_of(result: &Value) -> u64 {
    let v: Value = serde_json::from_str(&text_of(result)).unwrap();
    v["id"].as_u64().unwrap_or_else(|| panic!("no id in {v}"))
}

fn doc_json(s: &mut Server) -> Value {
    let r = call(s, 900, "run_command", json!({"command": "document.json"}));
    serde_json::from_str(&text_of(&r)).unwrap()
}

#[test]
fn text_effect_pathfinder_transform_graph_and_wrap_tools() {
    let mut s = server();
    // Text: point, area and on a path with an effect.
    let t = id_of(&call(&mut s, 1, "add_text", json!({"text": "Hello", "x": 50, "y": 60, "size": 24})));
    assert!(t > 0);
    let area = id_of(&call(&mut s, 2, "add_text", json!({"text": "word ".repeat(80), "x": 40, "y": 100, "width": 300, "height": 200})));
    let p = call(&mut s, 3, "draw_path", json!({"d": "M40 400 C150 300 250 500 360 400"}));
    let pid = id_of(&p);
    let ot = id_of(&call(&mut s, 4, "add_text", json!({"text": "On a wave", "path": pid, "mode": "onPath", "pathEffect": "skew"})));
    let d = doc_json(&mut s);
    assert!(d.to_string().contains("\"pathEffect\":\"skew\""), "{ot}");
    // Effects: catalogue, then a drop shadow on the selection.
    let cat = call(&mut s, 5, "apply_effect", json!({}));
    assert!(text_of(&cat).contains("stylize.dropShadow"));
    let r1 = id_of(&call(&mut s, 6, "draw_shape", json!({"shape": "rectangle", "x": 400, "y": 50, "width": 100, "height": 100})));
    let r2 = id_of(&call(&mut s, 7, "draw_shape", json!({"shape": "rectangle", "x": 450, "y": 100, "width": 100, "height": 100})));
    let fx = call(&mut s, 8, "apply_effect", json!({"effect": "stylize.dropShadow", "params": {"x": 4}, "ids": [r2]}));
    assert!(fx.get("isError").is_none_or(|e| e == false), "{fx}");
    // Pathfinder unite over two ids.
    let u = call(&mut s, 9, "pathfinder", json!({"operation": "unite", "ids": [r1, r2]}));
    assert!(text_of(&u).contains("ids"), "{u}");
    // Transform: move + rotate on the selection.
    let tr = call(&mut s, 10, "transform", json!({"dx": 10, "rotate": 45}));
    assert!(text_of(&tr).contains("rotate"));
    let bad = call(&mut s, 11, "transform", json!({}));
    assert_eq!(bad["isError"], true);
    // Graph.
    let g = id_of(&call(&mut s, 12, "create_graph", json!({"type": "pie", "x": 50, "y": 450, "width": 200, "height": 150, "csv": ",A,B\nX,1,3"})));
    assert!(doc_json(&mut s).to_string().contains("\"kind\":\"pie\""), "{g}");
    // Text wrap: a circle over the area type.
    let c = id_of(&call(&mut s, 13, "draw_shape", json!({"shape": "ellipse", "x": 40, "y": 120, "width": 120, "height": 120})));
    let w = call(&mut s, 14, "text_wrap", json!({"ids": [c], "offset": 8}));
    assert!(w.get("isError").is_none_or(|e| e == false), "{w}");
    let d = doc_json(&mut s);
    let ds = d.to_string();
    // The wrap object carries the options and the area type resolved its shape.
    assert!(ds.contains("\"wrap\":{\"invert\":false,\"offset\":8.0}") && ds.contains("\"wrap\":[{\"invert\":false,\"offset\":8.0"), "{area}");
    call(&mut s, 15, "text_wrap", json!({"ids": [c], "release": true}));
    assert!(!doc_json(&mut s).to_string().contains("\"offset\":8.0"));
}

#[test]
fn oversized_screenshot_and_png_export_are_errors() {
    // 16383 pt at the maximum scale (16) is 262 128 px a side: an error result, not a crashed server.
    let mut s = server();
    let r = call(&mut s, 1, "run_command", json!({"command": "file.new", "params": {"width": 16383, "height": 16383}}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 2, "screenshot", json!({"scale": 16}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(text_of(&r).contains("pixels"), "{r}");
    let path = tmp("huge.png");
    let r = call(&mut s, 3, "export", json!({"path": path.to_str().unwrap(), "scale": 16}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(!path.exists());
    // The session is still alive and a small screenshot works.
    let r = call(&mut s, 4, "screenshot", json!({"scale": 0.01}));
    assert_eq!(r["isError"], false, "{r}");
}

/// Preferences reach agents' gestures (#394): `prefs.set` Object Selection by Path Only, then a
/// click inside a filled square selects nothing; Command Click to Select Objects Behind, a
/// Cmd-click selects the square underneath.
#[test]
fn selection_preferences_apply_to_pointer_gestures() {
    let mut s = server();
    let square = |s: &mut Server, id: u64, x: u64| {
        let r = call(s, id, "draw_shape", json!({"shape": "rectangle", "x": x, "y": 100, "width": 100, "height": 100, "fill": "#ff0000"}));
        serde_json::from_str::<Value>(&text_of(&r)).unwrap()["id"].as_u64().unwrap()
    };
    let (back, front) = (square(&mut s, 1, 100), square(&mut s, 2, 150));
    let click = |s: &mut Server, id: u64, x: f64, mods: Value| {
        let events = json!([{"kind": "down", "x": x, "y": 125}, {"kind": "up", "x": x, "y": 125}]);
        let r = call(s, id, "pointer_gesture", json!({"tool": "selection", "events": events, "mods": mods}));
        assert_eq!(r["isError"], false, "{r}");
        serde_json::from_str::<Value>(&text_of(&r)).unwrap()["selection"].clone()
    };
    assert_eq!(click(&mut s, 3, 175.0, json!({})), json!([front]));
    assert_eq!(click(&mut s, 4, 175.0, json!({"cmd": true})), json!([back]), "Cmd-click selects behind");
    let r = call(&mut s, 5, "run_command", json!({"command": "prefs.set", "params": {"key": "objectSelectionByPathOnly", "value": true}}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(click(&mut s, 6, 225.0, json!({})), json!([]), "path only: the fill doesn't select");
    assert_eq!(click(&mut s, 7, 250.0, json!({})), json!([front]), "the path does");
}

#[test]
fn pointer_gesture_forwards_backquote_on_each_event() {
    let mut s = server();
    let made = call(&mut s, 1, "draw_shape", json!({"shape": "rectangle", "x": 100, "y": 100, "width": 50, "height": 50, "fill": "#ff0000"}));
    let id = serde_json::from_str::<Value>(&text_of(&made)).unwrap()["id"].as_u64().unwrap();
    let selected = call(&mut s, 2, "run_command", json!({"command": "select.set", "params": {"ids": [id]}}));
    assert_eq!(selected["isError"], false, "{selected}");
    let drag = call(
        &mut s,
        3,
        "pointer_gesture",
        json!({"tool": "selection", "backquote": true, "events": [
            {"kind": "down", "x": 125, "y": 125},
            {"kind": "drag", "x": 145, "y": 125},
            {"kind": "up", "x": 145, "y": 125}
        ]}),
    );
    assert_eq!(drag["isError"], false, "{drag}");
    let inspected = call(&mut s, 4, "inspect_document", json!({}));
    let doc = serde_json::from_str::<Value>(&text_of(&inspected)).unwrap();
    let rect = doc["layers"][0]["children"].as_array().unwrap().iter().find(|n| n["id"] == id).unwrap();
    assert_eq!(rect["bounds"]["x"], 100.0, "{doc}");
}

/// A Shift-drag marquee reaches agents (#483): `pointer_gesture` with `mods.shift` toggles the
/// objects it reaches, so a selected one leaves the selection and an unselected one joins it.
#[test]
fn shift_marquee_gesture_toggles_the_selection() {
    let mut s = server();
    let square = |s: &mut Server, id: u64, x: u64| {
        let r = call(s, id, "draw_shape", json!({"shape": "rectangle", "x": x, "y": 100, "width": 50, "height": 50}));
        serde_json::from_str::<Value>(&text_of(&r)).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b, c) = (square(&mut s, 1, 100), square(&mut s, 2, 200), square(&mut s, 3, 300));
    let r = call(&mut s, 4, "run_command", json!({"command": "select.set", "params": {"ids": [a, b]}}));
    assert_eq!(r["isError"], false, "{r}");
    let events = json!([{"kind": "down", "x": 180, "y": 80}, {"kind": "drag", "x": 300, "y": 200}, {"kind": "up", "x": 380, "y": 200}]);
    for tool in ["selection", "directSelection"] {
        let r = call(&mut s, 5, "pointer_gesture", json!({"tool": tool, "events": events, "mods": {"shift": true}}));
        assert_eq!(r["isError"], false, "{r}");
        let sel = &serde_json::from_str::<Value>(&text_of(&r)).unwrap()["selection"];
        // The Selection tool takes b out and adds c; Direct Selection's marquee then toggles their
        // anchors back: b's are selected again and c, whole, leaves.
        let want = if tool == "selection" { json!([a, c]) } else { json!([a, b]) };
        assert_eq!(*sel, want, "{tool}");
    }
}

/// `press_key {key: "e"}` while a Selection marquee is being dragged makes the marquee select only
/// the objects entirely inside it. With nothing dragged, E still chooses its tool.
#[test]
fn e_during_a_marquee_gesture_selects_only_the_objects_inside() {
    let mut s = server();
    let square = |s: &mut Server, id: u64, x: u64| {
        let r = call(s, id, "draw_shape", json!({"shape": "rectangle", "x": x, "y": 100, "width": 50, "height": 50}));
        serde_json::from_str::<Value>(&text_of(&r)).unwrap()["id"].as_u64().unwrap()
    };
    let (a, _b) = (square(&mut s, 1, 100), square(&mut s, 2, 200));
    let begin = json!([{"kind": "down", "x": 80, "y": 80}, {"kind": "drag", "x": 220, "y": 170}]);
    let r = call(&mut s, 3, "pointer_gesture", json!({"tool": "selection", "events": begin}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 4, "press_key", json!({"key": "e"}));
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["handledBy"], "tool", "{r}");
    let end = json!([{"kind": "drag", "x": 230, "y": 170}, {"kind": "up", "x": 230, "y": 170}]);
    let r = call(&mut s, 5, "pointer_gesture", json!({"tool": "selection", "events": end}));
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["selection"], json!([a]), "{r}");
    let r = call(&mut s, 6, "press_key", json!({"key": "e"}));
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["handledBy"], "tool.select", "{r}");
}

/// `press_key {key: "c"}` and `{key: "f"}` while an arc is being dragged go to the Arc tool, and
/// the release draws one arc. With nothing dragged, C is a shortcut again.
#[test]
fn c_and_f_during_an_arc_gesture_go_to_the_arc_tool() {
    let mut s = server();
    let count = |s: &mut Server, id: u64| {
        let r = call(s, id, "inspect_document", json!({}));
        serde_json::from_str::<Value>(&text_of(&r)).unwrap()["objects"].as_u64().unwrap()
    };
    let before = count(&mut s, 1);
    let begin = json!([{"kind": "down", "x": 100, "y": 100}, {"kind": "drag", "x": 200, "y": 180}]);
    let r = call(&mut s, 2, "pointer_gesture", json!({"tool": "arc", "events": begin}));
    assert_eq!(r["isError"], false, "{r}");
    for (id, key) in [(3, "c"), (4, "f")] {
        let r = call(&mut s, id, "press_key", json!({"key": key}));
        assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["handledBy"], "tool", "{key}: {r}");
    }
    let r = call(&mut s, 5, "pointer_gesture", json!({"tool": "arc", "events": [{"kind": "up", "x": 200, "y": 180}]}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(count(&mut s, 6), before + 1);
    let r = call(&mut s, 7, "press_key", json!({"key": "c"}));
    assert_ne!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["handledBy"], "tool", "{r}");
}

/// A click on one of two selected objects makes it the key object: `pointer_gesture` replies
/// with it as `key` and `inspect_document` lists it, and a click on the key again lets it go.
#[test]
fn pointer_gestures_report_the_key_object() {
    let mut s = server();
    let square = |s: &mut Server, id: u64, x: u64| {
        let r = call(s, id, "draw_shape", json!({"shape": "rectangle", "x": x, "y": 100, "width": 50, "height": 50, "fill": "#ff0000"}));
        serde_json::from_str::<Value>(&text_of(&r)).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (square(&mut s, 1, 100), square(&mut s, 2, 200));
    let r = call(&mut s, 3, "run_command", json!({"command": "select.set", "params": {"ids": [a, b]}}));
    assert_eq!(r["isError"], false, "{r}");
    let click = |s: &mut Server, id: u64| {
        let events = json!([{"kind": "down", "x": 225, "y": 125}, {"kind": "up", "x": 225, "y": 125}]);
        let r = call(s, id, "pointer_gesture", json!({"tool": "selection", "events": events}));
        assert_eq!(r["isError"], false, "{r}");
        serde_json::from_str::<Value>(&text_of(&r)).unwrap().get("key").cloned()
    };
    assert_eq!(click(&mut s, 4), Some(json!(b)), "a click on a selected object makes it the key");
    let doc: Value = serde_json::from_str(&text_of(&call(&mut s, 5, "inspect_document", json!({})))).unwrap();
    assert_eq!(doc.get("key"), Some(&json!(b)));
    assert_eq!(click(&mut s, 6), Some(Value::Null), "a click on the key again lets it go");
}

/// Preferences › Type › Show Font Names in English (#394): agents set it with `prefs.set`;
/// `text.fontList` stays the English (canonical) family names used by `text.setStyle`, not the
/// native UI labels.
#[test]
fn show_font_names_in_english_does_not_change_font_list_for_agents() {
    let mut s = server();
    let r = call(&mut s, 1, "run_command", json!({"command": "prefs.set", "params": {"key": "fontNamesInEnglish", "value": false}}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 2, "run_command", json!({"command": "prefs.get", "params": {"key": "fontNamesInEnglish"}}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap(), json!(false));
    let r = call(&mut s, 3, "run_command", json!({"command": "text.fontList", "params": {}}));
    assert_eq!(r["isError"], false, "{r}");
    let families = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["families"].as_array().unwrap().clone();
    // Canonical English name for commands, whatever the UI labels show with the preference off.
    assert!(families.iter().any(|f| f.as_str() == Some("Source Sans 3")), "{families:?}");
}

/// Preferences › Type › Highlight Alternate Glyphs (#394): off by default so the canvas stays
/// clear until an agent (or the Preferences dialog) turns it on. Headless MCP has no canvas.
#[test]
fn highlight_alternate_glyphs_pref_round_trips_for_agents() {
    let mut s = server();
    let r = call(&mut s, 1, "run_command", json!({"command": "prefs.get", "params": {"key": "highlightAlternateGlyphs"}}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap(), json!(false));
    let r = call(&mut s, 2, "run_command", json!({"command": "prefs.set", "params": {"key": "highlightAlternateGlyphs", "value": true}}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 3, "run_command", json!({"command": "prefs.get", "params": {"key": "highlightAlternateGlyphs"}}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap(), json!(true));
}

/// Type preferences reach agents (#394): type the Type tool places starts with placeholder text,
/// selected; Alt+→ tracks it by Tracking and Cmd+Shift+. steps its size by Size/Leading.
#[test]
fn type_preferences_apply_to_agents() {
    let mut s = server();
    let events = json!([{"kind": "down", "x": 100, "y": 100}, {"kind": "up", "x": 100, "y": 100}]);
    let r = call(&mut s, 1, "pointer_gesture", json!({"tool": "type", "events": events}));
    assert_eq!(r["isError"], false, "{r}");
    let id = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["selection"][0].as_u64().unwrap();
    let style = |s: &mut Server, n: u64| {
        let r = call(s, n, "run_command", json!({"command": "text.getRange", "params": {"id": id}}));
        let v = serde_json::from_str::<Value>(&text_of(&r)).unwrap();
        assert!(v["text"].as_str().unwrap().len() > 10, "placeholder text: {v}");
        let st = &v["runs"][0]["style"];
        (st["size"].as_f64().unwrap(), st["tracking"].as_f64().unwrap())
    };
    let (size, tracking) = style(&mut s, 2);
    let r = call(&mut s, 3, "run_command", json!({"command": "prefs.set", "params": {"values": {"typeSizeIncrement": 4, "trackingIncrement": 50}}}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 4, "press_key", json!({"key": "Right", "mods": {"alt": true}}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 5, "press_key", json!({"key": ".", "mods": {"cmd": true, "shift": true}}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(style(&mut s, 6), (size + 4.0, tracking + 50.0));
}

/// Cmd+Alt+Shift+. and , reach agents: the placeholder text the Type tool selects steps by five
/// times the Size/Leading increment (2 pt by default) and back.
#[test]
fn cmd_alt_shift_period_steps_the_font_size_by_five_for_agents() {
    let mut s = server();
    let events = json!([{"kind": "down", "x": 100, "y": 100}, {"kind": "up", "x": 100, "y": 100}]);
    let r = call(&mut s, 1, "pointer_gesture", json!({"tool": "type", "events": events}));
    assert_eq!(r["isError"], false, "{r}");
    let id = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["selection"][0].as_u64().unwrap();
    let r = call(&mut s, 2, "run_command", json!({"command": "text.setRangeStyle", "params": {"id": id, "size": 12}}));
    assert_eq!(r["isError"], false, "{r}");
    let size = |s: &mut Server, n| {
        let r = call(s, n, "run_command", json!({"command": "text.getRange", "params": {"id": id, "effective": true}}));
        serde_json::from_str::<Value>(&text_of(&r)).unwrap()["runs"][0]["style"]["size"].clone()
    };
    let r = call(&mut s, 3, "press_key", json!({"key": ".", "mods": {"cmd": true, "alt": true, "shift": true}}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["command"], "type.size.increaseByFive", "{r}");
    assert_eq!(size(&mut s, 4), json!(22.0));
    let r = call(&mut s, 5, "press_key", json!({"key": ",", "mods": {"cmd": true, "alt": true, "shift": true}}));
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["command"], "type.size.decreaseByFive", "{r}");
    assert_eq!(size(&mut s, 6), json!(12.0));
}

/// Cmd+Alt+Q reaches agents: the placeholder text the Type tool selects goes back to tracking 0
/// and Auto kerning.
#[test]
fn cmd_alt_q_resets_tracking_and_kerning_for_agents() {
    let mut s = server();
    let events = json!([{"kind": "down", "x": 100, "y": 100}, {"kind": "up", "x": 100, "y": 100}]);
    let r = call(&mut s, 1, "pointer_gesture", json!({"tool": "type", "events": events}));
    assert_eq!(r["isError"], false, "{r}");
    let id = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["selection"][0].as_u64().unwrap();
    let r = call(&mut s, 2, "run_command", json!({"command": "text.setRangeStyle", "params": {"id": id, "tracking": 200, "kerning": -40}}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 3, "press_key", json!({"key": "Q", "mods": {"cmd": true, "alt": true}}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["command"], "type.resetTracking", "{r}");
    let r = call(&mut s, 4, "run_command", json!({"command": "text.getRange", "params": {"id": id}}));
    let runs = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["runs"].clone();
    assert_eq!(runs.as_array().unwrap().len(), 1, "{runs}");
    assert_eq!((&runs[0]["style"]["tracking"], &runs[0]["style"]["kerning"]), (&json!(0.0), &Value::Null), "{runs}");
}

/// Cmd+Shift+X reaches agents: the placeholder text the Type tool selects goes back to a
/// horizontal scale of 100%.
#[test]
fn cmd_shift_x_resets_horizontal_scale_for_agents() {
    let mut s = server();
    let events = json!([{"kind": "down", "x": 100, "y": 100}, {"kind": "up", "x": 100, "y": 100}]);
    let r = call(&mut s, 1, "pointer_gesture", json!({"tool": "type", "events": events}));
    assert_eq!(r["isError"], false, "{r}");
    let id = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["selection"][0].as_u64().unwrap();
    let r = call(&mut s, 2, "run_command", json!({"command": "text.setRangeStyle", "params": {"id": id, "hScale": 150}}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 3, "press_key", json!({"key": "X", "mods": {"cmd": true, "shift": true}}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["command"], "type.resetHorizontalScale", "{r}");
    let r = call(&mut s, 4, "run_command", json!({"command": "text.getRange", "params": {"id": id}}));
    let runs = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["runs"].clone();
    assert_eq!(runs.as_array().unwrap().len(), 1, "{runs}");
    assert_eq!(runs[0]["style"]["h_scale"], json!(100.0), "{runs}");
}

/// `press_key` runs Cmd+/ and Cmd+Alt+/ for agents: a new fill and then a new stroke go on top of
/// the selected rectangle's appearance stack.
#[test]
fn cmd_slash_and_cmd_alt_slash_add_a_fill_and_a_stroke_for_agents() {
    let mut s = server();
    let r = call(&mut s, 1, "run_command", json!({"command": "shape.rectangle", "params": {"x": 10, "y": 10, "width": 100, "height": 100}}));
    let id = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["id"].as_u64().unwrap();
    let r = call(&mut s, 2, "run_command", json!({"command": "select.set", "params": {"ids": [id]}}));
    assert_eq!(r["isError"], false, "{r}");
    let kinds = |s: &mut Server, n: u64| {
        let r = call(s, n, "run_command", json!({"command": "effect.list", "params": {}}));
        let items = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["applied"][0]["items"].clone();
        items.as_array().unwrap().iter().map(|it| it["kind"].as_str().unwrap().to_string()).collect::<Vec<_>>()
    };
    assert_eq!(kinds(&mut s, 3), ["fill", "stroke"]);
    let r = call(&mut s, 4, "press_key", json!({"key": "/", "mods": {"cmd": true}}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["command"], "appearance.addFill", "{r}");
    assert_eq!(kinds(&mut s, 5), ["fill", "stroke", "fill"]);
    let r = call(&mut s, 6, "press_key", json!({"key": "/", "mods": {"cmd": true, "alt": true}}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["command"], "appearance.addStroke", "{r}");
    assert_eq!(kinds(&mut s, 7), ["fill", "stroke", "fill", "stroke"]);
}

/// The drawing tools snap to Smart Guides over MCP as with the mouse (#506): a Rectangle drawn
/// from near another object's corner starts on it, and a Pen anchor placed beside that object's
/// centre lines up with it.
#[test]
fn drawing_gestures_snap_to_smart_guides() {
    let mut s = server();
    let r = call(&mut s, 1, "run_command", json!({"command": "file.new", "params": {"width": 800, "height": 600}}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, 2, "draw_shape", json!({"shape": "rectangle", "x": 100, "y": 100, "width": 100, "height": 100}));
    assert_eq!(r["isError"], false, "{r}");
    let bounds = |s: &mut Server, n: u64, events: Value, tool: &str| {
        let r = call(s, n, "pointer_gesture", json!({"tool": tool, "events": events}));
        assert_eq!(r["isError"], false, "{r}");
        let id = serde_json::from_str::<Value>(&text_of(&r)).unwrap()["selection"][0].clone();
        let doc: Value = serde_json::from_str(&text_of(&call(s, n + 1, "inspect_document", json!({})))).unwrap();
        let made = doc["layers"][0]["children"].as_array().unwrap().iter().find(|c| c["id"] == id).expect("drawn").clone();
        let b = &made["bounds"];
        [&b["x"], &b["y"], &b["width"], &b["height"]].map(|v| v.as_f64().unwrap())
    };
    let events = json!([{"kind": "move", "x": 202, "y": 203}, {"kind": "down", "x": 202, "y": 203}, {"kind": "drag", "x": 330, "y": 341}, {"kind": "up", "x": 330, "y": 341}]);
    assert_eq!(bounds(&mut s, 3, events, "rectangle"), [200.0, 200.0, 130.0, 141.0]);
    let r = call(&mut s, 5, "run_command", json!({"command": "select.none", "params": {}}));
    assert_eq!(r["isError"], false, "{r}");
    let events = json!([{"kind": "down", "x": 300, "y": 420}, {"kind": "up", "x": 300, "y": 420}, {"kind": "move", "x": 151.5, "y": 431}, {"kind": "down", "x": 151.5, "y": 431}, {"kind": "up", "x": 151.5, "y": 431}]);
    assert_eq!(bounds(&mut s, 6, events, "pen"), [150.0, 420.0, 150.0, 11.0]);
}

/// #1471: add_text says when the text doesn't fit its frame, and takes area type's `fit`.
#[test]
fn add_text_reports_overflow_and_takes_a_fit() {
    let mut s = server();
    let made = |r: &Value| serde_json::from_str::<Value>(&text_of(r)).unwrap();
    let headline = json!({"text": "NEON DRIVE", "x": 0, "y": 20, "width": 400, "height": 60, "size": 60});
    let low = made(&call(&mut s, 1, "add_text", headline.clone()));
    assert_eq!(low["overflow"], true, "{low}");
    let mut auto = headline;
    auto["fit"] = json!("autoHeight");
    let auto = made(&call(&mut s, 2, "add_text", auto));
    assert!(auto["id"].is_u64() && auto.get("overflow").is_none(), "{auto}");
    let point = made(&call(&mut s, 3, "add_text", json!({"text": "NEON DRIVE", "x": 0, "y": 300, "size": 60})));
    assert!(point.get("overflow").is_none(), "{point}");
}
