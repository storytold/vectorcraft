use super::*;
use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::shapes;

mod adjust;
mod alternate;
mod bitmapglyphs;
mod blend;
mod changed;
mod charstroke;
mod clip;
mod cmykblend;
mod colrglyphs;
mod container;
mod dither;
pub(super) mod fontchange;
mod freeform;
mod fxzoom;
mod isolation;
mod knockout;
mod layeropts;
mod memory;
mod objectfx;
mod placed_document;
mod rasterize;
mod setup;
mod strokegradient;
mod tileedge;
mod viewraster;

fn rect_node(d: &mut Document, r: Rect, fill: Paint, stroke: Paint, w: f64) -> Node {
    let id = d.alloc_id();
    Node::path(id, shapes::rectangle(r), Appearance::basic(fill, stroke, w))
}

fn render(d: &Document) -> Rendered {
    Renderer::new().render(d, 100, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

#[test]
fn fills_red_rect() {
    let mut d = Document::new(100.0, 100.0);
    let n = rect_node(&mut d, Rect::new(10.0, 10.0, 50.0, 50.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let d = {
        let mut d2 = d.clone();
        d2.insert(Some(d.layers[0].id), 0, n).unwrap();
        d2
    };
    let r = render(&d);
    assert_eq!(r.pixel(30, 30), [255, 0, 0, 255]);
    assert_eq!(r.pixel(70, 70), [255, 255, 255, 255]);
}

#[test]
fn raster_size_limits() {
    let r = Rect::new(0.0, 0.0, 100.0, 50.0);
    assert_eq!(raster_size(r, 2.0), Ok((200, 100)));
    assert_eq!(raster_size(r, 0.001), Ok((1, 1)));
    assert_eq!(raster_size(Rect::new(0.0, 0.0, MAX_RASTER_SIDE as f64, 1.0), 1.0), Ok((MAX_RASTER_SIDE, 1)));
    assert!(raster_size(Rect::new(0.0, 0.0, MAX_RASTER_SIDE as f64 + 1.0, 1.0), 1.0).is_err());
    assert!(raster_size(Rect::new(0.0, 0.0, 20_000.0, 20_000.0), 1.0).is_err(), "400 megapixels");
    assert!(raster_size(r, f64::NAN).is_err());
    assert!(raster_size(r, f64::INFINITY).is_err());
    assert!(raster_size(r, 0.0).is_err());
}

#[test]
fn stroke_draws_outline_only() {
    let mut d = Document::new(100.0, 100.0);
    let n = rect_node(&mut d, Rect::new(10.0, 10.0, 90.0, 90.0), Paint::None, Paint::solid(Color::BLACK), 4.0);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let r = render(&d);
    assert_eq!(r.pixel(10, 50), [0, 0, 0, 255]);
    assert_eq!(r.pixel(50, 50), [255, 255, 255, 255]);
}

/// #1396: a stroke wider than its ellipse is a solid blob, pixel for pixel the disc a renderer
/// with a native stroke (the PDF and SVG exports) draws, at every alignment and zoom.
#[test]
fn a_stroke_wider_than_its_ellipse_paints_solid() {
    let doc = |r: f64, fill: Paint, stroke: Paint, w: f64, align: StrokeAlign| {
        let mut d = Document::new(100.0, 100.0);
        let id = d.alloc_id();
        let mut n = Node::path(id, shapes::ellipse(Rect::new(50.0 - r, 50.0 - r, 50.0 + r, 50.0 + r)), Appearance::basic(fill, stroke, w));
        if let Some(st) = n.appearance.stroke_mut() {
            st.align = align;
        }
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        d
    };
    let black = || Paint::solid(Color::BLACK);
    let render = |d: &Document, zoom: f64| {
        let view = Affine::translate((50.0, 50.0)) * Affine::scale(zoom) * Affine::translate((-50.0, -50.0));
        Renderer::new().render(d, 100, 100, view, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
    };
    // (ellipse radius, weight, alignment, radius of the disc it paints)
    for (r, w, align, disc) in [
        (10.0, 60.0, StrokeAlign::Center, 40.0),
        (6.0, 50.0, StrokeAlign::Center, 31.0),
        (15.0, 50.0, StrokeAlign::Center, 40.0),
        (10.0, 30.0, StrokeAlign::Outside, 40.0),
        (10.0, 30.0, StrokeAlign::Inside, 10.0),
    ] {
        for zoom in [0.5, 1.0, 2.0] {
            let got = render(&doc(r, Paint::None, black(), w, align), zoom);
            if align == StrokeAlign::Outside {
                // The shape itself stays unpainted, and the stroke around it has no gap.
                assert_eq!(got.pixel(50, 50), [255, 255, 255, 255], "{r} {w} {align:?} at {zoom}");
                assert_eq!(got.pixel(50 + (12.0 * zoom) as u32, 50), [0, 0, 0, 255], "{r} {w} {align:?} at {zoom}");
                continue;
            }
            let want = render(&doc(disc, black(), Paint::None, 0.0, StrokeAlign::Center), zoom);
            let off =
                (0..100).flat_map(|y| (0..100).map(move |x| (x, y))).filter(|&(x, y)| got.pixel(x, y)[0].abs_diff(want.pixel(x, y)[0]) > 40).count();
            assert!(off <= 8, "{r} {w} {align:?} at {zoom}: {off} pixels differ from the disc");
        }
    }
}

#[test]
fn inside_stroke_stays_inside() {
    let mut d = Document::new(100.0, 100.0);
    let mut n = rect_node(&mut d, Rect::new(20.0, 20.0, 80.0, 80.0), Paint::None, Paint::solid(Color::BLACK), 10.0);
    n.appearance.stroke_mut().unwrap().align = StrokeAlign::Inside;
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let r = render(&d);
    assert_eq!(r.pixel(17, 50), [255, 255, 255, 255]);
    assert_eq!(r.pixel(25, 50), [0, 0, 0, 255]);
}

#[test]
fn outside_stroke_stays_outside() {
    let mut d = Document::new(100.0, 100.0);
    let mut n = rect_node(&mut d, Rect::new(20.0, 20.0, 80.0, 80.0), Paint::solid(Color::WHITE), Paint::solid(Color::BLACK), 10.0);
    n.appearance.stroke_mut().unwrap().align = StrokeAlign::Outside;
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let r = render(&d);
    assert_eq!(r.pixel(15, 50), [0, 0, 0, 255]);
    assert_eq!(r.pixel(23, 50), [255, 255, 255, 255]);
}

#[test]
fn opacity_blends() {
    let mut d = Document::new(100.0, 100.0);
    let mut n = rect_node(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), Paint::solid(Color::BLACK), Paint::None, 0.0);
    n.opacity = 0.5;
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let p = render(&d).pixel(50, 50);
    assert!((p[0] as i32 - 128).abs() <= 2, "{p:?}");
}

#[test]
fn multiply_blend() {
    let mut d = Document::new(100.0, 100.0);
    let a = rect_node(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), Paint::solid(Color::rgb(1.0, 1.0, 0.0)), Paint::None, 0.0);
    let mut b = rect_node(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), Paint::solid(Color::rgb(0.0, 1.0, 1.0)), Paint::None, 0.0);
    b.blend = vectorcraft_color::BlendMode::Multiply;
    let l = d.layers[0].id;
    d.insert(Some(l), 0, a).unwrap();
    d.insert(Some(l), 1, b).unwrap();
    assert_eq!(render(&d).pixel(50, 50), [0, 255, 0, 255]);
}

#[test]
fn linear_gradient_goes_white_to_black() {
    let mut d = Document::new(100.0, 100.0);
    let n =
        rect_node(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), Paint::Gradient(Box::new(GradientPaint::new(Gradient::default()))), Paint::None, 0.0);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let r = render(&d);
    assert!(r.pixel(2, 50)[0] > 240);
    assert!(r.pixel(97, 50)[0] < 15);
    let m = r.pixel(50, 50)[0] as i32;
    assert!((m - 128).abs() < 12, "{m}");
}

#[test]
fn clip_group_clips() {
    let mut d = Document::new(100.0, 100.0);
    let mut clip = rect_node(&mut d, Rect::new(0.0, 0.0, 50.0, 100.0), Paint::None, Paint::None, 0.0);
    if let NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    let art = rect_node(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), Paint::solid(Color::BLACK), Paint::None, 0.0);
    let gid = d.alloc_id();
    let g = Node::new(gid, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(art)], clip: true });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    let r = render(&d);
    assert_eq!(r.pixel(25, 50), [0, 0, 0, 255]);
    assert_eq!(r.pixel(75, 50), [255, 255, 255, 255]);
}

#[test]
fn hidden_and_culled() {
    let mut d = Document::new(100.0, 100.0);
    let mut n = rect_node(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), Paint::solid(Color::BLACK), Paint::None, 0.0);
    n.visible = false;
    let far = rect_node(&mut d, Rect::new(1000.0, 1000.0, 1100.0, 1100.0), Paint::solid(Color::BLACK), Paint::None, 0.0);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d.insert(Some(l), 1, far).unwrap();
    let mut rr = Renderer::new();
    let r = rr.render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    assert_eq!(r.pixel(50, 50), [0, 0, 0, 0]);
    assert_eq!(rr.stats.culled, 1);
}

