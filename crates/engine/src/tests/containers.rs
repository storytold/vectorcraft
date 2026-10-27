//! Group, layer, type and image appearance through commands: their own fills, strokes and effects,
//! the Contents (Characters) slot, `target: "contents"`, Target Contents and effects on layers.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Node, NodeKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A selected group of two 40×40 squares at (20, 20) and (100, 20) → (group, [members]).
fn group(s: &mut Session) -> (NodeId, [NodeId; 2]) {
    let a = id_of(&run(s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})));
    let b = id_of(&run(s, "shape.rectangle", json!({"x": 100, "y": 20, "width": 40, "height": 40})));
    run(s, "select.set", json!({"ids": [a.0, b.0]}));
    run(s, "paint.setFill", json!({"color": "#ff0000"}));
    (id_of(&run(s, "object.group", json!({}))), [a, b])
}

fn pixel(s: &Session, x: u32, y: u32) -> [u8; 4] {
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts).pixel(x, y)
}

const GREEN: [u8; 4] = [0, 255, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];

#[test]
fn a_group_fill_paints_both_members_and_the_contents_slot_moves() {
    let mut s = session();
    let (g, [a, _]) = group(&mut s);
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "paint.setFill", json!({"color": "#00ff00", "item": 0}));
    let n = node(&s, g);
    assert_eq!((n.appearance.items.len(), n.appearance.contents_at()), (1, 0));
    // The members keep their own fill; the group's paints over both of them.
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(Color::from_hex("#ff0000").unwrap()));
    assert_eq!((pixel(&s, 40, 40), pixel(&s, 120, 40), pixel(&s, 80, 40)), (GREEN, GREEN, [255; 4]));
    // Contents above the fill: the members show again; one undo step brings it back.
    assert_eq!(run(&mut s, "appearance.moveItem", json!({"from": "contents", "to": 1}))["contents"], 1);
    assert_eq!((node(&s, g).appearance.contents_index, pixel(&s, 40, 40)), (Some(1), RED));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(pixel(&s, 40, 40), GREEN);
    // A stroke dragged under the contents in one step with `contents`.
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "appearance.moveItem", json!({"from": 1, "to": 0, "contents": 1}));
    let ap = node(&s, g).appearance;
    assert!(!ap.items[0].is_fill() && ap.contents_at() == 1, "{ap:?}");
    // Paths have no Contents row.
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    assert!(s.execute("appearance.moveItem", &json!({"from": "contents", "to": 1})).is_err());
}

#[test]
fn appearance_commands_target_the_object_or_its_contents() {
    let mut s = session();
    let (g, [a, b]) = group(&mut s);
    let before = node(&s, a).appearance.items.len();
    run(&mut s, "appearance.addStroke", json!({"target": "contents"}));
    assert_eq!((node(&s, a).appearance.items.len(), node(&s, b).appearance.items.len()), (before + 1, before + 1));
    assert!(node(&s, g).appearance.items.is_empty());
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "target": "contents"}));
    assert_eq!((node(&s, a).appearance.effects.len(), node(&s, g).appearance.effects.len()), (1, 0));
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow"}));
    assert_eq!(node(&s, g).appearance.effects.len(), 1);
    assert!(s.execute("appearance.addFill", &json!({"target": "inside"})).is_err());
    // Target Contents selects the members (what double-clicking the Contents row does).
    assert_eq!(run(&mut s, "appearance.targetContents", json!({}))["ids"], json!([a.0, b.0]));
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, b]);
    assert!(s.execute("appearance.targetContents", &json!({})).is_err(), "paths have no members");
}

#[test]
fn effects_and_fills_apply_to_layers_by_id() {
    let mut s = session();
    let (g, _) = group(&mut s);
    let layer = s.doc().unwrap().doc.layers[0].id;
    run(&mut s, "select.none", json!({}));
    let r = run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "ids": [layer.0]}));
    assert_eq!(r["ids"], json!([layer.0]));
    run(&mut s, "appearance.addFill", json!({"ids": [layer.0]}));
    let l = node(&s, layer);
    assert_eq!((l.appearance.effects.len(), l.appearance.items.len()), (1, 1));
    assert!(node(&s, g).appearance.effects.is_empty());
    // Two undo steps take both off again.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, layer).appearance, vectorcraft_doc::Appearance::default());
    // Nothing selected and no ids: an error, not a silent no-op.
    assert!(s.execute("appearance.addFill", &json!({})).is_err());
}

