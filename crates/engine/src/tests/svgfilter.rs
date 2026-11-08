//! Effect › SVG Filters (`svgFilter.*`): the filters a document holds, applying one as a live
//! `svg.filter` effect, and what SVG export and the native format keep of them.

use serde_json::{Value, json};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn err(s: &mut Session, id: &str, p: Value) -> String {
    s.execute(id, &p).unwrap_err().to_string()
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 30}))["id"].as_u64().unwrap())
}

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn names(s: &mut Session) -> Vec<String> {
    run(s, "svgFilter.list", json!({}))["filters"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect()
}

const BLUR: &str = r#"<filter id="Soft"><feGaussianBlur stdDeviation="2"/></filter>"#;
const SHIFT: &str = r#"<filter id="Shift"><feOffset dx="3" dy="3"/></filter>"#;

#[test]
fn filters_are_defined_edited_and_deleted_in_one_undo_step_each() {
    let mut s = session();
    assert_eq!(run(&mut s, "svgFilter.list", json!({}))["filters"], json!([]));
    assert_eq!(run(&mut s, "svgFilter.define", json!({"xml": format!(" {BLUR} ")}))["name"], "Soft");
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT}));
    assert_eq!(names(&mut s), ["Soft", "Shift"], "in the order they were added");
    assert_eq!(run(&mut s, "svgFilter.list", json!({}))["filters"][0]["xml"], BLUR, "the XML trimmed");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&mut s), ["Soft"], "one step");
    run(&mut s, "edit.redo", json!({}));
    // A name another filter has is refused; replace edits one in place, and it keeps its name.
    assert!(err(&mut s, "svgFilter.define", json!({"xml": BLUR})).contains("already has an SVG filter named \"Soft\""));
    let renamed = r#"<filter id="Softer"><feGaussianBlur stdDeviation="4"/></filter>"#;
    run(&mut s, "svgFilter.define", json!({"xml": renamed, "replace": "Soft"}));
    assert_eq!(names(&mut s), ["Soft", "Shift"]);
    assert!(err(&mut s, "svgFilter.define", json!({"xml": SHIFT, "replace": "Nope"})).contains("no SVG filter named \"Nope\""));
    // The same filter again changes nothing and makes no undo step.
    let before = steps(&s);
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT, "replace": "Shift"}));
    assert_eq!(steps(&s), before);
    assert_eq!(run(&mut s, "svgFilter.delete", json!({"name": "Soft"}))["filters"].as_array().unwrap().len(), 1);
    assert!(err(&mut s, "svgFilter.delete", json!({"name": "Soft"})).contains("no SVG filter named"));
    assert!(err(&mut s, "svgFilter.delete", json!({"name": " "})).contains("give `name`"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&mut s), ["Soft", "Shift"]);
}

#[test]
fn define_refuses_what_is_not_one_filter() {
    let mut s = session();
    for (p, why) in [
        (json!({}), "give `xml`"),
        (json!({"xml": 3}), "`xml` must be a string"),
        (json!({"xml": "<filter><feOffset/></filter>"}), "needs an id"),
        (json!({"xml": "<filter id=\"a\"/>"}), "no primitives"),
        (json!({"xml": "<g/>"}), "give one <filter> element"),
        (json!({"xml": SHIFT, "replace": 1}), "`replace` must be a string"),
    ] {
        let e = err(&mut s, "svgFilter.define", p.clone());
        assert!(e.contains(why), "{p}: {e}");
    }
    assert_eq!(steps(&s), 0, "nothing recorded");
}

#[test]
fn a_document_holds_at_most_64_filters() {
    let mut s = session();
    for i in 0..64 {
        run(&mut s, "svgFilter.define", json!({"xml": format!("<filter id=\"f{i}\"><feOffset/></filter>")}));
    }
    assert!(err(&mut s, "svgFilter.define", json!({"xml": SHIFT})).contains("at most 64 SVG filters"));
    run(&mut s, "svgFilter.define", json!({"xml": "<filter id=\"g0\"><feOffset dx=\"1\"/></filter>", "replace": "f0"}));
    assert_eq!(names(&mut s).len(), 64, "a replacement still fits");
}

