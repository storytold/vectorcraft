//! A long stroke without a width profile is outlined in and around the view only (`crop_line`,
//! and for dashes `dash_kept`), and draws the same pixels as when it is outlined whole.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, LineCap, LineJoin, Node, StrokeLayer, WidthProfile};
use vectorcraft_geom::{PathData, Point, Vec2};

use crate::renderer::{CROP_STROKES, crop_line};
use crate::*;

const SIZE: u32 = 200;

/// A document holding `path` stroked with `st` and no fill.
fn doc(path: &BezPath, st: StrokeLayer) -> Document {
    doc_with(path, vec![AppearanceItem::Stroke(st)])
}

/// A document holding `path` painted with `items`.
fn doc_with(path: &BezPath, items: Vec<AppearanceItem>) -> Document {
    let mut d = Document::new(1000.0, 1000.0);
    let n = Node::path(d.alloc_id(), PathData::from_bezpath(path), Appearance { items, ..Default::default() });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn stroke(width: f64, join: LineJoin, cap: LineCap) -> StrokeLayer {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), width);
    st.join = join;
    st.cap = cap;
    st.miter_limit = 10.0;
    st
}

/// The view that shows `at` in the middle of the 200×200 frame.
fn view_at(at: Point) -> Affine {
    Affine::translate((100.0 - at.x, 100.0 - at.y))
}

