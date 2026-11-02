//! Object → Expand Appearance: fills and strokes as objects, baked geometry effects, raster
//! effects as images, brushes as art, and the enable rule.

use serde_json::{Value, json};
use vectorcraft_color::BlendMode;
use vectorcraft_doc::{Node, NodeKind};

use crate::tests_menucmds::{place_undecodable_png, warns_undecoded};
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

/// A selected 100×100 square at (50, 50) with a red fill and a 4 pt blue stroke.
fn square(s: &mut Session) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"color": "#ff0000"}));
    run(s, "paint.setStroke", json!({"color": "#0000ff"}));
    run(s, "stroke.set", json!({"weight": 4}));
    id
}

fn render(s: &Session) -> vectorcraft_render::Rendered {
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts)
}

/// Pixels (of 200×200) whose colour differs by more than `tol` in any channel.
fn differing(a: &vectorcraft_render::Rendered, b: &vectorcraft_render::Rendered, tol: u8) -> usize {
    a.pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0)
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > tol))
        .count()
}

fn expandable(s: &Session) -> bool {
    s.commands().iter().find(|c| c.id == "effect.expandAppearance").unwrap().enabled
}

#[test]
fn two_fills_and_an_offset_stroke_become_three_paths_that_look_the_same() {
    let mut s = session();
    let id = square(&mut s);
    assert!(!expandable(&s), "a basic appearance has nothing to expand");
    // A second fill on top (half-transparent green) and an outset stroke.
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "appearance.setItem", json!({"index": 2, "color": "#00ff00", "opacity": 50}));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 10}, "item": 1}));
    assert!(expandable(&s));
    let before = render(&s);
    let r = run(&mut s, "effect.expandAppearance", json!({}));
    assert_eq!(r["ids"], json!([id.0]));
    let n = node(&s, id);
    let ch = n.children().expect("a group of the fills and the stroke");
    assert_eq!(ch.len(), 3);
    assert!(ch.iter().all(|c| matches!(c.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) && c.appearance.items.len() == 1));
    // In paint order: red fill, the stroke on the path offset 10 pt out (still a 4 pt stroke, which
    // Object › Expand would outline, #3089), the green fill.
    assert!(ch.iter().all(|c| c.appearance.items[0].effects().is_empty()));
    assert!(ch[0].appearance.items[0].is_fill() && !ch[1].appearance.items[0].is_fill() && ch[2].appearance.items[0].is_fill());
    assert_eq!(ch[1].appearance.stroke_width(), 4.0);
    let ring = ch[1].geometric_bounds().unwrap();
    assert!((ring.width() - 120.0).abs() < 0.5, "{ring:?}");
    let ring = ch[1].visual_bounds().unwrap();
    assert!((ring.width() - 124.0).abs() < 0.5, "{ring:?}");
    assert!((ch[2].opacity - 0.5).abs() < 1e-6, "the fill's opacity is the path's own");
    assert!(ch[2].appearance.is_basic());
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 200, "{} pixels differ", differing(&before, &after, 8));
    assert!(!expandable(&s));
    // One undo step.
    run(&mut s, "edit.undo", json!({}));
    assert!(matches!(node(&s, id).kind, NodeKind::Path { .. }));
    assert_eq!(node(&s, id).appearance.items.len(), 3);
}

/// The objects [`random_transforms`] makes.
pub(super) const RANDOM_CASES: [&str; 5] = ["copies", "group copies", "nested group copies", "crop marks", "group crop marks"];

