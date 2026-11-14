//! Object → Expand (M3.84): the Object, Fill and Stroke options, gradient fills into solid strips,
//! concentric ellipses or a gradient mesh inside a clip group, freeform gradients into a mesh,
//! pattern fills into their tiles inside a clip group, and Create Gradient Mesh taking a gradient
//! fill's colours.

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{AppearanceItem, Node, NodeKind};
use vectorcraft_geom::Rect;

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

/// A selected rectangle filled with `gradient` (paint.setFill's), with no stroke unless `stroke`.
fn gradient_rect(s: &mut Session, r: Rect, gradient: Value, stroke: bool) -> NodeId {
    let id = run(s, "shape.rectangle", json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()}))["id"].as_u64().unwrap();
    run(s, "paint.setFill", json!({ "gradient": gradient }));
    if stroke {
        run(s, "paint.setStroke", json!({"color": "#0000ff"}));
        run(s, "stroke.set", json!({"weight": 4}));
    } else {
        run(s, "paint.setStroke", json!({"none": true}));
    }
    NodeId(id)
}

/// A selected square `r` with a freeform fill whose four automatic points take `colors` in turn.
fn freeform_rect(s: &mut Session, r: Rect, colors: [Value; 4]) -> NodeId {
    let id = gradient_rect(s, r, json!({"kind": "freeform"}), false);
    for (i, c) in colors.into_iter().enumerate() {
        run(s, "paint.freeform.setPoint", json!({"index": i, "color": c, "stroke": false}));
    }
    id
}

/// A CMYK color's inks.
fn inks(c: Color) -> [f32; 4] {
    match c {
        Color::Cmyk { c, m, y, k } => [c, m, y, k],
        c => panic!("a CMYK color expected: {c:?}"),
    }
}

fn close_inks(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
}

/// The one selected object after a command.
fn selected(s: &Session) -> Node {
    let sel = &s.doc().unwrap().selection.objects;
    assert_eq!(sel.len(), 1, "one object selected");
    node(s, sel[0])
}

/// A clip group's clipping path and contents.
fn clip_parts(n: &Node) -> (&Node, Vec<&Node>) {
    match &n.kind {
        NodeKind::Group { clip: true, children } => (&children[0], children[1..].iter().map(|c| &**c).collect()),
        k => panic!("expected a clip group, got {k:?}"),
    }
}

fn fill_color(n: &Node) -> Color {
    n.appearance.fill_paint().color().unwrap_or_else(|| panic!("solid fill expected on {:?}", n.kind))
}

fn render(s: &Session) -> vectorcraft_render::Rendered {
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts)
}

/// The mean difference of the colour channels of two renders.
fn mean_diff(a: &vectorcraft_render::Rendered, b: &vectorcraft_render::Rendered) -> f64 {
    let sum: u64 = a.pixels.iter().zip(b.pixels.iter()).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    sum as f64 / a.pixels.len() as f64
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

/// Pattern "Dots" (a red 10×10 square with a black 1 pt stroke, centered in a 20×20 tile) and a
/// selected rectangle filled with it, with a blue 4 pt stroke when `stroke`.
fn pattern_rect(s: &mut Session, r: Rect, stroke: bool) -> NodeId {
    let dot = run(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].as_u64().unwrap();
    run(s, "paint.setFill", json!({"color": "#ff0000"}));
    run(s, "paint.setStroke", json!({"color": "#000000"}));
    run(s, "stroke.set", json!({"weight": 1}));
    run(s, "object.pattern.make", json!({"ids": [dot], "name": "Dots", "width": 20, "height": 20, "edit": false}));
    run(s, "select.set", json!({"ids": [dot]}));
    run(s, "edit.clear", json!({}));
    let id = run(s, "shape.rectangle", json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()}))["id"].as_u64().unwrap();
    run(s, "paint.setFill", json!({"swatch": "Dots"}));
    if stroke {
        run(s, "paint.setStroke", json!({"color": "#0000ff"}));
        run(s, "stroke.set", json!({"weight": 4}));
    } else {
        run(s, "paint.setStroke", json!({"none": true}));
    }
    NodeId(id)
}

/// `object.expand` with every option on, for a selected rectangle with a pattern fill only: the
/// one selected object is then the clip group of its tiles.
fn expand_to_tiles(s: &mut Session) {
    run(s, "object.expand", json!({}));
    let g = selected(s);
    assert!(!clip_parts(&g).1.is_empty(), "the pattern fill became tiles");
}

fn white_black() -> Value {
    json!({"kind": "linear", "stops": [{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]})
}

#[test]
fn a_placed_gradient_moved_by_a_transform_or_free_distort_expands_as_the_canvas_draws_it() {
    // The object's own effect, and the fill's (the first appearance item).
    let slant = json!({"corners": [[0.25, 0.0], [1.25, 0.0], [1.0, 1.0], [0.0, 1.0]]});
    for (effect, params, item) in [
        ("distort.transform", json!({"rotate": 90}), json!(null)),
        ("distort.transform", json!({"rotate": 30, "scaleH": 60}), json!(null)),
        ("distort.transform", json!({"rotate": 90}), json!(0)),
        ("distort.freeDistort", slant.clone(), json!(null)),
        ("distort.freeDistort", slant, json!(0)),
    ] {
        let mut s = session();
        let id = gradient_rect(&mut s, Rect::new(40.0, 70.0, 160.0, 130.0), white_black(), false);
        // Placed from a quarter of the way across to the right edge.
        run(&mut s, "paint.setGradientGeom", json!({"start": [70, 100], "end": [160, 100], "stroke": false}));
        let placed = s.doc().unwrap().doc.node(id).and_then(|n| match n.appearance.fill_paint() {
            vectorcraft_color::Paint::Gradient(g) => g.geom,
            _ => None,
        });
        assert!(placed.is_some(), "the fill's gradient is placed");
        run(&mut s, "effect.apply", json!({"effect": effect, "params": params, "ids": [id.0], "item": item}));
        let before = render(&s);
        // Fill alone: the gradient becomes strips inside the outline the effect makes.
        run(&mut s, "object.expand", json!({"object": false, "stroke": false, "steps": 64}));
        let after = render(&s);
        // The gradient moves with the rectangle on the canvas, and the strips follow it.
        let diff = mean_diff(&before, &after);
        assert!(diff < 1.0, "{effect} {params} on item {item}: mean difference {diff}");
    }
}

/// A gradient fitted to the object turns with a Transform effect on the canvas, and Object › Expand
/// of the fill makes strips that draw the same.
#[test]
fn a_fitted_gradient_turned_by_a_transform_expands_as_the_canvas_draws_it() {
    // The gray level at (x, y).
    let gray = |r: &vectorcraft_render::Rendered, x: usize, y: usize| r.pixels[(y * 200 + x) * 4] as i32;
    // The object's own effect, and the fill's (the first appearance item).
    for (params, item) in
        [(json!({"rotate": 90}), json!(null)), (json!({"rotate": 90}), json!(0)), (json!({"rotate": 30, "scaleH": 60}), json!(null))]
    {
        let mut s = session();
        let id = gradient_rect(&mut s, Rect::new(60.0, 60.0, 140.0, 140.0), white_black(), false);
        run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": params, "ids": [id.0], "item": item}));
        let before = render(&s);
        if params["rotate"] == 90 {
            // The white to black ramp runs up or down the turned square, not across it.
            let (top, bottom) = (gray(&before, 100, 70), gray(&before, 100, 130));
            let (left, right) = (gray(&before, 70, 100), gray(&before, 130, 100));
            assert!((top - bottom).abs() > 100 && (left - right).abs() < 20, "{params} on item {item}: {top} {bottom} {left} {right}");
        }
        run(&mut s, "object.expand", json!({"object": false, "stroke": false, "steps": 64}));
        let after = render(&s);
        let diff = mean_diff(&before, &after);
        assert!(diff < 1.0, "{params} on item {item}: mean difference {diff}");
    }
}

/// A Transform effect after a drop shadow, on the object or on the fill, scales and turns the
/// shadow on the canvas; Object › Expand of the gradient fill keeps the shadow the canvas drew.
#[test]
fn a_shadow_before_a_transform_expands_as_the_canvas_draws_it() {
    for item in [json!(null), json!(0)] {
        let mut s = session();
        let id = gradient_rect(&mut s, Rect::new(80.0, 80.0, 120.0, 120.0), white_black(), false);
        for (effect, params) in [
            ("stylize.dropShadow", json!({"x": 10, "y": 0, "blur": 0, "opacity": 100})),
            ("distort.transform", json!({"scaleH": 200, "scaleV": 200, "rotate": 90})),
        ] {
            run(&mut s, "effect.apply", json!({"effect": effect, "params": params, "ids": [id.0], "item": item}));
        }
        let before = render(&s);
        run(&mut s, "object.expand", json!({"object": false, "stroke": false, "steps": 64}));
        let after = render(&s);
        let diff = differing(&before, &after, 64);
        assert_eq!(diff, 0, "item {item}: {diff} pixels differ");
    }
}

/// The object's Transform effect scales and turns the drop shadow of its gradient fill on the
/// canvas; Object › Expand of the fill keeps the shadow the canvas drew.
#[test]
fn a_fills_shadow_under_the_objects_transform_expands_as_the_canvas_draws_it() {
    let mut s = session();
    let id = gradient_rect(&mut s, Rect::new(80.0, 80.0, 120.0, 120.0), white_black(), false);
    for (effect, params, item) in [
        ("stylize.dropShadow", json!({"x": 10, "y": 0, "blur": 0, "opacity": 100}), json!(0)),
        ("distort.transform", json!({"scaleH": 200, "scaleV": 100, "rotate": 90}), json!(null)),
    ] {
        run(&mut s, "effect.apply", json!({"effect": effect, "params": params, "ids": [id.0], "item": item}));
    }
    let before = render(&s);
    run(&mut s, "object.expand", json!({"object": false, "stroke": false, "steps": 64}));
    let after = render(&s);
    let diff = differing(&before, &after, 64);
    assert_eq!(diff, 0, "{diff} pixels differ");
}

/// A group's Transform effect scales and turns its member's drop shadow on the canvas, and an
/// object's scales and turns its fill's, with Scale Strokes & Effects off or on; Expand Appearance
/// keeps the shadows the canvas drew.
#[test]
fn shadows_under_a_groups_or_an_objects_transform_expand_appearance_as_the_canvas_draws_them() {
    let shadow = json!({"x": 10, "y": 0, "blur": 0, "opacity": 100});
    for strokes in [false, true] {
        let transform = json!({"scaleH": 200, "scaleV": 100, "rotate": 90, "strokes": strokes});
        for group in [false, true] {
            let mut s = session();
            let id = gradient_rect(&mut s, Rect::new(80.0, 80.0, 120.0, 120.0), white_black(), false);
            let item = if group { json!(null) } else { json!(0) };
            run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "params": shadow, "ids": [id.0], "item": item}));
            if group {
                run(&mut s, "object.group", json!({}));
            }
            run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": transform}));
            let before = render(&s);
            run(&mut s, "effect.expandAppearance", json!({}));
            let after = render(&s);
            let diff = differing(&before, &after, 64);
            assert_eq!(diff, 0, "group {group}, Scale Strokes & Effects {strokes}: {diff} pixels differ");
        }
    }
}

/// A group's Transform effect runs after its member's own Offset Path in Expand Appearance as on
/// the canvas: the 100 pt square offset to 120 pt and scaled by half expands 60 pt wide.
#[test]
fn a_groups_effects_after_its_members_own_expand_as_the_canvas_draws_them() {
    let mut s = session();
    run(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100}));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 10}}));
    run(&mut s, "object.group", json!({}));
    run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": {"scaleH": 50, "scaleV": 50}}));
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let after = render(&s);
    let diff = differing(&before, &after, 64);
    assert_eq!(diff, 0, "{diff} pixels differ");
    let doc = &s.doc().unwrap().doc;
    let w = doc.node(doc.layers[0].id).and_then(|l| l.geometric_bounds()).unwrap().width();
    assert!((w - 60.0).abs() < 1e-6, "{w}");
}