/// A document with symbol "Far" holding a red rectangle `art`, and an instance of it moved by
/// (-50, -50): its 20 × 20 square, (-60, -60) to (-40, -40), lies outside a view of the page.
fn far_instance(art: Rect) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let art = rect_node(&mut d, art, Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Far".into(), art: Arc::new(art) });
    let (id, l) = (d.alloc_id(), d.layers[0].id);
    d.insert(Some(l), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Far".into(), xf: Affine::translate((-50.0, -50.0)) })).unwrap();
    d
}

#[test]
fn an_instance_is_culled_by_its_symbols_art() {
    // The art drawn from (350, 350) is out of view.
    let away = far_instance(Rect::new(400.0, 400.0, 440.0, 440.0));
    let mut rr = Renderer::new();
    assert_eq!(rr.render(&away, 100, 100, Affine::IDENTITY, &RenderOptions::default()).pixel(70, 70), [0, 0, 0, 0]);
    assert_eq!(rr.stats.culled, 1);
    // The symbol's art, from (100, 100), drawn from (50, 50): in view. The instance is the same
    // node, and the renderer measures it again.
    let mut near = away.clone();
    let art = rect_node(&mut near, Rect::new(100.0, 100.0, 140.0, 140.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    near.symbols[0].art = Arc::new(art);
    assert_eq!(rr.render(&near, 100, 100, Affine::IDENTITY, &RenderOptions::default()).pixel(70, 70), [255, 0, 0, 255]);
    assert_eq!(rr.stats.culled, 0);
}

#[test]
fn an_instance_is_culled_by_the_effects_in_its_art() {
    // The art, (-1, -1) to (1, 1), drawn from (-5, 20) lies left of the view, and its drop shadow,
    // 8 pt to the right, reaches into it.
    let mut d = Document::new(100.0, 100.0);
    let mut art = rect_node(&mut d, Rect::new(-1.0, -1.0, 1.0, 1.0), Paint::solid(Color::BLACK), Paint::None, 0.0);
    let shadow = vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: serde_json::json!({"x": 8, "y": 0, "blur": 0}), visible: true };
    art.appearance.effects.push(shadow);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Dot".into(), art: Arc::new(art) });
    let (id, l) = (d.alloc_id(), d.layers[0].id);
    d.insert(Some(l), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Affine::translate((-5.0, 20.0)) })).unwrap();
    let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    assert!(img.pixel(3, 20)[3] > 0, "{:?}", img.pixel(3, 20));
}

#[test]
fn a_repeat_of_an_instance_is_culled_by_the_effects_in_its_art() {
    // The instance of the test above as the one copy of a one-by-one grid repeat.
    let mut d = Document::new(100.0, 100.0);
    let mut art = rect_node(&mut d, Rect::new(-1.0, -1.0, 1.0, 1.0), Paint::solid(Color::BLACK), Paint::None, 0.0);
    let shadow = vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: serde_json::json!({"x": 8, "y": 0, "blur": 0}), visible: true };
    art.appearance.effects.push(shadow);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Dot".into(), art: Arc::new(art) });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Affine::translate((-5.0, 20.0)) });
    let grid = vectorcraft_doc::TileType::Grid;
    let kind =
        vectorcraft_doc::RepeatKind::Grid { h_spacing: 0.0, v_spacing: 0.0, rows: 1, cols: 1, grid_type: grid, flip_rows: false, flip_cols: false };
    let repeat = Node::new(d.alloc_id(), NodeKind::Repeat(vectorcraft_doc::RepeatSpec { kind, source: vec![Arc::new(instance)] }));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, repeat).unwrap();
    let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    assert!(img.pixel(3, 20)[3] > 0, "{:?}", img.pixel(3, 20));
}

#[test]
fn past_the_measured_placements_an_instance_is_culled_by_an_estimate_of_its_art_effects() {
    // Symbol `Spot` is a square from (-1, -1) to (1, 1) with a drop shadow 8 pt to the right, and
    // symbols `Dot` and `Late` each hold an instance of it. 4,100 instances of `Dot`, each turned by
    // another angle, lie far left of the view: one frame measures the effects in the art of at most
    // 4,096 placements of symbols that hold instances. An instance of `Late` at (-5, 20) comes
    // after them: its square lies left of the view, and the shadow reaches into it.
    let mut d = Document::new(100.0, 100.0);
    let mut art = rect_node(&mut d, Rect::new(-1.0, -1.0, 1.0, 1.0), Paint::solid(Color::BLACK), Paint::None, 0.0);
    let shadow = vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: serde_json::json!({"x": 8, "y": 0, "blur": 0}), visible: true };
    art.appearance.effects.push(shadow);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Spot".into(), art: Arc::new(art) });
    for name in ["Dot", "Late"] {
        let spot = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Spot".into(), xf: Affine::IDENTITY });
        let group = Node::group(d.alloc_id(), vec![Arc::new(spot)]);
        d.symbols.push(vectorcraft_doc::Symbol { name: name.into(), art: Arc::new(group) });
    }
    let l = d.layers[0].id;
    for i in 0..4100 {
        let xf = Affine::translate((-1000.0, 50.0)) * Affine::rotate(f64::from(i) * 1e-4);
        let id = d.alloc_id();
        d.insert(Some(l), i as usize, Node::new(id, NodeKind::SymbolInstance { symbol: "Dot".into(), xf })).unwrap();
    }
    let id = d.alloc_id();
    d.insert(Some(l), 4100, Node::new(id, NodeKind::SymbolInstance { symbol: "Late".into(), xf: Affine::translate((-5.0, 20.0)) })).unwrap();
    let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    assert!(img.pixel(3, 20)[3] > 0, "{:?}", img.pixel(3, 20));
}

/// Symbols 0 to 29 each hold two instances of the next, sheared two ways, so that the 2^30
/// instances drawn from the first are all placed differently. Symbol 30 holds a square with a drop
/// shadow. An instance of symbol 0 lies far left of the view. With `effect`, every instance has
/// that effect of its own. The renderer must cull the instance of symbol 0 within 30 s.
fn nested_instances_are_culled_in_bounded_time(effect: Option<vectorcraft_doc::Effect>) {
    let mut d = Document::new(100.0, 100.0);
    let mut art = rect_node(&mut d, Rect::new(0.0, 0.0, 1.0, 1.0), Paint::solid(Color::BLACK), Paint::None, 0.0);
    let shadow = vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: serde_json::json!({"x": 2, "y": 0, "blur": 0}), visible: true };
    art.appearance.effects.push(shadow);
    d.symbols.push(vectorcraft_doc::Symbol { name: "S30".into(), art: Arc::new(art) });
    let instance = |d: &mut Document, symbol: String, xf: Affine| {
        let mut n = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol, xf });
        n.appearance.effects.extend(effect.clone());
        n
    };
    for i in (0..30).rev() {
        let next = format!("S{}", i + 1);
        let mut pair = vec![];
        for xf in [Affine::new([1.0, 0.0, 0.1, 1.0, 0.0, 0.0]), Affine::new([1.0, 0.1, 0.0, 1.0, 2.0, 0.0])] {
            pair.push(Arc::new(instance(&mut d, next.clone(), xf)));
        }
        let group = Node::group(d.alloc_id(), pair);
        d.symbols.push(vectorcraft_doc::Symbol { name: format!("S{i}"), art: Arc::new(group) });
    }
    let first = instance(&mut d, "S0".into(), Affine::translate((-100_000.0, 0.0)));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, first).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut r = Renderer::new();
        let img = r.render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
        tx.send((img.pixel(50, 50), r.stats.culled)).ok();
    });
    let (pixel, culled) = rx.recv_timeout(std::time::Duration::from_secs(30)).expect("the frame within 30 s");
    assert_eq!(pixel, [0, 0, 0, 0]);
    assert!(culled > 0);
}

#[test]
fn nested_instances_with_effects_are_culled_in_bounded_time() {
    nested_instances_are_culled_in_bounded_time(None);
}

#[test]
fn nested_instances_with_geometry_effects_of_their_own_are_culled_in_bounded_time() {
    // Each instance's Transform effect reshapes the art it shows before the effects in that art run.
    let moved = vectorcraft_doc::Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 1}), visible: true };
    nested_instances_are_culled_in_bounded_time(Some(moved));
}

#[test]
fn a_scaled_instance_is_culled_by_the_effects_in_its_art_as_placed() {
    // The art, (-1, -1) to (1, 1), scaled 30 times and centered on (-50, 50): (-80, 20) to
    // (-20, 80), out of view. Its Transform effect doubles the placed art: (-110, -10) to
    // (10, 110).
    let mut d = Document::new(100.0, 100.0);
    let mut art = rect_node(&mut d, Rect::new(-1.0, -1.0, 1.0, 1.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let double = vectorcraft_doc::Effect { id: "distort.transform".into(), params: serde_json::json!({"scaleH": 200, "scaleV": 200}), visible: true };
    art.appearance.effects.push(double);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Dot".into(), art: Arc::new(art) });
    let (id, l) = (d.alloc_id(), d.layers[0].id);
    let xf = Affine::translate((-50.0, 50.0)) * Affine::scale(30.0);
    d.insert(Some(l), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Dot".into(), xf })).unwrap();
    let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    assert_eq!(img.pixel(5, 50), [255, 0, 0, 255]);
}

#[test]
fn an_instance_with_a_geometry_effect_is_culled_by_its_reshaped_art() {
    // The art, (0, 0) to (20, 20), drawn from (-100, 10) is out of view, and the Transform effect
    // moves it 150 pt right, into view.
    let mut d = far_instance(Rect::new(0.0, 0.0, 20.0, 20.0));
    let id = d.layers[0].children().unwrap()[0].id;
    let inst = d.node_mut(id).unwrap();
    inst.kind = NodeKind::SymbolInstance { symbol: "Far".into(), xf: Affine::translate((-100.0, 10.0)) };
    let effect = vectorcraft_doc::Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 150}), visible: true };
    inst.appearance.effects.push(effect);
    assert_eq!(Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default()).pixel(60, 20), [255, 0, 0, 255]);
}