#[test]
fn import_adds_the_files_filters_it_does_not_have() {
    let mut s = session();
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT}));
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><defs>{BLUR}<filter id=\"Shift\"><feOffset dx=\"9\"/></filter><filter id=\"Bad\"/></defs><rect width=\"1\" height=\"1\"/></svg>"
    );
    let path = std::env::temp_dir().join(format!("vc-svgfilters-{}.svg", std::process::id()));
    std::fs::write(&path, &svg).unwrap();
    let r = run(&mut s, "svgFilter.import", json!({"path": path.to_string_lossy()}));
    std::fs::remove_file(&path).ok();
    assert_eq!(r["added"], json!(["Soft"]));
    assert_eq!(r["skipped"], 2, "a name the document has, and a filter without primitives");
    assert_eq!(names(&mut s), ["Shift", "Soft"]);
    assert_eq!(run(&mut s, "svgFilter.list", json!({}))["filters"][0]["xml"], SHIFT, "the document's own filter stays");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&mut s), ["Shift"], "one step");
    // Nothing new: no undo step.
    let before = steps(&s);
    assert_eq!(run(&mut s, "svgFilter.import", json!({"svg": "<svg xmlns=\"http://www.w3.org/2000/svg\"/>"}))["added"], json!([]));
    assert_eq!(steps(&s), before);
    assert!(err(&mut s, "svgFilter.import", json!({"svg": "<svg"})).contains("isn't valid SVG"));
    // Read with SVG import's limits: elements nested past them are refused before parsing.
    let deep = format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{}{BLUR}{}</svg>", "<g>".repeat(5000), "</g>".repeat(5000));
    assert!(err(&mut s, "svgFilter.import", json!({"svg": deep})).contains("isn't valid SVG"));
    assert!(err(&mut s, "svgFilter.import", json!({})).contains("give `path`"));
    let svg_text = format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{BLUR}</svg>");
    assert!(err(&mut s, "svgFilter.import", json!({"path": 3, "svg": svg_text})).contains("`path` must be a string"));
    assert!(err(&mut s, "svgFilter.import", json!({"svg": ["x"]})).contains("`svg` must be a string"));
    assert!(err(&mut s, "svgFilter.delete", json!({"name": 3})).contains("`name` must be a string"));
    let svgz = std::env::temp_dir().join(format!("vc-svgfilters-{}.svgz", std::process::id()));
    std::fs::write(&svgz, vectorcraft_svg::compress(&format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{BLUR}</svg>"))).unwrap();
    let e = err(&mut s, "svgFilter.import", json!({"path": svgz.to_string_lossy()}));
    std::fs::remove_file(&svgz).ok();
    assert!(e.contains("not SVGZ"), "{e}");
    let missing = std::env::temp_dir().join("vc-no-such-svgfilter-file (1).svg");
    assert!(s.execute("svgFilter.import", &json!({"path": missing.to_string_lossy()})).is_err());
}

#[test]
fn apply_adds_the_filter_as_a_live_effect_in_one_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    let b = rect(&mut s, 50.0);
    run(&mut s, "svgFilter.define", json!({"xml": BLUR}));
    let before = steps(&s);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let r = run(&mut s, "svgFilter.apply", json!({"name": "Soft"}));
    assert_eq!(r["ids"].as_array().unwrap().len(), 2);
    assert_eq!(steps(&s), before + 1, "one step for both objects");
    for id in [a, b] {
        let fx = s.doc().unwrap().doc.node(id).unwrap().appearance.effects.clone();
        assert_eq!(fx.len(), 1);
        assert_eq!(fx[0].id, "svg.filter");
        assert_eq!((fx[0].params["name"].as_str(), fx[0].params["xml"].as_str()), (Some("Soft"), Some(BLUR)));
    }
    run(&mut s, "svgFilter.delete", json!({"name": "Soft"}));
    assert!(err(&mut s, "svgFilter.apply", json!({"name": "Soft"})).contains("no SVG filter named"));
    assert!(err(&mut s, "svgFilter.apply", json!({})).contains("give `name`"));
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT}));
    run(&mut s, "select.set", json!({"ids": []}));
    assert!(s.execute("svgFilter.apply", &json!({"name": "Shift"})).is_err(), "nothing selected");
    run(&mut s, "svgFilter.apply", json!({"name": "Shift", "ids": [b.0]}));
    assert_eq!(s.doc().unwrap().doc.node(b).unwrap().appearance.effects.len(), 2, "given ids, nothing selected");
}

/// The xml the first effect of `id` draws with.
fn drawn(s: &Session, id: NodeId) -> Value {
    s.doc().unwrap().doc.node(id).unwrap().appearance.effects[0].params["xml"].clone()
}

const SHIFT_MORE: &str = r#"<filter id="Shift"><feOffset dx="9" dy="9"/></filter>"#;