fn render(r: &mut Renderer, d: &Document, at: Point) -> Rendered {
    r.render(d, SIZE, SIZE, view_at(at), &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

/// What a renderer that outlines every stroke whole draws.
fn whole(d: &Document, at: Point) -> Rendered {
    CROP_STROKES.with(|c| c.set(false));
    let mut r = Renderer::new();
    let img = render(&mut r, d, at);
    CROP_STROKES.with(|c| c.set(true));
    assert!(r.strokes.values().all(|e| e.crop.is_none()), "the reference outlines the stroke whole");
    img
}

/// 3,000 anchors 8 pt apart, up and down by 30 pt, every third segment curved: sharp miter joins.
fn zigzag() -> BezPath {
    let mut bp = BezPath::new();
    let at = |i: usize| Point::new(i as f64 * 8.0, if i.is_multiple_of(2) { 0.0 } else { 30.0 });
    bp.move_to(at(0));
    for i in 1..3000 {
        if i % 3 == 0 {
            bp.curve_to(at(i - 1) + Vec2::new(6.0, 0.0), at(i) + Vec2::new(-6.0, 10.0), at(i));
        } else {
            bp.line_to(at(i));
        }
    }
    bp
}

#[test]
fn a_long_profile_stroke_keeps_its_widths_when_outlined_near_the_view() {
    let mut lens = stroke(16.0, LineJoin::Miter, LineCap::Square);
    lens.profile = WidthProfile::preset("lens");
    let mut taper = stroke(12.0, LineJoin::Round, LineCap::Round);
    taper.profile = WidthProfile::preset("taperEnd");
    let mut asymmetric = stroke(12.0, LineJoin::Miter, LineCap::Square);
    asymmetric.profile = Some(WidthProfile { points: vec![(0.0, 2.0, 0.1), (1.0, 0.1, 2.0)] });
    let mut two = zigzag();
    two.move_to(Point::new(0.0, 100.0));
    for i in 1..3000 {
        two.line_to(Point::new(i as f64 * 8.0, 100.0));
    }
    for (name, d, at, cropped) in [
        ("open zigzag, lens at middle", doc(&zigzag(), lens), Point::new(12_000.0, 15.0), true),
        ("open zigzag, taper at start", doc(&zigzag(), taper.clone()), Point::new(0.0, 15.0), true),
        ("open zigzag, taper at end", doc(&zigzag(), taper.clone()), Point::new(23_990.0, 15.0), true),
        ("open zigzag, asymmetric profile", doc(&zigzag(), asymmetric), Point::new(12_000.0, 15.0), true),
        ("two open subpaths", doc(&two, taper.clone()), Point::new(12_000.0, 60.0), true),
        ("closed ring at seam", doc(&ring(4000, 5000.0), taper.clone()), Point::new(5000.0, 0.0), false),
        ("closed ring away from seam", doc(&ring(4000, 5000.0), taper), Point::new(0.0, 5000.0), false),
    ] {
        let mut renderer = Renderer::new();
        let near = render(&mut renderer, &d, at);
        assert_eq!(renderer.strokes.values().any(|e| e.crop.is_some()), cropped, "{name}: crop choice");
        let full = whole(&d, at);
        let mismatched: Vec<_> = near
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .zip(full.pixels.as_chunks::<4>().0.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(8)
            .map(|(i, (a, b))| (i % SIZE as usize, i / SIZE as usize, a.to_vec(), b.to_vec()))
            .collect();
        assert!(mismatched.is_empty(), "{name}: pixels differ at {mismatched:?}");
    }
}

/// A closed polygon of `n` anchors round a circle of radius `r` about the origin.
fn ring(n: usize, r: f64) -> BezPath {
    let mut bp = BezPath::new();
    for i in 0..n {
        let t = i as f64 / n as f64 * std::f64::consts::TAU;
        let p = Point::new(r * t.cos(), r * t.sin());
        if i == 0 { bp.move_to(p) } else { bp.line_to(p) }
    }
    bp.close_path();
    bp
}

#[test]
fn a_long_stroke_draws_the_same_pixels_outlined_near_the_view() {
    let mut inside = stroke(8.0, LineJoin::Round, LineCap::Butt);
    inside.align = vectorcraft_doc::StrokeAlign::Inside;
    // The ring starts at (5000, 0): the second view is on its start and end.
    let cases = [
        ("zigzag, miter joins, square caps", doc(&zigzag(), stroke(6.0, LineJoin::Miter, LineCap::Square)), Point::new(12_000.0, 15.0), true),
        ("ring, round joins", doc(&ring(4000, 5000.0), stroke(12.0, LineJoin::Round, LineCap::Round)), Point::new(0.0, 5000.0), true),
        ("ring through its start, bevel joins", doc(&ring(4000, 5000.0), stroke(5.0, LineJoin::Bevel, LineCap::Butt)), Point::new(5000.0, 0.0), true),
        ("ring, inside stroke", doc(&ring(4000, 5000.0), inside), Point::new(-5000.0, 0.0), true),
    ];
    for (what, d, at, crops) in &cases {
        let mut r = Renderer::new();
        let img = render(&mut r, d, *at);
        assert_eq!(r.strokes.values().any(|e| e.crop.is_some()), *crops, "{what}: outlined near the view");
        assert!(img.pixels == whole(d, *at).pixels, "{what}: the same pixels as the whole outline");
        // A view inside the region kept around the first one reuses the outline; one beyond it
        // outlines the stroke again.
        for step in [Point::new(60.0, 40.0), Point::new(450.0, 0.0), Point::new(-900.0, 300.0)] {
            let next = *at + step.to_vec2();
            assert!(render(&mut r, d, next).pixels == whole(d, next).pixels, "{what}, moved by {step:?}: the same pixels");
        }
    }
}

fn dashed(mut st: StrokeLayer, pattern: &[f64], offset: f64, align_corners: bool) -> StrokeLayer {
    st.dash = Some(vectorcraft_doc::Dash { pattern: pattern.to_vec(), offset, align_corners });
    st
}

/// A document holding `path` filled red and stroked with `st`.
fn filled(path: &BezPath, st: StrokeLayer) -> Document {
    let fill = AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(Paint::solid(Color::rgb(1.0, 0.0, 0.0))));
    doc_with(path, vec![fill, AppearanceItem::Stroke(st)])
}

#[test]
fn a_long_dashed_stroke_draws_the_same_pixels_outlined_near_the_view() {
    let round = stroke(6.0, LineJoin::Round, LineCap::Round);
    let mut two_rings = ring(4000, 5000.0);
    two_rings.extend(ring(3000, 4980.0).iter());
    let cases = [
        (
            "zigzag, miter joins, square caps",
            doc(&zigzag(), dashed(stroke(6.0, LineJoin::Miter, LineCap::Square), &[30.0, 12.0], 7.0, false)),
            Point::new(12_000.0, 15.0),
        ),
        (
            "ring through its start, a dash across it",
            doc(&ring(4000, 5000.0), dashed(round.clone(), &[40.0, 25.0], 0.0, false)),
            Point::new(5000.0, 0.0),
        ),
        ("ring, dots", doc(&ring(4000, 5000.0), dashed(round.clone(), &[0.0, 9.0], 0.0, false)), Point::new(0.0, 5000.0)),
        ("ring filled, dashed", filled(&ring(4000, 5000.0), dashed(round.clone(), &[15.0, 10.0, 0.0, 10.0], 4.0, false)), Point::new(-5000.0, 0.0)),
        (
            "zigzag, dashes fitted to corners",
            doc(&zigzag(), dashed(stroke(4.0, LineJoin::Bevel, LineCap::Butt), &[5.0, 3.0], 0.0, true)),
            Point::new(8000.0, 15.0),
        ),
        ("two subpaths, dashed", doc(&two_rings, dashed(round.clone(), &[25.0, 15.0], 0.0, false)), Point::new(0.0, -4990.0)),
        ("a pattern too fine, drawn solid", doc(&zigzag(), dashed(round, &[0.001, 0.001], 0.0, false)), Point::new(12_000.0, 15.0)),
    ];
    for (what, d, at) in &cases {
        let mut r = Renderer::new();
        let img = render(&mut r, d, *at);
        assert!(r.strokes.values().any(|e| e.crop.is_some()), "{what}: outlined near the view");
        assert!(img.pixels == whole(d, *at).pixels, "{what}: the same pixels as the whole outline");
        assert!(img.pixels.chunks(4).any(|p| p[0] < 128), "{what}: the stroke shows");
        for step in [Point::new(60.0, 40.0), Point::new(450.0, 0.0), Point::new(-900.0, 300.0)] {
            let next = *at + step.to_vec2();
            assert!(render(&mut r, d, next).pixels == whole(d, next).pixels, "{what}, moved by {step:?}: the same pixels");
        }
    }
}

#[test]
fn a_long_dashed_stroke_draws_the_same_pixels_after_its_path_changes() {
    let st = dashed(stroke(5.0, LineJoin::Miter, LineCap::Butt), &[18.0, 7.0], 3.0, false);
    let at = Point::new(12_000.0, 15.0);
    let mut r = Renderer::new();
    render(&mut r, &doc(&zigzag(), st.clone()), at);
    // An anchor in the view moved, then one far before it, which shifts every dash after it.
    for (i, by) in [(1501, Vec2::new(0.0, 12.0)), (40, Vec2::new(-3.0, 0.0))] {
        let mut bp = zigzag();
        let mut els = bp.elements().to_vec();
        let Some(kurbo::PathEl::LineTo(p)) = els.get_mut(i) else {
            panic!("anchor {i} is not a line");
        };
        *p += by;
        bp = BezPath::from_vec(els);
        let d = doc(&bp, st.clone());
        assert!(render(&mut r, &d, at).pixels == whole(&d, at).pixels, "anchor {i} moved: the same pixels");
    }
}

#[test]
fn short_dashed_strokes_are_outlined_whole() {
    let st = dashed(stroke(4.0, LineJoin::Round, LineCap::Round), &[6.0, 4.0], 0.0, false);
    let one = BezPath::from_vec(vec![kurbo::PathEl::MoveTo(Point::new(100.0, 100.0))]);
    let mut dot = one.clone();
    dot.line_to((100.0, 100.0));
    for (what, bp) in
        [("empty", BezPath::new()), ("one anchor", one), ("one anchor, closed on itself", dot), ("a ring of 40 anchors", ring(40, 80.0))]
    {
        let d = doc(&bp, st.clone());
        let mut r = Renderer::new();
        let img = render(&mut r, &d, Point::new(100.0, 100.0));
        assert!(r.strokes.values().all(|e| e.crop.is_none()), "{what}: outlined whole");
        assert!(img.pixels == whole(&d, Point::new(100.0, 100.0)).pixels, "{what}: the same pixels");
    }
}

#[test]
fn a_dashed_stroke_a_wide_stroke_crosses_over_draws_the_same_pixels() {
    // The closed sliver of `a_stroke_a_wide_stroke_crosses_over_stays_whole`, dashed.
    let mut bp = BezPath::new();
    bp.move_to((0.0, 0.0));
    for i in 1..=2000 {
        bp.line_to((f64::from(i) * 10.0, 0.0));
    }
    bp.line_to((20_000.0, 4.0));
    bp.line_to((0.0, 4.0));
    bp.close_path();
    let d = doc(&bp, dashed(stroke(10.0, LineJoin::Miter, LineCap::Butt), &[35.0, 15.0], 0.0, false));
    for at in [Point::new(20_000.0, 2.0), Point::new(9_000.0, 2.0), Point::new(0.0, 2.0)] {
        let mut r = Renderer::new();
        assert!(render(&mut r, &d, at).pixels == whole(&d, at).pixels, "at {at:?}: the same pixels");
    }
}

/// A zigzag of 3,000 anchors 8 pt apart between y = 60 and y = 550, through the bottom of a view
/// on y = 0.
fn tall_zigzag() -> BezPath {
    let mut bp = BezPath::new();
    let at = |i: usize| Point::new(i as f64 * 8.0, if i.is_multiple_of(2) { 60.0 } else { 550.0 });
    bp.move_to(at(0));
    for i in 1..3000 {
        bp.line_to(at(i));
    }
    bp
}

#[test]
fn a_long_stroke_under_a_wide_blur_draws_the_same_pixels_while_dragging_on_threads() {
    // While dragging on threads, a drop shadow this large is drawn through a filter layer whose
    // offscreen reaches 602 px past the view: the parts of the stroke that far away feed the blur.
    let mut d = Document::new(1000.0, 1000.0);
    let shadow = serde_json::json!({"x": 0, "y": 0, "blur": 400, "opacity": 100, "mode": "normal"});
    let look = Appearance { items: vec![AppearanceItem::Stroke(stroke(20.0, LineJoin::Round, LineCap::Butt))], ..Default::default() };
    let n = Node::path(d.alloc_id(), PathData::from_bezpath(&tall_zigzag()), look);
    // The shadow is the group's: the path itself draws as a plain path.
    let mut g = Node::group(d.alloc_id(), vec![n.into()]);
    g.appearance.effects.push(vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: shadow, visible: true });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    let at = Point::new(12_000.0, 0.0);
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), dragging: true, ..Default::default() };
    let draw = |crops: bool| {
        CROP_STROKES.with(|c| c.set(crops));
        let mut r = Renderer::new();
        r.threads = 2;
        let img = r.render(&d, SIZE, SIZE, view_at(at), &opts);
        CROP_STROKES.with(|c| c.set(true));
        (img, r.strokes.values().any(|e| e.crop.is_some()))
    };
    let ((near, cropped), (all, _)) = (draw(true), draw(false));
    assert!(cropped, "outlined near the view");
    assert!(all.pixel(100, 0)[0] < 250, "the shadow shows above the stroke: {:?}", all.pixel(100, 0));
    assert!(near.pixels == all.pixels, "the same pixels as the whole outline");
}

