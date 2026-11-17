//! Copy and paste between documents (resources, swatch conflicts) and paste placement (M4.22,
//! M4.23).

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::{Point, Rect};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

/// A second document in the same session (as when copying between windows); it becomes active.
fn new_doc(s: &mut Session) {
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn ids_of(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn ellipse(s: &mut Session) -> NodeId {
    id_of(&s.execute("shape.ellipse", &json!({"x": 300, "y": 200, "width": 40, "height": 20})).unwrap())
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn copy(s: &mut Session, ids: &[NodeId]) {
    sel(s, ids);
    s.execute("edit.copy", &json!({})).unwrap();
}

fn doc(s: &Session) -> &Document {
    &s.doc().unwrap().doc
}

fn node(s: &Session, id: NodeId) -> Node {
    doc(s).node(id).unwrap().clone()
}

fn fill(s: &Session, id: NodeId) -> Paint {
    node(s, id).appearance.fill_paint()
}

fn linked(color: Color, swatch: &str, tint: f32) -> Paint {
    Paint::Solid { color, swatch: Some(swatch.into()), tint }
}

fn hex(h: &str) -> Color {
    Color::from_hex(h).unwrap()
}

fn render(s: &Session) -> Vec<u8> {
    vectorcraft_render::Renderer::new().render_region(doc(s), Rect::new(0.0, 0.0, 400.0, 300.0), 1.0, true).pixels
}

/// A global swatch `name` of `hex` and a rectangle filled with it.
fn swatched_rect(s: &mut Session, name: &str, hex: &str) -> NodeId {
    s.execute("swatch.new", &json!({"name": name, "color": hex, "global": true})).unwrap();
    let r = rect(s, 20.0, 20.0, 40.0, 40.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": name})).unwrap();
    r
}

fn renamed(v: &Value) -> Vec<(String, String, String)> {
    let s = |r: &Value, k: &str| r[k].as_str().unwrap().to_string();
    v["renamed"].as_array().unwrap().iter().map(|r| (s(r, "kind"), s(r, "from"), s(r, "to"))).collect()
}

// ---------- M4.22: resources ----------

/// Verify ordinary paste retains image, symbol and pattern resources and their rendered appearance.
#[test]
fn an_image_symbol_and_pattern_rect_pasted_into_a_new_document_render_the_same() {
    resource_transfer(false);
}

/// Verify a translated artwork drop preserves editable resources, pattern phase, opacity and clipboard contents.
#[test]
fn dropped_artwork_preserves_editable_images_symbols_patterns_and_clipboard() {
    resource_transfer(true);
}

/// Exercise paste or tab-drop resource transfer and compare source and destination rendering.
fn resource_transfer(drag: bool) {
    let mut s = session();
    // Pattern "Dots" painting a rectangle.
    let dot = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [dot.0], "color": "#ff0000"})).unwrap();
    sel(&mut s, &[dot]);
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    sel(&mut s, &[dot]);
    s.execute("edit.clear", &json!({})).unwrap();
    let patterned = rect(&mut s, 20.0, 20.0, 100.0, 80.0);
    s.execute("paint.setFill", &json!({"ids": [patterned.0], "swatch": "Dots"})).unwrap();
    // Symbol "Badge" (a green square) placed once.
    let badge = rect(&mut s, 150.0, 20.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [badge.0], "color": "#00aa44"})).unwrap();
    sel(&mut s, &[badge]);
    s.execute("symbol.new", &json!({"name": "Badge"})).unwrap();
    // An embedded image (a rasterized blue ellipse).
    let e = id_of(&s.execute("shape.ellipse", &json!({"x": 240, "y": 40, "width": 90, "height": 60})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [e.0], "color": "#2040ff"})).unwrap();
    sel(&mut s, &[e]);
    s.execute("object.rasterize", &json!({"ppi": 72})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("transparency.set", &json!({"opacity":45})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(s.clipboard.nodes.len(), 3);
    assert_eq!((s.clipboard.symbols.len(), s.clipboard.patterns.len(), s.clipboard.images.len()), (1, 1, 1));
    let before = render(&s);
    let source = s.doc().unwrap().clone();
    let ids = source.selection.objects.clone();
    new_doc(&mut s);
    assert!(before != render(&s));
    let r = if drag {
        s.clipboard = Clipboard::default();
        s.execute("edit.dropSelection", &json!({"sourceDocument": source.uid, "ids": ids, "grab": [0,0], "at": [13,17]})).unwrap()
    } else {
        s.execute("edit.pasteInPlace", &json!({})).unwrap()
    };
    if drag {
        assert_eq!(s.clipboard, Clipboard::default());
    }
    assert_eq!(s.documents()[0].doc, source.doc);
    // The pattern (its swatch comes along), the symbol and the image blob.
    assert_eq!((r["added"].as_u64(), r["merged"].as_u64()), (Some(3), Some(0)), "{r}");
    let d = doc(&s);
    assert!(d.pattern("Dots").is_some() && d.swatch("Dots").is_some());
    assert!(d.symbols.iter().any(|x| x.name == "Badge"));
    assert_eq!(d.images.len(), 1);
    let after = if drag {
        vectorcraft_render::Renderer::new().render_region(doc(&s), Rect::new(13.0, 17.0, 413.0, 317.0), 1.0, true).pixels
    } else {
        render(&s)
    };
    assert!(before == after, "the transferred art, its transparency and pattern alignment render as in the source");
}

#[test]
fn a_spot_swatch_brings_its_plate() {
    let mut s = session();
    let r = swatched_rect(&mut s, "Spot Teal", "#008080");
    s.execute("swatch.setSpot", &json!({"name": "Spot Teal"})).unwrap();
    copy(&mut s, &[r]);
    new_doc(&mut s);
    let plates = |s: &Session| vectorcraft_render::proof::plates(doc(s)).iter().filter(|p| p.spot).map(|p| p.name.clone()).collect::<Vec<_>>();
    assert!(plates(&s).is_empty());
    s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(plates(&s), ["Spot Teal"]);
    let sw = doc(&s).swatch("Spot Teal").unwrap();
    assert!(sw.spot && sw.global);
}

#[test]
fn swatch_conflicts_merge_relinks_and_add_renames() {
    let mut s = session();
    let r = swatched_rect(&mut s, "Brand", "#ff0000");
    copy(&mut s, &[r]);
    new_doc(&mut s);
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#0000ff", "global": true})).unwrap();
    let c = s.execute("clipboard.conflicts", &json!({})).unwrap();
    assert_eq!(c["swatches"], json!([{"name": "Brand", "document": "#0000ff", "clipboard": "#ff0000", "spot": false}]));
    let swatches = doc(&s).swatches.len();

    // Merge (the default): the pasted object takes the document's Brand.
    let v = s.execute("edit.pasteInPlace", &json!({"swatchConflict": "merge"})).unwrap();
    assert_eq!((v["merged"].as_u64(), v["added"].as_u64()), (Some(1), Some(0)));
    assert_eq!(fill(&s, ids_of(&v)[0]), linked(hex("#0000ff"), "Brand", 1.0));
    assert_eq!(doc(&s).swatches.len(), swatches);
    s.execute("edit.undo", &json!({})).unwrap();

    // Add: the pasted swatch joins under a new name and the object follows it.
    let v = s.execute("edit.pasteInPlace", &json!({"swatchConflict": "add"})).unwrap();
    assert_eq!(v["renamed"], json!([{"kind": "swatch", "from": "Brand", "to": "Brand 2"}]));
    assert_eq!(fill(&s, ids_of(&v)[0]), linked(hex("#ff0000"), "Brand 2", 1.0));
    let added = doc(&s).swatch("Brand 2").unwrap();
    assert!(added.global && added.paint.color() == Some(hex("#ff0000")));
    s.execute("edit.undo", &json!({})).unwrap();

    // Per swatch name; the others merge.
    let v = s.execute("edit.paste", &json!({"swatchConflict": {"Other": "add"}})).unwrap();
    assert_eq!(v["merged"], 1);
    assert!(s.execute("edit.paste", &json!({"swatchConflict": "keep"})).is_err());
    assert!(s.execute("edit.paste", &json!({"swatchConflict": {"Brand": 1}})).is_err());
}

#[test]
fn tints_and_their_tint_swatches_follow_the_base() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Ink", "color": {"c": 0, "m": 100, "y": 100, "k": 0}, "spot": true})).unwrap();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": "Ink", "tint": 40})).unwrap();
    // The 40% tint swatch, and one of another tint the art doesn't use.
    assert_eq!(s.execute("swatch.new", &json!({})).unwrap()["name"], "Ink 40%");
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": "Ink", "tint": 70})).unwrap();
    s.execute("swatch.new", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": "Ink", "tint": 40})).unwrap();
    copy(&mut s, &[r]);
    assert_eq!(s.clipboard.swatches.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["Ink", "Ink 40%"]);

    // Into a document whose Ink is cyan: merged, the tint is 40% of the document's Ink.
    new_doc(&mut s);
    s.execute("swatch.new", &json!({"name": "Ink", "color": {"c": 100, "m": 0, "y": 0, "k": 0}, "spot": true})).unwrap();
    let v = s.execute("edit.pasteInPlace", &json!({})).unwrap();
    let cyan = Color::cmyk(1.0, 0.0, 0.0, 0.0);
    assert_eq!(fill(&s, ids_of(&v)[0]), linked(cyan.tinted(0.4), "Ink", 0.4));
    let tint = doc(&s).swatch("Ink 40%").unwrap();
    assert_eq!(tint.paint, linked(cyan.tinted(0.4), "Ink", 0.4), "the tint swatch links to the document's Ink");
    assert!(doc(&s).swatch("Ink 70%").is_none(), "unused tints stay behind");
    s.execute("edit.undo", &json!({})).unwrap();

    // Added: the tint and its tint swatch follow the renamed base.
    let v = s.execute("edit.pasteInPlace", &json!({"swatchConflict": "add"})).unwrap();
    let magenta = Color::cmyk(0.0, 1.0, 1.0, 0.0);
    assert_eq!(fill(&s, ids_of(&v)[0]), linked(magenta.tinted(0.4), "Ink 2", 0.4));
    assert_eq!(doc(&s).swatch("Ink 40%").unwrap().paint, linked(magenta.tinted(0.4), "Ink 2", 0.4));
}

#[test]
fn gradient_stops_stay_linked_to_their_swatches() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Sky", "color": "#3080ff", "global": true})).unwrap();
    let r = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "gradient": {"kind": "linear"}})).unwrap();
    s.edit("link stop", |d, _| {
        let n = d.node_mut(r).unwrap();
        let Paint::Gradient(mut g) = n.appearance.fill_paint() else { panic!() };
        g.gradient.stops[0].set_color(Color::from_hex("#3080ff").unwrap(), Some(("Sky".into(), 1.0)));
        n.appearance.set_fill(Paint::Gradient(g));
        Ok(())
    })
    .unwrap();
    copy(&mut s, &[r]);
    assert!(s.clipboard.swatches.iter().any(|w| w.name == "Sky"));
    new_doc(&mut s);
    s.execute("swatch.new", &json!({"name": "Sky", "color": "#000080", "global": true})).unwrap();
    let v = s.execute("edit.paste", &json!({"swatchConflict": "add"})).unwrap();
    let Paint::Gradient(g) = fill(&s, ids_of(&v)[0]) else { panic!() };
    assert_eq!((g.gradient.stops[0].swatch.as_deref(), g.gradient.stops[0].color), (Some("Sky 2"), hex("#3080ff")));
    assert!(doc(&s).swatch("Sky 2").is_some());
}

#[test]
fn gradient_swatches_come_along_with_their_stop_links() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Sky", "color": "#3080ff", "global": true})).unwrap();
    let glow = json!({"stops": [{"offset": 0, "swatch": "Sky"}, {"offset": 1, "color": "#ffffff"}]});
    s.execute("swatch.new", &json!({"name": "Glow", "gradient": glow})).unwrap();
    let r = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": "Glow"})).unwrap();
    copy(&mut s, &[r]);
    assert_eq!(s.clipboard.swatches.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["Sky", "Glow"]);
    // A document with another Glow and another Sky.
    new_doc(&mut s);
    s.execute("swatch.new", &json!({"name": "Sky", "color": "#000080", "global": true})).unwrap();
    s.execute("swatch.new", &json!({"name": "Glow", "gradient": {"stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#000000"}]}}))
        .unwrap();
    let v = s.execute("edit.paste", &json!({"swatchConflict": "add"})).unwrap();
    let Paint::Gradient(g) = fill(&s, ids_of(&v)[0]) else { panic!() };
    assert_eq!((g.swatch.as_deref(), g.gradient.stops[0].swatch.as_deref()), (Some("Glow 2"), Some("Sky 2")));
    let Paint::Gradient(sw) = doc(&s).swatch("Glow 2").unwrap().paint.clone() else { panic!() };
    assert_eq!(sw.gradient, g.gradient, "the added gradient swatch links its stop to the added Sky too");
}

/// A gradient swatch made from another one (by name, from a fill linked to it, or from a gradient
/// that names it, placed or not) holds the gradient alone, so copying art that uses it brings only
/// that swatch. A placed gradient keeps its aspect, and the swatch still fits each object it is
/// applied to.
#[test]
fn a_gradient_swatch_made_from_another_holds_no_link_to_it() {
    let mut s = session();
    let glow = json!({"kind": "radial", "stops": [{"offset": 0, "color": "#3080ff"}, {"offset": 1, "color": "#ffffff"}]});
    s.execute("swatch.new", &json!({"name": "Glow", "gradient": glow})).unwrap();
    let r = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": "Glow"})).unwrap();
    let Paint::Gradient(applied) = fill(&s, r) else { panic!("a gradient fill") };
    assert_eq!(applied.swatch.as_deref(), Some("Glow"));
    let mut by_gradient = vectorcraft_tools::params::gradient_params(&applied);
    by_gradient["swatch"] = json!("Glow");
    let mut placed = by_gradient.clone();
    placed["start"] = json!([25, 25]);
    placed["end"] = json!([50, 25]);
    placed["aspect"] = json!(50);
    let made = [
        ("By Name", json!({"name": "By Name", "swatch": "Glow"})),
        ("From Fill", json!({"name": "From Fill"})),
        ("From Gradient", json!({"name": "From Gradient", "gradient": by_gradient})),
        ("Placed", json!({"name": "Placed", "gradient": placed})),
    ];
    let mut links = vec![];
    for (name, p) in &made {
        s.execute("swatch.new", p).unwrap();
        let Paint::Gradient(sw) = doc(&s).swatch(name).unwrap().paint.clone() else { panic!("{name}: a gradient swatch") };
        assert_eq!(sw.gradient, applied.gradient, "{name}: Glow's gradient");
        links.push(sw.swatch.clone());
    }
    assert_eq!(links, [None, None, None, None], "no swatch links to Glow");
    let Paint::Gradient(sw) = doc(&s).swatch("Placed").unwrap().paint.clone() else { panic!("a gradient swatch") };
    assert!(sw.geom.is_some_and(|g| (g.aspect - 0.5).abs() < 1e-9), "the placed gradient keeps its aspect: {:?}", sw.geom);
    for (name, _) in &made {
        let t = rect(&mut s, 100.0, 0.0, 100.0, 60.0);
        s.execute("paint.setFill", &json!({"ids": [t.0], "swatch": name})).unwrap();
        if *name == "Placed" {
            let Paint::Gradient(g) = fill(&s, t) else { panic!("a gradient fill") };
            let geom = g.geom.unwrap();
            assert!(geom.start.distance(Point::new(150.0, 30.0)) < 1e-9 && (geom.aspect - 0.5).abs() < 1e-9, "{geom:?}");
        }
        copy(&mut s, &[t]);
        assert_eq!(s.clipboard.swatches.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), [*name], "Glow stays behind");
    }
}

