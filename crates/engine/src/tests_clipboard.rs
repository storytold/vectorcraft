//! Copy and paste between documents (resources, swatch conflicts) and paste placement (M4.22,
//! M4.23).

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::{Point, Rect};

use super::*;

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
        // An artboard that isn't there: where they were.
        let ids = ids_of(&s.execute(cmd, &json!({"artboard": 9})).unwrap());
        assert_eq!(doc(&s).bounds_of(&ids, false).unwrap().origin(), Point::new(boards[0].x0 + 10.0, boards[0].y0 + 15.0), "{cmd} on no artboard");
    }
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
    assert_eq!(s.clipboard.source_layers, [Some("Layer 1".to_string()), Some("Art".to_string())]);
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
