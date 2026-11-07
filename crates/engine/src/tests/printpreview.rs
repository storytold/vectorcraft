//! `print.preview`'s pages say where the art lands (what the Print dialog's preview draws): the
//! transform from the document onto the page, the region each page prints and its trim box.

use serde_json::{Value, json};

use crate::*;

/// `print.preview {settings}`'s first page.
fn first_sheet(s: &mut Session, settings: Value) -> Value {
    s.execute("print.preview", &json!({ "settings": settings })).unwrap()["sheets"][0].clone()
}

/// `[a, b, c, d, e, f]` applied to (x, y).
fn apply(t: &Value, (x, y): (f64, f64)) -> (f64, f64) {
    let n = |i: usize| t[i].as_f64().unwrap();
    (n(0) * x + n(2) * y + n(4), n(1) * x + n(3) * y + n(5))
}

fn close(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

#[test]
fn pages_place_the_artboard_on_the_trim_box() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("document.setup", &json!({"bleed": 9})).unwrap();
    // Centred on Letter, turned landscape for the wide artboard.
    let p = first_sheet(&mut s, json!({}));
    assert_eq!((p["width"].clone(), p["height"].clone()), (json!(792.0), json!(612.0)));
    assert_eq!(p["trim"], json!([246.0, 206.0, 546.0, 406.0]));
    assert!(close(apply(&p["transform"], (0.0, 0.0)), (246.0, 206.0)) && close(apply(&p["transform"], (300.0, 200.0)), (546.0, 406.0)));
    assert_eq!(p["area"], json!([-9.0, -9.0, 309.0, 209.0]), "the artboard with the document's bleed");
    // Flipped, the artboard's top-left corner lands on the trim box's bottom-right one.
    let p = first_sheet(
        &mut s,
        json!({"autoRotate": false, "orientation": "landscapeFlipped", "scaling": "custom", "scale": {"width": 50, "height": 50}}),
    );
    let t = &p["trim"];
    assert!(close(apply(&p["transform"], (0.0, 0.0)), (t[2].as_f64().unwrap(), t[3].as_f64().unwrap())), "{p}");
    assert_eq!(p["scale"], json!([50.0, 50.0]));
    // A tile prints its part of the artboard.
    let v = s.execute("print.preview", &json!({"settings": {"scaling": "tileFull", "scale": {"width": 400, "height": 400}}})).unwrap();
    let tiles = v["tiles"][0]["tiles"].as_array().unwrap();
    for (sheet, tile) in v["sheets"].as_array().unwrap().iter().zip(tiles) {
        let (a, t) = (&sheet["area"], tile);
        for i in 0..4 {
            let inside =
                if i < 2 { a[i].as_f64().unwrap() >= t[i].as_f64().unwrap() - 1e-6 } else { a[i].as_f64().unwrap() <= t[i].as_f64().unwrap() + 1e-6 };
            assert!(inside, "{a} within {t}");
        }
    }
}

#[test]
fn a_print_preset_lays_out_its_settings_with_settings_over_them() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": 4})).unwrap();
    s.execute("print.setup", &json!({"settings": {"media": "a4"}})).unwrap();
    s.execute("print.presets.save", &json!({"name": "Sheet", "settings": {"artboards": "range", "range": "4"}})).unwrap();
    let v = s.execute("print.preview", &json!({"preset": "Sheet", "settings": {"artboards": "range", "range": "3"}})).unwrap();
    assert_eq!((v["pages"].clone(), v["sheets"][0]["artboard"].clone()), (json!(1), json!(2)), "{v}");
    assert_eq!(v["settings"]["media"], "letter", "the preset's paper, not the document's");
    let v = s.execute("print.preview", &json!({"preset": "Sheet"})).unwrap();
    assert_eq!((v["pages"].clone(), v["sheets"][0]["artboard"].clone()), (json!(1), json!(3)));
    assert_eq!(s.doc().unwrap().doc.print_setup.as_ref().unwrap()["media"], "a4", "the preview keeps nothing");
}

/// Point type at (`x`, `y`) in `family` → its id.
fn type_in(s: &mut Session, x: f64, y: f64, family: &str) -> u64 {
    s.execute("text.create", &json!({"x": x, "y": y, "text": "Ab", "font": family, "style": "Regular"})).unwrap()["id"].as_u64().unwrap()
}

