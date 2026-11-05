//! A moved object draws its stroke from the outline made for it before it moved, moved by the same
//! offset, with the pixels a new renderer draws. A change other than a move outlines the stroke
//! again.

use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, ArrowAlign, Arrowhead, Dash, FillLayer, LineCap, LineJoin, Node, NodeId, NodeKind, StrokeLayer, WidthProfile};
use vectorcraft_geom::{PathData, Point, Vec2};

use crate::renderer::{STROKES_OUTLINED, offset_between};
use crate::*;

const SIZE: u32 = 200;

/// The view that shows `at` in the middle of the 200×200 frame.
fn view_at(at: Point) -> Affine {
    Affine::translate((100.0 - at.x, 100.0 - at.y))
}

fn render(r: &mut Renderer, d: &Document, at: Point) -> Rendered {
    r.render(d, SIZE, SIZE, view_at(at), &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

/// How many stroke outlines the renderers on this thread have made.
fn outlined() -> usize {
    STROKES_OUTLINED.with(std::cell::Cell::get)
}

/// A document holding `path` painted with `items`, and the path's id.
fn doc(path: &BezPath, items: Vec<AppearanceItem>) -> (Document, NodeId) {
    let mut d = Document::new(1000.0, 1000.0);
    let id = d.alloc_id();
    let n = Node::path(id, PathData::from_bezpath(path), Appearance { items, ..Default::default() });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    (d, id)
}

fn stroke(width: f64, join: LineJoin, cap: LineCap) -> StrokeLayer {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), width);
    st.join = join;
    st.cap = cap;
    st.miter_limit = 10.0;
    st
}

fn dashed(mut st: StrokeLayer, pattern: &[f64]) -> StrokeLayer {
    st.dash = Some(Dash { pattern: pattern.to_vec(), offset: 0.0, align_corners: false });
    st
}

