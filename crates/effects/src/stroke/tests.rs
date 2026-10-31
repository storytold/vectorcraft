use kurbo::{BezPath, PathEl, Point, Rect, Shape, Vec2};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{ArrowAlign, Arrowhead, Dash, LineCap, StrokeLayer, WidthProfile};

use super::*;

fn line(x0: f64, x1: f64) -> BezPath {
    let mut b = BezPath::new();
    b.move_to((x0, 0.0));
    b.line_to((x1, 0.0));
    b
}

fn square(size: f64) -> BezPath {
    Rect::new(0.0, 0.0, size, size).to_path(0.1)
}

fn stroke(width: f64, f: impl FnOnce(&mut StrokeLayer)) -> StrokeLayer {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), width);
    f(&mut st);
    st
}

fn dashed(pattern: &[f64], offset: f64) -> Dash {
    Dash { pattern: pattern.to_vec(), offset, align_corners: false }
}

fn length(bp: &BezPath) -> f64 {
    bp.segments().map(|s| kurbo::ParamCurveArclen::arclen(&s, 1e-9)).sum()
}

fn moves(bp: &BezPath) -> usize {
    bp.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count()
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// #1396: a stroke wider than the shape still covers the interior and reaches half the weight past it.
/// A thinner stroke keeps the hole in the middle.
#[test]
fn a_stroke_wider_than_the_shape_covers_the_interior() {
    let circle = kurbo::Circle::new(Point::new(0.0, 0.0), 5.0).to_path(0.1);
    let st = stroke(40.0, |_| {});
    let outline = line_outline(&circle, &st, 40.0, 0.25);
    let b = outline.bounding_box();
    assert!(b.x0 < -24.0 && b.x1 > 24.0, "half the weight past the radius: {b:?}");
    assert!(kurbo::Shape::contains(&outline, Point::ZERO), "the middle of a fat stroke is painted");
    let thin = line_outline(&circle, &stroke(4.0, |_| {}), 4.0, 0.25);
    assert!(!kurbo::Shape::contains(&thin, Point::ZERO), "a narrow stroke stays a ring");
    assert!(kurbo::Shape::contains(&thin, Point::new(5.0, 0.0)));
    let square = line_outline(&Rect::new(0.0, 0.0, 8.0, 8.0).to_path(0.1), &st, 40.0, 0.25);
    assert!(kurbo::Shape::contains(&square, Point::new(4.0, 4.0)), "a fat stroke on a small rectangle covers it");
}

// ---------------------------------------------------------------- dashes and dots

#[test]
fn zero_length_dashes_are_dots_along_the_tangent() {
    let d = dash(&line(0.0, 60.0), &dashed(&[0.0, 6.0], 0.0)).unwrap();
    assert!(d.path.elements().is_empty(), "no dash has length");
    let xs: Vec<f64> = d.dots.iter().map(|p| p.at.x).collect();
    assert_eq!(d.dots.len(), 11, "{xs:?}");
    for (i, dot) in d.dots.iter().enumerate() {
        assert!(near(dot.at.x, 6.0 * i as f64, 1e-9) && near(dot.at.y, 0.0, 1e-12));
        assert!(near(dot.dir.x, 1.0, 1e-12));
    }
    // The offset shifts the dots along the path.
    let d = dash(&line(0.0, 60.0), &dashed(&[0.0, 6.0], 3.0)).unwrap();
    assert_eq!(d.dots.len(), 10);
    assert!(near(d.dots[0].at.x, 3.0, 1e-9));
    // Dots on a curve follow its direction.
    let c = kurbo::Circle::new((0.0, 0.0), 50.0).to_path(1e-6);
    let d = dash(&c, &dashed(&[0.0, 10.0], 0.0)).unwrap();
    for dot in &d.dots {
        assert!(dot.dir.dot(dot.at.to_vec2()).abs() < 1e-3, "tangent ⟂ radius at {:?}", dot.at);
    }
}

#[test]
fn dots_on_a_closed_path_are_not_doubled_at_the_start() {
    let d = dash(&square(60.0), &dashed(&[0.0, 6.0], 0.0)).unwrap();
    assert_eq!(d.dots.len(), 40);
}

#[test]
fn dashes_follow_arc_length_and_join_across_the_start_of_closed_paths() {
    let d = dash(&line(10.0, 90.0), &dashed(&[10.0, 10.0], 0.0)).unwrap();
    assert_eq!(moves(&d.path), 4);
    assert!(near(length(&d.path), 40.0, 1e-6));
    // An odd pattern repeats with dashes and gaps swapped: 10 on, 10 off, 10 on…
    let odd = dash(&line(0.0, 80.0), &dashed(&[10.0], 0.0)).unwrap();
    assert_eq!(moves(&odd.path), 4);
    // Perimeter 400, dashes of 20 starting 10 into the pattern: the dash at 390..410 runs
    // through the start point and is one dash.
    let sq = dash(&square(100.0), &dashed(&[20.0, 20.0], 10.0)).unwrap();
    assert_eq!(moves(&sq.path), 10);
    assert!(near(length(&sq.path), 200.0, 1e-6));
    // A pattern without length is a solid line; one dash round a closed path stays closed.
    assert!(dash(&line(0.0, 10.0), &dashed(&[0.0, 0.0], 0.0)).is_none());
    let whole = dash(&square(10.0), &dashed(&[100.0, 1.0], 0.0)).unwrap();
    assert!(is_closed(&whole.path));
    // A pattern far too fine for the path is drawn solid rather than as millions of dashes.
    assert!(dash(&line(0.0, 1e6), &dashed(&[0.001, 0.001], 0.0)).is_none());
}

#[test]
fn dot_outlines_take_the_cap_shape_and_wind_like_stroked_dashes() {
    let dots = [Dot { at: Point::new(10.0, 0.0), dir: Vec2::new(1.0, 0.0), t: 0.0 }];
    let round = dot_outline(&dots, 4.0, LineCap::Round, 1e-4);
    assert!(near(round.area(), std::f64::consts::PI * 4.0, 1e-3));
    let sq = dot_outline(&dots, 4.0, LineCap::Square, 1e-4);
    assert!(near(sq.area(), 16.0, 1e-9));
    assert!(sq.contains(Point::new(11.9, 1.9)) && !round.contains(Point::new(11.9, 1.9)));
    assert!(dot_outline(&dots, 4.0, LineCap::Butt, 1e-4).elements().is_empty());
    // A dot overlapping a dash adds to it under the non-zero rule instead of cancelling it.
    let dash_outline = kurbo::stroke(line(0.0, 10.0).iter(), &kurbo::Stroke::new(4.0), &Default::default(), 1e-4);
    for o in [&round, &sq] {
        assert_eq!(o.area().signum(), dash_outline.area().signum());
    }
    let st = stroke(4.0, |s| {
        s.cap = LineCap::Round;
        s.dash = Some(dashed(&[10.0, 1.0, 0.0, 1.0], 0.0));
    });
    let o = line_outline(&line(0.0, 12.0), &st, 4.0, 1e-3);
    assert!(o.contains(Point::new(10.5, 0.0)), "the dot overlapping the dash end is painted");
}

#[test]
fn expand_stroke_region_of_a_round_line_stays_compact() {
    use std::f64::consts::PI;
    use vectorcraft_geom::{FillRule, PathData, Point, SubPath};
    let path = PathData::single(SubPath::polyline(&[Point::new(0.0, 50.0), Point::new(100.0, 50.0)], false));
    let st = stroke(10.0, |s| s.cap = vectorcraft_doc::LineCap::Round);
    let region = crate::stroke::outline::outline_region(&path, FillRule::NonZero, &st);
    let anchors = region.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>();
    assert!(anchors <= 8, "expand stroke should use few anchors for a round line, got {anchors}");
    let area = region.subpaths.iter().map(|s| s.area()).sum::<f64>().abs();
    assert!(near(area, 100.0 * 10.0 + PI * 5.0 * 5.0, 1.0), "{area}");
    // Extra colinear anchors along the spine must not multiply outline anchors (#3446).
    let dense: Vec<Point> = (0..=20).map(|i| Point::new(i as f64 * 5.0, 50.0)).collect();
    let dense_path = PathData::single(SubPath::polyline(&dense, false));
    let dense_region = crate::stroke::outline::outline_region(&dense_path, FillRule::NonZero, &st);
    let dense_anchors = dense_region.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>();
    assert_eq!(dense_anchors, anchors, "colinear spine points should not add outline anchors");
}

#[test]
fn line_outline_uses_the_profile_dashes_or_a_plain_stroke() {
    let l = line(0.0, 100.0);
    let plain = line_outline(&l, &stroke(10.0, |_| {}), 10.0, 1e-3);
    assert!(near(plain.area().abs(), 1000.0, 1e-6));
    let lens = line_outline(&l, &stroke(10.0, |s| s.profile = Some(WidthProfile::lens())), 10.0, 1e-3);
    assert!(near(lens.area().abs(), 500.0, 1e-6));
    // Each dash takes its widths from where it sits along the lens (0 → 1 → 0): the dash over
    // 0..25 averages a quarter of the width, the one over 50..75 three quarters.
    let st = stroke(10.0, |s| {
        s.profile = Some(WidthProfile::lens());
        s.dash = Some(dashed(&[25.0, 25.0], 0.0));
    });
    assert!(near(line_outline(&l, &st, 10.0, 1e-3).area().abs(), 62.5 + 187.5, 1e-6));
    assert!(line_outline(&BezPath::new(), &st, 10.0, 1e-3).elements().is_empty());
}

#[test]
fn aligned_width_doubles_only_closed_inside_and_outside_strokes() {
    let mut st = stroke(3.0, |s| s.align = vectorcraft_doc::StrokeAlign::Inside);
    assert_eq!(aligned_width(&st, true), 6.0);
    assert_eq!(aligned_width(&st, false), 3.0);
    st.align = vectorcraft_doc::StrokeAlign::Center;
    assert_eq!(aligned_width(&st, true), 3.0);
}

// ---------------------------------------------------------------- arrowheads

fn arrows(kind: Arrowhead, align: ArrowAlign, cap: LineCap) -> StrokeLayer {
    stroke(2.0, |s| {
        s.end_arrow = Some(kind);
        s.arrow_align = align;
        s.cap = cap;
    })
}

#[test]
fn no_heads_borrows_the_path_and_closed_paths_get_none() {
    let l = line(0.0, 50.0);
    assert!(matches!(stroke_pieces(&l, &stroke(2.0, |_| {})).line, Cow::Borrowed(_)));
    let st = stroke(2.0, |s| {
        s.start_arrow = Some(Arrowhead::Triangle);
        s.end_arrow = Some(Arrowhead::Triangle);
        s.arrow_align = ArrowAlign::Tip;
    });
    let sq = square(50.0);
    let p = stroke_pieces(&sq, &st);
    assert!(p.heads.is_empty());
    assert_eq!(p.line.as_ref(), &sq);
}

#[test]
fn tip_mode_shortens_the_stroke_by_the_head_and_puts_the_tip_on_the_end_point() {
    for kind in Arrowhead::ALL {
        let st = arrows(kind, ArrowAlign::Tip, LineCap::Butt);
        let l = line(0.0, 100.0);
        let p = stroke_pieces(&l, &st);
        let [h] = &p.heads[..] else { panic!("one head") };
        assert_eq!(h.tip, Point::new(100.0, 0.0), "{kind:?}");
        assert!(near(h.dir.x, 1.0, 1e-9), "{kind:?}");
        assert!(h.inset > 0.0 && h.inset <= 8.0, "{kind:?} inset {}", h.inset);
        assert!(near(length(&p.line), 100.0 - h.inset, 1e-6), "{kind:?}");
        // Nothing reaches past the tip.
        assert!(h.outline.bounding_box().x1 <= 100.0 + 0.01, "{kind:?}");
        assert!(h.outline.area().abs() > 0.5, "{kind:?}");
    }
    // Both ends, on a curve: the start head points back along the path.
    let mut c = BezPath::new();
    c.move_to((0.0, 0.0));
    c.curve_to((30.0, -40.0), (70.0, -40.0), (100.0, 0.0));
    let st = stroke(2.0, |s| {
        s.start_arrow = Some(Arrowhead::Triangle);
        s.end_arrow = Some(Arrowhead::Arrow);
        s.arrow_align = ArrowAlign::Tip;
    });
    let p = stroke_pieces(&c, &st);
    assert_eq!(p.heads.len(), 2);
    assert_eq!(p.heads[0].tip, Point::new(0.0, 0.0));
    assert!(p.heads[0].dir.x < 0.0 && p.heads[0].dir.y > 0.0, "{:?}", p.heads[0].dir);
    let total = length(&c);
    assert!(near(length(&p.line), total - p.heads[0].inset - p.heads[1].inset, 1e-5));
    // A path shorter than its heads leaves no line.
    let short = line(0.0, 3.0);
    let p = stroke_pieces(&short, &arrows(Arrowhead::Triangle, ArrowAlign::Tip, LineCap::Butt));
    assert!(p.line.elements().is_empty());
    assert_eq!(p.heads.len(), 1);
}

#[test]
fn extend_mode_keeps_the_path_and_overhangs_by_the_inset() {
    let l = line(0.0, 100.0);
    let st = arrows(Arrowhead::Triangle, ArrowAlign::Extend, LineCap::Butt);
    let p = stroke_pieces(&l, &st);
    assert!(matches!(p.line, Cow::Borrowed(_)));
    let h = &p.heads[0];
    assert!(near(h.tip.x, 100.0 + h.inset, 1e-9));
    // The same head as in tip mode, moved by the overhang.
    let tip = stroke_pieces(&l, &arrows(Arrowhead::Triangle, ArrowAlign::Tip, LineCap::Butt));
    let mut moved = tip.heads[0].outline.clone();
    moved.apply_affine(kurbo::Affine::translate((h.inset, 0.0)));
    let pts = |b: &BezPath| b.elements().iter().filter_map(|e| e.end_point()).collect::<Vec<_>>();
    let (a, b) = (pts(&moved), pts(&h.outline));
    assert_eq!(a.len(), b.len());
    assert!(a.iter().zip(&b).all(|(p, q)| p.distance(*q) < 1e-9));
}

#[test]
fn hollow_heads_are_rings_and_the_stroke_stays_out_of_the_hole() {
    for (kind, cap) in [(Arrowhead::CircleOpen, LineCap::Round), (Arrowhead::SquareOpen, LineCap::Square), (Arrowhead::TriangleOpen, LineCap::Butt)] {
        let st = arrows(kind, ArrowAlign::Tip, cap);
        let l = line(0.0, 100.0);
        let p = stroke_pieces(&l, &st);
        let h = &p.heads[0];
        let line_o = line_outline(&p.line, &st, st.width, 1e-3);
        // A point inside the hole is neither head nor line.
        let hole = Point::new(100.0 - 8.0 * 0.45, 0.0);
        assert_eq!(h.outline.winding(hole), 0, "{kind:?} hole");
        assert_eq!(line_o.winding(hole), 0, "{kind:?} line in the hole");
        // The line reaches the back wall, so they overlap (no seam).
        let back_wall = Point::new(100.0 - 8.0 + 0.25, 0.0);
        assert_ne!(h.outline.winding(back_wall), 0, "{kind:?} wall");
        assert_ne!(line_o.winding(back_wall - Vec2::new(0.2, 0.0)), 0, "{kind:?} line reaches the wall");
    }
    // The open arrow is a chevron: open between its arms.
    let l = line(0.0, 100.0);
    let p = stroke_pieces(&l, &arrows(Arrowhead::ArrowOpen, ArrowAlign::Tip, LineCap::Butt));
    let h = &p.heads[0];
    assert_eq!(h.outline.winding(Point::new(93.0, 0.0)), 0);
    assert_ne!(h.outline.winding(Point::new(99.0, 0.0)), 0);
}

#[test]
fn round_caps_never_poke_past_the_tip() {
    for kind in Arrowhead::ALL {
        for scale in [10.0, 100.0, 300.0] {
            let mut st = arrows(kind, ArrowAlign::Tip, LineCap::Round);
            st.arrow_scale = (scale, scale);
            let l = line(0.0, 100.0);
            let p = stroke_pieces(&l, &st);
            let o = line_outline(&p.line, &st, st.width, 1e-3);
            assert!(o.bounding_box().x1 <= 100.0 + 1e-3, "{kind:?} at {scale}%: {}", o.bounding_box().x1);
        }
    }
}

// ---------------------------------------------------------------- robustness

#[test]
fn degenerate_paths_and_patterns_neither_panic_nor_hang() {
    let finite = |b: &BezPath| b.elements().iter().filter_map(|e| e.end_point()).all(|q| q.x.is_finite() && q.y.is_finite());
    let mut point = BezPath::new();
    point.move_to((0.0, 0.0));
    let lone = point.clone();
    point.line_to((0.0, 0.0));
    let mut still_curve = BezPath::new();
    still_curve.move_to((0.0, 0.0));
    still_curve.curve_to((0.0, 0.0), (0.0, 0.0), (0.0, 0.0));
    // Drawing on after a ClosePath, and curves whose handles sit on their end points.
    let mut mixed = BezPath::new();
    mixed.move_to((0.0, 0.0));
    mixed.line_to((10.0, 0.0));
    mixed.close_path();
    mixed.line_to((5.0, 5.0));
    mixed.move_to((20.0, 0.0));
    mixed.curve_to((20.0, 0.0), (50.0, 50.0), (50.0, 50.0));
    mixed.quad_to((50.0, 50.0), (90.0, 0.0));
    let paths = [BezPath::new(), lone, point, still_curve, mixed, square(20.0)];
    // Negative, non-finite and sub-tolerance entries (the last used to loop forever on a
    // zero-length path), and offsets far outside the pattern.
    let patterns: [&[f64]; 6] = [&[0.0, 6.0], &[-1.0, 3.0], &[f64::NAN, 1.0], &[1e-12, 1e-12], &[0.0], &[3.0, 0.0]];
    for bp in &paths {
        for kind in Arrowhead::ALL {
            for align in [ArrowAlign::Extend, ArrowAlign::Tip] {
                for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
                    for pat in patterns {
                        for off in [0.0, -7.0, 1e12, f64::INFINITY] {
                            let st = stroke(2.0, |s| {
                                s.start_arrow = Some(kind);
                                s.end_arrow = Some(kind);
                                s.arrow_align = align;
                                s.cap = cap;
                                s.dash = Some(dashed(pat, off));
                            });
                            let p = stroke_pieces(bp, &st);
                            assert!(p.heads.iter().all(|h| finite(&h.outline)), "{bp:?} {kind:?}");
                            assert!(finite(&line_outline(&p.line, &st, 2.0, 0.01)), "{bp:?} {pat:?} {off}");
                        }
                    }
                }
            }
        }
    }
    assert!(dash(&line(0.0, 1e-9), &dashed(&[1e-12, 1e-12], 0.0)).is_none(), "too fine to advance: solid");
}

