use std::io::{BufRead, BufReader, Write};

use serde_json::{Value, json};

use crate::{Backend, Headless, PROTOCOL_VERSION, Remote, Server, tool_definitions};

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
        for line in BufReader::new(stream).lines() {
            let msg: Value = serde_json::from_str(&line.unwrap()).unwrap();
            let reply = match msg["method"].as_str().unwrap() {
                "document.inspect" => json!({"ok": true, "result": {"title": "Remote", "layers": []}}),
                "engine.execute" => json!({"ok": true, "result": {"echo": msg["params"]}}),
                "ui.inspect" => json!({"ok": true, "result": {"tool": "selection"}}),
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
fn remote_connect_fails_fast() {
    // A port this test holds, bound but not listening, so connecting to it is refused. (A port
    // bound and dropped could be taken by another process or test before the connect.)
    let held = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    held.bind(&std::net::SocketAddr::from(([127, 0, 0, 1], 0)).into()).unwrap();
    let port = held.local_addr().unwrap().as_socket().unwrap().port();
    let start = std::time::Instant::now();
    assert!(Remote::connect(&format!("127.0.0.1:{port}")).is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "took {:?}", start.elapsed());
    // Nothing else could listen there meanwhile.
    assert!(std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_err());
    drop(held);
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

#[test]
fn pathfinder_live_makes_a_compound_shape() {
    let mut s = server();
    let r1 = id_of(&call(&mut s, 1, "draw_shape", json!({"shape": "rectangle", "x": 0, "y": 0, "width": 100, "height": 100})));
    let r2 = id_of(&call(&mut s, 2, "draw_shape", json!({"shape": "rectangle", "x": 50, "y": 50, "width": 100, "height": 100})));
    let c = id_of(&call(&mut s, 3, "pathfinder", json!({"operation": "minusFront", "ids": [r1, r2], "live": true})));
    let d = doc_json(&mut s).to_string();
    assert!(d.contains("\"compoundShape\"") && d.contains("\"subtract\""), "{c} {d}");
    // Not a shape mode: an error.
    let e = call(&mut s, 4, "pathfinder", json!({"operation": "divide", "ids": [c], "live": true}));
    assert_eq!(e["isError"], true, "{e}");
}