#[test]
fn a_placed_gradient_moved_by_a_warp_expands_as_the_canvas_draws_it() {
    // The object's own effect, and the fill's (the first appearance item).
    for (effect, params, item) in [
        ("warp.arc", json!({"bend": 40}), json!(null)),
        ("warp.bulge", json!({"bend": 40, "orientation": "vertical"}), json!(null)),
        ("warp.inflate", json!({"bend": 40}), json!(0)),
    ] {
        let mut s = session();
        let id = gradient_rect(&mut s, Rect::new(40.0, 70.0, 160.0, 130.0), white_black(), false);
        // Placed from a quarter of the way across to the right edge.
        run(&mut s, "paint.setGradientGeom", json!({"start": [70, 100], "end": [160, 100], "stroke": false}));
        let placed = s.doc().unwrap().doc.node(id).and_then(|n| match n.appearance.fill_paint() {
            vectorcraft_color::Paint::Gradient(g) => g.geom,
            _ => None,
        });
        assert!(placed.is_some(), "the fill's gradient is placed");
        run(&mut s, "effect.apply", json!({"effect": effect, "params": params, "ids": [id.0], "item": item}));
        let before = render(&s);
        // Fill alone: the gradient becomes strips inside the outline the warp makes.
        run(&mut s, "object.expand", json!({"object": false, "stroke": false, "steps": 64}));
        let after = render(&s);
        let diff = mean_diff(&before, &after);
        assert!(diff < 1.0, "{effect} {params} on item {item}: mean difference {diff}");
        // The ramp's midpoint, 115 pt before the warp, moves and scales with the warped bounds.
        let doc = &s.doc().unwrap().doc;
        let b = doc.node(doc.layers[0].id).and_then(|l| l.geometric_bounds()).unwrap();
        let want = b.x0 + (115.0 - 40.0) / 120.0 * b.width();
        assert!((want - 115.0).abs() > 2.0, "{effect} {params}: the warp moves the midpoint ({b:?})");
        let red = |x: usize| f64::from(after.pixels[(100 * 200 + x) * 4]);
        // White turns dark once on row 100, and gradually, so the crossing is the ramp's midpoint
        // and not an edge of the outline.
        let crossings: Vec<usize> =
            (b.x0.ceil() as usize + 3..b.x1.floor() as usize - 4).filter(|&x| red(x) >= 127.5 && red(x + 1) < 127.5).collect();
        assert_eq!(crossings.len(), 1, "{effect} {params}: white turns dark once on row 100: {crossings:?}");
        let x = crossings[0];
        assert!(red(x - 2) - red(x + 3) < 100.0, "{effect} {params}: the ramp, not an edge: {} to {}", red(x - 2), red(x + 3));
        let half = x as f64 + 0.5 + (red(x) - 127.5) / (red(x) - red(x + 1));
        assert!((half - want).abs() < 1.0, "{effect} {params} on item {item}: the midpoint is at {half}, want {want}");
    }
}

#[test]
fn eight_strips_match_the_gradient_samples() {
    let mut s = session();
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 120.0), white_black(), false);
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "object.expand", json!({"steps": 8}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let g = selected(&s);
    let (clip, strips) = clip_parts(&g);
    assert!(matches!(clip.kind, NodeKind::Path { clipping: true, .. }));
    assert_eq!(clip.geometric_bounds().unwrap(), Rect::new(20.0, 20.0, 180.0, 120.0));
    assert_eq!(strips.len(), 8);
    for (k, strip) in strips.iter().enumerate() {
        // Band k starts at its eighth of the width (and runs on under the next bands, so no
        // seams show) and shows the gradient at its middle.
        let b = strip.geometric_bounds().unwrap();
        assert!((b.x0 - (20.0 + 20.0 * k as f64)).abs() < 1e-9, "strip {k} starts at {}", b.x0);
        assert!((b.x1 - 180.0).abs() < 1e-9 && (b.y0 - 20.0).abs() < 1e-9 && (b.y1 - 120.0).abs() < 1e-9);
        let gray = 1.0 - (k as f32 + 0.5) / 8.0;
        let [r, g, bl] = fill_color(strip).to_rgb();
        assert!((r - gray).abs() < 1e-4 && (g - gray).abs() < 1e-4 && (bl - gray).abs() < 1e-4, "strip {k}: {r} {g} {bl} vs {gray}");
    }
    run(&mut s, "edit.undo", json!({}));
    assert!(matches!(selected(&s).appearance.fill_paint(), Paint::Gradient(_)));
}

#[test]
fn expanding_a_group_keeps_its_hidden_and_locked_members_hidden_and_locked() {
    // A hidden stroked square with a gradient fill, a locked square with a pattern fill and a
    // plain square, grouped.
    let mut s = session();
    let pat = pattern_rect(&mut s, Rect::new(20.0, 100.0, 80.0, 160.0), false);
    let grad = gradient_rect(&mut s, Rect::new(20.0, 20.0, 80.0, 80.0), white_black(), true);
    let plain = NodeId(run(&mut s, "shape.rectangle", json!({"x": 100, "y": 20, "width": 40, "height": 40}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [pat.0, grad.0, plain.0]}));
    run(&mut s, "object.group", json!({}));
    s.edit("t", |d, _| {
        d.node_mut(grad).unwrap().visible = false;
        d.node_mut(pat).unwrap().locked = true;
        Ok(())
    })
    .unwrap();
    run(&mut s, "object.expand", json!({}));
    let g = selected(&s);
    let ch = g.children().unwrap();
    assert_eq!(ch.len(), 3);
    assert!(!clip_parts(&ch[0]).1.is_empty() && ch[0].locked && ch[0].visible, "the pattern's tiles stay locked: {:?}", ch[0]);
    assert!(!ch[1].visible && !ch[1].locked, "the gradient square's art stays hidden: {:?}", ch[1]);
    assert!(ch[2].visible && !ch[2].locked);
    // Showing or unlocking the new art shows or unlocks all of it.
    for c in ch[..2].iter().flat_map(|c| c.children().unwrap().iter()) {
        c.walk(&mut |n| assert!(n.visible && !n.locked, "{n:?}"));
    }
}

#[test]
fn a_radial_gradient_becomes_concentric_ellipses() {
    let mut s = session();
    gradient_rect(&mut s, Rect::new(50.0, 50.0, 150.0, 150.0), json!({"kind": "radial"}), false);
    run(&mut s, "object.expand", json!({"steps": 4}));
    let g = selected(&s);
    let (_, rings) = clip_parts(&g);
    assert_eq!(rings.len(), 4);
    // Largest first, all centred on the gradient's centre; the largest reaches the box's corners.
    let reach = 50.0 * 2f64.sqrt();
    for (i, ring) in rings.iter().enumerate() {
        let b = ring.geometric_bounds().unwrap();
        assert!((b.center().x - 100.0).abs() < 1e-6 && (b.center().y - 100.0).abs() < 1e-6);
        let r = reach * (4 - i) as f64 / 4.0;
        assert!((b.width() / 2.0 - r).abs() < 1e-6 && (b.height() / 2.0 - r).abs() < 1e-6, "ring {i}: {b:?} vs radius {r}");
    }
    // The centre disc shows the gradient at the middle of the first band (white → black).
    let t = (reach / 8.0 / 50.0) as f32;
    let [r, ..] = fill_color(rings[3]).to_rgb();
    assert!((r - (1.0 - t)).abs() < 1e-4, "{r} vs {}", 1.0 - t);
    // Translucent stops: rings with holes instead of stacked discs.
    let mut s = session();
    let gr = json!({"kind": "radial", "stops": [{"offset": 0, "color": "#ff0000", "opacity": 0.5}, {"offset": 1, "color": "#0000ff"}]});
    gradient_rect(&mut s, Rect::new(50.0, 50.0, 150.0, 150.0), gr, false);
    run(&mut s, "object.expand", json!({"steps": 4}));
    let g = selected(&s);
    let (_, rings) = clip_parts(&g);
    let subpaths = |n: &Node| n.path_data().unwrap().subpaths.len();
    assert_eq!(rings.iter().map(|r| subpaths(r)).collect::<Vec<_>>(), [2, 2, 2, 1]);
    let Some(vectorcraft_doc::AppearanceItem::Fill(f)) = rings[3].appearance.items.first() else { panic!() };
    assert!(f.opacity < 1.0 && f.opacity > 0.5, "the centre band is translucent: {}", f.opacity);
}

/// Three opaque stops sweeping once around the center.
fn conic_stops() -> Value {
    json!({"kind": "conic", "stops": [{"offset": 0, "color": "#ffff00"}, {"offset": 0.5, "color": "#ff00ff"}, {"offset": 1, "color": "#004080"}]})
}

#[test]
fn a_conic_gradient_becomes_wedges_or_a_mesh_that_paint_it() {
    // Objects: a wedge per step from the center, together painting what the canvas painted.
    let mut s = session();
    gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), conic_stops(), false);
    let before = render(&s);
    run(&mut s, "object.expand", json!({"steps": 180}));
    let g = selected(&s);
    let (_, wedges) = clip_parts(&g);
    assert_eq!(wedges.len(), 180);
    let first = wedges[0].path_data().unwrap().subpaths[0].anchors[0].p;
    assert!(first.distance(vectorcraft_geom::Point::new(100.0, 100.0)) < 1e-9, "wedges start at the center: {first:?}");
    let after = render(&s);
    assert!(differing(&before, &after, 48) < 200, "{}", differing(&before, &after, 48));
    assert!(mean_diff(&before, &after) < 2.0, "{}", mean_diff(&before, &after));
    // Translucent stops: each wedge is just its share of the turn, ending where the next starts,
    // so none doubles up (only the anti-aliased seams between them differ from the canvas).
    let mut s = session();
    let gr = json!({"kind": "conic", "stops": [{"offset": 0, "color": "#ff0000", "opacity": 0.5}, {"offset": 1, "color": "#0000ff"}]});
    gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), gr, false);
    let before = render(&s);
    run(&mut s, "object.expand", json!({"steps": 36}));
    let g = selected(&s);
    let (_, wedges) = clip_parts(&g);
    let outline = |n: &Node| n.path_data().unwrap().subpaths[0].anchors.iter().map(|a| a.p).collect::<Vec<_>>();
    for pair in wedges.windows(2) {
        let (a, b) = (outline(pair[0]), outline(pair[1]));
        assert!(a.last().unwrap().distance(b[1]) < 1e-6, "{a:?} then {b:?}");
    }
    assert!(mean_diff(&before, &render(&s)) < 4.0, "{}", mean_diff(&before, &render(&s)));
    // Mesh: columns around the center.
    let mut s = session();
    gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), conic_stops(), false);
    let before = render(&s);
    run(&mut s, "object.expand", json!({"gradient": "mesh"}));
    let g = selected(&s);
    let (_, content) = clip_parts(&g);
    let NodeKind::Mesh(m) = &content[0].kind else { panic!("a mesh node: {:?}", content[0].kind) };
    assert!(m.is_valid());
    let after = render(&s);
    assert!(differing(&before, &after, 48) < 200, "{}", differing(&before, &after, 48));
    assert!(mean_diff(&before, &after) < 2.5, "{}", mean_diff(&before, &after));
}