#[test]
fn an_instance_with_a_geometry_effect_is_culled_by_the_effects_in_its_reshaped_art() {
    // The art, (-1, -1) to (1, 1), and the instance at (-11, 20) each have a Transform effect that
    // scales by 4. The instance's effect scales the placed art, (-12, 19) to (-10, 21), to
    // (-15, 16) to (-7, 24), and the art's effect then scales that to (-27, 4) to (5, 36), which
    // reaches into the view.
    let mut d = Document::new(100.0, 100.0);
    let scale = serde_json::json!({"scaleH": 400, "scaleV": 400});
    let effect = || vectorcraft_doc::Effect { id: "distort.transform".into(), params: scale.clone(), visible: true };
    let mut art = rect_node(&mut d, Rect::new(-1.0, -1.0, 1.0, 1.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    art.appearance.effects.push(effect());
    d.symbols.push(vectorcraft_doc::Symbol { name: "Dot".into(), art: Arc::new(art) });
    let (id, l) = (d.alloc_id(), d.layers[0].id);
    let mut instance = Node::new(id, NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Affine::translate((-11.0, 20.0)) });
    instance.appearance.effects.push(effect());
    d.insert(Some(l), 0, instance).unwrap();
    let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    assert_eq!(img.pixel(2, 20), [255, 0, 0, 255]);
}

#[test]
fn view_transform_zooms() {
    let mut d = Document::new(100.0, 100.0);
    let n = rect_node(&mut d, Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(Color::BLACK), Paint::None, 0.0);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let r = Renderer::new().render(&d, 100, 100, Affine::scale(5.0), &RenderOptions::default());
    assert_eq!(r.pixel(45, 45)[3], 255);
    assert_eq!(r.pixel(55, 55)[3], 0);
}

#[test]
fn outline_mode_no_fill() {
    let mut d = Document::new(100.0, 100.0);
    let n = rect_node(&mut d, Rect::new(10.0, 10.0, 90.0, 90.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let r =
        Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions { outline: true, background: Some([255; 4]), ..Default::default() });
    assert_eq!(r.pixel(50, 50), [255, 255, 255, 255]);
    assert!(r.pixel(10, 50)[0] < 200);
}

/// A 100 × 100 document with each kind of path Outline view draws, and the ids of its objects by
/// name: a stroked square, a compound path with a hole, a path and a compound path with a live
/// effect, a guide, a clip group, an Image Trace object showing its outlines, and a path in a layer
/// with Preview off. The clip group holds a path reaching past its clipping path, a compound path,
/// a group of two paths, a nested clip group, a path with a live effect, a hidden path and an
/// unstroked path a tenth of a point wide.
fn outline_doc() -> (Document, Vec<(&'static str, NodeId)>) {
    let mut d = Document::new(100.0, 100.0);
    let mut ids = vec![];
    let black = || Paint::solid(Color::BLACK);
    let offset = |by: f64| vectorcraft_doc::Effect { id: "path.offsetPath".into(), params: serde_json::json!({ "offset": by }), visible: true };
    let compound = |d: &mut Document, outer: Rect, hole: Rect| {
        let children = [outer, hole].map(|r| Arc::new(rect_node(d, r, Paint::None, Paint::None, 0.0))).to_vec();
        let mut n = Node::new(d.alloc_id(), NodeKind::Compound { children, rule: FillRule::EvenOdd });
        n.appearance = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0).into();
        n
    };
    let clipping = |mut n: Node| {
        if let NodeKind::Path { clipping, .. } = &mut n.kind {
            *clipping = true;
        }
        n
    };
    let mut named = |name: &'static str, n: Node| {
        ids.push((name, n.id));
        Arc::new(n)
    };
    let square = named("square", rect_node(&mut d, Rect::new(5.0, 5.0, 25.0, 25.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), black(), 2.0));
    let holed = named("holed", compound(&mut d, Rect::new(30.0, 5.0, 50.0, 25.0), Rect::new(35.0, 10.0, 45.0, 20.0)));
    let mut fx_path = rect_node(&mut d, Rect::new(55.0, 8.0, 70.0, 22.0), black(), Paint::None, 0.0);
    fx_path.appearance.effects.push(offset(2.0));
    let fx_path = named("fx path", fx_path);
    let mut fx_compound = compound(&mut d, Rect::new(75.0, 5.0, 95.0, 25.0), Rect::new(80.0, 10.0, 90.0, 20.0));
    fx_compound.appearance.effects.push(offset(-1.0));
    let fx_compound = named("fx compound", fx_compound);
    let mut guide = Node::path(d.alloc_id(), shapes::line(Point::new(5.0, 30.0), Point::new(95.0, 30.0)), Appearance::default());
    if let NodeKind::Path { guide, .. } = &mut guide.kind {
        *guide = true;
    }
    let guide = named("guide", guide);
    // The clip group.
    let ellipse = Node::path(d.alloc_id(), shapes::ellipse(Rect::new(5.0, 35.0, 55.0, 85.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
    let clip_path = named("clipping path", clipping(ellipse));
    let past = named("past the clip", rect_node(&mut d, Rect::new(0.0, 50.0, 60.0, 60.0), Paint::None, black(), 1.0));
    let member_compound = named("member compound", compound(&mut d, Rect::new(10.0, 65.0, 20.0, 75.0), Rect::new(12.0, 67.0, 18.0, 73.0)));
    let pair_rect = named("pair rect", rect_node(&mut d, Rect::new(25.0, 65.0, 30.0, 75.0), black(), Paint::None, 0.0));
    let pair_ellipse = Node::path(d.alloc_id(), shapes::ellipse(Rect::new(32.0, 65.0, 40.0, 75.0)), Appearance::basic(black(), Paint::None, 0.0));
    let pair_ellipse = named("pair ellipse", pair_ellipse);
    let pair = named("pair", Node::group(d.alloc_id(), vec![pair_rect, pair_ellipse]));
    let nested_clip = named("nested clipping path", clipping(rect_node(&mut d, Rect::new(42.0, 62.0, 54.0, 80.0), Paint::None, Paint::None, 0.0)));
    let nested_path = named("nested path", rect_node(&mut d, Rect::new(44.0, 64.0, 70.0, 78.0), black(), Paint::None, 0.0));
    let nested = named("nested", Node::new(d.alloc_id(), NodeKind::Group { children: vec![nested_clip, nested_path], clip: true }));
    let mut member_fx = rect_node(&mut d, Rect::new(15.0, 40.0, 30.0, 47.0), black(), Paint::None, 0.0);
    member_fx.appearance.effects.push(offset(1.5));
    let member_fx = named("member fx", member_fx);
    let mut hidden = rect_node(&mut d, Rect::new(20.0, 80.0, 40.0, 84.0), Paint::None, black(), 1.0);
    hidden.visible = false;
    let hidden = named("hidden", hidden);
    let tiny = named("tiny", rect_node(&mut d, Rect::new(48.0, 56.0, 48.1, 56.1), black(), Paint::None, 0.0));
    let members = vec![clip_path, past, member_compound, pair, nested, member_fx, hidden, tiny];
    let clip = named("clip group", Node::new(d.alloc_id(), NodeKind::Group { children: members, clip: true }));
    // The Image Trace object: its hidden source, then the traced shapes.
    let mut source = rect_node(&mut d, Rect::new(60.0, 35.0, 95.0, 95.0), Paint::solid(Color::rgb(0.5, 0.5, 0.5)), Paint::None, 0.0);
    source.visible = false;
    let shape_a =
        named("trace shape a", rect_node(&mut d, Rect::new(62.0, 40.0, 80.0, 60.0), Paint::solid(Color::rgb(0.0, 0.5, 0.0)), Paint::None, 0.0));
    let shape_b = Node::path(
        d.alloc_id(),
        shapes::ellipse(Rect::new(70.0, 62.0, 92.0, 84.0)),
        Appearance::basic(Paint::solid(Color::rgb(1.0, 0.5, 0.0)), Paint::None, 0.0),
    );
    let shape_b = named("trace shape b", shape_b);
    let mut traced = Node::group(d.alloc_id(), vec![Arc::new(source), shape_a, shape_b]);
    traced.extras_mut().trace = Some(Box::new(serde_json::json!({ "view": "outlines" })));
    let traced = named("traced", traced);
    let unpreviewed =
        named("unpreviewed", rect_node(&mut d, Rect::new(5.0, 88.0, 40.0, 97.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0));
    let l = d.layers[0].id;
    for n in [square, holed, fx_path, fx_compound, guide, clip, traced] {
        d.insert(Some(l), usize::MAX, Arc::unwrap_or_clone(n)).unwrap();
    }
    let layer = Node::layer(d.alloc_id(), "Preview off", vectorcraft_doc::LayerColor::Preset(1));
    let off = d.insert(None, usize::MAX, layer).unwrap();
    if let Some(NodeKind::Layer { preview, .. }) = d.node_mut(off).map(|n| &mut n.kind) {
        *preview = false;
    }
    d.insert(Some(off), 0, Arc::unwrap_or_clone(unpreviewed)).unwrap();
    (d, ids)
}

/// The id `outline_doc` gave the object `name`.
fn id_of(ids: &[(&'static str, NodeId)], name: &str) -> NodeId {
    ids.iter().find(|(n, _)| *n == name).map(|(_, id)| *id).unwrap_or_else(|| panic!("no object {name}"))
}

/// Outline frames drawn from the geometry a renderer keeps between frames are the frames a new
/// renderer draws, at three views each drawn twice, and after a member of the clip group moves
/// (here with dotted guides). In Preview the Image Trace object's outlines and the layer with
/// Preview off are drawn in outline.
#[test]
fn outline_frames_from_kept_geometry_match_a_new_renderer() {
    let (d, ids) = outline_doc();
    let outline = RenderOptions { outline: true, guide: ([0, 128, 255], true), background: Some([255; 4]), ..Default::default() };
    let screen = RenderOptions { trace_views: true, background: Some([255; 4]), ..Default::default() };
    let views = [Affine::IDENTITY, Affine::scale(4.0) * Affine::translate((-40.0, -35.0)), Affine::translate((6.0, -4.0))];
    let mut moved = d.clone();
    if let Some(n) = moved.node_mut(id_of(&ids, "past the clip")) {
        n.transform(Affine::translate((3.0, 2.0)), false);
    }
    for opts in [&outline, &screen] {
        let mut kept = Renderer::new();
        for (d, view) in views.iter().chain(&views).map(|v| (&d, v)).chain([(&moved, &Affine::IDENTITY)]) {
            let new = Renderer::new().render(d, 100, 100, *view, opts);
            assert_eq!(kept.render(d, 100, 100, *view, opts).pixels, new.pixels, "outline: {}, view {view:?}", opts.outline);
        }
    }
    // Outline view draws the members of the clip group unclipped, the tenth-of-a-point path as a
    // dot, and no fills.
    let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &outline);
    assert!(img.pixel(58, 50)[0] < 200, "the member past the clipping path");
    assert!(img.pixel(48, 56)[0] < 255, "the tenth-of-a-point path");
    assert_eq!(img.pixel(15, 15), [255; 4], "the square's fill");
}

/// Outline view draws paths and compound paths from the geometry the renderer keeps for them,
/// the members of clip groups and the shapes of an Image Trace object showing its outlines
/// included: the next frame draws the same paths. A member of a clip group keeps its live-effect
/// geometry, and the members out of view are culled by bounds the renderer keeps.
#[test]
fn outline_view_draws_paths_from_kept_geometry() {
    let (d, ids) = outline_doc();
    let kept_paths = |r: &Renderer| -> HashMap<NodeId, Arc<BezPath>> { r.geom.values().filter_map(|e| Some((e.node.id, e.path.clone()?))).collect() };
    let outline = RenderOptions { outline: true, ..Default::default() };
    let screen = RenderOptions { trace_views: true, ..Default::default() };
    let in_outline = [
        "square",
        "holed",
        "fx path",
        "fx compound",
        "guide",
        "clipping path",
        "past the clip",
        "member compound",
        "pair rect",
        "pair ellipse",
        "nested clipping path",
        "nested path",
        "member fx",
        "tiny",
        "trace shape a",
        "trace shape b",
        "unpreviewed",
    ];
    for (opts, drawn) in [(&outline, &in_outline[..]), (&screen, &["trace shape a", "trace shape b", "unpreviewed"][..])] {
        let mut r = Renderer::new();
        r.render(&d, 100, 100, Affine::IDENTITY, opts);
        let first = kept_paths(&r);
        r.render(&d, 100, 100, Affine::IDENTITY, opts);
        let second = kept_paths(&r);
        for name in drawn {
            let id = id_of(&ids, name);
            let path = first.get(&id).unwrap_or_else(|| panic!("outline: {}: no path kept for {name}", opts.outline));
            assert!(second.get(&id).is_some_and(|p| Arc::ptr_eq(p, path)), "outline: {}: {name} drawn again from its kept path", opts.outline);
        }
        assert!(!first.contains_key(&id_of(&ids, "hidden")));
    }
    let mut r = Renderer::new();
    r.render(&d, 100, 100, Affine::IDENTITY, &outline);
    let member_fx = d.node(id_of(&ids, "member fx")).map(|n| n as *const Node as usize);
    assert!(member_fx.is_some_and(|key| r.fx_geom.contains_key(&key)), "the member's live-effect geometry is kept");
    // Zoomed in 4× on part of the clip group.
    let mut r = Renderer::new();
    r.render(&d, 100, 100, Affine::scale(4.0) * Affine::translate((-40.0, -35.0)), &outline);
    for name in ["member compound", "pair", "member fx"] {
        let id = id_of(&ids, name);
        assert!(r.geom.values().any(|e| e.node.id == id && e.bounds.is_some() && e.path.is_none()), "{name}: culled by kept bounds");
    }
}

/// An Image Trace object listed in `trace_source` draws its source image on screen whatever its
/// View, also inside a group whose raster effect a renderer keeps; without the list, and in
/// exports, it draws its result. A group that is not an Image Trace object draws its members.
#[test]
fn trace_source_draws_the_source_image_of_the_listed_objects() {
    use vectorcraft_doc::{ImageBlob, ImageObject};
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut d = Document::new(100.0, 100.0);
    d.images.insert("red".into(), ImageBlob::new("image/png", png));
    // A group at `x`: the hidden red source image, then a blue traced square over it.
    let group = |d: &mut Document, x: f64, traced: bool| {
        let xf = Affine::translate((x, 10.0)) * Affine::scale(20.0);
        let im = ImageObject { key: "red".into(), width: 1, height: 1, xf, link: None, placement: Default::default(), pixels: None };
        let mut source = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
        source.visible = false;
        let shape = rect_node(d, Rect::new(x, 10.0, x + 20.0, 30.0), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
        let mut g = Node::group(d.alloc_id(), vec![Arc::new(source), Arc::new(shape)]);
        if traced {
            g.extras_mut().trace = Some(Box::new(serde_json::json!({ "view": "tracingResult" })));
        }
        g
    };
    let plain = group(&mut d, 10.0, true);
    let blurred = group(&mut d, 40.0, true);
    let other = group(&mut d, 70.0, false);
    let ids = Arc::new(vec![plain.id, blurred.id, other.id]);
    let mut holder = Node::group(d.alloc_id(), vec![Arc::new(blurred)]);
    holder.appearance.effects.push(vectorcraft_doc::Effect { id: "blur.gaussian".into(), params: serde_json::json!({ "radius": 1 }), visible: true });
    let l = d.layers[0].id;
    for n in [plain, holder, other] {
        d.insert(Some(l), usize::MAX, n).unwrap();
    }
    let screen = RenderOptions { trace_views: true, background: Some([255; 4]), ..Default::default() };
    let held = RenderOptions { trace_source: Some(ids.clone()), ..screen.clone() };
    let export = RenderOptions { trace_source: Some(ids), background: Some([255; 4]), ..Default::default() };
    let red = |p: [u8; 4]| p[0] > 200 && p[2] < 60;
    let blue = |p: [u8; 4]| p[2] > 200 && p[0] < 60;
    let mut kept = Renderer::new();
    for (opts, source) in [(&screen, false), (&held, true), (&screen, false), (&export, false)] {
        for r in [kept.render(&d, 100, 100, Affine::IDENTITY, opts), Renderer::new().render(&d, 100, 100, Affine::IDENTITY, opts)] {
            for (x, what) in [(20, "the Image Trace object"), (50, "the one in the blurred group")] {
                let p = r.pixel(x, 20);
                assert!(if source { red(p) } else { blue(p) }, "{what} with trace_source {source}: {p:?}");
            }
            assert!(blue(r.pixel(80, 20)), "the plain group draws its square: {:?}", r.pixel(80, 20));
        }
    }
}

#[test]
fn artboards_white_on_pasteboard() {
    let d = Document::new(50.0, 50.0);
    let r = Renderer::new().render(
        &d,
        100,
        100,
        Affine::IDENTITY,
        &RenderOptions { artboards: true, background: Some([30, 30, 30, 255]), ..Default::default() },
    );
    assert_eq!(r.pixel(25, 25), [255, 255, 255, 255]);
    assert_eq!(r.pixel(75, 75), [30, 30, 30, 255]);
}

#[test]
fn region_export_png() {
    let mut d = Document::new(100.0, 100.0);
    let n = rect_node(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), Paint::solid(Color::BLACK), Paint::None, 0.0);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let r = Renderer::new().render_region(&d, d.artboards[0].rect, 2.0, true);
    assert_eq!((r.width, r.height), (200, 200));
    let png = r.to_png().unwrap();
    assert_eq!(&png[1..4], b"PNG");
}

#[test]
fn arrowheads_render() {
    let mut d = Document::new(100.0, 100.0);
    let id = d.alloc_id();
    let mut n =
        Node::path(id, shapes::line(Point::new(10.0, 50.0), Point::new(80.0, 50.0)), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0));
    n.appearance.stroke_mut().unwrap().end_arrow = Some(Arrowhead::Triangle);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let r = render(&d);
    // The arrowhead (past the end point by default: tip at 84) is wider than the 2 pt line.
    assert!(r.pixel(78, 48)[0] < 128);
    assert!(r.pixel(85, 50)[0] > 200, "nothing past the tip");
}

use vectorcraft_doc::Arrowhead;
use vectorcraft_geom::Point;

/// A red 10..90 square masked by a white square covering only its left half (10..50),
/// plus a mid-grey strip (50..70) — rendered both multi- and single-threaded.
fn masked_doc(clip: bool, invert: bool) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let mut n = rect_node(&mut d, Rect::new(10.0, 10.0, 90.0, 90.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let white = rect_node(&mut d, Rect::new(10.0, 10.0, 50.0, 90.0), Paint::solid(Color::WHITE), Paint::None, 0.0);
    let grey = rect_node(&mut d, Rect::new(50.0, 10.0, 70.0, 90.0), Paint::solid(Color::rgb(0.5, 0.5, 0.5)), Paint::None, 0.0);
    let gid = d.alloc_id();
    let art = Node::group(gid, vec![Arc::new(white), Arc::new(grey)]);
    let mut m = vectorcraft_doc::OpacityMask::new(art, clip);
    m.invert = invert;
    n.extras_mut().mask = Some(Box::new(m));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

#[test]
fn opacity_mask_uses_luminance_and_clips() {
    for threads in [0, 2] {
        let mut r = Renderer::new();
        r.threads = threads;
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let img = r.render(&masked_doc(true, false), 100, 100, Affine::IDENTITY, &opts);
        assert_eq!(img.pixel(30, 50), [255, 0, 0, 255], "white mask → opaque ({threads} threads)");
        let mid = img.pixel(60, 50);
        assert!((100..160).contains(&mid[1]), "grey mask → about half opacity, got {mid:?}");
        assert_eq!(img.pixel(80, 50), [255, 255, 255, 255], "clip hides outside the mask art");
    }
}

#[test]
fn opacity_mask_without_clip_and_inverted() {
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let img = Renderer::new().render(&masked_doc(false, false), 100, 100, Affine::IDENTITY, &opts);
    assert_eq!(img.pixel(80, 50), [255, 0, 0, 255], "no clip: outside the mask art stays visible");
    let img = Renderer::new().render(&masked_doc(true, true), 100, 100, Affine::IDENTITY, &opts);
    assert_eq!(img.pixel(30, 50), [255, 255, 255, 255], "inverted white → hidden");
    assert_eq!(img.pixel(80, 50), [255, 0, 0, 255], "inverted clip background → visible");
}

/// #3580: each opacity mask is rendered over its object's pixels, not over the whole frame. 300
/// masked 4 × 4 pt squares on a 1000 × 1000 page draw offscreen in proportion to their bounds, and
/// still draw where their masks show them.
#[test]
fn small_masks_render_over_their_objects_only() {
    let mut d = Document::new(1000.0, 1000.0);
    let l = d.layers[0].id;
    for i in 0..300 {
        let (x, y) = (10.0 + (i % 30) as f64 * 30.0, 10.0 + (i / 30) as f64 * 30.0);
        let square = Rect::new(x, y, x + 4.0, y + 4.0);
        let mut n = rect_node(&mut d, square, Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
        let white = rect_node(&mut d, square, Paint::solid(Color::WHITE), Paint::None, 0.0);
        n.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(white, true)));
        d.insert(Some(l), usize::MAX, n).unwrap();
    }
    let mut r = Renderer::new();
    r.threads = 0;
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    OFFSCREEN_PIXELS.with(|n| n.set(0));
    let img = r.render(&d, 1000, 1000, Affine::IDENTITY, &opts);
    let px = OFFSCREEN_PIXELS.with(|n| n.get());
    // Each square's region is at most 4 + 2 × 2 margin + 1 rounding pixels a side, mask and content.
    assert!(px <= 300 * 2 * 9 * 9, "{px} offscreen pixels for 300 masked 4 × 4 squares");
    assert_eq!(img.pixel(12, 12), [255, 0, 0, 255]);
    assert_eq!(img.pixel(12 + 29 * 30, 12 + 9 * 30), [255, 0, 0, 255]);
    assert_eq!(img.pixel(20, 20), [255, 255, 255, 255]);
}

#[test]
fn disabled_or_outline_mask_draws_unmasked() {
    let mut d = masked_doc(true, false);
    let id = d.layers[0].children().unwrap()[0].id;
    let n = d.node_mut(id).unwrap();
    n.extras_mut().mask.as_mut().unwrap().disabled = true;
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts);
    assert_eq!(img.pixel(80, 50), [255, 0, 0, 255]);
}

#[test]
fn linked_mask_moves_with_object() {
    let mut d = masked_doc(true, false);
    let id = d.layers[0].children().unwrap()[0].id;
    d.node_mut(id).unwrap().transform(Affine::translate((5.0, 0.0)), false);
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts);
    assert_eq!(img.pixel(52, 50), [255, 0, 0, 255], "mask art moved by 5 too");
}

#[test]
fn trim_view_clips_to_artboards() {
    let mut d = Document::new(50.0, 50.0);
    let n = rect_node(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), trim: true, ..Default::default() };
    let r = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts);
    assert_eq!(r.pixel(25, 25), [255, 0, 0, 255]);
    assert_eq!(r.pixel(75, 75), [255, 255, 255, 255]);
    assert_eq!(render(&d).pixel(75, 75), [255, 0, 0, 255]);
}

#[test]
fn drop_shadow_profile_and_cache() {
    let mut d = Document::new(100.0, 100.0);
    let mut n = rect_node(&mut d, Rect::new(20.0, 20.0, 60.0, 60.0), Paint::solid(Color::WHITE), Paint::None, 0.0);
    n.appearance.effects.push(vectorcraft_doc::Effect {
        id: "stylize.dropShadow".into(),
        params: serde_json::json!({"x": 10.0, "y": 10.0, "blur": 4.0, "opacity": 75.0}),
        visible: true,
    });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    for threads in [0, 4] {
        let mut r = Renderer::new();
        r.threads = threads;
        let a = r.render(&d, 100, 100, Affine::IDENTITY, &opts);
        // Inside the shadow (not under the object): 75% black. At its blurred edge: about half.
        let inside = a.pixel(65, 50)[0] as i32;
        assert!((inside - 64).abs() <= 6, "inside {inside}");
        let edge = a.pixel(70, 50)[0] as i32;
        assert!((edge - 160).abs() <= 20, "edge {edge}");
        assert_eq!(a.pixel(85, 50), [255, 255, 255, 255]);
        // Second frame comes from the cache: identical. A pan moves it rigidly.
        let b = r.render(&d, 100, 100, Affine::IDENTITY, &opts);
        assert_eq!(a.pixels, b.pixels);
        let c = r.render(&d, 100, 100, Affine::translate((-5.0, 0.0)), &opts);
        assert_eq!(c.pixel(60, 50), a.pixel(65, 50));
        assert_eq!(c.pixel(65, 50), a.pixel(70, 50));
    }
}

#[test]
fn blend_mode_on_a_single_fill_is_exact_without_a_layer() {
    let mut d = Document::new(100.0, 100.0);
    let red = rect_node(&mut d, Rect::new(0.0, 0.0, 50.0, 100.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let mut over = rect_node(&mut d, Rect::new(25.0, 0.0, 75.0, 100.0), Paint::solid(Color::rgb(0.5, 0.5, 1.0)), Paint::None, 0.0);
    over.blend = vectorcraft_color::BlendMode::Multiply;
    let l = d.layers[0].id;
    d.insert(Some(l), 0, red).unwrap();
    d.insert(Some(l), 1, over).unwrap();
    for threads in [0, 4] {
        let mut r = Renderer::new();
        r.threads = threads;
        let img = r.render(&d, 100, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() });
        let [rr, g, b, _] = img.pixel(35, 50);
        assert!((rr as i32 - 128).abs() <= 2 && g <= 1 && b <= 1, "multiply over red: {:?}", img.pixel(35, 50));
        let [rr, g, b, _] = img.pixel(65, 50);
        assert!((rr as i32 - 128).abs() <= 2 && (g as i32 - 128).abs() <= 2 && b >= 253, "multiply over white");
        assert_eq!(img.pixel(10, 50), [255, 0, 0, 255]);
    }
}

#[test]
fn invalid_dash_pattern_strokes_solid() {
    // A negative dash value makes the pattern invalid (PDF `d`, SVG `stroke-dasharray`): the stroke
    // is drawn solid. [-5, 3] used to send the dasher into an endless loop.
    let line = |dash: Option<Vec<f64>>| {
        let mut d = Document::new(100.0, 100.0);
        let mut bp = BezPath::new();
        bp.move_to((10.0, 50.0));
        bp.line_to((90.0, 50.0));
        let id = d.alloc_id();
        let mut n = Node::path(id, vectorcraft_geom::PathData::from_bezpath(&bp), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 6.0));
        n.appearance.stroke_mut().unwrap().dash = dash.map(|pattern| vectorcraft_doc::Dash { pattern, ..Default::default() });
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        d
    };
    let solid = render(&line(None));
    for pattern in [vec![-5.0, 3.0], vec![3.0, -5.0], vec![4.0, f64::NAN], vec![0.0, 0.0]] {
        let d = line(Some(pattern.clone()));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(render(&d)).ok());
        let img = rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap_or_else(|_| panic!("rendering dash {pattern:?} did not finish"));
        assert_eq!(img.pixels, solid.pixels, "dash {pattern:?}");
    }
}

/// An image that can't be decoded is counted on the thread that tried to decode it: an export
/// compares [`decode_failures`] before and after it draws, so another thread's failure meanwhile is
/// not its own.
#[test]
fn decode_failures_counts_this_threads_images_only() {
    let undecodable = || paint::decode_pixmap(b"\x89PNG\r\n\x1a\n").is_none();
    let before = decode_failures();
    let other = std::thread::spawn(move || {
        let start = decode_failures();
        undecodable() && decode_failures() == start + 1
    });
    assert!(other.join().unwrap(), "counted on the other thread");
    assert_eq!(decode_failures(), before, "the other thread's failure");
    assert!(undecodable());
    assert_eq!(decode_failures(), before + 1, "this thread's failure");
}

/// An image drawn larger than its pixels samples smoothly, or with `smooth_images` off each screen
/// pixel takes its nearest image pixel (Pixel Preview with File Handling › Display Bitmaps as
/// Anti-aliased Images off, #394).
#[test]
fn images_take_their_nearest_pixel_without_smoothing() {
    use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind};
    let img = image::RgbaImage::from_fn(2, 1, |x, _| image::Rgba(if x == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut d = Document::new(40.0, 20.0);
    d.images.insert("bw".into(), ImageBlob::new("image/png", png));
    let id = d.alloc_id();
    let im = ImageObject { key: "bw".into(), width: 2, height: 1, xf: Affine::scale(20.0), link: None, placement: Default::default(), pixels: None };
    d.insert(Some(d.layers[0].id), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    // How many pixels across the image are neither black nor white.
    let greys = |smooth_images| {
        let r = Renderer::new().render(&d, 40, 20, Affine::IDENTITY, &RenderOptions { smooth_images, ..Default::default() });
        (0..40).filter(|x| (16..240).contains(&r.pixel(*x, 10)[0])).count()
    };
    assert!(greys(true) >= 4, "smooth: a ramp from black to white");
    assert_eq!(greys(false), 0, "nearest: black, then white");
}

/// A PNG that says it is `w` × `h` 1-bit gray pixels (its header and no rows).
fn png_header(w: u32, h: u32) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let ihdr: Vec<u8> = [&b"IHDR"[..], &w.to_be_bytes(), &h.to_be_bytes(), &[1, 0, 0, 0, 0]].concat();
    for chunk in [&ihdr[..], b"IDAT\x78\x9c", b"IEND"] {
        out.extend((chunk.len() as u32 - 4).to_be_bytes());
        out.extend(chunk);
        let mut crc = flate2::Crc::new();
        crc.update(chunk);
        out.extend(crc.sum().to_be_bytes());
    }
    out
}

/// A little-endian TIFF that says it is `w` × `h` 8-bit gray pixels in one strip (its header and
/// no pixels).
fn tiff_header(w: u32, h: u32) -> Vec<u8> {
    let mut out = b"II*\0".to_vec();
    out.extend(8u32.to_le_bytes());
    // (tag, type: 3 SHORT or 4 LONG, value), one value each.
    let entries: [(u16, u16, u32); 9] =
        [(256, 4, w), (257, 4, h), (258, 3, 8), (259, 3, 1), (262, 3, 1), (273, 4, 8), (277, 3, 1), (278, 4, h), (279, 4, 0)];
    out.extend((entries.len() as u16).to_le_bytes());
    for (tag, kind, value) in entries {
        out.extend(tag.to_le_bytes());
        out.extend(kind.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(value.to_le_bytes());
    }
    out.extend(0u32.to_le_bytes());
    out
}

/// The decode allowance (2 GB) counts an image's pixels as RGBA, read from its header (#1717): a
/// 1-bit gray image whose 8-bit gray pixels are within it but whose RGBA is over it is not decoded
/// whole, and one a column narrower goes on to decoding. `decode_pixmap` reads the larger one at
/// reduced size; this header has no rows to read, so it draws nothing, counted for an export to
/// report.
#[test]
fn an_image_too_large_as_rgba_is_not_decoded() {
    let over = paint::decode_within_allowance(&png_header(23_171, 23_170));
    assert!(matches!(over, Err(image::ImageError::Limits(_))), "{over:?}");
    let failed = decode_failures();
    assert!(paint::decode_pixmap(&png_header(23_171, 23_170)).is_none());
    assert!(decode_failures() > failed);
    // Within the allowance: decoded, and its missing rows can't be read.
    let within = paint::decode_within_allowance(&png_header(23_170, 23_170));
    assert!(matches!(within, Err(image::ImageError::Decoding(_))), "{within:?}");
    // A header whose size as RGBA does not fit in 64 bits gets the same limit error.
    let huge = paint::decode_within_allowance(&tiff_header(1 << 31, 1 << 31));
    assert!(matches!(huge, Err(image::ImageError::Limits(_))), "{huge:?}");
}

/// A PNG of `w` × `h` noisy pixels cut off halfway through its bytes: its header reads, and its
/// pixels can't be decoded.
fn truncated_png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let v = (x * 7919 + y * 104_729).wrapping_mul(2_654_435_761).to_le_bytes();
        image::Rgba([v[0], v[1], v[2], 255])
    });
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png.truncate(png.len() / 2);
    png
}

