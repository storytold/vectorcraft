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
fn the_cnc_preset_sets_the_options_that_are_not_given() {
    let mut s = two_boards();
    let cnc = text(&s.execute("document.exportDxf", &json!({"dxfPreset": "cnc"})).unwrap());
    assert_eq!(header(&cnc, "$ACADVER").as_deref(), Some("AC1015"));
    assert_eq!(header(&cnc, "$INSUNITS").as_deref(), Some("4"));
    // document.export takes it too, beside a PDF preset's name.
    let r = s.execute("document.export", &json!({"format": "dxf", "dxfPreset": "cnc"})).unwrap();
    assert_eq!(header(&text(&r), "$ACADVER").as_deref(), Some("AC1015"));
    let o = dxf::options(&json!({"dxfPreset": "CNC", "unit": "in"})).unwrap();
    assert_eq!(
        (o.version, o.unit, o.colors, o.preserve, o.alter_paths, o.outline_text),
        (
            vectorcraft_cad::DxfVersion::R2000,
            vectorcraft_doc::Unit::Inches,
            vectorcraft_cad::ColorDepth::Aci256,
            vectorcraft_cad::Preserve::Editability,
            false,
            true
        )
    );
    assert_eq!((o.include_images, o.include_hatches), (false, false), "CNC / CAM leaves out images and fill hatches");
    let given = dxf::options(&json!({"dxfPreset": "cnc", "includeImages": true, "includeHatches": true})).unwrap();
    assert_eq!((given.include_images, given.include_hatches), (true, true), "the options given override it");
    let custom = dxf::options(&json!({})).unwrap();
    assert_eq!((custom.include_images, custom.include_hatches), (true, true), "Custom keeps both");
    assert_eq!(dxf::options(&json!({})).unwrap().version, vectorcraft_cad::DxfVersion::R2018, "custom by default");
    let e = s.execute("document.exportDxf", &json!({"dxfPreset": "laser"})).unwrap_err().to_string();
    assert!(e.contains("custom or cnc"), "{e}");
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
    assert_eq!(entities(&dxf), ["LINE"]);
    assert!(dxf.contains("\n  2\nTop\n") && !dxf.contains("\n  2\nLayer 1\n"), "only the selection's layer");
    s.execute("select.none", &json!({})).unwrap();
    let e = s.execute("document.export", &json!({"format": "dxf", "selectedOnly": true})).unwrap_err().to_string();
    assert!(e.contains("select something"), "{e}");
    // Other formats take it too.
    s.execute("select.set", &json!({"ids": [line]})).unwrap();
    let svg = s.execute("document.serialize", &json!({"format": "svg", "selectedOnly": true})).unwrap()["text"].as_str().unwrap().to_string();
    assert_eq!((vectorcraft_testkit::svg_shapes(&svg), svg.matches("<line ").count()), (1, 1), "{svg}");
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

/// `includeImages` and `includeHatches` leave out placed images (with their files) and fill
/// hatches, each on its own, through both commands, with and without a path; CNC / CAM leaves
/// out both, and the options given override it.
#[test]
fn placed_images_and_fill_hatches_can_be_left_out() {
    let mut s = super::tests_svg::image_session();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 1, "height": 1})).unwrap();
    let counts = |r: &Value| {
        let e = entities(&text(r));
        let n = |k: &str| e.iter().filter(|x| *x == k).count();
        (n("LWPOLYLINE"), n("HATCH"), n("IMAGE"), r.get("linked").and_then(Value::as_array).map_or(0, Vec::len))
    };
    let warned = |r: &Value, w: &str| r["warnings"].as_array().unwrap().iter().any(|x| x.as_str().is_some_and(|x| x.starts_with(w)));
    let all = s.execute("document.export", &json!({"format": "dxf"})).unwrap();
    assert_eq!(counts(&all), (1, 1, 1, 1));
    let r = s.execute("document.export", &json!({"format": "dxf", "includeImages": false})).unwrap();
    assert_eq!(counts(&r), (1, 1, 0, 0));
    assert!(warned(&r, "placed images are left out") && !warned(&r, "fill hatches"), "{r}");
    let r = s.execute("document.exportDxf", &json!({"includeHatches": false})).unwrap();
    assert_eq!(counts(&r), (1, 0, 1, 1));
    assert!(warned(&r, "fill hatches are left out") && !warned(&r, "placed images"), "{r}");
    let r = s.execute("document.exportDxf", &json!({"dxfPreset": "cnc"})).unwrap();
    assert_eq!(counts(&r), (1, 0, 0, 0));
    let r = s.execute("document.export", &json!({"format": "dxf", "dxfPreset": "cnc", "includeImages": true})).unwrap();
    assert_eq!(counts(&r), (1, 0, 1, 1));
    // With a path, no image file is written next to the drawing.
    let dir = std::env::temp_dir().join(format!("vc-dxf-out-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cut.dxf").to_string_lossy().to_string();
    let r = s.execute("document.exportDxf", &json!({"path": path, "dxfPreset": "cnc"})).unwrap();
    assert!(r.get("linked").is_none(), "{r}");
    let files: Vec<String> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert_eq!(files, ["cut.dxf"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// The reply gives what the drawings lose before a note on the options, across the files: the
/// first artboard's drawing notes the fill hatches that `includeHatches: false` leaves out, and
/// the second's also has a see-through rectangle that DXF 2000 writes opaque.
#[test]
fn what_the_drawings_lose_comes_before_what_an_option_left_out() {
    let mut s = two_boards();
    s.execute("object.setProps", &json!({"opacity": 50})).unwrap();
    let r = s.execute("document.export", &json!({"format": "dxf", "version": "2000", "includeHatches": false, "useArtboards": true})).unwrap();
    let warnings: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    let Some((option, losses)) = warnings.split_last() else { panic!("no notes") };
    assert!(option.starts_with("fill hatches are left out"), "{warnings:?}");
    assert!(losses.iter().any(|w| w.starts_with("transparency needs DXF 2004")), "{warnings:?}");
    assert!(!losses.iter().any(|w| w.starts_with("fill hatches")), "each note once: {warnings:?}");
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
        json!({"includeImages": "no"}),
        json!({"includeHatches": 0}),
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
        "includeImages",
        "includeHatches",
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