/// The warnings in `v` about type in the fallback font.
fn fallback_warnings(v: &Value) -> Vec<&str> {
    v["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|w| w.contains("fallback font")).collect()
}

/// `print.preview {settings}`'s warnings about type in the fallback font.
fn preview_fallback(s: &mut Session, settings: Value) -> Vec<String> {
    let v = s.execute("print.preview", &json!({ "settings": settings })).unwrap();
    fallback_warnings(&v).into_iter().map(str::to_string).collect()
}

#[test]
fn type_that_prints_in_the_fallback_font_is_named() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": 2})).unwrap();
    type_in(&mut s, 10.0, 50.0, vectorcraft_text::FALLBACK_FAMILY);
    assert_eq!(preview_fallback(&mut s, json!({})), Vec::<String>::new(), "an installed family");
    // The warning PDF export gives.
    let id = type_in(&mut s, 10.0, 100.0, "Nonexistent Sans Probe");
    let export = s.execute("document.exportPdf", &json!({})).unwrap();
    let warned = preview_fallback(&mut s, json!({}));
    assert_eq!(warned, fallback_warnings(&export));
    assert!(warned.first().is_some_and(|w| w.contains("Nonexistent Sans Probe")), "{warned:?}");
    // A hidden object doesn't print.
    s.execute("object.setProps", &json!({"ids": [id], "visible": false})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({})), Vec::<String>::new());
}

#[test]
fn fallback_fonts_count_only_layers_and_artboards_that_print() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": 2})).unwrap();
    // On the second artboard, the type prints only when that artboard does.
    let r = s.doc().unwrap().doc.artboards[1].rect;
    type_in(&mut s, r.x0 + 10.0, r.y0 + 50.0, "Nonexistent Sans Probe");
    assert_eq!(preview_fallback(&mut s, json!({"artboards": "range", "range": "1"})), Vec::<String>::new());
    assert_eq!(preview_fallback(&mut s, json!({"artboards": "range", "range": "2"})).len(), 1);
    assert_eq!(preview_fallback(&mut s, json!({})).len(), 1);
    // On a hidden layer, the type prints only with Print Layers: All.
    let layer = s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap();
    type_in(&mut s, 10.0, 50.0, "Nonexistent Sans Probe");
    s.execute("layer.setProps", &json!({"id": layer, "visible": false})).unwrap();
    for (layers, warns) in [("visiblePrintable", false), ("visible", false), ("all", true)] {
        let warned = preview_fallback(&mut s, json!({"artboards": "range", "range": "1", "printLayers": layers}));
        assert_eq!(!warned.is_empty(), warns, "{layers}");
    }
    // Template layers never print.
    s.execute("layer.setProps", &json!({"id": layer, "template": true})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({"artboards": "range", "range": "1", "printLayers": "all"})), Vec::<String>::new());
}

