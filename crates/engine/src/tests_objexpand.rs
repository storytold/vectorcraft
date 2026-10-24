//! Object → Expand (M3.84): the Object, Fill and Stroke options, gradient fills into solid strips,
//! concentric ellipses or a gradient mesh inside a clip group, freeform gradients into a mesh,
//! pattern fills into their tiles inside a clip group, and Create Gradient Mesh taking a gradient
//! fill's colours.

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{AppearanceItem, Node, NodeKind};
use vectorcraft_geom::Rect;

use super::*;

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
    assert_eq!(run(&mut s, "object.expand.info", json!({})), json!({"object": true, "fill": true, "stroke": false, "gradient": false}));
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
    assert_eq!(run(&mut s, "object.expand.info", json!({})), json!({"object": true, "fill": false, "stroke": true, "gradient": false}));
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

#[test]
fn a_freeform_gradient_expands_with_the_inks_of_its_points() {
    let ink = |c: f32, m: f32, y: f32, k: f32| json!({"c": c, "m": m, "y": y, "k": k});
    // The mesh's colors, after checking that it draws within `most` (mean channel difference) of
    // the gradient.
    let expanded = |s: &mut Session, most: f64| {
        let before = render(s);
        run(s, "object.expand", json!({}));
        let diff = mean_diff(&before, &render(s));
        assert!(diff < most, "looks the same: {diff}");
        let NodeKind::Mesh(m) = selected(s).kind else { panic!("a mesh: {:?}", selected(s).kind) };
        m.points.into_iter().map(|p| p.color).collect::<Vec<_>>()
    };
    // Points of one ink set give every mesh point that ink set.
    let mut s = session();
    let one = ink(0.5, 0.2, 0.0, 0.1);
    freeform_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), [one.clone(), one.clone(), one.clone(), one]);
    for c in expanded(&mut s, 1.0) {
        assert!(close_inks(inks(c), [0.5, 0.2, 0.0, 0.1]), "{c:?}");
    }
    // Cyan and black points mix as inks: cyan and black only, adding up to one.
    let mut s = session();
    let (cyan, black) = (ink(1.0, 0.0, 0.0, 0.0), ink(0.0, 0.0, 0.0, 1.0));
    freeform_rect(&mut s, Rect::new(20.0, 20.0, 180.0, 180.0), [cyan.clone(), cyan.clone(), black.clone(), black]);
    for c in expanded(&mut s, 5.0) {
        let [c, m, y, k] = inks(c);
        assert!(m.abs() < 1e-5 && y.abs() < 1e-5 && (c + k - 1.0).abs() < 1e-5, "C{c} M{m} Y{y} K{k}");
    }
    // Points in different models mix in display RGB, stored in a CMYK document as CMYK.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200, "colorMode": "cmyk"})).unwrap();
    let gray = json!({"gray": 0.5});
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
    assert_eq!(run(&mut s, "object.expand.info", json!({})), json!({"object": true, "fill": true, "stroke": true, "gradient": true}));
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
    assert_eq!(run(&mut s, "object.expand.info", json!({})), json!({"object": false, "fill": false, "stroke": false, "gradient": false}));
    assert!(s.execute("object.expand", &json!({})).is_err(), "nothing left to expand");
    // Bad options are refused before anything changes.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.execute("object.expand", &json!({"gradient": "rings"})).is_err());
    assert!(s.execute("object.expand", &json!({"steps": 0})).is_err());
    assert!(matches!(selected(&s).kind, NodeKind::Path { live: Some(_), .. }));
}