/// Objects name the document's filter: an edit changes what every one of them draws, in one undo
/// step, and a later application takes the edited filter.
#[test]
fn editing_a_filter_changes_every_object_that_uses_it() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    let b = rect(&mut s, 50.0);
    let c = rect(&mut s, 90.0);
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT}));
    run(&mut s, "svgFilter.apply", json!({"name": "Shift", "ids": [a.0, b.0]}));
    let before = steps(&s);
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT_MORE, "replace": "Shift"}));
    assert_eq!(steps(&s), before + 1);
    assert_eq!((drawn(&s, a), drawn(&s, b)), (json!(SHIFT_MORE), json!(SHIFT_MORE)));
    // One undo puts both back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((drawn(&s, a), drawn(&s, b)), (json!(SHIFT), json!(SHIFT)));
    run(&mut s, "edit.redo", json!({}));
    // Applied after the edit: the edited filter.
    run(&mut s, "svgFilter.apply", json!({"name": "Shift", "ids": [c.0]}));
    assert_eq!(drawn(&s, c), json!(SHIFT_MORE));
    // An edit whose code names another id keeps the filter's name.
    run(&mut s, "svgFilter.define", json!({"xml": r#"<filter id="Renamed"><feOffset dx="1"/></filter>"#, "replace": "Shift"}));
    assert_eq!(names(&mut s), ["Shift"]);
    assert_eq!(drawn(&s, a), json!(r#"<filter id="Shift"><feOffset dx="1"/></filter>"#));
}

/// Deleting a filter that objects use leaves their effects, which draw nothing until a filter of
/// that name exists again.
#[test]
fn deleting_a_filter_in_use_leaves_the_effect_drawing_nothing() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT}));
    run(&mut s, "svgFilter.apply", json!({"name": "Shift", "ids": [a.0]}));
    run(&mut s, "svgFilter.delete", json!({"name": "Shift"}));
    let fx = s.doc().unwrap().doc.node(a).unwrap().appearance.effects.clone();
    assert_eq!((fx.len(), fx[0].params["name"].as_str()), (1, Some("Shift")), "the effect stays, named");
    assert!(vectorcraft_render::effects::raster_effects(&fx).is_empty(), "drawn without the filter");
    // Imported again: the effect draws with it.
    run(&mut s, "svgFilter.import", json!({"svg": format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{SHIFT_MORE}</svg>")}));
    assert_eq!(drawn(&s, a), json!(SHIFT_MORE));
}

/// Art in a symbol's definition that uses a filter follows its edits too.
#[test]
fn a_symbols_art_follows_a_filter_edit() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT}));
    run(&mut s, "svgFilter.apply", json!({"name": "Shift", "ids": [a.0]}));
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "symbol.new", json!({"name": "Mark"}));
    let in_symbol = |s: &Session| {
        let mut xml = vec![];
        s.doc().unwrap().doc.symbols[0].art.walk(&mut |n| xml.extend(n.appearance.effects.iter().map(|e| e.params["xml"].clone())));
        xml
    };
    assert_eq!(in_symbol(&s), [json!(SHIFT)]);
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT_MORE, "replace": "Shift"}));
    assert_eq!(in_symbol(&s), [json!(SHIFT_MORE)]);
}

/// A graphic style that holds a filter follows its edits, so applying it later gives the edited
/// filter.
#[test]
fn a_graphic_style_follows_a_filter_edit() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    let b = rect(&mut s, 50.0);
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT}));
    run(&mut s, "svgFilter.apply", json!({"name": "Shift", "ids": [a.0]}));
    run(&mut s, "graphicStyle.new", json!({"name": "Shifted", "id": a.0}));
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT_MORE, "replace": "Shift"}));
    run(&mut s, "graphicStyle.apply", json!({"name": "Shifted", "ids": [b.0]}));
    assert_eq!(drawn(&s, b), json!(SHIFT_MORE));
}