#[test]
fn type_that_does_not_fit_its_frame_counts_only_where_it_shows() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let area = json!({"width": 100, "height": 20});
    let p = json!({"x": 10, "y": 10, "text": "Ab\nCd\nEf\nGh", "font": vectorcraft_text::FALLBACK_FAMILY, "size": 12, "area": area});
    let id = s.execute("text.create", &p).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setStyle", &json!({"ids": [id], "font": "Nonexistent Sans Probe", "start": 9, "end": 11})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({})), Vec::<String>::new(), "only the first line shows");
    s.execute("text.areaOptions", &json!({"ids": [id], "height": 150})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({})).len(), 1);
    // Nor does the pattern of characters that don't show.
    let tile = type_in(&mut s, 0.0, 15.0, "Pattern Probe Sans");
    s.execute("select.set", &json!({"ids": [tile]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Words", "width": 40, "height": 40})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [tile]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    s.execute("text.setStyle", &json!({"ids": [id], "font": vectorcraft_text::FALLBACK_FAMILY})).unwrap();
    s.execute("text.setStyle", &json!({"ids": [id], "tracking": 10, "start": 9, "end": 11})).unwrap();
    s.edit("pattern", |d, _| {
        if let vectorcraft_doc::NodeKind::Text(t) = &mut d.node_mut(vectorcraft_doc::NodeId(id)).unwrap().kind {
            t.runs.last_mut().unwrap().style.fill = vectorcraft_color::Paint::Pattern { pattern: "Words".into(), xf: Default::default() };
        }
        Ok(())
    })
    .unwrap();
    assert!(preview_fallback(&mut s, json!({}))[0].contains("Pattern Probe Sans"));
    s.execute("text.areaOptions", &json!({"ids": [id], "height": 20})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({})), Vec::<String>::new());
}

/// Area type 100 × 20 pt at (`x`, `y`) holding four lines, of which the first shows → its id.
fn overset(s: &mut Session, x: f64, y: f64) -> u64 {
    let v = s.execute("text.create", &json!({"x": x, "y": y, "text": "Ab\nCd\nEf\nGh", "size": 12, "area": {"width": 100, "height": 20}})).unwrap();
    assert_eq!(v["overflow"], json!([v["id"]]), "{v}");
    v["id"].as_u64().unwrap()
}

/// `print.preview {settings}`'s warnings about type that doesn't fit its frame.
fn preview_overflow(s: &mut Session, settings: Value) -> Vec<String> {
    let v = s.execute("print.preview", &json!({ "settings": settings })).unwrap();
    let over = |w: &&str| w.contains("fit its frame") || w.contains("fit their frames");
    v["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(over).map(str::to_string).collect()
}

#[test]
fn type_that_does_not_fit_its_frame_is_named() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let id = overset(&mut s, 10.0, 10.0);
    let one = format!("1 text object doesn't fit its frame: the text past the end of the frame doesn't print (id {id})");
    assert_eq!(preview_overflow(&mut s, json!({})), [one]);
    // A frame that holds its text, or a hidden object, leaves nothing out.
    s.execute("text.areaOptions", &json!({"ids": [id], "height": 150})).unwrap();
    assert_eq!(preview_overflow(&mut s, json!({})), Vec::<String>::new());
    s.execute("text.areaOptions", &json!({"ids": [id], "height": 20})).unwrap();
    s.execute("object.setProps", &json!({"ids": [id], "visible": false})).unwrap();
    assert_eq!(preview_overflow(&mut s, json!({})), Vec::<String>::new());
    s.execute("object.setProps", &json!({"ids": [id], "visible": true})).unwrap();
    // Type on a path that runs past the path's end.
    let path = s.execute("path.create", &json!({"d": "M10 150 L60 150"})).unwrap()["id"].clone();
    let on = s.execute("text.createInPath", &json!({"path": path, "mode": "onPath", "text": "Longer than its path"})).unwrap();
    assert_eq!(on["overflow"], json!([on["id"]]), "{on}");
    let both = format!("2 text objects don't fit their frames: the text past the end of each frame doesn't print (ids {id}, {})", on["id"]);
    assert_eq!(preview_overflow(&mut s, json!({})), [both]);
    // Past 20, the first 20 ids and how many more.
    let mut ids = vec![id, on["id"].as_u64().unwrap()];
    ids.extend((0..20).map(|i| overset(&mut s, 150.0, f64::from(i) * 5.0)));
    let named: Vec<String> = ids[..20].iter().map(u64::to_string).collect();
    let many =
        format!("22 text objects don't fit their frames: the text past the end of each frame doesn't print (ids {} and 2 more)", named.join(", "));
    assert_eq!(preview_overflow(&mut s, json!({})), [many]);
}

#[test]
fn type_that_does_not_fit_counts_only_layers_and_artboards_that_print() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": 2})).unwrap();
    let r = s.doc().unwrap().doc.artboards[1].rect;
    overset(&mut s, r.x0 + 10.0, r.y0 + 10.0);
    assert_eq!(preview_overflow(&mut s, json!({"artboards": "range", "range": "1"})), Vec::<String>::new());
    assert_eq!(preview_overflow(&mut s, json!({"artboards": "range", "range": "2"})).len(), 1);
    // On a hidden layer, the type prints only with Print Layers: All.
    let layer = s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap();
    overset(&mut s, 10.0, 10.0);
    s.execute("layer.setProps", &json!({"id": layer, "visible": false})).unwrap();
    for (layers, warns) in [("visiblePrintable", false), ("visible", false), ("all", true)] {
        let warned = preview_overflow(&mut s, json!({"artboards": "range", "range": "1", "printLayers": layers}));
        assert_eq!(!warned.is_empty(), warns, "{layers}");
    }
    // A layer that doesn't print.
    s.execute("layer.setProps", &json!({"id": layer, "visible": true, "printable": false})).unwrap();
    assert_eq!(preview_overflow(&mut s, json!({"artboards": "range", "range": "1"})), Vec::<String>::new());
    assert_eq!(preview_overflow(&mut s, json!({"artboards": "range", "range": "1", "printLayers": "visible"})).len(), 1);
}

