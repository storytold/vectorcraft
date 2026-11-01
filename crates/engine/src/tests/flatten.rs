//! Object → Flatten Transparency (M3.91): atomic regions with the composited colours, rasterized
//! complex regions, the options and presets, undo and the look of the result.

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{AppearanceItem, Node, NodeKind};
use vectorcraft_geom::{BezPath, Point, Rect, Shape};
use vectorcraft_render::{Rendered, Renderer};

use crate::tests_expand::{RANDOM_CASES, random_transforms};
use crate::tests_menucmds::{place_undecodable_png, warns_undecoded};
use crate::*;

/// Colours here go through the process-wide colour settings, which the colour management tests
/// change: one at a time with them.
fn settings_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

/// An unstroked rectangle filled with `color`.
fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, color: &str) -> u64 {
    let id = run(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"].as_u64().unwrap();
    run(s, "paint.setFill", json!({"color": color, "ids": [id]}));
    run(s, "paint.setStroke", json!({"none": true, "ids": [id]}));
    id
}

fn node(s: &Session, id: u64) -> Node {
    s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone()
}

fn exists(s: &Session, id: u64) -> bool {
    s.doc().unwrap().doc.node(NodeId(id)).is_some()
}

/// The document over white at 1 px per point.
fn picture(s: &Session) -> Rendered {
    Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true)
}

fn flatten(s: &mut Session, p: Value) -> (Vec<u64>, u64, u64) {
    let r = run(s, "object.flattenTransparency", p);
    let ids = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    (ids, r["vector"].as_u64().unwrap(), r["rasterized"].as_u64().unwrap())
}

fn outline(n: &Node) -> BezPath {
    match &n.kind {
        NodeKind::Path { path, .. } => path.to_bezpath(),
        NodeKind::Compound { children, .. } => children.iter().fold(BezPath::new(), |mut bp, c| {
            bp.extend(outline(c));
            bp
        }),
        _ => panic!("a region is a path: {}", n.kind_label()),
    }
}

/// The flat-colour regions in a flattened group.
fn regions(s: &Session, group: u64) -> Vec<Node> {
    let g = node(s, group);
    g.children().unwrap().iter().filter(|c| matches!(c.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })).map(|c| (**c).clone()).collect()
}

/// A pixel centre well inside `n` (2 pt from its edges).
fn interior(n: &Node) -> Option<Point> {
    let bp = outline(n);
    let b = bp.bounding_box();
    let inside = |p: Point| [(0.0, 0.0), (2.0, 0.0), (-2.0, 0.0), (0.0, 2.0), (0.0, -2.0)].iter().all(|(dx, dy)| bp.winding(p + (*dx, *dy)) != 0);
    (b.y0.floor() as i32..b.y1.ceil() as i32)
        .flat_map(|y| (b.x0.floor() as i32..b.x1.ceil() as i32).map(move |x| Point::new(x as f64 + 0.5, y as f64 + 0.5)))
        .find(|p| inside(*p))
}

/// A region's colour over white, as 8-bit RGB.
fn shown(n: &Node) -> [u8; 3] {
    let AppearanceItem::Fill(f) = &n.appearance.items[0] else { panic!("a region has one fill") };
    let [r, g, b] = f.paint.color().unwrap().to_rgb();
    let a = n.opacity;
    let q = |v: f32| ((v * a + 1.0 - a) * 255.0).round() as u8;
    [q(r), q(g), q(b)]
}

fn pixel(img: &Rendered, p: Point) -> [u8; 3] {
    let [r, g, b, _] = img.pixel(p.x as u32, p.y as u32);
    [r, g, b]
}

/// Every region of `group` shows the colour `before` had inside it, within `tol`/255: 1 where the
/// renderer composites once, 2 where it rounds a group's 8-bit pixels again (the regions' own
/// colours are exact).
fn assert_regions_match(s: &Session, before: &Rendered, group: u64, tol: u8, what: &str) {
    let regions = regions(s, group);
    assert!(!regions.is_empty(), "{what}: no regions");
    for r in &regions {
        let Some(p) = interior(r) else { continue };
        let (want, got) = (pixel(before, p), shown(r));
        assert!(want.iter().zip(got).all(|(a, b)| a.abs_diff(b) <= tol), "{what}: at {p:?} the art showed {want:?}, the region has {got:?}");
    }
}

