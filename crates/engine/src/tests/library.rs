//! The Libraries panel's libraries (#745): making, renaming and deleting them, adding graphics,
//! colours and text styles from the selection, using them in another document, and keeping them
//! in files across sessions.

use serde_json::{Value, json};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn objects(s: &Session) -> usize {
    s.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len())
}

fn hex_of(s: &Session, id: &Value, stroke: bool) -> String {
    let n = s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().clone();
    let paint = if stroke { n.appearance.stroke().unwrap().paint.clone() } else { n.appearance.fill().unwrap().paint.clone() };
    paint.color().unwrap().to_hex()
}

#[test]
fn libraries_are_made_renamed_and_deleted() {
    let mut s = session();
    assert_eq!(run(&mut s, "library.list", json!({}))["libraries"], json!([]));
    let id = run(&mut s, "library.create", json!({"name": "Brand"}))["id"].as_str().unwrap().to_string();
    let other = run(&mut s, "library.create", json!({"name": "Brand"}))["id"].as_str().unwrap().to_string();
    assert_ne!(id, other, "two libraries of one name are two files");
    run(&mut s, "library.rename", json!({"library": other, "name": "Client"}));
    let list = run(&mut s, "library.list", json!({}));
    let names: Vec<&str> = list["libraries"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Brand", "Client"]);
    assert!(list["folder"].is_null(), "no folder: libraries last for the session");
    run(&mut s, "library.delete", json!({"library": "client"}));
    assert_eq!(run(&mut s, "library.list", json!({}))["libraries"].as_array().unwrap().len(), 1);
    assert!(s.execute("library.get", &json!({"library": "Client"})).is_err());
}

/// A library's file is named after it, and a Windows device name (`CON`, `NUL`, `COM1`…) gets a
/// `-` after it, on every platform.
#[test]
fn a_library_named_after_a_windows_device_gets_a_dash() {
    let mut s = session();
    let ids: Vec<String> = ["CON", "nul", "Com1", "LPT³", "Console", "COM10"]
        .iter()
        .map(|name| run(&mut s, "library.create", json!({"name": name}))["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, ["CON-", "nul-", "Com1-", "LPT³-", "Console", "COM10"]);
}

#[test]
fn a_graphic_goes_into_a_library_and_comes_out_in_another_document() {
    let mut s = session();
    let id = run(&mut s, "shape.ellipse", json!({"x": 10, "y": 10, "width": 40, "height": 30}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"color": "#cc3300", "ids": [id]}));
    run(&mut s, "select.set", json!({"ids": [id]}));
    // With no library yet, adding makes "My Library".
    let added = run(&mut s, "library.add", json!({"kind": "graphic"}));
    assert_eq!(added["existing"], false);
    let lib = added["library"].as_str().unwrap().to_string();
    let item = added["id"].as_str().unwrap().to_string();
    let got = run(&mut s, "library.get", json!({"library": lib}));
    assert_eq!(got["name"], "My Library");
    let g = &got["graphics"][0];
    // Its size as it draws: the 1 pt stroke included.
    assert!((g["width"].as_f64().unwrap() - 41.0).abs() < 1e-6 && (g["height"].as_f64().unwrap() - 31.0).abs() < 1e-6, "{g}");
    assert!(!g["thumbnail"].as_str().unwrap().is_empty(), "a thumbnail of the art");
    // The same art again is the item already there.
    assert_eq!(run(&mut s, "library.add", json!({"kind": "graphic"}))["existing"], true);
    // Placed in another document, centred where asked, selected, one undo step.
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    let placed = run(&mut s, "library.use", json!({"library": lib, "kind": "graphic", "item": item, "center": [150, 100]}));
    let new_id = placed["ids"][0].clone();
    let st = s.doc().unwrap();
    let b = st.doc.node(NodeId(new_id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap();
    assert!((b.center().x - 150.0).abs() < 1e-6 && (b.center().y - 100.0).abs() < 1e-6, "{b:?}");
    assert_eq!(st.selection.objects, vec![NodeId(new_id.as_u64().unwrap())]);
    assert_eq!(hex_of(&s, &new_id, false), "#cc3300");
    assert_eq!(objects(&s), 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(objects(&s), 0);
    // A graphic is removed by id.
    run(&mut s, "library.removeItem", json!({"library": lib, "kind": "graphic", "item": item}));
    assert!(s.execute("library.use", &json!({"library": lib, "kind": "graphic", "item": item})).is_err());
}

#[test]
fn colours_come_from_the_selection_and_paint_it() {
    let mut s = session();
    let lib = run(&mut s, "library.create", json!({}))["id"].as_str().unwrap().to_string();
    let a = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"color": "#123456", "ids": [a]}));
    run(&mut s, "paint.setStroke", json!({"color": "#abcdef", "ids": [a]}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    assert_eq!(run(&mut s, "library.add", json!({"library": lib, "kind": "fillColor"}))["name"], "#123456");
    assert_eq!(run(&mut s, "library.add", json!({"library": lib, "kind": "strokeColor"}))["name"], "#abcdef");
    assert_eq!(run(&mut s, "library.add", json!({"library": lib, "kind": "fillColor"}))["existing"], true);
    let b = run(&mut s, "shape.rectangle", json!({"x": 50, "y": 0, "width": 20, "height": 20}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [b]}));
    run(&mut s, "library.use", json!({"library": lib, "kind": "fillColor", "item": "#123456"}));
    run(&mut s, "library.use", json!({"library": lib, "kind": "fillColor", "item": "#123456", "to": "stroke"}));
    assert_eq!(hex_of(&s, &b, false), "#123456");
    assert_eq!(hex_of(&s, &b, true), "#123456");
    assert!(s.execute("library.use", &json!({"library": lib, "kind": "fillColor", "item": "#nope"})).is_err());
}

/// The fill (`stroke`: stroke) paint of object `id`.
fn paint_of(s: &Session, id: &Value, stroke: bool) -> vectorcraft_color::Paint {
    let n = s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().clone();
    if stroke { n.appearance.stroke().unwrap().paint.clone() } else { n.appearance.fill().unwrap().paint.clone() }
}

/// The gradient of paint `p`.
fn gradient(p: &vectorcraft_color::Paint) -> &vectorcraft_color::GradientPaint {
    match p {
        vectorcraft_color::Paint::Gradient(g) => g,
        other => panic!("not a gradient: {other:?}"),
    }
}

/// A gradient added from a placed fill keeps its stops, kind and angle without the placement, and
/// paints another object's fill or stroke with the same stops; a gradient swatch adds under its
/// name.
#[test]
fn a_gradient_goes_into_a_library_and_paints_another_object() {
    let mut s = session();
    let lib = run(&mut s, "library.create", json!({}))["id"].as_str().unwrap().to_string();
    let a = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 20}))["id"].clone();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 0.4, "color": "#00ff00", "opacity": 0.5, "midpoint": 0.3}, {"offset": 1, "color": "#0000ff"}]);
    let g = json!({"kind": "radial", "stops": stops, "start": [0, 0], "end": [40, 20]});
    run(&mut s, "paint.setFill", json!({"gradient": g, "ids": [a]}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    assert!(s.execute("library.add", &json!({"library": lib, "kind": "strokeGradient"})).is_err(), "its stroke is solid");
    let added = run(&mut s, "library.add", json!({"library": lib, "kind": "fillGradient"}));
    assert_eq!((added["name"].as_str(), added["existing"].as_bool()), (Some("Gradient"), Some(false)));
    assert_eq!(run(&mut s, "library.add", json!({"library": lib, "kind": "fillGradient"}))["existing"], true);
    let source = gradient(&paint_of(&s, &a, false)).clone();
    // In another document: a rectangle's fill, then its stroke.
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    let b = run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 80, "height": 80}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [b]}));
    let r = run(&mut s, "library.use", json!({"library": lib, "kind": "fillGradient", "item": "Gradient"}));
    assert_eq!(r["kind"], "fillGradient");
    let fill = gradient(&paint_of(&s, &b, false)).clone();
    assert_eq!(fill.gradient, source.gradient, "the same kind and stops");
    assert_eq!((fill.angle, fill.geom), (source.angle, None), "fitted to the object it paints");
    run(&mut s, "library.use", json!({"library": lib, "kind": "fillGradient", "item": "Gradient", "to": "stroke"}));
    assert_eq!(gradient(&paint_of(&s, &b, true)).gradient, source.gradient);
    // A gradient swatch adds under its name.
    run(
        &mut s,
        "swatch.new",
        json!({"name": "Sunset", "gradient": {"stops": [{"offset": 0, "color": "#ffcc00"}, {"offset": 1, "color": "#cc3300"}]}}),
    );
    assert_eq!(run(&mut s, "library.add", json!({"library": lib, "kind": "fillGradient", "swatch": "Sunset"}))["name"], "Sunset");
    assert!(s.execute("library.add", &json!({"library": lib, "kind": "fillGradient", "swatch": "White"})).is_err(), "not a gradient swatch");
    let got = run(&mut s, "library.get", json!({"library": lib}));
    let names: Vec<&str> = got["gradients"].as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Gradient", "Sunset"]);
    assert!(s.execute("library.use", &json!({"library": lib, "kind": "fillGradient", "item": "Nope"})).is_err());
}

