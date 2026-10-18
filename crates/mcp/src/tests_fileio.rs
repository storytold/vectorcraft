//! File I/O through MCP: every readable format opens headless, and `export` sends one
//! `document.export` call whatever the backend.

use std::io::{BufRead, BufReader, Write};

use serde_json::{Value, json};

use crate::{Backend, Headless, Remote, call_tool, tool_definitions};

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
