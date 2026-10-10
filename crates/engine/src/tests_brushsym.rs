//! Brushes and symbols: commands, tools, rendering and persistence.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::Rect;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn line(s: &mut Session) -> NodeId {
    let r = s.execute("shape.line", &json!({"x1": 50, "y1": 100, "x2": 250, "y2": 100}));
    match r {
        Ok(v) if v["id"].is_u64() => NodeId(v["id"].as_u64().unwrap()),
        _ => {
            let r = s.execute("path.freehand", &json!({"points": [[50, 100], [150, 100], [250, 100]]})).unwrap();
            NodeId(r["id"].as_u64().unwrap())
        }
    }
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn brush_of(s: &Session, id: NodeId) -> Option<String> {
    node(s, id).appearance.stroke().and_then(|st| st.brush.clone())
}

fn select(s: &mut Session, ids: &[NodeId]) {
    let ids = ids.to_vec();
    s.select(|_, sel| sel.set(ids)).unwrap();
}

fn close(a: Rect, b: Rect, tol: f64) -> bool {
    (a.x0 - b.x0).abs() < tol && (a.y0 - b.y0).abs() < tol && (a.x1 - b.x1).abs() < tol && (a.y1 - b.y1).abs() < tol
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    node(s, id).geometric_bounds().unwrap()
}

// ---------- brushes ----------

#[test]
fn brush_list_has_the_default_library() {
    let mut s = session();
    let l = s.execute("brush.list", &json!({})).unwrap();
    let names: Vec<&str> = l["brushes"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect();
    for n in ["3 pt. Round", "6 pt. Flat", "Charcoal", "Arrow", "Dots", "Chain", "Bristle Round"] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    let def = s.execute("brush.get", &json!({"name": "6 pt. Flat"})).unwrap();
    assert_eq!(def["type"], "calligraphic");
    assert_eq!(def["angle"], json!(45.0));
}

#[test]
fn apply_and_remove_brush_with_undo() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    s.execute("brush.apply", &json!({"name": "Charcoal"})).unwrap();
    assert_eq!(brush_of(&s, a).as_deref(), Some("Charcoal"));
    assert_eq!(s.execute("brush.list", &json!({})).unwrap()["current"], "Charcoal");
    s.execute("brush.remove", &json!({})).unwrap();
    assert_eq!(brush_of(&s, a), None);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(brush_of(&s, a).as_deref(), Some("Charcoal"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(brush_of(&s, a), None);
    assert!(s.execute("brush.apply", &json!({"name": "Nope"})).is_err());
}

#[test]
fn apply_adds_a_stroke_when_missing_and_reaches_into_groups() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    let b = rect(&mut s, 80.0, 10.0, 50.0, 50.0);
    s.execute("paint.setStroke", &json!({"color": null, "ids": [a.0]})).ok();
    select(&mut s, &[a, b]);
    s.execute("object.group", &json!({})).unwrap();
    s.execute("brush.apply", &json!({"name": "Dots"})).unwrap();
    assert_eq!(brush_of(&s, a).as_deref(), Some("Dots"));
    assert_eq!(brush_of(&s, b).as_deref(), Some("Dots"));
    assert!(!node(&s, a).appearance.stroke_paint().is_none());
}

#[test]
fn new_brushes_from_params_and_selection() {
    let mut s = session();
    let r = s.execute("brush.new", &json!({"type": "calligraphic", "name": "Nib", "params": {"angle": 20, "roundness": 30, "size": 8}})).unwrap();
    assert_eq!(r["name"], "Nib");
    assert_eq!(s.execute("brush.get", &json!({"name": "Nib"})).unwrap()["size"], json!(8.0));
    // Art brushes need art.
    s.select(|_, sel| sel.clear()).unwrap();
    assert!(s.execute("brush.new", &json!({"type": "art"})).is_err());
    let a = rect(&mut s, 0.0, 0.0, 40.0, 8.0);
    let r = s.execute("brush.new", &json!({"type": "art", "name": "Bar"})).unwrap();
    assert_eq!(r["name"], "Bar");
    let r = s.execute("brush.new", &json!({"type": "pattern"})).unwrap();
    assert_eq!(r["name"], "New Pattern Brush");
    let def = s.execute("brush.get", &json!({"name": "New Pattern Brush"})).unwrap();
    assert!(def["side"].is_object());
    // The new art brush draws the rectangle along another path.
    let l = line(&mut s);
    s.execute("brush.apply", &json!({"name": "Bar", "ids": [l.0]})).unwrap();
    let g = s.execute("object.expandBrush", &json!({"ids": [l.0]})).unwrap();
    let gid = NodeId(g["ids"][0].as_u64().unwrap());
    let b = bounds(&s, gid);
    assert!((b.width() - 200.0).abs() < 0.5 && (b.height() - 8.0).abs() < 0.5, "{b:?}");
    let _ = a;
    // Undo removes the brush again; the library was not there before.
    assert!(s.execute("brush.new", &json!({"type": "nope"})).is_err());
}

#[test]
fn brush_options_rename_duplicate_and_delete_update_strokes() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    s.execute("brush.apply", &json!({"name": "3 pt. Round"})).unwrap();
    s.execute("brush.options", &json!({"name": "3 pt. Round", "params": {"size": 5}, "newName": "5 pt. Round"})).unwrap();
    assert_eq!(brush_of(&s, a).as_deref(), Some("5 pt. Round"));
    assert_eq!(s.execute("brush.get", &json!({"name": "5 pt. Round"})).unwrap()["size"], json!(5.0));
    assert_eq!(s.execute("brush.list", &json!({})).unwrap()["current"], "5 pt. Round");
    let d = s.execute("brush.duplicate", &json!({"name": "5 pt. Round"})).unwrap();
    assert_eq!(d["name"], "5 pt. Round copy");
    s.execute("brush.delete", &json!({"name": "5 pt. Round"})).unwrap();
    assert_eq!(brush_of(&s, a), None);
    assert!(s.execute("brush.get", &json!({"name": "5 pt. Round"})).is_err());
    assert!(s.execute("brush.delete", &json!({"name": "5 pt. Round"})).is_err());
}