/// A gradient added from an object that was rotated keeps the direction it shows there.
#[test]
fn a_gradient_from_rotated_art_keeps_its_direction() {
    let mut s = session();
    let lib = run(&mut s, "library.create", json!({}))["id"].as_str().unwrap().to_string();
    let a = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 20}))["id"].clone();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    run(&mut s, "paint.setFill", json!({"gradient": {"kind": "linear", "stops": stops}, "ids": [a]}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    run(&mut s, "object.rotate", json!({"angle": 90}));
    let shown = gradient(&paint_of(&s, &a, false)).geom.expect("placed by the rotation").angle_deg();
    assert!((shown.abs() - 90.0).abs() < 1e-6, "{shown}");
    run(&mut s, "library.add", json!({"library": lib, "kind": "fillGradient"}));
    let got = run(&mut s, "library.get", json!({"library": lib}));
    let angle = got["gradients"][0]["gradient"]["angle"].as_f64().unwrap();
    assert!((angle - shown).abs() < 1e-6, "{angle} vs {shown}");
}

/// Type's fill is its characters' paint: a gradient or a color on selected type is what it adds,
/// not the default for new art.
#[test]
fn type_adds_the_paint_of_its_characters() {
    let mut s = session();
    let lib = run(&mut s, "library.create", json!({}))["id"].as_str().unwrap().to_string();
    let t = run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Hi"}))["id"].clone();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    run(&mut s, "paint.setFill", json!({"gradient": {"kind": "linear", "stops": stops}, "ids": [t]}));
    run(&mut s, "paint.setStroke", json!({"color": "#00ff00", "ids": [t]}));
    // The defaults for new art differ from the type's paint.
    run(&mut s, "select.set", json!({"ids": []}));
    run(&mut s, "paint.setFill", json!({"color": "#123456"}));
    run(&mut s, "paint.setStroke", json!({"color": "#654321"}));
    run(&mut s, "select.set", json!({"ids": [t]}));
    run(&mut s, "library.add", json!({"library": lib, "kind": "fillGradient"}));
    run(&mut s, "library.add", json!({"library": lib, "kind": "strokeColor"}));
    let got = run(&mut s, "library.get", json!({"library": lib}));
    assert_eq!(got["gradients"][0]["gradient"]["gradient"]["stops"].as_array().map(Vec::len), Some(2), "{got}");
    assert_eq!(got["colors"][0]["name"].as_str().map(str::to_lowercase).as_deref(), Some("#00ff00"), "{got}");
    // Rotated type: the gradient keeps the direction it shows in the document.
    run(&mut s, "object.rotate", json!({"angle": 90}));
    let shown = s.doc().unwrap().doc.node(NodeId(t.as_u64().unwrap())).unwrap().proxy_gradient(false, None).unwrap().1.angle_deg();
    assert!((shown.abs() - 90.0).abs() < 1e-6, "{shown}");
    run(&mut s, "library.add", json!({"library": lib, "kind": "fillGradient"}));
    let got = run(&mut s, "library.get", json!({"library": lib}));
    let angle = got["gradients"].as_array().unwrap().last().unwrap()["gradient"]["angle"].as_f64().unwrap();
    assert!((angle - shown).abs() < 1e-6, "{angle} vs {shown}");
}

/// Pattern `name` made from object `id`, in a 20-point tile.
fn make_pattern(s: &mut Session, id: &Value, name: &str) {
    run(s, "select.set", json!({"ids": [id]}));
    run(s, "object.pattern.make", json!({"name": name, "width": 20, "height": 20}));
    run(s, "object.pattern.done", json!({}));
}

/// A pattern added from a fill comes with its art; used in a new document it defines the pattern
/// and its swatch and paints the selection in one undo step, used again it reuses that pattern,
/// and a document whose pattern of that name differs gets it under another name.
#[test]
fn a_pattern_item_defines_its_pattern_in_a_new_document() {
    let mut s = session();
    let lib = run(&mut s, "library.create", json!({}))["id"].as_str().unwrap().to_string();
    let dot = run(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"color": "#cc3300", "ids": [dot]}));
    make_pattern(&mut s, &dot, "Dots");
    let a = run(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 40, "height": 40}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"swatch": "Dots", "ids": [a]}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    assert!(s.execute("library.add", &json!({"library": lib, "kind": "strokePattern"})).is_err(), "its stroke is solid");
    let added = run(&mut s, "library.add", json!({"library": lib, "kind": "fillPattern"}));
    assert_eq!((added["name"].as_str(), added["existing"].as_bool()), (Some("Dots"), Some(false)));
    assert_eq!(run(&mut s, "library.add", json!({"library": lib, "kind": "fillPattern", "swatch": "Dots"}))["existing"], true);
    let got = run(&mut s, "library.get", json!({"library": lib}));
    assert!(!got["patterns"][0]["thumbnail"].as_str().unwrap().is_empty(), "a thumbnail of the pattern");
    let art = s.doc().unwrap().doc.pattern("Dots").unwrap().art.clone();
    // A new document has no pattern until the item is used.
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    let b = run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 80, "height": 80}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [b]}));
    let undo = s.doc().unwrap().history.undo.len();
    assert!(s.doc().unwrap().doc.pattern("Dots").is_none());
    let r = run(&mut s, "library.use", json!({"library": lib, "kind": "fillPattern", "item": "Dots"}));
    assert_eq!((r["kind"].as_str(), r["pattern"].as_str()), (Some("fillPattern"), Some("Dots")));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.pattern("Dots").unwrap().art.len(), art.len());
    assert!(d.swatch("Dots").is_some(), "its swatch comes along");
    assert!(matches!(paint_of(&s, &b, false), vectorcraft_color::Paint::Pattern { ref pattern, .. } if pattern == "Dots"));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // Used again, on the stroke: the same pattern.
    run(&mut s, "library.use", json!({"library": lib, "kind": "fillPattern", "item": "Dots", "to": "stroke"}));
    assert!(matches!(paint_of(&s, &b, true), vectorcraft_color::Paint::Pattern { ref pattern, .. } if pattern == "Dots"));
    assert_eq!(s.doc().unwrap().doc.patterns.len(), 1);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().doc.pattern("Dots").is_none(), "undone with the paint");
    // Another document's own "Dots" stays; the library's comes in under another name.
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    let sq = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 6, "height": 6}))["id"].clone();
    // Not the library's pattern, which the default fill and stroke now name.
    run(&mut s, "paint.setFill", json!({"color": "#0044aa", "ids": [sq]}));
    run(&mut s, "paint.setStroke", json!({"none": true, "ids": [sq]}));
    make_pattern(&mut s, &sq, "Dots");
    let c = run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 80, "height": 80}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [c]}));
    let r = run(&mut s, "library.use", json!({"library": lib, "kind": "fillPattern", "item": "Dots"}));
    let renamed = r["pattern"].as_str().unwrap().to_string();
    assert_ne!(renamed, "Dots");
    assert!(matches!(paint_of(&s, &c, false), vectorcraft_color::Paint::Pattern { ref pattern, .. } if *pattern == renamed));
    assert_eq!(s.doc().unwrap().doc.patterns.len(), 2);
}