/// An image whose pixels can't be decoded is tried once: later renders find the store's note of it,
/// and each render counts it in `decode_failures` for an export to report. On screen
/// (`mark_undecoded_images`) it draws as a crossed gray box; an export draws nothing there.
#[test]
fn an_undecodable_image_is_tried_once_and_marked_on_screen() {
    use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind};
    let png = truncated_png(61, 47);
    assert_eq!(image::ImageReader::new(std::io::Cursor::new(&png)).with_guessed_format().unwrap().into_dimensions().unwrap(), (61, 47));
    let mut d = Document::new(80.0, 60.0);
    d.images.insert("cut".into(), ImageBlob::new("image/png", png));
    let id = d.alloc_id();
    let im = ImageObject {
        key: "cut".into(),
        width: 61,
        height: 47,
        xf: Affine::translate((10.0, 5.0)),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    d.insert(Some(d.layers[0].id), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    // The alpha summed over the image's frame.
    let alpha = |r: &Rendered| (10..71).flat_map(|x| (5..52).map(move |y| u64::from(r.pixel(x, y)[3]))).sum::<u64>();
    let attempts = || paint::DECODE_ATTEMPTS.with(std::cell::Cell::get);
    let (tried, failed) = (attempts(), decode_failures());
    let screen = RenderOptions { mark_undecoded_images: true, ..Default::default() };
    let mut r = Renderer::new();
    for render in 1..=2 {
        let shown = r.render(&d, 80, 60, Affine::IDENTITY, &screen);
        assert!(alpha(&shown) > 0, "render {render}: the crossed box shows");
        assert_eq!(decode_failures() - failed, render, "render {render}: counted");
    }
    assert_eq!(attempts() - tried, 1, "tried once");
    let export = Renderer::new().render(&d, 80, 60, Affine::IDENTITY, &RenderOptions::default());
    assert_eq!(alpha(&export), 0, "an export draws nothing there");
    assert_eq!((attempts() - tried, decode_failures() - failed), (1, 3), "and counts it without decoding it again");
}

/// The rasters a renderer keeps of an object's raster effects are made again when
/// `mark_undecoded_images` changes: a blurred image whose pixels can't be decoded shows its blurred
/// mark only while the option is on.
#[test]
fn raster_effects_follow_the_undecoded_image_mark() {
    use vectorcraft_doc::{Effect, ImageBlob, ImageObject, NodeKind};
    let mut d = Document::new(80.0, 60.0);
    d.images.insert("cut".into(), ImageBlob::new("image/png", truncated_png(43, 31)));
    let id = d.alloc_id();
    let im = ImageObject {
        key: "cut".into(),
        width: 43,
        height: 31,
        xf: Affine::translate((15.0, 12.0)),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let mut n = Node::new(id, NodeKind::Image(Box::new(im)));
    n.appearance.effects.push(Effect { id: "blur.gaussian".into(), params: serde_json::json!({"radius": 2.0}), visible: true });
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    let alpha = |r: &Rendered| r.pixels.chunks(4).map(|p| u64::from(p[3])).sum::<u64>();
    let mut r = Renderer::new();
    for mark in [true, false, true] {
        let opts = RenderOptions { mark_undecoded_images: mark, ..Default::default() };
        let a = alpha(&r.render(&d, 80, 60, Affine::IDENTITY, &opts));
        assert_eq!(a > 0, mark, "mark_undecoded_images {mark}: alpha {a}");
    }
}

/// A gray image within the allowance draws its gray levels.
#[test]
fn a_gray_image_draws_its_gray_levels() {
    use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind};
    let img = image::GrayImage::from_fn(2, 1, |x, _| image::Luma([if x == 0 { 40 } else { 200 }]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut d = Document::new(40.0, 20.0);
    d.images.insert("gray".into(), ImageBlob::new("image/png", png));
    let id = d.alloc_id();
    let im =
        ImageObject { key: "gray".into(), width: 2, height: 1, xf: Affine::scale(20.0), link: None, placement: Default::default(), pixels: None };
    d.insert(Some(d.layers[0].id), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    let r = Renderer::new().render(&d, 40, 20, Affine::IDENTITY, &RenderOptions { smooth_images: false, ..Default::default() });
    assert_eq!((r.pixel(5, 10), r.pixel(35, 10)), ([40, 40, 40, 255], [200, 200, 200, 255]));
}

/// Runs `f` with the decode allowance of one image on this thread set to `bytes`.
fn with_allowance<T>(bytes: u64, f: impl FnOnce() -> T) -> T {
    paint::TEST_ALLOWANCE.with(|c| c.set(Some(bytes)));
    let out = f();
    paint::TEST_ALLOWANCE.with(|c| c.set(None));
    out
}

/// `img` encoded as `format`.
fn encoded(img: image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), format).unwrap();
    out
}

/// A 16-bit PNG past the decode allowance is read a row at a time at the largest size within the
/// allowance and draws its colors; the file's own size is kept.
#[test]
fn a_png_past_the_allowance_draws_at_reduced_size() {
    use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind};
    let img = image::ImageBuffer::<image::Rgba<u16>, _>::from_fn(64, 48, |x, _| {
        if x < 32 { image::Rgba([51400, 10280, 10280, 65535]) } else { image::Rgba([10280, 10280, 51400, 65535]) }
    });
    let png = encoded(image::DynamicImage::ImageRgba16(img), image::ImageFormat::Png);
    // 64 × 48 is 12,288 bytes as RGBA; 36 × 27 is the largest within 4,096.
    assert_eq!(with_allowance(4096, || paint::reduced_size(&png)), Some((36, 27)));
    assert_eq!(paint::reduced_size(&png), None, "within the 2 GB allowance");
    let mut d = Document::new(64.0, 48.0);
    d.images.insert("over".into(), ImageBlob::new("image/png", png));
    let id = d.alloc_id();
    let im = ImageObject { key: "over".into(), width: 64, height: 48, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    d.insert(Some(d.layers[0].id), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    let failed = decode_failures();
    // As on screen, where an image that can't be decoded draws as a crossed box.
    let screen = RenderOptions { smooth_images: false, mark_undecoded_images: true, ..Default::default() };
    let r = with_allowance(4096, || Renderer::new().render(&d, 64, 48, Affine::IDENTITY, &screen));
    assert_eq!(decode_failures(), failed);
    assert_eq!((r.pixel(8, 24), r.pixel(56, 24)), ([200, 40, 40, 255], [40, 40, 200, 255]));
    // 16 × 16 at 16 bits a sample is 2,048 bytes decoded and 1,024 as RGBA: past 1,500 as decoded
    // and within it as RGBA, it is read a row at a time at its own size.
    let small = image::ImageBuffer::<image::Rgba<u16>, _>::from_pixel(16, 16, image::Rgba([51400, 10280, 10280, 65535]));
    let small = encoded(image::DynamicImage::ImageRgba16(small), image::ImageFormat::Png);
    assert_eq!(with_allowance(1500, || paint::reduced_size(&small)), Some((16, 16)));
    let pm = with_allowance(1500, || paint::decode_pixmap(&small)).expect("read a row at a time");
    let c = pm.sample(3, 3);
    assert_eq!((pm.width(), pm.height(), [c.r, c.g, c.b, c.a]), (16, 16, [200, 40, 40, 255]));
    // A row is read only while the reader's row buffers, 32 bytes a column, fit the allowance:
    // 128 columns do within 4,096 bytes, and 129 columns are not decoded (their samples, 8 bytes a
    // pixel, are past it too).
    let wide = |w: u32| {
        let img = image::ImageBuffer::<image::Rgba<u16>, _>::from_pixel(w, 6, image::Rgba([51400, 10280, 10280, 65535]));
        encoded(image::DynamicImage::ImageRgba16(img), image::ImageFormat::Png)
    };
    assert_eq!(with_allowance(4096, || paint::reduced_size(&wide(128))), Some((128, 6)));
    assert_eq!(with_allowance(4096, || paint::reduced_size(&wide(129))), None);
    // An image of more pixels than the allowance has bytes is not read: 128 × 32 8-bit gray pixels
    // are drawn at 64 × 16 within 4,096 bytes, and 128 × 33 are not.
    let gray = |h: u32| encoded(image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(128, h, image::Luma([90]))), image::ImageFormat::Png);
    assert_eq!(with_allowance(4096, || paint::reduced_size(&gray(32))), Some((64, 16)));
    assert_eq!(with_allowance(4096, || paint::reduced_size(&gray(33))), None);
}

/// A JPEG whose RGB samples are within the allowance but whose RGBA is past it is decoded whole
/// and shrunk to the largest size within the allowance; an image whose decoded samples are past it
/// and can't be read a row at a time (a GIF, 4 bytes a pixel) is still not decoded, counted, and
/// noted as undecodable.
#[test]
fn a_jpeg_past_the_allowance_as_rgba_is_shrunk_and_a_gif_past_it_is_not_decoded() {
    let jpeg = encoded(image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(80, 60, image::Rgb([30, 120, 200]))), image::ImageFormat::Jpeg);
    // 14,400 bytes of RGB, 19,200 as RGBA: 73 × 54 is the largest within 16,000.
    let pm = with_allowance(16_000, || paint::decode_pixmap(&jpeg)).expect("decoded at reduced size");
    assert_eq!((pm.width(), pm.height()), (73, 54));
    let c = pm.sample(36, 27);
    assert!(c.r.abs_diff(30) <= 4 && c.g.abs_diff(120) <= 4 && c.b.abs_diff(200) <= 4 && c.a == 255, "{c:?}");
    // A placed document's preview asks for 20 × 15: decoded straight at that size.
    let pm = with_allowance(16_000, || paint::decode_pixmap_fit(&jpeg, 20, 15)).expect("decoded at the size asked for");
    assert_eq!((pm.width(), pm.height()), (20, 15));
    let gif =
        encoded(image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(80, 60, image::Rgba([30, 120, 200, 255]))), image::ImageFormat::Gif);
    assert_eq!(with_allowance(16_000, || paint::reduced_size(&gif)), None);
    let failed = decode_failures();
    let why = with_allowance(16_000, || paint::decode_pixmap_or_why(&gif)).err();
    assert_eq!(why, Some(paint::Undecoded::Unreadable), "noted as undecodable");
    assert_eq!(decode_failures(), failed + 1);
}

