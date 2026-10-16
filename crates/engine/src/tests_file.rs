use serde_json::json;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

#[test]
fn batch_is_one_undo_step() {
    let mut s = session();
    let r = s
        .execute(
            "command.batch",
            &json!({"label": "Logo", "commands": [
                {"command": "paint.setFill", "params": {"color": "#ff0000"}},
                {"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 50, "height": 50}},
                {"command": "shape.ellipse", "params": {"x": 60, "y": 0, "width": 50, "height": 50}},
            ]}),
        )
        .unwrap();
    assert_eq!(r["results"].as_array().unwrap().len(), 3);
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), 1);
    assert_eq!(st.history.undo[0].label, "Logo");
    assert_eq!(st.doc.layers[0].children().unwrap().len(), 2);
    assert_eq!(s.journal.iter().filter(|(c, _)| c == "command.batch").count(), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 0);
}

#[test]
fn batch_rolls_back_on_error() {
    let mut s = session();
    let r = s.execute(
        "command.batch",
        &json!({"commands": [
            {"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 50, "height": 50}},
            {"command": "no.such.command", "params": {}},
        ]}),
    );
    assert!(r.is_err());
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 0);
    assert!(s.doc().unwrap().history.undo.is_empty());
    assert!(!s.in_interaction());
}

#[test]
fn serialize_and_reopen() {
    let mut s = session();
    s.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap();
    let b64 = s.execute("document.serialize", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(svg.contains("<svg"));
    s.execute("document.open", &json!({"name": "copy.vectorcraft", "dataBase64": b64})).unwrap();
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.doc().unwrap().doc.node_count(), 2);
    let png = s.execute("document.serialize", &json!({"format": "png", "scale": 0.5})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    assert!(png.len() > 100);
}

#[test]
fn save_and_open_path() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap();
    let dir = std::env::temp_dir().join(format!("dc-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.vectorcraft");
    s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
    s.execute("document.open", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(path.to_string_lossy().as_ref()));
    let svg = dir.join("t.svg");
    s.execute("document.export", &json!({"path": svg.to_string_lossy()})).unwrap_or_default();
    s.execute("document.export", &json!({"format": "svg", "path": svg.to_string_lossy()})).unwrap();
    assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pdf_roundtrip_through_engine() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap();
    let b64 = s.execute("document.serialize", &json!({"format": "pdf"})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    s.execute("document.open", &json!({"name": "x.pdf", "dataBase64": b64})).unwrap();
    assert!(s.doc().unwrap().doc.node_count() >= 2);
}

#[test]
fn draw_behind_and_inside() {
    let mut s = session();
    let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("view.drawMode", &json!({"mode": "behind"})).unwrap();
    let b = s.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    let order: Vec<u64> = s.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id.0).collect();
    assert_eq!(order, vec![b, a]);
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    s.execute("view.drawMode", &json!({"mode": "inside"})).unwrap();
    let c = s.execute("shape.ellipse", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let d = &s.doc().unwrap().doc;
    let g = d.parent_of(NodeId(c)).unwrap();
    assert_eq!(d.node(g).unwrap().kind_label(), "Clip Group");
    assert_eq!(d.parent_of(NodeId(a)), Some(g));
    // A second shape goes into the same clip group.
    s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(g).unwrap().children().unwrap().len(), 4);
    s.execute("view.drawMode", &json!({"mode": "normal"})).unwrap();
    assert!(s.draw_inside.is_none());
}

/// In Draw Inside, a double-click with a selection tool outside the object being drawn into sets
/// the drawing mode to Draw Normal, and the tool still takes the double-click. Inside the object's
/// bounds, grown by the selection tolerance, and with other tools, Draw Inside stays on.
#[test]
fn double_clicking_outside_the_draw_inside_object_returns_to_draw_normal() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = session();
    let a = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let inside = |s: &mut Session| {
        s.execute("select.set", &json!({"ids": [a]})).unwrap();
        s.execute("view.drawMode", &json!({"mode": "inside"})).unwrap();
    };
    let double = |s: &mut Session, x: f64, y: f64| {
        s.pointer(&PointerEvent::new(PointerKind::DoubleClick, x, y), ViewInfo::default()).unwrap();
        (s.draw_mode, s.draw_inside)
    };
    inside(&mut s);
    s.select_tool("selection", ViewInfo::default()).unwrap();
    // Inside the rectangle the Selection tool isolates it, as in Draw Normal; 2 pt past its edge
    // is within the 3 px selection tolerance at 100%.
    assert_eq!(double(&mut s, 150.0, 150.0), (DrawMode::Inside, Some(NodeId(a))));
    assert_eq!(s.doc().unwrap().isolation, Some(NodeId(a)));
    assert_eq!(double(&mut s, 202.0, 150.0), (DrawMode::Inside, Some(NodeId(a))));
    // A double-click outside, on the empty canvas, sets Draw Normal and still leaves isolation mode.
    assert_eq!(double(&mut s, 300.0, 250.0), (DrawMode::Normal, None));
    assert_eq!(s.doc().unwrap().isolation, None);
    // With the Rectangle tool a double-click leaves Draw Inside on; with Direct Selection it sets Draw Normal.
    inside(&mut s);
    s.select_tool("rectangle", ViewInfo::default()).unwrap();
    assert_eq!(double(&mut s, 300.0, 250.0), (DrawMode::Inside, Some(NodeId(a))));
    s.select_tool("directSelection", ViewInfo::default()).unwrap();
    assert_eq!(double(&mut s, 300.0, 250.0), (DrawMode::Normal, None));
    // Once the object is gone, any double-click with a selection tool sets Draw Normal.
    inside(&mut s);
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(double(&mut s, 150.0, 150.0), (DrawMode::Normal, None));
}