/// A pattern used in a document that already holds it paints with the pattern even when a color
/// swatch there has the pattern's name (an imported SVG pattern can be named `White`).
#[test]
fn a_pattern_item_paints_its_pattern_over_a_color_swatch_of_its_name() {
    let mut s = session();
    let lib = run(&mut s, "library.create", json!({}))["id"].as_str().unwrap().to_string();
    let dot = run(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"color": "#cc3300", "ids": [dot]}));
    make_pattern(&mut s, &dot, "Dots");
    let a = run(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 40, "height": 40}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"swatch": "Dots", "ids": [a]}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    run(&mut s, "library.add", json!({"library": lib, "kind": "fillPattern"}));
    // The swatch named `Dots` becomes a solid color; the pattern of that name stays.
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    for sw in &mut doc.swatches {
        if sw.name == "Dots" {
            sw.paint = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(0.0, 1.0, 0.0));
        }
    }
    assert!(doc.pattern("Dots").is_some());
    let b = run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 40, "height": 40}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [b]}));
    let r = run(&mut s, "library.use", json!({"library": lib, "kind": "fillPattern", "item": "Dots"}));
    let used = r["pattern"].as_str().unwrap().to_string();
    let paint = paint_of(&s, &b, false);
    assert!(matches!(paint, vectorcraft_color::Paint::Pattern { ref pattern, .. } if *pattern == used), "{paint:?}");
}