/// `n` anchors 8 pt apart, up and down by 30 pt, every third segment curved.
fn zigzag(n: usize) -> BezPath {
    let mut bp = BezPath::new();
    let at = |i: usize| Point::new(i as f64 * 8.0, if i.is_multiple_of(2) { 0.0 } else { 30.0 });
    bp.move_to(at(0));
    for i in 1..n {
        if i % 3 == 0 {
            bp.curve_to(at(i - 1) + Vec2::new(6.0, 0.0), at(i) + Vec2::new(-6.0, 10.0), at(i));
        } else {
            bp.line_to(at(i));
        }
    }
    bp
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

/// The documents of the cases: open and closed paths, short and long (outlined near the view), plain,
/// dashed, with a width profile, with arrowheads, and filled with a gradient (which a move changes, so the moved
/// object has an appearance of its own), each with the point the view is centered on.
fn cases() -> Vec<(&'static str, (Document, NodeId), Point)> {
    let round = stroke(6.0, LineJoin::Round, LineCap::Round);
    let mut lens = stroke(10.0, LineJoin::Round, LineCap::Butt);
    lens.profile = Some(WidthProfile { points: vec![(0.0, 0.0, 0.0), (0.5, 1.0, 1.0), (1.0, 0.0, 0.0)] });
    let mut arrows = stroke(4.0, LineJoin::Miter, LineCap::Butt);
    arrows.start_arrow = Some(Arrowhead::Triangle);
    arrows.end_arrow = Some(Arrowhead::Circle);
    arrows.arrow_align = ArrowAlign::Tip;
    let gradient = FillLayer::new(Paint::Gradient(Box::new(GradientPaint::new(Gradient::default()))));
    let only = |st: StrokeLayer| vec![AppearanceItem::Stroke(st)];
    vec![
        ("a short open path", doc(&zigzag(20), only(stroke(5.0, LineJoin::Miter, LineCap::Square))), Point::new(70.0, 15.0)),
        ("a short closed path, dashed", doc(&ring(40, 80.0), only(dashed(round.clone(), &[12.0, 7.0]))), Point::new(0.0, 0.0)),
        ("a long open path", doc(&zigzag(3000), only(stroke(6.0, LineJoin::Miter, LineCap::Square))), Point::new(12_000.0, 15.0)),
        ("a long closed path", doc(&ring(4000, 5000.0), only(round.clone())), Point::new(0.0, 5000.0)),
        (
            "a long open path, dashed",
            doc(&zigzag(3000), only(dashed(stroke(5.0, LineJoin::Bevel, LineCap::Butt), &[30.0, 12.0]))),
            Point::new(8000.0, 15.0),
        ),
        (
            "a long closed path, dashed, filled with a gradient",
            doc(&ring(4000, 5000.0), vec![AppearanceItem::Fill(gradient), AppearanceItem::Stroke(dashed(round, &[25.0, 15.0]))]),
            Point::new(-5000.0, 0.0),
        ),
        ("a long open path, a width profile", doc(&zigzag(3000), only(lens)), Point::new(12_000.0, 15.0)),
        // The heads shorten the line, so the line is not the path itself.
        ("a short open path, arrowheads", doc(&zigzag(20), only(arrows.clone())), Point::new(70.0, 15.0)),
        ("a long open path, arrowheads", doc(&zigzag(3000), only(arrows)), Point::new(40.0, 15.0)),
    ]
}

/// Draws `d` after `change` with `r` and with a new renderer, checks that the two draw the same
/// pixels, and returns how many stroke outlines `r` made.
fn after(r: &mut Renderer, d: &mut Document, id: NodeId, at: Point, change: impl FnOnce(&mut Node)) -> usize {
    change(d.node_mut(id).unwrap());
    let before = outlined();
    let img = render(r, d, at);
    let made = outlined() - before;
    assert!(img.pixels == render(&mut Renderer::new(), d, at).pixels, "the same pixels as a new renderer");
    assert!(img.pixels.chunks(4).any(|p| p[0] < 128), "the stroke shows");
    made
}

#[test]
fn a_moved_path_draws_its_stroke_from_the_outline_made_before() {
    for (what, (mut d, id), at) in cases() {
        let mut r = Renderer::new();
        render(&mut r, &d, at);
        for by in [Vec2::new(1.0, 0.0), Vec2::new(0.25, -0.5), Vec2::new(-3.0, 2.0), Vec2::new(0.1, 0.0)] {
            let made = after(&mut r, &mut d, id, at, |n| n.transform(Affine::translate(by), true));
            assert_eq!(made, 0, "{what}, moved by {by:?}: no new outline");
        }
        // A fill of another color keeps the path where it is.
        let made = after(&mut r, &mut d, id, at, |n| {
            if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
                f.paint = Paint::solid(Color::rgb(0.0, 0.0, 1.0));
            }
        });
        assert_eq!(made, 0, "{what}, a fill changed: no new outline");
    }
}

#[test]
fn a_path_changed_otherwise_outlines_its_stroke_again() {
    let edit_anchor = |n: &mut Node| {
        let NodeKind::Path { path, .. } = &mut n.kind else {
            panic!("not a path");
        };
        let mut bp = path.to_bezpath();
        let mut els = bp.elements().to_vec();
        if let Some(kurbo::PathEl::LineTo(p)) = els.get_mut(2) {
            *p += Vec2::new(0.0, 3.0);
        }
        bp = BezPath::from_vec(els);
        *path = PathData::from_bezpath(&bp);
    };
    for (what, (mut d, id), at) in cases() {
        let mut r = Renderer::new();
        render(&mut r, &d, at);
        // A move, then an edit of an anchor near the path's start.
        assert_eq!(after(&mut r, &mut d, id, at, |n| n.transform(Affine::translate((0.5, 0.0)), true)), 0, "{what}: moved");
        assert_eq!(after(&mut r, &mut d, id, at, edit_anchor), 1, "{what}: an anchor moved");
        // A move after the edit uses the outline made for the edited path.
        assert_eq!(after(&mut r, &mut d, id, at, |n| n.transform(Affine::translate((0.5, 0.0)), true)), 0, "{what}: moved after the edit");
        // Scaled and rotated about the view's center, stroke weights kept.
        assert_eq!(after(&mut r, &mut d, id, at, |n| n.transform(Affine::scale_about(1.001, at), false)), 1, "{what}: scaled");
        assert_eq!(after(&mut r, &mut d, id, at, |n| n.transform(Affine::rotate_about(0.001, at), false)), 1, "{what}: rotated");
        // Another join.
        let rejoin = |n: &mut Node| {
            for i in n.appearance.items.iter_mut() {
                if let AppearanceItem::Stroke(s) = i {
                    s.join = if s.join == LineJoin::Round { LineJoin::Bevel } else { LineJoin::Round };
                }
            }
        };
        assert_eq!(after(&mut r, &mut d, id, at, rejoin), 1, "{what}: another join");
        // Back to the first join and moved again: the outline of the join before is not used.
        assert_eq!(
            after(&mut r, &mut d, id, at, |n| {
                rejoin(n);
                n.transform(Affine::translate((1.0, 0.0)), true);
            }),
            1,
            "{what}: the join changed back and moved"
        );
    }
}

