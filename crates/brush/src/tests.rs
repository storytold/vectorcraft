use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{Affine, BezPath, Point, Rect, shapes};

use super::*;

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    let mut bp = BezPath::new();
    bp.move_to((x0, y0));
    bp.line_to((x1, y1));
    bp
}

fn stroke(w: f64) -> StrokeLayer {
    StrokeLayer::new(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), w)
}

fn calli(angle: f64, roundness: f64, size: f64) -> Brush {
    Brush { name: "c".into(), kind: BrushKind::Calligraphic(Calligraphic { angle, roundness, size, ..Default::default() }) }
}

fn bounds(nodes: &[Node]) -> Rect {
    nodes.iter().fold(None, |a, n| vectorcraft_geom::union_opt(a, n.geometric_bounds())).unwrap()
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn all_points(n: &Node, out: &mut Vec<Point>) {
    n.walk(&mut |c| {
        if let Some(p) = c.path_data() {
            out.extend(p.anchors().map(|(_, _, a)| a.p));
        }
    });
}

#[test]
fn calligraphic_vertical_nib_is_full_width_across_horizontal_stroke() {
    let out = stroke_pieces(&Document::new(1.0, 1.0), &calli(90.0, 20.0, 10.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    assert_eq!(out.len(), 1);
    let b = bounds(&out);
    assert!(near(b.height(), 10.0, 0.05), "{b:?}");
    assert!(near(b.x0, -1.0, 0.05) && near(b.x1, 101.0, 0.05), "caps add half the minor axis: {b:?}");
}

#[test]
fn calligraphic_flat_nib_is_thin_along_its_angle() {
    let out = stroke_pieces(&Document::new(1.0, 1.0), &calli(0.0, 20.0, 10.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    assert!(near(bounds(&out).height(), 2.0, 0.05), "{:?}", bounds(&out));
}

#[test]
fn calligraphic_width_at_45_degrees_matches_ellipse_support() {
    let (a, b) = (5.0_f64, 1.0_f64);
    let out = stroke_pieces(&Document::new(1.0, 1.0), &calli(45.0, 20.0, 10.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    let expect = 2.0 * ((a * a + b * b) / 2.0).sqrt();
    assert!(near(bounds(&out).height(), expect, 0.05), "{} vs {expect}", bounds(&out).height());
    // A vertical stroke with the same nib has the same width (45° is symmetric).
    let v = stroke_pieces(&Document::new(1.0, 1.0), &calli(45.0, 20.0, 10.0), &line(0.0, 0.0, 0.0, 100.0), &stroke(1.0));
    assert!(near(bounds(&v).width(), expect, 0.05));
}

#[test]
fn calligraphic_scales_with_stroke_weight() {
    let out = stroke_pieces(&Document::new(1.0, 1.0), &calli(90.0, 100.0, 3.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(2.0));
    assert!(near(bounds(&out).height(), 6.0, 0.05));
}

#[test]
fn round_nib_is_direction_independent() {
    let out = stroke_pieces(&Document::new(1.0, 1.0), &calli(0.0, 100.0, 4.0), &line(0.0, 0.0, 60.0, 80.0), &stroke(1.0));
    let b = bounds(&out);
    // Round caps: the bounds grow by the radius on every side.
    assert!(near(b.x0, -2.0, 0.05) && near(b.y1, 82.0, 0.05), "{b:?}");
}

#[test]
fn closed_calligraphic_makes_a_ring() {
    let bp = shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let out = stroke_pieces(&Document::new(1.0, 1.0), &calli(0.0, 100.0, 6.0), &bp, &stroke(1.0));
    let pd = out[0].path_data().unwrap();
    assert_eq!(pd.subpaths.len(), 2);
    let poly_area = |sp: &vectorcraft_geom::SubPath| {
        let a = &sp.anchors;
        (0..a.len()).map(|i| a[i].p.x * a[(i + 1) % a.len()].p.y - a[(i + 1) % a.len()].p.x * a[i].p.y).sum::<f64>() / 2.0
    };
    let (outer, inner) = (poly_area(&pd.subpaths[0]), poly_area(&pd.subpaths[1]));
    assert!(outer * inner < 0.0, "opposite windings make a hole");
    let area = (outer.abs() - inner.abs()).abs();
    let expect = std::f64::consts::PI * (53.0f64.powi(2) - 47.0f64.powi(2));
    assert!((area - expect).abs() / expect < 0.03, "{area} vs {expect}");
}

fn art_brush(art: Node, scale: ArtScale) -> Brush {
    Brush { name: "a".into(), kind: BrushKind::Art(ArtBrush { art, scale, colorization: Colorization::None, ..Default::default() }) }
}

fn rect_art(w: f64, h: f64) -> Node {
    Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, -h / 2.0, w, h / 2.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0))
}

#[test]
fn art_brush_stretches_to_path_length() {
    let out =
        stroke_pieces(&Document::new(1.0, 1.0), &art_brush(rect_art(10.0, 4.0), ArtScale::Stretch), &line(20.0, 50.0, 220.0, 50.0), &stroke(1.0));
    let b = bounds(&out);
    assert!(near(b.x0, 20.0, 1e-6) && near(b.x1, 220.0, 1e-6), "{b:?}");
    assert!(near(b.height(), 4.0, 1e-6));
    // 3 pt stroke → 3× wider.
    let out =
        stroke_pieces(&Document::new(1.0, 1.0), &art_brush(rect_art(10.0, 4.0), ArtScale::Stretch), &line(20.0, 50.0, 220.0, 50.0), &stroke(3.0));
    assert!(near(bounds(&out).height(), 12.0, 1e-6));
}

#[test]
fn art_brush_arc_length_mapping_is_linear() {
    // A marker square at art x ∈ [5, 6] of a 10 wide art lands at 50..60 % of the path.
    let marker =
        Node::path(NodeId(0), shapes::rectangle(Rect::new(5.0, -1.0, 6.0, 1.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let frame = rect_art(10.0, 2.0);
    let mut spacer = frame.clone();
    spacer.appearance = Appearance::default().into();
    let art = Node::group(NodeId(0), vec![std::sync::Arc::new(spacer), std::sync::Arc::new(marker)]);
    let out = stroke_pieces(&Document::new(1.0, 1.0), &art_brush(art, ArtScale::Stretch), &line(0.0, 0.0, 300.0, 0.0), &stroke(1.0));
    let g = &out[0];
    let m = g.children().unwrap()[1].geometric_bounds().unwrap();
    assert!(near(m.x0, 150.0, 1e-6) && near(m.x1, 180.0, 1e-6), "{m:?}");
}

#[test]
fn art_brush_proportional_keeps_aspect() {
    let out =
        stroke_pieces(&Document::new(1.0, 1.0), &art_brush(rect_art(100.0, 4.0), ArtScale::Proportional), &line(0.0, 0.0, 200.0, 0.0), &stroke(1.0));
    assert!(near(bounds(&out).height(), 8.0, 1e-6));
}

#[test]
fn art_brush_between_guides_keeps_ends() {
    // Head (last 20 % of a 100 pt art) keeps its 20 pt length on a 300 pt path.
    let head =
        Node::path(NodeId(0), shapes::rectangle(Rect::new(80.0, -4.0, 100.0, 4.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let shaft = rect_art(80.0, 2.0);
    let art = Node::group(NodeId(0), vec![std::sync::Arc::new(shaft), std::sync::Arc::new(head)]);
    let out = stroke_pieces(
        &Document::new(1.0, 1.0),
        &art_brush(art, ArtScale::BetweenGuides { start: 0.0, end: 0.8 }),
        &line(0.0, 0.0, 300.0, 0.0),
        &stroke(1.0),
    );
    let h = out[0].children().unwrap()[1].geometric_bounds().unwrap();
    assert!(near(h.x0, 280.0, 1e-6) && near(h.x1, 300.0, 1e-6), "{h:?}");
}

#[test]
fn art_brush_overlap_splits_at_corners() {
    let mut bp = BezPath::new();
    bp.move_to((0.0, 0.0));
    bp.line_to((100.0, 0.0));
    bp.line_to((100.0, 100.0));
    let brush = |overlap: bool| Brush {
        name: "a".into(),
        kind: BrushKind::Art(ArtBrush {
            art: rect_art(10.0, 4.0),
            scale: ArtScale::Stretch,
            overlap,
            colorization: Colorization::None,
            ..Default::default()
        }),
    };
    let one = stroke_pieces(&Document::new(1.0, 1.0), &brush(false), &bp, &stroke(1.0));
    let two = stroke_pieces(&Document::new(1.0, 1.0), &brush(true), &bp, &stroke(1.0));
    assert_eq!(one.len(), 1, "one bent copy along the whole path");
    assert_eq!(two.len(), 2, "one copy per run between corners");
}

#[test]
fn art_brush_follows_curves() {
    let bp = shapes::ellipse(Rect::new(0.0, 0.0, 200.0, 200.0)).to_bezpath();
    let mut bp_open = BezPath::new();
    // Upper half of the circle, as an open path.
    for (i, el) in bp.elements().iter().enumerate() {
        if i < 3 {
            bp_open.push(*el);
        }
    }
    let out = stroke_pieces(&Document::new(1.0, 1.0), &art_brush(rect_art(10.0, 4.0), ArtScale::Stretch), &bp_open, &stroke(1.0));
    let mut pts = vec![];
    all_points(&out[0], &mut pts);
    assert!(pts.len() > 40, "long edges are subdivided so they bend: {}", pts.len());
    for p in pts {
        let r = p.distance(Point::new(100.0, 100.0));
        assert!((r - 100.0).abs() <= 2.0 + 0.3, "point {p:?} is {r} from the centre");
    }
}

fn scatter_brush(spacing: f64) -> Brush {
    Brush {
        name: "s".into(),
        kind: BrushKind::Scatter(Scatter {
            art: rect_art(10.0, 4.0),
            spacing: (spacing, spacing),
            colorization: Colorization::None,
            ..Default::default()
        }),
    }
}

#[test]
fn scatter_count_and_spacing() {
    let out = stroke_pieces(&Document::new(1.0, 1.0), &scatter_brush(100.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    assert_eq!(out.len(), 10);
    let centres: Vec<f64> = out.iter().map(|n| n.geometric_bounds().unwrap().center().x).collect();
    for (i, c) in centres.iter().enumerate() {
        assert!(near(*c, 5.0 + 10.0 * i as f64, 1e-6), "{centres:?}");
    }
    let out = stroke_pieces(&Document::new(1.0, 1.0), &scatter_brush(200.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    assert_eq!(out.len(), 5);
}

#[test]
fn scatter_is_deterministic_and_randomised() {
    let b = Brush {
        name: "r".into(),
        kind: BrushKind::Scatter(Scatter {
            art: rect_art(4.0, 4.0),
            size: (50.0, 150.0),
            scatter: (-100.0, 100.0),
            rotation: (-90.0, 90.0),
            ..Default::default()
        }),
    };
    let p = line(0.0, 0.0, 300.0, 0.0);
    let a1 = stroke_pieces(&Document::new(1.0, 1.0), &b, &p, &stroke(1.0));
    let a2 = stroke_pieces(&Document::new(1.0, 1.0), &b, &p, &stroke(1.0));
    assert_eq!(a1, a2);
    let ys: Vec<f64> = a1.iter().map(|n| n.geometric_bounds().unwrap().center().y).collect();
    assert!(ys.iter().any(|y| y.abs() > 0.5), "scatter offsets copies: {ys:?}");
}

fn pattern_brush(fit: PatternFit, corners: bool) -> Brush {
    let side = rect_art(10.0, 4.0);
    let corner =
        Node::path(NodeId(0), shapes::ellipse(Rect::new(0.0, -3.0, 6.0, 3.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    Brush {
        name: "p".into(),
        kind: BrushKind::Pattern(PatternBrush {
            side,
            outer_corner: corners.then(|| corner.clone()),
            inner_corner: corners.then_some(corner),
            fit,
            colorization: Colorization::None,
            ..Default::default()
        }),
    }
}

#[test]
fn pattern_tile_count_by_fit() {
    let p = line(0.0, 0.0, 100.0, 0.0);
    assert_eq!(stroke_pieces(&Document::new(1.0, 1.0), &pattern_brush(PatternFit::Stretch, false), &p, &stroke(1.0)).len(), 10);
    let p2 = line(0.0, 0.0, 104.0, 0.0);
    let st = stroke_pieces(&Document::new(1.0, 1.0), &pattern_brush(PatternFit::Stretch, false), &p2, &stroke(1.0));
    assert_eq!(st.len(), 10);
    assert!(near(bounds(&st).width(), 104.0, 1e-6), "stretch fills the path exactly");
    let sp = stroke_pieces(&Document::new(1.0, 1.0), &pattern_brush(PatternFit::AddSpace, false), &line(0.0, 0.0, 109.0, 0.0), &stroke(1.0));
    assert_eq!(sp.len(), 10);
    assert!(near(sp[0].geometric_bounds().unwrap().width(), 10.0, 1e-6), "add space keeps tile size");
    assert_eq!(stroke_pieces(&Document::new(1.0, 1.0), &pattern_brush(PatternFit::Stretch, false), &p, &stroke(2.0)).len(), 5);
}

#[test]
fn pattern_places_corner_tiles() {
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let out = stroke_pieces(&Document::new(1.0, 1.0), &pattern_brush(PatternFit::Stretch, true), &sq, &stroke(1.0));
    // The corner tile is an ellipse and the side tile a rectangle, so a kept handle tells them
    // apart. Anchor count no longer does: the side tiles keep their own four anchors now that a
    // straight run is not flattened (#3926), where they used to arrive as many-anchor polylines.
    let curved = |n: &Node| {
        matches!(&n.kind, NodeKind::Path { path, .. }
            if path.subpaths.iter().any(|s| s.anchors.iter().any(|a| a.h_in != a.p || a.h_out != a.p)))
    };
    let corners = out.iter().filter(|n| curved(n)).count();
    assert_eq!(corners, 4, "the four corner ellipses, still curves");
    // Sides lose half a corner tile at each end: 94 pt per side → 9 tiles each.
    assert_eq!(out.len() - corners, 36);
    for n in out.iter().filter(|n| !curved(n)) {
        let NodeKind::Path { path, .. } = &n.kind else { continue };
        assert_eq!(path.subpaths[0].anchors.len(), 4, "a rectangle tile on a straight run stays four anchors");
    }
}

/// The 10 × 4 pt side tile of [`pattern_brush`] with the given corner sources and no corner art.
fn auto_corner_brush(outer: Option<CornerSource>, inner: Option<CornerSource>) -> Brush {
    let mut b = pattern_brush(PatternFit::Stretch, false);
    if let BrushKind::Pattern(p) = &mut b.kind {
        p.outer_source = outer;
        p.inner_source = inner;
    }
    b
}

const AUTO: [CornerSource; 4] = [CornerSource::AutoCentered, CornerSource::AutoBetween, CornerSource::AutoSliced, CornerSource::AutoOverlap];

/// Whether `n` paints a fill over `q`: inside one of its paths, and inside the mask of every
/// clipping group it is in.
fn covers(n: &Node, q: Point) -> bool {
    let inside = |n: &Node| n.path_data().is_some_and(|p| kurbo::Shape::winding(&p.to_bezpath(), q) != 0);
    match &n.kind {
        NodeKind::Group { children, clip: true } => children.first().is_some_and(|m| inside(m)) && children.iter().skip(1).any(|c| covers(c, q)),
        NodeKind::Path { .. } => inside(n),
        _ => n.children().is_some_and(|c| c.iter().any(|c| covers(c, q))),
    }
}

fn points_of(nodes: &[Node]) -> Vec<Point> {
    let mut pts = vec![];
    for n in nodes {
        all_points(n, &mut pts);
    }
    pts
}

/// The corners of a 100 pt square and, for each, the outer corner of the 4 pt band around it.
const SQUARE_CORNERS: [((f64, f64), (f64, f64)); 4] =
    [((0.0, 0.0), (-2.0, -2.0)), ((100.0, 0.0), (102.0, -2.0)), ((100.0, 100.0), (102.0, 102.0)), ((0.0, 100.0), (-2.0, 102.0))];

/// Whether some point of `pts` is within `tol` of `(x, y)`.
fn reaches(pts: &[Point], (x, y): (f64, f64), tol: f64) -> bool {
    pts.iter().any(|p| p.distance(Point::new(x, y)) <= tol)
}

/// Distance from `p` to the outline of the square (0, 0)–(100, 100).
fn from_square_outline(p: Point) -> f64 {
    let dx = (p.x - p.x.clamp(0.0, 100.0)).abs();
    let dy = (p.y - p.y.clamp(0.0, 100.0)).abs();
    if dx > 0.0 || dy > 0.0 { dx.hypot(dy) } else { p.x.min(100.0 - p.x).min(p.y).min(100.0 - p.y) }
}

#[test]
fn pattern_auto_corners_fill_each_corner_of_a_square() {
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let mut outputs = vec![];
    for kind in AUTO {
        let out = stroke_pieces(&Document::new(1.0, 1.0), &auto_corner_brush(Some(kind), Some(kind)), &sq, &stroke(1.0));
        let pts = points_of(&out);
        for (corner, outside) in SQUARE_CORNERS {
            assert!(reaches(&pts, outside, 0.8), "{kind:?}: the art reaches the band's outer corner at {corner:?}");
        }
        let far = pts.iter().copied().map(from_square_outline).fold(0.0, f64::max);
        assert!(far <= 2.0 * 2f64.sqrt() + 0.05, "{kind:?}: every point stays in the 4 pt band, {far}");
        // The band is filled up to each corner along both of its sides.
        let filled = |x: f64, y: f64| out.iter().any(|n| covers(n, Point::new(x, y)));
        for ((cx, cy), _) in SQUARE_CORNERS {
            let (sx, sy) = ((50.0 - cx).signum(), (50.0 - cy).signum());
            for d in [0.5, 2.5, 4.5, 6.5, 8.5] {
                assert!(filled(cx + sx * d, cy) && filled(cx, cy + sy * d), "{kind:?}: filled {d} pt from ({cx}, {cy})");
            }
        }
        outputs.push(out);
    }
    for i in 0..outputs.len() {
        for j in i + 1..outputs.len() {
            assert_ne!(outputs[i], outputs[j], "{:?} and {:?} draw differently", AUTO[i], AUTO[j]);
        }
    }
}

/// A side tile three times as tall as it is long still fills a square's corners without a gap:
/// the two sides meet on the corner's diagonal.
#[test]
fn pattern_auto_corners_of_a_tall_tile_meet_on_the_diagonal() {
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)).to_bezpath();
    for kind in AUTO {
        let mut b = auto_corner_brush(Some(kind), Some(kind));
        if let BrushKind::Pattern(p) = &mut b.kind {
            p.side = rect_art(10.0, 30.0);
        }
        let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &sq, &stroke(1.0));
        let filled = |x: f64, y: f64| out.iter().any(|n| covers(n, Point::new(x, y)));
        for (cx, cy) in [(0.0_f64, 0.0_f64), (200.0, 0.0), (200.0, 200.0), (0.0, 200.0)] {
            let (sx, sy) = ((100.0 - cx).signum(), (100.0 - cy).signum());
            for d in [1.0, 5.0, 10.0, 14.0] {
                // Along the diagonal, outside and inside the square, and along both sides.
                assert!(filled(cx - sx * d, cy - sy * d), "{kind:?}: outside ({cx}, {cy}) by {d}");
                assert!(filled(cx + sx * d, cy + sy * d), "{kind:?}: inside ({cx}, {cy}) by {d}");
                assert!(filled(cx + sx * 3.0 * d, cy) && filled(cx, cy + sy * 3.0 * d), "{kind:?}: {} pt from ({cx}, {cy})", 3.0 * d);
            }
        }
    }
}

/// Where a curve meets a sharp corner, the corner tile reaches the side tiles on the curve: the
/// middle of the band is filled all the way along the path.
#[test]
fn pattern_auto_corners_join_a_curve_into_the_corner() {
    // A straight run into a quarter circle of radius 10, then a sharp turn.
    let mut bp = BezPath::new();
    bp.move_to((-50.0, 0.0));
    bp.line_to((10.0, 0.0));
    bp.curve_to((15.52, 0.0), (20.0, 4.48), (20.0, 10.0));
    bp.line_to((-80.0, 10.0));
    let track = &crate::track::tracks(&bp, 0.05)[0];
    let l = track.len();
    for kind in AUTO {
        let out = stroke_pieces(&Document::new(1.0, 1.0), &auto_corner_brush(Some(kind), Some(kind)), &bp, &stroke(1.0));
        let mut s = 0.25;
        while s < l - 0.25 {
            let q = track.map(s, 0.0);
            assert!(out.iter().any(|n| covers(n, q)), "{kind:?}: the band is filled at {s:.2} of {l:.2} ({q:?})");
            s += 0.25;
        }
    }
}

/// Where a curve meets a sharp corner, the two halves of a corner tile still meet on the corner's
/// diagonal: the band is filled across its width near the corner, and both Auto-Between halves end
/// at the diagonal's two ends.
#[test]
fn pattern_auto_corners_after_a_curve_meet_across_the_band() {
    // A quarter circle of radius 4 straight into a sharp turn.
    let mut bp = BezPath::new();
    bp.move_to((-50.0, 0.0));
    bp.line_to((10.0, 0.0));
    bp.curve_to((12.21, 0.0), (14.0, 1.79), (14.0, 4.0));
    bp.line_to((-80.0, 4.0));
    let track = &crate::track::tracks(&bp, 0.05)[0];
    let corner = track.corner_positions().first().map(|c| c.1).unwrap();
    for kind in [CornerSource::AutoCentered, CornerSource::AutoBetween] {
        let out = stroke_pieces(&Document::new(1.0, 1.0), &auto_corner_brush(Some(kind), Some(kind)), &bp, &stroke(1.0));
        let mut s = corner - 3.0;
        while s < corner + 3.0 {
            for j in -19..=19 {
                let y = j as f64 / 10.0;
                let q = track.map(s, y);
                assert!(out.iter().any(|n| covers(n, q)), "{kind:?}: the band is filled at {s:.2} ({y} across, {q:?})");
            }
            s += 0.05;
        }
    }
    // The path leaves the corner at (14, 4) heading left, so the 4 pt band's diagonal ends on the
    // lines y = 2 and y = 6, where both Auto-Between halves end at the same point.
    let both = auto_corner_brush(Some(CornerSource::AutoBetween), Some(CornerSource::AutoBetween));
    let out = stroke_pieces(&Document::new(1.0, 1.0), &both, &bp, &stroke(1.0));
    for edge in [2.0, 6.0] {
        let on_edge: Vec<Vec<Point>> = out
            .iter()
            .map(|n| points_of(std::slice::from_ref(n)).into_iter().filter(|q| near(q.y, edge, 1e-6) && near(q.x, 14.0, 3.0)).collect())
            .collect();
        let shared = on_edge.iter().enumerate().any(|(i, a)| on_edge.iter().skip(i + 1).any(|b| a.iter().any(|p| reaches(b, (p.x, p.y), 1e-6))));
        assert!(shared, "both halves end at the same point on y = {edge}");
    }
}

/// On a path shorter than the side tile, a corner tile is squeezed into the path it has: it does
/// not run past the ends, nor past half the way to the next corner.
#[test]
fn pattern_auto_corners_stay_within_a_short_path() {
    let mut bp = BezPath::new();
    bp.move_to((0.0, 0.0));
    bp.line_to((2.0, 0.0));
    bp.line_to((2.0, 2.0));
    let mut zigzag = BezPath::new();
    zigzag.move_to((0.0, 0.0));
    for j in 1..=6 {
        zigzag.line_to((3.0 * j as f64, if j % 2 == 1 { 3.0 } else { 0.0 }));
    }
    for kind in AUTO {
        for (path, (x0, y0, x1, y1)) in [(&bp, (0.0, 0.0, 2.0, 2.0)), (&zigzag, (0.0, 0.0, 18.0, 3.0))] {
            let out = stroke_pieces(&Document::new(1.0, 1.0), &auto_corner_brush(Some(kind), Some(kind)), path, &stroke(1.0));
            assert!(!out.is_empty(), "{kind:?}: something is drawn");
            // The band is 4 pt wide, and the diagonal of a sharp turn reaches further from it.
            let reach = 2.0 * 3.0;
            for p in points_of(&out) {
                assert!(p.x >= x0 - reach && p.x <= x1 + reach && p.y >= y0 - reach && p.y <= y1 + reach, "{kind:?}: {p:?} is past the path's ends");
            }
        }
    }
}

/// A corner squeezed into a path shorter than the band is wide still fills the band up to the
/// corner's diagonal from both sides, with no gap between the two halves.
#[test]
fn pattern_auto_corners_on_a_short_path_meet_on_the_diagonal() {
    let mut bp = BezPath::new();
    bp.move_to((0.0, 0.0));
    bp.line_to((1.0, 0.0));
    bp.line_to((1.0, 1.0));
    for kind in [CornerSource::AutoCentered, CornerSource::AutoBetween] {
        let out = stroke_pieces(&Document::new(1.0, 1.0), &auto_corner_brush(Some(kind), Some(kind)), &bp, &stroke(1.0));
        // The diagonal runs from the corner (1, 0) to the band's outer corner (3, -2).
        for f in [0.25, 0.5, 0.75] {
            let (dx, dy) = (1.0 + 2.0 * f, -2.0 * f);
            for q in [Point::new(dx - 0.05, dy), Point::new(dx, dy - 0.05)] {
                assert!(out.iter().any(|n| covers(n, q)), "{kind:?}: the band is filled at {q:?}");
            }
        }
    }
}

#[test]
fn pattern_without_corner_tiles_or_auto_corners_leaves_corners_empty() {
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let doc = Document::new(1.0, 1.0);
    let plain = stroke_pieces(&doc, &auto_corner_brush(None, None), &sq, &stroke(1.0));
    assert_eq!(plain.len(), 40, "ten side tiles a side");
    let pts = points_of(&plain);
    for (corner, outside) in SQUARE_CORNERS {
        assert!(!reaches(&pts, outside, 1.5), "no art at the band's outer corner at {corner:?}");
    }
    // A square turns the same way at every corner, so only one of the two kinds of corner tile
    // draws on it; the other leaves the stroke as it is without automatic corners.
    let outer = stroke_pieces(&doc, &auto_corner_brush(Some(CornerSource::AutoBetween), None), &sq, &stroke(1.0));
    let inner = stroke_pieces(&doc, &auto_corner_brush(None, Some(CornerSource::AutoBetween)), &sq, &stroke(1.0));
    assert!((outer == plain) != (inner == plain), "exactly one kind of corner draws on a square");
}

#[test]
fn pattern_corner_art_wins_over_corner_sources() {
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let doc = Document::new(1.0, 1.0);
    let saved = pattern_brush(PatternFit::Stretch, true);
    let mut with_auto = saved.clone();
    if let BrushKind::Pattern(p) = &mut with_auto.kind {
        p.outer_source = Some(CornerSource::AutoSliced);
        p.inner_source = Some(CornerSource::None);
    }
    assert_eq!(stroke_pieces(&doc, &with_auto, &sq, &stroke(1.0)), stroke_pieces(&doc, &saved, &sq, &stroke(1.0)));
}

/// A kind of corner without art takes the other kind's art, as saved brushes draw, unless its
/// source is None.
#[test]
fn pattern_corner_source_none_leaves_that_kind_of_corner_empty() {
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let doc = Document::new(1.0, 1.0);
    let draw = |b: &Brush| stroke_pieces(&doc, b, &sq, &stroke(1.0));
    let (saved, plain) = (draw(&pattern_brush(PatternFit::Stretch, true)), draw(&auto_corner_brush(None, None)));
    let without = |outer: bool, source: Option<CornerSource>| {
        let mut b = pattern_brush(PatternFit::Stretch, true);
        if let BrushKind::Pattern(p) = &mut b.kind {
            if outer {
                (p.outer_corner, p.outer_source) = (None, source);
            } else {
                (p.inner_corner, p.inner_source) = (None, source);
            }
        }
        draw(&b)
    };
    assert!(without(true, None) == saved && without(false, None) == saved, "the other kind's art fills in");
    let (outer, inner) = (without(true, Some(CornerSource::None)), without(false, Some(CornerSource::None)));
    assert!((outer == plain && inner == saved) || (inner == plain && outer == saved), "only the corners set to None are empty");
}

#[test]
fn pattern_corner_sources_round_trip_and_stay_out_of_brushes_without_them() {
    let saved = serde_json::to_value(pattern_brush(PatternFit::Stretch, true)).unwrap();
    assert!(saved.get("outer_source").is_none() && saved.get("inner_source").is_none(), "{saved}");
    let BrushKind::Pattern(p) = Brush::from_json(saved).unwrap().kind else { panic!("a pattern brush") };
    assert_eq!((p.outer_source, p.inner_source), (None, None));
    let v = serde_json::to_value(auto_corner_brush(Some(CornerSource::AutoCentered), Some(CornerSource::None))).unwrap();
    assert_eq!((v["outer_source"].as_str(), v["inner_source"].as_str()), (Some("autoCentered"), Some("none")));
    let BrushKind::Pattern(p) = Brush::from_json(v).unwrap().kind else { panic!("a pattern brush") };
    assert_eq!((p.outer_source, p.inner_source), (Some(CornerSource::AutoCentered), Some(CornerSource::None)));
}

/// Auto-Sliced masks each side tile at the corner's diagonal instead of cutting its paths, so a
/// tile's outline gains no stroke along the cut.
#[test]
fn pattern_auto_sliced_masks_stroked_tiles_at_the_diagonal() {
    let mut b = auto_corner_brush(Some(CornerSource::AutoSliced), Some(CornerSource::AutoSliced));
    if let BrushKind::Pattern(p) = &mut b.kind {
        p.side.appearance = Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::WHITE), 0.5).into();
    }
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &sq, &stroke(1.0));
    let clips: Vec<&Node> = out.iter().filter(|n| matches!(n.kind, NodeKind::Group { clip: true, .. })).collect();
    assert_eq!(clips.len(), 8, "two masked tiles at each corner");
    for g in clips {
        let kids = g.children().unwrap();
        assert_eq!(kids.len(), 2);
        assert!(kids[0].appearance.items.is_empty(), "the mask paints nothing");
        let mask = kids[0].path_data().unwrap().to_bezpath();
        let tile = kids[1].path_data().unwrap();
        assert!(
            tile.anchors().any(|(_, _, a)| kurbo::Shape::winding(&mask, a.p) == 0),
            "the tile keeps its outline past the diagonal, where the mask hides it"
        );
    }
}

/// Auto-Sliced on a corner whose legs are too short for the side tile squeezes the tile along,
/// but not its stroke: the mask still takes in the stroke past the tile's far end.
#[test]
fn pattern_auto_sliced_masks_keep_the_stroke_of_a_squeezed_tile() {
    let mut b = auto_corner_brush(Some(CornerSource::AutoSliced), Some(CornerSource::AutoSliced));
    if let BrushKind::Pattern(p) = &mut b.kind {
        p.side.appearance = Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::WHITE), 4.0).into();
    }
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 8.0, 8.0)).to_bezpath();
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &sq, &stroke(1.0));
    let clips: Vec<&Node> = out.iter().filter(|n| matches!(n.kind, NodeKind::Group { clip: true, .. })).collect();
    assert_eq!(clips.len(), 8, "two masked tiles at each corner");
    let center = Point::new(4.0, 4.0);
    for g in clips {
        let kids = g.children().unwrap();
        let mask = kids[0].path_data().unwrap().to_bezpath();
        let half = kids[1].appearance.stroke_width() / 2.0;
        assert!(half > 0.0);
        let tile = kids[1].path_data().unwrap();
        let anchors: Vec<Point> = tile.anchors().map(|(_, _, a)| a.p).collect();
        let mid = anchors.iter().fold(Point::ZERO, |m, a| m + a.to_vec2() / anchors.len() as f64);
        // The tile's corner, and its diagonal through the square's center; each mask keeps one
        // side of the diagonal.
        let p = [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)]
            .map(|(x, y)| Point::new(x, y))
            .into_iter()
            .min_by(|a, b| a.distance(mid).total_cmp(&b.distance(mid)))
            .unwrap();
        let cross = |q: Point| (q - p).cross(center - p);
        let side = cross(mid).signum();
        for a in &anchors {
            // The points of the stroke round the anchor, along and across the tile's leg.
            for d in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                let q = *a + kurbo::Vec2::new(d.0, d.1) * half * 0.9;
                if cross(q) * side > 0.01 && (q - p).hypot() < 7.0 {
                    assert!(kurbo::Shape::winding(&mask, q) != 0, "the mask takes in the stroke at {q:?} (half width {half})");
                }
            }
        }
    }
}

#[test]
fn bristle_makes_translucent_strands() {
    let b = Brush { name: "b".into(), kind: BrushKind::Bristle(Bristle::default()) };
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 200.0, 0.0), &stroke(1.0));
    assert!(out.len() >= 5, "{}", out.len());
    assert!(out.iter().all(|n| n.opacity < 1.0 && n.opacity > 0.0));
    let bb = bounds(&out);
    assert!(bb.height() <= 6.0 * 1.3 && bb.height() >= 3.0, "{bb:?}");
}

#[test]
fn tints_colorization_uses_stroke_colour() {
    let b = art_brush(rect_art(10.0, 4.0), ArtScale::Stretch);
    let BrushKind::Art(mut a) = b.kind else { panic!("not an art brush") };
    a.colorization = Colorization::Tints;
    let b = Brush { name: "t".into(), kind: BrushKind::Art(a) };
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 50.0, 0.0), &stroke(1.0));
    assert_eq!(out[0].appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    let mut white = rect_art(1.0, 1.0);
    white.appearance.set_fill(Paint::solid(Color::WHITE));
    colorize(&mut white, Colorization::Tints, &Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert_eq!(white.appearance.fill_paint().color().unwrap().to_rgb(), [1.0, 1.0, 1.0]);
}

/// Point type at (0, 0) whose characters are painted `fill` and `stroke` (1 pt wide).
fn type_art(fill: Paint, stroke: Paint) -> Node {
    use vectorcraft_doc::{CharStyle, TextObject};
    let style = CharStyle { fill, stroke, stroke_width: 1.0, ..CharStyle::default() };
    Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(0.0, 0.0), "Ag", style))))
}