#[test]
fn lab_spot_colours_show_as_the_target_documents_spot_option_says() {
    let mut s = session();
    let lab = Color::lab(60.0, 50.0, 20.0);
    s.execute("swatch.new", &json!({"name": "Lab Ink", "color": {"l": 60, "a": 50, "b": 20}, "spot": true})).unwrap();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": "Lab Ink"})).unwrap();
    assert_eq!(fill(&s, r).color(), Some(lab));
    copy(&mut s, &[r]);
    new_doc(&mut s);
    s.execute("swatch.spotOptions", &json!({"useLab": false})).unwrap();
    let id = ids_of(&s.execute("edit.paste", &json!({})).unwrap())[0];
    let cmyk = doc(&s).global_color("Lab Ink").unwrap();
    assert!(matches!(cmyk, Color::Cmyk { .. }));
    assert_eq!(fill(&s, id), linked(cmyk, "Lab Ink", 1.0), "the CMYK equivalent, still linked");
    assert_eq!(doc(&s).swatch("Lab Ink").unwrap().paint.color(), Some(lab), "the swatch keeps its Lab definition");
}

#[test]
fn registration_is_never_copied_as_a_swatch() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.edit("registration", |d, _| {
        d.node_mut(r).unwrap().appearance.set_fill(Paint::registration());
        Ok(())
    })
    .unwrap();
    copy(&mut s, &[r]);
    assert!(s.clipboard.swatches.is_empty());
    new_doc(&mut s);
    let swatches = doc(&s).swatches.len();
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(v["added"], 0);
    assert_eq!(doc(&s).swatches.len(), swatches);
    assert!(fill(&s, ids_of(&v)[0]).is_registration());
}

#[test]
fn graphic_style_links_follow_the_style_into_the_document() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 30.0, 30.0);
    s.execute("paint.setFill", &json!({"ids": [a.0], "color": "#ffcc00"})).unwrap();
    sel(&mut s, &[a]);
    s.execute("graphicStyle.new", &json!({"name": "Sun"})).unwrap();
    let src_id = node(&s, a).extras().graphic_style.unwrap();
    copy(&mut s, &[a]);
    assert_eq!(s.clipboard.graphic_styles.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Sun"]);

    // A document with another "Sun" and styles taking up the source id.
    new_doc(&mut s);
    let b = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [b.0], "color": "#0000ff"})).unwrap();
    sel(&mut s, &[b]);
    s.execute("graphicStyle.new", &json!({"name": "Sun"})).unwrap();
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(renamed(&v), [("graphicStyle".to_string(), "Sun".to_string(), "Sun 2".to_string())]);
    let d = doc(&s);
    let linked_to = d.graphic_style_by_id(node(&s, ids_of(&v)[0]).extras().graphic_style.unwrap()).unwrap().clone();
    assert_eq!(linked_to.name, "Sun 2");
    assert_eq!(linked_to.appearance.fill_paint(), Paint::solid(hex("#ffcc00")));
    assert!(d.graphic_styles.iter().filter(|g| g.id == linked_to.id).count() == 1, "a fresh id");
    // Pasting again finds the same style.
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!((v["added"].as_u64(), node(&s, ids_of(&v)[0]).extras().graphic_style), (Some(0), Some(linked_to.id)));
    // Back in the source document the link is kept as it was.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(node(&s, ids_of(&v)[0]).extras().graphic_style, Some(src_id));
}

#[test]
fn undo_removes_everything_the_paste_added() {
    let mut s = session();
    let r = swatched_rect(&mut s, "Brand", "#ff0000");
    sel(&mut s, &[r]);
    s.execute("symbol.new", &json!({"name": "Mark"})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    new_doc(&mut s);
    let before = doc(&s).clone();
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(v["added"], 2, "{v}");
    assert!(doc(&s).swatch("Brand").is_some() && !doc(&s).symbols.is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(*doc(&s) == before);
}

#[test]
fn styles_symbols_and_images_of_the_same_name_are_renamed() {
    let mut s = session();
    // A character style, a symbol and an image named like the target's own.
    let t = id_of(&s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hello"})).unwrap());
    s.execute("charStyle.new", &json!({"name": "Emphasis", "attrs": {"size": 24.0}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Emphasis", "id": t.0})).unwrap();
    let m = rect(&mut s, 200.0, 100.0, 30.0, 30.0);
    sel(&mut s, &[m]);
    let mark = id_of(&s.execute("symbol.new", &json!({"name": "Mark"})).unwrap());
    let e = rect(&mut s, 300.0, 100.0, 20.0, 20.0);
    sel(&mut s, &[e]);
    let img = id_of(&s.execute("object.rasterize", &json!({"ppi": 72})).unwrap());
    let key = |s: &Session, id: NodeId| match node(s, id).kind {
        NodeKind::Image(im) => im.key,
        _ => panic!("not an image"),
    };
    let src_key = key(&s, img);
    copy(&mut s, &[t, mark, img]);

    new_doc(&mut s);
    s.execute("charStyle.new", &json!({"name": "Emphasis", "attrs": {"size": 8.0}})).unwrap();
    let m2 = ellipse(&mut s);
    sel(&mut s, &[m2]);
    s.execute("symbol.new", &json!({"name": "Mark"})).unwrap();
    // Another image under the same key.
    let e2 = ellipse(&mut s);
    sel(&mut s, &[e2]);
    let img2 = id_of(&s.execute("object.rasterize", &json!({"ppi": 72})).unwrap());
    let other_key = key(&s, img2);
    s.edit("same key", |d, _| {
        let blob = d.images.remove(&other_key).unwrap();
        d.images.insert(src_key.clone(), blob);
        let NodeKind::Image(im) = &mut d.node_mut(img2).unwrap().kind else { panic!() };
        im.key = src_key.clone();
        Ok(())
    })
    .unwrap();

    let v = s.execute("edit.pasteInPlace", &json!({})).unwrap();
    let kinds: Vec<(String, String)> = renamed(&v).into_iter().map(|(k, _, to)| (k, to)).collect();
    for want in [("charStyle", "Emphasis 2"), ("symbol", "Mark 2"), ("image", &*format!("{src_key} 2"))] {
        assert!(kinds.contains(&(want.0.to_string(), want.1.to_string())), "{want:?} in {kinds:?}");
    }
    let ids = ids_of(&v);
    let NodeKind::Text(tx) = node(&s, ids[0]).kind else { panic!() };
    assert_eq!(tx.runs[0].style.style_name.as_deref(), Some("Emphasis 2"));
    assert!(matches!(node(&s, ids[1]).kind, NodeKind::SymbolInstance { ref symbol, .. } if symbol == "Mark 2"));
    let new_key = key(&s, ids[2]);
    assert_ne!(new_key, src_key, "a different image under the same key is re-keyed");
    assert!(doc(&s).images[&new_key].bytes != doc(&s).images[&src_key].bytes);
    // Their own objects are untouched.
    assert_eq!(key(&s, img2), src_key);
}

/// A graph whose series fill, value labels, legend rows and axis and ring line rows paint with global swatch "Brand" and
/// whose value labels are set in character style "Emphasis" and paragraph style "Body", as a
/// later redraw would give them to its marks and labels.
fn styled_graph(s: &mut Session) -> NodeId {
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#ff0000", "global": true})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Emphasis", "attrs": {"size": 24.0}})).unwrap();
    s.execute("paraStyle.new", &json!({"name": "Body", "attrs": {"space_after": 8.0}})).unwrap();
    let v = s.execute("graph.create", &json!({"type": "column", "x": 200, "y": 200, "width": 60, "height": 40, "csv": ",2024\nQ1,3\nQ2,5"}));
    let id = id_of(&v.unwrap());
    let brand = linked(hex("#ff0000"), "Brand", 1.0);
    let mut extras = doc_mut(s).node_mut(id).unwrap().extras_mut();
    let g = extras.graph.as_mut().unwrap();
    g.series_paints = vec![vectorcraft_doc::SeriesPaint { fill: Some(brand.clone()), ..Default::default() }];
    let l = &mut g.label_styles;
    l.values = Some(vectorcraft_doc::CharStyle { fill: brand.clone(), style_name: Some("Emphasis".into()), ..Default::default() });
    l.values_para = Some(vectorcraft_doc::ParaStyle { style_name: Some("Body".into()), ..Default::default() });
    let paint = vectorcraft_doc::SeriesPaint { stroke: Some(brand), ..Default::default() };
    let rows = vectorcraft_doc::graph::LabelRows { paint, ..Default::default() };
    (l.legend_rows, l.axis_rows, l.ring_rows) = (Some(rows.clone()), Some(rows.clone()), Some(rows));
    id
}

/// The swatch names a graph's stored paints link to, and its value labels' character and
/// paragraph style names.
fn graph_links(s: &Session, id: NodeId) -> (Vec<String>, Option<String>, Option<String>) {
    let n = node(s, id);
    let swatches = vectorcraft_doc::swatches::graph_paints(&n).filter_map(|p| match p {
        Paint::Solid { swatch: Some(w), .. } => Some(w.clone()),
        _ => None,
    });
    let l = &n.extras().graph.as_deref().unwrap().label_styles;
    let values = l.values.as_ref().and_then(|st| st.style_name.clone());
    (swatches.collect(), values, l.values_para.as_ref().and_then(|pa| pa.style_name.clone()))
}

/// A pasted graph brings the swatches and text styles its series and labels keep, and follows
/// them when they come in under new names.
#[test]
fn a_pasted_graph_brings_and_follows_its_label_styles_and_paints() {
    let mut s = session();
    let g = styled_graph(&mut s);
    copy(&mut s, &[g]);

    new_doc(&mut s);
    let v = s.execute("edit.pasteInPlace", &json!({})).unwrap();
    let d = doc(&s);
    assert!(d.swatch("Brand").is_some(), "{v}");
    assert!(d.char_styles.iter().any(|x| x.name == "Emphasis") && d.para_styles.iter().any(|x| x.name == "Body"), "{v}");
    let brand = vec!["Brand".to_string(); 5];
    assert_eq!(graph_links(&s, ids_of(&v)[0]), (brand.clone(), Some("Emphasis".into()), Some("Body".into())));

    // Into a document that gives each name something else: they come in renamed.
    new_doc(&mut s);
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#0000ff", "global": true})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Emphasis", "attrs": {"size": 8.0}})).unwrap();
    s.execute("paraStyle.new", &json!({"name": "Body", "attrs": {"space_after": 2.0}})).unwrap();
    let v = s.execute("edit.pasteInPlace", &json!({"swatchConflict": "add"})).unwrap();
    let brand2 = vec!["Brand 2".to_string(); 5];
    assert_eq!(graph_links(&s, ids_of(&v)[0]), (brand2, Some("Emphasis 2".into()), Some("Body 2".into())), "{v}");

    // Without formatting: the labels' text styles stay behind.
    new_doc(&mut s);
    let v = s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap();
    assert!(doc(&s).char_styles.is_empty() && doc(&s).para_styles.is_empty(), "{v}");
    // The value labels' red characters stay behind too; the series fill, the legend rows and the line rows keep Brand.
    assert_eq!(graph_links(&s, ids_of(&v)[0]), (vec!["Brand".to_string(); 4], None, None));
}

/// Each label of graph `id` (the text of its Axes and Legend groups) with its size and its first paragraph's format.
fn graph_label_looks(s: &Session, id: NodeId) -> Vec<(String, f64, vectorcraft_doc::ParaStyle)> {
    let mut out = vec![];
    node(s, id).walk(&mut |n| {
        if let NodeKind::Text(t) = &n.kind {
            out.push((t.plain_text(), t.first_style().size, t.para_at(0).clone()));
        }
    });
    out
}

/// Paste without Formatting sets a pasted graph's labels in a new graph's label size and paragraph attributes, and the
/// graph keeps them when it is drawn again: the character and paragraph formats its labels keep stay behind with their
/// text formatting.
#[test]
fn a_graph_pasted_without_formatting_keeps_plain_labels_when_drawn_again() {
    let mut s = session();
    let csv = ",2024,2025\nQ1,3,2\nQ2,5,4";
    let g = id_of(&s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 120, "height": 80, "csv": csv})).unwrap());
    let fresh = graph_label_looks(&s, g);
    sel(&mut s, &[g]);
    s.execute("text.setStyle", &json!({"size": 14})).unwrap();
    s.execute("text.setFormat", &json!({"spaceAfter": 6})).unwrap();
    assert!(graph_label_looks(&s, g).iter().all(|(_, size, para)| *size == 14.0 && para.space_after == 6.0));
    // The labels as drawn are also set in a character style.
    s.execute("charStyle.new", &json!({"name": "Emphasis", "attrs": {"size": 14.0}})).unwrap();
    let mut labels = vec![];
    node(&s, g).walk(&mut |n| {
        if matches!(n.kind, NodeKind::Text(_)) {
            labels.push(n.id);
        }
    });
    for id in labels {
        if let Some(NodeKind::Text(t)) = doc_mut(&mut s).node_mut(id).map(|n| &mut n.kind) {
            t.runs.iter_mut().for_each(|r| r.style.style_name = Some("Emphasis".into()));
        }
    }
    copy(&mut s, &[g]);
    assert_eq!(s.clipboard.char_styles.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Emphasis"]);
    // Where a redraw of the copied graph puts its plot.
    s.execute("graph.setData", &json!({"id": g.0, "csv": csv})).unwrap();
    let plot = node(&s, g).extras().graph.as_deref().unwrap().rect;

    new_doc(&mut s);
    let steps = s.doc().unwrap().history.undo.len();
    let v = s.execute("edit.pasteWithoutFormatting", &json!({"dx": 10, "dy": 20})).unwrap();
    let p = ids_of(&v)[0];
    assert!(doc(&s).char_styles.is_empty(), "the labels' character style stays behind: {v}");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Paste without Formatting"));
    let l = node(&s, p).extras().graph.as_deref().unwrap().label_styles.clone();
    assert_eq!((l.values, l.categories, l.legend), (None, None, None));
    assert_eq!((l.values_para, l.categories_para, l.legend_para), (None, None, None));
    let pasted = graph_label_looks(&s, p);
    assert_eq!(pasted, fresh, "the labels are set as a new graph's are");

    s.execute("graph.setData", &json!({"id": p.0, "csv": csv})).unwrap();
    assert_eq!(graph_label_looks(&s, p), pasted, "and stay so when the graph is drawn again");
    let r = node(&s, p).extras().graph.as_deref().unwrap().rect;
    let off = [r.x0 - plot.x0 - 10.0, r.y0 - plot.y0 - 20.0, r.x1 - plot.x1 - 10.0, r.y1 - plot.y1 - 20.0];
    assert!(off.iter().all(|d| d.abs() < 1e-6), "the plot moves by the offset and keeps its size: {plot:?} → {r:?}");
    s.execute("graph.setType", &json!({"id": p.0, "type": "column", "dropShadow": true})).unwrap();
    assert_eq!(graph_label_looks(&s, p), pasted, "a change of graph type options draws them the same way");

    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(doc(&s).node(p).is_none(), "one undo takes the paste back");
}

/// Paste without Formatting leaves the rest of a pasted graph's art as it was copied: a column painted by hand keeps its
/// paint, and nothing is drawn again.
#[test]
fn a_graph_pasted_without_formatting_keeps_its_art() {
    let mut s = session();
    let csv = ",2024,2025\nQ1,3,2\nQ2,5,4";
    let g = id_of(&s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 120, "height": 80, "csv": csv})).unwrap());
    let red = Paint::solid(hex("ff0000"));
    let mut column = None;
    node(&s, g).walk(&mut |n| {
        if column.is_none() && matches!(n.kind, NodeKind::Path { .. }) && n.appearance.fill_paint() != Paint::None {
            column = Some(n.id);
        }
    });
    let column = column.unwrap();
    let n = doc_mut(&mut s).node_mut(column).unwrap();
    n.appearance = vectorcraft_doc::Appearance::basic(red.clone(), Paint::None, 0.0).into();
    /// Every object of graph `id` that is not type: its kind's name, its fill and its bounds.
    fn art(s: &Session, id: NodeId) -> Vec<(String, Paint, Option<Rect>)> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if !matches!(n.kind, NodeKind::Text(_)) {
                out.push((format!("{:?}", std::mem::discriminant(&n.kind)), n.appearance.fill_paint(), n.geometric_bounds()));
            }
        });
        out
    }
    let copied = art(&s, g);
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({"dx": 15, "dy": 25})).unwrap())[0];
    let pasted = art(&s, p);
    let dx = pasted[0].2.unwrap().x0 - copied[0].2.unwrap().x0;
    let dy = pasted[0].2.unwrap().y0 - copied[0].2.unwrap().y0;
    let moved: Vec<_> = copied.into_iter().map(|(k, f, b)| (k, f, b.map(|b| b + vectorcraft_geom::Vec2::new(dx, dy)))).collect();
    assert_eq!(pasted.len(), moved.len());
    for (a, b) in pasted.iter().zip(&moved) {
        assert_eq!((&a.0, &a.1), (&b.0, &b.1));
        let (a, b) = (a.2.unwrap(), b.2.unwrap());
        assert!((a.x0 - b.x0).abs() + (a.y0 - b.y0).abs() + (a.x1 - b.x1).abs() + (a.y1 - b.y1).abs() < 1e-6, "{a:?} {b:?}");
    }
    assert!(pasted.iter().any(|(_, f, _)| *f == red), "the column keeps the paint it was given");
}

