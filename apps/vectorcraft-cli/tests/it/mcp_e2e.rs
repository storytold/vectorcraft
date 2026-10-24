//! End-to-end MCP over stdio against the built `vectorcraft-cli mcp --headless` binary.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};
use vectorcraft_testkit::raster::Image;

const BIN: &str = env!("CARGO_BIN_EXE_vectorcraft-cli");

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Mcp {
    fn spawn() -> Self {
        let mut child = Command::new(BIN)
            .args(["mcp", "--headless"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn vectorcraft-cli");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self { child, stdin, stdout, next: 1 }
    }
    fn send(&mut self, v: &Value) {
        let w = self.stdin.as_mut().unwrap();
        writeln!(w, "{v}").unwrap();
        w.flush().unwrap();
    }
    fn read(&mut self) -> Value {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).unwrap();
        assert!(n > 0, "server closed stdout");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad JSON line {line}: {e}"))
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let v = self.read();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], id, "reply to the wrong request: {v}");
        v
    }
    fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc": "2.0", "method": method}));
    }
    /// Call a tool; panics on protocol errors or tool errors.
    fn tool(&mut self, name: &str, args: Value) -> Value {
        let v = self.request("tools/call", json!({"name": name, "arguments": args}));
        assert!(v.get("error").is_none(), "{name}: {v}");
        let r = v["result"].clone();
        assert_ne!(r["isError"], json!(true), "{name} {args}: {r}");
        r
    }
    fn tool_json(&mut self, name: &str, args: Value) -> Value {
        let r = self.tool(name, args);
        let text: String =
            r["content"].as_array().unwrap().iter().filter(|c| c["type"] == "text").map(|c| c["text"].as_str().unwrap().to_string()).collect();
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }
    fn run(&mut self, command: &str, params: Value) -> Value {
        self.tool_json("run_command", json!({"command": command, "params": params}))
    }
    fn document(&mut self) -> Value {
        let v = self.request("resources/read", json!({"uri": "vectorcraft://document/json"}));
        serde_json::from_str(v["result"]["contents"][0]["text"].as_str().unwrap()).unwrap()
    }
    fn art_count(&mut self) -> usize {
        let d: vectorcraft_testkit::doc::Document = serde_json::from_value(self.document()).unwrap();
        d.node_count() - d.layers.len()
    }
    fn initialize(&mut self) {
        let v =
            self.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}}));
        let r = &v["result"];
        assert_eq!(r["serverInfo"]["name"], "vectorcraft");
        assert!(r["protocolVersion"].is_string());
        assert!(r["capabilities"]["tools"].is_object());
        self.notify("notifications/initialized");
    }
    fn close(mut self) {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        assert!(status.success(), "server exited with {status}");
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn tmp(name: &str) -> String {
    vectorcraft_testkit::temp_dir("cli-mcp-e2e").join(name).to_string_lossy().to_string()
}

#[test]
fn initialize_and_list_tools_with_valid_schemas() {
    let mut m = Mcp::spawn();
    m.initialize();
    let v = m.request("tools/list", json!({}));
    let tools = v["result"]["tools"].as_array().unwrap().clone();
    assert!(tools.len() >= 15);
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        assert!(!name.is_empty() && t["description"].as_str().is_some_and(|d| !d.is_empty()), "{t}");
        let s = &t["inputSchema"];
        assert_eq!(s["type"], "object", "{name}");
        let props = s["properties"].as_object().unwrap_or_else(|| panic!("{name}: no properties"));
        for r in s.get("required").and_then(Value::as_array).into_iter().flatten() {
            assert!(props.contains_key(r.as_str().unwrap()), "{name}: required {r} missing");
        }
    }
    for want in ["run_command", "draw_shape", "draw_path", "pointer_gesture", "screenshot", "export", "open_file", "undo", "redo", "inspect_document"]
    {
        assert!(tools.iter().any(|t| t["name"] == want), "missing tool {want}");
    }
    // ping and an unknown method.
    assert_eq!(m.request("ping", json!({}))["result"], json!({}));
    assert_eq!(m.request("no/such", json!({}))["error"]["code"], -32601);
    m.close();
}

#[test]
fn draw_inspect_batch_undo_redo() {
    let mut m = Mcp::spawn();
    m.initialize();
    let base = m.art_count();
    let r =
        m.tool_json("draw_shape", json!({"shape": "rectangle", "x": 20, "y": 20, "width": 100, "height": 80, "fill": "#ff0000", "stroke": "none"}));
    assert!(r["id"].is_u64(), "{r}");
    m.tool_json("draw_path", json!({"points": [[200, 50], [300, 50], [250, 150]], "closed": true, "fill": [0, 0, 1]}));
    m.tool_json("draw_path", json!({"d": "M 50 300 C 100 250 150 350 200 300", "stroke": "#00aa00", "strokeWidth": 3}));
    m.tool("pointer_gesture", json!({"tool": "ellipse", "events": [
        {"kind": "down", "x": 300, "y": 300}, {"kind": "drag", "x": 350, "y": 340}, {"kind": "drag", "x": 400, "y": 380}, {"kind": "up", "x": 400, "y": 380}
    ]}));
    assert_eq!(m.art_count(), base + 4);
    let inspect = m.tool_json("inspect_document", json!({}));
    let text = inspect.to_string();
    assert!(text.contains("Rectangle") || text.contains("rectangle"), "{text}");

    // A batch is one undo step.
    let before = m.document();
    m.run(
        "command.batch",
        json!({"commands": [
            {"command": "shape.star", "params": {"cx": 100, "cy": 500, "radius1": 40, "radius2": 20}},
            {"command": "shape.polygon", "params": {"cx": 200, "cy": 500, "radius": 30}},
            {"command": "select.all", "params": {}},
            {"command": "object.group", "params": {}},
        ]}),
    );
    let after = m.document();
    assert_ne!(before, after);
    m.tool("undo", json!({}));
    assert!(vectorcraft_testkit::invariants::json_approx_eq(&m.document(), &before, 1e-12));
    m.tool("redo", json!({}));
    assert!(vectorcraft_testkit::invariants::json_approx_eq(&m.document(), &after, 1e-12));

    // Tool errors are reported in-band, and the server keeps working.
    let v = m.request("tools/call", json!({"name": "run_command", "arguments": {"command": "no.such.command"}}));
    assert_eq!(v["result"]["isError"], json!(true), "{v}");
    let v = m.request("tools/call", json!({"name": "draw_shape", "arguments": {"shape": "hexagram"}}));
    assert_eq!(v["result"]["isError"], json!(true));
    m.tool("inspect_document", json!({}));
    m.close();
}