/// Grid points within `reach` of the inside of `path` (not past an open end) that `outline`
/// leaves unpainted.
fn unpainted_near(path: &BezPath, outline: &BezPath, reach: f64) -> Vec<Point> {
    let b = path.bounding_box().inflate(reach, reach);
    let ends: Vec<Point> = if is_closed(path) { vec![] } else { path.segments().flat_map(|s| [s.start(), s.end()]).collect() };
    let ends = [ends.first().copied(), ends.last().copied()];
    let mut missed = Vec::new();
    for i in 0..=60 {
        for j in 0..=60 {
            let p = Point::new(b.x0 + b.width() * f64::from(i) / 60.0, b.y0 + b.height() * f64::from(j) / 60.0);
            let nearest = path
                .segments()
                .map(|s| (s, kurbo::ParamCurveNearest::nearest(&s, p, 1e-6)))
                .min_by(|a, b| a.1.distance_sq.total_cmp(&b.1.distance_sq))
                .map(|(s, n)| (s.eval(n.t), n.distance_sq.sqrt()));
            let Some((at, dist)) = nearest else { continue };
            let past_end = ends.iter().flatten().any(|e| e.distance(at) < 1e-6);
            if dist <= reach && !past_end && outline.winding(p) == 0 {
                missed.push(p);
            }
        }
    }
    missed
}