/// Selected art with Random in Transform effects, from 50 pt squares at y 60 with an 8 pt black
/// stroke, as `case` says. "copies": a square at x 20 filled with a gradient, a random Transform
/// effect on its stroke and a Transform effect that copies it 80 pt right. "group copies": the
/// same square without the second effect, in a group that has it. "nested group copies": the same
/// square in a group with a random Transform effect, in a group with the second effect. "crop
/// marks": a red square at x 20 with a random Transform effect and crop marks. "group crop marks":
/// red squares at x 20 and x 110 in a group with a random Transform effect and crop marks. Returns
/// the object's id.
pub(super) fn random_transforms(s: &mut Session, case: &str) -> NodeId {
    let random = json!({"moveH": 15, "moveV": 15, "rotate": 40, "random": true});
    let square = |s: &mut Session, x: f64, fill: Value| {
        let id = run(s, "shape.rectangle", json!({"x": x, "y": 60, "width": 50, "height": 50}))["id"].as_u64().unwrap();
        run(s, "paint.setFill", fill);
        run(s, "paint.setStroke", json!({"color": "#000000"}));
        run(s, "stroke.set", json!({"weight": 8}));
        id
    };
    let red = json!({"color": "#ff0000"});
    let id = match case {
        "copies" | "group copies" | "nested group copies" => {
            let a = square(s, 20.0, json!({"gradient": {"kind": "linear"}}));
            run(s, "effect.apply", json!({"effect": "distort.transform", "params": random, "item": 1}));
            if case == "nested group copies" {
                run(s, "object.group", json!({}));
                run(s, "effect.apply", json!({"effect": "distort.transform", "params": random, "item": null}));
            }
            if case == "copies" { a } else { run(s, "object.group", json!({}))["id"].as_u64().unwrap() }
        }
        "crop marks" => square(s, 20.0, red),
        _ => {
            let a = square(s, 20.0, red.clone());
            let b = square(s, 110.0, red);
            run(s, "select.set", json!({"ids": [a, b]}));
            run(s, "object.group", json!({}))["id"].as_u64().unwrap()
        }
    };
    if case.ends_with("copies") {
        run(s, "effect.apply", json!({"effect": "distort.transform", "params": {"moveH": 80, "copies": 1}, "item": null}));
    } else {
        run(s, "effect.apply", json!({"effect": "distort.transform", "params": random, "item": null}));
        run(s, "effect.apply", json!({"effect": "cropMarks", "item": null}));
    }
    NodeId(id)
}

/// Random in a Transform effect takes its seed from the object's id. The expanded art draws as the
/// canvas drew each of [`random_transforms`]' objects.
#[test]
fn expanded_art_keeps_the_random_transforms_the_canvas_drew() {
    let mut differ = vec![];
    for case in RANDOM_CASES {
        let mut s = session();
        let id = random_transforms(&mut s, case);
        let before = render(&s);
        let r = run(&mut s, "effect.expandAppearance", json!({}));
        differ.push((case, differing(&before, &render(&s), 64)));
        let mut art = node(&s, id);
        // Crop marks: the reply names the group of the object and its marks, then the object.
        if case.ends_with("crop marks") {
            art = (*art.children().unwrap()[0]).clone();
            assert_eq!((&r["ids"][0], &r["ids"][1]), (&json!(id.0), &json!(art.id.0)), "{case}");
        }
        // No object keeps an effect, and in the copies cases every object has a basic appearance:
        // every copy is expanded (its stroke a stroked path of its own, #3089).
        art.walk(&mut |n| {
            let unexpanded = case.ends_with("copies") && !n.appearance.is_basic();
            assert!(n.appearance.effects.is_empty() && !unexpanded, "{case}: {n:?}");
        });
    }
    assert_eq!(differ, RANDOM_CASES.map(|case| (case, 0)));
}

/// The reply lists each object whose appearance expanded once, by the id it has in the document,
/// starting with the selected object. In "group copies" the two copies of the square share an id
/// until Expand Appearance gives them ids of their own, and the reply names the group, then each
/// copy by its new id.
#[test]
fn the_reply_names_each_expanded_copy_by_its_own_id() {
    for case in RANDOM_CASES {
        let mut s = session();
        let id = random_transforms(&mut s, case);
        let r = run(&mut s, "effect.expandAppearance", json!({}));
        let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
        assert_eq!(ids.first(), Some(&id.0), "{case}: {ids:?}");
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "{case}: {ids:?}");
        assert!(ids.iter().all(|&i| s.doc().unwrap().doc.node(NodeId(i)).is_some()), "{case}: {ids:?}");
        if case == "group copies" {
            let art = node(&s, id);
            let copies: Vec<u64> = art.children().unwrap()[0].children().unwrap().iter().map(|c| c.id.0).collect();
            assert_eq!(ids, [vec![id.0], copies].concat(), "{case}");
        }
    }
}

/// A square with a color adjustment and crop marks: the reply names the group of the square and its
/// marks once. The adjusted square has a basic appearance, so the reply does not name it.
#[test]
fn the_reply_names_an_adjusted_object_with_crop_marks_once() {
    let mut s = session();
    let id = square(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}}));
    run(&mut s, "effect.apply", json!({"effect": "cropMarks", "item": null}));
    let r = run(&mut s, "effect.expandAppearance", json!({}));
    assert!(node(&s, id).children().is_some_and(|c| c.len() > 1));
    assert_eq!(r["ids"], json!([id.0]));
}