#[test]
fn type_fills_go_above_or_below_the_characters() {
    let mut s = session();
    let t = id_of(&run(&mut s, "text.create", json!({"x": 20, "y": 100, "text": "Hi"})));
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    // A new fill on type copies its characters' fill (it has none of its own).
    run(&mut s, "appearance.addFill", json!({}));
    let n = node(&s, t);
    let NodeKind::Text(tx) = &n.kind else { panic!("type") };
    assert_eq!(n.appearance.fill_paint(), tx.runs[0].style.fill);
    run(&mut s, "appearance.moveItem", json!({"from": "contents", "to": 1}));
    assert_eq!(node(&s, t).appearance.contents_at(), 1);
}

#[test]
fn an_effect_on_the_types_own_stroke_exports_as_the_offset_outline() {
    let mut s = session();
    let t = id_of(&run(&mut s, "text.create", json!({"x": 20, "y": 100, "text": "Hi"})));
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "paint.setStroke", json!({"color": "#ff0000", "item": 0}));
    run(&mut s, "appearance.moveItem", json!({"from": "contents", "to": 1}));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 4}, "item": 0}));
    let doc = s.doc().unwrap().doc.clone();
    let svg = vectorcraft_svg::export(&doc, &Default::default());
    assert!(!svg.contains("<text") && svg.contains("<path"), "{svg}");
    // The baked type is its outlines, and the stroke's outline lies 4 pt outside the glyphs on every side.
    let glyphs = vectorcraft_render::effects::outline_text(&node(&s, t)).unwrap().geometric_bounds().unwrap();
    let baked = vectorcraft_render::effects::bake_document(&doc).unwrap();
    let n = baked.node(t).unwrap();
    assert!(!matches!(n.kind, NodeKind::Text(_)), "{n:?}");
    let b = n.geometric_bounds().unwrap();
    let off = [glyphs.x0 - b.x0, glyphs.y0 - b.y0, b.x1 - glyphs.x1, b.y1 - glyphs.y1];
    assert!(off.iter().all(|o| (o - 4.0).abs() < 0.5), "{off:?}");
}

#[test]
fn type_with_an_inline_graphic_stays_type_when_its_own_stroke_has_an_effect() {
    let mut s = session();
    run(&mut s, "shape.ellipse", json!({"x": 150, "y": 150, "width": 10, "height": 10}));
    run(&mut s, "symbol.new", json!({"name": "Dot"}));
    run(&mut s, "edit.clear", json!({}));
    let t = id_of(&run(&mut s, "text.create", json!({"x": 20, "y": 100, "text": "Hi"})));
    run(&mut s, "text.insertInline", json!({"id": t.0, "at": 0, "symbol": "Dot"}));
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "paint.setStroke", json!({"color": "#ff0000", "item": 0}));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 4}, "item": 0}));
    // Exports keep it as type, with its inline graphic.
    let baked = vectorcraft_render::effects::bake_document(&s.doc().unwrap().doc).unwrap();
    assert!(matches!(baked.node(t).unwrap().kind, NodeKind::Text(_)));
}