/// The characters' fills and strokes of every piece of type in `nodes`.
fn run_paints(nodes: &[Node]) -> Vec<(Paint, Paint)> {
    let mut out = vec![];
    for n in nodes {
        n.walk(&mut |c| {
            if let NodeKind::Text(t) = &c.kind {
                out.extend(t.runs.iter().map(|r| (r.style.fill.clone(), r.style.stroke.clone())));
            }
        });
    }
    out
}

/// A scatter brush's colorization recolors the type in its art as it recolors the art's paths.
#[test]
fn scatter_colorization_recolors_type() {
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let blue = StrokeLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 1.0);
    let mut path = rect_art(10.0, 4.0);
    path.appearance.set_fill(red.clone());
    let art = Node::group(NodeId(0), vec![Arc::new(path), Arc::new(type_art(red.clone(), red.clone()))]);
    let b = Brush { name: "s".into(), kind: BrushKind::Scatter(Scatter { art, colorization: Colorization::Tints, ..Default::default() }) };
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 100.0, 0.0), &blue);
    assert!(!out.is_empty());
    let path_fill = out[0].children().unwrap()[0].appearance.fill_paint();
    assert_ne!(path_fill, red);
    let paints = run_paints(&out);
    assert_eq!(paints.len(), out.len());
    assert!(paints.iter().all(|p| *p == (path_fill.clone(), path_fill.clone())), "{paints:?}");
    // Black type takes the stroke color itself.
    let black = Paint::solid(Color::BLACK);
    let b = Brush {
        name: "s".into(),
        kind: BrushKind::Scatter(Scatter { art: type_art(black.clone(), black), colorization: Colorization::Tints, ..Default::default() }),
    };
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 100.0, 0.0), &blue);
    assert!(!out.is_empty());
    assert!(run_paints(&out).iter().all(|p| *p == (blue.paint.clone(), blue.paint.clone())));
}