#[test]
fn export_for_screens_writes_every_combination() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "artboards": 2})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let dir = std::env::temp_dir().join(format!("dc-efs-{}", std::process::id()));
    let r = s
        .execute("document.exportForScreens", &json!({"folder": dir.to_string_lossy(), "formats": [{"format": "png", "scale": 1}, {"format": "png", "scale": 2}, {"format": "jpg"}, {"format": "svg"}, {"format": "webp"}]}))
        .unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 10);
    for f in files {
        assert!(std::fs::metadata(f.as_str().unwrap()).unwrap().len() > 50, "{f}");
    }
    assert!(files.iter().any(|f| f.as_str().unwrap().ends_with("Artboard-2@2x.png")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recolor_maps_fills_strokes_gradients() {
    let mut s = session();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute(
        "paint.setFill",
        &json!({"gradient": {"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}}),
    )
    .unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let c = s.execute("recolor.colors", &json!({})).unwrap();
    let hexes: Vec<&str> = c["colors"].as_array().unwrap().iter().map(|v| v["hex"].as_str().unwrap()).collect();
    assert!(hexes.contains(&"#ff0000") && hexes.contains(&"#0000ff") && hexes.contains(&"#000000"));
    s.execute("recolor.apply", &json!({"map": {"#FF0000": "#00ff00"}})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(a)).unwrap().clone();
    match n.appearance.fill_paint() {
        vectorcraft_color::Paint::Gradient(g) => assert_eq!(g.gradient.stops[0].color.to_hex(), "#00ff00"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn export_selection_crops_to_the_selection() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 200, "y": 100, "width": 80, "height": 40})).unwrap();
    let v = s.execute("document.exportSelection", &json!({"format": "png", "scale": 2})).unwrap();
    let b = v["bounds"].as_array().unwrap();
    // Only the selected (last) rectangle, with its stroke.
    let w = b[2].as_f64().unwrap();
    assert!((81.0..90.0).contains(&w) && b[0].as_f64().unwrap() > 190.0, "{v}");
    let png = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    assert_eq!(&png[..4], b"\x89PNG");
    assert_eq!(u32::from_be_bytes([png[16], png[17], png[18], png[19]]), (w * 2.0).round() as u32);
    let svg = s.execute("document.exportSelection", &json!({"format": "svg"})).unwrap();
    let text = String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(text.matches("<path").count(), 1, "{text}");
}

/// The crop of a selection export takes in the object's effects: a drop shadow, an outer glow and
/// an Offset Path effect paint inside the image, which is the image of the art that Export with
/// Use Artboards off writes.
#[test]
fn export_selection_keeps_the_effects_in_the_crop() {
    // The right edge of what the effect paints: the shadow 8 pt to the right, the glow and the
    // grown path 6 pt past the square (the stroke's half point included).
    for (effect, params, right) in [
        ("stylize.dropShadow", json!({"x": 8, "y": 0, "blur": 0}), 128.5),
        ("stylize.outerGlow", json!({"blur": 4}), 126.5),
        ("path.offsetPath", json!({"offset": 6}), 126.5),
    ] {
        let mut s = session();
        s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap();
        let plain = s.execute("document.exportSelection", &json!({"format": "png"})).unwrap();
        assert_eq!(plain["bounds"], json!([99.5, 99.5, 21.0, 21.0]), "a plain path keeps its crop: {plain}");
        s.execute("effect.apply", &json!({"effect": effect, "params": params})).unwrap();
        let png = |v: &serde_json::Value| {
            image::load_from_memory(&vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).unwrap().to_rgba8()
        };
        let v = s.execute("document.exportSelection", &json!({"format": "png", "background": "transparent"})).unwrap();
        let b: Vec<f64> = v["bounds"].as_array().unwrap().iter().map(|n| n.as_f64().unwrap()).collect();
        assert!(b[0] < 99.5 && b[1] < 99.5 && b[0] + b[2] >= right && b[1] + b[3] > 120.5, "{effect}: {v}");
        let img = png(&v);
        let (w, h) = img.dimensions();
        assert_eq!((w, h), (b[2].round() as u32, b[3].round() as u32), "{effect}");
        // 2 pt past the square's stroke, on the right.
        assert!(img.get_pixel((122.5 - b[0]) as u32, h / 2)[3] > 0, "{effect}: the effect is in the image");
        let edge = (0..w).flat_map(|x| [(x, 0), (x, h - 1)]).chain((0..h).flat_map(|y| [(0, y), (w - 1, y)]));
        assert!(edge.into_iter().all(|(x, y)| img.get_pixel(x, y)[3] == 0), "{effect}: nothing is cut off at the image's edge");
        let art = s.execute("document.export", &json!({"format": "png", "useArtboards": false, "background": "transparent"})).unwrap();
        assert!(png(&art) == img, "{effect}: the same image as the art's bounds");
    }
}

/// A group's own drop shadow is inside the crop of the group's selection export too.
#[test]
fn export_selection_keeps_a_groups_shadow_in_the_crop() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 130, "y": 100, "width": 20, "height": 20})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let group = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    let plain = s.execute("document.exportSelection", &json!({"format": "png"})).unwrap();
    assert_eq!(plain["bounds"], json!([99.5, 99.5, 51.0, 21.0]), "{plain}");
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [group], "params": {"x": 8, "y": 0, "blur": 0}})).unwrap();
    let v = s.execute("document.exportSelection", &json!({"format": "png", "background": "transparent"})).unwrap();
    let b: Vec<f64> = v["bounds"].as_array().unwrap().iter().map(|n| n.as_f64().unwrap()).collect();
    // The shadow of the right square reaches x = 158.5.
    assert!(b[0] < 99.5 && b[0] + b[2] >= 158.5, "{v}");
    let img = image::load_from_memory(&vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).unwrap().to_rgba8();
    assert!(img.get_pixel((155.0 - b[0]) as u32, img.height() / 2)[3] > 0, "the shadow is in the image");
}