/// The labels of a scaled graph pasted without formatting are set at a new graph's label size, unscaled, as a redraw
/// sets them.
#[test]
fn a_scaled_graph_pasted_without_formatting_gets_unscaled_labels() {
    let mut s = session();
    let csv = ",2024\nQ1,3\nQ2,5";
    let g = id_of(&s.execute("graph.create", &json!({"type": "column", "x": 50, "y": 50, "width": 120, "height": 80, "csv": csv})).unwrap());
    let fresh = graph_label_looks(&s, g)[0].1;
    sel(&mut s, &[g]);
    s.execute("object.scale", &json!({"sx": 200, "sy": 200})).unwrap();
    /// Each label's size as drawn: its character size times the scale of its transform.
    fn drawn_sizes(s: &Session, id: NodeId) -> Vec<f64> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind {
                out.push(t.first_style().size * t.xf.determinant().abs().sqrt());
            }
        });
        out
    }
    assert!(drawn_sizes(&s, g).iter().all(|z| (z - fresh * 2.0).abs() < 1e-6), "{:?}", drawn_sizes(&s, g));
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert!(drawn_sizes(&s, p).iter().all(|z| (z - fresh).abs() < 1e-6), "{:?}", drawn_sizes(&s, p));
    s.execute("graph.setData", &json!({"id": p.0, "csv": csv})).unwrap();
    assert!(drawn_sizes(&s, p).iter().all(|z| (z - fresh).abs() < 1e-6), "{:?}", drawn_sizes(&s, p));

    // A rotated graph's labels keep their rotation, so they stay turned with the rest of its art.
    /// Each label's rotation in degrees: the angle of its transform's x axis.
    fn angles(s: &Session, id: NodeId) -> Vec<f64> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind {
                let [a, b, ..] = t.xf.as_coeffs();
                out.push(b.atan2(a).to_degrees());
            }
        });
        out
    }
    let r = id_of(&s.execute("graph.create", &json!({"type": "column", "x": 300, "y": 50, "width": 120, "height": 80, "csv": csv})).unwrap());
    sel(&mut s, &[r]);
    s.execute("object.scale", &json!({"sx": 200, "sy": 200})).unwrap();
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    let turned = angles(&s, r);
    assert!(turned.iter().all(|a| (a.abs() - 30.0).abs() < 1e-6), "{turned:?}");
    copy(&mut s, &[r]);
    new_doc(&mut s);
    let q = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(angles(&s, q).len(), turned.len());
    assert!(angles(&s, q).iter().zip(&turned).all(|(a, b)| (a - b).abs() < 1e-6), "{:?} against {turned:?}", angles(&s, q));
    assert!(drawn_sizes(&s, q).iter().all(|z| (z - fresh).abs() < 1e-6), "{:?}", drawn_sizes(&s, q));
}

/// A pie's legend labels in their wedges keep a redraw's contrast when pasted without formatting: white on a dark
/// wedge, black on a light one.
#[test]
fn pie_labels_in_wedges_pasted_without_formatting_stay_readable() {
    let mut s = session();
    let csv = ",North,South\nP,1,1";
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    /// Each text of graph `id` with its characters' fill.
    fn fills(s: &Session, id: NodeId) -> Vec<(String, Paint)> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind {
                out.push((t.plain_text(), t.first_style().fill.clone()));
            }
        });
        out
    }
    let drawn = fills(&s, g);
    assert!(drawn.contains(&("North".into(), Paint::solid(Color::WHITE))), "the first wedge is black: {drawn:?}");
    sel(&mut s, &[g]);
    s.execute("text.setStyle", &json!({"size": 14})).unwrap();
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(fills(&s, p), drawn);
    s.execute("graph.setData", &json!({"id": p.0, "csv": csv})).unwrap();
    assert_eq!(fills(&s, p), drawn);
}

/// A pie whose legend labels were given a black fill of their own loses that mark when pasted without formatting, so a
/// later change to a legend label's size does not bring the black fill back on the dark wedge.
#[test]
fn a_legend_fill_of_its_own_stays_behind_when_pasted_without_formatting() {
    let mut s = session();
    let csv = ",North,South\nP,1,1";
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    /// The legend label of graph `id` that says `name`.
    fn label(s: &Session, id: NodeId, name: &str) -> (NodeId, Paint) {
        let mut out = None;
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && t.plain_text() == name
            {
                out = Some((n.id, t.first_style().fill.clone()));
            }
        });
        out.unwrap()
    }
    let (north, fill) = label(&s, g, "North");
    assert_eq!(fill, Paint::solid(Color::WHITE), "white on the black wedge");
    s.execute("text.setStyle", &json!({"ids": [north.0], "fill": "#000000"})).unwrap();
    assert!(node(&s, g).extras().graph.as_deref().unwrap().label_styles.legend_fill_set);
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert!(!node(&s, p).extras().graph.as_deref().unwrap().label_styles.legend_fill_set);
    assert_eq!(label(&s, p, "North").1, Paint::solid(Color::WHITE), "pasted as a redraw sets it");
    let (north, _) = label(&s, p, "North");
    s.execute("text.setStyle", &json!({"ids": [north.0], "size": 11})).unwrap();
    s.execute("graph.setData", &json!({"id": p.0, "csv": csv})).unwrap();
    assert_eq!(label(&s, p, "North").1, Paint::solid(Color::WHITE), "still white when the pie is drawn again");
}

/// A pie pasted into a document whose swatch of the same name is another color takes that color on its linked wedges
/// (Merge), and its legend labels in those wedges take the contrast a redraw gives them there: white on a wedge that
/// turned dark. Pasting with Add, which keeps the copied color, keeps the labels as they were.
#[test]
fn pie_legend_labels_follow_wedges_that_take_the_documents_swatch() {
    let mut s = session();
    let csv = ",North,South\nP,1,1";
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#ffff66", "global": true})).unwrap();
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    /// The fill of the legend label of graph `id` that says `name`.
    fn label(s: &Session, id: NodeId, name: &str) -> Paint {
        let mut out = None;
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && t.plain_text() == name
            {
                out = Some(t.first_style().fill.clone());
            }
        });
        out.unwrap()
    }
    // The North wedge, black when drawn, takes the light Brand swatch, and a redraw sets its label black.
    let mut wedges = vec![];
    for c in node(&s, g).children().unwrap().iter().filter(|c| c.extras().series_index == Some(0)) {
        c.walk(&mut |m| {
            if matches!(m.kind, NodeKind::Path { .. }) {
                wedges.push(m.id.0);
            }
        });
    }
    assert!(!wedges.is_empty());
    s.execute("paint.setFill", &json!({"ids": wedges, "swatch": "Brand"})).unwrap();
    s.execute("graph.setData", &json!({"id": g.0, "csv": csv})).unwrap();
    assert_eq!(label(&s, g, "North"), Paint::solid(Color::BLACK), "black on the light wedge");
    assert_eq!(label(&s, g, "South"), Paint::solid(Color::BLACK));
    copy(&mut s, &[g]);

    for (cmd, choice, north) in [
        ("edit.pasteWithoutFormatting", "merge", Color::WHITE),
        ("edit.paste", "merge", Color::WHITE),
        ("edit.pasteWithoutFormatting", "add", Color::BLACK),
        ("edit.paste", "add", Color::BLACK),
    ] {
        new_doc(&mut s);
        s.execute("swatch.new", &json!({"name": "Brand", "color": "#202020", "global": true})).unwrap();
        let p = ids_of(&s.execute(cmd, &json!({"swatchConflict": choice})).unwrap())[0];
        assert_eq!(label(&s, p, "North"), Paint::solid(north), "{cmd} {choice}");
        assert_eq!(label(&s, p, "South"), Paint::solid(Color::BLACK), "{cmd} {choice}");
        s.execute("graph.setData", &json!({"id": p.0, "csv": csv})).unwrap();
        assert_eq!(label(&s, p, "North"), Paint::solid(north), "{cmd} {choice}: as a redraw sets it");
    }

    // A copy of the pie in the opacity mask of a rectangle follows the same way.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    let r = rect(&mut s, 10.0, 10.0, 300.0, 300.0);
    let art = {
        let d = doc_mut(&mut s);
        let pie = d.node(g).unwrap().clone();
        d.reid(&pie)
    };
    doc_mut(&mut s).node_mut(r).unwrap().extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(art, true)));
    copy(&mut s, &[r]);
    new_doc(&mut s);
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#202020", "global": true})).unwrap();
    let p = ids_of(&s.execute("edit.paste", &json!({"swatchConflict": "merge"})).unwrap())[0];
    let mask = node(&s, p).extras().mask.as_deref().unwrap().art.clone();
    let mut north = None;
    mask.walk(&mut |n| {
        if let NodeKind::Text(t) = &n.kind
            && t.plain_text() == "North"
        {
            north = Some(t.first_style().fill.clone());
        }
    });
    assert_eq!(north, Some(Paint::solid(Color::WHITE)), "in an opacity mask");
}

/// A two-line legend label in a narrow dark wedge, whose text sits off the wedge's middle line, stays white when its graph
/// is pasted without formatting.
#[test]
fn a_legend_label_in_a_narrow_wedge_pasted_without_formatting_stays_readable() {
    let mut s = session();
    // The first wedge fills a quarter from the top; the dark fourth series ("Narrow", series 3) follows it for 0.2 degrees,
    // narrower than the label's distance from the wedge's middle line.
    let csv = ",A,B,C,Narrow|,E\nP,9,0,0,0.02,26.98";
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    fn narrow(s: &Session, id: NodeId) -> Paint {
        let mut out = None;
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && t.plain_text().starts_with("Narrow")
            {
                out = Some(t.first_style().fill.clone());
            }
        });
        out.unwrap()
    }
    assert_eq!(narrow(&s, g), Paint::solid(Color::WHITE));
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(narrow(&s, p), Paint::solid(Color::WHITE));
    s.execute("graph.setData", &json!({"id": p.0, "csv": csv})).unwrap();
    assert_eq!(narrow(&s, p), Paint::solid(Color::WHITE));
    // A standard legend beside the pie stays black.
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "standard"})).unwrap();
    assert_eq!(narrow(&s, g), Paint::solid(Color::BLACK));
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(narrow(&s, p), Paint::solid(Color::BLACK));
    // Two series with one name: each label keeps its own wedge's contrast, though the second label lies nearer the
    // middle of the first (dark) wedge's bounds than of its own.
    fn xs(s: &Session, id: NodeId) -> Vec<Paint> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && t.plain_text() == "X"
            {
                out.push(t.first_style().fill.clone());
            }
        });
        out
    }
    let csv = ",X,X,Y\nP,8,1,89";
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 0, "y": 0, "width": 2000, "height": 2000, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    let drawn = xs(&s, g);
    assert_eq!(drawn, vec![Paint::solid(Color::WHITE), Paint::solid(Color::BLACK)]);
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(xs(&s, p), drawn);
}