/// A pattern used in a document that holds it without a swatch (as an imported SVG can) gets its
/// swatch, in the same undo step as the paint.
#[test]
fn a_pattern_item_adds_the_swatch_of_a_pattern_the_document_holds_without_one() {
    let mut s = session();
    let lib = run(&mut s, "library.create", json!({}))["id"].as_str().unwrap().to_string();
    let dot = run(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"color": "#cc3300", "ids": [dot]}));
    make_pattern(&mut s, &dot, "Dots");
    let a = run(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 40, "height": 40}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"swatch": "Dots", "ids": [a]}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    run(&mut s, "library.add", json!({"library": lib, "kind": "fillPattern"}));
    let is_dots = |w: &vectorcraft_color::Swatch| matches!(&w.paint, vectorcraft_color::Paint::Pattern { pattern, .. } if pattern == "Dots");
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).swatches.retain(|w| !is_dots(w));
    let b = run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 40, "height": 40}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [b]}));
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "library.use", json!({"library": lib, "kind": "fillPattern", "item": "Dots"}));
    assert_eq!(s.doc().unwrap().doc.patterns.len(), 1, "the pattern there is reused");
    assert!(s.doc().unwrap().doc.swatches_iter().any(is_dots), "its swatch is added");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.doc().unwrap().doc.swatches_iter().any(is_dots), "undone with the paint");
}

/// Gradients and patterns go out to a library file and come back; read from a file, a gradient
/// with too few stops is dropped, one out of range is put in range, and a pattern whose data is
/// damaged or doesn't hold its pattern fails cleanly when used.
#[test]
fn library_gradients_and_patterns_round_trip_and_damaged_ones_fail_cleanly() {
    let mut s = session();
    run(&mut s, "library.create", json!({"name": "Brand"}));
    let a = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].clone();
    make_pattern(&mut s, &a, "Squares");
    run(
        &mut s,
        "swatch.new",
        json!({"name": "Sunset", "gradient": {"stops": [{"offset": 0, "color": "#ffcc00"}, {"offset": 1, "color": "#cc3300"}]}}),
    );
    run(&mut s, "library.add", json!({"kind": "fillPattern", "swatch": "Squares"}));
    run(&mut s, "library.add", json!({"kind": "fillGradient", "swatch": "Sunset"}));
    run(
        &mut s,
        "library.createGroup",
        json!({"name": "Fills", "items": [{"kind": "gradient", "item": "Sunset"}, {"kind": "pattern", "item": "Squares"}]}),
    );
    let data = run(&mut s, "library.export", json!({}))["data"].as_str().unwrap().to_string();
    let r = run(&mut s, "library.import", json!({"data": data}));
    assert_eq!((r["gradients"].as_u64(), r["patterns"].as_u64(), r["groups"].as_u64()), (Some(1), Some(1), Some(1)));
    let (x, y) = (run(&mut s, "library.get", json!({"library": "Brand"})), run(&mut s, "library.get", json!({"library": r["id"]})));
    for k in ["gradients", "patterns", "groups"] {
        assert_eq!(x[k], y[k], "{k}");
    }
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    run(&mut s, "library.use", json!({"library": r["id"], "kind": "fillPattern", "item": "Squares"}));
    assert!(s.doc().unwrap().doc.pattern("Squares").is_some(), "an imported pattern defines itself");
    let stop = |o: f64| json!({"offset": o, "color": {"model": "rgb", "r": 1, "g": 0, "b": 0}});
    let paint = |stops: Vec<Value>| json!({"gradient": {"kind": "Linear", "stops": stops}, "angle": 30});
    let doc = s.doc().unwrap().doc.clone();
    let empty = vectorcraft_format::base64_encode(&vectorcraft_format::save(&doc, false));
    let messy = json!({
        "name": "Messy",
        "gradients": [
            {"name": "One", "gradient": paint(vec![stop(0.0)])},
            {"name": "Wild", "gradient": paint(vec![stop(7.0), {let mut v = stop(-2.0); v["midpoint"] = json!(5); v["opacity"] = json!(-1); v}])}
        ],
        "patterns": [
            {"name": "Broken", "pattern": "P", "data": "%%%"},
            {"name": "Missing", "pattern": "Nowhere", "data": empty}
        ],
        "groups": [{"name": "G", "items": [{"kind": "gradient", "item": "One"}, {"kind": "gradient", "item": "Wild"}]}]
    });
    let r = run(&mut s, "library.import", json!({"data": messy.to_string()}));
    assert_eq!((r["gradients"].as_u64(), r["patterns"].as_u64()), (Some(1), Some(2)));
    let got = run(&mut s, "library.get", json!({}));
    let wild = &got["gradients"][0]["gradient"];
    assert_eq!(wild["gradient"]["stops"][0]["offset"], 0.0);
    assert!((wild["gradient"]["stops"][0]["midpoint"].as_f64().unwrap() - 0.87).abs() < 1e-6, "{wild}");
    assert_eq!(wild["gradient"]["stops"][0]["opacity"], 0.0);
    assert_eq!(wild["gradient"]["stops"][1]["offset"], 1.0);
    assert_eq!(got["groups"], json!([{"name": "G", "items": [{"kind": "gradient", "item": "Wild"}]}]));
    let undo = s.doc().unwrap().history.undo.len();
    for item in ["Broken", "Missing"] {
        let e = s.execute("library.use", &json!({"kind": "fillPattern", "item": item})).unwrap_err().to_string();
        assert!(e.contains("the library item is damaged"), "{item}: {e}");
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "nothing changed");
    run(&mut s, "library.use", json!({"kind": "fillGradient", "item": "Wild"}));
}