/// The Symbol Stainer mixes the characters' strokes of type toward the stain, as it mixes their fills.
#[test]
fn stain_mixes_type_strokes_too() {
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let blue = Color::rgb(0.0, 0.0, 1.0);
    let mut art = type_art(red.clone(), red);
    tint_node(&mut art, &blue, 1.0);
    assert_eq!(run_paints(&[art]), [(Paint::solid(blue), Paint::solid(blue))]);
    // Characters painted with a global swatch lose the link, as a path's paint does: the mixed color
    // is not the swatch's color.
    let linked = Paint::Solid { color: Color::rgb(1.0, 0.0, 0.0), swatch: Some("Red".into()), tint: 0.5 };
    let mut art = type_art(linked.clone(), linked);
    tint_node(&mut art, &blue, 1.0);
    assert_eq!(run_paints(&[art]), [(Paint::solid(blue), Paint::solid(blue))]);
}

#[test]
fn library_defaults_store_and_find() {
    let mut d = Document::new(100.0, 100.0);
    let lib = library(&d);
    assert!(lib.len() >= 10);
    for kind in ["calligraphic", "scatter", "art", "pattern", "bristle"] {
        assert!(lib.iter().any(|b| b.kind.type_id() == kind), "{kind}");
    }
    assert!(find(&d, "Chain").is_some());
    let mut lib = lib;
    lib.retain(|b| b.name != "Chain");
    store(&mut d, &lib);
    assert!(find(&d, "Chain").is_none());
    assert_eq!(library(&d).len(), lib.len());
    assert_eq!(unique_name(&lib, "Arrow"), "Arrow 2");
}

