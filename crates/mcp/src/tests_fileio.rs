//! File I/O through MCP: every readable format opens headless, `export` sends one
//! `document.export` call whatever the backend, and `save_file` one `app.save` call.

use std::io::{BufRead, BufReader, Write};

use serde_json::{Value, json};

use crate::{Backend, Headless, Remote, Server, call_tool, tool_definitions};

fn tmp(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("vectorcraft-mcp-fileio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().to_string()
}

fn text(r: &crate::ToolResult) -> Value {
    assert!(!r.is_error, "{r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap()
}

/// A headless session with two artboards of different widths (100 and 40 pt) and a rectangle.
fn two_boards() -> Headless {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 100, "height": 50, "artboards": 2}})).unwrap();
    h.call("engine.execute", json!({"command": "artboard.setProps", "params": {"index": 1, "width": 40}})).unwrap();
    h.call("engine.execute", json!({"command": "shape.rectangle", "params": {"x": 10, "y": 10, "width": 20, "height": 20}})).unwrap();
    h
}

#[test]
fn headless_opens_pdf_ai_and_templates() {
    let mut h = two_boards();
    let pdf = h.call("engine.execute", json!({"command": "document.serialize", "params": {"format": "pdf"}})).unwrap();
    let pdf = vectorcraft_format::base64_decode(pdf["dataBase64"].as_str().unwrap()).unwrap();
    for name in ["x.pdf", "x.ai"] {
        let path = tmp(name);
        std::fs::write(&path, &pdf).unwrap();
        let r = text(&call_tool(&mut h, "open_file", &json!({"path": path})));
        assert_eq!(r["title"], name);
        assert_eq!(h.session.doc().unwrap().doc.artboards.len(), 2, "one artboard per page");
    }
    let ait = tmp("x.ait");
    std::fs::write(&ait, &pdf).unwrap();
    let r = text(&call_tool(&mut h, "open_file", &json!({"path": ait})));
    assert!(r["title"].as_str().unwrap().starts_with("Untitled-"), "{r}");
    let saved = h.call("engine.execute", json!({"command": "file.saveAsTemplate", "params": {"path": tmp("t.vectorcraft")}})).unwrap();
    let tpl = tmp("t.vctemplate");
    assert_eq!(saved["path"], tpl.as_str(), "saved with the template's extension: {saved}");
    let r = text(&call_tool(&mut h, "open_file", &json!({"path": tpl})));
    assert!(r["title"].as_str().unwrap().starts_with("Untitled-"), "{r}");
    assert_eq!(h.session.doc().unwrap().path, None, "Save asks for a new name");
}

#[test]
fn export_tool_lists_engine_formats_and_options() {
    let tools = tool_definitions();
    let export = tools.iter().find(|t| t["name"] == "export").unwrap();
    let props = &export["inputSchema"]["properties"];
    assert_eq!(
        props["format"]["enum"],
        json!([
            "vectorcraft",
            "svg",
            "svgz",
            "pdf",
            "ai",
            "png",
            "jpg",
            "gif",
            "webp",
            "tiff",
            "bmp",
            "template",
            "png8",
            "txt",
            "dxf",
            "eps",
            "emf",
            "wmf",
            "tga",
            "psd"
        ])
    );
    for k in ["artboard", "range", "options"] {
        assert!(props.get(k).is_some(), "export takes {k}");
    }
    // The description names every format of the enum.
    let words: Vec<&str> = export["description"].as_str().unwrap().split(|c: char| !c.is_ascii_alphanumeric()).collect();
    for f in props["format"]["enum"].as_array().unwrap() {
        assert!(words.contains(&f.as_str().unwrap()), "{f}");
    }
    let open = tools.iter().find(|t| t["name"] == "open_file").unwrap();
    assert!(open["description"].as_str().unwrap().contains(".ait"));
    assert!(open["description"].as_str().unwrap().contains(".eps, .epsf, .ps, "), "{open}");
    assert!(open["description"].as_str().unwrap().contains(".bmp, .tga, .vda, .icb, .vst, "), "{open}");
}

/// A fake app that records `engine.execute` calls and answers each with an empty object.
fn recording_app() -> (String, std::thread::JoinHandle<Vec<Value>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let h = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut seen = vec![];
        for line in BufReader::new(stream).lines() {
            let msg: Value = serde_json::from_str(&line.unwrap()).unwrap();
            writeln!(out, "{}", json!({"id": msg["id"], "ok": true, "result": {}})).unwrap();
            seen.push(msg);
        }
        seen
    });
    (addr, h)
}