/// A mirrored conic, and one whose extent ellipse is flat (as a file can hold it), expand as the
/// canvas draws them.
#[test]
fn a_mirrored_or_flat_conic_expands_as_the_canvas_draws_it() {
    for aspect in [-0.6, 0.0] {
        for how in [json!({"steps": 180}), json!({"gradient": "mesh"})] {
            let mut s = session();
            let id = gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), conic_stops(), false);
            let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
            let n = d.node_mut(id).unwrap();
            let Paint::Gradient(mut g) = n.appearance.fill_paint().clone() else { panic!("a gradient fill") };
            let (start, end) = (vectorcraft_geom::Point::new(90.0, 110.0), vectorcraft_geom::Point::new(130.0, 80.0));
            g.geom = Some(vectorcraft_color::GradientGeom { start, end, aspect, focal: None });
            n.appearance.set_fill(Paint::Gradient(g));
            let before = render(&s);
            run(&mut s, "object.expand", how.clone());
            let after = render(&s);
            assert!(differing(&before, &after, 48) < 200, "{aspect} {how}: {}", differing(&before, &after, 48));
            assert!(mean_diff(&before, &after) < 2.5, "{aspect} {how}: {}", mean_diff(&before, &after));
        }
    }
}

/// Three opaque stops running from the center out to the square.
fn diamond_stops() -> Value {
    json!({"kind": "diamond", "stops": [{"offset": 0, "color": "#ffff00"}, {"offset": 0.5, "color": "#ff00ff"}, {"offset": 1, "color": "#004080"}]})
}

#[test]
fn a_diamond_gradient_becomes_nested_squares_or_a_mesh_that_paint_it() {
    // Objects: nested squares about the center, largest first, together painting what the
    // canvas painted.
    let mut s = session();
    gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), diamond_stops(), false);
    let before = render(&s);
    run(&mut s, "object.expand", json!({"steps": 120}));
    let g = selected(&s);
    let (_, squares) = clip_parts(&g);
    assert_eq!(squares.len(), 120);
    let corners = |n: &Node| n.path_data().unwrap().subpaths[0].anchors.iter().map(|a| a.p).collect::<Vec<_>>();
    let (outer, inner) = (corners(squares[0]), corners(squares[119]));
    assert_eq!((outer.len(), inner.len()), (4, 4));
    let reach = |pts: &[vectorcraft_geom::Point]| pts.iter().map(|p| p.distance(vectorcraft_geom::Point::new(100.0, 100.0))).fold(0.0, f64::max);
    assert!(reach(&outer) > reach(&inner) * 100.0, "largest first: {outer:?} then {inner:?}");
    let after = render(&s);
    assert!(differing(&before, &after, 48) < 200, "{}", differing(&before, &after, 48));
    assert!(mean_diff(&before, &after) < 2.0, "{}", mean_diff(&before, &after));
    // Translucent stops: each square is a ring with the next one cut out, so none doubles up.
    let mut s = session();
    let gr = json!({"kind": "diamond", "stops": [{"offset": 0, "color": "#ff0000", "opacity": 0.5}, {"offset": 1, "color": "#0000ff"}]});
    gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), gr, false);
    let before = render(&s);
    run(&mut s, "object.expand", json!({"steps": 60}));
    let g = selected(&s);
    let (_, squares) = clip_parts(&g);
    assert!(squares[..59].iter().all(|n| n.path_data().unwrap().subpaths.len() == 2), "rings");
    assert!(mean_diff(&before, &render(&s)) < 4.0, "{}", mean_diff(&before, &render(&s)));
    // Mesh: rings of patches about the center.
    let mut s = session();
    gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), diamond_stops(), false);
    let before = render(&s);
    run(&mut s, "object.expand", json!({"gradient": "mesh"}));
    let g = selected(&s);
    let (_, content) = clip_parts(&g);
    let NodeKind::Mesh(m) = &content[0].kind else { panic!("a mesh node: {:?}", content[0].kind) };
    assert!(m.is_valid());
    let after = render(&s);
    assert!(differing(&before, &after, 48) < 200, "{}", differing(&before, &after, 48));
    assert!(mean_diff(&before, &after) < 2.5, "{}", mean_diff(&before, &after));
}

/// A turned, squashed diamond, and one whose square is flat (as a file can hold it), expand as
/// the canvas draws them.
#[test]
fn a_turned_or_flat_diamond_expands_as_the_canvas_draws_it() {
    for aspect in [0.6, 0.0] {
        for how in [json!({"steps": 120}), json!({"gradient": "mesh"})] {
            let mut s = session();
            let id = gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), diamond_stops(), false);
            let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
            let n = d.node_mut(id).unwrap();
            let Paint::Gradient(mut g) = n.appearance.fill_paint().clone() else { panic!("a gradient fill") };
            let (start, end) = (vectorcraft_geom::Point::new(90.0, 110.0), vectorcraft_geom::Point::new(130.0, 80.0));
            g.geom = Some(vectorcraft_color::GradientGeom { start, end, aspect, focal: None });
            n.appearance.set_fill(Paint::Gradient(g));
            let before = render(&s);
            run(&mut s, "object.expand", how.clone());
            let after = render(&s);
            assert!(differing(&before, &after, 48) < 200, "{aspect} {how}: {}", differing(&before, &after, 48));
            assert!(mean_diff(&before, &after) < 2.5, "{aspect} {how}: {}", mean_diff(&before, &after));
        }
    }
}

/// A gradient that repeats or reflects expands, into objects or a mesh, as the canvas paints it:
/// its stops repeated across the box, for each kind.
#[test]
fn a_spread_gradient_expands_as_the_canvas_paints_it() {
    for kind in ["linear", "radial", "conic", "diamond"] {
        for spread in ["repeat", "reflect"] {
            for to in [json!({"steps": 240}), json!({"gradient": "mesh"})] {
                let mut s = session();
                let gr = json!({"kind": kind, "spread": spread, "repeats": 3, "stops": [{"offset": 0, "color": "#ffff00"}, {"offset": 1, "color": "#004080"}]});
                gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), gr, false);
                let before = render(&s);
                run(&mut s, "object.expand", to.clone());
                let after = render(&s);
                // Where Repeat starts the stops again, the hard edge may land a pixel off.
                assert!(mean_diff(&before, &after) < 4.0, "{kind} {spread} {to}: {}", mean_diff(&before, &after));
            }
        }
    }
}

/// A gradient that repeats or reflects expands into the same whole number of objects in each run,
/// so each run starts on an object's edge: a sharp step in the middle of each of 7 runs, expanded
/// into about 15 objects (2 a run), lands where the canvas paints it.
#[test]
fn a_spread_gradient_expands_into_whole_bands_for_each_run() {
    let stops = json!([
        {"offset": 0, "color": "#ffff00"}, {"offset": 0.5, "color": "#ffff00"},
        {"offset": 0.5, "color": "#004080"}, {"offset": 1, "color": "#004080"}
    ]);
    for kind in ["linear", "conic"] {
        for spread in ["repeat", "reflect"] {
            let mut s = session();
            // A conic gradient turns about the box's center.
            let start = if kind == "conic" { [100, 100] } else { [30, 100] };
            let gr = json!({"kind": kind, "spread": spread, "repeats": 7, "start": start, "end": [170, 100], "stops": stops});
            gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), gr, false);
            let before = render(&s);
            run(&mut s, "object.expand", json!({"steps": 15}));
            let after = render(&s);
            assert!(mean_diff(&before, &after) < 2.0, "{kind} {spread}: {}", mean_diff(&before, &after));
        }
    }
}

/// A gradient that repeats across a box in more runs than are written (700 runs of 0.2 pt) is not
/// expanded into its first runs and a flat last color: the fill stays a gradient, and the rest of
/// the object (its stroke) still expands. `object.expand.info` reports no gradient or fill to
/// expand, and Expand with only Fill on reports nothing to expand; the same gradient in 70 runs
/// (10 repeats) reports both.
#[test]
fn a_spread_gradient_with_more_runs_than_written_stays_a_gradient() {
    let gradient = |repeats: u32| {
        json!({"kind": "linear", "spread": "repeat", "repeats": repeats, "start": [30, 100], "end": [50, 100],
            "stops": [{"offset": 0, "color": "#ffff00"}, {"offset": 1, "color": "#004080"}]})
    };
    for (repeats, expands) in [(10, true), (100, false)] {
        let mut s = session();
        gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), gradient(repeats), true);
        let info = s.execute("object.expand.info", &json!({})).unwrap();
        assert_eq!(
            (info["gradient"].clone(), info["fill"].clone(), info["stroke"].clone()),
            (json!(expands), json!(expands), json!(true)),
            "{repeats}"
        );
        let only_fill = s.execute("object.expand", &json!({"object": false, "stroke": false}));
        assert_eq!(only_fill.is_ok(), expands, "{repeats}: {only_fill:?}");
    }
    for to in [json!({"steps": 240}), json!({"gradient": "mesh"})] {
        let mut s = session();
        gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), gradient(100), true);
        let before = render(&s);
        run(&mut s, "object.expand", to.clone());
        let after = render(&s);
        assert!(mean_diff(&before, &after) < 2.0, "{to}: {}", mean_diff(&before, &after));
        let mut gradients = 0;
        s.doc().unwrap().doc.walk(|n| gradients += usize::from(matches!(n.appearance.fill_paint(), Paint::Gradient(_))));
        assert_eq!(gradients, 1, "{to}");
    }
}

/// Selected type "Hi" at 48 pt, its characters filled with a gradient that repeats 100 times along
/// a vector `len` pt long.
fn spread_gradient_type(s: &mut Session, len: f64) -> u64 {
    let t = run(s, "text.create", json!({"x": 50, "y": 100, "text": "Hi", "size": 48}))["id"].as_u64().unwrap();
    run(s, "select.set", json!({"ids": [t]}));
    run(
        s,
        "paint.setFill",
        json!({"gradient": {"kind": "linear", "spread": "repeat", "repeats": 100, "start": [50, 100], "end": [50.0 + len, 100],
            "stops": [{"offset": 0, "color": "#ffff00"}, {"offset": 1, "color": "#004080"}]}}),
    );
    t
}

/// How many objects in the document have a gradient fill, and how many are type.
fn gradients_and_type(s: &Session) -> (usize, usize) {
    let (mut gradients, mut texts) = (0, 0);
    s.doc().unwrap().doc.walk(|n| {
        gradients += usize::from(matches!(n.appearance.fill_paint(), Paint::Gradient(_)));
        texts += usize::from(matches!(n.kind, NodeKind::Text(_)));
    });
    (gradients, texts)
}

/// Type is outlined before its fills expand, and each glyph's gradient fits that glyph's own box:
/// `object.expand.info` reports a gradient (and a fill) only when a glyph's box needs no more runs
/// than are written, so Expand turns it into art. Both Expand Gradient To options leave the
/// glyphs whose gradient needs more runs with their gradient, and Object still outlines the type.
/// The query leaves the document and its history as they are.
#[test]
fn type_whose_gradient_needs_more_runs_than_written_reports_no_gradient() {
    // 0.2 pt runs (100 repeats over 20 pt) across glyphs about 30 pt wide: under the limit.
    // 0.02 pt runs (over 2 pt): far over it.
    for (len, expands) in [(20.0, true), (2.0, false)] {
        let mut s = session();
        spread_gradient_type(&mut s, len);
        let before = (s.doc().unwrap().revision, s.doc().unwrap().history.undo.len(), s.doc().unwrap().doc.layers.clone());
        let info = run(&mut s, "object.expand.info", json!({}));
        assert_eq!((info["gradient"].clone(), info["fill"].clone(), info["object"].clone()), (json!(expands), json!(expands), json!(true)), "{len}");
        assert!(
            before == (s.doc().unwrap().revision, s.doc().unwrap().history.undo.len(), s.doc().unwrap().doc.layers.clone()),
            "{len}: the query changed the document"
        );
        for to in [json!({}), json!({"gradient": "mesh"})] {
            let mut s = session();
            spread_gradient_type(&mut s, len);
            run(&mut s, "object.expand", to.clone());
            let (gradients, texts) = gradients_and_type(&s);
            assert_eq!(texts, 0, "{len} {to}: outlined");
            assert_eq!(gradients == 0, expands, "{len} {to}: {gradients} gradient fills left");
        }
    }
}