#[test]
fn type_that_does_not_fit_is_named_only_among_the_objects_of_the_document() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let id = overset(&mut s, 10.0, 10.0);
    assert_eq!(preview_overflow(&mut s, json!({})).len(), 1);
    s.execute("symbol.new", &json!({"name": "Label", "ids": [id]})).unwrap();
    assert_eq!(preview_overflow(&mut s, json!({})), Vec::<String>::new(), "the art of a symbol");
}

#[test]
fn a_threaded_story_is_named_only_when_its_last_frame_does_not_hold_it() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let story = "The quick brown fox jumps over the lazy dog. ".repeat(8);
    let a = s.execute("text.create", &json!({"x": 10, "y": 10, "text": story, "area": {"width": 150, "height": 60}})).unwrap();
    assert_eq!(a["overflow"], json!([a["id"]]), "{a}");
    let b = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [a["id"], b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    assert_eq!(preview_overflow(&mut s, json!({})), Vec::<String>::new(), "the second frame holds the rest");
    s.execute("text.editRange", &json!({"id": a["id"], "start": 0, "end": 0, "insert": story.repeat(4)})).unwrap();
    let warned = preview_overflow(&mut s, json!({}));
    assert!(warned.len() == 1 && warned[0].ends_with(&format!("(id {b})")), "{warned:?}");
}

#[test]
fn type_of_an_artboard_larger_than_the_paper_counts_only_on_the_paper() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 2000, "height": 200})).unwrap();
    type_in(&mut s, 1800.0, 100.0, "Nonexistent Sans Probe");
    // At 100% the middle of the artboard prints on Letter paper.
    assert_eq!(preview_fallback(&mut s, json!({"scaling": "none"})), Vec::<String>::new());
    for scaling in ["fit", "tileFull"] {
        assert_eq!(preview_fallback(&mut s, json!({ "scaling": scaling })).len(), 1, "{scaling}");
    }
}

#[test]
fn symbol_instances_and_inline_graphics_print_the_type_of_their_symbol() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": 2})).unwrap();
    let r = s.doc().unwrap().doc.artboards[1].rect;
    let id = type_in(&mut s, r.x0 + 10.0, r.y0 + 50.0, "Nonexistent Sans Probe");
    s.execute("symbol.new", &json!({"name": "Label", "ids": [id]})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({"artboards": "range", "range": "1"})), Vec::<String>::new());
    assert_eq!(preview_fallback(&mut s, json!({"artboards": "range", "range": "2"})).len(), 1);
    // An inline graphic in type on the first artboard prints its symbol's art.
    let id = type_in(&mut s, 10.0, 50.0, vectorcraft_text::FALLBACK_FAMILY);
    s.execute("text.insertInline", &json!({"id": id, "at": 0, "symbol": "Label"})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({"artboards": "range", "range": "1"})).len(), 1);
}

#[test]
fn a_placed_document_prints_its_type() {
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 100, "height": 50})).unwrap();
    type_in(&mut src, 10.0, 30.0, "Nonexistent Sans Probe");
    let path = std::env::temp_dir().join(format!("vc-printfonts-{}.vectorcraft", std::process::id()));
    std::fs::write(&path, vectorcraft_format::save_file(&src.doc().unwrap().doc)).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("file.place", &json!({"path": path.to_string_lossy(), "at": [150, 100]})).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(preview_fallback(&mut s, json!({})).len(), 1);
}