#[test]
fn export_is_the_same_call_in_the_app_and_headless() {
    let args = json!({"format": "png", "artboard": 1, "options": {"scale": 2, "artboard": 0, "range": "1-2"}});
    let (addr, app) = recording_app();
    let mut remote = Remote::connect(&addr).unwrap();
    assert!(!call_tool(&mut remote, "export", &args).is_error);
    drop(remote);
    let seen = app.join().unwrap();
    assert_eq!(seen[0]["method"], "engine.execute");
    assert_eq!(seen[0]["params"], json!({"command": "document.export", "params": {"format": "png", "artboard": 1, "scale": 2}}));

    // Headless runs that very call: artboard 1 (40 pt wide) at scale 2.
    let mut h = two_boards();
    let r = text(&call_tool(&mut h, "export", &args));
    let png = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
    assert_eq!(&png[16..20], 80u32.to_be_bytes(), "IHDR width");
    let direct = h.call("engine.execute", seen[0]["params"].clone()).unwrap();
    assert_eq!(direct["dataBase64"], r["dataBase64"]);
}

#[test]
fn the_raster_scale_leaves_other_formats_alone() {
    // DXF's own `scale` (drawing units per mm) comes through `options`; the raster one would resize
    // the drawing and leave its units unset.
    let mut h = two_boards();
    let dxf = |h: &mut Headless, args: Value| text(&call_tool(h, "export", &args))["dataBase64"].clone();
    let plain = dxf(&mut h, json!({"format": "dxf"}));
    assert!(dxf(&mut h, json!({"format": "dxf", "scale": 2})) == plain, "the raster scale changed the DXF");
    assert!(dxf(&mut h, json!({"format": "dxf", "options": {"scale": 2}})) != plain, "the DXF scale was dropped");
}

#[test]
fn exporting_native_keeps_the_document_path() {
    let mut h = two_boards();
    let doc = tmp("keep.vectorcraft");
    h.call("app.save", json!({"path": doc})).unwrap();
    let copy = tmp("copy.vectorcraft");
    let r = text(&call_tool(&mut h, "export", &json!({"path": copy})));
    assert_eq!(r["format"], "vectorcraft");
    assert_eq!(h.session.doc().unwrap().path.as_deref(), Some(doc.as_str()));
    // PDF range through the tool: one page.
    let pdf = text(&call_tool(&mut h, "export", &json!({"format": "pdf", "range": "2"})));
    let pdf = vectorcraft_format::base64_decode(pdf["dataBase64"].as_str().unwrap()).unwrap();
    assert_eq!(pdf_pages(&pdf), 1);
    // An .ai export holds every artboard by default and one artboard with a range, and the document keeps its path.
    for (args, pages) in [(json!({"format": "ai"}), 2), (json!({"format": "ai", "range": "2"}), 1)] {
        let ai = text(&call_tool(&mut h, "export", &args));
        assert_eq!(ai["format"], "ai");
        assert_eq!(pdf_pages(&vectorcraft_format::base64_decode(ai["dataBase64"].as_str().unwrap()).unwrap()), pages, "{args}");
    }
    assert_eq!(h.session.doc().unwrap().path.as_deref(), Some(doc.as_str()));
}

/// The bytes `save_file` writes to `name` with `args`.
fn saved_with(h: &mut Headless, name: &str, mut args: Value) -> Vec<u8> {
    let path = tmp(name);
    args["path"] = json!(path);
    let r = text(&call_tool(h, "save_file", &args));
    assert_eq!(r["format"], "vectorcraft", "{r}");
    std::fs::read(&path).unwrap()
}