#[test]
fn text_styles_come_from_the_selected_type_and_apply_in_one_step() {
    let mut s = session();
    let lib = run(&mut s, "library.create", json!({}))["id"].as_str().unwrap().to_string();
    assert!(s.execute("library.add", &json!({"library": lib, "kind": "charStyle"})).is_err(), "no type selected");
    let t = run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Heading", "size": 31}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [t]}));
    let name = run(&mut s, "library.add", json!({"library": lib, "kind": "charStyle"}))["name"].as_str().unwrap().to_string();
    assert!(name.ends_with("31 pt"), "{name}");
    run(&mut s, "library.add", json!({"library": lib, "kind": "paraStyle"}));
    // Another document: the style is added and applied, and one undo takes both back.
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let u = run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Body", "size": 9}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [u]}));
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "library.use", json!({"library": lib, "kind": "charStyle", "item": name}));
    assert_eq!(r["style"], name);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let size = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(u.as_u64().unwrap())).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.runs[0].style.size,
        _ => 0.0,
    };
    assert_eq!(size(&s), 31.0);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(size(&s), 9.0);
    assert!(s.doc().unwrap().doc.char_styles.iter().all(|c| c.name != name), "the style went with the undo");
    // A style of that name with other attributes in the document: the library's comes in numbered.
    run(&mut s, "charStyle.new", json!({"name": name, "attrs": {"size": 5}}));
    let r = run(&mut s, "library.use", json!({"library": lib, "kind": "charStyle", "item": name}));
    assert_eq!(r["style"], format!("{name} 2"));
}

/// A library with the character and paragraph styles of 31 pt type, an undo group opened in the
/// first document with a rectangle drawn in it, and a second document, active, with type
/// selected → (session, library, character style, paragraph style).
fn group_open_in_another_document() -> (Session, String, String, String) {
    let mut s = session();
    let lib = run(&mut s, "library.create", json!({}))["id"].as_str().unwrap().to_string();
    let t = run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Heading", "size": 31}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [t]}));
    let char_style = run(&mut s, "library.add", json!({"library": lib, "kind": "charStyle"}))["name"].as_str().unwrap().to_string();
    let para_style = run(&mut s, "library.add", json!({"library": lib, "kind": "paraStyle"}))["name"].as_str().unwrap().to_string();
    // The second document exists before the group begins, so no group opens in it.
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    run(&mut s, "document.activate", json!({"index": 0}));
    run(&mut s, "edit.undoGroup.begin", json!({}));
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}));
    run(&mut s, "document.activate", json!({"index": 1}));
    let u = run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Body", "size": 9}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [u]}));
    (s, lib, char_style, para_style)
}

/// A library style used in one document does not close an undo group that is open in another
/// document, so that document's next edit still joins the group's undo step.
#[test]
fn a_library_style_leaves_another_documents_undo_group_open() {
    let (mut s, lib, name, _) = group_open_in_another_document();
    run(&mut s, "library.use", json!({"library": lib, "kind": "charStyle", "item": name}));
    assert!(s.documents()[0].undo_group.is_some(), "the first document's undo group stays open");
    s.set_active(0);
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "shape.rectangle", json!({"x": 30, "y": 0, "width": 20, "height": 20}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "the edit joins the group's step");
}

/// A library style that fails to apply takes back only the edits it made in its own document: an
/// undo group open in another document stays open, and the edits made in it stay.
#[test]
fn a_library_style_that_fails_takes_back_only_its_own_edits() {
    let (mut s, lib, _, name) = group_open_in_another_document();
    run(&mut s, "select.none", json!({}));
    assert!(s.execute("library.use", &json!({"library": lib, "kind": "paraStyle", "item": name})).is_err(), "no type selected");
    assert!(s.doc().unwrap().doc.para_styles.iter().all(|p| p.name != name), "the style it added is taken back");
    let first = &s.documents()[0];
    assert_eq!(first.doc.layers[0].children().map_or(0, |c| c.len()), 2, "the type and the rectangle drawn in the group stay");
    assert!(first.undo_group.is_some(), "the first document's undo group stays open");
}

/// A library style that fails to apply inside an undo group open in its own document takes back
/// the style it added, and the group stays open with the edits made in it before.
#[test]
fn a_library_style_that_fails_in_an_open_group_takes_back_its_own_edits() {
    let (mut s, lib, _, name) = group_open_in_another_document();
    s.set_active(0);
    run(&mut s, "select.none", json!({}));
    let undo = s.doc().unwrap().history.undo.len();
    assert!(s.execute("library.use", &json!({"library": lib, "kind": "paraStyle", "item": name})).is_err(), "no type selected");
    let d = s.doc().unwrap();
    assert!(d.doc.para_styles.iter().all(|p| p.name != name), "the style it added is taken back");
    assert_eq!(d.doc.layers[0].children().map_or(0, |c| c.len()), 2, "the type and the rectangle drawn in the group stay");
    assert_eq!(d.history.undo.len(), undo, "still the group's one step");
    assert!(d.undo_group.is_some(), "the group stays open");
}