/// Export with Use Artboards off and Fit to Artwork Bounds take in the drop shadow of a group and
/// of type.
#[test]
fn the_arts_bounds_take_in_the_shadow_of_a_group_and_of_type() {
    for kind in ["group", "type"] {
        let mut s = session();
        let id = if kind == "group" {
            s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap();
            s.execute("shape.rectangle", &json!({"x": 130, "y": 100, "width": 20, "height": 20})).unwrap();
            s.execute("select.all", &json!({})).unwrap();
            s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap()
        } else {
            s.execute("text.create", &json!({"x": 100, "y": 120, "text": "Hill"})).unwrap()["id"].as_u64().unwrap()
        };
        let plain = s.doc().unwrap().doc.node(NodeId(id)).unwrap().visual_bounds().unwrap();
        s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id], "params": {"x": 8, "y": 0, "blur": 0}})).unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let png = |v: &serde_json::Value| {
            image::load_from_memory(&vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).unwrap().to_rgba8()
        };
        let art = s.execute("document.export", &json!({"format": "png", "useArtboards": false, "background": "transparent"})).unwrap();
        let sel = s.execute("document.exportSelection", &json!({"format": "png", "background": "transparent"})).unwrap();
        let x0 = sel["bounds"][0].as_f64().unwrap();
        let img = png(&art);
        assert!(png(&sel) == img, "{kind}: the same image as Export Selection's");
        let past = (plain.x1 + 1.0 - x0).ceil() as u32;
        assert!(img.enumerate_pixels().any(|(x, _, p)| x >= past && p[3] > 0), "{kind}: the shadow is in the image, right of the object");
        s.execute("artboard.fitToArt", &json!({})).unwrap();
        let board = s.doc().unwrap().doc.artboards[0].rect;
        assert!(board.x0 <= plain.x0 && board.x1 >= plain.x1 + 8.0, "{kind}: the artboard holds the shadow 8 pt right of {plain:?}: {board:?}");
    }
}

/// The crop of a brushed path is its visual bounds, with no room added for brush art.
#[test]
fn export_selection_crops_a_brushed_path_to_its_visual_bounds() {
    let mut s = session();
    let id = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("brush.apply", &json!({"name": "Charcoal"})).unwrap();
    let v = s.execute("document.exportSelection", &json!({"format": "png"})).unwrap();
    let b = s.doc().unwrap().doc.node(NodeId(id)).unwrap().visual_bounds().unwrap();
    assert_eq!(v["bounds"], json!([b.x0, b.y0, b.width(), b.height()]), "{v}");
}