/// Two-line legend labels of a light and a dark series with one name keep a redraw's colors when pasted without
/// formatting, though the narrow light wedge's label reaches into the dark wedge after it.
#[test]
fn shared_name_labels_in_a_narrow_wedge_pasted_without_formatting_keep_their_colors() {
    let mut s = session();
    let csv = ",A,B,Q|,Q|\nP,9,0,0.02,26.98";
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    fn qs(s: &Session, id: NodeId) -> Vec<Paint> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && t.plain_text().starts_with('Q')
            {
                out.push(t.first_style().fill.clone());
            }
        });
        out
    }
    let drawn = qs(&s, g);
    assert_eq!(drawn.len(), 2);
    assert_ne!(drawn[0], drawn[1], "one light and one dark series");
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(qs(&s, p), drawn);
    // Names that hold a line break of their own (a redraw sets them on lines centered on the point, as it sets `|`) keep
    // them too.
    for name in ["Q\nQ", "Q\nQ\nQ\nQ\nQ\nQ\nQ\nQ\nQ"] {
        new_doc(&mut s);
        let named = json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "series": ["A", "B", name, name], "categories": ["P"], "rows": [[9, 0, 0.02, 26.98]]});
        let g = id_of(&s.execute("graph.create", &named).unwrap());
        s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
        let broken = qs(&s, g);
        assert_eq!(broken.len(), 2);
        assert_ne!(broken[0], broken[1], "names with line breaks");
        copy(&mut s, &[g]);
        new_doc(&mut s);
        let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
        assert_eq!(qs(&s, p), broken, "names with {} line breaks", name.matches('\n').count());
    }
    // So do labels whose names show the same text, one with a line break of its own and one with `|`.
    new_doc(&mut s);
    let named = json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "series": ["Q\n", "B", "Q|", "Q|"], "categories": ["P"], "rows": [[9, 0, 0.02, 26.98]]});
    let g = id_of(&s.execute("graph.create", &named).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    // The fills of the Q labels by their place from left to right and top to bottom.
    fn placed(s: &Session, id: NodeId) -> Vec<Paint> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && t.plain_text().starts_with('Q')
            {
                let o = t.xf.translation();
                out.push((o.x, o.y, t.first_style().fill.clone()));
            }
        });
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        out.into_iter().map(|(_, _, f)| f).collect()
    }
    let alike = placed(&s, g);
    assert_eq!(alike.len(), 3);
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(placed(&s, p), alike, "names that show the same text");
    // The narrow light wedge's label (the second) brought to the front keeps them too.
    new_doc(&mut s);
    let g = id_of(&s.execute("graph.create", &named).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    let legend = node(&s, g).children().unwrap().iter().find(|c| c.name.as_deref() == Some("Legend")).unwrap().clone();
    let labels = legend.children().unwrap();
    assert_eq!(labels.len(), 3);
    s.execute("object.arrange.bringToFront", &json!({"ids": [labels[1].id.0]})).unwrap();
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(placed(&s, p), alike, "names that show the same text, labels reordered");
    // Labels given another size, which leaves them where they were drawn, keep them too.
    new_doc(&mut s);
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    sel(&mut s, &[g]);
    s.execute("text.setStyle", &json!({"size": 14})).unwrap();
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(qs(&s, p), drawn, "resized labels");
    // Large labels of four series of one name, drawn again at that size, keep the colors that drawing gave them.
    new_doc(&mut s);
    let four = ",Q|,Q|,Q|,Q|\nP,1,1,1,1";
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": four})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    sel(&mut s, &[g]);
    s.execute("text.setStyle", &json!({"size": 200})).unwrap();
    s.execute("graph.setData", &json!({"id": g.0, "csv": four})).unwrap();
    let large = placed(&s, g);
    assert_eq!(large.len(), 4);
    assert!(large.contains(&Paint::solid(Color::WHITE)) && large.contains(&Paint::solid(Color::BLACK)), "{large:?}");
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(placed(&s, p), large, "large labels drawn again");
    // A scaled and rotated pie keeps them too.
    new_doc(&mut s);
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    sel(&mut s, &[g]);
    s.execute("object.scale", &json!({"sx": 400, "sy": 400})).unwrap();
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    let drawn = qs(&s, g);
    assert_ne!(drawn[0], drawn[1]);
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(qs(&s, p), drawn);
}

/// The legend labels of a pie that kept a fill of their own (red, or black on every wedge), scaled, with two series of
/// one name, are pasted without formatting in the colors a redraw gives them.
#[test]
fn recolored_wedge_labels_pasted_without_formatting_match_a_redraw() {
    for fill in ["#ff0000", "#000000"] {
        recolored_wedge_labels_match_a_redraw(fill);
    }
}

fn recolored_wedge_labels_match_a_redraw(fill: &str) {
    let mut s = session();
    let csv = ",A,B,Q|,Q|\nP,9,0,0.02,26.98";
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    sel(&mut s, &[g]);
    s.execute("text.setStyle", &json!({"fill": fill})).unwrap();
    s.execute("object.scale", &json!({"sx": 400, "sy": 400})).unwrap();
    fn legend(s: &Session, id: NodeId) -> Vec<(String, Paint)> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && ["A", "Q"].iter().any(|p| t.plain_text().starts_with(p))
            {
                out.push((t.plain_text(), t.first_style().fill.clone()));
            }
        });
        out
    }
    assert!(legend(&s, g).iter().all(|(_, f)| *f == Paint::solid(hex(fill))), "{fill}: {:?}", legend(&s, g));
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    let pasted = legend(&s, p);
    s.execute("graph.setData", &json!({"id": p.0, "csv": csv})).unwrap();
    let redrawn = legend(&s, p);
    assert_eq!(pasted, redrawn, "{fill}");
    assert!(
        redrawn.iter().any(|(_, f)| *f == Paint::solid(Color::WHITE)) && redrawn.iter().any(|(_, f)| *f == Paint::solid(Color::BLACK)),
        "{fill}: {redrawn:?}"
    );
}

/// A wedge whose fill was changed after its label was drawn gives the label, pasted without formatting, the color a
/// redraw gives it.
#[test]
fn a_repainted_wedge_sets_its_label_color_when_pasted_without_formatting() {
    let mut s = session();
    let csv = ",North,South\nP,1,1";
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    s.execute("graph.setData", &json!({"id": g.0, "csv": csv})).unwrap();
    fn labels(s: &Session, id: NodeId) -> Vec<(String, Paint)> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && ["North", "South"].contains(&t.plain_text().as_str())
            {
                out.push((t.plain_text(), t.first_style().fill.clone()));
            }
        });
        out
    }
    // Each wedge takes the other's contrast: the dark one (its label white) white, the light one black.
    let drawn = labels(&s, g);
    let white = |name: &str| drawn.iter().any(|(t, f)| t == name && *f == Paint::solid(Color::WHITE));
    let mut wedges = vec![];
    for c in node(&s, g).children().unwrap().iter() {
        let Some(series) = c.extras().series_index else { continue };
        let dark = white(["North", "South"][series as usize]);
        c.walk(&mut |m| {
            if let NodeKind::Path { .. } = m.kind {
                wedges.push((m.id, dark));
            }
        });
    }
    assert!(wedges.iter().any(|w| w.1) && wedges.iter().any(|w| !w.1), "{drawn:?}");
    for (id, dark) in &wedges {
        s.execute("paint.setFill", &json!({"ids": [id.0], "color": if *dark { "#ffffff" } else { "#000000" }})).unwrap();
    }
    let before = labels(&s, g);
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    let pasted = labels(&s, p);
    s.execute("graph.setData", &json!({"id": p.0, "csv": csv})).unwrap();
    assert_eq!(pasted, labels(&s, p), "copied as {before:?}");
}

/// Wedges linked to a global swatch that the destination document gives a dark color, merged on Paste without
/// Formatting, give their legend labels white characters, as a redraw does; added under a new name, they keep the
/// copied light color and black labels.
#[test]
fn wedges_merged_into_a_dark_swatch_give_white_labels_when_pasted_without_formatting() {
    let csv = ",North,South\nP,1,1";
    fn labels(s: &Session, id: NodeId) -> Vec<(String, Paint)> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && ["North", "South"].contains(&t.plain_text().as_str())
            {
                out.push((t.plain_text(), t.first_style().fill.clone()));
            }
        });
        out
    }
    let both = |c: Color| vec![("North".to_string(), Paint::solid(c)), ("South".to_string(), Paint::solid(c))];
    for (conflict, expected) in [("merge", Color::WHITE), ("add", Color::BLACK)] {
        let mut s = session();
        let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
        s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
        s.execute("swatch.new", &json!({"name": "Ink", "color": "#ffffff", "global": true})).unwrap();
        let mut wedges = vec![];
        node(&s, g).walk(&mut |m| {
            if let NodeKind::Path { .. } = m.kind {
                wedges.push(m.id.0);
            }
        });
        s.execute("paint.setFill", &json!({"ids": wedges, "swatch": "Ink"})).unwrap();
        s.execute("graph.setData", &json!({"id": g.0, "csv": csv})).unwrap();
        assert_eq!(labels(&s, g), both(Color::BLACK), "light wedges have black labels");
        copy(&mut s, &[g]);
        new_doc(&mut s);
        s.execute("swatch.new", &json!({"name": "Ink", "color": "#000000", "global": true})).unwrap();
        let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({"swatchConflict": conflict})).unwrap())[0];
        let pasted = labels(&s, p);
        assert_eq!(pasted, both(expected), "{conflict}");
        s.execute("graph.setData", &json!({"id": p.0, "csv": csv})).unwrap();
        assert_eq!(pasted, labels(&s, p), "{conflict}: as a redraw sets them");
    }
}

/// An object linked to a graphic style and to a swatch merged with the document's keeps its graphic style link when
/// pasted without formatting, and its color shows the document's swatch.
#[test]
fn a_merged_swatch_keeps_graphic_style_links_when_pasted_without_formatting() {
    let mut s = session();
    let r = swatched_rect(&mut s, "Ink", "#ffffff");
    sel(&mut s, &[r]);
    s.execute("graphicStyle.new", &json!({"name": "Sun"})).unwrap();
    copy(&mut s, &[r]);
    new_doc(&mut s);
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#000000", "global": true})).unwrap();
    let v = s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap();
    assert_eq!(v["merged"], 1);
    let p = ids_of(&v)[0];
    let style = node(&s, p).extras().graphic_style.and_then(|id| doc(&s).graphic_style_by_id(id)).map(|g| g.name.clone());
    assert_eq!(style.as_deref(), Some("Sun"));
    assert_eq!(fill(&s, p), linked(hex("#000000"), "Ink", 1.0));
}

/// When one pie's wedge of a series is repainted and another pie's wedge of that series is not, each pasted legend label
/// takes the contrast of the wedge it is in.
#[test]
fn wedges_of_one_series_painted_apart_keep_their_label_colors_when_pasted_without_formatting() {
    let mut s = session();
    fn north(s: &Session, id: NodeId) -> Vec<Paint> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && t.plain_text() == "North"
            {
                out.push(t.first_style().fill.clone());
            }
        });
        out
    }
    // Two pies side by side, and two stacked pies of one series (a disk on a larger one, and on one nearly its size).
    for (csv, position) in [(",North,South\nP,1,1\nQ,1,1", "even"), (",North\nP,1\nQ,3", "stacked"), (",North\nP,1000\nQ,999", "stacked")] {
        // The second time, the repainted wedge is brought to the front of its series first.
        let mut pasted = vec![];
        for reorder in [false, true] {
            pasted.push(paste_painted_apart(&mut s, csv, position, reorder, north));
        }
        assert_eq!(pasted[0], pasted[1], "{csv}");
    }
    // A series name of nine lines, from line breaks a quoted CSV cell keeps, on two stacked pies: each label is centered
    // on its own ring, as a redraw centers it.
    fn tall(s: &Session, id: NodeId) -> Vec<Paint> {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && t.plain_text() == ["N"; 9].join("\n")
            {
                out.push(t.first_style().fill.clone());
            }
        });
        out
    }
    let csv = format!(",\"{}\"\nP,1\nQ,3", ["N"; 9].join("\n"));
    paste_painted_apart(&mut s, &csv, "stacked", false, tall);
}

/// Paste without Formatting a pie graph made from `csv` with pies at `position` and Legends in Wedges, after its first
/// wedge of the first series takes the other contrast and, with `reorder`, comes to the front of its series. Returns
/// what `north` reads from the pasted graph, after checking the two values differ.
fn paste_painted_apart(s: &mut Session, csv: &str, position: &str, reorder: bool, north: fn(&Session, NodeId) -> Vec<Paint>) -> Vec<Paint> {
    new_doc(s);
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 400, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges", "piePosition": position})).unwrap();
    let drawn = north(s, g);
    assert_eq!(drawn.len(), 2);
    assert_eq!(drawn[0], drawn[1]);
    // Repaint the first North wedge with the contrast its label shows, so it takes the other contrast.
    let mut first = None;
    for c in node(s, g).children().unwrap().iter() {
        if c.extras().series_index == Some(0) && first.is_none() {
            c.walk(&mut |m| {
                if first.is_none() && matches!(m.kind, NodeKind::Path { .. }) {
                    first = Some(m.id);
                }
            });
        }
    }
    let first = first.unwrap();
    let color = if drawn[0] == Paint::solid(Color::WHITE) { "#ffffff" } else { "#000000" };
    s.execute("paint.setFill", &json!({"ids": [first.0], "color": color})).unwrap();
    if reorder {
        s.execute("object.arrange.bringToFront", &json!({"ids": [first.0]})).unwrap();
    }
    copy(s, &[g]);
    new_doc(s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    let labels = north(s, p);
    assert_ne!(labels[0], labels[1], "drawn as {drawn:?}, {position}, reorder {reorder}");
    labels
}

/// Legend labels of series that share a name, brought into another order, each keep the contrast of the wedge they are
/// in when the pie is pasted without formatting.
#[test]
fn reordered_shared_name_labels_keep_their_colors_when_pasted_without_formatting() {
    let mut s = session();
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": ",X,X\nP,1,1"})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    // Each label's fill, by its place from left to right and top to bottom.
    fn labels(s: &Session, id: NodeId) -> (Vec<NodeId>, Vec<Paint>) {
        let mut out = vec![];
        node(s, id).walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && t.plain_text() == "X"
            {
                let o = t.xf.translation();
                out.push((o.x, o.y, n.id, t.first_style().fill.clone()));
            }
        });
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        out.into_iter().map(|(_, _, id, f)| (id, f)).unzip()
    }
    let (ids, drawn) = labels(&s, g);
    assert_eq!(drawn.len(), 2);
    assert_ne!(drawn[0], drawn[1]);
    let first = node(&s, g).children().unwrap().iter().find(|c| c.name.as_deref() == Some("Legend")).unwrap().children().unwrap()[0].id;
    assert!(ids.contains(&first));
    s.execute("object.arrange.bringToFront", &json!({"ids": [first.0]})).unwrap();
    copy(&mut s, &[g]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    assert_eq!(labels(&s, p).1, drawn);
}

/// A wedge's hidden fill does not set its legend label's color: the fill it shows does.
#[test]
fn a_hidden_wedge_fill_does_not_color_its_label() {
    let mut s = session();
    let csv = ",North,South\nP,1,1";
    let g = id_of(&s.execute("graph.create", &json!({"type": "pie", "x": 50, "y": 50, "width": 200, "height": 200, "csv": csv})).unwrap());
    s.execute("graph.setType", &json!({"id": g.0, "pieLegend": "wedges"})).unwrap();
    let mut n = node(&s, g).clone();
    // A hidden black fill above the light South wedge's fill.
    for c in n.children_mut().unwrap().iter_mut().filter(|c| c.extras().series_index == Some(1)) {
        for w in std::sync::Arc::make_mut(c).children_mut().unwrap().iter_mut() {
            let w = std::sync::Arc::make_mut(w);
            if let Some(vectorcraft_doc::AppearanceItem::Fill(f)) = w.appearance.items.iter().find(|it| it.is_fill()).cloned() {
                let hidden = vectorcraft_doc::FillLayer { paint: Paint::solid(Color::BLACK), visible: false, ..f };
                w.appearance.items.push(vectorcraft_doc::AppearanceItem::Fill(hidden));
            }
        }
    }
    crate::cmd::graph::plain_labels(&mut n, &|_| {});
    let mut south = None;
    n.walk(&mut |m| {
        if let NodeKind::Text(t) = &m.kind
            && t.plain_text() == "South"
        {
            south = Some(t.first_style().fill.clone());
        }
    });
    assert_eq!(south, Some(Paint::solid(Color::BLACK)));
}