#[test]
fn brushes_serde_round_trip() {
    let lib = defaults().to_vec();
    let s = serde_json::to_string(&lib).unwrap();
    let back: Vec<Brush> = serde_json::from_str(&s).unwrap();
    assert_eq!(back, lib);
    // Partial definitions fill in defaults.
    let b: Brush = serde_json::from_value(serde_json::json!({"name": "x", "type": "calligraphic", "angle": 30})).unwrap();
    assert_eq!(b.kind, BrushKind::Calligraphic(Calligraphic { angle: 30.0, ..Default::default() }));
}

#[test]
fn every_default_brush_draws_something() {
    let bp = preview_path(120.0, 24.0);
    for b in defaults() {
        let out = stroke_pieces(&Document::new(1.0, 1.0), b, &bp, &stroke(1.0));
        assert!(!out.is_empty(), "{}", b.name);
        let bb = bounds(&out);
        assert!(bb.width() > 50.0 && bb.width() < 200.0, "{}: {bb:?}", b.name);
    }
}

#[test]
fn expand_keeps_fill_and_adds_pieces() {
    let mut d = Document::new(200.0, 200.0);
    let mut ap = Appearance::basic(Paint::solid(Color::WHITE), Paint::solid(Color::BLACK), 1.0);
    ap.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    let n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 90.0)), ap);
    assert!(has_brush(&n));
    let g = expand(&d, &n).unwrap();
    let ch = g.children().unwrap();
    assert_eq!(ch.len(), 2);
    assert!(ch[0].appearance.stroke().is_none() && !ch[0].appearance.fill_paint().is_none());
    assert_eq!(ch[1].appearance.fill_paint(), Paint::solid(Color::BLACK));
    let b = ch[1].geometric_bounds().unwrap();
    assert!(near(b.x0, 8.5, 0.05) && near(b.x1, 91.5, 0.05), "{b:?}");
    let plain = Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::default_art());
    assert!(expand(&d, &plain).is_none());
}