#[test]
fn type_prints_where_effects_paint_it() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": 2})).unwrap();
    let r = s.doc().unwrap().doc.artboards[1].rect;
    let id = type_in(&mut s, r.x0 + 10.0, r.y0 + 50.0, "Nonexistent Sans Probe");
    // Onto the first artboard.
    s.execute("effect.apply", &json!({"effect": "distort.transform", "ids": [id], "params": {"moveH": -r.x0, "moveV": -r.y0}})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({"artboards": "range", "range": "1"})).len(), 1);
    assert_eq!(preview_fallback(&mut s, json!({"artboards": "range", "range": "2"})), Vec::<String>::new());
    // A group's shadow reaching the artboard prints the type above it.
    let id = type_in(&mut s, 10.0, -40.0, "Sans Probe Above");
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let group = s.execute("object.group", &json!({})).unwrap()["id"].clone();
    assert!(!preview_fallback(&mut s, json!({"artboards": "range", "range": "1"}))[0].contains("Sans Probe Above"));
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [group], "params": {"x": 0, "y": 60, "blur": 0}})).unwrap();
    assert!(preview_fallback(&mut s, json!({"artboards": "range", "range": "1"}))[0].contains("Sans Probe Above"));
    // A hidden effect leaves each member of a group printing where it lies.
    let shape = s.execute("shape.rectangle", &json!({"x": 10, "y": 120, "width": 20, "height": 20})).unwrap()["id"].clone();
    let id = type_in(&mut s, r.x0 + 10.0, r.y0 + 150.0, "Sans Probe Hidden");
    s.execute("select.set", &json!({"ids": [shape, id]})).unwrap();
    let both = s.execute("object.group", &json!({})).unwrap()["id"].clone();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [both]})).unwrap();
    s.execute("effect.setParams", &json!({"index": 0, "visible": false, "ids": [both], "item": null})).unwrap();
    assert!(!preview_fallback(&mut s, json!({"artboards": "range", "range": "1"}))[0].contains("Sans Probe Hidden"));
    // A group's own fill, drawn from the glyphs of the type in it and moved onto the artboard, prints it too.
    let id = type_in(&mut s, 10.0, -80.0, "Sans Probe Fill");
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let group = s.execute("object.group", &json!({})).unwrap()["id"].clone();
    assert!(!preview_fallback(&mut s, json!({"artboards": "range", "range": "1"}))[0].contains("Sans Probe Fill"));
    s.execute("appearance.addFill", &json!({})).unwrap();
    s.execute("effect.apply", &json!({"effect": "distort.transform", "ids": [group], "item": 0, "params": {"moveV": 100}})).unwrap();
    assert!(preview_fallback(&mut s, json!({"artboards": "range", "range": "1"}))[0].contains("Sans Probe Fill"));
}

#[test]
fn a_pattern_prints_the_type_of_its_tile() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let id = type_in(&mut s, 0.0, 15.0, "Nonexistent Sans Probe");
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Words", "width": 40, "height": 40})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({})), Vec::<String>::new(), "a pattern no object uses");
    let shape = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 200, "height": 100})).unwrap()["id"].clone();
    s.execute("paint.setFill", &json!({"ids": [shape], "swatch": "Words"})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({})).len(), 1);
    // A hidden fill paints nothing.
    let n = s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(shape.as_u64().unwrap())).unwrap().clone();
    let fill = n.appearance.items.iter().position(|i| i.is_fill()).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [shape], "index": fill, "visible": false})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({})), Vec::<String>::new());
}

#[test]
fn a_brushed_stroke_prints_the_type_of_its_brush() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let id = type_in(&mut s, 0.0, 15.0, "Nonexistent Sans Probe");
    s.execute("brush.new", &json!({"type": "scatter", "name": "Words", "ids": [id]})).unwrap();
    s.execute("brush.new", &json!({"type": "art", "name": "Bent", "ids": [id]})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    let shape = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 200, "height": 100})).unwrap()["id"].clone();
    assert_eq!(preview_fallback(&mut s, json!({})), Vec::<String>::new());
    // An art brush bends paths and leaves type out.
    s.execute("brush.apply", &json!({"name": "Bent", "ids": [shape]})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({})), Vec::<String>::new());
    s.execute("brush.apply", &json!({"name": "Words", "ids": [shape]})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({})).len(), 1);
}

#[test]
fn type_in_an_opacity_mask_prints_and_type_clipped_away_does_not() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let shape = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 200, "height": 100})).unwrap()["id"].clone();
    let mask = type_in(&mut s, 20.0, 60.0, "Nonexistent Sans Probe");
    s.execute("transparency.makeOpacityMask", &json!({"ids": [shape, mask]})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({})).len(), 1, "the mask's type");
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let id = type_in(&mut s, 10.0, 150.0, "Nonexistent Sans Probe");
    assert_eq!(preview_fallback(&mut s, json!({})).len(), 1);
    let clip = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [id, clip]})).unwrap();
    let group = s.execute("object.clippingMask.make", &json!({})).unwrap()["id"].clone();
    assert_eq!(preview_fallback(&mut s, json!({})), Vec::<String>::new(), "outside its clipping path");
    // Also in a symbol's art.
    s.execute("symbol.new", &json!({"name": "Clipped", "ids": [group]})).unwrap();
    assert_eq!(preview_fallback(&mut s, json!({})), Vec::<String>::new(), "outside its clipping path in a symbol");
}