#[test]
fn a_shadow_becomes_an_image_under_the_art_and_transparency_stays_on_the_group() {
    let mut s = session();
    let id = square(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "params": {"x": 8, "y": 8, "blur": 2, "opacity": 100}}));
    run(&mut s, "transparency.set", json!({"opacity": 60, "blend": "multiply"}));
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!((n.opacity - 0.6).abs() < 1e-6 && n.blend == BlendMode::Multiply);
    assert!(n.appearance.effects.is_empty() && n.appearance.items.is_empty());
    let ch = n.children().unwrap();
    assert_eq!(ch.len(), 3, "the shadow image, the fill, the outlined stroke");
    let NodeKind::Image(im) = &ch[0].kind else { panic!("the shadow is an image: {}", ch[0].kind_label()) };
    assert!(s.doc().unwrap().doc.images.contains_key(&im.key));
    assert!(ch[1..].iter().all(|c| c.opacity == 1.0 && c.appearance.effects.is_empty()));
    let after = render(&s);
    assert!(differing(&before, &after, 24) < 400, "{} pixels differ", differing(&before, &after, 24));
    // The shadow is still there, below and right of the square.
    let p = after.pixel(155, 155);
    assert!(p[0] < 200, "{p:?}");
}

#[test]
fn a_blur_turns_the_object_into_an_image() {
    let mut s = session();
    let id = square(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "blur.gaussian", "params": {"radius": 3}}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::Image(_)), "{}", n.kind_label());
    assert!(n.appearance.effects.is_empty());
}

#[test]
fn expanding_a_blur_says_when_an_image_could_not_be_decoded() {
    let mut s = session();
    let cut = place_undecodable_png(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "blur.gaussian", "params": {"radius": 3}, "ids": [cut.0]}));
    let r = run(&mut s, "effect.expandAppearance", json!({"ids": [cut.0]}));
    assert!(warns_undecoded(&r), "{r}");
    assert!(matches!(node(&s, cut).kind, NodeKind::Image(_)));
}

#[test]
fn an_art_brush_becomes_a_group_of_its_art() {
    let mut s = session();
    let id = NodeId(run(&mut s, "shape.line", json!({"x1": 20, "y1": 100, "x2": 180, "y2": 100}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "brush.apply", json!({"name": "Charcoal"}));
    assert!(expandable(&s));
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::Group { .. }), "{}", n.kind_label());
    let mut brushed = false;
    n.walk(&mut |c| brushed |= vectorcraft_brush::has_brush(c));
    assert!(!brushed, "no brush strokes are left");
    let after = render(&s);
    assert!(differing(&before, &after, 24) < 200, "{} pixels differ", differing(&before, &after, 24));
    run(&mut s, "edit.undo", json!({}));
    assert!(vectorcraft_brush::has_brush(&node(&s, id)));
}

#[test]
fn a_targeted_layer_with_a_fill_expands_into_art_among_its_members() {
    let mut s = session();
    let a = square(&mut s);
    let layer = s.doc().unwrap().doc.layers[0].id;
    run(&mut s, "layer.target", json!({"id": layer.0}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "stroke.set", json!({"item": 0, "weight": 6}));
    assert!(expandable(&s));
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let l = node(&s, layer);
    assert!(l.appearance.items.is_empty());
    let ch = l.children().unwrap();
    assert_eq!(ch.len(), 2, "the square, then the layer's stroke above it");
    assert_eq!(ch[0].id, a);
    // The layer's stroke stays a stroke, on a path of its own (#3089).
    let mut six = 0;
    ch[1].walk(&mut |n| {
        assert!(n.appearance.items.iter().all(|i| !i.is_fill()), "{n:?}");
        six += usize::from(n.appearance.stroke_width() == 6.0);
    });
    assert!(six > 0, "the layer's 6 pt stroke: {:?}", ch[1]);
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 200, "{} pixels differ", differing(&before, &after, 8));
}