/// A Calligraphic brush (round nib) whose size follows the pen pressure by `var` either way.
fn pressure_calli(size: f64, var: f64) -> Brush {
    let c = Calligraphic {
        size,
        variation: [0.0, 0.0, var],
        modes: Some([Variation::Fixed, Variation::Fixed, Variation::Pressure]),
        ..Default::default()
    };
    Brush { name: "p".into(), kind: BrushKind::Calligraphic(c) }
}

/// The stroke's half-width (largest |y|) among outline points with x in `x0..x1`.
fn half_width(nodes: &[Node], x0: f64, x1: f64) -> f64 {
    let mut pts = vec![];
    for n in nodes {
        all_points(n, &mut pts);
    }
    pts.iter().filter(|p| p.x >= x0 && p.x <= x1).map(|p| p.y.abs()).fold(0.0, f64::max)
}

fn pressed(points: Vec<(f64, f64)>) -> StrokeLayer {
    StrokeLayer { pressure: Some(vectorcraft_doc::PressureProfile { points }), ..stroke(1.0) }
}

/// #852: with Size on Pressure the nib goes from size − variation at the lightest pressure to
/// size + variation at the heaviest, along the stroke where the pen pressed.
#[test]
fn pressure_maps_to_size_along_the_stroke() {
    let b = pressure_calli(10.0, 4.0);
    let light_to_heavy = pressed(vec![(0.0, 0.0), (1.0, 1.0)]);
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 100.0, 0.0), &light_to_heavy);
    // Lightest: 6 pt (half 3); heaviest: 14 pt (half 7); in between the pressure there.
    assert!(near(half_width(&out, 0.5, 1.5), 3.0, 0.1), "{}", half_width(&out, 0.5, 1.5));
    assert!(near(half_width(&out, 98.5, 99.5), 7.0, 0.1), "{}", half_width(&out, 98.5, 99.5));
    assert!(near(half_width(&out, 49.5, 50.5), 5.0, 0.1), "{}", half_width(&out, 49.5, 50.5));
    // The stroke weight scales it.
    let heavy = StrokeLayer { width: 2.0, ..light_to_heavy };
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 100.0, 0.0), &heavy);
    assert!(near(half_width(&out, 98.5, 99.5), 14.0, 0.2));
    // No pressure recorded (a mouse, or a path the brush was applied to): the size itself.
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    assert!(near(bounds(&out).height(), 10.0, 0.05), "{:?}", bounds(&out));
    // Kept within the valid range: 2 ± 4 never goes below nothing.
    let out = stroke_pieces(&Document::new(1.0, 1.0), &pressure_calli(2.0, 4.0), &line(0.0, 0.0, 100.0, 0.0), &pressed(vec![(0.0, 0.0), (1.0, 0.0)]));
    assert!(!out.is_empty() && bounds(&out).height() < 0.2, "{:?}", bounds(&out));
}

/// Angle and roundness follow the pressure the same way, clamped to their ranges.
#[test]
fn pressure_varies_angle_and_roundness() {
    let c = |modes| Calligraphic { angle: 0.0, roundness: 60.0, size: 10.0, variation: [90.0, 60.0, 0.0], modes: Some(modes) };
    let horizontal = line(0.0, 0.0, 100.0, 0.0);
    let draw = |c: Calligraphic, p: f64| {
        let b = Brush { name: "a".into(), kind: BrushKind::Calligraphic(c) };
        bounds(&stroke_pieces(&Document::new(1.0, 1.0), &b, &horizontal, &pressed(vec![(0.0, p), (1.0, p)]))).height()
    };
    use Variation::{Fixed, Pressure};
    // Angle 0 ± 90: heavy pressure stands the flat nib up across the stroke.
    assert!(near(draw(c([Pressure, Fixed, Fixed]), 0.5), 6.0, 0.05));
    assert!(near(draw(c([Pressure, Fixed, Fixed]), 1.0), 10.0, 0.05));
    // Roundness 60 ± 60, kept within 1..100: light is a hairline, heavy is round.
    let vertical = Calligraphic { angle: 90.0, ..c([Fixed, Pressure, Fixed]) };
    let wide = |p| {
        let b = Brush { name: "r".into(), kind: BrushKind::Calligraphic(vertical.clone()) };
        bounds(&stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 0.0, 100.0), &pressed(vec![(0.0, p), (1.0, p)]))).width()
    };
    assert!(near(wide(0.0), 0.1, 0.05), "{}", wide(0.0));
    assert!(near(wide(1.0), 10.0, 0.05), "{}", wide(1.0));
}