/// A graph inside a group, and a graph inside a graph, get plain labels too.
#[test]
fn graphs_in_groups_pasted_without_formatting_get_plain_labels() {
    let mut s = session();
    let csv = ",2024\nQ1,3\nQ2,5";
    let g = id_of(&s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 120, "height": 80, "csv": csv})).unwrap());
    let fresh = graph_label_looks(&s, g);
    sel(&mut s, &[g]);
    s.execute("text.setStyle", &json!({"size": 14})).unwrap();
    let r = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    sel(&mut s, &[g, r]);
    let group = id_of(&s.execute("object.group", &json!({})).unwrap());
    // A copy of the graph placed inside it, as a file may hold one.
    let inner = {
        let d = doc_mut(&mut s);
        let mut copy = d.node(g).unwrap().clone();
        copy = d.reid(&copy);
        let id = copy.id;
        d.node_mut(g).unwrap().children_mut().unwrap().push(std::sync::Arc::new(copy));
        id
    };
    copy(&mut s, &[group]);
    new_doc(&mut s);
    let p = ids_of(&s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap())[0];
    let mut graphs = vec![];
    node(&s, p).walk(&mut |n| {
        if let Some(spec) = n.extras().graph.as_deref() {
            graphs.push((n.id, spec.label_styles.clone()));
        }
    });
    assert_eq!(graphs.len(), 2, "{inner:?}");
    for (id, l) in graphs {
        assert_eq!((l.values, l.categories, l.legend, l.values_para, l.categories_para, l.legend_para), (None, None, None, None, None, None));
        let looks = graph_label_looks(&s, id);
        assert!(looks.iter().all(|(_, size, _)| *size == fresh[0].1), "{looks:?}");
    }
}

/// A graph without its generated bounds (as a file may hold it) is drawn again where Paste without Formatting put it.
#[test]
fn a_graph_without_generated_bounds_pasted_without_formatting_follows_the_offset() {
    let mut s = session();
    let csv = ",2024\nQ1,3\nQ2,5";
    let g = id_of(&s.execute("graph.create", &json!({"type": "bar", "x": 50, "y": 60, "width": 100, "height": 80, "csv": csv})).unwrap());
    let plot = {
        let mut extras = doc_mut(&mut s).node_mut(g).unwrap().extras_mut();
        let spec = extras.graph.as_mut().unwrap();
        spec.placed = None;
        spec.rect
    };
    copy(&mut s, &[g]);
    let v = s.execute("edit.pasteWithoutFormatting", &json!({"dx": 30, "dy": -5})).unwrap();
    let p = ids_of(&v)[0];
    s.execute("graph.setData", &json!({"id": p.0, "csv": csv})).unwrap();
    let r = node(&s, p).extras().graph.as_deref().unwrap().rect;
    let off = [r.x0 - plot.x0 - 30.0, r.y0 - plot.y0 + 5.0, r.x1 - plot.x1 - 30.0, r.y1 - plot.y1 + 5.0];
    assert!(off.iter().all(|d| d.abs() < 1e-6), "{plot:?} → {r:?}");

    // A graph saved with the bounds of all its art but not those of its art without type, drawn with 30 pt labels: the
    // labels set at 9 pt do not read as a move or scale when it is drawn again.
    let g = id_of(&s.execute("graph.create", &json!({"type": "bar", "x": 300, "y": 60, "width": 100, "height": 80, "csv": csv})).unwrap());
    sel(&mut s, &[g]);
    s.execute("text.setStyle", &json!({"size": 30})).unwrap();
    s.execute("graph.setData", &json!({"id": g.0, "csv": csv})).unwrap();
    let plot = {
        let mut extras = doc_mut(&mut s).node_mut(g).unwrap().extras_mut();
        let spec = extras.graph.as_mut().unwrap();
        spec.placed_art = None;
        spec.rect
    };
    copy(&mut s, &[g]);
    let v = s.execute("edit.pasteWithoutFormatting", &json!({"dx": 30, "dy": -5})).unwrap();
    let p = ids_of(&v)[0];
    s.execute("graph.setData", &json!({"id": p.0, "csv": csv})).unwrap();
    let r = node(&s, p).extras().graph.as_deref().unwrap().rect;
    let off = [r.x0 - plot.x0 - 30.0, r.y0 - plot.y0 + 5.0, r.x1 - plot.x1 - 30.0, r.y1 - plot.y1 + 5.0];
    assert!(off.iter().all(|d| d.abs() < 1e-6), "without the bounds of the art without type: {plot:?} → {r:?}");
}