/// A selected 30×30 red image placed with its frame from (50, 50) to (80, 80).
fn image(s: &mut Session) -> NodeId {
    let mut png = vec![];
    image::RgbaImage::from_pixel(30, 30, image::Rgba([255, 0, 0, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let r = run(s, "file.place", json!({"name": "red.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [65, 65]}));
    NodeId(r["ids"][0].as_u64().unwrap())
}

#[test]
fn new_image_fills_and_strokes_go_below_the_image() {
    let mut s = session();
    let im = image(&mut s);
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "appearance.addFill", json!({}));
    // Each lands right below the Contents row: [stroke, fill | image].
    let ap = node(&s, im).appearance;
    assert!(!ap.items[0].is_fill() && ap.items[1].is_fill() && ap.contents_at() == 2, "{ap:?}");
    assert_eq!(run(&mut s, "appearance.moveItem", json!({"from": "contents", "to": 1}))["contents"], 1);
    assert_eq!(node(&s, im).appearance.contents_at(), 1);
    // The active row above the image stays active when a new item goes in below it.
    run(&mut s, "appearance.setActiveItem", json!({"index": 1}));
    run(&mut s, "appearance.addStroke", json!({}));
    assert_eq!(s.appearance_item(), Some(2));
    assert!(node(&s, im).appearance.items[2].is_fill());
}

#[test]
fn an_images_stroke_with_outline_object_draws_exports_and_expands_along_its_frame() {
    let mut s = session();
    let im = image(&mut s);
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "appearance.setItem", json!({"index": 0, "weight": 4, "color": "#000000"}));
    // Without an effect the stroke draws nothing, and Use Preview Bounds measures the frame.
    assert_eq!(pixel(&s, 49, 65), [255; 4]);
    s.prefs.use_preview_bounds = true;
    assert_eq!(s.transform_bounds(&[im]), Some(vectorcraft_geom::Rect::new(50.0, 50.0, 80.0, 80.0)));
    assert!(s.execute("effect.expandAppearance", &json!({})).is_err());
    run(&mut s, "effect.apply", json!({"effect": "path.outlineObject", "item": 0}));
    // Under the image, the stroke's outer 2 pt shows.
    assert_eq!((pixel(&s, 48, 65), pixel(&s, 49, 65), pixel(&s, 51, 65)), ([0, 0, 0, 255], [0, 0, 0, 255], RED));
    assert_eq!(pixel(&s, 47, 65), [255; 4]);
    // SVG and PDF hold the image and the frame stroked 4 pt.
    let doc = s.doc().unwrap().doc.clone();
    let svg = vectorcraft_svg::export(&doc, &Default::default());
    assert!(svg.contains("<image") && svg.contains("stroke-width=\"4\""), "{svg}");
    let back = vectorcraft_pdf::import(&vectorcraft_pdf::export(&doc, &Default::default()).unwrap()).unwrap();
    let (mut images, mut frames) = (0, 0);
    for l in &back.layers {
        l.walk(&mut |n| {
            images += usize::from(matches!(n.kind, NodeKind::Image(_)));
            let framed = n.geometric_bounds().is_some_and(|b| [b.x0 - 50.0, b.y0 - 50.0, b.x1 - 80.0, b.y1 - 80.0].iter().all(|d| d.abs() < 0.01));
            frames += usize::from(matches!(n.kind, NodeKind::Path { .. }) && framed && (n.appearance.stroke_width() - 4.0).abs() < 0.01);
        });
    }
    assert_eq!((images, frames), (1, 1));
    // Expand Appearance leaves the stroke's outline (a filled path) below the image, in one step.
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "effect.expandAppearance", json!({}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let n = node(&s, im);
    let kids = n.children().unwrap();
    assert_eq!(kids.len(), 2, "{n:?}");
    assert!(matches!(kids[1].kind, NodeKind::Image(_)), "{n:?}");
    let outline = kids[0].visual_bounds().unwrap();
    assert!((outline.x0 - 48.0).abs() < 0.01 && (outline.x1 - 82.0).abs() < 0.01, "{outline:?}");
    assert_eq!(kids[0].appearance.fill_paint(), Paint::solid(Color::BLACK));
}

/// Click with the Selection tool at (x, y) on a 100% view: what is selected afterwards.
fn click(s: &mut Session, x: f64, y: f64) -> Vec<NodeId> {
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    for k in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
        s.pointer(&vectorcraft_tools::PointerEvent::new(k, x, y), v).unwrap();
    }
    s.active().unwrap().selection.objects.clone()
}

#[test]
fn a_click_on_an_images_stroke_or_fill_outside_its_frame_selects_the_image() {
    let mut s = session();
    let im = image(&mut s);
    // A 4 pt stroke 12 pt outside the frame: it paints from x 36 to 40, the frame starts at 50.
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "appearance.setItem", json!({"index": 0, "weight": 4, "color": "#000000"}));
    run(&mut s, "effect.apply", json!({"effect": "path.outlineObject", "item": 0}));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 12}, "item": 0}));
    assert_eq!(pixel(&s, 38, 65), [0, 0, 0, 255]);
    run(&mut s, "select.none", json!({}));
    assert_eq!(click(&mut s, 38.0, 65.0), [im]);
    // Between the stroke and the frame, and outside the stroke, nothing paints.
    run(&mut s, "select.none", json!({}));
    assert!(click(&mut s, 45.0, 65.0).is_empty());
    assert!(click(&mut s, 30.0, 65.0).is_empty());
    // A fill that Convert to Shape makes 9 pt larger than the frame on each side.
    run(&mut s, "select.set", json!({"ids": [im.0]}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "paint.setFill", json!({"color": "#00ff00", "item": 1}));
    run(&mut s, "effect.apply", json!({"effect": "convertToShape.rectangle", "item": 1}));
    assert_eq!(pixel(&s, 44, 65), GREEN);
    run(&mut s, "select.none", json!({}));
    assert_eq!(click(&mut s, 44.0, 65.0), [im]);
    // In a group, the stroke selects the group.
    let r = id_of(&run(&mut s, "shape.rectangle", json!({"x": 120, "y": 120, "width": 20, "height": 20})));
    run(&mut s, "select.set", json!({"ids": [im.0, r.0]}));
    let g = id_of(&run(&mut s, "object.group", json!({})));
    run(&mut s, "select.none", json!({}));
    assert_eq!(click(&mut s, 38.0, 65.0), [g]);
}