/// An edited filter keeps its name, characters XML escapes included.
#[test]
fn an_edit_keeps_a_name_with_escaped_characters() {
    let mut s = session();
    run(&mut s, "svgFilter.define", json!({"xml": r#"<filter id="a&amp;b"><feOffset/></filter>"#}));
    assert_eq!(names(&mut s), ["a&b"]);
    run(&mut s, "svgFilter.define", json!({"xml": r#"<filter id="x"><feOffset dx="2"/></filter>"#, "replace": "a&b"}));
    assert_eq!(names(&mut s), ["a&b"]);
}

#[test]
fn effect_apply_takes_a_filter_and_names_it_by_its_id() {
    let mut s = session();
    rect(&mut s, 10.0);
    let r = run(&mut s, "effect.apply", json!({"effect": "svg.filter", "params": {"xml": SHIFT}}));
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().appearance.effects[0].params["name"], "Shift");
    assert!(err(&mut s, "effect.apply", json!({"effect": "svg.filter"})).contains("give `xml`"));
    for scale in [json!(0), json!(1001), json!("2")] {
        let e = err(&mut s, "effect.apply", json!({"effect": "svg.filter", "params": {"xml": SHIFT, "scale": scale}}));
        assert!(e.contains("`scale` must be a number"), "{scale}: {e}");
    }
    assert!(err(&mut s, "effect.apply", json!({"effect": "svg.filter", "params": {"xml": "<g/>"}})).contains("give one <filter> element"));
    assert!(err(&mut s, "effect.setParams", json!({"index": 0, "params": {"xml": "<filter/>"}})).contains("needs an id"));
    assert!(err(&mut s, "effect.setParams", json!({"index": 0, "params": {"scale": 0}})).contains("`scale` must be a number from 0.001"));
    assert!(err(&mut s, "effect.setParams", json!({"index": 0, "params": {"name": "Other"}})).contains("give `xml` with `name`"));
    // Another filter renames the effect.
    run(&mut s, "effect.setParams", json!({"index": 0, "params": {"xml": BLUR}}));
    let p = &s.doc().unwrap().doc.node(id).unwrap().appearance.effects[0].params;
    assert_eq!((p["name"].as_str(), p["xml"].as_str()), (Some("Soft"), Some(BLUR)));
}

/// An effect names the document's filter, so effect.apply and effect.setParams refuse XML that
/// differs from the document's filter of the same id, and take XML that matches it.
#[test]
fn effect_commands_refuse_another_definition_of_a_document_filter() {
    let mut s = session();
    rect(&mut s, 10.0);
    run(&mut s, "svgFilter.define", json!({"xml": BLUR}));
    let other = r#"<filter id="Soft"><feGaussianBlur stdDeviation="9"/></filter>"#;
    let before = steps(&s);
    let e = err(&mut s, "effect.apply", json!({"effect": "svg.filter", "params": {"xml": other}}));
    assert!(e.contains("another SVG filter named \"Soft\""), "{e}");
    assert_eq!(steps(&s), before, "nothing recorded");
    run(&mut s, "effect.apply", json!({"effect": "svg.filter", "params": {"xml": format!(" {BLUR} ")}}));
    run(&mut s, "effect.apply", json!({"effect": "svg.filter", "params": {"xml": SHIFT}}));
    let e = err(&mut s, "effect.setParams", json!({"index": 1, "params": {"xml": other}}));
    assert!(e.contains("another SVG filter named \"Soft\""), "{e}");
    run(&mut s, "effect.setParams", json!({"index": 1, "params": {"xml": BLUR}}));
}

/// An effect whose filter `check` refuses (put there by an opened file or a script, not by a
/// command) is neither drawn nor written to SVG.
#[test]
fn svg_export_leaves_out_a_filter_that_fails_the_check() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    let bad = r#"<filter id="Bad"><feOffset/><script>alert(1)</script></filter>"#;
    assert!(err(&mut s, "effect.apply", json!({"effect": "svg.filter", "ids": [a.0], "params": {"xml": bad}})).contains("<script>"));
    let effect = vectorcraft_doc::Effect { id: "svg.filter".into(), params: json!({"name": "Bad", "xml": bad}), visible: true };
    s.edit("Plant", |d, _| {
        d.node_mut(a).unwrap().appearance.effects.push(effect);
        Ok(())
    })
    .unwrap();
    let out = svg(&mut s);
    assert!(!out.contains("script") && !out.contains("<filter"), "{out}");
}

/// A user-space region in percentages is measured against what the filter runs on: SVG export keeps
/// the filter only when it runs on the object alone, and otherwise writes the object as an image.
#[test]
fn svg_export_keeps_a_measured_region_only_on_the_object_alone() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    let measured = r#"<filter id="M" filterUnits="userSpaceOnUse" x="-50%" y="-50%" width="200%" height="200%"><feOffset dx="3"/></filter>"#;
    run(&mut s, "effect.apply", json!({"effect": "svg.filter", "ids": [a.0], "params": {"xml": measured}}));
    let alone = svg(&mut s);
    assert!(alone.contains("<feOffset dx=\"3\"/>") && !alone.contains("<image"), "{alone}");
    // On a second object, after a blur: an image.
    let mut t = session();
    let b = rect(&mut t, 10.0);
    run(&mut t, "effect.apply", json!({"effect": "blur.gaussian", "ids": [b.0], "params": {"radius": 1}}));
    run(&mut t, "effect.apply", json!({"effect": "svg.filter", "ids": [b.0], "params": {"xml": measured}}));
    let after = svg(&mut t);
    assert!(after.contains("<image") && !after.contains("<feOffset dx=\"3\""), "{after}");
}

/// SVG export writes a filter rebuilt from its filter elements and filter attributes, not the XML
/// the effect holds.
#[test]
fn svg_export_writes_the_filter_rebuilt() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    let noted = r#"<filter id="Noted" data-x="1"><!-- <script>x</script> --><desc>&lt;b&gt;note</desc><feOffset dx="3" dy="3" foo="bar"/></filter>"#;
    run(&mut s, "effect.apply", json!({"effect": "svg.filter", "ids": [a.0], "params": {"xml": noted}}));
    let out = svg(&mut s);
    assert!(out.contains("<feOffset dx=\"3\" dy=\"3\"/></filter>"), "{out}");
    assert!(!out.contains("script") && !out.contains("note") && !out.contains("data-x") && !out.contains("foo"), "{out}");
}

/// The filter's user space moves with the object: its `origin` follows a move.
#[test]
fn moving_the_object_moves_the_filters_origin() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    run(&mut s, "effect.apply", json!({"effect": "svg.filter", "ids": [a.0], "params": {"xml": SHIFT, "origin": [3, 4]}}));
    run(&mut s, "object.move", json!({"ids": [a.0], "dx": 5, "dy": -2}));
    let p = &s.doc().unwrap().doc.node(a).unwrap().appearance.effects[0].params;
    assert_eq!(p["origin"], json!([8.0, 2.0]));
}

#[test]
fn filters_survive_a_native_round_trip() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    run(&mut s, "svgFilter.define", json!({"xml": BLUR}));
    run(&mut s, "svgFilter.apply", json!({"name": "Soft", "ids": [a.0]}));
    let doc = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(back.svg_filters, doc.svg_filters);
    assert_eq!(back.node(a).unwrap().appearance.effects, doc.node(a).unwrap().appearance.effects);
}

