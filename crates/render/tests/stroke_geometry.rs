//! Pixel tests for the shared stroke geometry (`vectorcraft_effects::stroke`) on the canvas:
//! dotted lines (zero-length dashes) and arrowheads (hollow kinds, alignment, compositing).
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, ArrowAlign, Arrowhead, Dash, Document, LineCap, StrokeLayer};
use vectorcraft_geom::{Point, Rect, Shape, shapes};
use vectorcraft_render::{Renderer, effects};
use vectorcraft_testkit::fixtures::DocBuilder;
use vectorcraft_testkit::raster::{Image, assert_similar, render_region};
use vectorcraft_testkit::svg;

/// Pixels per point in these tests.
const S: f64 = 4.0;

fn stroked(width: f64, f: impl FnOnce(&mut StrokeLayer)) -> Appearance {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), width);
    f(&mut st);
    Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() }
}

/// A horizontal line from x0 to x1 at y = 50 on a 100 × 100 artboard.
fn line_doc(x0: f64, x1: f64, app: Appearance, opacity: f32) -> Document {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::line(Point::new(x0, 50.0), Point::new(x1, 50.0)), app, |n| n.opacity = opacity);
    b.build()
}

fn render(d: &Document) -> Image {
    render_region(d, Rect::new(0.0, 0.0, 100.0, 100.0), S)
}

/// Is the pixel at document point (x, y) dark?
fn dark(img: &Image, x: f64, y: f64) -> bool {
    img.over_white((x * S) as u32, (y * S) as u32)[0] < 128
}

fn dotted(cap: LineCap) -> Document {
    line_doc(
        20.0,
        80.0,
        stroked(4.0, |s| {
            s.cap = cap;
            s.dash = Some(Dash { pattern: vec![0.0, 6.0], offset: 0.0, align_corners: false });
        }),
        1.0,
    )
}

#[test]
fn zero_length_dashes_draw_dots_with_a_round_cap_and_squares_with_a_projecting_cap() {
    let round = render(&dotted(LineCap::Round));
    // 60 pt with a dot every 6 pt: 11 separate dots along the centre line.
    let row: Vec<bool> = (0..round.width).map(|x| round.over_white(x, (50.0 * S) as u32)[0] < 128).collect();
    let runs = row.windows(2).filter(|w| !w[0] && w[1]).count();
    assert_eq!(runs, 11, "dots along the line");
    for k in 0..=10 {
        let x = 20.0 + 6.0 * k as f64;
        assert!(dark(&round, x, 50.0), "dot {k}");
        assert!(!dark(&round, x + 3.0, 50.0), "gap after dot {k}");
    }
    // A disc's corner region is empty; a square fills it.
    assert!(!dark(&round, 20.0 + 1.8, 50.0 + 1.8));
    let square = render(&dotted(LineCap::Square));
    assert!(dark(&square, 20.0 + 1.8, 50.0 + 1.8));
    assert!(dark(&square, 26.0 - 1.8, 50.0 - 1.8));
    // A butt cap gives a zero-length dash no area at all.
    assert_eq!(render(&dotted(LineCap::Butt)).ink(), 0);
}

#[test]
fn dotted_lines_survive_svg_export_and_render_the_same() {
    for cap in [LineCap::Round, LineCap::Square] {
        let d = dotted(cap);
        let text = svg::export(&d, &Default::default());
        let back = svg::import(&text).unwrap();
        assert_similar(&render(&d), &render(&back), 0.05, 0.001);
    }
}

fn arrow_doc(kind: Arrowhead, align: ArrowAlign, opacity: f32, f: impl FnOnce(&mut StrokeLayer)) -> Document {
    line_doc(
        20.0,
        80.0,
        stroked(4.0, |s| {
            s.end_arrow = Some(kind);
            s.arrow_align = align;
            f(s);
        }),
        opacity,
    )
}

/// The arrowhead the shared geometry computes for the line of `d`.
fn head_of(d: &Document) -> effects::stroke::Arrow {
    let n = d.layers[0].children().unwrap()[0].clone();
    let bp = n.path_data().unwrap().to_bezpath();
    effects::stroke::stroke_pieces(&bp, n.appearance.stroke().unwrap()).heads.remove(0)
}