/// Share of pixels whose colour moved by more than 8/255.
fn changed(a: &Rendered, b: &Rendered) -> f64 {
    let n = a
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0)
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 8))
        .count();
    n as f64 / (a.pixels.len() / 4) as f64
}

#[test]
fn two_half_transparent_rects_give_three_regions_with_their_pixels() {
    let _settings = settings_lock();
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 100.0, 100.0, "#ff0000");
    let b = rect(&mut s, 60.0, 60.0, 100.0, 100.0, "#0000ff");
    run(&mut s, "transparency.set", json!({"ids": [a, b], "opacity": 50}));
    let before = picture(&s);
    let (ids, vector, rasterized) = flatten(&mut s, json!({"ids": [a, b]}));
    assert_eq!((ids.len(), vector, rasterized), (1, 3, 0));
    assert!(!exists(&s, a) && !exists(&s, b), "the objects are replaced");
    let rs = regions(&s, ids[0]);
    assert_eq!(rs.len(), 3);
    assert!(rs.iter().all(|r| r.opacity == 1.0 && r.has_default_transparency()), "regions are opaque");
    assert_regions_match(&s, &before, ids[0], 1, "50% rects");
    // Red alone over white is pink; blue over red over white is purple.
    let at = |p: Point| rs.iter().find(|r| outline(r).winding(p) != 0).map(shown).unwrap();
    assert_eq!(at(Point::new(30.0, 30.0)), [255, 128, 128]);
    assert!(at(Point::new(85.0, 85.0)).iter().zip([128, 64, 191]).all(|(x, y)| x.abs_diff(y) <= 1), "{:?}", at(Point::new(85.0, 85.0)));
    assert_eq!(s.doc().unwrap().selection.objects, vec![NodeId(ids[0])]);
}

#[test]
fn every_blend_mode_flattens_to_its_pixels() {
    let _settings = settings_lock();
    for mode in BlendMode::ALL {
        // Normal at full opacity is no transparency: nothing to flatten.
        for opacity in if mode == BlendMode::Normal { &[70][..] } else { &[100, 70][..] } {
            let mut s = session();
            let a = rect(&mut s, 10.0, 10.0, 100.0, 100.0, "#cc6633");
            let b = rect(&mut s, 60.0, 60.0, 100.0, 100.0, "#4d99e6");
            run(&mut s, "transparency.set", json!({"ids": [b], "blend": mode.label(), "opacity": opacity}));
            let before = picture(&s);
            let (ids, vector, _) = flatten(&mut s, json!({"ids": [a, b]}));
            assert_eq!(vector, 3, "{mode:?}");
            // The renderer rounds the blended source again before mixing it in by its opacity.
            assert_regions_match(&s, &before, ids[0], 2, &format!("{mode:?} at {opacity}%"));
            // The overlap is the reference formula's colour.
            let [r, g, b] = [0xcc, 0x66, 0x33].map(|v| v as f32 / 255.0);
            let src = [0x4d, 0x99, 0xe6].map(|v| v as f32 / 255.0);
            let want = vectorcraft_color::blend::composite(mode, [r, g, b, 1.0], [src[0], src[1], src[2], *opacity as f32 / 100.0]);
            let overlap = regions(&s, ids[0]).into_iter().find(|r| outline(r).winding(Point::new(85.0, 85.0)) != 0).unwrap();
            let got = shown(&overlap);
            assert!((0..3).all(|i| (want[i] * 255.0 - got[i] as f32).abs() <= 0.5 + 1e-3), "{mode:?} at {opacity}%: {got:?} vs {want:?}");
        }
    }
}