/// An image past the allowance is decoded at reduced size only while the result and what reading
/// it takes (the image decoded whole, or the PNG reader's row buffers) fit beside the images
/// paints hold; when they don't, it isn't decoded and is not noted as undecodable, and it is
/// decoded once there is room.
#[test]
fn a_reduced_decode_counts_what_reading_takes_beside_the_images_paints_hold() {
    /// Runs `f` with room for `bytes` beside the images paints hold.
    fn with_room<T>(bytes: u64, f: impl FnOnce() -> T) -> T {
        paint::TEST_ROOM.with(|c| c.set(Some(bytes)));
        let out = f();
        paint::TEST_ROOM.with(|c| c.set(None));
        out
    }
    // Decoded whole: 14,400 bytes of RGB and a 16-row band of 8,960 bytes, beside a 73 × 54 result
    // of 15,768 bytes.
    let jpeg = encoded(image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(80, 60, image::Rgb([30, 120, 200]))), image::ImageFormat::Jpeg);
    // Read a row at a time: 64 columns of 32 bytes in the reader's row buffers, beside a 36 × 27
    // result of 3,888 bytes.
    let png = encoded(
        image::DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(64, 48, image::Rgba([51400, 10280, 10280, 65535]))),
        image::ImageFormat::Png,
    );
    for (bytes, allowance, room, size) in [(&jpeg, 16_000, 15_768 + 14_400 + 8_960, (73, 54)), (&png, 4_096, 3_888 + 2_048, (36, 27))] {
        let failed = decode_failures();
        let none = with_allowance(allowance, || with_room(room - 1, || paint::decode_pixmap_or_why(bytes)));
        assert_eq!(none.err(), Some(paint::Undecoded::NoRoom), "not decoded with a byte too few, and tried again later");
        assert_eq!(decode_failures(), failed + 1);
        let pm = with_allowance(allowance, || with_room(room, || paint::decode_pixmap(bytes))).expect("decoded once there is room");
        assert_eq!((pm.width(), pm.height()), size);
    }
}