/// Brushes saved before the variation modes keep their look: a variation means Random, none
/// means Fixed, and they are written back without modes.
#[test]
fn brushes_without_modes_vary_randomly_as_before() {
    let old: Brush = serde_json::from_value(serde_json::json!({"name": "x", "type": "calligraphic", "size": 8, "variation": [0, 0, 3]})).unwrap();
    let BrushKind::Calligraphic(c) = &old.kind else { panic!() };
    assert_eq!(c.modes, None);
    assert_eq!(c.modes(), [Variation::Fixed, Variation::Fixed, Variation::Random]);
    assert!(!c.uses_pressure());
    assert!(serde_json::to_value(&old).unwrap().get("modes").is_none(), "written back as it was");
    // The same strokes as the brush with its modes spelled out.
    let explicit = Brush {
        kind: BrushKind::Calligraphic(Calligraphic { modes: Some([Variation::Fixed, Variation::Fixed, Variation::Random]), ..c.clone() }),
        ..old.clone()
    };
    let bp = preview_path(120.0, 24.0);
    assert_eq!(
        stroke_pieces(&Document::new(1.0, 1.0), &old, &bp, &stroke(1.0)),
        stroke_pieces(&Document::new(1.0, 1.0), &explicit, &bp, &stroke(1.0))
    );
    // Fixed ignores the variation; Random still draws a different size per stroke.
    let fixed = Brush { kind: BrushKind::Calligraphic(Calligraphic { modes: Some([Variation::Fixed; 3]), ..c.clone() }), ..old.clone() };
    assert!(near(bounds(&stroke_pieces(&Document::new(1.0, 1.0), &fixed, &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0))).height(), 8.0, 0.05));
    // Recorded pressure changes nothing for a brush that doesn't use it.
    assert_eq!(
        stroke_pieces(&Document::new(1.0, 1.0), &old, &bp, &stroke(1.0)),
        stroke_pieces(&Document::new(1.0, 1.0), &old, &bp, &pressed(vec![(0.0, 0.0), (1.0, 1.0)]))
    );
    // Modes are saved by name.
    assert_eq!(serde_json::to_value(Variation::ALL).unwrap(), serde_json::json!(["fixed", "random", "pressure"]));
    assert!(Variation::ALL.iter().all(|v| Variation::parse(v.id()) == Some(*v)));
}

/// Junk definitions and pressure never panic and never draw non-finite art.
#[test]
fn junk_pressure_brushes_draw_finite_art() {
    let bp = preview_path(120.0, 24.0);
    for (size, var) in [(1e308, 1e308), (-1e308, 1e308), (0.0, -1e308), (f64::MAX, f64::MAX)] {
        for profile in [vec![], vec![(0.0, 2.0), (0.0, -1.0), (5.0, 0.5)], vec![(1.0, 1.0), (0.0, 0.0)]] {
            let out = stroke_pieces(&Document::new(1.0, 1.0), &pressure_calli(size, var), &bp, &pressed(profile));
            let mut pts = vec![];
            for n in &out {
                all_points(n, &mut pts);
            }
            assert!(pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()), "{size} {var}");
        }
    }
}

/// A scatter brush of 10 × 4 pt rectangles whose size goes from `lo` % to `hi` % as `mode` says.
fn sized_scatter(lo: f64, hi: f64, mode: Variation) -> Brush {
    let s = scatter_brush(100.0);
    let BrushKind::Scatter(sc) = s.kind else { panic!("not a scatter brush") };
    let modes = [mode, Variation::Fixed, Variation::Fixed, Variation::Fixed];
    Brush { kind: BrushKind::Scatter(Scatter { size: (lo, hi), modes: Some(modes), ..sc }), ..s }
}

/// Scatter Brush Options: Fixed takes the first value, Pressure goes from the first at the
/// lightest pen pressure to the second at the heaviest; brushes saved before modes keep their look.
#[test]
fn scatter_values_follow_their_modes() {
    let bp = line(0.0, 0.0, 400.0, 0.0);
    let heights = |b: &Brush, st: &StrokeLayer| -> Vec<f64> {
        stroke_pieces(&Document::new(1.0, 1.0), b, &bp, st).iter().map(|n| n.geometric_bounds().unwrap().height()).collect()
    };
    // Fixed: every copy at 50 %, whatever the second value.
    assert!(heights(&sized_scatter(50.0, 300.0, Variation::Fixed), &stroke(1.0)).iter().all(|h| near(*h, 2.0, 1e-6)));
    // Pressure: light at the start (50 %: 2 pt tall), heavy at the end (300 %: 12 pt).
    let ramp = pressed(vec![(0.0, 0.0), (1.0, 1.0)]);
    let h = heights(&sized_scatter(50.0, 300.0, Variation::Pressure), &ramp);
    let (first, last) = (h[0], h[h.len() - 1]);
    assert!(near(first, 2.0, 0.1) && last > 10.0, "{h:?}");
    assert!(h.windows(2).all(|w| w[1] >= w[0] - 1e-9), "grows with the pressure: {h:?}");
    // Without recorded pressure, half way between the two.
    assert!(heights(&sized_scatter(50.0, 300.0, Variation::Pressure), &stroke(1.0)).iter().all(|h| near(*h, 7.0, 1e-6)));
    // No modes saved: Random where the values differ, as before; Fixed and Random draw the same.
    let old = sized_scatter(50.0, 300.0, Variation::Random);
    let BrushKind::Scatter(sc) = &old.kind else { panic!("not a scatter brush") };
    assert_eq!(Scatter { modes: None, ..sc.clone() }.modes(), [Variation::Random, Variation::Fixed, Variation::Fixed, Variation::Fixed]);
    let unsaved = Brush { kind: BrushKind::Scatter(Scatter { modes: None, ..sc.clone() }), ..old.clone() };
    assert_eq!(
        stroke_pieces(&Document::new(1.0, 1.0), &old, &bp, &stroke(1.0)),
        stroke_pieces(&Document::new(1.0, 1.0), &unsaved, &bp, &stroke(1.0))
    );
    assert!(serde_json::to_value(&unsaved).unwrap().get("modes").is_none(), "not saved when unset");
}

/// Art Brush Options: with Width on Pressure the art goes from Width at the lightest pen pressure
/// to the second width at the heaviest, along the stroke where the pen pressed; Fixed keeps Width.
#[test]
fn art_brush_width_follows_the_pressure() {
    let brush = |mode: Option<Variation>, max: Option<f64>| {
        let mut b = art_brush(rect_art(10.0, 4.0), ArtScale::Stretch);
        if let BrushKind::Art(a) = &mut b.kind {
            (a.width, a.width_mode, a.width_max) = (50.0, mode, max);
        }
        b
    };
    let bp = line(0.0, 0.0, 100.0, 0.0);
    let draw = |b: &Brush, st: &StrokeLayer| stroke_pieces(&Document::new(1.0, 1.0), b, &bp, st);
    let ramp = pressed(vec![(0.0, 0.0), (1.0, 1.0)]);
    // 4 pt art at 50 %: 1 pt either side at the lightest; at 300 %: 6 pt at the heaviest.
    let out = draw(&brush(Some(Variation::Pressure), Some(300.0)), &ramp);
    assert!(near(half_width(&out, 0.0, 1.0), 1.0, 0.1), "{}", half_width(&out, 0.0, 1.0));
    assert!(near(half_width(&out, 99.0, 100.0), 6.0, 0.1), "{}", half_width(&out, 99.0, 100.0));
    assert!(near(half_width(&out, 49.0, 51.0), 3.5, 0.15), "{}", half_width(&out, 49.0, 51.0));
    // The stroke weight scales it.
    let out = draw(&brush(Some(Variation::Pressure), Some(300.0)), &StrokeLayer { width: 2.0, ..ramp.clone() });
    assert!(near(half_width(&out, 99.0, 100.0), 12.0, 0.2), "{}", half_width(&out, 99.0, 100.0));
    // Without recorded pressure, half way between the two widths.
    assert!(near(bounds(&draw(&brush(Some(Variation::Pressure), Some(300.0)), &stroke(1.0))).height(), 7.0, 1e-6));
    // Fixed (or a definition without a mode) keeps Width, whatever the pressure and the second width.
    for (mode, max) in [(None, Some(300.0)), (None, None), (Some(Variation::Pressure), None)] {
        let out = draw(&brush(mode, max), &ramp);
        assert!(near(bounds(&out).height(), 2.0, 1e-6), "{mode:?} {max:?}: {:?}", bounds(&out));
    }
    // A stroke in the art takes the width at the middle of its path: 50 % and 300 % for an even
    // light and heavy pressure, 175 % without recorded pressure.
    let mut stroked = brush(Some(Variation::Pressure), Some(300.0));
    if let BrushKind::Art(a) = &mut stroked.kind {
        let mut bp = BezPath::new();
        bp.move_to((0.0, 0.0));
        bp.line_to((10.0, 0.0));
        a.art = Node::path(NodeId(0), vectorcraft_geom::PathData::from_bezpath(&bp), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0));
    }
    for (st, want) in [(pressed(vec![(0.0, 0.0), (1.0, 0.0)]), 1.0), (pressed(vec![(0.0, 1.0), (1.0, 1.0)]), 6.0), (stroke(1.0), 3.5)] {
        let out = draw(&stroked, &st);
        let w = out.first().and_then(|n| n.appearance.stroke()).map(|s| s.width);
        assert!(w.is_some_and(|w| near(w, want, 1e-6)), "{want}: {w:?}");
    }
    // Stretch Between Guides: an end outside the guides keeps the proportions of the width at its
    // end of the stroke. A 2 pt marker at the start of 10 × 4 pt art (guides at 20 % and 80 %),
    // 100 % to 300 % wide: 2 pt long at the lightest pressure, 6 pt at the heaviest.
    let marker =
        Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, -1.0, 2.0, 1.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let mut spacer = rect_art(10.0, 4.0);
    spacer.appearance = Appearance::default().into();
    let art = Node::group(NodeId(0), vec![std::sync::Arc::new(spacer), std::sync::Arc::new(marker)]);
    let mut guided = art_brush(art, ArtScale::BetweenGuides { start: 0.2, end: 0.8 });
    if let BrushKind::Art(a) = &mut guided.kind {
        (a.width_mode, a.width_max) = (Some(Variation::Pressure), Some(300.0));
    }
    for (p, long) in [(0.0, 2.0), (1.0, 6.0)] {
        let out = draw(&guided, &pressed(vec![(0.0, p), (1.0, p)]));
        let m = out.first().and_then(|g| g.children()).and_then(|c| c.get(1)).and_then(|m| m.geometric_bounds());
        assert!(m.is_some_and(|m| near(m.x0, 0.0, 1e-6) && near(m.x1, long, 1e-6)), "{p}: {m:?}");
    }
    // A mode the width does not take is read as Fixed, and the second width is kept within range.
    let lib = parse_library(&serde_json::json!([
        {"name": "a", "type": "art", "art": rect_art(10.0, 4.0), "width_mode": "random", "width_max": 1e308},
        {"name": "b", "type": "art", "art": rect_art(10.0, 4.0), "width_mode": "pressure", "width_max": 0.0},
    ]));
    let [BrushKind::Art(a), BrushKind::Art(b)] = [&lib[0].kind, &lib[1].kind] else { panic!("{lib:?}") };
    assert_eq!((a.width_mode, a.width_max, b.width_mode, b.width_max), (None, Some(1000.0), Some(Variation::Pressure), Some(1.0)));
    // A brush without the new fields is written back without them.
    let plain = serde_json::to_value(art_brush(rect_art(10.0, 4.0), ArtScale::Stretch)).unwrap();
    assert!(plain.get("width_mode").is_none() && plain.get("width_max").is_none(), "{plain}");
}