#[test]
fn pasting_again_reuses_what_an_earlier_paste_added() {
    let mut s = session();
    let r = swatched_rect(&mut s, "Brand", "#ff0000");
    // A character style whose fill links to Brand.
    let t = id_of(&s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hi"})).unwrap());
    let red = serde_json::to_value(linked(hex("#ff0000"), "Brand", 1.0)).unwrap();
    s.execute("charStyle.new", &json!({"name": "Loud", "attrs": {"fill": red}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Loud", "id": t.0})).unwrap();
    copy(&mut s, &[r, t]);
    new_doc(&mut s);
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#0000ff", "global": true})).unwrap();
    let v = s.execute("edit.paste", &json!({"swatchConflict": "add"})).unwrap();
    assert_eq!(v["added"], 2, "Brand 2 and Loud: {v}");
    let style_fill = |s: &Session| serde_json::from_value::<Paint>(doc(s).char_styles[0].attrs["fill"].clone()).unwrap();
    assert_eq!(style_fill(&s), linked(hex("#ff0000"), "Brand 2", 1.0), "the style's colour follows the renamed swatch");
    // Again: Brand 2 and Loud are the pasted ones, nothing piles up.
    let v = s.execute("edit.paste", &json!({"swatchConflict": "add"})).unwrap();
    assert_eq!(v["added"], 0, "{v}");
    assert_eq!(renamed(&v), [("swatch".to_string(), "Brand".to_string(), "Brand 2".to_string())]);
    assert_eq!(fill(&s, ids_of(&v)[0]), linked(hex("#ff0000"), "Brand 2", 1.0));
    assert!(doc(&s).swatch("Brand 3").is_none() && doc(&s).char_styles.len() == 1);
}

/// The active document, to edit its definitions directly.
fn doc_mut(s: &mut Session) -> &mut Document {
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc)
}

/// The pattern a paint paints with.
fn pattern_of(p: &Paint) -> Option<&str> {
    match p {
        Paint::Pattern { pattern, .. } => Some(pattern),
        _ => None,
    }
}

/// The pattern the fill of the active document's character style `name` paints with.
fn style_pattern(s: &Session, name: &str) -> Option<String> {
    let attrs = &doc(s).char_styles.iter().find(|x| x.name == name)?.attrs;
    pattern_of(&serde_json::from_value::<Paint>(attrs.get("fill")?.clone()).ok()?).map(str::to_string)
}

/// A pattern "Dots" made from a `size` square of `color` in the active document.
fn dots(s: &mut Session, size: f64, color: &str) {
    let square = rect(s, 300.0, 200.0, size, size);
    s.execute("paint.setFill", &json!({"ids": [square.0], "color": color})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Dots", "edit": false, "ids": [square.0]})).unwrap();
}

/// Type "Hi" filled with the pattern "Dots" (a small red square), set in a character style
/// "Patterned" whose fill is that pattern.
fn patterned_type(s: &mut Session) -> NodeId {
    dots(s, 10.0, "#ff0000");
    let t = id_of(&s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hi"})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [t.0], "swatch": "Dots"})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Patterned", "id": t.0})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Patterned", "id": t.0})).unwrap();
    t
}

/// The character style name of a type object's first run, and the pattern that run paints with.
fn type_links(s: &Session, id: NodeId) -> (Option<String>, Option<String>) {
    let NodeKind::Text(tx) = node(s, id).kind else { panic!("not type") };
    (tx.runs[0].style.style_name.clone(), pattern_of(&tx.runs[0].style.fill).map(str::to_string))
}

/// Type set in a style that paints with a pattern, pasted into a document with another pattern of
/// that name: the pattern comes in as "Dots 2", and the style paints with "Dots 2" too, so applying
/// it later paints with the pasted pattern. Pasting again adds nothing.
#[test]
fn pasted_styles_paint_with_the_renamed_pattern_they_use() {
    let mut s = session();
    let t = patterned_type(&mut s);
    copy(&mut s, &[t]);
    new_doc(&mut s);
    dots(&mut s, 30.0, "#0000ff");
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert!(doc(&s).pattern("Dots 2").is_some(), "{v}");
    assert_eq!(style_pattern(&s, "Patterned").as_deref(), Some("Dots 2"), "{v}");
    let named = |n: &str| Some(n.to_string());
    assert_eq!(type_links(&s, ids_of(&v)[0]), (named("Patterned"), named("Dots 2")));
    let other = id_of(&s.execute("text.create", &json!({"x": 10, "y": 150, "text": "Yo"})).unwrap());
    s.execute("charStyle.apply", &json!({"name": "Patterned", "id": other.0})).unwrap();
    assert_eq!(type_links(&s, other).1.as_deref(), Some("Dots 2"), "the applied style paints with the pasted pattern");
    let counts = |s: &Session| (doc(s).patterns.len(), doc(s).char_styles.len());
    let before = counts(&s);
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(v["added"], 0, "{v}");
    assert_eq!(counts(&s), before);
    assert_eq!(type_links(&s, ids_of(&v)[0]), (named("Patterned"), named("Dots 2")));
}

/// A document whose "Patterned" style has the pasted style's attributes but whose "Dots" is
/// another pattern: that style paints with the document's "Dots", so the pasted one comes in as
/// "Patterned 2", painting with "Dots 2", and the document's own is left as it was. Pasting again
/// uses the numbered copies.
#[test]
fn pasted_styles_are_not_matched_to_styles_that_paint_with_another_pattern() {
    let mut s = session();
    let t = patterned_type(&mut s);
    let style = doc(&s).char_styles.iter().find(|x| x.name == "Patterned").unwrap().clone();
    copy(&mut s, &[t]);
    new_doc(&mut s);
    dots(&mut s, 30.0, "#0000ff");
    doc_mut(&mut s).char_styles.push(style);
    let own = (doc(&s).char_styles.clone(), doc(&s).pattern("Dots").cloned());
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(style_pattern(&s, "Patterned 2").as_deref(), Some("Dots 2"), "{v}");
    let d = doc(&s);
    assert_eq!((&d.char_styles[..own.0.len()], d.pattern("Dots")), (&own.0[..], own.1.as_ref()), "the document's own are unchanged");
    let named = |n: &str| Some(n.to_string());
    let links = (named("Patterned 2"), named("Dots 2"));
    assert_eq!(type_links(&s, ids_of(&v)[0]), links);
    let counts = |s: &Session| (doc(s).patterns.len(), doc(s).char_styles.len());
    let before = counts(&s);
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(v["added"], 0, "{v}");
    assert_eq!(counts(&s), before);
    assert_eq!(type_links(&s, ids_of(&v)[0]), links);
}

/// A style that paints with a pattern whose tile sets type in that same style is pasted with the
/// pattern: when the pattern comes in as "Dots 2", the style paints with "Dots 2", and the tile's
/// type is set in the style. Pasting again adds nothing. The commands do not make that loop, so the
/// source is a document edited directly.
#[test]
fn a_pasted_style_and_the_pattern_whose_tile_uses_it_come_in_together() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Tile"})).unwrap());
    s.execute("charStyle.new", &json!({"name": "Accent", "attrs": {"kerning": 40}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Accent", "id": t.0})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Dots", "edit": false, "ids": [t.0]})).unwrap();
    let other = id_of(&s.execute("text.create", &json!({"x": 10, "y": 90, "text": "B"})).unwrap());
    s.execute("charStyle.apply", &json!({"name": "Accent", "id": other.0})).unwrap();
    s.execute("paint.setFill", &json!({"swatch": "Dots", "ids": [other.0]})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Painted", "id": other.0})).unwrap();
    let d = doc_mut(&mut s);
    let painted = d.char_styles.iter().find(|x| x.name == "Painted").unwrap().attrs.clone();
    d.char_styles.retain(|x| x.name != "Painted");
    d.char_styles.iter_mut().find(|x| x.name == "Accent").unwrap().attrs = painted;
    assert_eq!(style_pattern(&s, "Accent").as_deref(), Some("Dots"));
    copy(&mut s, &[other]);
    new_doc(&mut s);
    dots(&mut s, 30.0, "#0000ff");
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert!(doc(&s).pattern("Dots 2").is_some(), "{v}");
    assert_eq!(style_pattern(&s, "Accent").as_deref(), Some("Dots 2"), "{v}");
    let NodeKind::Text(tile) = &doc(&s).pattern("Dots 2").unwrap().art[0].kind else { panic!("the tile is type") };
    assert_eq!(tile.runs[0].style.style_name.as_deref(), Some("Accent"));
    let after = doc(&s).clone();
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(v["added"], 0, "{v}");
    assert!(doc(&s).patterns == after.patterns && doc(&s).char_styles == after.char_styles, "nothing is added");
    // Back in the source document, the loop is the document's own.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!((v["added"].as_u64(), v["renamed"].as_array().map(Vec::len)), (Some(0), Some(0)), "{v}");
}

/// A symbol whose art paints with a pattern is not matched to the document's symbol of that name
/// and art when the document's pattern of that name is another one: it comes in as "Mark 2",
/// painting with "Dots 2".
#[test]
fn pasted_symbols_are_not_matched_to_symbols_that_paint_with_another_pattern() {
    let mut s = session();
    dots(&mut s, 10.0, "#ff0000");
    let r = rect(&mut s, 20.0, 20.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": "Dots"})).unwrap();
    sel(&mut s, &[r]);
    let mark = id_of(&s.execute("symbol.new", &json!({"name": "Mark"})).unwrap());
    copy(&mut s, &[mark]);
    // The document gets "Mark" and "Dots" from a first paste; then its "Dots" gets another tile.
    new_doc(&mut s);
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(v["renamed"], json!([]), "{v}");
    let tile = &mut doc_mut(&mut s).patterns.iter_mut().find(|p| p.name == "Dots").unwrap().tile;
    *tile = Rect::new(tile.x0, tile.y0, tile.x1 + 30.0, tile.y1 + 30.0);
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert!(renamed(&v).contains(&("symbol".to_string(), "Mark".to_string(), "Mark 2".to_string())), "{v}");
    let art = |s: &Session, name: &str| fill_of(&doc(s).symbols.iter().find(|x| x.name == name).unwrap().art);
    assert_eq!((art(&s, "Mark").as_deref(), art(&s, "Mark 2").as_deref()), (Some("Dots"), Some("Dots 2")));
    assert!(matches!(node(&s, ids_of(&v)[0]).kind, NodeKind::SymbolInstance { ref symbol, .. } if symbol == "Mark 2"));
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(v["added"], 0, "{v}");
}

/// The pattern the first filled object in `n` paints with.
fn fill_of(n: &Node) -> Option<String> {
    let mut found = None;
    n.walk(&mut |m| {
        if found.is_none() {
            found = pattern_of(&m.appearance.fill_paint()).map(str::to_string);
        }
    });
    found
}

/// A brush whose art sets type in a character style is not matched to the document's brush of
/// that name and art when the document's style of that name is another one: the style comes in as
/// "Accent 2", and the brush as a copy whose type is set in "Accent 2".
#[test]
fn pasted_brushes_are_not_matched_to_brushes_that_use_another_style() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Ink"})).unwrap());
    s.execute("charStyle.new", &json!({"name": "Accent", "attrs": {"kerning": 40}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Accent", "id": t.0})).unwrap();
    let name = s.execute("brush.new", &json!({"type": "art", "name": "Lettering", "ids": [t.0]})).unwrap()["name"].as_str().unwrap().to_string();
    let path = rect(&mut s, 200.0, 200.0, 40.0, 10.0);
    s.execute("brush.apply", &json!({"name": name, "ids": [path.0]})).unwrap();
    let brush = vectorcraft_brush::find(doc(&s), &name).unwrap();
    copy(&mut s, &[path]);
    new_doc(&mut s);
    s.execute("charStyle.new", &json!({"name": "Accent", "attrs": {"kerning": 10}})).unwrap();
    let d = doc_mut(&mut s);
    let mut lib = vectorcraft_brush::library(d);
    lib.push(brush);
    vectorcraft_brush::store(d, &lib);
    let v = s.execute("edit.paste", &json!({})).unwrap();
    let names = renamed(&v);
    let copied = names.iter().find(|(k, from, _)| k == "brush" && *from == name).map(|(_, _, to)| to.clone());
    assert!(names.contains(&("charStyle".to_string(), "Accent".to_string(), "Accent 2".to_string())), "{v}");
    let copied = copied.unwrap_or_else(|| panic!("the brush is added under a free name: {v}"));
    let style_of = |s: &Session, b: &str| {
        let b = vectorcraft_brush::find(doc(s), b).unwrap();
        let mut found = None;
        for a in b.art() {
            a.walk(&mut |m| {
                if let NodeKind::Text(tx) = &m.kind {
                    found = found.take().or_else(|| tx.runs[0].style.style_name.clone());
                }
            });
        }
        found
    };
    assert_eq!((style_of(&s, &name).as_deref(), style_of(&s, &copied).as_deref()), (Some("Accent"), Some("Accent 2")));
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(v["added"], 0, "{v}");
}

#[test]
fn brushes_travel_with_their_strokes() {
    let mut s = session();
    let name = s
        .execute("brush.new", &json!({"type": "calligraphic", "name": "Wide Nib", "params": {"angle": 30, "roundness": 20, "size": 9}}))
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_string();
    let r = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    s.execute("brush.apply", &json!({"name": name, "ids": [r.0]})).unwrap();
    copy(&mut s, &[r]);
    assert_eq!(s.clipboard.brushes.len(), 1);
    new_doc(&mut s);
    assert!(vectorcraft_brush::find(doc(&s), &name).is_none());
    s.execute("edit.paste", &json!({})).unwrap();
    assert!(vectorcraft_brush::find(doc(&s), &name).is_some());
}

#[test]
fn pasting_back_into_the_source_document_adds_nothing() {
    let mut s = session();
    let r = swatched_rect(&mut s, "Brand", "#ff0000");
    copy(&mut s, &[r]);
    // Redefining the swatch afterwards is no conflict in its own document.
    s.execute("swatch.edit", &json!({"name": "Brand", "color": "#00ff00"})).unwrap();
    assert_eq!(s.execute("clipboard.conflicts", &json!({})).unwrap()["swatches"], json!([]));
    let v = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!((v["added"].as_u64(), v["renamed"].as_array().map(Vec::len)), (Some(0), Some(0)));
    assert_eq!(fill(&s, ids_of(&v)[0]), linked(hex("#00ff00"), "Brand", 1.0), "it shows the swatch as it is now");
}

#[test]
fn paste_without_formatting_leaves_text_styles_behind() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hello"})).unwrap());
    s.execute("charStyle.new", &json!({"name": "Emphasis", "attrs": {"size": 24.0}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Emphasis", "id": t.0})).unwrap();
    copy(&mut s, &[t]);
    new_doc(&mut s);
    s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap();
    assert!(doc(&s).char_styles.is_empty());
    s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(doc(&s).char_styles.len(), 1);
}

#[test]
fn clipboard_svg_carries_its_own_patterns() {
    let mut s = session();
    let dot = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[dot]);
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    let r = rect(&mut s, 20.0, 20.0, 100.0, 80.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": "Dots"})).unwrap();
    copy(&mut s, &[r]);
    // Another document is active when the clipboard is published.
    new_doc(&mut s);
    let svg = s.clipboard_svg().unwrap();
    assert!(svg.contains("<pattern"), "{svg}");
}

#[test]
fn a_failed_batch_puts_the_clipboard_back() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    copy(&mut s, &[a]);
    let steps = json!([
        {"command": "select.set", "params": {"ids": [a.0, b.0]}},
        {"command": "edit.copy"},
        {"command": "no.such.command"},
    ]);
    assert!(s.execute("command.batch", &json!({"commands": steps})).is_err());
    assert_eq!(s.clipboard.nodes.len(), 1);
}

// ---------- M4.23: placement ----------

#[test]
fn paste_centres_on_the_given_point() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 40.0, 20.0);
    copy(&mut s, &[r]);
    let v = s.execute("edit.paste", &json!({"center": [300, 200]})).unwrap();
    let b = doc(&s).bounds_of(&ids_of(&v), false).unwrap();
    assert!((b.center() - Point::new(300.0, 200.0)).hypot() < 1e-9, "{b:?}");
    // Without a centre, the offset applies.
    let v = s.execute("edit.paste", &json!({"dx": 5, "dy": 7})).unwrap();
    assert_eq!(doc(&s).bounds_of(&ids_of(&v), false).unwrap().origin(), Point::new(5.0, 7.0));
}

#[test]
fn front_and_back_without_a_selection_use_the_current_layers_ends() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    copy(&mut s, &[a, b]);
    let layer = |s: &Session| doc(s).layers[0].children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>();
    s.execute("select.none", &json!({})).unwrap();
    let back = ids_of(&s.execute("edit.pasteInBack", &json!({})).unwrap());
    assert_eq!(&layer(&s)[..2], &back[..], "pasted at the bottom (index 0), in order");
    s.execute("select.none", &json!({})).unwrap();
    let front = ids_of(&s.execute("edit.pasteInFront", &json!({})).unwrap());
    let l = layer(&s);
    assert_eq!(&l[l.len() - 2..], &front[..], "pasted on top, in order");
    // With a selection: right above / below it.
    sel(&mut s, &[a]);
    let above = ids_of(&s.execute("edit.pasteInFront", &json!({})).unwrap());
    let l = layer(&s);
    let i = l.iter().position(|id| *id == a).unwrap();
    assert_eq!(&l[i + 1..i + 3], &above[..]);
    sel(&mut s, &[a]);
    let below = ids_of(&s.execute("edit.pasteInBack", &json!({})).unwrap());
    let l = layer(&s);
    let i = l.iter().position(|id| *id == a).unwrap();
    assert_eq!(&l[i - 2..i], &below[..]);
}

#[test]
fn front_and_back_beside_a_compound_paths_member_go_beside_the_compound_path() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    let c = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let members = |s: &Session| doc(s).node(c).unwrap().children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>();
    let before = members(&s);
    let layer = |s: &Session| doc(s).layers[0].children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>();
    // A compound path and a group, each pasted with one of the compound path's members selected.
    let other = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[other]);
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    for copied in [c, group] {
        copy(&mut s, &[copied]);
        sel(&mut s, &[before[0]]);
        let above = ids_of(&s.execute("edit.pasteInFront", &json!({})).unwrap());
        let l = layer(&s);
        let i = l.iter().position(|id| *id == c).unwrap();
        assert_eq!(&l[i + 1..i + 2], &above[..], "right above the compound path");
        sel(&mut s, &[before[1]]);
        let below = ids_of(&s.execute("edit.pasteInBack", &json!({})).unwrap());
        let l = layer(&s);
        let i = l.iter().position(|id| *id == c).unwrap();
        assert_eq!(&l[i - 1..i], &below[..], "right below the compound path");
        assert_eq!(members(&s), before, "the compound path keeps its own members");
    }
    // A path still goes in front of the member, inside the compound path.
    let loose = rect(&mut s, 60.0, 0.0, 10.0, 10.0);
    copy(&mut s, &[loose]);
    sel(&mut s, &[before[0]]);
    let path = ids_of(&s.execute("edit.pasteInFront", &json!({})).unwrap());
    assert_eq!(members(&s), [before[0], path[0], before[1]]);
}

#[test]
fn paste_in_back_of_a_clipping_path_keeps_it_the_clipping_path() {
    let mut s = session();
    let children = |s: &Session, g: NodeId| doc(s).node(g).unwrap().children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>();
    let loose = rect(&mut s, 200.0, 0.0, 10.0, 10.0);
    let other = rect(&mut s, 220.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[other]);
    let group = id_of(&s.execute("object.group", &json!({})).unwrap());
    // A path as the clipping path, and a compound path as the clipping path (a group pasted in back
    // of a member goes beside the compound path).
    let art = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let mask = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    sel(&mut s, &[art, mask]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    let art2 = rect(&mut s, 0.0, 120.0, 100.0, 100.0);
    let (a, b) = (rect(&mut s, 10.0, 130.0, 20.0, 20.0), rect(&mut s, 40.0, 130.0, 20.0, 20.0));
    sel(&mut s, &[a, b]);
    let c = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    sel(&mut s, &[art2, c]);
    let g2 = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    for (g, clipping, target, copies) in [(g, mask, mask, vec![loose, group]), (g2, c, a, vec![group])] {
        assert_eq!(children(&s, g)[0], clipping);
        for copied in copies {
            copy(&mut s, &[copied]);
            sel(&mut s, &[target]);
            let pasted = ids_of(&s.execute("edit.pasteInBack", &json!({})).unwrap());
            let ch = children(&s, g);
            assert_eq!(ch[0], clipping, "the clipping path stays the bottom child");
            assert_eq!(&ch[1..2], &pasted[..], "right above the clipping path");
        }
    }
}

#[test]
fn paste_on_all_artboards_keeps_the_offset_to_the_source_artboard() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let boards: Vec<Rect> = doc(&s).artboards.iter().map(|a| a.rect).collect();
    // On artboard 2, 10 pt right and 15 pt down from its corner.
    let r = rect(&mut s, boards[1].x0 + 10.0, boards[1].y0 + 15.0, 20.0, 20.0);
    copy(&mut s, &[r]);
    assert_eq!(s.clipboard.source_artboard, Some(boards[1]));
    let ids = ids_of(&s.execute("edit.pasteOnAllArtboards", &json!({})).unwrap());
    assert_eq!(ids.len(), 3);
    for (id, b) in ids.iter().zip(&boards) {
        assert_eq!(doc(&s).bounds_of(&[*id], false).unwrap().origin(), Point::new(b.x0 + 10.0, b.y0 + 15.0));
    }
}

/// Paste in Place, in Front and in Back onto the active artboard (#693): where the objects were on
/// the artboard they were copied from, on the one named; without it, where they were.
#[test]
fn in_place_pastes_go_onto_the_artboard_named() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let boards: Vec<Rect> = doc(&s).artboards.iter().map(|a| a.rect).collect();
    let r = rect(&mut s, boards[0].x0 + 10.0, boards[0].y0 + 15.0, 20.0, 20.0);
    copy(&mut s, &[r]);
    for cmd in ["edit.pasteInPlace", "edit.pasteInFront", "edit.pasteInBack"] {
        let ids = ids_of(&s.execute(cmd, &json!({"artboard": 2})).unwrap());
        assert_eq!(doc(&s).bounds_of(&ids, false).unwrap().origin(), Point::new(boards[2].x0 + 10.0, boards[2].y0 + 15.0), "{cmd}");
        let ids = ids_of(&s.execute(cmd, &json!({})).unwrap());
        assert_eq!(doc(&s).bounds_of(&ids, false).unwrap().origin(), Point::new(boards[0].x0 + 10.0, boards[0].y0 + 15.0), "{cmd} without one");
        // An artboard that isn't there, or a value that isn't an index: an error, and nothing pasted.
        let before = doc(&s).clone();
        for bad in [json!(9), json!(-1), json!(1.5), json!("1")] {
            assert!(s.execute(cmd, &json!({"artboard": bad})).is_err(), "{cmd} with artboard {bad}");
        }
        assert_eq!(doc(&s), &before, "{cmd}");
    }
}

/// Paste in Place, in Front and in Back onto several artboards (the Artboard tool's selection, which
/// the app passes as `artboards`): a copy on each, where the objects were on the artboard they were
/// copied from, the copies in artboard order and in their own paint order, all selected, in one undo
/// step.
#[test]
fn in_place_pastes_put_a_copy_on_each_artboard_named() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 4})).unwrap();
    let boards: Vec<Rect> = doc(&s).artboards.iter().map(|a| a.rect).collect();
    let a = rect(&mut s, boards[1].x0 + 10.0, boards[1].y0 + 15.0, 20.0, 20.0);
    let b = rect(&mut s, boards[1].x0 + 50.0, boards[1].y0 + 5.0, 10.0, 10.0);
    copy(&mut s, &[a, b]);
    for cmd in ["edit.pasteInPlace", "edit.pasteInFront", "edit.pasteInBack"] {
        let undo = s.doc().unwrap().history.undo.len();
        let before = doc(&s).clone();
        let ids = ids_of(&s.execute(cmd, &json!({"artboards": [3, 0]})).unwrap());
        assert_eq!(ids.len(), 4, "{cmd}: two objects on each of two artboards");
        for (pair, board) in ids.chunks(2).zip([boards[0], boards[3]]) {
            assert_eq!(doc(&s).bounds_of(&pair[..1], false).unwrap().origin(), Point::new(board.x0 + 10.0, board.y0 + 15.0), "{cmd}");
            assert_eq!(doc(&s).bounds_of(&pair[1..], false).unwrap().origin(), Point::new(board.x0 + 50.0, board.y0 + 5.0), "{cmd}");
        }
        let layer: Vec<NodeId> = doc(&s).layers[0].children().into_iter().flatten().map(|n| n.id).collect();
        let at: Vec<usize> = ids.iter().map(|id| layer.iter().position(|n| n == id).unwrap()).collect();
        assert!(at.windows(2).all(|w| w[0] < w[1]), "{cmd}: in paint order, artboard by artboard: {at:?}");
        assert_eq!(s.doc().unwrap().selection.in_paint_order(doc(&s)), ids, "{cmd}: the copies are selected");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{cmd}: one undo step");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(doc(&s), &before, "{cmd}: undo takes every copy away");
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(doc(&s).layers[0].children().into_iter().flatten().count(), layer.len(), "{cmd}: redo");
        // `artboard` wins over `artboards`.
        let ids = ids_of(&s.execute(cmd, &json!({"artboard": 2, "artboards": [0, 3]})).unwrap());
        assert_eq!(doc(&s).bounds_of(&ids, false).unwrap().origin(), Point::new(boards[2].x0 + 10.0, boards[2].y0 + 5.0), "{cmd}");
        // A list that is empty, names an artboard the document lacks or names one twice, holds a
        // value that is not a whole number, or isn't a list, and an `artboard` that is not a whole
        // number or not an artboard: an error naming the fault, and nothing pasted. Both keys are
        // checked when both are given.
        let (before, undo) = (doc(&s).clone(), s.doc().unwrap().history.undo.len());
        let (not_list, missing, not_whole) = (
            "`artboards` must be a list of artboard indices",
            "`artboards` names an artboard the document does not have",
            "`artboards` must contain whole numbers",
        );
        let cases = [
            (json!({"artboards": []}), not_list),
            (json!({"artboards": 2}), not_list),
            (json!({"artboards": "0,1"}), not_list),
            (json!({"artboards": [0, 4]}), missing),
            (json!({"artboards": [1, 1]}), "`artboards` names an artboard twice"),
            (json!({"artboards": [0, -1]}), not_whole),
            (json!({"artboards": [0, 1.5]}), not_whole),
            (json!({"artboards": [4, "1"]}), not_whole),
            (json!({"artboard": 1, "artboards": [7]}), missing),
            (json!({"artboard": 1, "artboards": ["1"]}), not_whole),
            (json!({"artboard": 4}), "`artboard` must be the index of an artboard of the document"),
            (json!({"artboard": -1}), "`artboard` must be a whole number"),
            (json!({"artboard": 1.5}), "`artboard` must be a whole number"),
            (json!({"artboard": "1"}), "`artboard` must be a whole number"),
            (json!({"artboard": null}), "`artboard` must be a whole number"),
            (json!({"artboard": "1", "artboards": [9]}), "`artboard` must be a whole number"),
        ];
        for (p, want) in cases {
            let e = s.execute(cmd, &p).unwrap_err().to_string();
            assert_eq!(e, format!("invalid parameters for `{cmd}`: {want}"), "{cmd} {p}");
        }
        assert_eq!((doc(&s), s.doc().unwrap().history.undo.len()), (&before, undo), "{cmd}");
    }
}