#[test]
fn expand_brush_replaces_path_with_art_group() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("brush.apply", &json!({"name": "10 pt. Oval"})).unwrap();
    let r = s.execute("object.expandBrush", &json!({})).unwrap();
    let g = NodeId(r["ids"][0].as_u64().unwrap());
    assert!(s.doc().unwrap().doc.node(a).is_none());
    let gn = node(&s, g);
    assert!(matches!(gn.kind, NodeKind::Group { .. }));
    // White fill kept + the black nib outline.
    assert_eq!(gn.children().unwrap().len(), 2);
    let b = bounds(&s, g);
    assert!(b.width() > 104.0 && b.width() < 112.0, "{b:?}");
    assert_eq!(s.doc().unwrap().selection.objects, vec![g]);
    assert!(s.execute("object.expandBrush", &json!({})).is_err(), "nothing brushed left");
}

#[test]
fn brush_freehand_paints_with_a_brush_in_one_step() {
    let mut s = session();
    let n = s.doc().unwrap().history.undo.len();
    s.begin_interaction("Paintbrush").unwrap();
    s.preview("brush.freehand", &json!({"points": [[10, 10], [60, 40], [120, 10]], "style": "brush", "brush": "Charcoal"})).unwrap();
    s.commit_interaction().unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), n + 1);
    let id = s.doc().unwrap().selection.objects[0];
    assert_eq!(brush_of(&s, id).as_deref(), Some("Charcoal"));
}

