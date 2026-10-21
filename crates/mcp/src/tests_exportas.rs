//! Export As through MCP: `options` carries `useArtboards` and the raster options to
//! `document.export`, so a range writes one file per artboard.

use serde_json::{Value, json};

use crate::{Backend, Headless, call_tool};

fn text(r: &crate::ToolResult) -> Value {
    assert!(!r.is_error, "{r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap()
}

fn three_boards() -> Headless {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 50, "height": 30, "artboards": 3}})).unwrap();
    h
}

#[test]
fn raster_export_uses_document_resolution_unless_overridden() {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 72, "height": 36, "rasterEffectsPpi": 300}})).unwrap();
    for (args, size) in [
        (json!({}), (300, 150)),
        (json!({"scale": 1}), (72, 36)),
        (json!({"options": {"scale": 2}}), (144, 72)),
        (json!({"scale": 2, "options": {"ppi": 150}}), (150, 75)),
    ] {
        let mut params = args.clone();
        params["format"] = json!("png");
        let r = text(&call_tool(&mut h, "export", &params));
        let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().to_rgba8().dimensions(), size, "{args}");
    }
}

#[test]
fn export_with_a_range_writes_one_file_per_artboard() {
    let mut h = three_boards();
    let dir = std::env::temp_dir().join(format!("vectorcraft-mcp-exportas-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("icon.png").to_string_lossy().to_string();
    let args = json!({"path": path, "range": "1, 3", "options": {"useArtboards": true, "ppi": 144, "background": "white"}});
    let r = text(&call_tool(&mut h, "export", &args));
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{r}");
    for (f, board) in files.iter().zip(["Artboard-1", "Artboard-3"]) {
        let p = f.as_str().unwrap();
        assert!(p.ends_with(&format!("icon-{board}.png")), "{p}");
        let img = image::load_from_memory(&std::fs::read(p).unwrap()).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (100, 60), "144 ppi");
        assert!(img.pixels().all(|px| px[3] == 255), "white background");
    }
    // Without a path the files come back as bytes.
    let r = text(&call_tool(&mut h, "export", &json!({"format": "jpg", "options": {"useArtboards": true}})));
    let names: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Untitled-1-Artboard-1.jpg", "Untitled-1-Artboard-2.jpg", "Untitled-1-Artboard-3.jpg"]);
    // SVG, SVGZ and EPS write one file per artboard of a range without useArtboards.
    for format in ["svg", "svgz", "eps"] {
        let r = text(&call_tool(&mut h, "export", &json!({"format": format, "range": "1, 3"})));
        assert_eq!(r["files"].as_array().map(Vec::len), Some(2), "{format}: {r}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
