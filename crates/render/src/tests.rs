use super::*;
use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::shapes;

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
fn encoders_report_a_bad_pixel_buffer_instead_of_panicking() {
    // A buffer that doesn't match width × height used to panic in `to_png` ("size").
    let r = Rendered { width: 4, height: 4, pixels: vec![0; 8] };
    assert!(r.to_png().is_err());
    assert!(r.to_jpeg(90).is_err());
    assert!(r.to_webp().is_err());
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
    n.mask = Some(Box::new(m));
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

#[test]
fn disabled_or_outline_mask_draws_unmasked() {
    let mut d = masked_doc(true, false);
    let id = d.layers[0].children().unwrap()[0].id;
    let n = d.node_mut(id).unwrap();
    n.mask.as_mut().unwrap().disabled = true;
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