/// A text style that a library file names with spaces around it comes into the document with the
/// name trimmed, as New Character Style and New Paragraph Style trim a name, and applies.
#[test]
fn a_library_style_named_with_spaces_comes_in_trimmed() {
    let mut s = session();
    let style = |attrs: Value| json!([{"name": " Body ", "attrs": attrs}]);
    let data = json!({"name": "Spaced", "charStyles": style(json!({"size": 31.0})), "paraStyles": style(json!({}))});
    let lib = run(&mut s, "library.import", json!({"data": data.to_string()}))["id"].as_str().unwrap().to_string();
    let t = run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Body", "size": 9}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [t]}));
    for kind in ["charStyle", "paraStyle"] {
        assert_eq!(run(&mut s, "library.use", json!({"library": lib, "kind": kind, "item": " Body "}))["style"], "Body", "{kind}");
    }
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.char_styles[0].name.as_str(), d.para_styles[0].name.as_str()), ("Body", "Body"));
    let vectorcraft_doc::NodeKind::Text(text) = &d.node(NodeId(t.as_u64().unwrap())).unwrap().kind else { panic!("not text") };
    assert_eq!((text.runs[0].style.size, text.runs[0].style.style_name.as_deref()), (31.0, Some("Body")));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn libraries_are_kept_in_files_across_sessions() {
    let dir = std::env::temp_dir().join(format!("vc-libraries-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let folder = dir.to_string_lossy().to_string();
    let mut s = session();
    s.libraries.set_dir(Some(folder.clone()));
    let lib = run(&mut s, "library.create", json!({"name": "Kept"}))["id"].as_str().unwrap().to_string();
    let id = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [id]}));
    run(&mut s, "library.add", json!({"library": lib, "kind": "graphic", "name": "Square"}));
    run(&mut s, "library.add", json!({"library": lib, "kind": "fillColor"}));
    let mut t = session();
    t.libraries.set_dir(Some(folder.clone()));
    let got = run(&mut t, "library.get", json!({"library": "Kept"}));
    assert_eq!(got["graphics"][0]["name"], "Square");
    assert_eq!(got["colors"].as_array().unwrap().len(), 1);
    run(&mut t, "library.delete", json!({"library": "Kept"}));
    let mut u = session();
    u.libraries.set_dir(Some(folder));
    assert_eq!(run(&mut u, "library.list", json!({}))["libraries"], json!([]), "deleting removes the file");
    // A damaged file is skipped.
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Bad.vclibrary"), b"{not json").unwrap();
    let mut v = session();
    v.libraries.set_dir(Some(dir.to_string_lossy().to_string()));
    assert_eq!(run(&mut v, "library.list", json!({}))["libraries"], json!([]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A library "Brand" holding a graphic, a colour and a character style, each added from the
/// selection → (session, the graphic's id, the colour's name, the style's name).
fn brand() -> (Session, String, String, String) {
    let mut s = session();
    run(&mut s, "library.create", json!({"name": "Brand"}));
    let r = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"color": "#336699", "ids": [r]}));
    run(&mut s, "select.set", json!({"ids": [r]}));
    let g = run(&mut s, "library.add", json!({"kind": "graphic", "name": "Logo"}))["id"].as_str().unwrap().to_string();
    let c = run(&mut s, "library.add", json!({"kind": "fillColor"}))["name"].as_str().unwrap().to_string();
    let t = run(&mut s, "text.create", json!({"x": 10, "y": 80, "text": "Type", "size": 18}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [t]}));
    let st = run(&mut s, "library.add", json!({"kind": "charStyle"}))["name"].as_str().unwrap().to_string();
    (s, g, c, st)
}

/// The current library's groups as `library.get` reports them: (name, [(kind, item)]).
fn groups(s: &mut Session) -> Vec<(String, Vec<(String, String)>)> {
    let got = run(s, "library.get", json!({}));
    got["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            let items = g["items"].as_array().unwrap().iter().map(|r| (r["kind"].as_str().unwrap().into(), r["item"].as_str().unwrap().into()));
            (g["name"].as_str().unwrap().to_string(), items.collect())
        })
        .collect()
}

fn refs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter().map(|(k, i)| (k.to_string(), i.to_string())).collect()
}

/// Graphics added one after another get ids of their own (the third used to repeat the second's).
#[test]
fn every_graphic_gets_an_id_of_its_own() {
    let mut s = session();
    run(&mut s, "library.create", json!({}));
    let mut ids = vec![];
    for x in [0, 30, 60, 90] {
        let r = run(&mut s, "shape.rectangle", json!({"x": x, "y": 0, "width": 10 + x, "height": 10}))["id"].clone();
        ids.push(run(&mut s, "library.add", json!({"kind": "graphic", "ids": [r]}))["id"].as_str().unwrap().to_string());
    }
    assert_eq!(ids, ["graphic", "graphic-2", "graphic-3", "graphic-4"]);
}

/// Items go into user-named groups and back out (#926); deleting a group keeps its items.
#[test]
fn items_are_grouped_moved_and_ungrouped() {
    let (mut s, g, c, st) = brand();
    assert!(groups(&mut s).is_empty(), "no groups yet");
    // A group made with items; a graphic is named by its name or its id.
    let r = run(&mut s, "library.createGroup", json!({"name": "Logos", "items": [{"kind": "graphic", "item": "Logo"}]}));
    assert_eq!(r["group"], "Logos");
    assert_eq!(groups(&mut s), [("Logos".into(), refs(&[("graphic", &g)]))]);
    // A name taken (ignoring case) gets a number.
    assert_eq!(run(&mut s, "library.createGroup", json!({"name": "logos"}))["group"], "logos 2");
    run(&mut s, "library.renameGroup", json!({"group": "logos 2", "name": "Palette"}));
    assert!(s.execute("library.renameGroup", &json!({"group": "Palette", "name": "LOGOS"})).is_err(), "another group's name");
    assert!(s.execute("library.renameGroup", &json!({"group": "Nope", "name": "X"})).is_err());
    // Moving an item takes it out of the group it was in; a colour is named as fill or stroke.
    run(&mut s, "library.moveItem", json!({"kind": "strokeColor", "item": c, "group": "palette"}));
    run(&mut s, "library.moveItem", json!({"kind": "charStyle", "item": st, "group": "Palette"}));
    let moved = run(&mut s, "library.moveItem", json!({"kind": "graphic", "item": "Logo", "group": "Palette"}));
    assert_eq!((moved["item"].as_str(), moved["group"].as_str()), (Some(g.as_str()), Some("Palette")));
    assert_eq!(groups(&mut s), [("Logos".into(), vec![]), ("Palette".into(), refs(&[("color", &c), ("charStyle", &st), ("graphic", &g)]))]);
    // Without a group, out of its group.
    assert!(run(&mut s, "library.moveItem", json!({"kind": "fillColor", "item": c}))["group"].is_null());
    assert!(s.execute("library.moveItem", &json!({"kind": "fillColor", "item": c, "group": "Nope"})).is_err());
    assert!(s.execute("library.moveItem", &json!({"kind": "fillColor", "item": "#nope", "group": "Palette"})).is_err());
    assert!(s.execute("library.createGroup", &json!({"items": [{"kind": "graphic", "item": "nope"}]})).is_err(), "no such item");
    assert_eq!(groups(&mut s).len(), 2, "a failed command makes no group");
    // A deleted item leaves its group.
    run(&mut s, "library.removeItem", json!({"kind": "charStyle", "item": st}));
    assert_eq!(groups(&mut s)[1].1, refs(&[("graphic", &g)]));
    // A deleted group's items stay, ungrouped.
    assert_eq!(run(&mut s, "library.deleteGroup", json!({"group": "Palette"}))["ungrouped"], 1);
    assert_eq!(run(&mut s, "library.get", json!({}))["graphics"].as_array().unwrap().len(), 1);
    assert_eq!(groups(&mut s), [("Logos".into(), vec![])]);
    assert_eq!(run(&mut s, "library.list", json!({}))["libraries"][0]["groups"], 1);
}