#[test]
fn save_file_takes_the_native_save_options() {
    let mut h = two_boards();
    let gz = saved_with(&mut h, "gz.vectorcraft", json!({"compress": true}));
    assert_eq!(gz[..2], [0x1f, 0x8b], "gzipped");
    let plain = saved_with(&mut h, "plain.vectorcraft", json!({"compress": false}));
    assert_eq!(plain[0], b'{', "plain JSON");
    let nodes = h.session.doc().unwrap().doc.node_count();
    for name in ["gz.vectorcraft", "plain.vectorcraft"] {
        let mut back = Headless::new();
        text(&call_tool(&mut back, "open_file", &json!({"path": tmp(name)})));
        assert_eq!(back.session.doc().unwrap().doc.node_count(), nodes, "{name} reopens");
    }
    let opt = saved_with(&mut h, "opt.vectorcraft", json!({"options": {"compress": true}}));
    assert_eq!(opt[..2], [0x1f, 0x8b], "`options` carries them too");
    let v3: Value = serde_json::from_slice(&saved_with(&mut h, "v3.vectorcraft", json!({"version": 3, "preview": null}))).unwrap();
    assert_eq!(v3["version"], 3);
    let shown = saved_with(&mut h, "preview.vectorcraft", json!({"preview": true}));
    assert!(vectorcraft_format::preview(&shown).is_some(), "a preview");
    assert!(vectorcraft_format::preview(&plain).is_none(), "off by default");
    // Version 2 files can't be compressed: the engine's error comes back.
    let r = call_tool(&mut h, "save_file", &json!({"path": tmp("v2.vectorcraft"), "version": 2, "compress": true}));
    assert!(r.is_error && r.content[0]["text"].as_str().unwrap().contains("can't be compressed"), "{r:?}");

    // The options are in the schema; a misspelled one is still refused.
    let mut s = Server::new(Box::new(Headless::with_document()));
    let call = |s: &mut Server, args: Value| {
        s.handle(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "save_file", "arguments": args}})).unwrap()
    };
    let r = call(&mut s, json!({"path": tmp("ok.vectorcraft"), "compress": true}));
    assert_eq!(r["result"]["isError"], false, "{r}");
    let r = call(&mut s, json!({"path": tmp("typo.vectorcraft"), "compresss": true}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
    assert!(r["error"]["message"].as_str().unwrap().contains("unknown argument `compresss`"), "{r}");
}

/// `warnings` name each native save option the written format doesn't read, and a top-level option
/// that disagrees with the same key in `options` is an error.
#[test]
fn save_file_warns_about_options_it_does_not_use() {
    let mut h = two_boards();
    let warnings = |r: &Value| -> Vec<String> { r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect() };
    let ignored = |w: &[String], key: &str| w.iter().any(|w| w.starts_with(&format!("`{key}` was not used")));

    // SVG reads none of them, at the top level or in `options`.
    let path = tmp("ignored.svg");
    let r = text(&call_tool(&mut h, "save_file", &json!({"path": path, "compress": true, "version": 3, "options": {"preview": true}})));
    assert_eq!(r["format"], "svg", "{r}");
    let w = warnings(&r);
    assert!(ignored(&w, "compress") && ignored(&w, "version") && ignored(&w, "options.preview"), "{w:?}");
    assert_eq!(w.iter().filter(|w| w.starts_with("`compress`")).count(), 1, "said once: {w:?}");
    assert!(w.iter().any(|w| w.contains("SVG")), "names the format: {w:?}");
    assert!(std::fs::read(&path).unwrap().starts_with(b"<"), "plain SVG");
    // A .ai file reads `compress` but not `version`.
    let r = text(&call_tool(&mut h, "save_file", &json!({"path": tmp("ignored.ai"), "compress": false, "version": 3})));
    let w = warnings(&r);
    assert!(ignored(&w, "version") && !ignored(&w, "compress"), "{w:?}");

    // A top-level value that disagrees with `options` is refused, and nothing is written.
    let path = tmp("both.vectorcraft");
    let _ = std::fs::remove_file(&path);
    let r = call_tool(&mut h, "save_file", &json!({"path": path, "compress": true, "options": {"compress": false}}));
    let e = r.content[0]["text"].as_str().unwrap();
    assert!(r.is_error && e.contains("`compress: true` and `options.compress: false` disagree"), "{r:?}");
    assert!(!std::path::Path::new(&path).exists(), "nothing is written");
    // Nothing to say when they agree or when every option applies.
    for mut args in [json!({"compress": true, "options": {"compress": true}}), json!({"compress": false, "version": 3, "preview": true})] {
        args["path"] = json!(tmp("quiet.vectorcraft"));
        let r = text(&call_tool(&mut h, "save_file", &args));
        assert_eq!(r["warnings"], json!([]), "{args}: {r}");
    }

    // The app's save warns about the top-level options itself; the reply gets the notes on
    // `options`, after the warnings the app sent.
    struct App;
    impl Backend for App {
        fn call(&mut self, _: &str, _: Value) -> Result<Value, String> {
            Ok(json!({"path": "/out/a.svg", "format": "svg", "warnings": ["the app's own warning"]}))
        }
        fn has_ui(&self) -> bool {
            true
        }
        fn describe(&self) -> String {
            "app".into()
        }
    }
    let r = text(&call_tool(&mut App, "save_file", &json!({"path": "/out/a.svg", "compress": true, "options": {"preview": true}})));
    let w = warnings(&r);
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(w[0] == "the app's own warning" && ignored(&w[1..], "options.preview"), "{w:?}");
}

#[test]
fn save_file_is_the_same_call_in_the_app_and_headless() {
    let path = tmp("app.vectorcraft");
    let (addr, app) = recording_app();
    let mut remote = Remote::connect(&addr).unwrap();
    let args = json!({"path": path, "compress": true, "version": 3, "preview": null, "options": {"preview": true}});
    assert!(!call_tool(&mut remote, "save_file", &args).is_error);
    drop(remote);
    let seen = app.join().unwrap();
    assert_eq!(seen[0]["method"], "app.save");
    assert_eq!(seen[0]["params"], json!({"path": path, "compress": true, "version": 3, "options": {"preview": true}, "wait": true}));
}

/// The app is asked to write the file before it replies (`wait`), even with Background Save on, so a
/// save that fails is the call's error; a headless session always does, and is sent no `wait`.
#[test]
fn save_file_waits_for_the_file_in_the_app() {
    struct Recording {
        ui: bool,
        seen: Vec<(String, Value)>,
    }
    impl Backend for Recording {
        fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
            self.seen.push((method.to_string(), params));
            if self.ui { Err("Saving a.vectorcraft failed: disk full".into()) } else { Ok(json!({})) }
        }
        fn has_ui(&self) -> bool {
            self.ui
        }
        fn describe(&self) -> String {
            "recording".into()
        }
    }
    for ui in [true, false] {
        let mut b = Recording { ui, seen: vec![] };
        let r = call_tool(&mut b, "save_file", &json!({"path": "/out/a.vectorcraft"}));
        assert_eq!(r.is_error, ui, "the app's error is the call's: {r:?}");
        let [(method, params)] = &b.seen[..] else { panic!("one call: {:?}", b.seen) };
        assert_eq!(method, "app.save");
        // The tool sends the path made absolute (on Windows, against the current drive).
        let path = std::path::absolute("/out/a.vectorcraft").unwrap().to_string_lossy().into_owned();
        let mut want = json!({"path": path});
        if ui {
            want["wait"] = json!(true);
        }
        assert_eq!(params, &want, "has_ui: {ui}");
    }
}