/// One glyph whose box needs more runs than are written does not hide another that Expand turns
/// into art: the 200 pt "H" keeps its gradient, the 48 pt "i" expands, and the query reports it.
#[test]
fn type_reports_a_gradient_when_any_glyph_expands() {
    let mut s = session();
    let t = spread_gradient_type(&mut s, 20.0);
    run(&mut s, "text.setRangeStyle", json!({"id": t, "start": 0, "end": 1, "size": 200}));
    let info = run(&mut s, "object.expand.info", json!({}));
    assert_eq!((info["gradient"].clone(), info["fill"].clone()), (json!(true), json!(true)));
    run(&mut s, "object.expand", json!({}));
    assert_eq!(gradients_and_type(&s), (1, 0), "the large glyph keeps its gradient");
}

/// The same rule for type in the art of a symbol instance that Object expands. The symbol's art
/// is the type scaled down (its "i" about 2.5 pt wide), so its glyphs need more runs than are
/// written with 0.002 pt runs (over a 0.2 pt vector).
#[test]
fn type_in_symbol_art_reports_a_gradient_only_when_it_expands() {
    for (len, expands) in [(20.0, true), (0.2, false)] {
        let mut s = session();
        spread_gradient_type(&mut s, len);
        let inst = run(&mut s, "symbol.new", json!({"name": "Hi"}))["id"].as_u64().unwrap();
        run(&mut s, "select.set", json!({"ids": [inst]}));
        let info = run(&mut s, "object.expand.info", json!({}));
        assert_eq!(info["gradient"], json!(expands), "{len}");
        assert_eq!((info["symbolArt"]["gradient"].clone(), info["symbolArt"]["fill"].clone()), (json!(expands), json!(expands)), "{len}: {info}");
    }
}

/// Selected type "Hi" at 64 pt whose characters have no paint, with fills and strokes of its own:
/// each of `items` is an `appearance.setItem` edit of a new object-level stroke when it has a
/// `weight`, else of a new object-level fill; the first `below` of them paint below the characters.
fn type_with_own_paint(s: &mut Session, items: &[Value], below: usize) -> NodeId {
    let t = run(s, "text.create", json!({"x": 25, "y": 110, "text": "Hi", "size": 64}))["id"].as_u64().unwrap();
    run(s, "select.set", json!({"ids": [t]}));
    run(s, "paint.setFill", json!({"none": true}));
    run(s, "paint.setStroke", json!({"none": true}));
    for (i, item) in items.iter().enumerate() {
        let add = if item.get("weight").is_some() { "appearance.addStroke" } else { "appearance.addFill" };
        run(s, add, json!({"target": "object"}));
        let mut p = item.clone();
        p["index"] = json!(i);
        run(s, "appearance.setItem", p);
    }
    run(s, "appearance.moveItem", json!({"from": "contents", "to": below}));
    NodeId(t)
}

fn blue_stroke() -> Value {
    json!({"color": "#0000ff", "weight": 5})
}

fn red_blue() -> Value {
    json!({"gradient": {"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}})
}

/// Create Outlines draws what the type drew: a gradient fill of the type object's own, below its
/// characters, keeps one gradient across both glyphs over the layout's box, and its stroke stays
/// over them.
#[test]
fn create_outlines_draws_the_type_objects_own_paint() {
    let mut s = session();
    type_with_own_paint(&mut s, &[red_blue(), blue_stroke()], 1);
    let before = render(&s);
    run(&mut s, "type.createOutlines", json!({}));
    let after = render(&s);
    let n = differing(&before, &after, 24);
    assert!(n < 40, "{n} pixels differ");
    assert!(differing(&before, &render(&session()), 24) > 1000, "the type paints something");
}

/// `object.expand.info` reports the type object's own stroke, pattern fill and gradient fill, which
/// Object turns into paths of its whole outline: a gradient only when that outline's box needs no
/// more runs than are written.
#[test]
fn type_objects_own_stroke_pattern_and_gradient_are_reported() {
    let mut s = session();
    type_with_own_paint(&mut s, &[blue_stroke()], 0);
    let info = run(&mut s, "object.expand.info", json!({}));
    assert_eq!((info["object"].clone(), info["stroke"].clone(), info["fill"].clone()), (json!(true), json!(true), json!(false)), "{info}");

    let mut s = session();
    pattern_rect(&mut s, Rect::new(150.0, 150.0, 190.0, 190.0), false);
    let t = type_with_own_paint(&mut s, &[json!({"swatch": "Dots"})], 0);
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    let info = run(&mut s, "object.expand.info", json!({}));
    assert_eq!((info["fill"].clone(), info["gradient"].clone(), info["stroke"].clone()), (json!(true), json!(false), json!(false)), "{info}");

    // 0.4 pt runs across about 80 pt: under the limit; 0.02 pt runs: far over it.
    for (len, expands) in [(40.0, true), (2.0, false)] {
        let mut s = session();
        let gradient = json!({"gradient": {"kind": "linear", "spread": "repeat", "repeats": 100, "start": [25, 100], "end": [25.0 + len, 100],
            "stops": [{"offset": 0, "color": "#ffff00"}, {"offset": 1, "color": "#004080"}]}});
        type_with_own_paint(&mut s, &[gradient], 0);
        let info = run(&mut s, "object.expand.info", json!({}));
        assert_eq!((info["gradient"].clone(), info["fill"].clone()), (json!(expands), json!(expands)), "{len}: {info}");
        run(&mut s, "object.expand", json!({}));
        let (gradients, texts) = gradients_and_type(&s);
        assert_eq!(texts, 0, "{len}: outlined");
        assert_eq!(gradients == 0, expands, "{len}: {gradients} gradient fills left");
    }
}

/// Type with only spaces has no outline for its own stroke or pattern fill to go on, so
/// `object.expand.info` reports neither.
#[test]
fn type_without_an_outline_reports_no_own_stroke_or_fill() {
    let mut s = session();
    pattern_rect(&mut s, Rect::new(150.0, 150.0, 190.0, 190.0), false);
    let t = run(&mut s, "text.create", json!({"x": 25, "y": 110, "text": "   ", "size": 64}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [t]}));
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    for (i, item) in [json!({"swatch": "Dots"}), blue_stroke()].into_iter().enumerate() {
        let add = if item.get("weight").is_some() { "appearance.addStroke" } else { "appearance.addFill" };
        run(&mut s, add, json!({"target": "object"}));
        let mut p = item;
        p["index"] = json!(i);
        run(&mut s, "appearance.setItem", p);
    }
    let info = run(&mut s, "object.expand.info", json!({}));
    assert_eq!(
        (info["object"].clone(), info["stroke"].clone(), info["fill"].clone(), info["gradient"].clone()),
        (json!(true), json!(false), json!(false), json!(false)),
        "{info}"
    );
}

/// How many objects in the document have a visible stroke with paint and weight.
fn painted_strokes(s: &Session) -> usize {
    let mut n = 0;
    s.doc().unwrap().doc.walk(|c| {
        n += usize::from(
            c.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(st) if st.visible && !st.paint.is_none() && st.width > 0.0)),
        );
    });
    n
}

/// A visible stroke under a topmost stroke with no paint still has something for Stroke to
/// outline, on type's own appearance and on a path: `object.expand.info` reports it, and Expand
/// outlines it.
#[test]
fn a_stroke_under_an_unpainted_stroke_is_reported_and_outlined() {
    let mut s = session();
    type_with_own_paint(&mut s, &[blue_stroke(), json!({"none": true, "weight": 1})], 0);
    let mut r = session();
    run(&mut r, "shape.rectangle", json!({"x": 40, "y": 40, "width": 100, "height": 80}));
    run(&mut r, "paint.setStroke", json!({"color": "#0000ff"}));
    run(&mut r, "stroke.set", json!({"weight": 4}));
    run(&mut r, "appearance.addStroke", json!({"target": "object"}));
    let top = node(&r, selected(&r).id).appearance.items.len() - 1;
    run(&mut r, "appearance.setItem", json!({"index": top, "none": true}));
    for (what, s) in [("type", &mut s), ("path", &mut r)] {
        let info = run(s, "object.expand.info", json!({}));
        assert_eq!(info["stroke"], json!(true), "{what}: {info}");
        run(s, "object.expand", json!({}));
        assert_eq!(painted_strokes(s), 0, "{what}: the stroke is outlined");
    }
}

/// Expand keeps the type object's own stroke and gradient fill through Object: Stroke outlines the
/// stroke and Fill expands the gradient, and with those options off they stay live on the path of
/// the whole outline. The picture stays the same either way.
#[test]
fn expand_keeps_the_type_objects_own_paint() {
    for (opts, live) in [(json!({}), false), (json!({"fill": false, "stroke": false}), true)] {
        let mut s = session();
        type_with_own_paint(&mut s, &[red_blue(), blue_stroke()], 1);
        let before = render(&s);
        run(&mut s, "object.expand", opts.clone());
        let after = render(&s);
        assert!(mean_diff(&before, &after) < 2.0, "{opts}: {}", mean_diff(&before, &after));
        let (gradients, texts) = gradients_and_type(&s);
        assert_eq!(texts, 0, "{opts}: outlined");
        assert_eq!(gradients == 1, live, "{opts}: {gradients} gradient fills");
        let mut strokes = 0;
        s.doc().unwrap().doc.walk(|n| strokes += usize::from(!n.appearance.stroke_paint().is_none()));
        assert_eq!(strokes == 1, live, "{opts}: {strokes} strokes");
    }
}

#[test]
fn the_mesh_option_makes_a_mesh_that_paints_the_gradient() {
    for gradient in [
        json!({"kind": "linear", "angle": 30, "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 0.4, "color": "#00ff00", "midpoint": 0.3}, {"offset": 1, "color": "#0000ff"}]}),
        json!({"kind": "radial", "stops": [{"offset": 0, "color": "#ffff00"}, {"offset": 0.5, "color": "#ff00ff"}, {"offset": 1, "color": "#004080"}]}),
        // Perceptual: the mesh follows the OKLab curve between the stops, not a straight RGB mix.
        json!({"kind": "linear", "method": "perceptual", "stops": [{"offset": 0, "color": "#2050f0"}, {"offset": 1, "color": "#f5d020"}]}),
    ] {
        let mut s = session();
        gradient_rect(&mut s, Rect::new(30.0, 40.0, 170.0, 160.0), gradient.clone(), false);
        let before = render(&s);
        run(&mut s, "object.expand", json!({"gradient": "mesh"}));
        let g = selected(&s);
        let (_, content) = clip_parts(&g);
        assert_eq!(content.len(), 1);
        let NodeKind::Mesh(m) = &content[0].kind else { panic!("a mesh node: {:?}", content[0].kind) };
        assert!(m.is_valid());
        // The mesh is drawn as small flat pieces, so it is close but not identical.
        let after = render(&s);
        assert_eq!(differing(&before, &after, 48), 0, "{gradient}");
        assert!(mean_diff(&before, &after) < 2.5, "{gradient}: {}", mean_diff(&before, &after));
    }
}

#[test]
fn a_hard_stop_keeps_its_edge_in_the_mesh() {
    let mut s = session();
    let gr = json!({"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 0.5, "color": "#ff0000"}, {"offset": 0.5, "color": "#0000ff"}, {"offset": 1, "color": "#0000ff"}]});
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), gr, false);
    run(&mut s, "object.expand", json!({"gradient": "mesh"}));
    let g = selected(&s);
    let (_, content) = clip_parts(&g);
    let NodeKind::Mesh(m) = &content[0].kind else { panic!() };
    // Columns at 0, 0.5 (red), 0.5 (blue), 1: the middle patch has no width.
    assert_eq!(m.cols, 3);
    let (a, b) = (&m.points[m.idx(0, 1)], &m.points[m.idx(0, 2)]);
    assert!(a.p.distance(b.p) < 1e-9);
    assert_eq!((a.color.to_hex(), b.color.to_hex()), ("#ff0000".into(), "#0000ff".into()));
}

#[test]
fn a_freeform_gradient_expands_to_a_mesh_shaped_like_the_object() {
    let mut s = session();
    let gr = json!({"kind": "freeform", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 0.5, "color": "#00ff00"}, {"offset": 1, "color": "#0000ff"}]});
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), gr, false);
    let before = render(&s);
    // Even with Specify … Objects.
    run(&mut s, "object.expand", json!({"gradient": "objects"}));
    let n = selected(&s);
    let NodeKind::Mesh(m) = &n.kind else { panic!("a mesh: {:?}", n.kind) };
    assert!(m.rows > 1 && m.cols > 1);
    let b = m.outline().bounds().unwrap();
    assert!((b.x0 - 20.0).abs() < 1e-6 && (b.y0 - 20.0).abs() < 1e-6 && (b.x1 - 180.0).abs() < 1e-6 && (b.y1 - 180.0).abs() < 1e-6, "{b:?}");
    let after = render(&s);
    assert!(mean_diff(&before, &after) < 2.5, "{}", mean_diff(&before, &after));
}