/// With the Artboard tool chosen and several artboards selected, Paste in Place, in Front and in
/// Back put a copy on each without naming them; the journal notes them, so a replay pastes on the
/// same artboards, and a recorded action leaves them out. With one selected, or another tool, the
/// objects go where they were.
#[test]
fn in_place_pastes_follow_the_artboard_tool_selection() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let boards: Vec<Rect> = doc(&s).artboards.iter().map(|a| a.rect).collect();
    let r = rect(&mut s, boards[0].x0 + 10.0, boards[0].y0 + 15.0, 20.0, 20.0);
    copy(&mut s, &[r]);
    let origins = |s: &Session, ids: &[NodeId]| ids.iter().map(|id| doc(s).bounds_of(&[*id], false).unwrap().origin()).collect::<Vec<_>>();
    let on = |b: &[usize]| b.iter().map(|i| Point::new(boards[*i].x0 + 10.0, boards[*i].y0 + 15.0)).collect::<Vec<_>>();
    s.select_tool("artboard", ViewInfo::default()).unwrap();
    for cmd in ["edit.pasteInPlace", "edit.pasteInFront", "edit.pasteInBack"] {
        s.set_tool_option("selected", &json!([2, 1]));
        let start = s.journal.len();
        let ids = ids_of(&s.execute(cmd, &json!({})).unwrap());
        assert_eq!(origins(&s, &ids), on(&[1, 2]), "{cmd}");
        assert_eq!(s.journal.since(start).last().map(|(_, p)| p["artboards"].clone()), Some(json!([1, 2])), "{cmd}: journaled");
        assert!(s.journal_for_action(start).iter().all(|(_, p)| p.get("artboards").is_none()), "{cmd}: not in an action");
        // A stale index in the tool's selection is passed over.
        s.set_tool_option("selected", &json!([0, 9]));
        let ids = ids_of(&s.execute(cmd, &json!({})).unwrap());
        assert_eq!(origins(&s, &ids), on(&[0]), "{cmd}: one left");
        s.set_tool_option("selected", &json!([1]));
        let ids = ids_of(&s.execute(cmd, &json!({})).unwrap());
        assert_eq!(origins(&s, &ids), on(&[0]), "{cmd}: one selected, where it was");
    }
    s.set_tool_option("selected", &json!([1, 2]));
    s.select_tool("selection", ViewInfo::default()).unwrap();
    let ids = ids_of(&s.execute("edit.pasteInPlace", &json!({})).unwrap());
    assert_eq!(origins(&s, &ids), on(&[0]), "another tool: where it was");
}

/// Art copied from no artboard onto several: it keeps its place on the first and its offset to the
/// first on the others. Guides alone go on each artboard named.
#[test]
fn in_place_pastes_onto_several_artboards_of_art_off_them_and_guides() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let boards: Vec<Rect> = doc(&s).artboards.iter().map(|a| a.rect).collect();
    let r = rect(&mut s, boards[0].x0 - 100.0, boards[0].y0 + 15.0, 20.0, 20.0);
    copy(&mut s, &[r]);
    assert_eq!(s.clipboard.source_artboard, None);
    let ids = ids_of(&s.execute("edit.pasteInPlace", &json!({"artboards": [1, 2]})).unwrap());
    let at: Vec<Point> = ids.iter().map(|id| doc(&s).bounds_of(&[*id], false).unwrap().origin()).collect();
    let first = Point::new(boards[0].x0 - 100.0, boards[0].y0 + 15.0);
    assert_eq!(at, [first, first + (boards[2].origin() - boards[1].origin())]);
    // Guides alone, copied from artboard 1: one on each artboard named, 30 pt into it.
    s.execute("guide.add", &json!({"vertical": true, "pos": boards[0].x0 + 30.0})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    let r = s.execute("edit.pasteInFront", &json!({"artboards": [1, 2]})).unwrap();
    assert_eq!(r["guides"], 2);
    let mut guides: Vec<f64> = doc(&s).ruler_guides().iter().map(|g| g.pos).collect();
    guides.sort_by(f64::total_cmp);
    assert_eq!(guides, [boards[0].x0 + 30.0, boards[1].x0 + 30.0, boards[2].x0 + 30.0]);
}

/// Artboards on the clipboard come back once, where they were, whichever artboards are named.
#[test]
fn in_place_pastes_of_copied_artboards_ignore_the_artboards_named() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    s.execute("artboard.copy", &json!({"index": 0, "art": true})).unwrap();
    let r = s.execute("edit.pasteInPlace", &json!({"artboards": [1, 2]})).unwrap();
    assert_eq!(r["artboards"], json!([3]));
    assert_eq!(ids_of(&r).len(), 1);
    assert_eq!(doc(&s).artboards[3].rect, doc(&s).artboards[0].rect);
}

/// Art copied from beside the artboards goes back where it was, whichever artboard is active (#1924).
#[test]
fn in_place_pastes_of_art_off_the_artboards_stay_where_it_was() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    let boards: Vec<Rect> = doc(&s).artboards.iter().map(|a| a.rect).collect();
    let r = rect(&mut s, boards[0].x0 - 100.0, boards[0].y0 + 15.0, 20.0, 20.0);
    copy(&mut s, &[r]);
    for cmd in ["edit.pasteInPlace", "edit.pasteInFront", "edit.pasteInBack"] {
        let ids = ids_of(&s.execute(cmd, &json!({"artboard": 1})).unwrap());
        assert_eq!(doc(&s).bounds_of(&ids, false).unwrap().origin(), Point::new(boards[0].x0 - 100.0, boards[0].y0 + 15.0), "{cmd}");
    }
}

/// Art centered on an artboard's right or bottom edge is copied from that artboard: Paste in Place
/// puts it on the active artboard where it was on that one.
#[test]
fn in_place_pastes_of_art_on_an_artboard_edge_go_onto_the_artboard_named() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    let boards: Vec<Rect> = doc(&s).artboards.iter().map(|a| a.rect).collect();
    let b = boards[1];
    // A line along the right edge of artboard 2, then one along its bottom edge.
    for (x1, y1, x2, y2) in [(b.x1, b.y0 + 10.0, b.x1, b.y0 + 40.0), (b.x0 + 10.0, b.y1, b.x0 + 40.0, b.y1)] {
        let line = id_of(&s.execute("shape.line", &json!({"x1": x1, "y1": y1, "x2": x2, "y2": y2})).unwrap());
        copy(&mut s, &[line]);
        assert_eq!(s.clipboard.source_artboard, Some(b));
        let ids = ids_of(&s.execute("edit.pasteInPlace", &json!({"artboard": 0})).unwrap());
        let moved = doc(&s).bounds_of(&ids, false).unwrap().origin() - doc(&s).bounds_of(&[line], false).unwrap().origin();
        assert_eq!(moved, boards[0].origin() - b.origin());
    }
}

#[test]
fn paste_remembers_layers_restores_the_source_layers() {
    let mut s = session();
    let first = doc(&s).layers[0].id;
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let art = id_of(&s.execute("layer.new", &json!({"name": "Art"})).unwrap());
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    copy(&mut s, &[a, b]);
    let names: Vec<_> = s.clipboard.source_layers.iter().map(|l| l.as_ref().map(|l| l.names.clone())).collect();
    assert_eq!(names, [Some(vec!["Layer 1".to_string()]), Some(vec!["Art".to_string()])]);
    let other = id_of(&s.execute("layer.new", &json!({"name": "Other"})).unwrap());
    assert_eq!(s.execute("layer.pasteRemembersLayers", &json!({})).unwrap()["on"], true);
    assert!(doc(&s).paste_remembers_layers);
    assert_eq!(s.execute("document.inspect", &json!({})).unwrap()["pasteRemembersLayers"], true);
    let ids = ids_of(&s.execute("edit.paste", &json!({})).unwrap());
    let parents: Vec<_> = ids.iter().map(|id| doc(&s).parent_of(*id)).collect();
    assert_eq!(parents, [Some(first), Some(art)]);
    // In a document without those layers they are made.
    new_doc(&mut s);
    s.execute("layer.pasteRemembersLayers", &json!({"on": true})).unwrap();
    let ids = ids_of(&s.execute("edit.paste", &json!({})).unwrap());
    let names: Vec<_> = ids.iter().map(|id| doc(&s).node(doc(&s).parent_of(*id).unwrap()).unwrap().name.clone().unwrap()).collect();
    assert_eq!(names, ["Layer 1", "Art"]);
    assert_eq!(doc(&s).layers.len(), 2, "Layer 1 exists; Art is new");
    // The option is an undoable document setting.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!doc(&s).paste_remembers_layers);
    // Off: everything goes to the current layer.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!(s.execute("layer.pasteRemembersLayers", &json!({"on": false})).unwrap()["on"], false);
    s.execute("layer.setCurrent", &json!({"id": other.0})).unwrap();
    let ids = ids_of(&s.execute("edit.paste", &json!({})).unwrap());
    assert!(ids.iter().all(|id| doc(&s).parent_of(*id) == Some(other)));
}

/// A sublayer `name` made inside `parent`; it becomes the current layer.
fn sublayer(s: &mut Session, parent: NodeId, name: &str) -> NodeId {
    id_of(&s.execute("layer.newSublayer", &json!({"parent": parent.0, "name": name})).unwrap())
}

/// The names of the layers around `id`, the top-level layer first.
fn layer_path(s: &Session, id: NodeId) -> Vec<String> {
    let d = doc(s);
    d.ancestry(id).unwrap().iter().filter_map(|a| d.node(*a).filter(|n| n.is_layer())).map(|l| l.name.clone().unwrap()).collect()
}

/// Art from two sublayers of the same name in different layers pastes back into each one; in
/// another document a missing sublayer is made inside its layer, and a missing layer on top with
/// the sublayer inside, in the paste's one undo step.
#[test]
fn paste_remembers_layers_keeps_same_named_sublayers_apart() {
    let mut s = session();
    let first = doc(&s).layers[0].id;
    let sub1 = sublayer(&mut s, first, "Sub");
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let second = id_of(&s.execute("layer.new", &json!({"name": "Layer 2", "top": true})).unwrap());
    let sub2 = sublayer(&mut s, second, "Sub");
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    copy(&mut s, &[a, b]);
    s.execute("layer.pasteRemembersLayers", &json!({"on": true})).unwrap();
    s.execute("layer.setCurrent", &json!({"id": first.0})).unwrap();
    let ids = ids_of(&s.execute("edit.paste", &json!({})).unwrap());
    let parents: Vec<_> = ids.iter().map(|id| doc(&s).parent_of(*id)).collect();
    assert_eq!(parents, [Some(sub1), Some(sub2)]);
    new_doc(&mut s);
    s.execute("layer.pasteRemembersLayers", &json!({"on": true})).unwrap();
    let before = doc(&s).clone();
    let ids = ids_of(&s.execute("edit.paste", &json!({})).unwrap());
    let paths: Vec<_> = ids.iter().map(|id| layer_path(&s, *id)).collect();
    assert_eq!(paths, [["Layer 1", "Sub"], ["Layer 2", "Sub"]]);
    assert_eq!(doc(&s).layers.len(), 2, "Layer 1 is kept; Layer 2 is new");
    assert_eq!(doc(&s).layers[0].children().unwrap().len(), 1, "Sub is made inside Layer 1");
    let after = doc(&s).clone();
    // Pasting again finds the layers made.
    let again = ids_of(&s.execute("edit.paste", &json!({})).unwrap());
    let parents: Vec<_> = again.iter().map(|id| doc(&s).parent_of(*id)).collect();
    assert_eq!(parents, ids.iter().map(|id| doc(&s).parent_of(*id)).collect::<Vec<_>>());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*doc(&s), after);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*doc(&s), before, "one undo step takes the art and the layers made for it");
}

/// Art whose sublayer was deleted after the copy pastes into a new sublayer of that name inside
/// the same layer, not into a sublayer of that name in another layer.
#[test]
fn paste_remembers_layers_makes_a_deleted_sublayer_again_inside_its_layer() {
    let mut s = session();
    let first = doc(&s).layers[0].id;
    let sub = sublayer(&mut s, first, "Sub");
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let second = id_of(&s.execute("layer.new", &json!({"name": "Layer 2", "top": true})).unwrap());
    let elsewhere = sublayer(&mut s, second, "Sub");
    copy(&mut s, &[a]);
    s.execute("layer.delete", &json!({"id": sub.0})).unwrap();
    s.execute("layer.pasteRemembersLayers", &json!({"on": true})).unwrap();
    let ids = ids_of(&s.execute("edit.paste", &json!({})).unwrap());
    let made = doc(&s).parent_of(ids[0]).unwrap();
    assert_ne!(made, elsewhere);
    assert_eq!(doc(&s).parent_of(made), Some(first));
    assert_eq!(layer_path(&s, ids[0]), ["Layer 1", "Sub"]);
}

/// Art whose sublayer was moved into another layer after the copy pastes into that sublayer where
/// it is now; no sublayer is made where it was.
#[test]
fn paste_remembers_layers_follows_a_moved_sublayer() {
    let mut s = session();
    let first = doc(&s).layers[0].id;
    let sub = sublayer(&mut s, first, "Sub");
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let second = id_of(&s.execute("layer.new", &json!({"name": "Layer 2", "top": true})).unwrap());
    copy(&mut s, &[a]);
    s.execute("layer.move", &json!({"id": sub.0, "target": second.0})).unwrap();
    s.execute("layer.pasteRemembersLayers", &json!({"on": true})).unwrap();
    let ids = ids_of(&s.execute("edit.paste", &json!({})).unwrap());
    assert_eq!(doc(&s).parent_of(ids[0]), Some(sub));
    assert_eq!(layer_path(&s, ids[0]), ["Layer 2", "Sub"]);
    assert!(node(&s, first).children().unwrap().is_empty(), "no Sub is made in Layer 1");
}