fn pdf_pages(pdf: &[u8]) -> usize {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "document.open", "params": {"name": "p.pdf", "dataBase64": vectorcraft_format::base64_encode(pdf)}}))
        .unwrap();
    h.session.doc().unwrap().doc.artboards.len()
}

/// Issue #421 over MCP: RGB greys switched to CMYK with `grays: "black"` print on the black plate
/// only, in the document and in the exported PDF.
#[test]
fn document_color_mode_puts_rgb_greys_on_the_black_plate() {
    let svg = tmp("greys.svg");
    std::fs::write(
        &svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200pt" height="50pt" viewBox="0 0 200 50">
<rect x="0" y="0" width="50" height="50" fill="#000000"/><rect x="50" y="0" width="50" height="50" fill="#333333"/>
<rect x="100" y="0" width="50" height="50" fill="#808080"/><rect x="150" y="0" width="50" height="50" fill="#e6e6e6"/>
</svg>"##,
    )
    .unwrap();
    let want = [1.0, 0.8, 0.498, 0.098];
    let k_only = |h: &Headless| -> Vec<f32> {
        let mut ks = vec![];
        for l in &h.session.doc().unwrap().doc.layers {
            l.walk(&mut |n| {
                if n.children().is_none()
                    && let Some(c) = n.appearance.fill_paint().color()
                {
                    let v = serde_json::to_value(c).unwrap();
                    assert!(v["model"] == "cmyk" && v["c"] == 0.0 && v["m"] == 0.0 && v["y"] == 0.0, "not K only: {v}");
                    ks.push(v["k"].as_f64().unwrap() as f32);
                }
            });
        }
        ks
    };
    let close = |ks: &[f32]| ks.len() == want.len() && ks.iter().zip(want).all(|(k, w)| (k - w).abs() < 0.005);
    let mut h = Headless::new();
    text(&call_tool(&mut h, "open_file", &json!({"path": svg})));
    let r = text(&call_tool(&mut h, "run_command", &json!({"command": "file.documentColorMode", "params": {"mode": "cmyk", "grays": "black"}})));
    assert_eq!(r["changed"], 4);
    let ks = k_only(&h);
    assert!(close(&ks), "{ks:?}");
    // The PDF carries the same inks (DeviceCMYK opens as CMYK).
    let pdf = text(&call_tool(&mut h, "export", &json!({"format": "pdf"})));
    let mut back = Headless::new();
    back.call("engine.execute", json!({"command": "document.open", "params": {"name": "greys.pdf", "dataBase64": pdf["dataBase64"]}})).unwrap();
    let ks = k_only(&back);
    assert!(close(&ks), "{ks:?}");
    // document.open takes the same option.
    let mut o = Headless::new();
    let r = call_tool(&mut o, "run_command", &json!({"command": "document.open", "params": {"path": svg, "colorMode": "cmyk", "grays": "black"}}));
    text(&r);
    let ks = k_only(&o);
    assert!(close(&ks), "{ks:?}");
    let bad = call_tool(&mut o, "run_command", &json!({"command": "file.documentColorMode", "params": {"mode": "rgb", "grays": "rich"}}));
    assert!(bad.is_error, "{bad:?}");
}