#[test]
fn a_pattern_fill_becomes_its_tiles_clipped_by_the_object() {
    let mut s = session();
    pattern_rect(&mut s, Rect::new(25.0, 25.0, 85.0, 65.0), false);
    // Fill expands the pattern; Expand Gradient To has no gradient fill to apply to.
    assert_eq!(
        run(&mut s, "object.expand.info", json!({})),
        json!({"object": true, "fill": true, "stroke": false, "gradient": false, "appearance": false})
    );
    let before = render(&s);
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "object.expand", json!({}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let g = selected(&s);
    let (clip, tiles) = clip_parts(&g);
    assert!(matches!(clip.kind, NodeKind::Path { clipping: true, live: None, .. }));
    assert_eq!(clip.geometric_bounds().unwrap(), Rect::new(25.0, 25.0, 85.0, 65.0));
    // The 20 pt tile is centered on the square, so the squares sit at 5 + 20k; the tiles that reach
    // the rectangle are 4 across and 3 down.
    assert_eq!(tiles.len(), 12);
    for t in &tiles {
        let b = t.geometric_bounds().unwrap();
        assert!((b.x0 - 5.0) % 20.0 == 0.0 && (b.y0 - 5.0) % 20.0 == 0.0 && b.width() == 10.0 && b.height() == 10.0, "{b:?}");
        assert_eq!(fill_color(t).to_hex(), "#ff0000");
    }
    let mut ids = std::collections::HashSet::new();
    g.walk(&mut |n| assert!(ids.insert(n.id), "fresh ids: {:?}", n.id));
    assert_eq!(differing(&before, &render(&s), 2), 0, "looks the same");
    // The tiles are the pattern's art as it is (a stroked live rectangle that Object and Stroke can
    // expand).
    assert_eq!(
        run(&mut s, "object.expand.info", json!({})),
        json!({"object": true, "fill": false, "stroke": true, "gradient": false, "appearance": false})
    );
    run(&mut s, "edit.undo", json!({}));
    assert!(matches!(selected(&s).appearance.fill_paint(), Paint::Pattern { pattern, .. } if pattern == "Dots"));
    // Fill off: the pattern stays live.
    run(&mut s, "object.expand", json!({"fill": false}));
    assert!(matches!(selected(&s).appearance.fill_paint(), Paint::Pattern { .. }));
}

#[test]
fn the_tiles_of_a_scaled_pattern_keep_the_stroke_weight_the_canvas_draws() {
    let mut s = session();
    let id = pattern_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), false);
    run(&mut s, "pattern.transform", json!({"ids": [id.0], "matrix": [2, 0, 0, 2, 0, 0]}));
    let before = render(&s);
    run(&mut s, "object.expand", json!({}));
    let g = selected(&s);
    let (_, tiles) = clip_parts(&g);
    assert!(!tiles.is_empty());
    for t in &tiles {
        assert_eq!(t.geometric_bounds().unwrap().width(), 20.0);
        let Some(AppearanceItem::Stroke(st)) = t.appearance.items.iter().find(|i| matches!(i, AppearanceItem::Stroke(_))) else { panic!() };
        assert_eq!(st.width, 2.0, "the 1 pt stroke drawn at 200%");
    }
    assert!(mean_diff(&before, &render(&s)) < 1.0, "{}", mean_diff(&before, &render(&s)));
}

#[test]
fn the_tiles_of_a_stretched_pattern_draw_their_strokes_stretched() {
    let mut s = session();
    let id = pattern_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), false);
    // Twice as wide: the 1 pt stroke draws 2 pt wide on the sides and 1 pt on the top and bottom.
    run(&mut s, "pattern.transform", json!({"ids": [id.0], "matrix": [2, 0, 0, 1, 0, 0]}));
    let before = render(&s);
    run(&mut s, "object.expand", json!({}));
    let g = selected(&s);
    let (_, tiles) = clip_parts(&g);
    assert!(!tiles.is_empty());
    let mut strokes = 0;
    for t in &tiles {
        t.walk(&mut |n| strokes += n.appearance.items.iter().filter(|i| matches!(i, AppearanceItem::Stroke(_))).count());
    }
    assert_eq!(strokes, 0, "the strokes became filled outlines");
    assert_eq!(differing(&before, &render(&s), 2), 0, "looks the same");
}

#[test]
fn the_rounded_corners_of_a_stretched_pattern_stay_stretched() {
    let mut s = session();
    let tile = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 16, "height": 16, "radius": 6}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "object.pattern.make", json!({"ids": [tile], "name": "Round", "width": 20, "height": 20, "edit": false}));
    run(&mut s, "select.set", json!({"ids": [tile]}));
    run(&mut s, "edit.clear", json!({}));
    let id = run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 160, "height": 160}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"swatch": "Round"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "pattern.transform", json!({"ids": [id], "matrix": [3, 0, 0, 1, 0, 0]}));
    let before = render(&s);
    expand_to_tiles(&mut s);
    // The canvas draws the tile as an image stretched 3×, so the edges differ a little; round
    // corners would differ by far more.
    assert_eq!(differing(&before, &render(&s), 64), 0, "corners stretched as the canvas draws them");
}

#[test]
fn a_gradient_along_a_tile_stroke_follows_the_stroke_in_a_stretched_pattern() {
    let mut s = session();
    let tile = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 14, "height": 14}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "stroke.set", json!({"weight": 4}));
    run(&mut s, "paint.editGradient", json!({"stroke": true, "strokeMode": "along"}));
    run(&mut s, "object.pattern.make", json!({"ids": [tile], "name": "Along", "width": 20, "height": 20, "edit": false}));
    run(&mut s, "select.set", json!({"ids": [tile]}));
    run(&mut s, "edit.clear", json!({}));
    let id = run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 160, "height": 160}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"swatch": "Along"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "pattern.transform", json!({"ids": [id], "matrix": [2, 0, 0, 1, 0, 0]}));
    let before = render(&s);
    expand_to_tiles(&mut s);
    // The meshes are drawn as small flat pieces and the canvas stretches the tile image, so the
    // pictures are close but not identical; a gradient across the outline's box differs almost
    // everywhere on the strokes.
    let after = render(&s);
    assert!(mean_diff(&before, &after) < 2.0, "{}", mean_diff(&before, &after));
    assert!(differing(&before, &after, 64) < 500, "{}", differing(&before, &after, 64));
}

#[test]
fn a_brushed_tile_stroke_draws_its_brush_stretched() {
    let mut s = session();
    let tile = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 12, "height": 12}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "paint.setStroke", json!({"color": "#000000"}));
    run(&mut s, "brush.apply", json!({"name": "6 pt. Flat"}));
    run(&mut s, "object.pattern.make", json!({"ids": [tile], "name": "Brushed", "width": 20, "height": 20, "edit": false}));
    run(&mut s, "select.set", json!({"ids": [tile]}));
    run(&mut s, "edit.clear", json!({}));
    let id = run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 160, "height": 160}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"swatch": "Brushed"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "pattern.transform", json!({"ids": [id], "matrix": [2, 0, 0, 1, 0, 0]}));
    let before = render(&s);
    expand_to_tiles(&mut s);
    // The canvas stretches the tile image, so edges differ a little.
    assert_eq!(differing(&before, &render(&s), 64), 0, "the brush draws stretched as on the canvas");
    let brushed = |i: &AppearanceItem| matches!(i, AppearanceItem::Stroke(st) if st.brush.is_some());
    let mut left = 0;
    selected(&s).walk(&mut |n| left += n.appearance.items.iter().filter(|i| brushed(i)).count());
    assert_eq!(left, 0, "the brush strokes became brush art");
}

#[test]
fn a_flat_brush_in_a_rotated_pattern_turns_with_it() {
    let mut s = session();
    let tile = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 12, "height": 12}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "paint.setStroke", json!({"color": "#000000"}));
    run(&mut s, "brush.apply", json!({"name": "6 pt. Flat"}));
    run(&mut s, "object.pattern.make", json!({"ids": [tile], "name": "Flat", "width": 20, "height": 20, "edit": false}));
    run(&mut s, "select.set", json!({"ids": [tile]}));
    run(&mut s, "edit.clear", json!({}));
    let id = run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 160, "height": 160}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"swatch": "Flat"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    // A quarter turn: the nib's thick and thin sides swap with the tile.
    run(&mut s, "pattern.transform", json!({"ids": [id], "matrix": [0, 1, -1, 0, 0, 0]}));
    let before = render(&s);
    expand_to_tiles(&mut s);
    assert_eq!(differing(&before, &render(&s), 64), 0, "the brush turns with the tiles as on the canvas");
}

#[test]
fn an_unlinked_opacity_mask_in_the_tile_art_moves_with_each_tile() {
    let mut s = session();
    let red = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    // A white mask over the square's left half, unlinked from it.
    let mask = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 10}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"color": "#ffffff"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "transparency.makeOpacityMask", json!({"ids": [red, mask]}));
    run(&mut s, "transparency.unlinkOpacityMask", json!({"id": red}));
    run(&mut s, "object.pattern.make", json!({"ids": [red], "name": "Half", "width": 20, "height": 20, "edit": false}));
    run(&mut s, "select.set", json!({"ids": [red]}));
    run(&mut s, "edit.clear", json!({}));
    run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 160, "height": 160}));
    run(&mut s, "paint.setFill", json!({"swatch": "Half"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let before = render(&s);
    expand_to_tiles(&mut s);
    assert_eq!(differing(&before, &render(&s), 2), 0, "each tile keeps its half mask");
}

#[test]
fn a_rounded_mask_in_a_stretched_pattern_stays_stretched() {
    let mut s = session();
    let red = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 16, "height": 16}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    // A white rounded square masks the red one.
    let mask = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 16, "height": 16, "radius": 6}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"color": "#ffffff"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "transparency.makeOpacityMask", json!({"ids": [red, mask]}));
    run(&mut s, "object.pattern.make", json!({"ids": [red], "name": "Masked", "width": 20, "height": 20, "edit": false}));
    run(&mut s, "select.set", json!({"ids": [red]}));
    run(&mut s, "edit.clear", json!({}));
    let id = run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 160, "height": 160}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"swatch": "Masked"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "pattern.transform", json!({"ids": [id], "matrix": [3, 0, 0, 1, 0, 0]}));
    let before = render(&s);
    expand_to_tiles(&mut s);
    // As for rounded tiles: the canvas stretches the tile image, so edges differ a little.
    assert_eq!(differing(&before, &render(&s), 64), 0, "the mask's corners stretched as on the canvas");
}