#[test]
fn screenshot_is_a_decodable_png() {
    let mut m = Mcp::spawn();
    m.initialize();
    m.tool("draw_shape", json!({"shape": "rectangle", "x": 0, "y": 0, "width": 100, "height": 100, "fill": "#ff0000", "stroke": "none"}));
    let path = tmp("shot.png");
    let r = m.tool("screenshot", json!({"scale": 0.5, "path": path}));
    let img = r["content"].as_array().unwrap().iter().find(|c| c["type"] == "image").expect("image content").clone();
    assert_eq!(img["mimeType"], "image/png");
    let png = vectorcraft_testkit::format::base64_decode(img["data"].as_str().unwrap()).expect("base64");
    let decoded = Image::from_png(&png).expect("decodable PNG");
    let doc: vectorcraft_testkit::doc::Document = serde_json::from_value(m.document()).unwrap();
    let ab = doc.artboards[0].rect;
    assert_eq!(decoded.width, (ab.width() * 0.5).round() as u32);
    assert_eq!(decoded.height, (ab.height() * 0.5).round() as u32);
    // The red square covers the top-left 50×50 px.
    let p = decoded.over_white(10, 10);
    assert!(p[0] > 240 && p[1] < 20 && p[2] < 20, "{p:?}");
    let q = decoded.over_white(decoded.width - 5, decoded.height - 5);
    assert_eq!(q, [255, 255, 255]);
    // The saved file is the same PNG.
    assert_eq!(Image::from_png(&std::fs::read(&path).unwrap()).unwrap(), decoded);
    m.close();
}

#[test]
fn export_formats_and_reopen() {
    let mut m = Mcp::spawn();
    m.initialize();
    m.tool("draw_shape", json!({"shape": "ellipse", "x": 50, "y": 50, "width": 200, "height": 120, "fill": "#3366ff"}));
    m.tool("draw_shape", json!({"shape": "star", "cx": 300, "cy": 300, "radius1": 80, "radius2": 40, "fill": "#ffcc00"}));
    let original = m.document();
    let n = m.art_count();

    let dc = tmp("e2e.vectorcraft");
    let svg = tmp("e2e.svg");
    let pdf = tmp("e2e.pdf");
    let png = tmp("e2e.png");
    m.tool("export", json!({"format": "vectorcraft", "path": dc}));
    m.tool("export", json!({"path": svg}));
    m.tool("export", json!({"path": png, "scale": 0.25}));
    m.run("document.export", json!({"format": "pdf", "path": pdf}));
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));
    assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));
    Image::from_png(&std::fs::read(&png).unwrap()).expect("exported PNG decodes");

    // Re-open each: .vectorcraft reproduces the document; SVG and PDF bring the art back.
    m.tool("open_file", json!({"path": dc}));
    assert!(vectorcraft_testkit::invariants::json_approx_eq(&m.document(), &original, 1e-12));
    m.tool("open_file", json!({"path": svg}));
    assert!(m.art_count() >= n, "svg re-open lost art");
    m.run("document.open", json!({"path": pdf}));
    assert!(m.art_count() >= n, "pdf re-open lost art");
    let docs = m.tool_json("inspect_document", json!({}));
    assert!(docs.is_object());

    // save_file writes the native format to a new path and it loads.
    let saved = tmp("saved.vectorcraft");
    m.tool("save_file", json!({"path": saved}));
    let d = vectorcraft_testkit::format::load(&std::fs::read(&saved).unwrap()).unwrap();
    vectorcraft_testkit::invariants::check_document(&d).unwrap();
    m.close();
}

#[test]
fn many_requests_in_one_write_are_answered_in_order() {
    let mut m = Mcp::spawn();
    m.initialize();
    let mut batch = String::new();
    for i in 0..50u64 {
        batch.push_str(&json!({"jsonrpc": "2.0", "id": 1000 + i, "method": "tools/call", "params": {"name": "draw_shape", "arguments": {"shape": "rectangle", "x": i, "y": i, "width": 5, "height": 5}}}).to_string());
        batch.push('\n');
    }
    let w = m.stdin.as_mut().unwrap();
    w.write_all(batch.as_bytes()).unwrap();
    w.flush().unwrap();
    for i in 0..50u64 {
        let v = m.read();
        assert_eq!(v["id"], 1000 + i);
        assert_ne!(v["result"]["isError"], json!(true));
    }
    assert!(m.art_count() >= 50);
    m.close();
}