#[test]
fn a_member_below_the_view_casts_its_wide_shadow_while_dragging_on_threads() {
    // A 400 pt shadow of a group whose only member, a rectangle, lies 50 pt below the view.
    let mut d = Document::new(1000.0, 1000.0);
    let shadow = serde_json::json!({"x": 0, "y": 0, "blur": 400, "opacity": 100, "mode": "normal"});
    let look = Appearance { items: vec![AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(Paint::solid(Color::BLACK)))], ..Default::default() };
    let n = Node::path(d.alloc_id(), PathData::from_bezpath(&Rect::new(-1000.0, 150.0, 1000.0, 550.0).to_path(0.1)), look);
    let mut g = Node::group(d.alloc_id(), vec![n.into()]);
    g.appearance.effects.push(vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: shadow, visible: true });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), dragging: true, ..Default::default() };
    let mut r = Renderer::new();
    r.threads = 2;
    let img = r.render(&d, SIZE, SIZE, view_at(Point::ZERO), &opts);
    assert!(img.pixel(100, 199)[0] < 200, "the shadow shows at the bottom of the view: {:?}", img.pixel(100, 199));
}

#[test]
fn a_long_stroke_is_outlined_near_the_view_only() {
    let d = doc(&zigzag(), stroke(6.0, LineJoin::Miter, LineCap::Square));
    let at = Point::new(12_000.0, 15.0);
    let mut r = Renderer::new();
    render(&mut r, &d, at);
    let near = r.strokes.values().map(|e| e.outline.elements().len()).sum::<usize>();
    CROP_STROKES.with(|c| c.set(false));
    let mut full = Renderer::new();
    render(&mut full, &d, at);
    CROP_STROKES.with(|c| c.set(true));
    let all = full.strokes.values().map(|e| e.outline.elements().len()).sum::<usize>();
    assert!(near > 0 && near * 10 < all, "{near} outline elements near the view, {all} for the whole stroke");
}