#[test]
fn a_turned_pattern_whose_tiles_have_effects_it_cannot_bake_stays_live() {
    // A drop shadow on the square, and an Offset Path on a group around it: the canvas draws both
    // turned with each tile.
    for group in [false, true] {
        let mut s = session();
        let id = pattern_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), false);
        run(&mut s, "pattern.transform", json!({"ids": [id.0], "matrix": [0, 1, -1, 0, 0, 0]}));
        let square = s.doc().unwrap().doc.pattern("Dots").unwrap().art[0].clone();
        let art = if group {
            let mut g = Node::group(NodeId(0), vec![square]);
            g.appearance.effects.push(vectorcraft_render::effects::new_effect("path.offsetPath", &json!({"offset": 2})).unwrap());
            g
        } else {
            let mut a = (*square).clone();
            a.appearance.effects.push(vectorcraft_render::effects::new_effect("stylize.dropShadow", &json!({"x": 4, "y": 0})).unwrap());
            a
        };
        s.edit("Effect", |d, _| {
            d.pattern_mut("Dots").unwrap().art = vec![std::sync::Arc::new(art)];
            Ok(())
        })
        .unwrap();
        assert!(s.execute("object.expand", &json!({"object": false, "stroke": false})).is_err(), "group {group}: nothing to expand");
        assert!(matches!(node(&s, id).appearance.fill_paint(), Paint::Pattern { .. }));
    }
}

#[test]
fn a_random_transform_in_the_tile_art_draws_every_tile_alike() {
    // The canvas draws the pattern's art once, so each tile shows the same random move and turn:
    // on the square as the pattern is, then turned a quarter, and on a group around the square,
    // which Expand cannot bake and leaves live.
    for (turned, group) in [(false, false), (true, false), (false, true)] {
        let mut s = session();
        let id = pattern_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), false);
        let square = s.doc().unwrap().doc.pattern("Dots").unwrap().art[0].clone();
        let mut art = if group { Node::group(NodeId(0), vec![square]) } else { (*square).clone() };
        let fx = json!({"moveH": 4, "rotate": 40, "random": true});
        art.appearance.effects.push(vectorcraft_render::effects::new_effect("distort.transform", &fx).unwrap());
        s.edit("Effect", |d, _| {
            d.pattern_mut("Dots").unwrap().art = vec![std::sync::Arc::new(art)];
            Ok(())
        })
        .unwrap();
        if turned {
            run(&mut s, "pattern.transform", json!({"ids": [id.0], "matrix": [0, 1, -1, 0, 0, 0]}));
        }
        if group {
            assert!(s.execute("object.expand", &json!({"object": false, "stroke": false})).is_err(), "nothing to expand");
            assert!(matches!(node(&s, id).appearance.fill_paint(), Paint::Pattern { .. }));
            continue;
        }
        let before = render(&s);
        expand_to_tiles(&mut s);
        // The turned squares' edges differ a little.
        assert_eq!(differing(&before, &render(&s), 64), 0, "turned {turned}: every tile as on the canvas");
    }
}

#[test]
fn a_random_brush_in_the_tile_art_draws_every_tile_alike() {
    // Confetti scatters its art at random from the path's points; the canvas draws the pattern's
    // art once, so each tile shows the same confetti, as the pattern is and turned a quarter.
    for turned in [false, true] {
        let mut s = session();
        let tile = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 12, "height": 12}))["id"].as_u64().unwrap();
        run(&mut s, "paint.setFill", json!({"none": true}));
        run(&mut s, "paint.setStroke", json!({"color": "#000000"}));
        run(&mut s, "brush.apply", json!({"name": "Confetti"}));
        run(&mut s, "object.pattern.make", json!({"ids": [tile], "name": "Confetti", "width": 60, "height": 60, "edit": false}));
        run(&mut s, "select.set", json!({"ids": [tile]}));
        run(&mut s, "edit.clear", json!({}));
        let id = run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 160, "height": 160}))["id"].as_u64().unwrap();
        run(&mut s, "paint.setFill", json!({"swatch": "Confetti"}));
        run(&mut s, "paint.setStroke", json!({"none": true}));
        if turned {
            run(&mut s, "pattern.transform", json!({"ids": [id], "matrix": [0, 1, -1, 0, 0, 0]}));
        }
        let before = render(&s);
        expand_to_tiles(&mut s);
        assert_eq!(differing(&before, &render(&s), 64), 0, "turned {turned}: every tile as on the canvas");
    }
}

#[test]
fn roughen_in_the_tile_art_of_a_scaled_pattern_keeps_its_detail() {
    // Roughen's detail is per inch: the canvas roughens the 10 pt square and scales the tile, so
    // the expanded square has as many points as the canvas draws, not twice as many.
    let mut s = session();
    let id = pattern_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), false);
    let mut art = (*s.doc().unwrap().doc.pattern("Dots").unwrap().art[0]).clone();
    let fx = json!({"size": 2, "relative": false, "detail": 20, "points": "corner", "seed": 3});
    art.appearance.effects.push(vectorcraft_render::effects::new_effect("distort.roughen", &fx).unwrap());
    s.edit("Effect", |d, _| {
        d.pattern_mut("Dots").unwrap().art = vec![std::sync::Arc::new(art)];
        Ok(())
    })
    .unwrap();
    run(&mut s, "pattern.transform", json!({"ids": [id.0], "matrix": [2, 0, 0, 2, 0, 0]}));
    let before = render(&s);
    expand_to_tiles(&mut s);
    let mut live = 0;
    selected(&s).walk(&mut |n| live += n.appearance.effects.len());
    assert_eq!(live, 0, "Roughen became filled art");
    // The canvas scales the tile image 2×, so edges differ a little.
    let after = render(&s);
    assert!(mean_diff(&before, &after) < 1.0 && differing(&before, &after, 64) < 100, "every tile as on the canvas");
}

#[test]
fn a_scaled_pattern_whose_tiles_hold_a_symbol_stays_live() {
    // The canvas draws the tile scaled, the symbol's 1 pt stroke included; a scaled instance keeps
    // its stroke weight. A pattern that only moves expands.
    for scaled in [false, true] {
        let mut s = session();
        run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
        run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
        run(&mut s, "paint.setStroke", json!({"color": "#000000"}));
        run(&mut s, "stroke.set", json!({"weight": 1}));
        let inst = run(&mut s, "symbol.new", json!({"name": "Box"}))["id"].as_u64().unwrap();
        run(&mut s, "object.pattern.make", json!({"ids": [inst], "name": "Boxes", "width": 20, "height": 20, "edit": false}));
        run(&mut s, "select.set", json!({"ids": [inst]}));
        run(&mut s, "edit.clear", json!({}));
        let id = run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 160, "height": 160}))["id"].as_u64().unwrap();
        run(&mut s, "paint.setFill", json!({"swatch": "Boxes"}));
        run(&mut s, "paint.setStroke", json!({"none": true}));
        if scaled {
            run(&mut s, "pattern.transform", json!({"ids": [id], "matrix": [2, 0, 0, 2, 0, 0]}));
            assert!(s.execute("object.expand", &json!({"object": false, "stroke": false})).is_err(), "nothing to expand");
            assert!(matches!(node(&s, NodeId(id)).appearance.fill_paint(), Paint::Pattern { .. }));
            continue;
        }
        let before = render(&s);
        expand_to_tiles(&mut s);
        assert_eq!(differing(&before, &render(&s), 2), 0, "looks the same");
    }
}

#[test]
fn a_pattern_inside_a_stretched_pattern_moves_with_each_tile() {
    let mut s = session();
    // "Inner": a 4 pt red square in an 8 pt tile; "Outer": a 10 pt square filled with Inner, in a
    // 20 pt tile, so each Outer tile starts 4 pt into Inner's period from the one before it.
    let dot = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 4, "height": 4}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "object.pattern.make", json!({"ids": [dot], "name": "Inner", "width": 8, "height": 8, "edit": false}));
    let sq = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"swatch": "Inner"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "object.pattern.make", json!({"ids": [sq], "name": "Outer", "width": 20, "height": 20, "edit": false}));
    run(&mut s, "select.set", json!({"ids": [dot, sq]}));
    run(&mut s, "edit.clear", json!({}));
    let id = run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 160, "height": 160}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"swatch": "Outer"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "pattern.transform", json!({"ids": [id], "matrix": [2, 0, 0, 1, 0, 0]}));
    let before = render(&s);
    expand_to_tiles(&mut s);
    assert_eq!(differing(&before, &render(&s), 2), 0, "looks the same");
}

#[test]
fn a_fill_effect_stays_on_the_expanded_fill() {
    let has_effect = |n: &Node| {
        let mut found = false;
        n.walk(&mut |c| found |= matches!(c.kind, NodeKind::Group { clip: true, .. }) && !c.appearance.effects.is_empty());
        found
    };
    // A pattern fill and a gradient fill, each the object's only item, with a drop shadow.
    let kept = [true, false].map(|pattern| {
        let mut s = session();
        if pattern {
            pattern_rect(&mut s, Rect::new(25.0, 25.0, 85.0, 65.0), false);
        } else {
            gradient_rect(&mut s, Rect::new(25.0, 25.0, 85.0, 65.0), white_black(), false);
        }
        run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "item": 0}));
        run(&mut s, "object.expand", json!({"steps": 4}));
        has_effect(&selected(&s))
    });
    assert_eq!(kept, [true, true], "the fill's drop shadow is on its clip group");
}

#[test]
fn a_geometry_effect_shapes_the_clip_of_an_expanded_pattern_and_not_its_tiles() {
    // Offset Path on the pattern fill, then on the object: either grows the area the fill paints by
    // 6 pt on each side, and the canvas lays the tiles over that area unchanged.
    for item in [true, false] {
        let mut s = session();
        pattern_rect(&mut s, Rect::new(40.0, 40.0, 160.0, 160.0), false);
        let mut p = json!({"effect": "path.offsetPath", "params": {"offset": 6}});
        if item {
            p["item"] = json!(0);
        }
        run(&mut s, "effect.apply", p);
        let before = render(&s);
        run(&mut s, "object.expand", json!({"object": false, "stroke": false}));
        let g = selected(&s);
        assert!(g.appearance.effects.is_empty(), "item {item}: the offset is not left on the tiles: {:?}", g.appearance.effects);
        let (clip, tiles) = clip_parts(&g);
        let b = clip.geometric_bounds().unwrap();
        assert!((b.x0 - 34.0).abs() < 0.01 && (b.x1 - 166.0).abs() < 0.01, "item {item}: the clip is the offset outline: {b:?}");
        assert!(tiles.iter().all(|t| t.geometric_bounds().is_some_and(|b| (b.width() - 10.0).abs() < 0.01)), "item {item}: tiles keep their size");
        assert_eq!(differing(&before, &render(&s), 2), 0, "item {item}: looks the same");
    }
}

#[test]
fn a_geometry_effect_shapes_the_clip_of_an_expanded_gradient_and_not_its_strips() {
    // As for patterns: the canvas fits the gradient to the offset outline and fills it.
    for item in [true, false] {
        let mut s = session();
        gradient_rect(&mut s, Rect::new(40.0, 40.0, 160.0, 160.0), white_black(), false);
        let mut p = json!({"effect": "path.offsetPath", "params": {"offset": 6}});
        if item {
            p["item"] = json!(0);
        }
        run(&mut s, "effect.apply", p);
        let before = render(&s);
        run(&mut s, "object.expand", json!({"object": false, "stroke": false}));
        let g = selected(&s);
        assert!(g.appearance.effects.is_empty(), "item {item}: the offset is not left on the strips: {:?}", g.appearance.effects);
        let b = clip_parts(&g).0.geometric_bounds().unwrap();
        assert!((b.x0 - 34.0).abs() < 0.01 && (b.x1 - 166.0).abs() < 0.01, "item {item}: the clip is the offset outline: {b:?}");
        assert_eq!(differing(&before, &render(&s), 2), 0, "item {item}: looks the same");
    }
}