/// In another document with layers of the same names, art goes into the topmost chain of layers
/// that matches the most names, a shown and unlocked one before a hidden or locked one.
#[test]
fn paste_remembers_layers_takes_the_topmost_open_match_of_repeated_names() {
    let mut s = session();
    let first = doc(&s).layers[0].id;
    sublayer(&mut s, first, "Sub");
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    copy(&mut s, &[a]);
    new_doc(&mut s);
    s.execute("layer.pasteRemembersLayers", &json!({"on": true})).unwrap();
    let lower = doc(&s).layers[0].id;
    let lower_sub = sublayer(&mut s, lower, "Sub");
    s.execute("layer.new", &json!({"name": "Sub", "top": true})).unwrap();
    let upper = id_of(&s.execute("layer.new", &json!({"name": "Layer 1", "top": true})).unwrap());
    // The upper Layer 1 has no Sub, and a top-level Sub is not inside a Layer 1: the lower chain
    // matches both names.
    let ids = ids_of(&s.execute("edit.paste", &json!({})).unwrap());
    assert_eq!(doc(&s).parent_of(ids[0]), Some(lower_sub));
    let upper_sub = sublayer(&mut s, upper, "Sub");
    let ids = ids_of(&s.execute("edit.paste", &json!({})).unwrap());
    assert_eq!(doc(&s).parent_of(ids[0]), Some(upper_sub), "the topmost of two full matches");
    s.execute("layer.setProps", &json!({"id": upper_sub.0, "locked": true})).unwrap();
    let ids = ids_of(&s.execute("edit.paste", &json!({})).unwrap());
    assert_eq!(doc(&s).parent_of(ids[0]), Some(lower_sub), "an unlocked match before a locked one");
}

/// A remembered layer or sublayer that is hidden or locked, or inside a layer that is, refuses the
/// whole paste: the document, the selection and the undo history stay as they were.
#[test]
fn paste_remembers_layers_refuses_a_hidden_or_locked_layer() {
    let (locked, hidden) = (json!({"locked": true}), json!({"visible": false}));
    for (on_sub, props) in [(true, &locked), (true, &hidden), (false, &locked), (false, &hidden)] {
        let mut s = session();
        let first = doc(&s).layers[0].id;
        let sub = sublayer(&mut s, first, "Sub");
        let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
        let other = id_of(&s.execute("layer.new", &json!({"name": "Other", "top": true})).unwrap());
        let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
        copy(&mut s, &[b, a]);
        s.execute("layer.pasteRemembersLayers", &json!({"on": true})).unwrap();
        let mut p = props.clone();
        p["id"] = json!(if on_sub { sub.0 } else { first.0 });
        s.execute("layer.setProps", &p).unwrap();
        s.execute("layer.setCurrent", &json!({"id": other.0})).unwrap();
        let before = s.doc().unwrap().clone();
        assert!(s.execute("edit.paste", &json!({})).is_err(), "{p}");
        assert_eq!(s.doc().unwrap().doc, before.doc, "{p}");
        assert_eq!(s.doc().unwrap().selection, before.selection, "{p}");
        assert_eq!(s.doc().unwrap().history.undo.len(), before.history.undo.len(), "{p}");
    }
    // In another document, a locked layer of the copied layer's name is refused too.
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    copy(&mut s, &[a]);
    new_doc(&mut s);
    s.execute("layer.pasteRemembersLayers", &json!({"on": true})).unwrap();
    let locked = doc(&s).layers[0].id;
    id_of(&s.execute("layer.new", &json!({"name": "Flyer"})).unwrap());
    s.execute("layer.setProps", &json!({"id": locked.0, "locked": true})).unwrap();
    let before = doc(&s).clone();
    assert!(s.execute("edit.paste", &json!({})).is_err());
    assert_eq!(*doc(&s), before, "no second Layer 1 is made");
}

#[test]
fn paste_remembers_layers_survives_save_and_old_files_load_without_it() {
    let mut s = session();
    s.execute("layer.pasteRemembersLayers", &json!({"on": true})).unwrap();
    let bytes = vectorcraft_format::save(doc(&s), false);
    assert!(vectorcraft_format::load(&bytes).unwrap().paste_remembers_layers);
    let v: Value = serde_json::from_slice(&vectorcraft_format::save(&Document::new(10.0, 10.0), false)).unwrap();
    assert!(v.get("paste_remembers_layers").is_none(), "off is not written");
    assert!(!vectorcraft_format::load(v.to_string().as_bytes()).unwrap().paste_remembers_layers);
}

/// Verify document-scoped object IDs, grab-point placement and one destination Undo/Redo step.
#[test]
fn artwork_drop_uses_source_ids_hotspot_and_one_target_undo_step() {
    let mut s = session();
    let r = swatched_rect(&mut s, "Brand", "#ff0000");
    let source = s.doc().unwrap().clone();
    new_doc(&mut s);
    let other = swatched_rect(&mut s, "Brand", "#0000ff");
    assert_eq!(r, other, "node ids collide across documents");
    copy(&mut s, &[other]);
    let clip = s.clipboard.clone();
    let target = s.doc().unwrap().clone();
    let result = s.execute("edit.dropSelection", &json!({"sourceDocument": source.uid, "ids": [r, r], "grab": [25,30], "at": [175,130]})).unwrap();
    let ids = ids_of(&result);
    assert_eq!(ids.len(), 1);
    assert_eq!(fill(&s, ids[0]), linked(hex("#ff0000"), "Brand 2", 1.0));
    assert_eq!(fill(&s, other), linked(hex("#0000ff"), "Brand", 1.0));
    assert_eq!(node(&s, ids[0]).geometric_bounds().unwrap(), Rect::new(170.0, 120.0, 210.0, 160.0));
    assert_eq!(s.doc().unwrap().history.undo.len(), target.history.undo.len() + 1);
    assert_eq!(s.clipboard, clip);
    assert_eq!(s.documents()[0].doc, source.doc);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, target.doc);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, ids);
}

/// Verify malformed, oversized, stale and overflowing drop requests leave document and clipboard state intact.
#[test]
fn invalid_artwork_drop_is_atomic() {
    let mut s = session();
    let r = rect(&mut s, 10.0, 10.0, 40.0, 40.0);
    let uid = s.execute("document.inspect", &json!({})).unwrap()["uid"].as_u64().unwrap();
    new_doc(&mut s);
    let target = s.doc().unwrap().clone();
    let clipboard = s.clipboard.clone();
    for patch in [
        json!({"sourceDocument": u64::MAX}),
        json!({"sourceDocument": -1}),
        json!({"sourceDocument": 1.5}),
        json!({"sourceDocument": null}),
        json!({"ids": vec![r;10001]}),
        json!({"ids": null}),
        json!({"ids": [r,-1]}),
        json!({"ids": [r,1.5]}),
        json!({"grab": [0,0,0]}),
        json!({"grab": ["0",0]}),
        json!({"ids": []}),
        json!({"ids": [r, 9999999]}),
        json!({"ids": ["1"]}),
        json!({"grab": [0]}),
        json!({"at": [null, 0]}),
        json!({"at": [1e308, 0]}),
    ] {
        let mut p = json!({"sourceDocument": uid, "ids": [r], "grab": [0,0], "at": [50,60]});
        p.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        assert!(s.execute("edit.dropSelection", &p).is_err(), "{p}");
        assert_eq!(s.doc().unwrap().doc, target.doc);
        assert_eq!(s.doc().unwrap().history.undo.len(), target.history.undo.len());
        assert_eq!(s.doc().unwrap().selection, target.selection);
        assert_eq!(s.clipboard, clipboard);
    }
    s.execute("file.close", &json!({"index":0})).unwrap();
    assert!(s.execute("edit.dropSelection", &json!({"sourceDocument": uid,"ids":[r],"grab":[0,0],"at":[50,60]})).is_err());
    assert_eq!(s.doc().unwrap().doc, target.doc);
}

/// Verify refusing a locked insertion context preserves the target document and its undo history.
#[test]
fn artwork_drop_into_a_locked_destination_is_atomic() {
    let mut s = session();
    let r = swatched_rect(&mut s, "Brand", "#ff0000");
    let uid = s.doc().unwrap().uid;
    new_doc(&mut s);
    let layer = s.doc().unwrap().doc.layers[0].id;
    s.execute("layer.setProps", &json!({"id":layer,"locked":true})).unwrap();
    let target = s.doc().unwrap().clone();
    assert!(s.execute("edit.dropSelection", &json!({"sourceDocument":uid,"ids":[r],"grab":[0,0],"at":[100,100]})).is_err());
    assert_eq!(s.doc().unwrap().doc, target.doc);
    assert_eq!(s.doc().unwrap().selection, target.selection);
    assert_eq!(s.doc().unwrap().history.undo.len(), target.history.undo.len());
}

/// Verify tab drops use the editable current layer despite Paste Remembers Layers and source-layer name collisions.
#[test]
fn artwork_drop_uses_the_current_layer_even_when_paste_remembers_hidden_or_locked_source_layers() {
    for hidden in [true, false] {
        let mut s = session();
        let r = rect(&mut s, 10.0, 10.0, 40.0, 40.0);
        let uid = s.doc().unwrap().uid;
        new_doc(&mut s);
        let blocked = doc(&s).layers[0].id;
        s.execute("layer.setProps", &json!({"id":blocked,"visible":!hidden,"locked":!hidden})).unwrap();
        let current = id_of(&s.execute("layer.new", &json!({"name":"Flyer"})).unwrap());
        s.execute("layer.pasteRemembersLayers", &json!({"on":true})).unwrap();
        let blocked_before = node(&s, blocked);
        let placed = ids_of(&s.execute("edit.dropSelection", &json!({"sourceDocument":uid,"ids":[r],"grab":[0,0],"at":[100,100]})).unwrap());
        assert_eq!(doc(&s).parent_of(placed[0]), Some(current));
        assert_eq!(node(&s, blocked), blocked_before);
        assert_eq!(doc(&s).layers.len(), 2);
        assert!(doc(&s).paste_remembers_layers, "the preference still applies to ordinary Paste");
    }
}

/// Two touching rectangles made into 9-slice symbol `Audit`, its instance scaled 150% × 75% and
/// copied (#5902).
fn copied_nine_slice_instance(s: &mut Session) -> Document {
    let red = rect(s, 30.0, 30.0, 80.0, 60.0);
    s.execute("paint.setFill", &json!({"ids": [red.0], "color": "#ff0000"})).unwrap();
    let blue = rect(s, 110.0, 90.0, 40.0, 30.0);
    s.execute("paint.setFill", &json!({"ids": [blue.0], "color": "#0000ff"})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("symbol.new", &json!({"name": "Audit", "nineSlice": true, "registration": 0, "exportType": "graphic"})).unwrap();
    s.execute("object.scale", &json!({"sx": 150, "sy": 75})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    doc(s).clone()
}

/// The symbol options pasted symbol `to` should have: those of `from` in `src`.
fn assert_same_symbol_options(src: &Document, from: &str, d: &Document, to: &str) {
    assert!(src.symbol_nine_slice(from).is_some());
    assert_eq!(d.symbol_nine_slice(to), src.symbol_nine_slice(from));
    assert_eq!(d.symbol_registration(to), src.symbol_registration(from));
    assert_eq!(d.symbol_export_type(to), src.symbol_export_type(from));
    assert_eq!(d.symbol_natural_xf(to), src.symbol_natural_xf(from));
}

#[test]
fn a_nine_slice_symbol_pasted_into_another_document_keeps_its_options_and_look() {
    let mut s = session();
    let src = copied_nine_slice_instance(&mut s);
    let before = render(&s);
    new_doc(&mut s);
    s.execute("edit.pasteInPlace", &json!({})).unwrap();
    assert_same_symbol_options(&src, "Audit", doc(&s), "Audit");
    assert!(before == render(&s), "the pasted instance draws as in the source");
    // Pasting again uses the symbol the first paste added.
    let v = s.execute("edit.pasteInPlace", &json!({})).unwrap();
    assert_eq!(v["added"], 0, "{v}");
}

#[test]
fn a_renamed_nine_slice_symbol_keeps_its_options_under_the_new_name() {
    let mut s = session();
    let src = copied_nine_slice_instance(&mut s);
    new_doc(&mut s);
    let other = ellipse(&mut s);
    sel(&mut s, &[other]);
    s.execute("symbol.new", &json!({"name": "Audit"})).unwrap();
    let own = doc(&s).clone();
    let v = s.execute("edit.pasteInPlace", &json!({})).unwrap();
    assert!(renamed(&v).contains(&("symbol".into(), "Audit".into(), "Audit 2".into())), "{v}");
    assert_same_symbol_options(&src, "Audit", doc(&s), "Audit 2");
    // The document's own symbol keeps its options.
    assert_eq!(doc(&s).symbol_nine_slice("Audit"), None);
    assert_eq!(doc(&s).symbol_natural_xf("Audit"), own.symbol_natural_xf("Audit"));
}

#[test]
fn a_same_named_symbol_with_other_options_is_not_reused() {
    let mut s = session();
    let src = copied_nine_slice_instance(&mut s);
    new_doc(&mut s);
    s.execute("edit.pasteInPlace", &json!({})).unwrap();
    // The document's `Audit` has the same art but 9-slice scaling off now.
    s.execute("symbol.options", &json!({"name": "Audit", "nineSlice": false})).unwrap();
    let v = s.execute("edit.pasteInPlace", &json!({})).unwrap();
    assert!(renamed(&v).contains(&("symbol".into(), "Audit".into(), "Audit 2".into())), "{v}");
    assert_same_symbol_options(&src, "Audit", doc(&s), "Audit 2");
    assert_eq!(doc(&s).symbol_nine_slice("Audit"), None, "the document's own symbol keeps its options");
}

#[test]
fn a_symbol_without_a_recorded_natural_size_is_pasted_without_one() {
    let mut s = session();
    let r = rect(&mut s, 30.0, 30.0, 80.0, 60.0);
    sel(&mut s, &[r]);
    s.execute("symbol.new", &json!({"name": "Plain"})).unwrap();
    s.edit("Forget size", |d, _| {
        if let Some(m) = d.unknown.get_mut(vectorcraft_doc::SYMBOL_SIZES).and_then(Value::as_object_mut) {
            m.remove("Plain");
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(doc(&s).unknown.get(vectorcraft_doc::SYMBOL_SIZES).and_then(|m| m.get("Plain")), None);
    s.execute("edit.copy", &json!({})).unwrap();
    let before = render(&s);
    new_doc(&mut s);
    s.execute("edit.pasteInPlace", &json!({})).unwrap();
    assert!(doc(&s).symbols.iter().any(|x| x.name == "Plain"));
    assert_eq!(doc(&s).unknown.get(vectorcraft_doc::SYMBOL_SIZES).and_then(|m| m.get("Plain")), None, "no size is made up for it");
    assert!(before == render(&s), "the pasted instance draws as in the source");
}