const RED: &str = r##"<filter id="Red"><feFlood flood-color="#ff0000"/><feComposite in2="SourceGraphic" operator="in"/></filter>"##;

/// A blue square with no stroke, 100 pt at (50, 50) in a 200 pt document.
fn blue_square() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = rect_at(&mut s, 50.0, 100.0);
    run(&mut s, "paint.setFill", json!({"color": "#0000ff", "ids": [id.0]}));
    run(&mut s, "paint.setStroke", json!({"none": true, "ids": [id.0]}));
    (s, id)
}

fn rect_at(s: &mut Session, at: f64, size: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": at, "y": at, "width": size, "height": size}))["id"].as_u64().unwrap())
}

fn render(s: &Session) -> vectorcraft_render::Rendered {
    vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, vectorcraft_geom::Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true)
}

#[test]
fn the_canvas_and_pdf_draw_the_filter() {
    let (mut s, id) = blue_square();
    assert_eq!(&render(&s).pixel(100, 100)[..3], &[0, 0, 255]);
    run(&mut s, "svgFilter.define", json!({"xml": RED}));
    run(&mut s, "svgFilter.apply", json!({"name": "Red", "ids": [id.0]}));
    let r = render(&s);
    assert_eq!(&r.pixel(100, 100)[..3], &[255, 0, 0], "the flood, cut to the square");
    assert_eq!(r.pixel(20, 20), render(&session_without_art()).pixel(20, 20), "outside the square: nothing");
    // PDF writes the effected square as an image that reads back as the canvas shows it.
    let doc = s.doc().unwrap().doc.clone();
    let out = crate::cmd::rasterfx::export_pdf_with_report(&doc, &Default::default()).unwrap();
    let back = vectorcraft_pdf::import(&out.bytes).unwrap();
    let again = vectorcraft_render::Renderer::new().render_region(&back, vectorcraft_geom::Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true);
    let px = again.pixel(100, 100);
    assert!(px[0] > 240 && px[1] < 15 && px[2] < 15, "{px:?}");
}