/// The names of the current library's `list` ("colors", "graphics"…), in their order.
fn order(s: &mut Session, list: &str, key: &str) -> Vec<String> {
    let got = run(s, "library.get", json!({}));
    got[list].as_array().unwrap().iter().map(|v| v[key].as_str().unwrap().to_string()).collect()
}

/// Dragging in the Libraries panel: an item moves just before or after another of its kind, into
/// that one's group or out of every group, and groups move to another place in the list.
#[test]
fn items_and_groups_are_reordered() {
    let mut s = session();
    run(&mut s, "library.create", json!({"name": "Brand"}));
    for (name, color) in [("Red", "#ff0000"), ("Green", "#00ff00"), ("Blue", "#0000ff"), ("Gray", "#808080")] {
        run(&mut s, "library.add", json!({"kind": "fillColor", "color": color, "name": name}));
    }
    // Before an item, and after one; on itself it stays.
    run(&mut s, "library.moveItem", json!({"kind": "fillColor", "item": "Blue", "before": "Red"}));
    assert_eq!(order(&mut s, "colors", "name"), ["Blue", "Red", "Green", "Gray"]);
    run(&mut s, "library.moveItem", json!({"kind": "fillColor", "item": "Blue", "after": "Gray"}));
    assert_eq!(order(&mut s, "colors", "name"), ["Red", "Green", "Gray", "Blue"]);
    run(&mut s, "library.moveItem", json!({"kind": "fillColor", "item": "Red", "before": "Red"}));
    assert_eq!(order(&mut s, "colors", "name"), ["Red", "Green", "Gray", "Blue"]);
    // Into a group next to an item there: the group lists it next to that one too.
    run(
        &mut s,
        "library.createGroup",
        json!({"name": "Cool", "items": [{"kind": "fillColor", "item": "Green"}, {"kind": "fillColor", "item": "Blue"}]}),
    );
    let r = run(&mut s, "library.moveItem", json!({"kind": "fillColor", "item": "Gray", "group": "Cool", "before": "Blue"}));
    assert_eq!(r["group"], "Cool");
    assert_eq!(order(&mut s, "colors", "name"), ["Red", "Green", "Gray", "Blue"]);
    assert_eq!(groups(&mut s), [("Cool".into(), refs(&[("color", "Green"), ("color", "Gray"), ("color", "Blue")]))]);
    // Next to an ungrouped item, without a group: out of its group.
    run(&mut s, "library.moveItem", json!({"kind": "fillColor", "item": "Blue", "before": "Red"}));
    assert_eq!(order(&mut s, "colors", "name"), ["Blue", "Red", "Green", "Gray"]);
    assert_eq!(groups(&mut s), [("Cool".into(), refs(&[("color", "Green"), ("color", "Gray")]))]);
    // Wrong asks change nothing.
    assert!(s.execute("library.moveItem", &json!({"kind": "fillColor", "item": "Red", "before": "Nope"})).is_err());
    assert!(s.execute("library.moveItem", &json!({"kind": "fillColor", "item": "Red", "before": "Blue", "after": "Gray"})).is_err());
    assert!(s.execute("library.moveItem", &json!({"kind": "charStyle", "item": "Red", "before": "Blue"})).is_err(), "not a style");
    assert_eq!(order(&mut s, "colors", "name"), ["Blue", "Red", "Green", "Gray"]);
    // Groups: to an index, past the end to the last place.
    run(&mut s, "library.createGroup", json!({"name": "Warm"}));
    run(&mut s, "library.createGroup", json!({"name": "Neutral"}));
    let names = |s: &mut Session| groups(s).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
    assert_eq!(run(&mut s, "library.moveGroup", json!({"group": "neutral", "to": 0}))["to"], 0);
    assert_eq!(names(&mut s), ["Neutral", "Cool", "Warm"]);
    assert_eq!(run(&mut s, "library.moveGroup", json!({"group": "Neutral", "to": 99}))["to"], 2);
    assert_eq!(names(&mut s), ["Cool", "Warm", "Neutral"]);
    assert!(s.execute("library.moveGroup", &json!({"group": "Nope", "to": 0})).is_err());
    assert!(s.execute("library.moveGroup", &json!({"group": "Cool", "to": -1})).is_err());
    assert!(s.execute("library.moveGroup", &json!({"group": "Cool"})).is_err());
    assert_eq!(groups(&mut s)[0].1, refs(&[("color", "Green"), ("color", "Gray")]), "a group keeps its items when it moves");
}