#[test]
fn a_click_on_the_copy_of_an_images_stroke_that_a_transform_effect_makes_selects_the_image() {
    let mut s = session();
    let im = image(&mut s);
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "appearance.setItem", json!({"index": 0, "weight": 4, "color": "#000000"}));
    run(&mut s, "effect.apply", json!({"effect": "path.outlineObject", "item": 0}));
    // One copy 60 pt to the right: its stroke paints from x 138 to 142 on the right.
    run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": {"moveH": 60, "copies": 1}, "item": null}));
    assert_eq!(pixel(&s, 141, 65), [0, 0, 0, 255]);
    run(&mut s, "select.none", json!({}));
    assert_eq!(click(&mut s, 141.0, 65.0), [im]);
    run(&mut s, "select.none", json!({}));
    assert!(click(&mut s, 147.0, 65.0).is_empty());
}

#[test]
fn group_appearance_round_trips_and_expands() {
    let mut s = session();
    let (g, _) = group(&mut s);
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "appearance.moveItem", json!({"from": "contents", "to": 1}));
    run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": {"moveV": 50}}));
    let doc = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(back.node(g).unwrap().appearance, doc.node(g).unwrap().appearance);
    assert_eq!(back.node(g).unwrap().appearance.contents_index, Some(1));
    // Expand Appearance: the fill and stroke become paths below and above the moved members.
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, g);
    assert!(n.appearance.items.is_empty() && n.appearance.effects.is_empty());
    let ch = n.children().unwrap();
    assert_eq!(ch.len(), 4, "fill, two members, stroke");
    // Each is a group of one path per member (sharing the item's opacity and blend mode); the
    // stroke's paths are its outlines, filled.
    let first = |n: &Node| n.children().unwrap()[0].appearance.items[0].is_fill();
    assert!(first(&ch[0]) && first(&ch[3]) && ch[0].children().unwrap().len() == 2);
    let outline = ch[3].children().unwrap()[0].geometric_bounds().unwrap();
    assert!((outline.width() - 41.0).abs() < 0.1, "{outline:?}");
    // Moved down 50 pt; the outlined 1 pt stroke reaches half a point beyond.
    assert!((n.geometric_bounds().unwrap().y0 - 69.5).abs() < 1e-6);
}

#[test]
fn type_in_a_group_with_a_geometry_effect_exports_and_expands_with_its_look() {
    let mut s = session();
    let square = id_of(&run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})));
    let t = id_of(&run(&mut s, "text.create", json!({"x": 20, "y": 100, "text": "Hi"})));
    run(&mut s, "transparency.set", json!({"ids": [t.0], "opacity": 50}));
    run(&mut s, "select.set", json!({"ids": [square.0, t.0]}));
    let g = id_of(&run(&mut s, "object.group", json!({})));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 0}}));
    // SVG export writes the type's outlines at its opacity, and none of them while it is hidden.
    let svg = |s: &Session| vectorcraft_svg::export(&s.doc().unwrap().doc, &Default::default());
    let shown = svg(&s);
    assert!(!shown.contains("<text") && shown.contains("opacity=\"0.5\""), "{shown}");
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "object.hide", json!({}));
    let hidden = svg(&s);
    assert_eq!(hidden.matches("<path").count() + 1, shown.matches("<path").count(), "{hidden}");
    // Expand Appearance keeps the type's opacity and visibility on its outlines.
    run(&mut s, "select.set", json!({"ids": [g.0]}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, g);
    let outlines = &n.children().unwrap()[1];
    assert!(!matches!(outlines.kind, NodeKind::Text(_)) && (outlines.opacity - 0.5).abs() < 1e-6 && !outlines.visible, "{outlines:?}");
}

#[test]
fn hidden_type_with_a_geometry_effect_is_left_out_of_svg_export() {
    let mut s = session();
    let t = id_of(&run(&mut s, "text.create", json!({"x": 20, "y": 100, "text": "Hi"})));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 0}, "ids": [t.0]}));
    let svg = |s: &Session| vectorcraft_svg::export(&s.doc().unwrap().doc, &Default::default());
    let shown = svg(&s);
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "object.hide", json!({}));
    let hidden = svg(&s);
    assert_eq!((shown.matches("<path").count(), hidden.matches("<path").count()), (1, 0), "{hidden}");
}