#[test]
fn groups_flatten_as_they_composite() {
    let _settings = settings_lock();
    type Setup = fn(&mut Session, u64, u64, u64);
    let cases: [(&str, Setup); 8] = [
        ("group with its own fill", |s, _, b, c| {
            run(s, "select.set", json!({"ids": [b, c]}));
            let g = run(s, "object.group", json!({}))["id"].clone();
            run(s, "appearance.addFill", json!({"ids": [g]}));
            run(s, "appearance.setItem", json!({"ids": [g], "index": 0, "color": "#ffff00", "opacity": 50, "blend": "Multiply"}));
        }),
        ("knockout shape", |s, _, b, c| {
            run(s, "transparency.set", json!({"ids": [b], "opacity": 50}));
            run(s, "transparency.set", json!({"ids": [c], "opacity": 40, "knockoutShape": true}));
            run(s, "select.set", json!({"ids": [b, c]}));
            run(s, "object.group", json!({}));
            run(s, "transparency.set", json!({"knockout": "on"}));
        }),
        ("knockout group", |s, _, b, c| {
            run(s, "transparency.set", json!({"ids": [b, c], "opacity": 50}));
            run(s, "select.set", json!({"ids": [b, c]}));
            run(s, "object.group", json!({}));
            run(s, "transparency.set", json!({"knockout": "on"}));
        }),
        ("non-isolated group with opacity", |s, _, b, c| {
            run(s, "transparency.set", json!({"ids": [c], "blend": "Multiply"}));
            run(s, "select.set", json!({"ids": [b, c]}));
            run(s, "object.group", json!({}));
            run(s, "transparency.set", json!({"opacity": 60}));
        }),
        ("isolated group", |s, _, b, c| {
            run(s, "transparency.set", json!({"ids": [c], "blend": "Difference"}));
            run(s, "select.set", json!({"ids": [b, c]}));
            run(s, "object.group", json!({}));
            run(s, "transparency.set", json!({"opacity": 80, "isolate": true}));
        }),
        ("blending group with blending content", |s, _, b, c| {
            run(s, "transparency.set", json!({"ids": [c], "blend": "Multiply"}));
            run(s, "select.set", json!({"ids": [b, c]}));
            run(s, "object.group", json!({}));
            run(s, "transparency.set", json!({"blend": "Screen", "opacity": 90}));
        }),
        ("non-isolated knockout group", |s, _, b, c| {
            run(s, "transparency.set", json!({"ids": [b], "opacity": 50}));
            run(s, "transparency.set", json!({"ids": [c], "blend": "Multiply", "opacity": 80}));
            run(s, "select.set", json!({"ids": [b, c]}));
            run(s, "object.group", json!({}));
            run(s, "transparency.set", json!({"knockout": "on"}));
        }),
        ("clip group", |s, _, b, c| {
            run(s, "transparency.set", json!({"ids": [b], "opacity": 50}));
            let e = run(s, "shape.ellipse", json!({"x": 40, "y": 40, "width": 120, "height": 120}))["id"].clone();
            run(s, "select.set", json!({"ids": [b, c, e]}));
            run(s, "object.clippingMask.make", json!({}));
        }),
    ];
    for (what, setup) in cases {
        let mut s = session();
        let a = rect(&mut s, 0.0, 80.0, 200.0, 40.0, "#33aa55");
        let b = rect(&mut s, 10.0, 10.0, 100.0, 100.0, "#e04020");
        let c = rect(&mut s, 60.0, 60.0, 100.0, 100.0, "#2060e0");
        setup(&mut s, a, b, c);
        let before = picture(&s);
        run(&mut s, "select.all", json!({}));
        let (ids, vector, rasterized) = flatten(&mut s, json!({}));
        assert!(ids.len() == 1 && vector >= 3 && rasterized == 0, "{what}: {ids:?} {vector} {rasterized}");
        assert_regions_match(&s, &before, ids[0], 2, what);
        assert!(changed(&before, &picture(&s)) < 0.02, "{what}: the look changed");
    }
}

#[test]
fn balance_trades_vector_regions_for_an_image() {
    let _settings = settings_lock();
    let setup = || {
        let mut s = session();
        let a = rect(&mut s, 10.0, 10.0, 100.0, 100.0, "#ff8000");
        let b = rect(&mut s, 60.0, 60.0, 100.0, 100.0, "#0080ff");
        run(&mut s, "transparency.set", json!({"ids": [b], "opacity": 50, "blend": "Multiply"}));
        (s, a, b)
    };
    let (mut s, a, b) = setup();
    assert_eq!(flatten(&mut s, json!({"ids": [a, b], "balance": 100})).1, 3);
    let (mut s, a, b) = setup();
    let before = picture(&s);
    let (ids, vector, rasterized) = flatten(&mut s, json!({"ids": [a, b], "balance": 0, "clipComplexRegions": true, "lineArtPpi": 144}));
    assert_eq!((vector, rasterized), (0, 1));
    let g = node(&s, ids[0]);
    let clip = &g.children().unwrap()[0];
    assert!(clip.clips(), "clipped to the regions");
    let NodeKind::Image(im) = &clip.children().unwrap()[1].kind else { panic!("an image") };
    assert!(im.width >= 300, "at the line art resolution: {}", im.width);
    assert!(changed(&before, &picture(&s)) < 0.02);
    // Unclipped, the image is a plain rectangle.
    let (mut s, a, b) = setup();
    let (ids, ..) = flatten(&mut s, json!({"ids": [a, b], "balance": 0, "clipComplexRegions": false}));
    assert!(matches!(node(&s, ids[0]).children().unwrap()[0].kind, NodeKind::Image(_)));
    assert!(changed(&before, &picture(&s)) < 0.02);
    // In between, a group splitting into more regions than the balance allows is rasterized.
    let (mut s, a, b) = setup();
    assert_eq!(flatten(&mut s, json!({"ids": [a, b], "balance": 5})).2, 1);
}