#[test]
fn tiles_that_blend_blend_only_with_each_other() {
    let mut s = session();
    // A blue backdrop under a rectangle filled with a pattern whose square multiplies.
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 200}));
    run(&mut s, "paint.setFill", json!({"color": "#0000ff"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let dot = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "transparency.set", json!({"blend": "multiply"}));
    run(&mut s, "object.pattern.make", json!({"ids": [dot], "name": "Multiply", "width": 20, "height": 20, "edit": false}));
    run(&mut s, "select.set", json!({"ids": [dot]}));
    run(&mut s, "edit.clear", json!({}));
    run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 160, "height": 160}));
    run(&mut s, "paint.setFill", json!({"swatch": "Multiply"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let before = render(&s);
    run(&mut s, "object.expand", json!({}));
    assert_eq!(differing(&before, &render(&s), 2), 0, "looks the same");
    assert!(selected(&s).isolate, "the tiles' clip group isolates their blending");
}

#[test]
fn a_pattern_fill_that_lays_no_tiles_stays_live() {
    // A 0.1 pt tile over a 160 pt square needs more instances than a pattern lays.
    let mut s = session();
    let id = pattern_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), false);
    run(&mut s, "pattern.options", json!({"name": "Dots", "width": 0.1, "height": 0.1}));
    // Fill alone: nothing to expand, and the live rectangle is left as it was.
    assert!(s.execute("object.expand", &json!({"object": false, "stroke": false})).is_err());
    assert!(matches!(node(&s, id).kind, NodeKind::Path { live: Some(_), .. }));
    run(&mut s, "object.expand", json!({}));
    let n = selected(&s);
    assert!(matches!(n.kind, NodeKind::Path { live: None, .. }), "Object still expands the rectangle");
    assert!(matches!(n.appearance.fill_paint(), Paint::Pattern { .. }), "the fill is kept");
    // A pattern scaled by 1e-20 covers the square with more tile positions than can be counted.
    let mut s = session();
    let id = pattern_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), false);
    run(&mut s, "pattern.transform", json!({"ids": [id.0], "matrix": [1e-20, 0, 0, 1e-20, 0, 0]}));
    assert!(s.execute("object.expand", &json!({"object": false, "stroke": false})).is_err());
    assert!(matches!(node(&s, id).appearance.fill_paint(), Paint::Pattern { .. }), "the fill is kept");
}

#[test]
fn type_filled_with_a_pattern_expands_to_outlines_holding_the_tiles() {
    let mut s = session();
    pattern_rect(&mut s, Rect::new(25.0, 25.0, 85.0, 65.0), false);
    run(&mut s, "edit.clear", json!({}));
    let t = run(&mut s, "text.create", json!({"x": 50, "y": 100, "text": "Hi", "size": 48}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [t]}));
    run(&mut s, "paint.setFill", json!({"swatch": "Dots"}));
    assert_eq!(run(&mut s, "object.expand.info", json!({}))["fill"], json!(true));
    run(&mut s, "object.expand", json!({}));
    let (mut clips, mut patterns) = (0, 0);
    for id in s.doc().unwrap().selection.objects.clone() {
        node(&s, id).walk(&mut |n| {
            clips += usize::from(matches!(n.kind, NodeKind::Group { clip: true, .. }));
            patterns += n.appearance.items.iter().filter(|i| matches!(i.paint(), Paint::Pattern { .. })).count();
        });
    }
    assert!(clips > 0 && patterns == 0, "{clips} clip groups, {patterns} pattern paints left");
}

#[test]
fn an_expanded_pattern_fill_keeps_its_transparency_and_the_stroke_on_top() {
    let mut s = session();
    pattern_rect(&mut s, Rect::new(25.0, 25.0, 85.0, 65.0), true);
    // [Fill (pattern), Stroke, Fill (green, on top)], the pattern fill at 50% in Multiply.
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "paint.setFill", json!({"item": 2, "color": "#00ff00"}));
    run(&mut s, "transparency.set", json!({"item": 2, "opacity": 30}));
    run(&mut s, "transparency.set", json!({"item": 0, "opacity": 50, "blend": "multiply"}));
    let before = render(&s);
    run(&mut s, "object.expand", json!({"stroke": false}));
    let g = selected(&s);
    let NodeKind::Group { children, clip: false } = &g.kind else { panic!("{:?}", g.kind) };
    assert_eq!(children.len(), 2);
    assert_eq!((children[0].opacity, children[0].blend), (0.5, BlendMode::Multiply));
    assert_eq!(clip_parts(&children[0]).1.len(), 12);
    // The stroke and the top fill stay on a copy above, in paint order.
    let items = &children[1].appearance.items;
    assert_eq!(items.len(), 2);
    assert_eq!(children[1].appearance.stroke_paint().color().unwrap().to_hex(), "#0000ff");
    assert!(matches!(&items[1], AppearanceItem::Fill(f) if f.paint.color().unwrap().to_hex() == "#00ff00"));
    assert_eq!(differing(&before, &render(&s), 2), 0, "looks the same");
    // Stroke on as well: the stroke becomes a filled outline over the tiles.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "object.expand", json!({}));
    let g = selected(&s);
    let NodeKind::Group { children, clip: false } = &g.kind else { panic!("{:?}", g.kind) };
    assert_eq!(clip_parts(&children[0]).1.len(), 12);
    assert_eq!(fill_color(&children[1]).to_hex(), "#0000ff", "the stroke became a filled outline");
}

#[test]
fn a_gradient_that_cannot_be_expanded_leaves_the_object_as_it_was() {
    // A freeform gradient on a path of one anchor has no outline to shape a mesh to.
    let mut s = session();
    let id = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 50, "y": 50}]}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "paint.setFill", json!({"gradient": {"kind": "freeform"}}));
    let before = node(&s, id);
    assert!(s.execute("object.expand", &json!({"object": false, "stroke": false})).is_err(), "nothing to expand");
    assert_eq!(node(&s, id), before);
}

#[test]
fn create_gradient_mesh_takes_its_colours_from_the_gradient() {
    let mut s = session();
    let id = gradient_rect(&mut s, Rect::new(0.0, 0.0, 100.0, 50.0), white_black(), false);
    run(&mut s, "object.mesh.create", json!({"rows": 1, "cols": 2}));
    let NodeKind::Mesh(m) = &node(&s, id).kind else { panic!() };
    let hex = |r: usize, c: usize| m.points[m.idx(r, c)].color.to_hex();
    for r in 0..=1 {
        assert_eq!(hex(r, 0), "#ffffff", "left column white");
        assert_eq!(hex(r, 2), "#000000", "right column black");
        assert_eq!(hex(r, 1), "#808080", "middle grey");
    }
    // A solid fill still gives one colour, the highlight still lightens.
    let mut s = session();
    let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "object.mesh.create", json!({"rows": 2, "cols": 2, "appearance": "center", "highlight": 100}));
    let NodeKind::Mesh(m) = &node(&s, id).kind else { panic!() };
    assert_eq!(m.points[m.idx(0, 0)].color.to_hex(), "#ff0000");
    assert_eq!(m.points[m.idx(1, 1)].color.to_hex(), "#ffffff");
}

#[test]
fn create_gradient_mesh_keeps_the_inks_of_freeform_points() {
    let mut s = session();
    let one = json!({"c": 0.5, "m": 0.2, "y": 0.0, "k": 0.1});
    let id = freeform_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), [one.clone(), one.clone(), one.clone(), one]);
    run(&mut s, "object.mesh.create", json!({"rows": 2, "cols": 2}));
    let NodeKind::Mesh(m) = &node(&s, id).kind else { panic!() };
    for p in &m.points {
        assert!(close_inks(inks(p.color), [0.5, 0.2, 0.0, 0.1]), "{:?}", p.color);
    }
}

#[test]
fn expanded_colours_keep_the_stops_colour_model() {
    let mut s = session();
    let gr = json!({"kind": "linear", "stops": [{"offset": 0, "color": {"c": 1, "m": 0, "y": 0, "k": 0}}, {"offset": 1, "color": {"c": 0, "m": 1, "y": 0, "k": 0}}]});
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 120.0), gr, false);
    run(&mut s, "object.expand", json!({"steps": 4}));
    let g = selected(&s);
    let (_, strips) = clip_parts(&g);
    let Color::Cmyk { c, m, .. } = fill_color(strips[1]) else { panic!("CMYK strip expected: {:?}", fill_color(strips[1])) };
    assert!((c - 0.625).abs() < 1e-5 && (m - 0.375).abs() < 1e-5, "interpolated in CMYK: {c} {m}");
}

/// A new 200 × 200 CMYK document.
fn cmyk_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200, "colorMode": "cmyk"})).unwrap();
    s
}

/// The selection's freeform gradient expanded: the mesh's colors, after checking that it draws
/// within `most` (mean channel difference) of the gradient.
fn expanded(s: &mut Session, most: f64) -> Vec<Color> {
    let before = render(s);
    run(s, "object.expand", json!({}));
    let diff = mean_diff(&before, &render(s));
    assert!(diff < most, "looks the same: {diff}");
    let NodeKind::Mesh(m) = selected(s).kind else { panic!("a mesh: {:?}", selected(s).kind) };
    m.points.into_iter().map(|p| p.color).collect()
}

/// Cyan and black points mix as inks: cyan and black only, adding up to one. The gradient and the
/// mesh both mix inks, so the two draw alike.
fn cyan_and_black_points_expand_as_they_draw(mut s: Session) {
    let (cyan, black) = (json!({"c": 1.0, "m": 0.0, "y": 0.0, "k": 0.0}), json!({"c": 0.0, "m": 0.0, "y": 0.0, "k": 1.0}));
    freeform_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), [cyan.clone(), cyan, black.clone(), black]);
    for c in expanded(&mut s, 2.5) {
        let [c, m, y, k] = inks(c);
        assert!(m.abs() < 1e-5 && y.abs() < 1e-5 && (c + k - 1.0).abs() < 1e-5, "C{c} M{m} Y{y} K{k}");
    }
}

#[test]
fn cyan_and_black_freeform_points_expand_as_they_draw_in_an_rgb_document() {
    cyan_and_black_points_expand_as_they_draw(session());
}

#[test]
fn cyan_and_black_freeform_points_expand_as_they_draw_in_a_cmyk_document() {
    cyan_and_black_points_expand_as_they_draw(cmyk_session());
}

#[test]
fn a_freeform_gradient_expands_with_the_inks_of_its_points() {
    let ink = |c: f32, m: f32, y: f32, k: f32| json!({"c": c, "m": m, "y": y, "k": k});
    // Points of one ink set give every mesh point that ink set.
    let mut s = session();
    let one = ink(0.5, 0.2, 0.0, 0.1);
    freeform_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), [one.clone(), one.clone(), one.clone(), one]);
    for c in expanded(&mut s, 1.0) {
        assert!(close_inks(inks(c), [0.5, 0.2, 0.0, 0.1]), "{c:?}");
    }
    // Points in different models mix in display RGB, stored in a CMYK document as CMYK.
    let (cyan, gray) = (ink(1.0, 0.0, 0.0, 0.0), json!({"gray": 0.5}));
    let mut s = cmyk_session();
    freeform_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), [cyan.clone(), cyan, gray.clone(), gray]);
    for c in expanded(&mut s, 2.5) {
        inks(c);
    }
}