/// Art Optimized exports are supersampled (#983): two red shapes meeting along a diagonal leave no
/// seam of the white background between them, where shape-by-shape smoothing shows one; the outer
/// edges stay smooth.
#[test]
fn supersampled_art_leaves_no_seam_between_shapes_that_meet() {
    let red = || Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    // Two triangles sharing the diagonal of the square (10, 10)–(90, 90).
    for pts in [[(10.0, 10.0), (90.0, 10.0), (90.0, 90.0)], [(10.0, 10.0), (90.0, 90.0), (10.0, 90.0)]] {
        let mut b = vectorcraft_geom::BezPath::new();
        b.move_to(pts[0]);
        b.line_to(pts[1]);
        b.line_to(pts[2]);
        b.close_path();
        let id = d.alloc_id();
        d.insert(Some(layer), 0, Node::path(id, vectorcraft_geom::PathData::from_bezpath(&b), Appearance::basic(red(), Paint::None, 0.0))).unwrap();
    }
    let draw = |supersample: bool| {
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), precise: true, supersample, ..Default::default() };
        Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts)
    };
    // The palest pixel on the shared diagonal, away from the corners.
    let palest = |r: &Rendered| (20..80).map(|i| r.pixel(i, i)[1].max(r.pixel(i + 1, i)[1])).max().unwrap();
    assert!(palest(&draw(false)) > 20, "shape-by-shape smoothing shows the seam");
    assert_eq!(palest(&draw(true)), 0, "supersampled: no seam");
    // The outer edge is still smoothed: a pixel half covered is between white and red.
    let ss = draw(true);
    let edge = ss.pixel(50, 9);
    assert!(edge == [255, 255, 255, 255] || (edge[1] > 0 && edge[1] < 255) || ss.pixel(50, 10)[1] < 255, "{edge:?}");
    // The factor fits the size and the renderer's limits.
    assert_eq!(supersample_factor(100, 100), Some(4));
    assert_eq!(supersample_factor(2000, 2000), Some(3));
    assert_eq!(supersample_factor(3000, 3000), Some(2));
    assert_eq!(supersample_factor(5000, 5000), None);
    assert_eq!(supersample_factor(40_000, 10), None);
}