#[test]
fn gradients_rasterize_and_flat_areas_stay_vector() {
    let _settings = settings_lock();
    for clip in [true, false] {
        let mut s = session();
        let g = rect(&mut s, 10.0, 10.0, 100.0, 100.0, "#000000");
        run(
            &mut s,
            "paint.setFill",
            json!({"ids": [g], "gradient": {"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}}),
        );
        let t = rect(&mut s, 80.0, 80.0, 100.0, 100.0, "#20c040");
        run(&mut s, "transparency.set", json!({"ids": [t], "opacity": 50}));
        let before = picture(&s);
        let (ids, vector, rasterized) = flatten(&mut s, json!({"ids": [g, t], "preset": "high", "clipComplexRegions": clip}));
        assert_eq!(rasterized, 1, "clip {clip}");
        assert!(vector >= 1, "the green alone stays vector (clip {clip})");
        assert_regions_match(&s, &before, ids[0], 1, "gradient");
        assert!(changed(&before, &picture(&s)) < 0.02, "clip {clip}");
    }
}

#[test]
fn flatten_says_when_an_image_could_not_be_decoded() {
    let _settings = settings_lock();
    let mut s = session();
    let cut = place_undecodable_png(&mut s).0;
    let t = rect(&mut s, 60.0, 60.0, 100.0, 100.0, "#20c040");
    run(&mut s, "transparency.set", json!({"ids": [t], "opacity": 50}));
    let r = run(&mut s, "object.flattenTransparency", json!({"ids": [cut, t]}));
    assert!(warns_undecoded(&r), "{r}");
    assert_eq!(r["rasterized"], 1, "the image area is still flattened: {r}");
}

#[test]
fn flatten_is_one_undo_step() {
    let _settings = settings_lock();
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 100.0, 100.0, "#ff0000");
    let b = rect(&mut s, 60.0, 60.0, 100.0, 100.0, "#0000ff");
    run(&mut s, "transparency.set", json!({"ids": [b], "opacity": 50}));
    let steps = s.doc().unwrap().history.undo.len();
    let doc = s.doc().unwrap().doc.clone();
    flatten(&mut s, json!({"ids": [a, b]}));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Flatten Transparency");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().doc.layers, doc.layers);
    run(&mut s, "edit.redo", json!({}));
    assert!(!exists(&s, a));
}

/// Flattening a document for export in which two objects have the same id (a damaged file)
/// removes both objects and puts one group in their place.
#[test]
fn flattening_objects_that_share_an_id_removes_both() {
    let _settings = settings_lock();
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 100.0, 100.0, "#ff0000");
    let b = rect(&mut s, 60.0, 60.0, 100.0, 100.0, "#0000ff");
    run(&mut s, "transparency.set", json!({"ids": [a, b], "opacity": 50}));
    let mut doc = (*s.doc().unwrap().doc).clone();
    doc.node_mut(NodeId(b)).unwrap().id = NodeId(a);
    let flat = crate::cmd::flatten::flatten_document(&doc, &crate::cmd::FlattenOptions::default()).unwrap().unwrap();
    let art = flat.layers[0].children().unwrap();
    assert_eq!(art.len(), 1, "one group where the two objects were");
    assert!(matches!(art[0].kind, NodeKind::Group { .. }) && art[0].id != NodeId(a));
}

#[test]
fn objects_without_transparency_stay_unless_outlined() {
    let _settings = settings_lock();
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 50.0, 50.0, "#ff0000");
    run(&mut s, "transparency.set", json!({"ids": [a], "opacity": 50}));
    let far = rect(&mut s, 120.0, 120.0, 50.0, 50.0, "#00ff00");
    run(&mut s, "paint.setStroke", json!({"color": "#000000", "ids": [far]}));
    let text = run(&mut s, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 24}))["id"].as_u64().unwrap();
    // Nothing transparent: nothing happens.
    let steps = s.doc().unwrap().history.undo.len();
    let (ids, vector, rasterized) = flatten(&mut s, json!({"ids": [far, text]}));
    assert_eq!((ids, vector, rasterized, s.doc().unwrap().history.undo.len()), (vec![far, text], 0, 0, steps));
    // The transparent rectangle is flattened; the others keep their ids.
    let (ids, vector, _) = flatten(&mut s, json!({"ids": [a, far, text]}));
    assert_eq!(vector, 1);
    assert!(exists(&s, far) && exists(&s, text) && !exists(&s, a) && ids.contains(&far) && ids.contains(&text));
    // On request, type and strokes are outlined too.
    let (ids, ..) = flatten(&mut s, json!({"ids": [far, text], "textToOutlines": true, "strokesToOutlines": true}));
    assert!(!exists(&s, far) && !exists(&s, text) && ids.len() == 2);
    for id in ids {
        node(&s, id).walk(&mut |c| {
            assert!(!matches!(c.kind, NodeKind::Text(_)), "type is outlined");
            assert!(c.appearance.stroke().is_none_or(|st| st.paint.is_none()), "strokes are outlined");
        });
    }
}