/// Definitions from files and commands are kept within their ranges.
#[test]
fn sanitize_keeps_values_within_their_ranges() {
    let mut v = serde_json::to_value(sized_scatter(1e308, -1e308, Variation::Random)).unwrap();
    v["rotation"] = serde_json::json!([720.0, -1e300]);
    v["scatter"] = serde_json::json!([5000.0, 0.0]);
    let lib = parse_library(&serde_json::json!([
        v,
        {"name": "a", "type": "art", "art": rect_art(10.0, 4.0), "width": 1e308, "scale": {"mode": "betweenGuides", "start": -3.0, "end": 9.0}},
        {"name": "p", "type": "pattern", "side": rect_art(10.0, 4.0), "scale": 0.0, "spacing": -5.0},
        {"name": "c", "type": "calligraphic", "angle": 400.0, "roundness": -1.0, "size": 1e9, "variation": [1e9, -1.0, 2.0]},
        {"name": "b", "type": "bristle", "size": 0.0, "length": 1000.0, "density": 0.0},
    ]));
    let [s, a, p, c, b] = lib.as_slice() else { panic!("{lib:?}") };
    let BrushKind::Scatter(s) = &s.kind else { panic!() };
    assert_eq!((s.size, s.rotation, s.scatter), ((10000.0, 1.0), (180.0, -180.0), (1000.0, 0.0)));
    let BrushKind::Art(a) = &a.kind else { panic!() };
    assert_eq!((a.width, a.scale), (1000.0, ArtScale::BetweenGuides { start: 0.0, end: 1.0 }));
    let BrushKind::Pattern(p) = &p.kind else { panic!() };
    assert_eq!((p.scale, p.spacing), (1.0, 0.0));
    let BrushKind::Calligraphic(c) = &c.kind else { panic!() };
    assert_eq!((c.angle, c.roundness, c.size, c.variation), (180.0, 0.0, 1296.0, [180.0, 0.0, 2.0]));
    let BrushKind::Bristle(b) = &b.kind else { panic!() };
    assert_eq!((b.size, b.length, b.density), (0.1, 300.0, 1.0));
}

/// Hue Shift's key colour defaults to the art's most used colour.
#[test]
fn art_colors_list_the_most_used_first() {
    let red = Color::rgb(1.0, 0.0, 0.0);
    let blue = Color::rgb(0.0, 0.0, 1.0);
    let path =
        |c: Color| Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::basic(Paint::solid(c), Paint::None, 0.0));
    let g = Node::group(NodeId(0), vec![Arc::new(path(red)), Arc::new(path(blue)), Arc::new(path(blue))]);
    assert_eq!(art_colors([&g]), vec![blue, red]);
    // Type counts its characters' fills and drawn strokes.
    let text = type_art(Paint::solid(red), Paint::solid(red));
    let g = Node::group(NodeId(0), vec![Arc::new(path(blue)), Arc::new(text.clone())]);
    assert_eq!(art_colors([&g]), vec![red, blue]);
    let NodeKind::Text(mut t) = text.kind else { panic!("not type") };
    t.runs[0].style.stroke_width = 0.0;
    let text = Node::new(NodeId(0), NodeKind::Text(t));
    let g = Node::group(NodeId(0), vec![Arc::new(text), Arc::new(path(blue)), Arc::new(path(blue))]);
    assert_eq!(art_colors([&g]), vec![blue, red], "a stroke 0 wide is not counted");
    let b = defaults().iter().find(|b| b.name == "Chain").unwrap();
    assert_eq!(b.art().len(), 3, "the side and both corner tiles");
    assert!(calli(0.0, 100.0, 3.0).art().is_empty());
}

/// A Hue Shift definition without a key color takes the art's most used color, or black when the
/// art has no solid color; a key that is given is kept.
#[test]
fn hue_shift_without_a_key_takes_the_most_used_art_color() {
    let red = Color::rgb(1.0, 0.0, 0.0);
    let blue = Color::rgb(0.0, 0.0, 1.0);
    let path = |fill: Paint| Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::basic(fill, Paint::None, 0.0));
    let art = Node::group(NodeId(0), vec![Arc::new(path(Paint::solid(red))), Arc::new(path(Paint::solid(blue))), Arc::new(path(Paint::solid(blue)))]);
    let lib = parse_library(&serde_json::json!([
        {"name": "s", "type": "scatter", "art": art, "colorization": {"method": "hueShift"}},
        {"name": "a", "type": "art", "art": art, "colorization": {"method": "hueShift", "key": null}},
        {"name": "p", "type": "pattern", "side": art, "colorization": {"method": "hueShift", "key": red}},
        {"name": "n", "type": "art", "art": path(Paint::None), "colorization": {"method": "hueShift"}},
    ]));
    let keys: Vec<(&str, Colorization)> = lib.iter().map(|b| (b.name.as_str(), b.kind.colorization())).collect();
    assert_eq!(
        keys,
        vec![
            ("s", Colorization::HueShift { key: blue }),
            ("a", Colorization::HueShift { key: blue }),
            ("p", Colorization::HueShift { key: red }),
            ("n", Colorization::HueShift { key: Color::BLACK }),
        ]
    );
}

/// Brush pieces draw no brushed stroke, also not by name: a symbol instance whose art is brushed
/// is left out, a pattern paint whose tile is brushed becomes no paint, and plain ones stay.
#[test]
fn pieces_leave_out_what_draws_a_brushed_stroke_by_name() {
    use vectorcraft_doc::{PatternDef, Symbol};
    let mut doc = Document::new(100.0, 100.0);
    let mut brushed = rect_art(10.0, 4.0);
    brushed.appearance.items.push(vectorcraft_doc::AppearanceItem::Stroke(StrokeLayer { brush: Some("s".into()), ..stroke(1.0) }));
    for (name, art) in [("plain", rect_art(10.0, 4.0)), ("brushed", brushed)] {
        doc.symbols.push(Symbol { name: name.into(), art: std::sync::Arc::new(art.clone()) });
        doc.patterns.push(PatternDef::new(name, vec![std::sync::Arc::new(art)]));
    }
    let instance = |symbol: &str| Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: symbol.into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let painted = |pattern: &str| {
        let fill = Paint::Pattern { pattern: pattern.into(), xf: vectorcraft_geom::Affine::IDENTITY };
        Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 4.0)), Appearance::basic(fill, Paint::None, 0.0))
    };
    let art = Node::group(NodeId(0), [instance("plain"), instance("brushed"), painted("plain"), painted("brushed")].map(std::sync::Arc::new).into());
    let b = Brush { name: "s".into(), kind: BrushKind::Scatter(Scatter { art, colorization: Colorization::None, ..Default::default() }) };
    let out = stroke_pieces(&doc, &b, &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    assert!(!out.is_empty());
    for piece in &out {
        let children = piece.children().unwrap();
        let symbols: Vec<&str> = children
            .iter()
            .filter_map(|c| if let NodeKind::SymbolInstance { symbol, .. } = &c.kind { Some(symbol.as_str()) } else { None })
            .collect();
        assert_eq!(symbols, ["plain"]);
        let fills: Vec<Paint> =
            children.iter().filter(|c| c.path_data().is_some()).filter_map(|c| c.appearance.fill().map(|f| f.paint.clone())).collect();
        assert_eq!(fills, [Paint::Pattern { pattern: "plain".into(), xf: vectorcraft_geom::Affine::IDENTITY }, Paint::None]);
    }
}

/// Anchors a brushed stroke emitted, which is what the output budget counts.
fn anchors(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|n| match &n.kind {
            NodeKind::Path { path, .. } => path.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>(),
            _ => 0,
        })
        .sum()
}