#[test]
fn paintbrush_tool_uses_the_current_brush() {
    let mut s = session();
    s.execute("brush.setCurrent", &json!({"name": "Arrow"})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("paintbrush", v).unwrap();
    for (k, x, y) in
        [(PointerKind::Down, 10.0, 10.0), (PointerKind::Drag, 50.0, 30.0), (PointerKind::Drag, 90.0, 20.0), (PointerKind::Up, 130.0, 10.0)]
    {
        s.pointer(&PointerEvent::new(k, x, y), v).unwrap();
    }
    let id = s.doc().unwrap().selection.objects[0];
    assert_eq!(brush_of(&s, id).as_deref(), Some("Arrow"));
    // A tool option overrides the document's current brush.
    s.set_tool_option("brush", &json!("Dots"));
    for (k, x, y) in [(PointerKind::Down, 10.0, 110.0), (PointerKind::Drag, 50.0, 130.0), (PointerKind::Up, 130.0, 110.0)] {
        s.pointer(&PointerEvent::new(k, x, y), v).unwrap();
    }
    let id = s.doc().unwrap().selection.objects[0];
    assert_eq!(brush_of(&s, id).as_deref(), Some("Dots"));
    assert!(s.execute("brush.setCurrent", &json!({"name": "Nope"})).is_err());
}

/// A Size: Pressure variant of "3 pt. Round" (3 ± 3 pt).
fn pressure_round(s: &mut Session) {
    let params = json!({"variation": [0, 0, 3], "modes": ["fixed", "fixed", "pressure"]});
    s.execute("brush.options", &json!({"name": "3 pt. Round", "params": params})).unwrap();
}

fn pressure_of(s: &Session, id: NodeId) -> Option<vectorcraft_doc::PressureProfile> {
    node(s, id).appearance.stroke().and_then(|st| st.pressure.clone())
}

/// #852: the Paintbrush records the pen pressure on the stroke, the brush draws it, and it lasts
/// through saving and every export; a mouse stroke records none.
#[test]
fn paintbrush_records_pen_pressure_on_the_stroke() {
    let mut s = session();
    pressure_round(&mut s);
    s.execute("brush.setCurrent", &json!({"name": "3 pt. Round"})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("paintbrush", v).unwrap();
    let strokes = |s: &mut Session, y: f64, pressure: [f32; 4]| {
        let kinds = [PointerKind::Down, PointerKind::Drag, PointerKind::Drag, PointerKind::Up];
        for ((k, x), p) in kinds.into_iter().zip([20.0, 120.0, 220.0, 320.0]).zip(pressure) {
            s.pointer(&PointerEvent { pressure: p, ..PointerEvent::new(k, x, y) }, v).unwrap();
        }
        let id = s.doc().unwrap().selection.objects[0];
        // Deselected, so the next stroke starts a path of its own.
        select(s, &[]);
        id
    };
    let pen = strokes(&mut s, 60.0, [0.0, 0.4, 0.8, 1.0]);
    let p = pressure_of(&s, pen).expect("pressure recorded");
    assert!(p.at(0.0) < 0.05 && p.at(1.0) > 0.95 && (p.at(0.5) - 0.6).abs() < 0.05, "{p:?}");
    let mouse = strokes(&mut s, 200.0, [1.0; 4]);
    assert_eq!(pressure_of(&s, mouse), None, "a mouse presses fully: nothing to record");
    // Drawn: thin where the pen pressed lightly, 6 pt where it pressed fully.
    let doc: Document = (*s.doc().unwrap().doc).clone();
    let art = |d: &Document, id: NodeId| vectorcraft_brush::node_pieces(d, d.node(id).unwrap()).unwrap();
    let heavy_end = vectorcraft_brush::pieces_bounds(&art(&doc, pen)).unwrap();
    assert!((heavy_end.height() - 6.0).abs() < 0.3, "{heavy_end:?}");
    let even = vectorcraft_brush::pieces_bounds(&art(&doc, mouse)).unwrap();
    assert!((even.height() - 3.0).abs() < 0.1, "no pressure draws the size itself: {even:?}");
    // Saved and reopened as it was.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(back.node(pen).unwrap().appearance.stroke().unwrap().pressure, Some(p));
    // SVG and PDF draw the brush through the same stroke code: without the pressure they differ.
    let mut flat = doc.clone();
    flat.node_mut(pen).unwrap().appearance.stroke_mut().unwrap().pressure = None;
    assert_ne!(vectorcraft_svg::export(&doc, &Default::default()), vectorcraft_svg::export(&flat, &Default::default()));
    let pdf = |d: &Document| {
        vectorcraft_pdf::export(d, &vectorcraft_pdf::PdfOptions { created: Some(0), ..vectorcraft_pdf::PdfOptions::uncompressed() }).unwrap()
    };
    assert_ne!(pdf(&doc), pdf(&flat));
}

/// Continuing a pressure stroke keeps its pressure where it was and adds the new stroke's after it.
#[test]
fn continuing_a_stroke_joins_its_pressure() {
    let mut s = session();
    let r = s.execute("path.freehand", &json!({"points": [[0, 50, 0], [100, 50, 1]], "style": "brush"})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("path.freehand", &json!({"points": [[100, 50, 1], [200, 50, 0]], "style": "brush", "extend": {"id": id.0, "end": "end"}})).unwrap();
    let p = pressure_of(&s, id).unwrap();
    assert!(p.at(0.0) < 0.05 && (p.at(0.5) - 1.0).abs() < 0.05 && p.at(1.0) < 0.05, "{p:?}");
    // Continued at its start: the new stroke comes first, running backwards from where it joined.
    let r = s.execute("path.freehand", &json!({"points": [[0, 150], [100, 150]], "style": "brush"})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("path.freehand", &json!({"points": [[0, 150, 1], [-100, 150, 0]], "style": "brush", "extend": {"id": id.0, "end": "start"}})).unwrap();
    let p = pressure_of(&s, id).unwrap();
    assert!(p.at(0.0) < 0.05 && (p.at(0.5) - 1.0).abs() < 0.05, "{p:?}");
    assert_eq!(p.at(1.0), vectorcraft_doc::PressureProfile::MID, "the old part had no pressure");
}

/// Pen pressure belongs to its path: new art, graphic styles and Brush Options' edits leave it.
#[test]
fn pressure_stays_with_its_own_path() {
    let mut s = session();
    s.execute("appearance.setNewArtBasic", &json!({"on": false})).unwrap();
    let r = s.execute("brush.freehand", &json!({"points": [[0, 50, 0.2], [100, 50, 0.9]], "style": "brush", "brush": "3 pt. Round"})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    assert!(pressure_of(&s, id).is_some());
    select(&mut s, &[id]);
    s.execute("graphicStyle.new", &json!({"name": "Inked"})).unwrap();
    let style = s.doc().unwrap().doc.graphic_style("Inked").unwrap().appearance.clone();
    assert!(style.stroke().is_some_and(|st| st.pressure.is_none()));
    let other = rect(&mut s, 10.0, 100.0, 50.0, 50.0);
    assert_eq!(pressure_of(&s, other), None, "new art takes the look, not the pressure");
    pressure_round(&mut s);
    assert!(pressure_of(&s, id).is_some(), "editing the brush keeps the strokes' pressure");
}

/// Brush Options sets the variation modes; bad modes are refused, never a panic.
#[test]
fn brush_options_set_variation_modes() {
    let mut s = session();
    pressure_round(&mut s);
    let b = s.execute("brush.get", &json!({"name": "3 pt. Round"})).unwrap();
    assert_eq!(b["modes"], json!(["fixed", "fixed", "pressure"]));
    assert_eq!(b["variation"], json!([0.0, 0.0, 3.0]));
    for bad in [json!({"modes": ["fixed", "fixed", "tilt"]}), json!({"modes": "pressure"}), json!({"variation": "x"}), json!({"modes": [1, 2, 3]})] {
        assert!(s.execute("brush.options", &json!({"name": "3 pt. Round", "params": bad})).is_err(), "{bad}");
    }
    assert_eq!(s.execute("brush.get", &json!({"name": "3 pt. Round"})).unwrap(), b, "a refused edit changes nothing");
}

/// The bounds of the brush art `id`'s strokes draw.
fn brushed_bounds(s: &Session, id: NodeId) -> Rect {
    let st = s.doc().unwrap();
    let pieces = vectorcraft_brush::node_pieces(&st.doc, &node(s, id)).expect("brushed");
    vectorcraft_brush::pieces_bounds(&pieces).expect("draws")
}

/// Scatter, Art and Pattern Brush Options: the params reach the brush and every stroke painted
/// with it redraws; values out of range are clamped, unknown choices refused.
#[test]
fn scatter_art_and_pattern_brush_options_update_strokes() {
    let mut s = session();
    // A rectangle as an art brush (straight edges: no flattening between sizes).
    let r = rect(&mut s, 10.0, 200.0, 40.0, 8.0);
    let bar = s.execute("brush.new", &json!({"type": "art", "name": "Bar", "ids": [r.0]})).unwrap()["name"].as_str().unwrap().to_string();
    let id = line(&mut s);
    select(&mut s, &[id]);
    let edit = |s: &mut Session, name: &str, params: Value| -> f64 {
        s.execute("brush.apply", &json!({ "name": name })).unwrap();
        let before = brushed_bounds(s, id).height();
        s.execute("brush.options", &json!({ "name": name, "params": params })).unwrap();
        brushed_bounds(s, id).height() / before
    };
    // Art: Width 300 % draws the art three times as wide across the path.
    let art = json!({"width": 300, "direction": "rightToLeft", "flip_across": true});
    assert!((edit(&mut s, &bar, art) - 3.0).abs() < 1e-6);
    s.execute("brush.options", &json!({"name": bar, "params": {"scale": {"mode": "betweenGuides", "start": 0.2, "end": 0.6}}})).unwrap();
    let b = s.execute("brush.get", &json!({ "name": bar })).unwrap();
    assert_eq!(
        (&b["direction"], &b["flip_across"], &b["scale"]),
        (&json!("rightToLeft"), &json!(true), &json!({"mode": "betweenGuides", "start": 0.2, "end": 0.6}))
    );
    // Pattern: Scale 200 % doubles the tiles.
    assert!((edit(&mut s, "Stitches", json!({"scale": 200, "spacing": 50, "fit": "addSpace", "flip_along": true})) - 2.0).abs() < 0.05);
    let b = s.execute("brush.get", &json!({"name": "Stitches"})).unwrap();
    assert_eq!((&b["spacing"], &b["fit"]), (&json!(50.0), &json!("addSpace")));
    // Scatter: a fixed Size of 200 % doubles the dots; the modes and Hue Shift are kept.
    let key = json!({"model": "rgb", "r": 0.0, "g": 0.0, "b": 0.0});
    let scatter = json!({"size": [200, 50], "modes": ["fixed", "fixed", "fixed", "random"], "rotation_relative_to_path": true, "colorization": {"method": "hueShift", "key": key}});
    assert!((edit(&mut s, "Dots", scatter) - 2.0).abs() < 0.05);
    let b = s.execute("brush.get", &json!({"name": "Dots"})).unwrap();
    assert_eq!(
        (&b["modes"], &b["colorization"]["method"], &b["colorization"]["key"]),
        (&json!(["fixed", "fixed", "fixed", "random"]), &json!("hueShift"), &key)
    );
    // Out of range: clamped. Unknown choices: refused, nothing changes.
    let junk = json!({"size": [1e308, -1e308], "spacing": [0, 1e308], "rotation": [720, -1e9], "scatter": [-1e308, 5]});
    s.execute("brush.options", &json!({"name": "Dots", "params": junk})).unwrap();
    let b = s.execute("brush.get", &json!({"name": "Dots"})).unwrap();
    assert_eq!(
        (&b["size"], &b["spacing"], &b["rotation"], &b["scatter"]),
        (&json!([10000.0, 1.0]), &json!([1.0, 10000.0]), &json!([180.0, -180.0]), &json!([-1000.0, 5.0]))
    );
    assert!(brushed_bounds(&s, id).is_finite());
    for (name, bad) in [
        ("Dots", json!({"modes": ["fixed", "tilt", "fixed", "fixed"]})),
        ("Dots", json!({"modes": ["fixed"]})),
        ("Dots", json!({"colorization": {"method": "sparkle"}})),
        ("Arrow", json!({"direction": "diagonal"})),
        ("Arrow", json!({"width": "wide"})),
        ("Chain", json!({"fit": "squeeze"})),
    ] {
        let before = s.execute("brush.get", &json!({ "name": name })).unwrap();
        assert!(s.execute("brush.options", &json!({ "name": name, "params": bad })).is_err(), "{bad}");
        assert_eq!(s.execute("brush.get", &json!({ "name": name })).unwrap(), before);
    }
    s.execute("brush.options", &json!({"name": "Chain", "params": {"scale": 1e308, "spacing": -1e308}})).unwrap();
    let b = s.execute("brush.get", &json!({"name": "Chain"})).unwrap();
    assert_eq!((&b["scale"], &b["spacing"]), (&json!(10000.0), &json!(0.0)));
}

// ---------- symbols ----------

fn make_symbol(s: &mut Session) -> (NodeId, String, Rect) {
    let a = rect(s, 100.0, 50.0, 80.0, 40.0);
    let b = bounds(s, a);
    let r = s.execute("symbol.new", &json!({"name": "Box"})).unwrap();
    (NodeId(r["id"].as_u64().unwrap()), r["name"].as_str().unwrap().to_string(), b)
}

#[test]
fn new_symbol_replaces_selection_with_instance_of_same_bounds() {
    let mut s = session();
    let (inst, name, b) = make_symbol(&mut s);
    assert_eq!(name, "Box");
    let n = node(&s, inst);
    assert!(matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Box"));
    assert!(close(n.geometric_bounds().unwrap(), b, 1e-9), "{:?} vs {b:?}", n.geometric_bounds());
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.symbols.len(), 1);
    assert_eq!(d.layers[0].children().unwrap().len(), 1);
    let l = s.execute("symbol.list", &json!({})).unwrap();
    assert_eq!(l["symbols"][0]["instances"], 1);
    assert_eq!(l["symbols"][0]["size"], json!([80.0, 40.0]));
    // A second symbol gets a unique name.
    rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    assert_eq!(s.execute("symbol.new", &json!({"name": "Box"})).unwrap()["name"], "Box 2");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.symbols.len(), 1);
}

#[test]
fn place_and_break_link_round_trip_geometry() {
    let mut s = session();
    let (_, name, _) = make_symbol(&mut s);
    let r = s.execute("symbol.place", &json!({"name": name, "x": 300, "y": 200})).unwrap();
    let p = NodeId(r["id"].as_u64().unwrap());
    assert!(close(bounds(&s, p), Rect::new(260.0, 180.0, 340.0, 220.0), 1e-9), "{:?}", bounds(&s, p));
    let r = s.execute("symbol.breakLink", &json!({"ids": [p.0]})).unwrap();
    let art = NodeId(r["ids"][0].as_u64().unwrap());
    let n = node(&s, art);
    assert!(matches!(n.kind, NodeKind::Path { .. }));
    assert!(close(n.geometric_bounds().unwrap(), Rect::new(260.0, 180.0, 340.0, 220.0), 1e-6));
    assert_eq!(n.appearance.stroke_width(), 1.0, "strokes are not scaled by normalisation");
}

#[test]
fn edit_then_update_redefines_all_instances() {
    let mut s = session();
    let (inst, name, _) = make_symbol(&mut s);
    let other = NodeId(s.execute("symbol.place", &json!({"x": 300, "y": 200})).unwrap()["id"].as_u64().unwrap());
    select(&mut s, &[inst]);
    let r = s.execute("symbol.edit", &json!({})).unwrap();
    assert_eq!(r["name"], name);
    let art = NodeId(r["ids"][0].as_u64().unwrap());
    // Make the art twice as wide, then redefine.
    s.execute("object.transform", &json!({"ids": [art.0], "matrix": [2, 0, 0, 1, -140, 0]})).ok();
    let w = bounds(&s, art).width();
    let r = s.execute("symbol.update", &json!({"name": name})).unwrap();
    let new_inst = NodeId(r["id"].as_u64().unwrap());
    assert!((bounds(&s, new_inst).width() - w).abs() < 1e-6);
    assert!((bounds(&s, other).width() - w).abs() < 1e-6, "other instances pick up the new size");
    assert!((bounds(&s, other).center().x - 300.0).abs() < 1e-6);
}

#[test]
fn delete_duplicate_and_replace_symbols() {
    let mut s = session();
    let (inst, name, _) = make_symbol(&mut s);
    let d = s.execute("symbol.duplicate", &json!({"name": name})).unwrap();
    assert_eq!(d["name"], "Box copy");
    rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    s.execute("symbol.new", &json!({"name": "Small"})).unwrap();
    select(&mut s, &[inst]);
    s.execute("symbol.replace", &json!({"name": "Small"})).unwrap();
    assert!(matches!(&node(&s, inst).kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Small"));
    assert!(close(bounds(&s, inst), Rect::new(130.0, 60.0, 150.0, 80.0), 1e-6), "{:?}", bounds(&s, inst));
    s.execute("symbol.delete", &json!({"name": "Small"})).unwrap();
    let doc = &s.doc().unwrap().doc;
    assert!(doc.symbols.iter().all(|x| x.name != "Small"));
    assert!(doc.node(inst).is_none());
    let mut plain = 0;
    doc.walk(|n| plain += matches!(n.kind, NodeKind::Path { .. }) as i32);
    assert_eq!(plain, 2, "both instances of Small were expanded");
    assert!(s.execute("symbol.delete", &json!({"name": "Small"})).is_err());
}

#[test]
fn sprayer_command_creates_a_symbol_set_and_alt_removes() {
    let mut s = session();
    make_symbol(&mut s);
    s.select(|_, sel| sel.clear()).unwrap();
    let pts: Vec<Value> = (0..20).map(|i| json!([20.0 + i as f64 * 15.0, 150.0])).collect();
    let r = s.execute("symbol.spray", &json!({"points": pts, "radius": 10, "density": 8})).unwrap();
    let set = NodeId(r["id"].as_u64().unwrap());
    let n = r["count"].as_u64().unwrap() as usize;
    assert!(n >= 3, "{n}");
    let g = node(&s, set);
    assert_eq!(g.name.as_deref(), Some("Symbol Set"));
    assert_eq!(g.children().unwrap().len(), n);
    // Spraying again with the set selected adds to it.
    s.execute("symbol.spray", &json!({"points": [[200, 250]]})).unwrap();
    assert_eq!(node(&s, set).children().unwrap().len(), n + 1);
    let r = s.execute("symbol.spray", &json!({"points": [[200, 250]], "radius": 60, "alt": true})).unwrap();
    assert!(r["count"].as_u64().unwrap() >= 1);
    assert_eq!(node(&s, set).children().unwrap().len(), n + 1 - r["count"].as_u64().unwrap() as usize);
}

#[test]
fn symbolism_adjustments() {
    let mut s = session();
    let (inst, _, b) = make_symbol(&mut s);
    let c = b.center();
    let at = json!([[c.x, c.y]]);
    s.execute("symbol.adjust", &json!({"tool": "size", "points": at, "radius": 50, "intensity": 10})).unwrap();
    assert!(bounds(&s, inst).width() > b.width() + 1.0);
    s.execute("symbol.adjust", &json!({"tool": "screen", "points": at, "intensity": 10})).unwrap();
    assert!(node(&s, inst).opacity < 1.0);
    s.execute("symbol.adjust", &json!({"tool": "stain", "points": at, "color": "#ff0000", "intensity": 10})).unwrap();
    let f = node(&s, inst).appearance.fill().cloned().unwrap();
    assert!(f.opacity > 0.0 && f.paint.color().unwrap().to_hex() == "#ff0000");
    s.execute("symbol.adjust", &json!({"tool": "shift", "points": [[c.x, c.y], [c.x + 20.0, c.y]], "intensity": 10})).unwrap();
    assert!(bounds(&s, inst).center().x > c.x + 5.0);
    let before = bounds(&s, inst);
    s.execute("symbol.adjust", &json!({"tool": "spin", "points": at, "intensity": 10})).unwrap();
    assert!(!close(bounds(&s, inst), before, 1e-6), "spin rotates");
    let far = s.execute("symbol.adjust", &json!({"tool": "size", "points": [[0, 290]], "radius": 5})).unwrap();
    assert_eq!(far["count"], 0);
    assert!(s.execute("symbol.adjust", &json!({"tool": "nope", "points": at})).is_err());
}

#[test]
fn symbol_sprayer_tool_gesture_is_one_undo_step() {
    let mut s = session();
    make_symbol(&mut s);
    s.select(|_, sel| sel.clear()).unwrap();
    let n = s.doc().unwrap().history.undo.len();
    let v = ViewInfo::default();
    s.select_tool("symbolSprayer", v).unwrap();
    let mut evs = vec![PointerEvent::new(PointerKind::Down, 50.0, 200.0)];
    for i in 1..15 {
        evs.push(PointerEvent::new(PointerKind::Drag, 50.0 + i as f64 * 20.0, 200.0));
    }
    evs.push(PointerEvent::new(PointerKind::Up, 350.0, 200.0).with_mods(Mods::default()));
    for e in evs {
        s.pointer(&e, v).unwrap();
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), n + 1);
    let set = s.doc().unwrap().selection.objects[0];
    assert!(node(&s, set).children().unwrap().len() >= 3);
    // The sizer tool grows the sprayed instances.
    let first = node(&s, set).children().unwrap()[0].clone();
    let c = first.geometric_bounds().unwrap().center();
    s.select_tool("symbolSizer", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, c.x, c.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, c.x, c.y), v).unwrap();
    let after = s.doc().unwrap().doc.node(first.id).unwrap().geometric_bounds().unwrap();
    assert!(after.width() > first.geometric_bounds().unwrap().width());
}

// ---------- rendering & persistence ----------

#[test]
fn brushed_strokes_and_instances_render() {
    let mut s = session();
    make_symbol(&mut s);
    let l = line(&mut s);
    s.execute("brush.apply", &json!({"name": "Chain", "ids": [l.0]})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let img = vectorcraft_render::Renderer::new().render_region(&doc, Rect::new(0.0, 0.0, 400.0, 300.0), 1.0, true);
    let dark = |x: u32, y: u32| {
        let p = img.pixel(x, y);
        (p[0] as u32 + p[1] as u32 + p[2] as u32) < 300
    };
    // The instance's outline (the rectangle's black stroke) is drawn.
    assert!((45..=55).any(|y| dark(100, y)), "instance stroke rendered");
    // Chain links are thicker than the 1 pt stroke, with gaps.
    let hits = (50..250).filter(|x| (95..=105).any(|y| dark(*x, y))).count();
    assert!(hits > 100, "chain covers most of the line: {hits}");
}

#[test]
fn brushes_and_symbols_survive_vectorcraft_round_trip() {
    let mut s = session();
    make_symbol(&mut s);
    s.execute("brush.new", &json!({"type": "calligraphic", "name": "Mine", "params": {"size": 7}})).unwrap();
    let l = line(&mut s);
    s.execute("brush.apply", &json!({"name": "Mine", "ids": [l.0]})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let bytes = vectorcraft_format::save(&doc, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(vectorcraft_brush::library(&back), vectorcraft_brush::library(&doc));
    assert_eq!(vectorcraft_brush::find(&back, "Mine").map(|b| b.kind.type_id()), Some("calligraphic"));
    assert_eq!(back.symbols, doc.symbols);
    assert_eq!(back.unknown.get("symbolSizes"), doc.unknown.get("symbolSizes"));
    let bl = back.node(l).unwrap();
    assert_eq!(bl.appearance.stroke().unwrap().brush.as_deref(), Some("Mine"));
    // A reloaded document places instances at the natural size.
    let mut s2 = Session::new();
    s2.add_document(back, None);
    let r = s2.execute("symbol.place", &json!({"name": "Box", "x": 100, "y": 100})).unwrap();
    let b = s2.doc().unwrap().doc.node(NodeId(r["id"].as_u64().unwrap())).unwrap().geometric_bounds().unwrap();
    assert!(close(b, Rect::new(60.0, 80.0, 140.0, 120.0), 1e-9));
}