#[test]
fn outlined_type_keeps_its_underline() {
    let _settings = settings_lock();
    let mut s = session();
    let text = run(&mut s, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 24}))["id"].as_u64().unwrap();
    run(&mut s, "text.setFormat", json!({"ids": [text], "underline": true}));
    let (ids, ..) = flatten(&mut s, json!({"ids": [text], "textToOutlines": true}));
    // "Hi" has no ink below its baseline (y 150): the outlines reach the bar under it.
    let b = node(&s, ids[0]).geometric_bounds().unwrap();
    assert!(b.y1 > 151.0, "{b:?}");
}

#[test]
fn outlined_type_keeps_its_gradient_on_the_outlines_its_transform_effect_moves() {
    let _settings = settings_lock();
    // "IIII" at 80 pt with its baseline at (x, y), its characters filled with a red-to-blue gradient
    // that runs left to right, with a Transform effect of `params` if any.
    let typed = |x: f64, y: f64, params: Option<Value>| {
        let mut s = session();
        let t = run(&mut s, "text.create", json!({"x": x, "y": y, "text": "IIII", "size": 80}))["id"].as_u64().unwrap();
        let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
        run(&mut s, "paint.setFill", json!({"ids": [t], "gradient": {"kind": "linear", "stops": stops}}));
        if let Some(params) = params {
            run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": params, "ids": [t], "item": null}));
        }
        (s, t)
    };
    let mut diffs = vec![];
    // The effect moves the type 30 pt right (along the gradient) or 20 pt down (across it). Type
    // laid out at the moved baseline is the reference.
    for (dx, dy) in [(30.0, 0.0), (0.0, 20.0)] {
        let reference = picture(&typed(10.0 + dx, 150.0 + dy, None).0);
        let (mut s, t) = typed(10.0, 150.0, Some(json!({"moveH": dx, "moveV": dy})));
        flatten(&mut s, json!({"ids": [t], "textToOutlines": true}));
        diffs.push(changed(&picture(&s), &reference));
        // The outlines draw the same after the document is saved and opened again.
        let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
        diffs.push(changed(&Renderer::new().render_region(&back, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true), &reference));
    }
    assert!(diffs.iter().all(|&d| d < 0.0002), "shares of the pixels that changed: {diffs:?}");
}