fn session_without_art() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

/// The SVG `document.serialize` writes.
fn svg(s: &mut Session) -> String {
    run(s, "document.serialize", json!({"format": "svg"}))["text"].as_str().unwrap().to_string()
}

#[test]
fn svg_export_writes_the_filter_when_it_is_the_last_effect() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    run(&mut s, "svgFilter.define", json!({"xml": SHIFT}));
    run(&mut s, "svgFilter.apply", json!({"name": "Shift", "ids": [a.0]}));
    let out = svg(&mut s);
    assert!(out.contains("<feOffset dx=\"3\" dy=\"3\"/></filter>"), "{out}");
    assert!(out.contains("<g filter=\"url(#"), "{out}");
    assert!(!out.contains("<image"), "the object stays vector art: {out}");
    // SVG import reads it back as the same filter.
    let (back, warnings) = vectorcraft_svg::import_with_report(&out).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut kept = vec![];
    back.walk(|n| kept.extend(n.appearance.effects.iter().filter(|e| e.id == "svg.filter").cloned()));
    assert_eq!(kept.len(), 1);
    assert!(kept[0].params["xml"].as_str().unwrap().ends_with("<feOffset dx=\"3\" dy=\"3\"/></filter>"), "{kept:?}");
    // SVG Tiny 1.2 has no filters: the object is written as an image.
    let tiny = run(&mut s, "document.export", json!({"format": "svg", "profile": "tiny12"}));
    let tiny = String::from_utf8(vectorcraft_format::base64_decode(tiny["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(tiny.contains("<image") && !tiny.contains("<filter"), "{tiny}");
    // An artboard away from the origin: the object keeps the document's coordinates, which the
    // filter's user space measures, in a group that moves it.
    let user = r#"<filter id="Here" filterUnits="userSpaceOnUse" x="0" y="0" width="60" height="60"><feOffset dx="1"/></filter>"#;
    let mut t = session();
    run(&mut t, "artboard.setProps", json!({"index": 0, "x": 5, "y": 5}));
    let b = rect(&mut t, 10.0);
    run(&mut t, "effect.apply", json!({"effect": "svg.filter", "ids": [b.0], "params": {"xml": user}}));
    let moved = svg(&mut t);
    let at = moved.find("<g transform=\"matrix(1 0 0 1 -5 -5)\">").unwrap_or_else(|| panic!("{moved}"));
    assert!(moved[at..].contains("10"), "the object in document coordinates: {moved}");
    assert!(!moved[at..].contains(" 5 5 ") && moved.contains("x=\"0\" y=\"0\" width=\"60\""), "the filter as written: {moved}");
    // Opened again, the filter's user space starts where the group places it.
    let (back, warnings) = vectorcraft_svg::import_with_report(&moved).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut kept = vec![];
    back.walk(|n| kept.extend(n.appearance.effects.iter().filter(|e| e.id == "svg.filter").cloned()));
    assert_eq!(kept.len(), 1, "{moved}");
    assert_eq!(kept[0].params["origin"], json!([-5.0, -5.0]));
    assert!(err(&mut t, "effect.setParams", json!({"ids": [b.0], "index": 0, "params": {"origin": [1]}})).contains("`origin` must be [x, y]"));
    // An effect after it: the object is drawn as an image.
    run(&mut s, "effect.apply", json!({"effect": "blur.gaussian", "ids": [a.0], "params": {"radius": 1}}));
    let out = svg(&mut s);
    assert!(out.contains("<image"), "{out}");
    assert!(!out.contains("<feOffset dx=\"3\""), "{out}");
}

/// A filter SVG 1.1 can't hold (a blend mode SVG 1.1 lacks) is exported as an image, as the canvas
/// draws it; an SVG 1.1 one stays a filter.
#[test]
fn svg_export_draws_a_filter_svg_1_1_lacks_as_an_image() {
    for (mode, image) in [("overlay", true), ("multiply", false)] {
        let mut s = session();
        let a = rect(&mut s, 10.0);
        let xml = format!(r#"<filter id="B"><feFlood flood-color="red"/><feBlend in2="SourceGraphic" mode="{mode}"/></filter>"#);
        run(&mut s, "effect.apply", json!({"effect": "svg.filter", "ids": [a.0], "params": {"xml": xml}}));
        let out = svg(&mut s);
        assert_eq!((out.contains("<image"), out.contains("<feBlend")), (image, !image), "{mode}: {out}");
    }
}