/// A library goes out to a file with its art and groups and comes back as a new library, numbered
/// when its name is taken; the graphic it brings places in a document (#926).
#[test]
fn a_library_is_exported_and_imported() {
    let (mut s, g, c, _) = brand();
    run(&mut s, "library.createGroup", json!({"name": "Logos", "items": [{"kind": "graphic", "item": g}, {"kind": "fillColor", "item": c}]}));
    let out = run(&mut s, "library.export", json!({}));
    assert_eq!(out["name"], "Brand");
    let data = out["data"].as_str().unwrap().to_string();
    let r = run(&mut s, "library.import", json!({"data": data}));
    let counts = ["graphics", "colors", "charStyles", "paraStyles", "groups"].map(|k| r[k].as_u64().unwrap());
    assert_eq!((r["name"].as_str(), counts), (Some("Brand (2)"), [1, 1, 1, 0, 1]));
    let id = r["id"].as_str().unwrap().to_string();
    assert_eq!(run(&mut s, "library.list", json!({}))["current"], id, "the imported library is the current one");
    let a = run(&mut s, "library.get", json!({"library": "Brand"}));
    let b = run(&mut s, "library.get", json!({"library": id}));
    for k in ["graphics", "colors", "charStyles", "paraStyles", "groups"] {
        assert_eq!(a[k], b[k], "{k}");
    }
    // Again: the next number; given a name, that name.
    let b64 = vectorcraft_format::base64_encode(data.as_bytes());
    assert_eq!(run(&mut s, "library.import", json!({"dataBase64": b64}))["name"], "Brand (3)");
    assert_eq!(run(&mut s, "library.import", json!({"data": data, "name": "Client"}))["name"], "Client");
    // Its graphic places in another document.
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let placed = run(&mut s, "library.use", json!({"library": id, "kind": "graphic", "item": "Logo", "center": [50, 50]}));
    assert_eq!(placed["ids"].as_array().unwrap().len(), 1);
    // Not a library file.
    let swatches = json!({"format": "vcswatches", "name": "S"}).to_string();
    for bad in [json!({"data": "{not json"}), json!({"data": "[1, 2]"}), json!({"data": swatches}), json!({"dataBase64": "%%"}), json!({})] {
        assert!(s.execute("library.import", &bad).is_err(), "{bad}");
    }
}

/// A file written before groups loads; one with repeated names, groups naming items that aren't
/// there or kinds this version doesn't know, and a thumbnail claiming a huge image is made safe.
#[test]
fn imported_libraries_are_made_consistent() {
    let mut s = session();
    let old = json!({"name": "Old", "colors": [{"name": "Red", "color": {"model": "rgb", "r": 1, "g": 0, "b": 0}}]});
    let r = run(&mut s, "library.import", json!({"data": old.to_string()}));
    assert_eq!((r["colors"].as_u64(), r["groups"].as_u64()), (Some(1), Some(0)));
    // A PNG header claiming 100000 × 100000 pixels.
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend(100_000u32.to_be_bytes());
    png.extend(100_000u32.to_be_bytes());
    png.extend([8, 6, 0, 0, 0]);
    let messy = json!({
        "name": "  ",
        "colors": [{"name": "Red", "color": {"model": "rgb", "r": 1, "g": 0, "b": 0}}, {"name": "Red", "color": {"model": "rgb", "r": 0, "g": 1, "b": 0}}, {"name": "", "color": {"model": "rgb", "r": 0, "g": 0, "b": 1}}],
        "graphics": [
            {"id": "g", "name": "A", "width": 1, "height": 1, "data": "", "thumbnail": vectorcraft_format::base64_encode(&png)},
            {"id": "g", "name": "B", "width": -5, "height": 1, "data": ""}
        ],
        "groups": [
            {"name": "One", "items": [{"kind": "color", "item": "Red 2"}, {"kind": "graphic", "item": "nope"}, {"kind": "brush", "item": "Red"}]},
            {"name": "one", "items": [{"kind": "color", "item": "Red 2"}, {"kind": "graphic", "item": "g-2"}]}
        ]
    });
    let r = run(&mut s, "library.import", json!({"data": messy.to_string()}));
    assert_eq!(r["name"], "Library", "a blank name");
    let got = run(&mut s, "library.get", json!({}));
    let names: Vec<&str> = got["colors"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Red", "Red 2", "Color"]);
    let graphics = got["graphics"].as_array().unwrap();
    assert_eq!((graphics[0]["id"].as_str(), graphics[1]["id"].as_str()), (Some("g"), Some("g-2")));
    assert_eq!((graphics[0]["thumbnail"].as_str(), graphics[1]["width"].as_f64()), (Some(""), Some(0.0)));
    assert_eq!(
        got["groups"],
        json!([{"name": "One", "items": [{"kind": "color", "item": "Red 2"}]}, {"name": "one 2", "items": [{"kind": "graphic", "item": "g-2"}]}])
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_library_is_exported_to_a_file_and_imported_from_it() {
    let dir = vectorcraft_testkit::temp_dir("library-export");
    let path = dir.join("Brand.vclibrary").to_string_lossy().to_string();
    let (mut s, g, ..) = brand();
    run(&mut s, "library.createGroup", json!({"name": "Logos", "items": [{"kind": "graphic", "item": g}]}));
    assert_eq!(run(&mut s, "library.export", json!({"library": "Brand", "path": path}))["path"], path.as_str());
    let mut t = session();
    let r = run(&mut t, "library.import", json!({"path": path}));
    assert_eq!((r["name"].as_str(), r["graphics"].as_u64(), r["groups"].as_u64()), (Some("Brand"), Some(1), Some(1)));
    assert!(t.execute("library.import", &json!({"path": dir.join("missing.vclibrary").to_string_lossy()})).is_err());
    // Groups are kept in the library folder's files too.
    let folder = dir.join("folder").to_string_lossy().to_string();
    let mut u = session();
    u.libraries.set_dir(Some(folder.clone()));
    run(&mut u, "library.import", json!({"path": path}));
    let mut v = session();
    v.libraries.set_dir(Some(folder));
    assert_eq!(groups(&mut v), [("Logos".into(), refs(&[("graphic", &g)]))]);
}