/// A selection export reports the encoder's warnings like a whole-document export, and an empty
/// list when nothing was approximated.
#[test]
fn export_selection_reports_the_encoders_warnings() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    s.execute("transparency.set", &json!({"blend": "Multiply"})).unwrap();
    let warnings = |v: &serde_json::Value| v["warnings"].as_array().cloned().unwrap_or_else(|| panic!("no warnings key: {v}"));
    // Control: nothing approximated.
    let plain = s.execute("document.exportSelection", &json!({"format": "svg"})).unwrap();
    assert_eq!(plain["format"], "svg");
    assert!(warnings(&plain).is_empty(), "{plain}");
    for (format, options, says) in [
        ("webp", json!({"lossless": false}), "lossless"),
        ("svg", json!({"profile": "tiny12"}), "blend"),
        ("pdf", json!({"createLayers": true, "compatibility": "1.4"}), "layers"),
    ] {
        let mut p = options.clone();
        p["format"] = json!(format);
        let sel = s.execute("document.exportSelection", &p).unwrap();
        let doc = s.execute("document.export", &p).unwrap();
        let w = warnings(&sel);
        assert!(w.iter().any(|w| w.as_str().unwrap_or("").to_lowercase().contains(says)), "{format}: {sel}");
        assert_eq!(w, warnings(&doc), "{format}: the same warnings as a whole-document export");
    }
}

#[test]
fn template_opens_as_untitled() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    let dir = std::env::temp_dir().join(format!("vc-tpl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("letterhead.vectorcraft");
    s.execute("file.saveAsTemplate", &json!({"path": path.to_str().unwrap()})).unwrap();
    s.execute("document.open", &json!({"path": path.to_str().unwrap()})).unwrap();
    let st = s.doc().unwrap();
    assert!(st.path.is_none() && !st.doc.template);
    assert!(st.doc.title.starts_with("Untitled-"), "{}", st.doc.title);
    assert_eq!(st.doc.layers[0].children().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn raster_export_too_large_is_an_error_not_a_crash() {
    // The largest artboard (16383 pt) at 300 ppi is ~68 000 px a side: more than the rasteriser can
    // address. The export used to panic inside the renderer (crashing the app or the MCP server).
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 16383, "height": 16383})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 100})).unwrap();
    let dir = std::env::temp_dir().join(format!("vc-raster-limit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for format in ["png", "jpg", "webp"] {
        let path = dir.join(format!("big.{format}"));
        let e = s.execute("document.export", &json!({"path": path.to_string_lossy(), "scale": 300.0 / 72.0})).unwrap_err().to_string();
        assert!(e.contains("pixels"), "{format}: {e}");
        assert!(!path.exists(), "{format}");
        assert!(s.execute("document.serialize", &json!({"format": format, "scale": 300.0 / 72.0})).is_err(), "{format}");
    }
    // A size within the limits still exports.
    let path = dir.join("ok.png");
    s.execute("document.export", &json!({"path": path.to_string_lossy(), "scale": 0.25})).unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(b"\x89PNG"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Japanese file and folder names save, open, export and title documents as typed: the native
/// format, SVG, PNG and PDF, in a folder with a Japanese name too.
#[test]
fn japanese_file_names_save_open_and_export() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 80, "text": "日本語のテキスト"})).unwrap();
    let dir = std::env::temp_dir().join(format!("vc-日本語フォルダ-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("運動会カード（最終版）・2024.vectorcraft");
    s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    assert!(path.exists());
    assert_eq!(s.doc().unwrap().title(), "運動会カード（最終版）・2024.vectorcraft", "the tab names the file as typed");
    s.execute("document.open", &json!({"path": path.to_string_lossy()})).unwrap();
    let d = s.doc().unwrap();
    assert_eq!(d.path.as_deref(), Some(path.to_string_lossy().as_ref()));
    let mut found = false;
    d.doc.walk(|n| found |= matches!(&n.kind, vectorcraft_doc::NodeKind::Text(t) if t.plain_text() == "日本語のテキスト"));
    assert!(found, "the Japanese text comes back");
    for (format, name) in [("svg", "書き出し.svg"), ("png", "書き出し.png"), ("pdf", "書き出し.pdf")] {
        let out = dir.join(name);
        s.execute("document.export", &json!({"format": format, "path": out.to_string_lossy()})).unwrap();
        assert!(std::fs::metadata(&out).is_ok_and(|m| m.len() > 0), "{name}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