/// Random in a Transform effect takes its seed from the object's id. Each of [`random_transforms`]'
/// objects at 50% opacity draws as the canvas drew it with its geometry effects baked, as Flatten
/// Transparency bakes them first, and after Flatten Transparency, within the seams between its
/// regions (the same objects without Random change 1.4% to 2.3% of the pixels).
#[test]
fn flattened_art_keeps_the_random_transforms_the_canvas_drew() {
    let _settings = settings_lock();
    let mut diffs = vec![];
    for case in RANDOM_CASES {
        let mut s = session();
        let id = random_transforms(&mut s, case);
        run(&mut s, "transparency.set", json!({"ids": [id.0], "opacity": 50}));
        let before = picture(&s);
        let baked = vectorcraft_render::effects::bake_document(&s.doc().unwrap().doc).unwrap();
        let baked = Renderer::new().render_region(&baked, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true);
        flatten(&mut s, json!({"ids": [id.0]}));
        diffs.push((case, changed(&before, &baked), changed(&before, &picture(&s))));
    }
    assert!(diffs.iter().all(|&(_, baked, flat)| baked == 0.0 && flat < 0.03), "shares of the pixels that changed: {diffs:?}");
}

#[test]
fn preserve_alpha_composites_over_nothing() {
    let _settings = settings_lock();
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 100.0, 100.0, "#ff0000");
    run(&mut s, "transparency.set", json!({"ids": [a], "opacity": 50}));
    let before = picture(&s);
    let (ids, ..) = flatten(&mut s, json!({"ids": [a], "preserveAlpha": true}));
    let r = &regions(&s, ids[0])[0];
    assert_eq!((r.opacity, r.appearance.fill_paint()), (0.5, Paint::solid(Color::rgb(1.0, 0.0, 0.0))));
    assert_regions_match(&s, &before, ids[0], 1, "preserve alpha");
    // Over white by default: opaque pink.
    run(&mut s, "edit.undo", json!({}));
    let (ids, ..) = flatten(&mut s, json!({"ids": [a]}));
    let r = &regions(&s, ids[0])[0];
    assert_eq!((r.opacity, shown(r)), (1.0, [255, 128, 128]));
}

#[test]
fn single_paints_keep_their_colour_and_overprint() {
    let _settings = settings_lock();
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 100.0, 100.0, "#000000");
    run(&mut s, "paint.setFill", json!({"ids": [a], "color": {"c": 0, "m": 100, "y": 50, "k": 0}}));
    run(&mut s, "object.setOverprint", json!({"ids": [a], "fill": true}));
    let b = rect(&mut s, 60.0, 60.0, 100.0, 100.0, "#0000ff");
    run(&mut s, "transparency.set", json!({"ids": [b], "opacity": 50}));
    let cmyk_region = |s: &Session, ids: &[u64]| regions(s, ids[0]).into_iter().find(|r| outline(r).winding(Point::new(30.0, 30.0)) != 0).unwrap();
    let (ids, ..) = flatten(&mut s, json!({"ids": [a, b]}));
    let r = cmyk_region(&s, &ids);
    let AppearanceItem::Fill(f) = &r.appearance.items[0] else { panic!() };
    assert!(matches!(f.paint.color(), Some(Color::Cmyk { .. })) && f.overprint, "{:?}", f);
    run(&mut s, "edit.undo", json!({}));
    let (ids, ..) = flatten(&mut s, json!({"ids": [a, b], "preserveOverprints": false}));
    let r = cmyk_region(&s, &ids);
    let AppearanceItem::Fill(f) = &r.appearance.items[0] else { panic!() };
    assert!(matches!(f.paint.color(), Some(Color::Rgb { .. })) && !f.overprint);
}

#[test]
fn options_come_from_presets_and_params() {
    let _settings = settings_lock();
    use crate::cmd::FlattenOptions;
    let high = FlattenOptions::preset("High Resolution").unwrap();
    assert_eq!((high.balance, high.line_art_ppi, high.clip_complex_regions), (100.0, 1200.0, true));
    assert_eq!(FlattenOptions::default(), FlattenOptions::preset("medium").unwrap());
    assert_ne!(FlattenOptions::preset("low"), FlattenOptions::preset("medium"));
    let o = FlattenOptions::from_params(&json!({"preset": "low", "balance": 40, "options": {"antiAlias": false}})).unwrap();
    assert_eq!((o.balance, o.gradient_ppi, o.anti_alias), (40.0, 150.0, false));
    for bad in [json!({"preset": "ultra"}), json!({"balance": 120}), json!({"lineArtPpi": 0}), json!({"balance": "lots"})] {
        assert!(FlattenOptions::from_params(&bad).is_err(), "{bad}");
    }
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 50.0, 50.0, "#ff0000");
    run(&mut s, "transparency.set", json!({"ids": [a], "opacity": 50}));
    assert!(s.execute("object.flattenTransparency", &json!({"ids": [a], "balance": -1})).is_err());
    run(&mut s, "select.none", json!({}));
    assert!(s.execute("object.flattenTransparency", &json!({})).is_err(), "nothing to flatten");
    let r = run(&mut s, "object.flattenTransparency", json!({"ids": [a], "preset": "high"}));
    assert_eq!(r["options"]["lineArtPpi"], 1200.0);
}