#[test]
fn a_hollow_circle_head_is_empty_inside() {
    let d = arrow_doc(Arrowhead::CircleOpen, ArrowAlign::Extend, 1.0, |_| {});
    let h = head_of(&d);
    let centre = h.tip - h.dir * 8.0;
    let img = render(&d);
    assert!(!dark(&img, centre.x, centre.y), "the centre of an open circle is empty");
    assert!(dark(&img, h.tip.x - 1.0, 50.0), "its wall is painted");
    // The filled circle is solid.
    let solid = render(&arrow_doc(Arrowhead::Circle, ArrowAlign::Extend, 1.0, |_| {}));
    assert!(dark(&solid, centre.x, centre.y));
}

#[test]
fn heads_are_drawn_where_the_shared_geometry_puts_them_and_never_past_the_tip() {
    for align in [ArrowAlign::Extend, ArrowAlign::Tip] {
        for kind in Arrowhead::ALL {
            let d = arrow_doc(kind, align, 1.0, |s| s.cap = LineCap::Round);
            let h = head_of(&d);
            if align == ArrowAlign::Tip {
                assert_eq!(h.tip, Point::new(80.0, 50.0));
            } else {
                assert!((h.tip.x - (80.0 + h.inset)).abs() < 1e-9);
            }
            let img = render(&d);
            // The rightmost ink is the tip (allowing for antialiasing).
            let right = (0..img.width).rev().find(|&x| (0..img.height).any(|y| img.over_white(x, y)[0] < 250)).unwrap();
            let bb = h.outline.bounding_box();
            assert!((right as f64 / S - bb.x1).abs() <= 0.5, "{kind:?} {align:?}: ink ends at {} vs head {}", right as f64 / S, bb.x1);
            assert!(!dark(&img, h.tip.x + 0.5, 50.0), "{kind:?} {align:?}: nothing past the tip");
        }
    }
}

/// Straight-alpha pixel at document point (x, y), rendered on transparent.
fn alpha_at(d: &Document, x: f64, y: f64) -> u8 {
    let r = Renderer::new().render_region(d, Rect::new(0.0, 0.0, 100.0, 100.0), S, false);
    r.pixel((x * S) as u32, (y * S) as u32)[3]
}

#[test]
fn line_and_head_take_the_opacity_once() {
    for (node, stroke) in [(0.5, 1.0), (1.0, 0.5)] {
        let d = arrow_doc(Arrowhead::Triangle, ArrowAlign::Extend, node, |s| s.opacity = stroke);
        let h = head_of(&d);
        let line = alpha_at(&d, 40.0, 50.0);
        assert!((120..=135).contains(&line), "half-opaque line: {line}");
        // Where the line runs under the head, and in the head beside it.
        let overlap = alpha_at(&d, h.tip.x - h.inset - 1.0, 50.0);
        let beside = alpha_at(&d, h.tip.x - 12.0, 50.0 + 4.5);
        assert_eq!(overlap, line, "no double darkening where line and head overlap");
        assert_eq!(beside, line, "the head has the line's opacity");
    }
}

#[test]
fn a_gradient_runs_on_into_the_head() {
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let paint = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let d = arrow_doc(Arrowhead::Square, ArrowAlign::Tip, 1.0, |s| s.paint = paint);
    let h = head_of(&d);
    let img = render(&d);
    // Just behind the head's back edge (line) and just inside it (head): nearly the same colour.
    let back = h.tip.x - 16.0;
    let (a, b) = (img.over_white(((back - 0.5) * S) as u32, (50.0 * S) as u32), img.over_white(((back + 0.5) * S) as u32, (50.0 * S) as u32));
    assert!(a[0].abs_diff(b[0]) < 12 && a[2].abs_diff(b[2]) < 12, "{a:?} vs {b:?}");
    // The head is part of the same gradient: bluer than the line's start.
    let start = img.over_white((22.0 * S) as u32, (50.0 * S) as u32);
    assert!(b[2] > start[2] + 100, "{b:?} vs {start:?}");
}

#[test]
fn a_head_past_the_end_counts_in_the_visual_bounds_and_is_never_culled() {
    let d = arrow_doc(Arrowhead::Triangle, ArrowAlign::Extend, 1.0, |s| s.join = vectorcraft_doc::LineJoin::Round);
    let h = head_of(&d);
    let vb = d.layers[0].children().unwrap()[0].visual_bounds().unwrap();
    let head = h.outline.bounding_box();
    assert_eq!(vb.union(head), vb, "{vb:?} covers the head {head:?}");
    // A view that shows only the overhanging part of the head (the line ends at 80).
    let img = render_region(&d, Rect::new(83.0, 40.0, 100.0, 60.0), S);
    assert!(img.ink() > 0, "the head is drawn");
}