/// #1396: whatever the weight is next to the size of the shape, every point within half the
/// weight of the path is painted (the stroke is the path swept by a pen that wide).
#[test]
fn a_fat_stroke_paints_everything_within_half_its_weight() {
    let mut shapes: Vec<(String, BezPath)> = Vec::new();
    for r in [3.0, 6.0, 10.0, 15.0, 20.0, 24.0, 40.0] {
        let circle = kurbo::Circle::new(Point::ZERO, r).to_path(0.01);
        let mut ellipse = kurbo::Ellipse::new(Point::ZERO, (r, r / 2.0), 0.3).to_path(0.01);
        ellipse.close_path();
        let arc = BezPath::from_vec(circle.elements().iter().take(3).copied().collect());
        let mut l = BezPath::new();
        l.move_to((0.0, 0.0));
        for p in [(r * 2.0, 0.0), (r * 2.0, r), (r, r), (r, r * 2.0), (0.0, r * 2.0)] {
            l.line_to(p);
        }
        l.close_path();
        let mut two = circle.clone();
        two.extend(kurbo::Circle::new(Point::new(200.0, 0.0), 60.0).to_path(0.01).iter());
        shapes.extend([
            (format!("circle r={r}"), circle),
            (format!("ellipse {r}"), ellipse),
            (format!("rect {r}"), Rect::new(0.0, 0.0, r, r * 1.5).to_path(0.1)),
            (format!("open arc r={r}"), arc),
            (format!("L {r}"), l),
            (format!("circle r={r} beside a large one"), two),
        ]);
    }
    let mut failures = Vec::new();
    for (name, path) in &shapes {
        for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
            let outline = line_outline(path, &stroke(50.0, |s| s.join = join), 50.0, 0.05);
            // A bevel stops short of the pen's reach at a corner.
            let reach = if join == LineJoin::Bevel { 16.0 } else { 24.5 };
            let missed = unpainted_near(path, &outline, reach);
            if !missed.is_empty() {
                failures.push(format!("{name} {join:?}: {} unpainted, e.g. {:?} ({} contours)", missed.len(), missed.first(), moves(&outline)));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A stroke the shape has room for keeps its hole, also beside a subpath that has none.
#[test]
fn a_stroke_narrower_than_the_shape_keeps_its_hole() {
    let mut path = kurbo::Circle::new(Point::ZERO, 30.0).to_path(0.01);
    path.extend(kurbo::Circle::new(Point::new(200.0, 0.0), 10.0).to_path(0.01).iter());
    for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
        let outline = line_outline(&path, &stroke(50.0, |s| s.join = join), 50.0, 0.05);
        assert_eq!(outline.winding(Point::ZERO), 0, "{join:?}: the hole of the wide ring");
        assert_eq!(outline.winding(Point::new(4.0, 0.0)), 0, "{join:?}: up to its inner edge");
        assert_ne!(outline.winding(Point::new(6.0, 0.0)), 0, "{join:?}");
        assert_ne!(outline.winding(Point::new(200.0, 0.0)), 0, "{join:?}: the small circle is solid");
        assert_eq!(moves(&outline), 3, "{join:?}: two contours for the ring, one for the blob");
    }
}