/// A supersampled render draws Outline view's lines one output pixel wide, as an unsupersampled
/// one does, instead of a quarter of a pixel: the line down x = 50.5 draws darker than 64 of 255
/// either way, and no wider than one pixel. So do the outlines of type and a dotted guide along
/// y = 90.5.
#[test]
fn supersampled_outlines_stay_one_pixel_wide() {
    let _fonts = crate::tests_fontchange::fonts_steady();
    let mut d = Document::new(100.0, 100.0);
    let style = vectorcraft_doc::text::CharStyle { size: 30.0, ..Default::default() };
    let text = vectorcraft_doc::text::TextObject::point(vectorcraft_geom::Point::new(60.0, 80.0), "H", style);
    let t = Node::new(d.alloc_id(), NodeKind::Text(Box::new(text)));
    d.insert(Some(d.layers[0].id), 0, t).unwrap();
    let n = rect_node(&mut d, Rect::new(20.0, 20.0, 50.5, 80.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    let mut guide = Node::path(d.alloc_id(), shapes::line(Point::new(5.0, 90.5), Point::new(95.0, 90.5)), Appearance::default());
    if let NodeKind::Path { guide, .. } = &mut guide.kind {
        *guide = true;
    }
    d.insert(Some(d.layers[0].id), 1, guide).unwrap();
    let draw = |supersample: bool| {
        let opts = RenderOptions {
            background: Some([255, 255, 255, 255]),
            outline: true,
            guide: ([0, 0, 0], true),
            precise: true,
            supersample,
            ..Default::default()
        };
        Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts)
    };
    let darkest = |r: &Rendered| (48..=52).map(|x| r.pixel(x, 50)[0]).min().unwrap();
    assert!(darkest(&draw(false)) < 64, "unsupersampled: {}", darkest(&draw(false)));
    assert!(darkest(&draw(true)) < 64, "supersampled: {}", darkest(&draw(true)));
    let type_darkest = |r: &Rendered| (56..=95).flat_map(|x| (45..=85).map(move |y| (x, y))).map(|(x, y)| r.pixel(x, y)[0]).min().unwrap();
    assert!(type_darkest(&draw(false)) < 64, "unsupersampled type: {}", type_darkest(&draw(false)));
    assert!(type_darkest(&draw(true)) < 64, "supersampled type: {}", type_darkest(&draw(true)));
    let guide_darkest = |r: &Rendered| (10..=90).flat_map(|x| (89..=92).map(move |y| (x, y))).map(|(x, y)| r.pixel(x, y)[0]).min().unwrap();
    assert!(guide_darkest(&draw(false)) < 64, "unsupersampled guide: {}", guide_darkest(&draw(false)));
    assert!(guide_darkest(&draw(true)) < 64, "supersampled guide: {}", guide_darkest(&draw(true)));
    // One pixel wide, not wider: across each line, the pixels darker than `dark` are one pixel. The
    // path lies on pixel centers; the type's stems do not, and the guide's dots are 1.5 pixels
    // wide, so their edges are softer.
    let widest = |values: Vec<u8>, dark: u8| {
        let mut runs = vec![0usize];
        for v in values {
            if v < dark {
                *runs.last_mut().unwrap() += 1;
            } else if runs.last() != Some(&0) {
                runs.push(0);
            }
        }
        runs.into_iter().max().unwrap()
    };
    for supersample in [false, true] {
        let r = draw(supersample);
        let row = |y: u32, xs: std::ops::RangeInclusive<u32>| xs.map(|x| r.pixel(x, y)[0]).collect::<Vec<_>>();
        assert_eq!(widest(row(50, 44..=56), 200), 1, "path, supersample {supersample}: {:?}", row(50, 44..=56));
        // Below the H's crossbar, across its two stems.
        assert_eq!(widest(row(76, 56..=95), 128), 1, "type, supersample {supersample}: {:?}", row(76, 56..=95));
        let guide = (10..=90).map(|x| widest((86..=95).map(|y| r.pixel(x, y)[0]).collect(), 128)).max().unwrap();
        assert_eq!(guide, 1, "guide, supersample {supersample}");
    }
}

/// #1613: a zero-length subpath (`m` then `L` to the same point, as `.ai` files write dots) is a
/// dot as wide as the stroke: a disc under a round cap, a square under a projecting one, nothing
/// under a butt cap (PDF 32000-1 8.5.3.2, SVG 2 "Path data" zero-length segments).
#[test]
fn zero_length_subpaths_draw_dots_with_their_cap() {
    let dots = |cap: vectorcraft_doc::LineCap| {
        let mut d = Document::new(100.0, 100.0);
        let mut bp = BezPath::new();
        for at in [(30.0, 50.0), (70.0, 50.0)] {
            bp.move_to(at);
            bp.line_to(at);
        }
        let id = d.alloc_id();
        let mut n = Node::path(id, vectorcraft_geom::PathData::from_bezpath(&bp), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 10.0));
        n.appearance.stroke_mut().unwrap().cap = cap;
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        render(&d)
    };
    let (black, white) = ([0, 0, 0, 255], [255, 255, 255, 255]);
    let round = dots(vectorcraft_doc::LineCap::Round);
    for x in [30, 70] {
        assert_eq!(round.pixel(x, 50), black, "round dot at {x}");
        assert_eq!(round.pixel(x + 2, 52), black, "inside the round dot at {x}");
        assert_eq!(round.pixel(x + 4, 54), white, "the round dot's corner at {x}");
    }
    let square = dots(vectorcraft_doc::LineCap::Square);
    for x in [30, 70] {
        assert_eq!(square.pixel(x, 50), black, "square dot at {x}");
        assert_eq!(square.pixel(x + 4, 54), black, "the square dot's corner at {x}");
        assert_eq!(square.pixel(x + 6, 50), white, "past the square dot at {x}");
    }
    let butt = dots(vectorcraft_doc::LineCap::Butt);
    assert_eq!(butt.pixels.iter().filter(|&&p| p != 255).count(), 0, "a butt cap draws no dot");
}