#[test]
fn a_stroke_a_wide_stroke_crosses_over_stays_whole() {
    // A closed sliver 4 pt tall: a 10 pt stroke covers it from side to side.
    let mut bp = BezPath::new();
    bp.move_to((0.0, 0.0));
    for i in 1..=2000 {
        bp.line_to((f64::from(i) * 10.0, 0.0));
    }
    bp.line_to((20_000.0, 4.0));
    bp.line_to((0.0, 4.0));
    bp.close_path();
    let d = doc(&bp, stroke(10.0, LineJoin::Miter, LineCap::Butt));
    let at = Point::new(20_000.0, 2.0);
    let mut r = Renderer::new();
    let img = render(&mut r, &d, at);
    assert!(r.strokes.values().all(|e| e.crop.is_none()));
    assert!(img.pixels == whole(&d, at).pixels);
}

fn path(svg: &str) -> BezPath {
    BezPath::from_svg(svg).unwrap()
}

#[test]
fn crop_line_keeps_the_runs_of_segments_near_the_region() {
    let region = Rect::new(0.0, 0.0, 100.0, 100.0);
    let crop = |svg: &str| crop_line(&path(svg), region, 5.0, 1.0).map(|bp| bp.to_svg());
    // Nothing to leave out: an empty path, one segment, a path in the region.
    assert_eq!(crop(""), None);
    assert_eq!(crop("M10,10 L90,90"), None);
    assert_eq!(crop("M10,10 L90,90 L10,90 Z"), None);
    // A segment within the reach of the region stays; one beyond it goes, and the open path is cut.
    assert_eq!(crop("M50,50 L104,50 L200,50 L300,50").as_deref(), Some("M50,50 L104,50 L200,50"));
    assert_eq!(crop("M50,50 L200,50 L300,50 L400,50").as_deref(), Some("M50,50 L200,50"));
    // A curve whose control point reaches the region stays.
    assert_eq!(crop("M200,0 C50,50 300,0 400,0 L500,0").as_deref(), Some("M200,0 C50,50 300,0 400,0"));
    // Two runs, each its own open subpath.
    assert_eq!(crop("M50,10 L300,10 L300,90 L50,90").as_deref(), Some("M50,10 L300,10 M300,90 L50,90"));
    // Every segment goes.
    assert_eq!(crop("M300,300 L400,400").as_deref(), Some(""));
    // A closed subpath is walked from a segment it loses: its start stays inside one run.
    assert_eq!(crop("M50,50 L50,500 L500,500 L500,50 Z").as_deref(), Some("M500,50 L50,50 L50,500"));
    // A subpath with no segments stays, and so does an untouched subpath beside a cut one.
    assert_eq!(crop("M900,900 M10,10 L20,20 M50,50 L300,50 L400,50").as_deref(), Some("M900,900 M10,10 L20,20 M50,50 L300,50"));
    // A closed subpath that the stroke crosses over keeps all its segments.
    assert_eq!(crop_line(&path("M50,0 L50,2 L500,2 L500,0 Z"), region, 5.0, 3.0), None);
    // A coordinate that is not a number keeps its segment.
    let mut nan = path("M50,50 L300,50 L400,50");
    nan.line_to((f64::NAN, 50.0));
    assert!(crop_line(&nan, region, 5.0, 1.0).is_some_and(|bp| bp.elements().iter().any(|e| e.end_point().is_some_and(|p| p.x.is_nan()))));
}