#[test]
fn a_mixed_scene_looks_the_same_flattened() {
    let _settings = settings_lock();
    for preset in FlattenPresets::ALL {
        let mut s = session();
        rect(&mut s, 0.0, 0.0, 200.0, 200.0, "#ffcc00");
        let r = rect(&mut s, 30.0, 30.0, 80.0, 80.0, "#3355cc");
        run(&mut s, "paint.setStroke", json!({"color": "#000000", "ids": [r]}));
        run(&mut s, "stroke.set", json!({"weight": 8, "ids": [r]}));
        run(&mut s, "transparency.set", json!({"ids": [r], "opacity": 60}));
        let t = run(&mut s, "text.create", json!({"x": 40, "y": 170, "text": "Hi", "size": 48, "color": "#e01010"}))["id"].clone();
        run(&mut s, "transparency.set", json!({"ids": [t], "opacity": 70, "blend": "Multiply"}));
        let e = run(&mut s, "shape.ellipse", json!({"x": 120, "y": 40, "width": 60, "height": 60}))["id"].clone();
        run(&mut s, "paint.setFill", json!({"color": "#20a040", "ids": [e]}));
        run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "ids": [e]}));
        let m = rect(&mut s, 130.0, 130.0, 40.0, 40.0, "#8040c0");
        run(&mut s, "transparency.set", json!({"ids": [m], "opacity": 50}));
        run(&mut s, "select.set", json!({"ids": [m]}));
        let sym = run(&mut s, "symbol.new", json!({"name": "Half"}))["id"].as_u64().unwrap();
        assert!(matches!(node(&s, sym).kind, NodeKind::SymbolInstance { .. }));
        let before = picture(&s);
        run(&mut s, "select.all", json!({}));
        let (ids, vector, rasterized) = flatten(&mut s, json!({"preset": preset, "lineArtPpi": 216, "gradientPpi": 144}));
        assert!(ids.len() == 1 && rasterized == 1, "{preset}: the shadow is rasterized ({ids:?}, {rasterized})");
        assert!(preset == "low" || vector > 4, "{preset}: {vector} regions");
        let mut kinds = vec![];
        node(&s, ids[0]).walk(&mut |c| kinds.push(c.kind_label()));
        assert!(!kinds.iter().any(|k| ["Type", "Symbol"].contains(k)), "{preset}: {kinds:?}");
        let diff = changed(&before, &picture(&s));
        assert!(diff < 0.03, "{preset}: {:.1}% of the pixels changed", diff * 100.0);
    }
}

struct FlattenPresets;
impl FlattenPresets {
    const ALL: [&'static str; 3] = crate::cmd::FlattenOptions::PRESETS;
}

#[test]
fn opacity_masks_are_rasterized_with_their_look() {
    let _settings = settings_lock();
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 200.0, 200.0, "#ffffff");
    let a = rect(&mut s, 20.0, 20.0, 160.0, 160.0, "#d02060");
    let m = rect(&mut s, 40.0, 40.0, 120.0, 120.0, "#000000");
    run(
        &mut s,
        "paint.setFill",
        json!({"ids": [m], "gradient": {"kind": "linear", "stops": [{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]}}),
    );
    run(&mut s, "transparency.makeOpacityMask", json!({"ids": [a, m]}));
    assert!(node(&s, a).extras().mask.is_some());
    let before = picture(&s);
    run(&mut s, "select.all", json!({}));
    let (ids, _, rasterized) = flatten(&mut s, json!({"lineArtPpi": 144}));
    assert_eq!((ids.len(), rasterized), (1, 1));
    let mut masks = 0;
    node(&s, ids[0]).walk(&mut |c| masks += c.extras().mask.is_some() as usize);
    assert_eq!(masks, 0, "no transparency is left");
    assert!(changed(&before, &picture(&s)) < 0.02);
}