/// Checks that each case, drawn once and then changed by `change` (given the point the view is
/// centered on), is drawn with one new outline and the pixels of a new renderer.
fn outlined_again(change: impl Fn(&mut Node, Point)) {
    for (what, (mut d, id), at) in cases() {
        let mut r = Renderer::new();
        render(&mut r, &d, at);
        assert_eq!(after(&mut r, &mut d, id, at, |n| change(n, at)), 1, "{what}");
    }
}

/// Applies `change` to every stroke of `n`.
fn restroke(n: &mut Node, change: impl Fn(&mut StrokeLayer)) {
    for i in n.appearance.items.iter_mut() {
        if let AppearanceItem::Stroke(s) = i {
            change(s);
        }
    }
}

#[test]
fn a_sheared_path_outlines_its_stroke_again() {
    // Sheared about the view's center, stroke weights kept.
    outlined_again(|n, at| n.transform(Affine::translate(at.to_vec2()) * Affine::skew(0.001, 0.0) * Affine::translate(-at.to_vec2()), false));
}

#[test]
fn another_stroke_width_outlines_the_stroke_again() {
    outlined_again(|n, _| restroke(n, |s| s.width += 1.0));
}

#[test]
fn another_cap_outlines_the_stroke_again() {
    outlined_again(|n, _| restroke(n, |s| s.cap = if s.cap == LineCap::Round { LineCap::Butt } else { LineCap::Round }));
}

#[test]
fn another_dash_pattern_outlines_the_stroke_again() {
    outlined_again(|n, _| {
        restroke(n, |s| match &mut s.dash {
            Some(d) => d.pattern[0] += 4.0,
            None => s.dash = Some(Dash { pattern: vec![9.0, 5.0], offset: 0.0, align_corners: false }),
        })
    });
}

#[test]
fn another_width_profile_outlines_the_stroke_again() {
    outlined_again(|n, _| {
        restroke(n, |s| match &mut s.profile {
            Some(p) => p.points[1].1 = 0.5,
            None => s.profile = Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 0.25, 0.25)] }),
        })
    });
}

#[test]
fn a_small_edit_far_from_the_origin_is_not_taken_for_a_move() {
    let far = Affine::translate((1e9, -1e9));
    let was = far * zigzag(20);
    let by = Vec2::new(0.25, 3.0);
    let moved = Affine::translate(by) * was.clone();
    assert_eq!(offset_between(&was, &moved), Some(moved.elements()[0].end_point().unwrap() - was.elements()[0].end_point().unwrap()));
    let mut els = moved.elements().to_vec();
    if let Some(kurbo::PathEl::LineTo(p)) = els.get_mut(2) {
        p.y += 0.0005;
    }
    assert_eq!(offset_between(&was, &BezPath::from_vec(els)), None, "an anchor moved by 0.0005 pt");
}

#[test]
fn a_path_moved_out_of_the_region_its_outline_was_cropped_to_is_outlined_again() {
    let (mut d, id) = doc(&zigzag(3000), vec![AppearanceItem::Stroke(stroke(6.0, LineJoin::Miter, LineCap::Square))]);
    let at = Point::new(12_000.0, 15.0);
    let mut r = Renderer::new();
    render(&mut r, &d, at);
    assert!(r.strokes.values().any(|e| e.crop.is_some()), "outlined near the view");
    // The outline reaches the view's size past each side of it: 200 pt.
    assert_eq!(after(&mut r, &mut d, id, at, |n| n.transform(Affine::translate((150.0, 0.0)), true)), 0, "moved within the region");
    assert_eq!(after(&mut r, &mut d, id, at, |n| n.transform(Affine::translate((150.0, 0.0)), true)), 1, "moved past it");
}
