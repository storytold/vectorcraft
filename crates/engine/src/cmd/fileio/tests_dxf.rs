//! DXF export through the commands: options, artboards, selection, images, and the formats that
//! can't be written (DWG, PICT).

use serde_json::{Value, json};

use super::*;

fn session(width: f64, height: f64, artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": width, "height": height, "artboards": artboards})).unwrap();
    s
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn text(v: &Value) -> String {
    String::from_utf8(b64(v)).unwrap()
}

/// The value after header variable `var`.
fn header(dxf: &str, var: &str) -> Option<String> {
    let lines: Vec<&str> = dxf.lines().collect();
    let i = lines.iter().position(|l| *l == var)?;
    lines.get(i + 2).map(|s| s.to_string())
}

/// The entity types of a DXF, in order.
fn entities(dxf: &str) -> Vec<String> {
    let lines: Vec<&str> = dxf.lines().collect();
    let start = lines.windows(3).position(|w| w[1] == "ENTITIES" && w[0].trim() == "2").unwrap();
    let pairs: Vec<(&str, &str)> = lines[start + 2..].chunks(2).map(|p| (p[0].trim(), p[1])).collect();
    pairs.iter().take_while(|p| p.1 != "ENDSEC").filter(|p| p.0 == "0").map(|p| p.1.to_string()).collect()
}

/// Two artboards side by side, a rectangle on each.
fn two_boards() -> Session {
    let mut s = session(100.0, 100.0, 2);
    let second = s.doc().unwrap().doc.artboards[1].rect.x0;
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("shape.rectangle", &json!({"x": second + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    s
}

#[test]
fn export_writes_dxf_of_the_chosen_version() {
    let mut s = two_boards();
    let r = s.execute("document.export", &json!({"format": "dxf"})).unwrap();
    assert_eq!(r["format"], "dxf");
    let dxf = text(&r);
    assert_eq!(header(&dxf, "$ACADVER").as_deref(), Some("AC1032"));
    assert_eq!(header(&dxf, "$INSUNITS").as_deref(), Some("4"), "millimetres by default");
    assert_eq!(entities(&dxf), ["LWPOLYLINE", "HATCH", "LWPOLYLINE", "HATCH"], "the whole document, origin at the first artboard");
    for (v, acadver) in [("R12", "AC1009"), ("R14", "AC1014"), ("2000", "AC1015"), ("2004", "AC1018"), ("2013", "AC1027")] {
        let r = s.execute("document.exportDxf", &json!({"version": v})).unwrap();
        assert_eq!(header(&text(&r), "$ACADVER").as_deref(), Some(acadver), "{v}");
    }
    // A number reads as a version too; serialize gives the same bytes.
    assert_eq!(header(&text(&s.execute("document.exportDxf", &json!({"version": 2007})).unwrap()), "$ACADVER").as_deref(), Some("AC1021"));
    let ser = s.execute("document.serialize", &json!({"format": "dxf", "unit": "in"})).unwrap();
    assert_eq!(header(&text(&ser), "$INSUNITS").as_deref(), Some("1"));
}

#[test]
fn use_artboards_writes_one_drawing_per_artboard() {
    let mut s = two_boards();
    let r = s.execute("document.export", &json!({"format": "dxf", "useArtboards": true})).unwrap();
    let files = r["files"].as_array().unwrap();
    let names: Vec<&str> = files.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Untitled-1-Artboard-1.dxf", "Untitled-1-Artboard-2.dxf"]);
    for f in files {
        assert_eq!(entities(&text(f)), ["LWPOLYLINE", "HATCH"], "each holds the art over its artboard");
    }
    let r = s.execute("document.export", &json!({"format": "dxf", "useArtboards": true, "range": "2"})).unwrap();
    assert!(r.get("files").is_none());
    assert_eq!(entities(&text(&r)), ["LWPOLYLINE", "HATCH"]);
    // Without artboards: the origin at the art's bounds.
    let r = s.execute("document.export", &json!({"format": "dxf", "useArtboards": false, "unit": "pt"})).unwrap();
    let dxf = text(&r);
    assert_eq!(header(&dxf, "$LIMMIN").as_deref(), Some("0"));
    assert!(s.execute("document.export", &json!({"format": "dxf", "artboard": 5})).is_err());
    let e = s.execute("document.export", &json!({"format": "dxf", "range": "1-2"})).unwrap_err().to_string();
    assert!(e.contains("useArtboards"), "{e}");
}

#[test]
fn selected_only_exports_the_selection_in_its_layers() {
    let mut s = two_boards();
    s.execute("layer.new", &json!({"name": "Top"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    let line = s.execute("shape.line", &json!({"x1": 0, "y1": 90, "x2": 90, "y2": 90})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [line]})).unwrap();
    let dxf = text(&s.execute("document.export", &json!({"format": "dxf", "selectedOnly": true})).unwrap());
    assert_eq!(entities(&dxf), ["LWPOLYLINE"]);
    assert!(dxf.contains("\n  2\nTop\n") && !dxf.contains("\n  2\nLayer 1\n"), "only the selection's layer");
    s.execute("select.none", &json!({})).unwrap();
    let e = s.execute("document.export", &json!({"format": "dxf", "selectedOnly": true})).unwrap_err().to_string();
    assert!(e.contains("select something"), "{e}");
    // Other formats take it too.
    s.execute("select.set", &json!({"ids": [line]})).unwrap();
    let svg = s.execute("document.serialize", &json!({"format": "svg", "selectedOnly": true})).unwrap()["text"].as_str().unwrap().to_string();
    assert_eq!((svg.matches("<path").count(), svg.matches("<rect").count()), (1, 0), "{svg}");
    assert!(svg.contains("id=\"Top\"") && !svg.contains("Layer"), "{svg}");
}

#[test]
fn placed_images_come_back_as_linked_files() {
    let mut s = super::tests_svg::image_session();
    let r = s.execute("document.export", &json!({"format": "dxf", "rasterFormat": "jpeg"})).unwrap();
    assert_eq!(entities(&text(&r)), ["IMAGE"]);
    let linked = r["linked"].as_array().unwrap();
    assert_eq!(linked.len(), 1);
    assert!(linked[0]["name"].as_str().unwrap().ends_with(".jpg"));
    assert_eq!(&b64(&linked[0])[..2], [0xFF, 0xD8]);
    let dir = std::env::temp_dir().join(format!("vc-dxf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plan.dxf").to_string_lossy().to_string();
    let r = s.execute("document.exportDxf", &json!({"path": path})).unwrap();
    assert_eq!(r["path"], path.as_str());
    let png = r["linked"][0].as_str().unwrap();
    assert!(png.ends_with(".png") && std::fs::read(png).unwrap().starts_with(b"\x89PNG"));
    assert!(std::fs::read_to_string(&path).unwrap().contains(&png.rsplit(['/', '\\']).next().unwrap().to_string()));
    let _ = std::fs::remove_dir_all(dir);
}

/// Two placed images whose pixel data can't be decoded are left out of the drawing and get no
/// linked file, and `warnings` holds the note raster exports give, once, without naming either
/// image's key.
#[test]
fn images_that_cant_be_decoded_give_the_raster_note_once() {
    const NOTE: &str = "an image couldn't be decoded and is missing from the picture";
    let mut s = session(200.0, 100.0, 1);
    crate::tests_menucmds::place_undecodable_png(&mut s);
    crate::tests_menucmds::place_undecodable_png_sized(&mut s, 32, 16);
    let keys: Vec<String> = s.doc().unwrap().doc.images.keys().cloned().collect();
    assert_eq!(keys.len(), 2);
    let r = s.execute("document.export", &json!({"format": "dxf"})).unwrap();
    let w: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(w.iter().filter(|w| **w == NOTE).count(), 1, "{w:?}");
    assert!(!w.iter().any(|w| keys.iter().any(|k| w.contains(k.as_str()))), "{w:?}");
    assert!(entities(&text(&r)).is_empty() && r.get("linked").is_none(), "{r}");
}

#[test]
fn bad_dxf_options_are_refused() {
    let mut s = two_boards();
    for p in [
        json!({"version": "2019"}),
        json!({"colors": 12}),
        json!({"scale": 0}),
        json!({"scale": -2}),
        json!({"unit": "furlong"}),
        json!({"rasterFormat": "gif"}),
        json!({"preserve": "both"}),
        json!({"alterPaths": "yes"}),
    ] {
        let e = s.execute("document.exportDxf", &p).unwrap_err().to_string();
        assert!(e.contains("DXF"), "{p}: {e}");
    }
    // Export for Screens writes images, SVG and PDF only.
    assert!(s.execute("document.exportForScreens", &json!({"formats": [{"format": "dxf"}]})).is_err());
}

#[test]
fn formats_list_dxf_and_what_can_not_be_written() {
    let mut s = Session::new();
    let r = s.execute("document.formats", &json!({})).unwrap();
    let dxf = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "dxf").unwrap().clone();
    // Read too since DXF import (tests_dxfimport).
    assert_eq!((dxf["read"].as_bool(), dxf["write"].as_bool()), (Some(true), Some(true)));
    for o in [
        "version",
        "unit",
        "scale",
        "scaleLineweights",
        "colors",
        "rasterFormat",
        "preserve",
        "alterPaths",
        "outlineText",
        "selectedOnly",
        "useArtboards",
    ] {
        assert!(dxf["options"].get(o).is_some(), "{o}");
    }
    assert!(r["writable"].as_array().unwrap().contains(&json!("dxf")));
    let unsupported: Vec<&str> = r["unsupported"].as_array().unwrap().iter().map(|u| u["id"].as_str().unwrap()).collect();
    assert_eq!(unsupported, ["dwg", "pict", "pmd"]);
}

#[test]
fn dwg_and_pict_say_what_to_use_instead() {
    let mut s = two_boards();
    for p in [json!({"format": "dwg"}), json!({"path": "/tmp/plan.DWG"})] {
        let e = s.execute("document.export", &p).unwrap_err().to_string();
        assert!(e.contains("DWG is not available") && e.contains("DXF"), "{p}: {e}");
    }
    let e = s.execute("document.export", &json!({"format": "pict"})).unwrap_err().to_string();
    assert!(e.contains("PICT is a retired format"), "{e}");
    let e = s
        .execute("document.open", &json!({"name": "old.pct", "dataBase64": vectorcraft_format::base64_encode(&[0u8; 600])}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("old.pct") && e.contains("PICT"), "{e}");
    let e = s
        .execute("document.open", &json!({"name": "site.dwg", "dataBase64": vectorcraft_format::base64_encode(b"AC1032\0\0")}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("DWG is not available"), "{e}");
    let e = s
        .execute("document.open", &json!({"name": "brochure.PMD", "dataBase64": vectorcraft_format::base64_encode(b"PageMaker")}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("brochure.PMD") && e.contains("PageMaker"), "{e}");
}