/// #2788: a dashed solid stroke is one paint, so its object opacity folds into its colour instead
/// of compositing a viewport-wide layer; the result matches the layered one, overlapping dashes
/// included.
#[test]
fn dashed_stroke_opacity_folds_into_its_colour() {
    let dashed = |d: &mut Document, object: f32, stroke: f32| {
        let mut n = rect_node(d, Rect::new(20.0, 20.0, 80.0, 60.0), Paint::None, Paint::solid(Color::rgb(0.2, 0.4, 0.9)), 6.0);
        let st = n.appearance.stroke_mut().unwrap();
        st.dash = Some(vectorcraft_doc::Dash { pattern: vec![4.0, 1.0], ..Default::default() });
        st.cap = vectorcraft_doc::LineCap::Round;
        st.opacity = stroke;
        n.opacity = object;
        n
    };
    let doc = |object: f32, stroke: f32| {
        let mut d = Document::new(100.0, 100.0);
        let n = dashed(&mut d, object, stroke);
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        d
    };
    let mut d = Document::new(100.0, 100.0);
    assert_eq!(painted_items(&dashed(&mut d, 0.6, 1.0)), 1, "a dashed stroke takes the folded fast path");
    let mut brushed = dashed(&mut d, 0.6, 1.0);
    brushed.appearance.stroke_mut().unwrap().brush = Some("Calligraphic".into());
    assert_ne!(painted_items(&brushed), 1, "a dashed brush stroke draws its pieces one by one: no folding");
    let folded = render(&doc(0.6, 1.0));
    let layered = render(&doc(1.0, 0.6));
    assert_ne!(folded.pixel(20, 40), [255, 255, 255, 255], "the stroke is drawn");
    for (a, b) in folded.pixels.chunks(4).zip(layered.pixels.chunks(4)) {
        assert!(a.iter().zip(b).all(|(x, y)| (*x as i32 - *y as i32).abs() <= 1), "folded {a:?} vs layered {b:?}");
    }
}