#[test]
fn type_with_a_shadow_stays_type_and_type_with_its_own_stroke_is_outlined() {
    let mut s = session();
    let t = NodeId(run(&mut s, "text.create", json!({"x": 20, "y": 80, "text": "Hi", "size": 48}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow"}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, t);
    let ch = n.children().unwrap();
    assert!(matches!(ch[0].kind, NodeKind::Image(_)) && matches!(ch[1].kind, NodeKind::Text(_)));
    // A stroke of its own: outlined glyphs.
    let u = NodeId(run(&mut s, "text.create", json!({"x": 20, "y": 160, "text": "Yo", "size": 48}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [u.0]}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let mut text = false;
    node(&s, u).walk(&mut |c| text |= matches!(c.kind, NodeKind::Text(_)));
    assert!(!text, "the type is outlined");
    assert!(!expandable(&s));
}

#[test]
fn type_with_its_own_stroke_expands_with_its_underline() {
    let mut s = session();
    let t = NodeId(run(&mut s, "text.create", json!({"x": 20, "y": 160, "text": "Hi", "size": 48}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "text.setFormat", json!({"underline": true}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "effect.expandAppearance", json!({}));
    // "Hi" has no ink below its baseline (y 160): the pieces reach the bar under it.
    let b = node(&s, t).geometric_bounds().unwrap();
    assert!(b.y1 > 162.0, "{b:?}");
}

#[test]
fn a_blurred_layer_keeps_its_image_as_its_only_member() {
    let mut s = session();
    square(&mut s);
    let layer = s.doc().unwrap().doc.layers[0].id;
    run(&mut s, "layer.clippingMask.toggle", json!({"id": layer.0}));
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 200}));
    run(&mut s, "layer.target", json!({"id": layer.0}));
    run(&mut s, "effect.apply", json!({"effect": "blur.gaussian"}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let l = node(&s, layer);
    assert!(l.is_layer() && !l.clips() && l.appearance.effects.is_empty());
    let ch = l.children().unwrap();
    assert!(ch.len() == 1 && matches!(ch[0].kind, NodeKind::Image(_)));
}

/// A selected open curve with no fill, stroked in blue with `stroke` (stroke.set's params) and
/// given a Zig Zag (#3089).
fn zig_zag_curve(s: &mut Session, stroke: Value) -> NodeId {
    let id = NodeId(run(s, "path.create", json!({"d": "M 30 100 C 70 40 130 160 170 100"}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"none": true}));
    run(s, "paint.setStroke", json!({"color": "#0000ff"}));
    run(s, "stroke.set", stroke);
    run(s, "effect.apply", json!({"effect": "distort.zigZag", "params": {"size": 8, "ridges": 5, "relative": false}}));
    id
}

#[test]
fn a_stroke_stays_a_stroke_with_its_dashes_arrowheads_alignment_and_width_profile() {
    for stroke in [
        json!({"weight": 4, "cap": "round", "join": "round", "dash": [10, 5]}),
        json!({"weight": 3, "startArrow": "Arrow", "endArrow": "Circle"}),
        json!({"weight": 8, "profile": "lens"}),
        json!({"weight": 6, "align": "inside"}),
    ] {
        let mut s = session();
        let id = zig_zag_curve(&mut s, stroke.clone());
        let was = node(&s, id).appearance.stroke().unwrap().clone();
        let before = render(&s);
        run(&mut s, "effect.expandAppearance", json!({}));
        // The zig-zag is baked into the path, which keeps the very same stroke and no fill: no outline.
        let n = node(&s, id);
        assert!(matches!(n.kind, NodeKind::Path { .. }), "{stroke}: a path, not a group: {:?}", n.kind);
        assert!(n.appearance.effects.is_empty() && n.appearance.is_basic(), "{stroke}");
        assert_eq!(n.appearance.stroke(), Some(&was), "{stroke}: the stroke's attributes stay");
        assert!(n.appearance.fill().is_none_or(|f| f.paint.is_none()), "{stroke}: no fill");
        assert!(n.path_data().unwrap().anchors().count() > 4, "{stroke}: the zig-zag is baked");
        let after = render(&s);
        assert!(differing(&before, &after, 8) < 40, "{stroke}: {} pixels differ", differing(&before, &after, 8));
        assert!(!expandable(&s), "{stroke}: nothing left for Expand Appearance");
    }
}

#[test]
fn several_strokes_become_stroked_paths_in_appearance_order() {
    let mut s = session();
    let id = square(&mut s);
    // [red fill, 4 pt blue stroke, 12 pt green stroke] with a Zig Zag on the object.
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "stroke.set", json!({"item": 2, "weight": 12}));
    run(&mut s, "appearance.setItem", json!({"index": 2, "color": "#00ff00"}));
    run(&mut s, "effect.apply", json!({"effect": "distort.zigZag", "params": {"size": 6, "ridges": 4, "relative": false}}));
    let items = node(&s, id).appearance.items.clone();
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    let ch = n.children().expect("a group of the fill and the two strokes");
    assert_eq!(ch.len(), 3);
    for (c, item) in ch.iter().zip(&items) {
        assert!(matches!(c.kind, NodeKind::Path { .. }) && c.appearance.items.len() == 1, "{:?}", c.kind);
        assert_eq!(&c.appearance.items[0], item, "each piece keeps its own fill or stroke, in appearance order");
    }
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 60, "{} pixels differ", differing(&before, &after, 8));
}