#[test]
fn fill_off_leaves_the_fill_live_and_stroke_off_keeps_the_stroke() {
    // Fill unchecked: the stroke is outlined, the gradient fill stays a gradient.
    let mut s = session();
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 120.0), white_black(), true);
    run(&mut s, "object.expand", json!({"fill": false}));
    let g = selected(&s);
    let NodeKind::Group { children, clip: false } = &g.kind else { panic!("fill + outlined stroke group: {:?}", g.kind) };
    assert_eq!(children.len(), 2);
    assert!(matches!(children[0].appearance.fill_paint(), Paint::Gradient(_)), "the fill is still live");
    assert_eq!(fill_color(&children[1]).to_hex(), "#0000ff", "the stroke became a filled outline");
    // Stroke unchecked: the gradient is expanded and the stroke stays a stroke on a copy above.
    let mut s = session();
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 120.0), white_black(), true);
    let before = render(&s);
    run(&mut s, "object.expand", json!({"stroke": false, "steps": 255}));
    let g = selected(&s);
    let NodeKind::Group { children, clip: false } = &g.kind else { panic!("{:?}", g.kind) };
    assert_eq!(clip_parts(&children[0]).1.len(), 255);
    assert_eq!(children[1].appearance.stroke_paint().color().unwrap().to_hex(), "#0000ff");
    assert!(children[1].appearance.fill_paint().is_none());
    assert_eq!(differing(&before, &render(&s), 2), 0, "looks the same");
}

#[test]
fn defaults_match_the_dialog_and_info_tells_what_applies() {
    let mut s = session();
    gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 120.0), white_black(), true);
    assert_eq!(
        run(&mut s, "object.expand.info", json!({})),
        json!({"object": true, "fill": true, "stroke": true, "gradient": true, "appearance": false})
    );
    let r = run(&mut s, "object.expand", json!({}));
    assert_eq!(r["ids"].as_array().unwrap().len(), 1);
    // Object, Fill and Stroke on; 255 objects: the live rectangle, its stroke and its gradient.
    let g = selected(&s);
    let NodeKind::Group { children, .. } = &g.kind else { panic!("{:?}", g.kind) };
    let (clip, strips) = clip_parts(&children[0]);
    assert!(matches!(clip.kind, NodeKind::Path { live: None, .. }));
    assert_eq!(strips.len(), 255);
    assert!(children[1].appearance.stroke_paint().is_none(), "outlined stroke");
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Expand");
    assert_eq!(
        run(&mut s, "object.expand.info", json!({})),
        json!({"object": false, "fill": false, "stroke": false, "gradient": false, "appearance": false})
    );
    assert!(s.execute("object.expand", &json!({})).is_err(), "nothing left to expand");
    // Bad options are refused before anything changes.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.execute("object.expand", &json!({"gradient": "rings"})).is_err());
    assert!(s.execute("object.expand", &json!({"steps": 0})).is_err());
    assert!(matches!(selected(&s).kind, NodeKind::Path { live: Some(_), .. }));
}

/// Expand refuses an option of the wrong type with an error naming it, and leaves the art, the
/// selection and the undo history as they were. Null counts as an option left out, which keeps its
/// default.
#[test]
fn expand_refuses_an_option_of_the_wrong_type() {
    let mut s = session();
    let id = gradient_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 120.0), white_black(), true);
    let state = |s: &Session| {
        let st = s.doc().unwrap();
        (st.revision, st.history.undo.len(), st.history.redo.len(), st.selection.objects.clone())
    };
    let before = (state(&s), node(&s, id));
    let switch = |k: &str| format!("`{k}` must be true or false");
    for (p, want) in [
        (json!({"object": "yes"}), switch("object")),
        (json!({"object": 0}), switch("object")),
        (json!({"fill": "false"}), switch("fill")),
        (json!({"stroke": 1}), switch("stroke")),
        (json!({"gradient": 3}), "`gradient` must be a string".to_string()),
        (json!({"gradient": true}), "`gradient` must be a string".to_string()),
        (json!({"steps": "16"}), "`steps` must be a number, not \"16\"".to_string()),
        (json!({"steps": [16]}), "`steps` must be a number, not [16]".to_string()),
    ] {
        let r = s.execute("object.expand", &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{p}: {r:?}");
        assert_eq!((state(&s), node(&s, id)), before, "{p}");
    }
    // Null options take their defaults: Object, Fill and Stroke on, 255 solid strips.
    run(&mut s, "object.expand", json!({"object": null, "fill": null, "stroke": null, "gradient": null, "steps": null}));
    let g = selected(&s);
    let NodeKind::Group { children, .. } = &g.kind else { panic!("{:?}", g.kind) };
    let (clip, strips) = clip_parts(&children[0]);
    assert!(matches!(clip.kind, NodeKind::Path { live: None, .. }));
    assert_eq!(strips.len(), 255);
    assert!(children[1].appearance.stroke_paint().is_none(), "outlined stroke");
}

/// A selected open line with a 2 pt black stroke, no fill and a Zig Zag effect (#3089).
fn zig_zag_line(s: &mut Session) -> NodeId {
    let id = NodeId(run(s, "path.create", json!({"d": "M 20 100 L 180 100"}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"none": true}));
    run(s, "paint.setStroke", json!({"color": "#000000"}));
    run(s, "stroke.set", json!({"weight": 2}));
    run(s, "effect.apply", json!({"effect": "distort.zigZag", "params": {"size": 10, "ridges": 6, "relative": false}}));
    id
}

#[test]
fn expand_info_says_when_expand_appearance_comes_first() {
    let mut s = session();
    zig_zag_line(&mut s);
    assert_eq!(run(&mut s, "object.expand.info", json!({}))["appearance"], json!(true), "Zig Zag is an appearance attribute");
    // Expand Appearance bakes the zig-zag into the line, which keeps its stroke and gets no fill.
    run(&mut s, "effect.expandAppearance", json!({}));
    let line = selected(&s);
    assert!(matches!(line.kind, NodeKind::Path { .. }), "a path: {:?}", line.kind);
    assert!(line.path_data().unwrap().anchors().count() > 6, "the zig-zag is baked");
    assert!(line.appearance.effects.is_empty());
    assert!(!line.appearance.stroke_paint().is_none(), "still a stroked line");
    assert!(line.appearance.fill().is_none_or(|f| f.paint.is_none()), "no fill");
    assert_eq!(run(&mut s, "object.expand.info", json!({}))["appearance"], json!(false));
    // Then Expand outlines the stroke into a filled shape.
    run(&mut s, "object.expand", json!({}));
    let outline = selected(&s);
    assert!(!outline.appearance.fill_paint().is_none(), "a filled outline");
    assert!(outline.appearance.stroke_paint().is_none());
}

/// The paths under `n` (itself included): (how many have a stroke, how many are filled black).
fn strokes_and_black_fills(n: &Node) -> (usize, usize) {
    let (mut stroked, mut black) = (0, 0);
    n.walk(&mut |c| {
        if matches!(c.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) {
            stroked += usize::from(!c.appearance.stroke_paint().is_none());
            black += usize::from(c.appearance.fill_paint() == Paint::solid(Color::BLACK));
        }
    });
    (stroked, black)
}

#[test]
fn object_expand_outlines_a_stroked_line_with_an_effect_in_one_step() {
    let mut s = session();
    zig_zag_line(&mut s);
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "object.expand", json!({}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let n = selected(&s);
    assert!(n.appearance.effects.is_empty(), "the zig-zag is baked");
    assert_eq!(strokes_and_black_fills(&n), (0, 1), "the stroke is outlined: {n:?}");
}

#[test]
fn object_expand_outlines_the_stroke_a_groups_appearance_expands_to() {
    // A group of two red squares without strokes; the group has a black stroke and a Transform
    // effect of its own, so Stroke has nothing to outline until Expand Appearance has run.
    let mut s = session();
    let a = NodeId(run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40}))["id"].as_u64().unwrap());
    let b = NodeId(run(&mut s, "shape.rectangle", json!({"x": 100, "y": 20, "width": 40, "height": 40}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    run(&mut s, "object.group", json!({}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "appearance.setItem", json!({"index": 0, "weight": 2, "color": "#000000"}));
    run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": {"moveV": 50}}));
    assert_eq!(run(&mut s, "object.expand.info", json!({}))["stroke"], json!(false), "no stroked path yet");
    run(&mut s, "object.expand", json!({}));
    let g = selected(&s);
    assert!(g.appearance.items.is_empty() && g.appearance.effects.is_empty(), "the appearance is expanded");
    let (stroked, black) = strokes_and_black_fills(&g);
    assert_eq!(stroked, 0, "no stroke is left: {g:?}");
    assert_eq!(black, 2, "each square's copy of the group's stroke is outlined: {g:?}");
}

#[test]
fn expand_info_counts_two_fills_but_not_a_plain_rectangle() {
    let mut s = session();
    run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60}));
    assert_eq!(run(&mut s, "object.expand.info", json!({}))["appearance"], json!(false), "a basic appearance");
    run(&mut s, "appearance.addFill", json!({}));
    assert_eq!(run(&mut s, "object.expand.info", json!({}))["appearance"], json!(true), "two fills");
}

/// A Transform effect left out past the effect budget (its result over 8 million lines drawn)
/// leaves the art where it is, and Expand, Expand Appearance and Outline Stroke keep the drop
/// shadow before it with its own offset and blur, as the canvas draws it: the object's, the fill's
/// and the stroke's.
#[test]
fn a_shadow_before_a_skipped_transform_expands_with_its_own_offset() {
    use vectorcraft_doc::{Appearance, Effect};
    use vectorcraft_geom::{Anchor, PathData, Point, SubPath};
    let anchors = (0..=30_000)
        .map(|i| {
            let x = 2.0 * f64::from(i);
            Anchor::with_handles(Point::new(x, 0.0), Point::new(x - 0.5, -20.0), Point::new(x + 0.5, 20.0))
        })
        .collect();
    let path = PathData::single(SubPath::new(anchors, true));
    let bounds = path.bounds().unwrap();
    let effects = || {
        vec![
            Effect { id: "stylize.dropShadow".into(), params: json!({"x": 10, "y": 5, "blur": 4}), visible: true },
            Effect { id: "distort.transform".into(), params: json!({"scaleH": 100000, "scaleV": 100000}), visible: true },
        ]
    };
    let shadow = |effects: &[Effect]| match vectorcraft_render::effects::raster_effects(effects).first() {
        Some(vectorcraft_render::effects::RasterFx::DropShadow { dx, dy, blur, .. }) => (*dx, *dy, *blur),
        other => panic!("expected a drop shadow, got {other:?}"),
    };
    let mut d = vectorcraft_doc::Document::new(200.0, 200.0);
    let basic = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 2.0);
    let mut object = Node::path(NodeId(1), path.clone(), basic.clone());
    object.appearance.effects = effects();
    let m = crate::cmd::expand::reshaped(&d, &object).unwrap();
    assert_eq!(m.geometric_bounds(), Some(bounds), "the art stays where it is");
    assert_eq!(shadow(&m.appearance.effects), (10.0, 5.0, 4.0), "the object's shadow");
    let mut fill = Node::path(NodeId(2), path.clone(), basic.clone());
    *fill.appearance.items[0].effects_mut() = effects();
    let (_, _, _, moved) = crate::cmd::expand::fill_shape(&d, &fill, &fill.appearance.items[0], &path, bounds).unwrap();
    assert_eq!(shadow(moved.effects()), (10.0, 5.0, 4.0), "the fill's shadow");
    let mut stroke = Node::path(NodeId(3), path.clone(), basic);
    *stroke.appearance.items[1].effects_mut() = effects();
    let outlined = crate::cmd::pathops::outline_strokes(&mut d, &[], &stroke).unwrap();
    let mut drawn = vec![];
    outlined.walk(&mut |n| {
        for item in &n.appearance.items {
            if !vectorcraft_render::effects::raster_effects(item.effects()).is_empty() {
                drawn.push(shadow(item.effects()));
            }
        }
    });
    assert_eq!(drawn, [(10.0, 5.0, 4.0)], "the stroke's shadow");
}