/// A path of `subpaths` long parallel lines, each long enough to want more tiles than the
/// per-run cap allows.
fn many_long_subpaths(subpaths: usize) -> BezPath {
    let mut bp = BezPath::new();
    for i in 0..subpaths {
        let y = i as f64 * 50.0;
        bp.move_to((0.0, y));
        bp.line_to((300_000.0, y));
    }
    bp
}

/// #3594: the per-factor caps are per track, and a path has as many tracks as it has subpaths,
/// so they multiply instead of bounding. Measured before the budget: 20,000 nodes and 320,000
/// anchors for one subpath, and exactly N times that for N, so sixteen gave 5,120,000.
#[test]
fn pattern_output_is_capped_across_subpaths_not_only_per_run() {
    let brush = pattern_brush(PatternFit::Stretch, false);
    let one = stroke_pieces(&Document::new(1.0, 1.0), &brush, &many_long_subpaths(1), &stroke(1.0));
    assert_eq!(one.len(), 20_000, "one track still fills its own per-run cap");
    let a1 = anchors(&one);
    assert!(a1 < 2_000_000, "one track stays under the total budget: {a1}");

    // Two tracks are well inside the budget, so nothing is cut: it must not bite before the
    // per-factor caps have actually multiplied past it.
    let two = stroke_pieces(&Document::new(1.0, 1.0), &brush, &many_long_subpaths(2), &stroke(1.0));
    assert_eq!(two.len(), 2 * 20_000, "two tracks are under the budget and keep every piece");
    assert_eq!(anchors(&two), 2 * a1);

    // Enough tracks that the per-run caps alone would pass the budget. Derived from what one
    // track costs rather than written down, because how many anchors a tile holds is not fixed:
    // a tile on a straight run keeps its own anchors instead of being flattened (#3926), which
    // moved this by a factor of four.
    let tracks = 2_000_000 / a1 + 2;
    let many = stroke_pieces(&Document::new(1.0, 1.0), &brush, &many_long_subpaths(tracks), &stroke(1.0));
    let an = anchors(&many);
    assert!(an <= 2_000_000, "{tracks} subpaths must not exceed the budget: {an}");
    assert!(an > anchors(&two), "but more is drawn than two tracks' worth: {an}");
    assert!(many.len() < tracks * 20_000, "and fewer pieces than the per-run caps alone would allow: {}", many.len());
}

/// The same for scatter, whose per-track guard is also 20,000 copies.
#[test]
fn scatter_output_is_capped_across_subpaths() {
    let b = Brush {
        name: "s".into(),
        kind: BrushKind::Scatter(Scatter { art: rect_art(4.0, 4.0), colorization: Colorization::None, ..Default::default() }),
    };
    let few = stroke_pieces(&Document::new(1.0, 1.0), &b, &many_long_subpaths(1), &stroke(1.0));
    let per_track = anchors(&few);
    assert!(per_track > 0, "one track draws something");
    // Enough tracks that the per-track guards alone would pass the budget.
    let tracks = 2_000_000 / per_track + 4;
    let many = stroke_pieces(&Document::new(1.0, 1.0), &b, &many_long_subpaths(tracks), &stroke(1.0));
    let an = anchors(&many);
    assert!(an <= 2_000_000, "{tracks} tracks must not exceed the budget: {an}");
    assert!(an > per_track, "but more is drawn than one track's worth: {an}");
    assert!(many.len() < tracks * few.len().max(1), "fewer copies than the per-track guards alone would allow");
}

/// A short, ordinary stroke is untouched by the budget: it is the absurd ones that are cut.
#[test]
fn an_ordinary_stroke_is_unaffected_by_the_budget() {
    let brush = pattern_brush(PatternFit::Stretch, false);
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let out = stroke_pieces(&Document::new(1.0, 1.0), &brush, &sq, &stroke(1.0));
    assert_eq!(out.len(), 40, "ten tiles on each of the four sides, as before the budget");
    assert!(anchors(&out) < 2_000_000);
}

/// Anchors under `n`, and how many of them keep a handle (so the segment is a curve).
fn anchors_and_curved(n: &Node) -> (usize, usize) {
    let NodeKind::Path { path, .. } = &n.kind else { return (0, 0) };
    let all: Vec<_> = path.subpaths.iter().flat_map(|s| s.anchors.iter()).collect();
    (all.len(), all.iter().filter(|a| a.h_in != a.p || a.h_out != a.p).count())
}

/// #3926: a Pattern Brush on a straight path only moves, turns and scales its tile, so the
/// tile's curves survive. They used to be flattened into chords by the bending code, which is
/// visible in the stroke itself and leaves no handles after Expand Brush Strokes.
#[test]
fn a_pattern_tile_keeps_its_curves_on_a_straight_path() {
    let circle =
        Node::path(NodeId(0), shapes::ellipse(Rect::new(0.0, -20.0, 40.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let (tile_anchors, tile_curved) = anchors_and_curved(&circle);
    assert_eq!((tile_anchors, tile_curved), (4, 4), "the tile is a 4-anchor smooth circle");

    let brush = Brush {
        name: "p".into(),
        kind: BrushKind::Pattern(PatternBrush { side: circle, fit: PatternFit::Stretch, colorization: Colorization::None, ..Default::default() }),
    };
    let out = stroke_pieces(&Document::new(1.0, 1.0), &brush, &line(0.0, 0.0, 480.0, 0.0), &stroke(1.0));
    assert!(!out.is_empty(), "the brush drew something");
    for n in &out {
        assert_eq!(anchors_and_curved(n), (4, 4), "each circle is still a 4-anchor curve, not a polygon");
    }
    // The ink is in the same place: a circle tile still covers the path it was laid along.
    let b = bounds(&out);
    assert!(near(b.width(), 480.0, 1.0), "the tiles still span the path: {}", b.width());
    assert!(near(b.height(), 40.0, 1.0), "and still stand the tile's height: {}", b.height());
}

/// The other half of the same behaviour: where the path really does bend, the tile is still
/// subdivided, because an affine cannot reproduce a bend. The fit is checked before it is used,
/// so a bent run falls back to exactly what it did before.
#[test]
fn a_pattern_tile_on_a_bent_path_is_still_subdivided() {
    let circle =
        Node::path(NodeId(0), shapes::ellipse(Rect::new(0.0, -20.0, 40.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let brush = Brush {
        name: "p".into(),
        kind: BrushKind::Pattern(PatternBrush { side: circle, fit: PatternFit::Stretch, colorization: Colorization::None, ..Default::default() }),
    };
    // A circle to run along: every tile sits on a curving section.
    let round = shapes::ellipse(Rect::new(0.0, 0.0, 400.0, 400.0)).to_bezpath();
    let out = stroke_pieces(&Document::new(1.0, 1.0), &brush, &round, &stroke(1.0));
    assert!(!out.is_empty());
    let bent = out.iter().filter(|n| anchors_and_curved(n).0 > 8).count();
    assert!(bent > out.len() / 2, "tiles on a bend are still densified: {bent} of {}", out.len());
}

/// The kind of every node in a piece of brushed art, in paint order.
fn kinds(n: &Node) -> Vec<&'static str> {
    let mut out = vec![match &n.kind {
        NodeKind::Path { .. } => "path",
        NodeKind::Group { .. } => "group",
        NodeKind::SymbolInstance { .. } => "symbol",
        _ => "other",
    }];
    for c in n.children().into_iter().flatten() {
        out.extend(kinds(c));
    }
    out
}

/// The straight-run route differs from the bending route in the tile's curves and in nothing else,
/// which includes the node kinds it keeps. A tile holding a kind neither route can map (here a
/// symbol instance, whose art lives in the symbol and not in the node) loses it on a straight run
/// exactly as it does on a bend: an affine is not a way for unmappable art to arrive untouched.
#[test]
fn a_straight_run_keeps_the_same_node_kinds_as_a_bend() {
    let circle =
        Node::path(NodeId(0), shapes::ellipse(Rect::new(0.0, -20.0, 40.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let sym = Node::new(NodeId(1), NodeKind::SymbolInstance { symbol: "s".into(), xf: Affine::IDENTITY });
    let tile = Node::group(NodeId(2), vec![Arc::new(circle), Arc::new(sym)]);
    let brush = Brush {
        name: "p".into(),
        kind: BrushKind::Pattern(PatternBrush { side: tile, fit: PatternFit::Stretch, colorization: Colorization::None, ..Default::default() }),
    };
    let doc = Document::new(1.0, 1.0);
    let straight = stroke_pieces(&doc, &brush, &line(0.0, 0.0, 480.0, 0.0), &stroke(1.0));
    let round = shapes::ellipse(Rect::new(0.0, 0.0, 400.0, 400.0)).to_bezpath();
    let bent = stroke_pieces(&doc, &brush, &round, &stroke(1.0));
    assert!(!straight.is_empty() && !bent.is_empty(), "both paths drew something");
    for (what, out) in [("a straight run", &straight), ("a bend", &bent)] {
        for n in out {
            assert_eq!(kinds(n), ["group", "path"], "{what} keeps the path and drops the symbol instance");
        }
    }
}
