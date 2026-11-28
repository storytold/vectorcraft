use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{Affine, BezPath, Point, Rect, Vec2, shapes};

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

/// Points along every outline under `n`, its anchors among them. Bent art keeps its curves and
/// long lines since #3926, so its anchors alone no longer trace it.
fn all_points(n: &Node, out: &mut Vec<Point>) {
    n.walk(&mut |c| {
        for sp in c.path_data().into_iter().flat_map(|p| &p.subpaths) {
            out.extend(sp.anchors.iter().map(|a| a.p));
            for i in 0..sp.segment_count() {
                let seg = sp.segment(i);
                out.extend((1..32).map(|j| kurbo::ParamCurve::eval(&seg, j as f64 / 32.0)));
            }
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

/// Scale Proportionately art is scaled across by the stroke weight and the Width, as Stretch art
/// is, on top of the scale the path length gives it.
#[test]
fn art_brush_proportional_grows_with_weight() {
    let brush = |width: f64, overlap: bool| {
        let mut b = art_brush(rect_art(100.0, 4.0), ArtScale::Proportional);
        if let BrushKind::Art(a) = &mut b.kind {
            (a.width, a.overlap) = (width, overlap);
        }
        b
    };
    let draw = |b: &Brush, bp: &BezPath, st: &StrokeLayer| stroke_pieces(&Document::new(1.0, 1.0), b, bp, st);
    // 3 pt stroke → 3× wider than at 1 pt (8 pt); the length still follows the path.
    let b = bounds(&draw(&brush(100.0, false), &line(0.0, 0.0, 200.0, 0.0), &stroke(3.0)));
    assert!(near(b.height(), 24.0, 1e-6) && near(b.x0, 0.0, 1e-6) && near(b.x1, 200.0, 1e-6), "{b:?}");
    // The Width (50 %) applies on top of the stroke weight.
    let b = bounds(&draw(&brush(50.0, false), &line(0.0, 0.0, 200.0, 0.0), &stroke(3.0)));
    assert!(near(b.height(), 12.0, 1e-6), "{b:?}");
    // A path twice as long at twice the weight (as Scale Strokes & Effects at 200 % leaves it):
    // 2× from the length and 2× from the weight.
    let b = bounds(&draw(&brush(100.0, false), &line(0.0, 0.0, 400.0, 0.0), &stroke(2.0)));
    assert!(near(b.height(), 32.0, 1e-6), "{b:?}");
    // With Overlap, each run between corners is scaled by its own length and by the weight: the
    // 100 pt run 4 pt × 3 across, the 50 pt run 2 pt × 3.
    let mut bent = BezPath::new();
    bent.move_to((0.0, 0.0));
    bent.line_to((100.0, 0.0));
    bent.line_to((100.0, 50.0));
    let out = draw(&brush(100.0, true), &bent, &stroke(3.0));
    assert_eq!(out.len(), 2);
    let (r0, r1) = (out[0].geometric_bounds().unwrap(), out[1].geometric_bounds().unwrap());
    assert!(near(r0.height(), 12.0, 1e-6) && near(r1.width(), 6.0, 1e-6), "{r0:?} {r1:?}");
    // A width that follows the pen: 50 % at the lightest pressure, 300 % at the heaviest, times
    // the 2 pt weight (4 pt art on a 100 pt path: 4 pt across at 100 %, 1 pt either side).
    let mut pen = brush(50.0, false);
    if let BrushKind::Art(a) = &mut pen.kind {
        (a.width_mode, a.width_max) = (Some(Variation::Pressure), Some(300.0));
    }
    let ramp = StrokeLayer { width: 2.0, ..pressed(vec![(0.0, 0.0), (1.0, 1.0)]) };
    let out = draw(&pen, &line(0.0, 0.0, 100.0, 0.0), &ramp);
    assert!(near(half_width(&out, 0.0, 1.0), 2.0, 0.1), "{}", half_width(&out, 0.0, 1.0));
    assert!(near(half_width(&out, 99.0, 100.0), 12.0, 0.2), "{}", half_width(&out, 99.0, 100.0));
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
    // The long edges bend as curves (#3926), where they used to be cut into some 400 chords. The
    // path is run along in 0.05 pt chords, which the curves follow, so a few dozen remain.
    let (anchors, curved) = anchors_and_curved(&out[0]);
    assert!(anchors < 100 && curved > anchors / 2, "long edges bend as curves: {curved} of {anchors} anchors with handles");
    let mut pts = vec![];
    all_points(&out[0], &mut pts);
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
fn scatter_spaces_stroked_art_by_its_stroked_width() {
    let art = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(0.0, -2.0, 10.0, 2.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::BLACK), 2.0),
    );
    let b = Brush { name: "s".into(), kind: BrushKind::Scatter(Scatter { art, colorization: Colorization::None, ..Default::default() }) };
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 120.0, 0.0), &stroke(1.0));
    let boxes: Vec<Rect> = out.iter().filter_map(Node::visual_bounds).collect();
    assert!(boxes.len() > 1, "{boxes:?}");
    for w in boxes.windows(2) {
        assert!(near(w[1].x0, w[0].x1, 1e-6), "stroked copies at 100% spacing touch: {boxes:?}");
    }
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

/// A pattern brush whose corner tiles have a 4 × 4 pt frame and, as art, a 2 × 2 pt square in the
/// top-left quarter of the frame. Corner art joins a side tile on its right to one below it, so
/// that quarter is on the outside of the turn. `inner` and `outer` say which kinds of corner have
/// art; `flips` are the brush's Flip Along and Flip Across.
fn marked_corner_brush(inner: bool, outer: bool, (flip_along, flip_across): (bool, bool)) -> Brush {
    let marker =
        Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, -2.0, 2.0, 0.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let mut b = pattern_brush(PatternFit::Stretch, false);
    if let BrushKind::Pattern(p) = &mut b.kind {
        let frame = Some(Rect::new(0.0, -2.0, 4.0, 2.0));
        if inner {
            (p.inner_corner, p.inner_corner_frame) = (Some(marker.clone()), frame);
        }
        if outer {
            (p.outer_corner, p.outer_corner_frame) = (Some(marker), frame);
        }
        (p.flip_along, p.flip_across) = (flip_along, flip_across);
    }
    b
}

/// Each combination of corner art (both kinds, inner only, outer only) and of the two flips.
fn marked_corner_brushes() -> Vec<(String, Brush)> {
    let mut v = vec![];
    for (inner, outer) in [(true, true), (true, false), (false, true)] {
        for flips in [(false, false), (true, false), (false, true), (true, true)] {
            v.push((format!("inner {inner}, outer {outer}, flips {flips:?}"), marked_corner_brush(inner, outer, flips)));
        }
    }
    v
}

/// The bounds of the 2 × 2 pt corner markers among `out`, turned or not.
fn marker_bounds(out: &[Node]) -> Vec<Rect> {
    out.iter().filter_map(Node::geometric_bounds).filter(|r| r.width() < 3.0 && r.height() < 3.0).collect()
}

/// Corner tiles are square to the sides at a right angle and turn 90° with each turn of the path:
/// on a square drawn either way round, each corner's marker fills the quarter of the band's corner
/// outside the turn, with the brush flipped or not and with art for one kind of corner only
/// (#6110).
#[test]
fn pattern_corner_tiles_are_square_to_the_sides() {
    let doc = Document::new(1.0, 1.0);
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let mut reversed = BezPath::new();
    reversed.move_to((0.0, 0.0));
    for (x, y) in [(0.0, 100.0), (100.0, 100.0), (100.0, 0.0)] {
        reversed.line_to((x, y));
    }
    reversed.close_path();
    // The square turns the same way at all four corners, and the reversed square the other way,
    // so one draws only inner corner tiles and the other only outer ones.
    for (name, brush) in marked_corner_brushes() {
        for path in [&sq, &reversed] {
            let out = stroke_pieces(&doc, &brush, path, &stroke(1.0));
            let marks = marker_bounds(&out);
            assert_eq!(marks.len(), 4, "{name}: one marker a corner");
            for (corner, outside) in SQUARE_CORNERS {
                let want = Rect::from_points(corner, outside);
                assert!(
                    marks
                        .iter()
                        .any(|r| near(r.x0, want.x0, 1e-6) && near(r.y0, want.y0, 1e-6) && near(r.x1, want.x1, 1e-6) && near(r.y1, want.y1, 1e-6)),
                    "{name}: the marker at {corner:?} fills {want:?}: {marks:?}"
                );
            }
        }
    }
}

/// At a corner that is not a right angle, a corner tile splits the difference between the two
/// sides: its marker lies on the line that halves the angle, outside the turn, for sharp and
/// shallow turns either way. Near a hairpin the marker moves smoothly towards the line the path
/// arrives on.
#[test]
fn pattern_corner_tiles_sit_on_the_corners_diagonal() {
    let doc = Document::new(1.0, 1.0);
    for (name, brush) in marked_corner_brushes() {
        for turn in [-179.9_f64, -170.0, -135.0, -60.0, -45.0, 45.0, 60.0, 135.0, 170.0, 179.9] {
            let (a, b) = (Vec2::new(1.0, 0.0), Vec2::new(turn.to_radians().cos(), turn.to_radians().sin()));
            let mut bp = BezPath::new();
            bp.move_to((0.0, 0.0));
            bp.line_to((50.0, 0.0));
            bp.line_to(Point::new(50.0, 0.0) + b * 50.0);
            let out = stroke_pieces(&doc, &brush, &bp, &stroke(1.0));
            let marks = marker_bounds(&out);
            assert_eq!(marks.len(), 1, "{name}, {turn}°: one corner marker");
            let away = a - b;
            let want = Point::new(50.0, 0.0) + away / away.hypot() * 2.0_f64.sqrt();
            assert!(marks[0].center().distance(want) < 1e-6, "{name}, {turn}°: the marker is at {:?}, not {want:?}", marks[0].center());
        }
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
fn pattern_without_start_tile_runs_side_to_path_start() {
    let path = line(0.0, 0.0, 100.0, 0.0);
    let doc = Document::new(1.0, 1.0);
    let mut b = pattern_brush(PatternFit::Stretch, false);
    if let BrushKind::Pattern(p) = &mut b.kind {
        p.start = Some(rect_art(20.0, 4.0));
    }
    let with_start = stroke_pieces(&doc, &b, &path, &stroke(1.0));
    assert_eq!(with_start[0].geometric_bounds().unwrap().width(), 20.0);
    if let BrushKind::Pattern(p) = &mut b.kind {
        p.start = None;
    }
    let without_start = stroke_pieces(&doc, &b, &path, &stroke(1.0));
    assert_eq!(without_start.len(), 10);
    assert!(near(without_start[0].geometric_bounds().unwrap().x0, 0.0, 1e-6));
    assert!(near(without_start[0].geometric_bounds().unwrap().width(), 10.0, 1e-6));
}

#[test]
fn pattern_without_end_tile_runs_side_to_path_end() {
    let path = line(0.0, 0.0, 100.0, 0.0);
    let doc = Document::new(1.0, 1.0);
    let mut b = pattern_brush(PatternFit::Stretch, false);
    if let BrushKind::Pattern(p) = &mut b.kind {
        p.end = Some(rect_art(20.0, 4.0));
    }
    let with_end = stroke_pieces(&doc, &b, &path, &stroke(1.0));
    assert!(with_end.iter().any(|n| near(n.geometric_bounds().unwrap().width(), 20.0, 1e-6)));
    if let BrushKind::Pattern(p) = &mut b.kind {
        p.end = None;
    }
    let without_end = stroke_pieces(&doc, &b, &path, &stroke(1.0));
    assert_eq!(without_end.len(), 10);
    assert!(near(without_end.last().unwrap().geometric_bounds().unwrap().x1, 100.0, 1e-6));
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
fn bristle_pen_pressure_changes_its_footprint() {
    let b = Brush { name: "b".into(), kind: BrushKind::Bristle(Bristle::default()) };
    let render = |pressure| {
        let s = StrokeLayer { pressure: Some(vectorcraft_doc::PressureProfile { points: vec![(0.0, pressure), (1.0, pressure)] }), ..stroke(1.0) };
        stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 200.0, 0.0), &s)
    };
    let light = render(0.0);
    let heavy = render(1.0);
    assert_eq!(light.len(), heavy.len());
    assert!(bounds(&heavy).height() > bounds(&light).height() * 1.8);
}

#[test]
fn bristle_pen_pressure_is_sampled_along_the_stroke() {
    let b = Brush { name: "b".into(), kind: BrushKind::Bristle(Bristle::default()) };
    let s = StrokeLayer { pressure: Some(vectorcraft_doc::PressureProfile { points: vec![(0.0, 0.0), (1.0, 1.0)] }), ..stroke(1.0) };
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 200.0, 0.0), &s);
    let mut points = Vec::new();
    for n in &out {
        all_points(n, &mut points);
    }
    let early = points.iter().filter(|p| p.x > 20.0 && p.x < 60.0).map(|p| p.y.abs()).fold(0.0_f64, f64::max);
    let late = points.iter().filter(|p| p.x > 140.0 && p.x < 180.0).map(|p| p.y.abs()).fold(0.0_f64, f64::max);
    assert!(late > early * 1.4, "spread along stroke: {early} -> {late}");
}

#[test]
fn trimmed_bristle_strands_read_pressure_where_they_begin() {
    let b = Brush { name: "b".into(), kind: BrushKind::Bristle(Bristle { size: 20.0, length: 300.0, ..Default::default() }) };
    let render = |points| {
        let s = StrokeLayer { pressure: Some(vectorcraft_doc::PressureProfile { points }), ..stroke(1.0) };
        stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 80.0, 0.0), &s)
    };
    let ramp = render(vec![(0.0, 0.0), (1.0, 1.0)]);
    let neutral = render(vec![(0.0, 0.5), (1.0, 0.5)]);
    assert_eq!(ramp.len(), neutral.len());
    let start = |n: &Node| {
        let anchors = &n.path_data().unwrap().subpaths[0].anchors;
        let first = anchors[0].p;
        let opposite = anchors[anchors.len() - 16].p;
        (first.x, (first.y - opposite.y).abs())
    };
    let mut checked = false;
    for (ramp, neutral) in ramp.iter().zip(&neutral) {
        let (x, width) = start(ramp);
        if x < 12.0 {
            continue;
        }
        let (_, neutral_width) = start(neutral);
        let expected = 0.5 + x / 80.0;
        assert!(width / neutral_width > expected - 0.08, "strand at {x}: {width} vs {neutral_width}, expected ratio {expected}");
        checked = true;
    }
    assert!(checked, "at least one strand begins past the pressure ramp's first eighth");
}

#[test]
fn bristle_pen_orientation_changes_its_footprint() {
    let b = Brush { name: "b".into(), kind: BrushKind::Bristle(Bristle::default()) };
    let render = |tilt, bearing, rotation| {
        let flat = |x| vec![(0.0, x), (1.0, x)];
        let s = StrokeLayer {
            tilt: Some(vectorcraft_doc::PressureProfile { points: flat(tilt) }),
            bearing: Some(vectorcraft_doc::AngleProfile { points: flat(bearing) }),
            rotation: Some(vectorcraft_doc::AngleProfile { points: flat(rotation) }),
            ..stroke(1.0)
        };
        stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 200.0, 0.0), &s)
    };
    let neutral = bounds(&render(0.5, 0.0, 0.0)).height();
    let flat = bounds(&render(1.0, 0.0, 0.0)).height();
    let upright = bounds(&render(0.0, 0.0, 0.0)).height();
    let bearing = bounds(&render(0.5, 90.0, 0.0)).height();
    let rotation = bounds(&render(0.5, 0.0, 90.0)).height();
    assert!(flat > neutral && neutral > upright, "tilt: {flat} > {neutral} > {upright}");
    assert!(bearing < neutral * 0.7, "bearing: {bearing} vs {neutral}");
    assert!(rotation < neutral * 0.7, "rotation: {rotation} vs {neutral}");
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

/// Under Tints, a stroke color linked to a swatch gives tints of that swatch, linked to it: black
/// art the stroke's own tint of it, 60% black ink 60% of that, white none; the colors shown are the
/// same as for the unlinked color, a process or an RGB swatch. Tints and Shades stay unlinked.
#[test]
fn tints_of_a_swatch_stay_linked_to_it() {
    let spot = Color::cmyk(0.0, 0.5, 1.0, 0.0);
    let stroke = Paint::Solid { color: spot.tinted(0.8), swatch: Some("Brand orange".into()), tint: 0.8 };
    let mapped = |art: Color| {
        let mut n = rect_art(1.0, 1.0);
        n.appearance.set_fill(Paint::solid(art));
        colorize(&mut n, Colorization::Tints, &stroke);
        let mut plain = rect_art(1.0, 1.0);
        plain.appearance.set_fill(Paint::solid(art));
        colorize(&mut plain, Colorization::Tints, &Paint::solid(spot.tinted(0.8)));
        (n.appearance.fill_paint(), plain.appearance.fill_paint().color().unwrap())
    };
    for (art, want) in [(Color::cmyk(0.0, 0.0, 0.0, 1.0), 0.8), (Color::cmyk(0.0, 0.0, 0.0, 0.6), 0.48), (Color::cmyk(0.0, 0.0, 0.0, 0.0), 0.0)] {
        let (paint, shown) = mapped(art);
        let Paint::Solid { color, swatch: Some(s), tint } = paint else { panic!("linked: {paint:?}") };
        assert_eq!(s, "Brand orange");
        assert!((tint - want).abs() < 1e-5, "{art:?}: {tint}");
        let close = color.to_cmyk().iter().zip(shown.to_cmyk()).all(|(a, b)| (a - b).abs() < 1e-5);
        assert!(close, "{color:?} {shown:?}");
    }
    // An RGB swatch: its tints mix toward white, as the unlinked color's do.
    let rgb = Color::rgb(1.0, 0.4, 0.0);
    let mut n = rect_art(1.0, 1.0);
    n.appearance.set_fill(Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 0.5)));
    colorize(&mut n, Colorization::Tints, &Paint::Solid { color: rgb, swatch: Some("Brand red".into()), tint: 1.0 });
    let Paint::Solid { color, swatch: Some(s), tint } = n.appearance.fill_paint() else { panic!("linked") };
    assert_eq!(s, "Brand red");
    assert!((tint - 0.5).abs() < 1e-5, "{tint}");
    let [r, g, b] = color.to_rgb();
    assert!((r - 1.0).abs() < 1e-5 && (g - 0.7).abs() < 1e-5 && (b - 0.5).abs() < 1e-5, "{r} {g} {b}");
    // Tints and Shades darken past the swatch, which a tint can't: its pieces stay unlinked.
    let mut n = rect_art(1.0, 1.0);
    n.appearance.set_fill(Paint::solid(Color::cmyk(0.0, 0.0, 0.0, 1.0)));
    colorize(&mut n, Colorization::TintsAndShades, &stroke);
    assert!(matches!(n.appearance.fill_paint(), Paint::Solid { swatch: None, .. }), "{:?}", n.appearance.fill_paint());
}

/// Black ink alone is a tint by its amount: a process black stroke leaves it as it is, and another
/// process stroke color takes that share of its inks. Tints of a process color stay process.
#[test]
fn tints_of_black_ink_are_its_amount_of_the_stroke_color() {
    let tinted = |art: Color, stroke: Color| {
        let mut n = rect_art(1.0, 1.0);
        n.appearance.set_fill(Paint::solid(art));
        colorize(&mut n, Colorization::Tints, &Paint::solid(stroke));
        n.appearance.fill_paint().color().unwrap()
    };
    let ink = Color::cmyk(0.0, 0.0, 0.0, 0.6);
    let close = |a: Color, b: Color| {
        let (Color::Cmyk { c, m, y, k }, Color::Cmyk { c: c2, m: m2, y: y2, k: k2 }) = (a, b) else { panic!("{a:?} {b:?}") };
        (c - c2).abs() < 1e-5 && (m - m2).abs() < 1e-5 && (y - y2).abs() < 1e-5 && (k - k2).abs() < 1e-5
    };
    assert!(close(tinted(ink, Color::cmyk(0.0, 0.0, 0.0, 1.0)), ink));
    assert!(close(tinted(ink, Color::cmyk(0.0, 1.0, 1.0, 0.0)), Color::cmyk(0.0, 0.6, 0.6, 0.0)));
    let [r, g, b] = tinted(ink, Color::rgb(1.0, 0.0, 0.0)).to_rgb();
    assert!((r - 1.0).abs() < 1e-5 && (g - 0.4).abs() < 1e-5 && (b - 0.4).abs() < 1e-5, "{r} {g} {b}");
    let Color::Gray { k } = tinted(ink, Color::gray(0.5)) else { panic!("tints of a gray stroke color stay gray") };
    assert!((k - 0.3).abs() < 1e-5, "{k}");
}

/// Under Tints and Shades black ink stays black, however light 100% K displays, and a band of black
/// ink is its amount: K 0.5 is the stroke color. A process or gray stroke color stays in its model.
#[test]
fn tints_and_shades_of_black_ink_go_by_its_amount() {
    let mapped = |k: f32, stroke: Color| {
        let mut n = rect_art(1.0, 1.0);
        n.appearance.set_fill(Paint::solid(Color::cmyk(0.0, 0.0, 0.0, k)));
        colorize(&mut n, Colorization::TintsAndShades, &Paint::solid(stroke));
        n.appearance.fill_paint().color().unwrap()
    };
    let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5);
    let inks = |c: Color| {
        let Color::Cmyk { c, m, y, k } = c else { panic!("a process color: {c:?}") };
        [c, m, y, k]
    };
    let orange = Color::cmyk(0.0, 0.6, 1.0, 0.0);
    assert!(close(inks(mapped(1.0, orange)), [0.0, 0.0, 0.0, 1.0]), "black stays black");
    assert!(close(inks(mapped(0.5, orange)), [0.0, 0.6, 1.0, 0.0]), "the stroke color");
    assert!(close(inks(mapped(0.25, orange)), [0.0, 0.3, 0.5, 0.0]), "a tint");
    assert!(close(inks(mapped(0.75, orange)), [0.0, 0.3, 0.5, 0.5]), "a shade");
    let Color::Gray { k } = mapped(0.75, Color::gray(0.4)) else { panic!("a gray stroke color stays gray") };
    assert!((k - 0.7).abs() < 1e-5, "{k}");
    let [r, g, b] = mapped(1.0, Color::rgb(1.0, 0.0, 0.0)).to_rgb();
    assert!(r < 1e-5 && g < 1e-5 && b < 1e-5, "black stays black: {r} {g} {b}");
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

/// A scatter brush of rectangles filled `fill`, with `colorization` and `hue_jitter`.
fn jittered(fill: Color, colorization: Colorization, hue_jitter: f64) -> Brush {
    let mut art = rect_art(10.0, 4.0);
    art.appearance.set_fill(Paint::solid(fill));
    Brush { name: "s".into(), kind: BrushKind::Scatter(Scatter { art, colorization, hue_jitter, ..Default::default() }) }
}

/// The fill color of each copy.
fn fill_colors(out: &[Node]) -> Vec<Color> {
    out.iter().map(|n| n.appearance.fill_paint().color().unwrap()).collect()
}

/// How far hue `h` is from `from`, in degrees, either way round.
fn hue_gap(h: f32, from: f32) -> f32 {
    let d = (h - from).rem_euclid(360.0);
    d.min(360.0 - d)
}

#[test]
fn hue_jitter_turns_each_copy_the_same_every_time() {
    let doc = Document::new(1.0, 1.0);
    let path = line(0.0, 0.0, 200.0, 0.0);
    let b = jittered(Color::rgb(1.0, 0.0, 0.0), Colorization::None, 90.0);
    let out = stroke_pieces(&doc, &b, &path, &stroke(1.0));
    assert_eq!(out.len(), 20);
    assert_eq!(stroke_pieces(&doc, &b, &path, &stroke(1.0)), out, "the same stroke draws the same colors");
    let colors = fill_colors(&out);
    let hues: Vec<f32> = colors.iter().map(|c| c.to_hsb()[0]).collect();
    assert!(hues.iter().all(|h| hue_gap(*h, 0.0) <= 90.01), "{hues:?}");
    assert!(hues.iter().any(|h| hue_gap(*h, 0.0) > 10.0), "the copies turn: {hues:?}");
    let distinct: std::collections::HashSet<String> = colors.iter().map(Color::to_hex).collect();
    assert!(distinct.len() > 10, "each copy turns by its own amount: {distinct:?}");
    // Saturation and brightness stay; the copies stay where they were, at their size and angle.
    assert!(colors.iter().all(|c| near(f64::from(c.to_hsb()[1]), 1.0, 1e-3) && near(f64::from(c.to_hsb()[2]), 1.0, 1e-3)), "{colors:?}");
    let plain = stroke_pieces(&doc, &jittered(Color::rgb(1.0, 0.0, 0.0), Colorization::None, 0.0), &path, &stroke(1.0));
    let boxes = |o: &[Node]| o.iter().map(|n| n.geometric_bounds().unwrap()).collect::<Vec<_>>();
    assert_eq!(boxes(&out), boxes(&plain));
    // Another path draws other amounts.
    let other = fill_colors(&stroke_pieces(&doc, &b, &line(0.0, 10.0, 200.0, 10.0), &stroke(1.0)));
    assert_ne!(other, colors);
}

#[test]
fn hue_jitter_of_zero_changes_nothing() {
    let red = Color::rgb(1.0, 0.0, 0.0);
    let out = stroke_pieces(&Document::new(1.0, 1.0), &jittered(red, Colorization::None, 0.0), &line(0.0, 0.0, 200.0, 0.0), &stroke(1.0));
    assert!(!out.is_empty() && fill_colors(&out).iter().all(|c| *c == red));
    // A brush without hue jitter is saved as before, and one saved before reads as 0.
    let v = serde_json::to_value(jittered(red, Colorization::None, 0.0)).unwrap();
    assert!(v.get("hue_jitter").is_none(), "{v}");
    let mut old = serde_json::to_value(jittered(red, Colorization::None, 45.0)).unwrap();
    assert_eq!(old.as_object_mut().unwrap().remove("hue_jitter"), Some(serde_json::json!(45.0)));
    let BrushKind::Scatter(s) = Brush::from_json(old).unwrap().kind else { panic!("not a scatter brush") };
    assert_eq!(s.hue_jitter, 0.0, "a brush saved without the key reads as 0");
    let mut with = v.clone();
    with["hue_jitter"] = serde_json::json!(45.0);
    let BrushKind::Scatter(s) = Brush::from_json(with).unwrap().kind else { panic!("not a scatter brush") };
    assert_eq!(s.hue_jitter, 45.0);
    // Values outside 0..180 are clamped.
    for (given, kept) in [(500.0, 180.0), (-20.0, 0.0)] {
        let mut v = v.clone();
        v["hue_jitter"] = serde_json::json!(given);
        let BrushKind::Scatter(s) = Brush::from_json(v).unwrap().kind else { panic!("not a scatter brush") };
        assert_eq!(s.hue_jitter, kept, "{given}");
    }
}

#[test]
fn hue_jitter_turns_after_the_colorization() {
    let doc = Document::new(1.0, 1.0);
    let path = line(0.0, 0.0, 200.0, 0.0);
    // Tints: black art takes the blue stroke color, which then turns.
    let blue = StrokeLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 1.0);
    let hues: Vec<f32> =
        fill_colors(&stroke_pieces(&doc, &jittered(Color::BLACK, Colorization::Tints, 30.0), &path, &blue)).iter().map(|c| c.to_hsb()[0]).collect();
    assert!(!hues.is_empty() && hues.iter().all(|h| hue_gap(*h, 240.0) <= 30.01), "{hues:?}");
    assert!(hues.iter().any(|h| hue_gap(*h, 240.0) > 3.0), "{hues:?}");
    // A gray stroke color has no hue: its tints stay as they are.
    let gray = StrokeLayer::new(Paint::solid(Color::rgb(0.5, 0.5, 0.5)), 1.0);
    let turned = stroke_pieces(&doc, &jittered(Color::BLACK, Colorization::Tints, 180.0), &path, &gray);
    let still = stroke_pieces(&doc, &jittered(Color::BLACK, Colorization::Tints, 0.0), &path, &gray);
    assert_eq!(fill_colors(&turned), fill_colors(&still));
    // Nor does black ink alone, nor a gray, as a process stroke color.
    for key in [Color::cmyk(0.0, 0.0, 0.0, 1.0), Color::cmyk(0.0, 0.0, 0.0, 0.5), Color::gray(0.4)] {
        let key = StrokeLayer::new(Paint::solid(key), 1.0);
        let turned = stroke_pieces(&doc, &jittered(Color::BLACK, Colorization::Tints, 180.0), &path, &key);
        let still = stroke_pieces(&doc, &jittered(Color::BLACK, Colorization::Tints, 0.0), &path, &key);
        assert_eq!(fill_colors(&turned), fill_colors(&still), "{key:?}");
    }
    // A turned process color stays a process color, turned over its C, M and Y with its K kept.
    let cmyk = fill_colors(&stroke_pieces(&doc, &jittered(Color::cmyk(0.0, 1.0, 1.0, 0.25), Colorization::None, 60.0), &path, &stroke(1.0)));
    assert!(cmyk.iter().any(|c| *c != Color::cmyk(0.0, 1.0, 1.0, 0.25)), "{cmyk:?}");
    for c in &cmyk {
        let Color::Cmyk { c, m, y, k } = *c else { panic!("not CMYK: {c:?}") };
        assert_eq!(k, 0.25);
        // Full ink in one channel and none in another, as the color before it.
        assert!(near(f64::from(c.max(m).max(y)), 1.0, 1e-4) && near(f64::from(c.min(m).min(y)), 0.0, 1e-4), "{c} {m} {y}");
    }
    // An RGB color keeps its saturation and brightness in its own values.
    let rgb = fill_colors(&stroke_pieces(&doc, &jittered(Color::rgb(0.8, 0.2, 0.4), Colorization::None, 90.0), &path, &stroke(1.0)));
    for c in &rgb {
        let Color::Rgb { r, g, b } = *c else { panic!("not RGB: {c:?}") };
        assert!(near(f64::from(r.max(g).max(b)), 0.8, 1e-4) && near(f64::from(r.min(g).min(b)), 0.2, 1e-4), "{c:?}");
    }
    // A Lab color keeps its lightness and chroma.
    let lab = fill_colors(&stroke_pieces(&doc, &jittered(Color::lab(50.0, 40.0, 30.0), Colorization::None, 90.0), &path, &stroke(1.0)));
    assert!(lab.iter().any(|c| *c != Color::lab(50.0, 40.0, 30.0)), "{lab:?}");
    for c in &lab {
        let Color::Lab { l, a, b } = *c else { panic!("not Lab: {c:?}") };
        assert!(l == 50.0 && near(f64::from(a.hypot(b)), 50.0, 1e-3), "{c:?}");
    }
}

/// Hue jitter turns type's characters, the colors Tints and Shades and Hue Shift give, and a color
/// linked to a swatch (which loses the link); a pattern paint stays.
#[test]
fn hue_jitter_turns_type_every_colorization_and_linked_colors() {
    let doc = Document::new(1.0, 1.0);
    let path = line(0.0, 0.0, 200.0, 0.0);
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let typed = |hue_jitter| Brush {
        name: "s".into(),
        kind: BrushKind::Scatter(Scatter { art: type_art(red.clone(), red.clone()), hue_jitter, ..Default::default() }),
    };
    let still = run_paints(&stroke_pieces(&doc, &typed(0.0), &path, &stroke(1.0)));
    let turned = run_paints(&stroke_pieces(&doc, &typed(90.0), &path, &stroke(1.0)));
    assert!(!still.is_empty() && still.iter().all(|p| *p == (red.clone(), red.clone())), "{still:?}");
    assert_eq!(turned.len(), still.len());
    assert!(turned.iter().any(|p| *p != (red.clone(), red.clone())), "the characters turn: {turned:?}");
    assert!(turned.iter().all(|(f, s)| f == s), "a copy's character fill and stroke turn by the same amount: {turned:?}");
    // Tints and Shades and Hue Shift: the colors they give turn.
    let blue = StrokeLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 1.0);
    for colorization in [Colorization::TintsAndShades, Colorization::HueShift { key: Color::rgb(1.0, 0.0, 0.0) }] {
        let art = Color::rgb(1.0, 0.0, 0.0);
        let still = fill_colors(&stroke_pieces(&doc, &jittered(art, colorization, 0.0), &path, &blue));
        let turned = fill_colors(&stroke_pieces(&doc, &jittered(art, colorization, 45.0), &path, &blue));
        assert_eq!(turned.len(), still.len());
        assert_ne!(turned, still, "{colorization:?}");
        for (t, s) in turned.iter().zip(&still) {
            assert!(hue_gap(t.to_hsb()[0], s.to_hsb()[0]) <= 45.01, "{colorization:?}: {t:?} from {s:?}");
        }
    }
    // A color linked to a swatch turns and loses the link; a pattern paint stays.
    let linked = Paint::Solid { color: Color::rgb(1.0, 0.0, 0.0), swatch: Some("Red".into()), tint: 1.0 };
    let pattern = Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY };
    let mut art = rect_art(10.0, 4.0);
    art.appearance.set_fill(linked.clone());
    art.appearance.set_stroke(pattern.clone());
    let b = |hue_jitter| Brush { name: "s".into(), kind: BrushKind::Scatter(Scatter { art: art.clone(), hue_jitter, ..Default::default() }) };
    let still = stroke_pieces(&doc, &b(0.0), &path, &stroke(1.0));
    let turned = stroke_pieces(&doc, &b(90.0), &path, &stroke(1.0));
    assert!(!still.is_empty() && still.iter().all(|n| n.appearance.fill_paint() == linked));
    assert_eq!(turned.len(), still.len());
    for n in &turned {
        assert!(matches!(n.appearance.fill_paint(), Paint::Solid { swatch: None, .. }), "{:?}", n.appearance.fill_paint());
        assert_eq!(n.appearance.stroke().map(|s| &s.paint), Some(&pattern));
    }
}

/// An instance whose fill has an opacity of 0 does not stain its symbol's art, so the art keeps its
/// swatch links.
#[test]
fn a_fill_with_no_opacity_leaves_the_symbol_art_linked() {
    let spot = Paint::Solid { color: Color::cmyk(1.0, 0.7, 0.0, 0.4), swatch: Some("Spot".into()), tint: 1.0 };
    let art = Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(spot.clone(), Paint::None, 0.0));
    let mut inst = Node::new(NodeId(2), NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Affine::IDENTITY });
    inst.appearance = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0).into();
    if let Some(f) = inst.appearance.fill_mut() {
        f.opacity = 0.0;
    }
    assert_eq!(stain(&inst), None);
    assert_eq!(instance_art(&art, &inst).appearance.fill_paint(), spot);
    // Any opacity above 0 still stains.
    if let Some(f) = inst.appearance.fill_mut() {
        f.opacity = 0.01;
    }
    assert_ne!(instance_art(&art, &inst).appearance.fill_paint(), spot);
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

/// A stored library that repeats a name reads with the first brush of that name as stored and each
/// later one numbered, past every name the library already has; strokes of the name keep drawing
/// the first brush.
#[test]
fn repeated_brush_names_are_numbered_when_read() {
    let mut d = Document::new(100.0, 100.0);
    let named = |name: &str, size: f64| Brush { name: name.into(), ..calli(0.0, 100.0, size) };
    store(&mut d, &[named("Nib", 1.0), named("Nib 2", 2.0), named("Nib", 3.0), named("Other", 4.0), named("Nib", 5.0), named("Nib 2", 6.0)]);
    let lib = library(&d);
    let names: Vec<&str> = lib.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Nib", "Nib 2", "Nib 3", "Other", "Nib 4", "Nib 2 2"]);
    let size = |b: &Brush| if let BrushKind::Calligraphic(c) = &b.kind { c.size } else { 0.0 };
    assert_eq!(lib.iter().map(size).collect::<Vec<_>>(), [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_eq!(find(&d, "Nib").map(|b| size(&b)), Some(1.0));
    assert_eq!(find(&d, "Nib 3").map(|b| size(&b)), Some(3.0));
    // Reading again gives the same names, and the numbered library stored again reads as stored.
    assert_eq!(library(&d), lib);
    store(&mut d, &lib);
    assert_eq!(library(&d), lib);
    // Many brushes of one name take the numbers in order, past the numbered names the file has.
    let many: Vec<Brush> = (0..5000).map(|i| named(if i == 1 { "Nib 3" } else { "Nib" }, 1.0)).collect();
    store(&mut d, &many);
    let names: Vec<String> = library(&d).into_iter().map(|b| b.name).collect();
    assert_eq!(names[..5], ["Nib", "Nib 3", "Nib 2", "Nib 4", "Nib 5"]);
    assert_eq!(names.last().map(String::as_str), Some("Nib 5000"));
    assert_eq!(names.iter().collect::<std::collections::HashSet<_>>().len(), names.len());
}

/// Art of `w`×`h` points holding a linked image: a group of [`rect_art`] and the image, both with id 0.
fn linked_art(w: f64, h: f64) -> Node {
    let image = Node::new(
        NodeId(0),
        NodeKind::Image(Box::new(vectorcraft_doc::ImageObject {
            key: "k".into(),
            width: 4,
            height: 4,
            xf: Affine::IDENTITY,
            link: Some(vectorcraft_doc::LinkInfo::new("/art/a.png")),
            placement: Default::default(),
            pixels: None,
        })),
    );
    Node::group(NodeId(0), vec![std::sync::Arc::new(rect_art(w, h)), std::sync::Arc::new(image)])
}

/// The art of the stored brushes that holds images mounts on the document as stored art, each tile
/// with the brush, its place in the library and the tile as owner, its objects with ids of their
/// own; writing it back changes only the entries whose art changed, and leaves an entry that does
/// not read as a brush, a brush without art and a brush whose art holds no image as they were stored.
#[test]
fn stored_brush_art_mounts_and_writes_back_only_what_changed() {
    use stored_art::StoredArt;
    let mut d = Document::new(100.0, 100.0);
    let layer_object = d.layers[0].id;
    let mut side = linked_art(10.0, 4.0);
    side.id = layer_object;
    let pattern =
        Brush { name: "Frame".into(), kind: BrushKind::Pattern(PatternBrush { side, start: Some(linked_art(4.0, 4.0)), ..Default::default() }) };
    let scatter = Brush {
        name: "Dots".into(),
        kind: BrushKind::Scatter(Scatter { art: linked_art(2.0, 2.0), colorization: Colorization::None, ..Default::default() }),
    };
    // Written as a file holds it, without the ids `store` gives.
    let lib = [calli(0.0, 100.0, 3.0), pattern, scatter, art_brush(rect_art(10.0, 4.0), ArtScale::Stretch)];
    d.unknown.insert(DOC_KEY.into(), serde_json::to_value(lib).unwrap());
    let odd = serde_json::json!({ "name": "Odd", "type": "unknown" });
    if let Some(list) = d.unknown.get_mut(DOC_KEY).and_then(serde_json::Value::as_array_mut) {
        list.insert(1, odd.clone());
    }
    let stored = d.unknown[DOC_KEY].clone();
    let art = StoredArt::read(&d);
    art.mount(&mut d);
    let owners: Vec<&serde_json::Value> = d.mounted_art.iter().map(|(_, o)| o).collect();
    assert_eq!(
        owners,
        [
            &serde_json::json!({ "kind": "brush", "name": "Frame", "index": 1, "tile": "side" }),
            &serde_json::json!({ "kind": "brush", "name": "Frame", "index": 1, "tile": "start" }),
            &serde_json::json!({ "kind": "brush", "name": "Dots", "index": 2 }),
        ]
    );
    let ids: Vec<NodeId> = d.mounted_art.iter().map(|(n, _)| n.id).collect();
    let mut all = vec![];
    d.visit_stored(|n, _| all.push(n.id));
    assert!(all.iter().all(|id| id.0 != 0) && !all[1..].contains(&layer_object), "{all:?}");
    assert_eq!(all.iter().collect::<std::collections::HashSet<_>>().len(), all.len(), "{all:?}");
    // The scatter brush's art changes; the pattern brush's tiles keep the ids mounting gave them.
    d.stored_node_mut(ids[2]).unwrap().name = Some("changed".into());
    art.write_back(&mut d);
    assert!(d.mounted_art.is_empty());
    let now = d.unknown[DOC_KEY].as_array().unwrap();
    assert_eq!(now.len(), 5);
    assert_eq!((&now[0], &now[1], &now[4]), (&stored[0], &odd, &stored[4]), "the entries without images stay as stored");
    assert_ne!(now[2], stored[2], "the pattern brush keeps the ids its art was given");
    let lib = library(&d);
    let Some(BrushKind::Pattern(p)) = lib.iter().find(|b| b.name == "Frame").map(|b| &b.kind) else { panic!("no pattern brush") };
    assert_eq!((p.side.id, p.start.as_ref().map(|n| n.id)), (ids[0], Some(ids[1])));
    assert_eq!(find(&d, "Dots").unwrap().art()[0].name.as_deref(), Some("changed"));
    // Mounting again gives the same ids and changes nothing, so writing back leaves the library as it is.
    let again = d.unknown[DOC_KEY].clone();
    let art = StoredArt::read(&d);
    art.mount(&mut d);
    assert_eq!(d.mounted_art.iter().map(|(n, _)| n.id).collect::<Vec<_>>(), ids);
    art.write_back(&mut d);
    assert_eq!(d.unknown[DOC_KEY], again);
}

/// A brush whose stored name repeats an earlier brush's is named in its art's owner as the Brushes
/// panel lists it, numbered.
#[test]
fn stored_brush_art_owner_names_a_repeated_name_as_listed() {
    use stored_art::StoredArt;
    let mut d = Document::new(100.0, 100.0);
    let scatter = Brush { name: "Dots".into(), kind: BrushKind::Scatter(Scatter { art: linked_art(2.0, 2.0), ..Default::default() }) };
    store(&mut d, &[scatter.clone(), scatter]);
    StoredArt::read(&d).mount(&mut d);
    let names: Vec<&serde_json::Value> = d.mounted_art.iter().map(|(_, o)| &o["name"]).collect();
    assert_eq!(names, ["Dots", "Dots 2"]);
}

/// Storing a library whose brush art holds an image gives that art ids of its own, so mounting it
/// later gives the same ids whatever the document allocated in between; art without images keeps
/// the ids it was stored with.
#[test]
fn storing_brush_art_with_images_keeps_ids_of_its_own() {
    use stored_art::StoredArt;
    let mut d = Document::new(100.0, 100.0);
    let mut art = linked_art(2.0, 2.0);
    art.id = d.layers[0].id;
    let scatter = Brush { name: "Dots".into(), kind: BrushKind::Scatter(Scatter { art, ..Default::default() }) };
    store(&mut d, &[scatter, art_brush(rect_art(10.0, 4.0), ArtScale::Stretch)]);
    let lib = library(&d);
    let stored = lib[0].art()[0].id;
    assert_ne!(stored, d.layers[0].id);
    assert_eq!(lib[1].art()[0].id, NodeId(0), "art without images is stored as it was");
    d.alloc_id();
    StoredArt::read(&d).mount(&mut d);
    assert_eq!(d.mounted_art[0].0.id, stored);
}

/// When the library is written again while the art is mounted, numbering a repeated name and
/// adding a brush (as placing a document's brushes into it does), writing back changes the entry
/// at its own place that reads as the brush apart from its new name, keeping that name, even when
/// an earlier brush is the same as the one it was, and gives the added brush's art ids of its own.
#[test]
fn brush_art_written_back_finds_its_renamed_entry_and_numbers_added_art() {
    use stored_art::StoredArt;
    /// `n` with ids from `next` on, in paint order.
    fn numbered(mut n: Node, next: &mut u64) -> Node {
        n.id = NodeId(*next);
        *next += 1;
        for c in n.children_mut().into_iter().flatten() {
            *c = std::sync::Arc::new(numbered((**c).clone(), next));
        }
        n
    }
    let mut d = Document::new(100.0, 100.0);
    let scatter =
        |n: &str| Brush { name: n.into(), kind: BrushKind::Scatter(Scatter { art: numbered(linked_art(2.0, 2.0), &mut 50), ..Default::default() }) };
    // Two brushes the same apart from the ids mounting gives the second.
    d.unknown.insert(DOC_KEY.into(), serde_json::to_value([scatter("Dots"), scatter("Dots")]).unwrap());
    let art = StoredArt::read(&d);
    art.mount(&mut d);
    assert_eq!(d.mounted_art[0].0.id, NodeId(50));
    let second = d.mounted_art[1].0.id;
    // The library is written again: the repeated name numbered, a brush whose art repeats an id added.
    let mut added = scatter("New");
    if let BrushKind::Scatter(s) = &mut added.kind {
        s.art.id = d.layers[0].id;
    }
    let lib = [library(&d), vec![added]].concat();
    d.unknown.insert(DOC_KEY.into(), serde_json::to_value(&lib).unwrap());
    assert_eq!(lib[1].name, "Dots 2");
    d.stored_node_mut(second).unwrap().name = Some("changed".into());
    art.write_back(&mut d);
    let lib = library(&d);
    assert_eq!(lib.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Dots", "Dots 2", "New"]);
    assert_eq!((lib[0].art()[0].name.as_deref(), lib[1].art()[0].name.as_deref()), (None, Some("changed")));
    assert_ne!(lib[2].art()[0].id, d.layers[0].id, "the added brush's art has ids of its own");
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

/// A nib that follows the pen is sampled between the vertices of the flattened path too. Along a
/// curve those samples sit on the curve, and the outline keeps its distance from it.
#[test]
fn pen_driven_nib_outline_follows_the_curve_between_vertices() {
    // The top half of a circle of radius 50; a round nib 20 pt across, the same at any pressure.
    let (r, half) = (50.0, 10.0);
    let c = Point::new(r, r);
    let ring = shapes::ellipse(Rect::new(0.0, 0.0, 2.0 * r, 2.0 * r)).to_bezpath();
    let half_circle: BezPath = ring.elements().iter().take(3).copied().collect();
    let out = stroke_pieces(&Document::new(1.0, 1.0), &pressure_calli(20.0, 0.0), &half_circle, &pressed(vec![(0.0, 0.5), (1.0, 0.5)]));
    let mut pts = vec![];
    for n in &out {
        all_points(n, &mut pts);
    }
    // Away from the round caps at the two ends, every outline point is `half` from the curve.
    let middle = pts.iter().filter(|p| ((p.y - c.y).atan2(p.x - c.x).abs() - std::f64::consts::FRAC_PI_2).abs() <= 1.2);
    let worst = middle.map(|p| ((p.distance(c) - (r + half)).abs()).min((p.distance(c) - (r - half)).abs())).fold(0.0, f64::max);
    assert!(worst < 0.05, "an outline point strays {worst:.4} pt from the nib's reach");
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
    // The angle turns on past 180°: 170 + 40 is 210°, the nib at 30° (not stopped at 180°).
    let turned = Calligraphic { angle: 170.0, roundness: 10.0, variation: [40.0, 0.0, 0.0], ..c([Pressure, Fixed, Fixed]) };
    let expected = 2.0 * (25.0 * 0.25 + 0.25 * 0.75f64).sqrt();
    let h = draw(turned, 1.0);
    assert!(near(h, expected, 0.05), "{h} vs {expected}");
}

/// A stroke that recorded an airbrush pen's finger wheel going from `from` to `to` along it.
fn wheeled(from: f64, to: f64) -> StrokeLayer {
    StrokeLayer { wheel: Some(vectorcraft_doc::PressureProfile { points: vec![(0.0, from), (1.0, to)] }), ..stroke(1.0) }
}

/// Stylus Wheel: a Calligraphic brush's size goes from size − variation at one end of the finger
/// wheel to size + variation at the other, a Scatter brush's size from its first value to its
/// second, and an Art brush's width from Width to the second width. Each reads only the wheel: the
/// pressure, tilt, bearing or rotation recorded instead leaves the value half way.
#[test]
fn the_stylus_wheel_varies_sizes_and_widths() {
    let doc = Document::new(1.0, 1.0);
    let bp = line(0.0, 0.0, 100.0, 0.0);
    let c = Calligraphic {
        size: 6.0,
        variation: [0.0, 0.0, 4.0],
        modes: Some([Variation::Fixed, Variation::Fixed, Variation::StylusWheel]),
        ..Default::default()
    };
    let nib = Brush { name: "w".into(), kind: BrushKind::Calligraphic(c) };
    let out = stroke_pieces(&doc, &nib, &bp, &wheeled(0.0, 1.0));
    // Round nib: 2 pt across at the start, 10 pt at the end, 6 pt half way.
    assert!(near(half_width(&out, 0.5, 1.5), 1.0, 0.1), "{}", half_width(&out, 0.5, 1.5));
    assert!(near(half_width(&out, 98.5, 99.5), 5.0, 0.1), "{}", half_width(&out, 98.5, 99.5));
    assert!(near(half_width(&out, 49.5, 50.5), 3.0, 0.1), "{}", half_width(&out, 49.5, 50.5));
    let others = [
        pressed(vec![(0.0, 0.0), (1.0, 1.0)]),
        StrokeLayer { tilt: Some(vectorcraft_doc::PressureProfile { points: vec![(0.0, 0.0), (1.0, 1.0)] }), ..stroke(1.0) },
        posed(None, Some(90.0), Some(-90.0)),
    ];
    for st in &others {
        let h = bounds(&stroke_pieces(&doc, &nib, &bp, st)).height();
        assert!(near(h, 6.0, 0.05), "{st:?}: {h}");
    }
    // Scatter size 50 % to 300 % of 4 pt art: 2 pt at one end of the wheel, 12 pt at the other.
    let spray = sized_scatter(50.0, 300.0, Variation::StylusWheel);
    let h: Vec<f64> =
        stroke_pieces(&doc, &spray, &line(0.0, 0.0, 400.0, 0.0), &wheeled(1.0, 0.0)).iter().map(|n| n.geometric_bounds().unwrap().height()).collect();
    assert!(near(h[0], 12.0, 0.2) && h[h.len() - 1] < 3.0, "{h:?}");
    // Art width 50 % to 300 % of 4 pt art.
    let mut ribbon = art_brush(rect_art(10.0, 4.0), ArtScale::Stretch);
    if let BrushKind::Art(a) = &mut ribbon.kind {
        (a.width, a.width_mode, a.width_max) = (50.0, Some(Variation::StylusWheel), Some(300.0));
    }
    let out = stroke_pieces(&doc, &ribbon, &bp, &wheeled(1.0, 0.0));
    assert!(near(half_width(&out, 0.0, 1.0), 6.0, 0.1), "{}", half_width(&out, 0.0, 1.0));
    assert!(near(half_width(&out, 99.0, 100.0), 1.0, 0.1), "{}", half_width(&out, 99.0, 100.0));
    for st in &others {
        let h = bounds(&stroke_pieces(&doc, &ribbon, &bp, st)).height();
        assert!(near(h, 7.0, 1e-6), "{st:?}: {h}");
    }
    // A definition keeps the mode by name.
    let lib = parse_library(&serde_json::json!([
        {"name": "a", "type": "art", "art": rect_art(10.0, 4.0), "width_mode": "stylusWheel", "width_max": 300.0},
    ]));
    assert!(matches!(&lib[0].kind, BrushKind::Art(a) if a.width_mode == Some(Variation::StylusWheel)));
}

/// A stroke that recorded the pen's tilt (`tilt`: 0 upright to 1 flat), bearing and barrel
/// rotation (degrees counter-clockwise), each the same all along it where given.
fn posed(tilt: Option<f64>, bearing: Option<f64>, rotation: Option<f64>) -> StrokeLayer {
    let flat = |v: f64| vec![(0.0, v), (1.0, v)];
    StrokeLayer {
        tilt: tilt.map(|t| vectorcraft_doc::PressureProfile { points: flat(t) }),
        bearing: bearing.map(|b| vectorcraft_doc::AngleProfile { points: flat(b) }),
        rotation: rotation.map(|r| vectorcraft_doc::AngleProfile { points: flat(r) }),
        ..stroke(1.0)
    }
}

/// #764: with Roundness on Tilt the nib goes from roundness − variation with the pen lying flat to
/// roundness + variation with it upright, along the stroke as the pen leaned.
#[test]
fn tilt_maps_to_roundness_along_the_stroke() {
    let c = Calligraphic {
        roundness: 50.0,
        size: 10.0,
        variation: [0.0, 50.0, 0.0],
        modes: Some([Variation::Fixed, Variation::Tilt, Variation::Fixed]),
        ..Default::default()
    };
    let b = Brush { name: "t".into(), kind: BrushKind::Calligraphic(c) };
    let upright_to_flat = StrokeLayer { tilt: Some(vectorcraft_doc::PressureProfile { points: vec![(0.0, 0.0), (1.0, 1.0)] }), ..stroke(1.0) };
    let out = stroke_pieces(&Document::new(1.0, 1.0), &b, &line(0.0, 0.0, 100.0, 0.0), &upright_to_flat);
    // Upright: round (half 5); half way: roundness 50 (half 2.5); flat: a hairline.
    assert!(near(half_width(&out, 0.5, 1.5), 5.0, 0.1), "{}", half_width(&out, 0.5, 1.5));
    assert!(near(half_width(&out, 49.5, 50.5), 2.5, 0.1), "{}", half_width(&out, 49.5, 50.5));
    assert!(half_width(&out, 98.5, 99.5) < 0.2, "{}", half_width(&out, 98.5, 99.5));
}

/// #764: with Angle on Bearing or on Rotation and a variation of 180°, the nib turns with the way
/// the pen leans or with its barrel.
#[test]
fn bearing_and_rotation_turn_the_nib() {
    let horizontal = line(0.0, 0.0, 100.0, 0.0);
    let height = |mode: Variation, st: &StrokeLayer| {
        let c = Calligraphic {
            roundness: 10.0,
            size: 10.0,
            variation: [180.0, 0.0, 0.0],
            modes: Some([mode, Variation::Fixed, Variation::Fixed]),
            ..Default::default()
        };
        let b = Brush { name: "b".into(), kind: BrushKind::Calligraphic(c) };
        bounds(&stroke_pieces(&Document::new(1.0, 1.0), &b, &horizontal, st)).height()
    };
    // Leaning right (0°): the nib lies along the stroke (1 pt across); leaning up the page (90°)
    // or down it (−90°), the nib stands across it (10 pt).
    assert!(near(height(Variation::Bearing, &posed(Some(0.5), Some(0.0), None)), 1.0, 0.05));
    assert!(near(height(Variation::Bearing, &posed(Some(0.5), Some(90.0), None)), 10.0, 0.05));
    assert!(near(height(Variation::Bearing, &posed(Some(0.5), Some(-90.0), None)), 10.0, 0.05));
    // The barrel turned a quarter turn stands it across the stroke too.
    assert!(near(height(Variation::Rotation, &posed(None, None, Some(0.0))), 1.0, 0.05));
    assert!(near(height(Variation::Rotation, &posed(None, None, Some(90.0))), 10.0, 0.05));
    // Each reads only its own recording.
    assert!(near(height(Variation::Rotation, &posed(None, Some(90.0), None)), 1.0, 0.05));
    assert!(near(height(Variation::Bearing, &posed(None, None, Some(90.0))), 1.0, 0.05));
}

/// A stroke without the pen's tilt, bearing or rotation (a mouse, a pen without that input, or a
/// path the brush was applied to) draws Tilt, Bearing and Rotation values at the brush's own value.
#[test]
fn pen_modes_without_a_recording_draw_the_value_itself() {
    let horizontal = line(0.0, 0.0, 100.0, 0.0);
    for mode in [Variation::Tilt, Variation::Bearing, Variation::Rotation, Variation::StylusWheel] {
        let c = Calligraphic { angle: 90.0, roundness: 50.0, size: 10.0, variation: [90.0, 50.0, 6.0], modes: Some([mode; 3]) };
        let b = Brush { name: "n".into(), kind: BrushKind::Calligraphic(c) };
        let h = bounds(&stroke_pieces(&Document::new(1.0, 1.0), &b, &horizontal, &stroke(1.0))).height();
        assert!(near(h, 10.0, 0.05), "{mode:?}: {h}");
        let sc = Scatter {
            size: (50.0, 300.0),
            modes: Some([mode, Variation::Fixed, Variation::Fixed, Variation::Fixed]),
            ..scatter(&scatter_brush(100.0))
        };
        let s = Brush { name: "s".into(), kind: BrushKind::Scatter(sc) };
        let heights: Vec<f64> =
            stroke_pieces(&Document::new(1.0, 1.0), &s, &horizontal, &stroke(1.0)).iter().map(|n| n.geometric_bounds().unwrap().height()).collect();
        assert!(!heights.is_empty() && heights.iter().all(|h| near(*h, 7.0, 1e-6)), "{mode:?}: {heights:?}");
    }
}

/// A mouse stroke records no pressure, tilt, bearing or rotation. On it, a brush whose values are
/// on Tilt, Bearing or Rotation draws exactly what the same brush on Pressure draws, the output a
/// mouse stroke gave before these modes existed.
#[test]
fn a_mouse_stroke_draws_pen_modes_as_pressure_did() {
    let doc = Document::new(1.0, 1.0);
    let wavy = {
        let mut bp = line(0.0, 0.0, 60.0, 0.0);
        bp.curve_to((90.0, 0.0), (90.0, 40.0), (120.0, 40.0));
        bp
    };
    let mouse = stroke(1.0);
    assert!(mouse.pressure.is_none() && mouse.tilt.is_none() && mouse.bearing.is_none() && mouse.rotation.is_none());
    let calli = |mode: Variation| {
        let c = Calligraphic { angle: 30.0, roundness: 40.0, size: 8.0, variation: [60.0, 30.0, 4.0], modes: Some([mode; 3]) };
        stroke_pieces(&doc, &Brush { name: "c".into(), kind: BrushKind::Calligraphic(c) }, &wavy, &mouse)
    };
    let scattered = |mode: Variation| {
        let sc = Scatter {
            size: (50.0, 300.0),
            spacing: (80.0, 160.0),
            scatter: (-40.0, 40.0),
            rotation: (-90.0, 90.0),
            modes: Some([mode; 4]),
            ..scatter(&scatter_brush(100.0))
        };
        stroke_pieces(&doc, &Brush { name: "s".into(), kind: BrushKind::Scatter(sc) }, &wavy, &mouse)
    };
    let (calli_before, scatter_before) = (calli(Variation::Pressure), scattered(Variation::Pressure));
    assert!(!calli_before.is_empty() && !scatter_before.is_empty());
    for mode in [Variation::Tilt, Variation::Bearing, Variation::Rotation, Variation::StylusWheel] {
        assert_eq!(calli(mode), calli_before, "{mode:?}");
        assert_eq!(scattered(mode), scatter_before, "{mode:?}");
    }
}

/// The scatter brush of `b`.
fn scatter(b: &Brush) -> Scatter {
    let BrushKind::Scatter(sc) = &b.kind else { panic!("not a scatter brush") };
    sc.clone()
}

/// #764: Scatter values on Tilt go from the first value with the pen lying flat to the second
/// upright; on Bearing, from the first at −180° to the second at 180°.
#[test]
fn scatter_values_follow_tilt_and_bearing() {
    let bp = line(0.0, 0.0, 400.0, 0.0);
    let boxes = |b: &Brush, st: &StrokeLayer| -> Vec<Rect> {
        stroke_pieces(&Document::new(1.0, 1.0), b, &bp, st).iter().map(|n| n.geometric_bounds().unwrap()).collect()
    };
    // Size on Tilt, 50 % to 300 %: upright at the start (12 pt tall), flat at the end (2 pt).
    let upright_to_flat = StrokeLayer { tilt: Some(vectorcraft_doc::PressureProfile { points: vec![(0.0, 0.0), (1.0, 1.0)] }), ..stroke(1.0) };
    let h: Vec<f64> = boxes(&sized_scatter(50.0, 300.0, Variation::Tilt), &upright_to_flat).iter().map(Rect::height).collect();
    assert!(near(h[0], 12.0, 0.2) && h[h.len() - 1] < 3.0, "{h:?}");
    assert!(h.windows(2).all(|w| w[1] <= w[0] + 1e-9), "shrinks as the pen tips over: {h:?}");
    // Rotation on Bearing, −180° to 180°: the copies turn with the bearing, a quarter turn
    // standing the 10 × 4 pt art on end.
    let sc = Scatter {
        rotation: (-180.0, 180.0),
        modes: Some([Variation::Fixed, Variation::Fixed, Variation::Fixed, Variation::Bearing]),
        ..scatter(&scatter_brush(100.0))
    };
    let turning = Brush { name: "r".into(), kind: BrushKind::Scatter(sc) };
    for (bearing, (w, h)) in [(0.0, (10.0, 4.0)), (90.0, (4.0, 10.0)), (-90.0, (4.0, 10.0))] {
        let b = boxes(&turning, &posed(Some(0.5), Some(bearing), None));
        assert!(!b.is_empty() && b.iter().all(|r| near(r.width(), w, 1e-6) && near(r.height(), h, 1e-6)), "{bearing}: {b:?}");
    }
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
    assert_eq!(
        serde_json::to_value(Variation::ALL).unwrap(),
        serde_json::json!(["fixed", "random", "pressure", "tilt", "bearing", "rotation", "stylusWheel"])
    );
    assert!(Variation::ALL.iter().all(|v| Variation::parse(v.id()) == Some(*v) && serde_json::to_value(v).unwrap() == v.id()));
    assert!(!Variation::offered(false).contains(&Variation::StylusWheel) && Variation::offered(true).contains(&Variation::StylusWheel));
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

/// Art Brush Options: Width on Tilt goes from Width with the pen lying flat to the second width
/// upright, and on Bearing or Rotation from Width at −180° to the second width at 180°, along the
/// stroke as the pen recorded it; each mode reads only its own input.
#[test]
fn art_brush_width_follows_tilt_bearing_and_rotation() {
    let brush = |mode: Variation| {
        let mut b = art_brush(rect_art(10.0, 4.0), ArtScale::Stretch);
        if let BrushKind::Art(a) = &mut b.kind {
            (a.width, a.width_mode, a.width_max) = (50.0, Some(mode), Some(300.0));
        }
        b
    };
    let bp = line(0.0, 0.0, 100.0, 0.0);
    let draw = |b: &Brush, st: &StrokeLayer| stroke_pieces(&Document::new(1.0, 1.0), b, &bp, st);
    // Tilt, upright at the start (300 %: 6 pt either side of 4 pt art) to flat at the end (50 %: 1 pt).
    let upright_to_flat = StrokeLayer { tilt: Some(vectorcraft_doc::PressureProfile { points: vec![(0.0, 0.0), (1.0, 1.0)] }), ..stroke(1.0) };
    let out = draw(&brush(Variation::Tilt), &upright_to_flat);
    assert!(near(half_width(&out, 0.0, 1.0), 6.0, 0.1), "{}", half_width(&out, 0.0, 1.0));
    assert!(near(half_width(&out, 99.0, 100.0), 1.0, 0.1), "{}", half_width(&out, 99.0, 100.0));
    assert!(near(half_width(&out, 49.0, 51.0), 3.5, 0.15), "{}", half_width(&out, 49.0, 51.0));
    // Bearing and Rotation: half way at 0°, a quarter of the way at −90°, three quarters at 90°.
    for (mode, angle, half) in [
        (Variation::Bearing, 0.0, 3.5),
        (Variation::Bearing, -90.0, 2.25),
        (Variation::Bearing, 90.0, 4.75),
        (Variation::Rotation, -90.0, 2.25),
        (Variation::Rotation, 90.0, 4.75),
    ] {
        let st = if mode == Variation::Bearing { posed(Some(0.5), Some(angle), None) } else { posed(None, None, Some(angle)) };
        let h = bounds(&draw(&brush(mode), &st)).height();
        assert!(near(h, 2.0 * half, 1e-6), "{mode:?} {angle}: {h}");
    }
    // Each mode reads its own input: without it (the pressure, or another of the pen's values
    // recorded instead), the art is half way between the two widths.
    let others = [
        (Variation::Tilt, posed(None, Some(90.0), Some(90.0))),
        (Variation::Tilt, pressed(vec![(0.0, 0.0), (1.0, 1.0)])),
        (Variation::Bearing, posed(Some(0.0), None, Some(90.0))),
        (Variation::Rotation, posed(Some(0.0), Some(90.0), None)),
        (Variation::Pressure, posed(Some(0.0), Some(90.0), Some(90.0))),
    ];
    for (mode, st) in others {
        let h = bounds(&draw(&brush(mode), &st)).height();
        assert!(near(h, 7.0, 1e-6), "{mode:?}: {h}");
    }
    // The pen's modes are kept when a definition is read, and Random is read as Fixed.
    let lib = parse_library(&serde_json::json!([
        {"name": "t", "type": "art", "art": rect_art(10.0, 4.0), "width_mode": "tilt", "width_max": 300.0},
        {"name": "b", "type": "art", "art": rect_art(10.0, 4.0), "width_mode": "bearing", "width_max": 300.0},
        {"name": "r", "type": "art", "art": rect_art(10.0, 4.0), "width_mode": "rotation", "width_max": 300.0},
        {"name": "x", "type": "art", "art": rect_art(10.0, 4.0), "width_mode": "random", "width_max": 300.0},
    ]));
    let modes: Vec<_> = lib.iter().map(|b| if let BrushKind::Art(a) = &b.kind { a.width_mode } else { None }).collect();
    assert_eq!(modes, [Some(Variation::Tilt), Some(Variation::Bearing), Some(Variation::Rotation), None]);
}

/// A pattern brush with `scale`, `scale_mode` and `scale_max`, its 10 × 4 pt side tile on `fit`.
fn scaled_pattern(fit: PatternFit, corners: bool, scale: f64, mode: Option<Variation>, max: Option<f64>) -> Brush {
    let mut b = pattern_brush(fit, corners);
    if let BrushKind::Pattern(p) = &mut b.kind {
        (p.scale, p.scale_mode, p.scale_max) = (scale, mode, max);
    }
    b
}

/// The bounds of each piece, left to right.
fn piece_spans(out: &[Node]) -> Vec<Rect> {
    let mut spans: Vec<Rect> = out.iter().filter_map(Node::geometric_bounds).collect();
    spans.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    spans
}

/// Pattern Brush Options: with Scale on Pressure the tiles go from Scale at the lightest pen
/// pressure to the second scale at the heaviest, taller and longer along the stroke where the pen
/// pressed harder, and each Fit still fits them to the path.
#[test]
fn pattern_brush_scale_follows_the_pressure() {
    let bp = line(0.0, 0.0, 200.0, 0.0);
    let draw = |b: &Brush, st: &StrokeLayer| stroke_pieces(&Document::new(1.0, 1.0), b, &bp, st);
    let ramp = pressed(vec![(0.0, 0.0), (1.0, 1.0)]);
    // 4 pt tall tiles at 50 %: 1 pt either side at the lightest; at 300 %: 6 pt at the heaviest.
    let out = draw(&scaled_pattern(PatternFit::Stretch, false, 50.0, Some(Variation::Pressure), Some(300.0)), &ramp);
    assert!(near(half_width(&out, 0.0, 1.0), 1.0, 0.1), "{}", half_width(&out, 0.0, 1.0));
    assert!(near(half_width(&out, 199.0, 200.0), 6.0, 0.2), "{}", half_width(&out, 199.0, 200.0));
    assert!(near(half_width(&out, 99.0, 101.0), 3.5, 0.2), "{}", half_width(&out, 99.0, 101.0));
    // Stretch to Fit fills the path, and the tiles grow longer along it.
    let spans = piece_spans(&out);
    assert!(near(bounds(&out).x0, 0.0, 1e-6) && near(bounds(&out).x1, 200.0, 1e-6), "{:?}", bounds(&out));
    assert!(spans.windows(2).all(|w| w[1].width() > w[0].width()), "{spans:?}");
    assert!(spans.last().unwrap().width() > 4.0 * spans[0].width(), "{spans:?}");
    // The stroke weight scales it.
    let out =
        draw(&scaled_pattern(PatternFit::Stretch, false, 50.0, Some(Variation::Pressure), Some(300.0)), &StrokeLayer { width: 2.0, ..ramp.clone() });
    assert!(near(half_width(&out, 199.0, 200.0), 12.0, 0.4), "{}", half_width(&out, 199.0, 200.0));
    // Without recorded pressure, half way between the two scales.
    let out = draw(&scaled_pattern(PatternFit::Stretch, false, 50.0, Some(Variation::Pressure), Some(300.0)), &stroke(1.0));
    assert!(near(bounds(&out).height(), 7.0, 1e-6), "{:?}", bounds(&out));
    // With an even pressure, Add Space with spacing places as many tiles as the one scale that
    // pressure gives: the spacing counts when the tiles are counted again.
    let even = pressed(vec![(0.0, 0.5), (1.0, 0.5)]);
    let spaced = |mode: Option<Variation>, max: Option<f64>| {
        let mut b = scaled_pattern(PatternFit::AddSpace, false, if max.is_some() { 50.0 } else { 100.0 }, mode, max);
        if let BrushKind::Pattern(p) = &mut b.kind {
            p.spacing = 100.0;
        }
        piece_spans(&draw(&b, &even)).len()
    };
    assert_eq!(spaced(Some(Variation::Pressure), Some(150.0)), spaced(None, None));
    // With spacing, Stretch to Fit still fills the path and Approximate Path still centers the
    // tiles, with the gaps between them counted and none after the last.
    for fit in [PatternFit::Stretch, PatternFit::Approximate] {
        let mut b = scaled_pattern(fit, false, 50.0, Some(Variation::Pressure), Some(300.0));
        if let BrushKind::Pattern(p) = &mut b.kind {
            p.spacing = 50.0;
        }
        let spans = piece_spans(&draw(&b, &ramp));
        let (first, last) = (spans[0], spans[spans.len() - 1]);
        assert!(spans.len() > 1 && near((first.x0 + last.x1) / 2.0, 100.0, 0.01), "{fit:?}: {spans:?}");
        if fit == PatternFit::Stretch {
            assert!(near(first.x0, 0.0, 1e-6) && near(last.x1, 200.0, 1e-6), "{spans:?}");
        }
    }
    // Add Space to Fit keeps the tiles apart within the path; Approximate Path centers them on it.
    for fit in [PatternFit::AddSpace, PatternFit::Approximate] {
        let out = draw(&scaled_pattern(fit, false, 50.0, Some(Variation::Pressure), Some(300.0)), &ramp);
        let spans = piece_spans(&out);
        assert!(spans.len() > 1 && spans.windows(2).all(|w| w[1].x0 >= w[0].x1 - 1e-6), "{fit:?}: {spans:?}");
        assert!(spans.windows(2).all(|w| w[1].width() > w[0].width()), "{fit:?}: {spans:?}");
        let (first, last) = (spans[0], spans[spans.len() - 1]);
        if fit == PatternFit::AddSpace {
            assert!(first.x0 >= -1e-6 && last.x1 <= 200.0 + 1e-6, "{spans:?}");
        } else {
            assert!(near((first.x0 + last.x1) / 2.0, 100.0, 0.01), "{spans:?}");
        }
    }
}

/// Pattern Brush Options: Scale on Tilt, Bearing, Rotation or Stylus Wheel reads only its own
/// input, half way without it; Fixed, a mode without a second scale and equal scales draw as a
/// brush without the setting does.
#[test]
fn pattern_brush_scale_follows_tilt_bearing_rotation_and_the_wheel() {
    let bp = line(0.0, 0.0, 100.0, 0.0);
    let draw = |b: &Brush, st: &StrokeLayer| stroke_pieces(&Document::new(1.0, 1.0), b, &bp, st);
    let brush = |mode: Variation| scaled_pattern(PatternFit::Stretch, false, 50.0, Some(mode), Some(300.0));
    let wheel = |v: f64| StrokeLayer { wheel: Some(vectorcraft_doc::PressureProfile { points: vec![(0.0, v), (1.0, v)] }), ..stroke(1.0) };
    // 4 pt tiles: 2 pt tall at 50 %, 12 pt at 300 %.
    for (mode, st, height) in [
        (Variation::Tilt, posed(Some(0.0), None, None), 12.0),
        (Variation::Tilt, posed(Some(1.0), None, None), 2.0),
        (Variation::Bearing, posed(Some(0.5), Some(90.0), None), 9.5),
        (Variation::Rotation, posed(None, None, Some(-90.0)), 4.5),
        (Variation::StylusWheel, wheel(1.0), 12.0),
        (Variation::StylusWheel, wheel(0.0), 2.0),
        // Without its own input, half way.
        (Variation::Tilt, posed(None, Some(90.0), Some(90.0)), 7.0),
        (Variation::Bearing, posed(Some(0.0), None, Some(90.0)), 7.0),
        (Variation::Rotation, posed(Some(0.0), Some(90.0), None), 7.0),
        (Variation::StylusWheel, pressed(vec![(0.0, 1.0), (1.0, 1.0)]), 7.0),
        (Variation::Pressure, posed(Some(0.0), Some(90.0), Some(90.0)), 7.0),
    ] {
        let h = bounds(&draw(&brush(mode), &st)).height();
        assert!(near(h, height, 1e-6), "{mode:?}: {h}");
    }
    // Fixed, a mode without a second scale, and a second scale equal to the first draw what the
    // brush draws without them.
    let ramp = pressed(vec![(0.0, 0.0), (1.0, 1.0)]);
    let plain = draw(&scaled_pattern(PatternFit::Stretch, true, 50.0, None, None), &ramp);
    for (mode, max) in [(None, Some(300.0)), (Some(Variation::Pressure), None), (Some(Variation::Pressure), Some(50.0))] {
        assert_eq!(draw(&scaled_pattern(PatternFit::Stretch, true, 50.0, mode, max), &ramp), plain, "{mode:?} {max:?}");
    }
    let saved = serde_json::to_value(scaled_pattern(PatternFit::Stretch, false, 50.0, None, None)).unwrap();
    assert!(saved.get("scale_mode").is_none() && saved.get("scale_max").is_none(), "{saved}");
    // A mode the scale does not take is read as Fixed, and the second scale is kept within range.
    let lib = parse_library(&serde_json::json!([
        {"name": "a", "type": "pattern", "side": rect_art(10.0, 4.0), "scale_mode": "random", "scale_max": 1e308},
        {"name": "b", "type": "pattern", "side": rect_art(10.0, 4.0), "scale_mode": "stylusWheel", "scale_max": 0.0},
    ]));
    let [BrushKind::Pattern(a), BrushKind::Pattern(b)] = [&lib[0].kind, &lib[1].kind] else { panic!("{lib:?}") };
    assert_eq!((a.scale_mode, a.scale_max, b.scale_mode, b.scale_max), (None, Some(10000.0), Some(Variation::StylusWheel), Some(1.0)));
    // A stroke sets its own mode and second scale.
    let mut own = serde_json::Map::new();
    own.insert("scale_mode".into(), serde_json::json!("pressure"));
    own.insert("scale_max".into(), serde_json::json!(300.0));
    let edited = scaled_pattern(PatternFit::Stretch, false, 100.0, None, None).with_stroke_options(&own).unwrap();
    assert!(matches!(&edited.kind, BrushKind::Pattern(p) if p.scale_range() == (100.0, 300.0)), "{edited:?}");
}

/// A pattern brush whose scale follows the pen keeps its start, end and corner tiles at the scale
/// where they are, on open and closed paths, and stays finite at the ends of the scale's range.
#[test]
fn pattern_brush_scale_from_the_pen_places_start_end_and_corner_tiles() {
    let ramp = pressed(vec![(0.0, 0.0), (1.0, 1.0)]);
    let doc = Document::new(1.0, 1.0);
    // Start and end tiles, 6 × 4 pt: the start tile at the light end is smaller than the end tile.
    let mut ends = scaled_pattern(PatternFit::Stretch, false, 50.0, Some(Variation::Pressure), Some(300.0));
    if let BrushKind::Pattern(p) = &mut ends.kind {
        (p.start, p.end) = (Some(rect_art(6.0, 4.0)), Some(rect_art(6.0, 4.0)));
    }
    let out = stroke_pieces(&doc, &ends, &line(0.0, 0.0, 200.0, 0.0), &ramp);
    let (start, end) = (out[0].geometric_bounds().unwrap(), out[1].geometric_bounds().unwrap());
    assert!(near(start.x0, 0.0, 1e-6) && near(end.x1, 200.0, 1e-6), "{start:?} {end:?}");
    assert!(start.width() < 3.5 && end.width() > 17.0 && start.height() < end.height() / 4.0, "{start:?} {end:?}");
    // A closed rectangle with corner tiles, on each Fit, and the scale's range at its ends.
    let mut rect = BezPath::new();
    rect.move_to((0.0, 0.0));
    rect.line_to((100.0, 0.0));
    rect.line_to((100.0, 60.0));
    rect.line_to((0.0, 60.0));
    rect.close_path();
    for fit in [PatternFit::Stretch, PatternFit::AddSpace, PatternFit::Approximate] {
        for (scale, max) in [(50.0, 300.0), (1.0, 10000.0), (10000.0, 1.0)] {
            for bp in [&rect, &line(0.0, 0.0, 3.0, 0.0)] {
                let b = scaled_pattern(fit, true, scale, Some(Variation::Pressure), Some(max));
                let out = stroke_pieces(&doc, &b, bp, &ramp);
                assert!(!out.is_empty(), "{fit:?} {scale} {max}");
                let r = bounds(&out);
                assert!([r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()), "{fit:?} {scale} {max}: {r:?}");
            }
        }
    }
    // On the rectangle the corner tiles grow with the pressure: the first corner's tile is smaller
    // than the last one's.
    let b = scaled_pattern(PatternFit::Stretch, true, 50.0, Some(Variation::Pressure), Some(300.0));
    let out = stroke_pieces(&doc, &b, &rect, &ramp);
    let fixed = stroke_pieces(&doc, &scaled_pattern(PatternFit::Stretch, true, 50.0, None, None), &rect, &ramp);
    assert_ne!(out, fixed);
    let corner = |n: &Node| n.geometric_bounds().map_or(0.0, |r| r.width().max(r.height()));
    let corners: Vec<f64> = out.iter().take(4).map(corner).collect();
    assert!(corners[0] < corners[3], "{corners:?}");
    // The side tiles start half the corner tile beside them away from the corner: 1.5 pt from the
    // first corner, where the 6 pt corner tile is drawn at 50 %, not half the largest corner tile.
    let first_side = out.get(4).and_then(Node::geometric_bounds);
    assert!(first_side.is_some_and(|r| near(r.x0, 1.5, 1e-3)), "{first_side:?}");
}

/// A start or end tile whose scale follows the pen is placed where it is as long as the scale at
/// its middle, even when the scale rises again further along and the tile would not fit at the
/// whole run's length.
#[test]
fn pattern_brush_scale_from_the_pen_fits_start_and_end_tiles_where_the_scale_falls_back() {
    let doc = Document::new(1.0, 1.0);
    let bp = line(0.0, 0.0, 100.0, 0.0);
    // From 100 % to 10000 %: 300 % from 5 to 20 pt, 10000 % from 50 pt on; mirrored for the end.
    let q = 2.0 / 99.0;
    for (start, points, span) in [
        (true, vec![(0.0, 0.0), (0.05, q), (0.2, q), (0.5, 1.0), (1.0, 1.0)], (0.0, 18.0)),
        (false, vec![(0.0, 1.0), (0.5, 1.0), (0.8, q), (0.95, q), (1.0, 0.0)], (82.0, 100.0)),
    ] {
        let mut b = scaled_pattern(PatternFit::Stretch, false, 100.0, Some(Variation::Pressure), Some(10000.0));
        if let BrushKind::Pattern(p) = &mut b.kind {
            let tile = Some(rect_art(6.0, 4.0));
            (p.start, p.end) = if start { (tile, None) } else { (None, tile) };
        }
        // The 6 pt tile at 300 %: 18 pt long, its middle where the scale is 300 %.
        let out = stroke_pieces(&doc, &b, &bp, &pressed(points));
        let r = out.first().and_then(Node::geometric_bounds);
        assert!(r.is_some_and(|r| near(r.x0, span.0, 1e-6) && near(r.x1, span.1, 1e-6)), "start {start}: {r:?}");
    }
    // 400 % at the start, 100 % from 15 to 20 pt, 400 % from 40 pt on, mirrored for the end: a 30 pt
    // tile at 100 % fits at its middle, although at the scale at its first length's middle it is as
    // long as that length.
    for (start, points, span) in [
        (true, vec![(0.0, 1.0), (0.15, 0.0), (0.2, 0.0), (0.4, 1.0), (1.0, 1.0)], (0.0, 30.0)),
        (false, vec![(0.0, 1.0), (0.6, 1.0), (0.8, 0.0), (0.85, 0.0), (1.0, 1.0)], (70.0, 100.0)),
    ] {
        let mut b = scaled_pattern(PatternFit::Stretch, false, 100.0, Some(Variation::Pressure), Some(400.0));
        if let BrushKind::Pattern(p) = &mut b.kind {
            let tile = Some(rect_art(30.0, 4.0));
            (p.start, p.end) = if start { (tile, None) } else { (None, tile) };
        }
        let out = stroke_pieces(&doc, &b, &bp, &pressed(points));
        let r = out.first().and_then(Node::geometric_bounds);
        assert!(r.is_some_and(|r| near(r.x0, span.0, 1e-6) && near(r.x1, span.1, 1e-6)), "start {start}: {r:?}");
    }
}

/// On a closed path, a tile that Approximate Path starts before the path's start takes the scale
/// recorded at the end of the stroke there, where the tile is drawn.
#[test]
fn pattern_brush_scale_from_the_pen_wraps_before_a_closed_paths_start() {
    let doc = Document::new(1.0, 1.0);
    let b = scaled_pattern(PatternFit::Approximate, false, 50.0, Some(Variation::Pressure), Some(300.0));
    // Light over the first 90 % of the stroke, heavy at its end.
    let st = pressed(vec![(0.0, 0.0), (0.9, 0.0), (1.0, 1.0)]);
    let mut before = 0;
    for r in (100..=120).map(f64::from) {
        // A circle of radius `r` as 720 sides, from (r, 0) on towards +y.
        let mut bp = BezPath::new();
        bp.move_to((r, 0.0));
        for i in 1..720 {
            let a = std::f64::consts::TAU * f64::from(i) / 720.0;
            bp.line_to((r * a.cos(), r * a.sin()));
        }
        bp.close_path();
        let out = stroke_pieces(&doc, &b, &bp, &st);
        let Some(first) = out.first().and_then(Node::geometric_bounds) else { panic!("{r}: no tiles") };
        if first.y0 < -0.5 {
            // The 4 pt tall tile is 2 pt tall at the light 50 % and nearly 12 pt at the heavy end.
            before += 1;
            assert!(first.x1 > r + 3.0, "{r}: {first:?}");
        }
    }
    assert!(before > 0, "no first tile starts before the path's start");
}

/// On a closed path, a tile over the path's start follows the scale on both sides of the start,
/// even when the scale is the same at the tile's ends, its middle and every pen sample within it.
#[test]
fn pattern_brush_scale_from_the_pen_changes_where_a_closed_path_wraps_round() {
    let doc = Document::new(1.0, 1.0);
    let mut b = scaled_pattern(PatternFit::Approximate, false, 100.0, Some(Variation::Pressure), Some(300.0));
    if let BrushKind::Pattern(p) = &mut b.kind {
        p.side = rect_art(54.0, 4.0);
    }
    // A closed path 256 pt long without corners, from (0, 0) along +x: its sides in each quarter
    // turn, and the same turned three more times.
    let quarter = [(28.0, 0.0), (12.0, 5.0), (4.0, 3.0), (3.0, 4.0), (5.0, 12.0)];
    let mut bp = BezPath::new();
    let mut at = (0.0, 0.0);
    bp.move_to(at);
    for turn in 0..4 {
        for &(x, y) in &quarter {
            let (dx, dy) = match turn {
                0 => (x, y),
                1 => (-y, x),
                2 => (-x, -y),
                _ => (y, -x),
            };
            at = (at.0 + dx, at.1 + dy);
            if at != (0.0, 0.0) {
                bp.line_to(at);
            }
        }
    }
    bp.close_path();
    // 100 % up to 4 pt before the end, rising to 300 % at the end. Approximate Path places five
    // 54 pt tiles, the first from 7 pt before the start: at 100 % at both its ends, its middle and
    // the samples at 4 pt before the start and at the start, but larger from 4 pt before it.
    let st = pressed(vec![(0.0, 0.0), (252.0 / 256.0, 0.0), (1.0, 1.0)]);
    let out = stroke_pieces(&doc, &b, &bp, &st);
    let Some(first) = out.first().and_then(Node::geometric_bounds) else { panic!("no tiles") };
    // The 4 pt tall tile is 2 pt either side of the path at 100 %, and 5 pt at 1 pt before the
    // start, where the scale is 250 %.
    assert!(first.y0 < -3.5, "{first:?}");
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
        doc.symbols.push(Symbol { dynamic: false, name: name.into(), art: std::sync::Arc::new(art.clone()) });
        doc.patterns.push(PatternDef::new(name, vec![std::sync::Arc::new(art)]));
    }
    let instance = |symbol: &str| Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: symbol.into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let painted = |pattern: &str| {
        let fill = Paint::Pattern { pattern: pattern.into(), xf: vectorcraft_geom::Affine::IDENTITY };
        Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 4.0)), Appearance::basic(fill, Paint::None, 0.0))
    };
    let art = Node::group(NodeId(0), [instance("plain"), instance("brushed"), painted("plain"), painted("brushed")].map(std::sync::Arc::new).into());
    let b = Brush { name: "s".into(), kind: BrushKind::Scatter(Scatter { art, colorization: Colorization::None, ..Default::default() }) };
    store(&mut doc, std::slice::from_ref(&b));
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

/// A symbol instance at the root of an opacity mask is filtered like one among the children: when
/// its symbol draws a brushed stroke the mask goes (it would run the brush again from inside the
/// mask, without end), and a mask whose root symbol is plain stays.
#[test]
fn pieces_leave_out_a_mask_whose_root_draws_a_brushed_stroke() {
    use vectorcraft_doc::{OpacityMask, Symbol};
    let mut doc = Document::new(100.0, 100.0);
    let mut brushed = rect_art(10.0, 4.0);
    brushed.appearance.items.push(vectorcraft_doc::AppearanceItem::Stroke(StrokeLayer { brush: Some("s".into()), ..stroke(1.0) }));
    // A brush the library lacks draws a plain stroke: its mask stays.
    let mut unresolved = rect_art(10.0, 4.0);
    unresolved.appearance.items.push(vectorcraft_doc::AppearanceItem::Stroke(StrokeLayer { brush: Some("missing".into()), ..stroke(1.0) }));
    for (name, art) in [("plain", rect_art(10.0, 4.0)), ("brushed", brushed), ("unresolved", unresolved)] {
        doc.symbols.push(Symbol { dynamic: false, name: name.into(), art: std::sync::Arc::new(art) });
    }
    let instance = |symbol: &str| Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: symbol.into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let masked = |symbol: &str| {
        let mut n = rect_art(10.0, 4.0);
        n.extras_mut().mask = Some(Box::new(OpacityMask::new(instance(symbol), true)));
        n
    };
    let art = Node::group(NodeId(0), [masked("plain"), masked("brushed"), masked("unresolved")].map(std::sync::Arc::new).into());
    let b = Brush { name: "s".into(), kind: BrushKind::Scatter(Scatter { art, colorization: Colorization::None, ..Default::default() }) };
    store(&mut doc, std::slice::from_ref(&b));
    let out = stroke_pieces(&doc, &b, &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    assert!(!out.is_empty());
    for piece in &out {
        let roots: Vec<Option<&str>> = piece
            .children()
            .unwrap()
            .iter()
            .map(|c| match c.extras().mask.as_ref().map(|m| &m.art.kind) {
                Some(NodeKind::SymbolInstance { symbol, .. }) => Some(symbol.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(roots, [Some("plain"), None, Some("unresolved")], "the plain and unresolved masks stay and the brushed one goes");
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

/// A straight-sided art brush on a smooth curve draws smooth edges: they do not turn at the smooth
/// anchors the fit puts along them.
#[test]
fn art_brush_edges_stay_smooth_along_a_curve() {
    for &(r, w, h, sw) in &[(100.0, 100.0, 10.0, 1.0), (300.0, 100.0, 10.0, 2.0), (30.0, 100.0, 10.0, 1.0), (2000.0, 100.0, 10.0, 1.0)] {
        // The top half of a circle, as one open path of two curves.
        let ring = shapes::ellipse(Rect::new(0.0, 0.0, 2.0 * r, 2.0 * r)).to_bezpath();
        let half_circle: BezPath = ring.elements().iter().take(3).copied().collect();
        let out = stroke_pieces(&Document::new(1.0, 1.0), &art_brush(rect_art(w, h), ArtScale::Stretch), &half_circle, &stroke(sw));
        let NodeKind::Path { path, .. } = &out[0].kind else { panic!("an art brush on a path draws a path") };

        // The turn at each smooth anchor of the edges: the angle between the incoming and the
        // outgoing direction. Real corners (the ends of the art) turn by far more than 60°.
        let mut kink = 0.0f64;
        for sp in &path.subpaths {
            for (k, a) in sp.anchors.iter().enumerate().skip(1).take(sp.anchors.len().saturating_sub(2)) {
                let h_in = if a.h_in != a.p { a.h_in } else { sp.anchors[k - 1].p };
                let h_out = if a.h_out != a.p { a.h_out } else { sp.anchors[k + 1].p };
                let (u, v) = (a.p - h_in, h_out - a.p);
                if u.hypot() < 1e-9 || v.hypot() < 1e-9 || u.dot(v) < 0.5 * u.hypot() * v.hypot() {
                    continue;
                }
                kink = kink.max(u.cross(v).atan2(u.dot(v)).abs().to_degrees());
            }
        }
        assert!(kink < 0.5, "r={r} art={w}x{h} weight {sw}: an edge turns by {kink:.3}° at a smooth anchor");
    }
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

/// The other half of the same behaviour: where the path really does bend, an affine cannot
/// reproduce the bend, so the tile is bent, and it still comes out as curves: cubics fitted to the
/// bent art rather than the chords it used to be flattened into (#3926). Here a 4-anchor circle
/// tile runs round a circle of radius 200, so its outline stays in the band 20 either side of it.
#[test]
fn a_pattern_tile_on_a_bent_path_keeps_its_curves() {
    let circle =
        Node::path(NodeId(0), shapes::ellipse(Rect::new(0.0, -20.0, 40.0, 20.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let brush = Brush {
        name: "p".into(),
        kind: BrushKind::Pattern(PatternBrush { side: circle, fit: PatternFit::Stretch, colorization: Colorization::None, ..Default::default() }),
    };
    // A circle to run along: every tile sits on a curving section.
    let round = shapes::ellipse(Rect::new(0.0, 0.0, 400.0, 400.0)).to_bezpath();
    let out = stroke_pieces(&Document::new(1.0, 1.0), &brush, &round, &stroke(1.0));
    assert!(out.len() > 20, "tiles all the way round: {}", out.len());
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for n in &out {
        let (anchors, curved) = anchors_and_curved(n);
        assert!((4..=16).contains(&anchors), "a bent circle tile is a few curves, not a polygon: {anchors} anchors");
        assert!(curved == anchors, "every anchor keeps a handle: {curved} of {anchors}");
        let mut pts = vec![];
        all_points(n, &mut pts);
        for q in pts {
            let r = q.distance(Point::new(200.0, 200.0));
            (lo, hi) = (lo.min(r), hi.max(r));
        }
    }
    // The path is a cubic circle (0.06 out at most) run along in chords of 0.05, and the fit holds
    // a quarter of the 0.25 flattening tolerance, so the band holds to 0.2.
    assert!(lo > 179.8 && hi < 220.2, "the curves stay in the tile's band round the path: {lo:.3}..{hi:.3}");
    assert!(lo < 180.2 && hi > 219.8, "and reach both edges of it: {lo:.3}..{hi:.3}");
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

/// `options` as a stroke's own brush settings.
fn with_options(options: serde_json::Value) -> StrokeLayer {
    StrokeLayer { brush_options: options.as_object().cloned().unwrap_or_default(), ..stroke(1.0) }
}

#[test]
fn a_strokes_own_options_change_its_art_and_leave_the_brush() {
    let doc = Document::new(1.0, 1.0);
    let brush = art_brush(rect_art(10.0, 4.0), ArtScale::Stretch);
    let path = line(20.0, 50.0, 220.0, 50.0);
    let half = stroke_pieces(&doc, &brush, &path, &with_options(json!({"width": 50.0})));
    assert!(near(bounds(&half).height(), 2.0, 1e-6), "width 50% halves the art across the stroke: {:?}", bounds(&half));
    assert!(near(bounds(&stroke_pieces(&doc, &brush, &path, &stroke(1.0))).height(), 4.0, 1e-6), "a stroke without options draws the brush");
    let BrushKind::Art(a) = &brush.kind else { panic!("art brush") };
    assert_eq!(a.width, 100.0, "the brush keeps its width");
    // A calligraphic stroke's own size and angle.
    let nib = stroke_pieces(&doc, &calli(0.0, 20.0, 10.0), &line(0.0, 0.0, 100.0, 0.0), &with_options(json!({"angle": 90.0})));
    assert!(near(bounds(&nib).height(), 10.0, 0.05), "{:?}", bounds(&nib));
    // A pattern stroke's own scale doubles the tiles across it.
    let pattern = pattern_brush(PatternFit::Stretch, false);
    let plain = bounds(&stroke_pieces(&doc, &pattern, &path, &stroke(1.0))).height();
    let doubled = bounds(&stroke_pieces(&doc, &pattern, &path, &with_options(json!({"scale": 200.0})))).height();
    assert!(near(doubled, plain * 2.0, 1e-6), "{plain} → {doubled}");
}

#[test]
fn stroke_options_are_checked_and_kept_within_their_ranges() {
    let brush = art_brush(rect_art(10.0, 4.0), ArtScale::Stretch);
    let map = |v: serde_json::Value| v.as_object().cloned().unwrap_or_default();
    // Out of range: clamped; equal to the brush's own value: left out.
    let got = brush.stroke_options_that_differ(&map(json!({"width": 5000.0, "flip_along": false}))).unwrap();
    assert_eq!(serde_json::Value::Object(got), json!({"width": 1000.0}));
    // A field the type does not take, and a value of the wrong kind, are errors.
    assert!(brush.stroke_options_that_differ(&map(json!({"angle": 10.0}))).unwrap_err().contains("angle"));
    assert!(brush.stroke_options_that_differ(&map(json!({"width": "wide"}))).is_err());
    let bristle = Brush { name: "b".into(), kind: BrushKind::Bristle(Bristle::default()) };
    assert!(bristle.stroke_options_that_differ(&map(json!({"size": 2.0}))).is_err());
    assert!(bristle.stroke_options_that_differ(&map(json!({}))).unwrap().is_empty());
    // A colorization object changes only the fields it gives: Hue Shift keeps the key it has.
    let key = Color::rgb(0.0, 0.0, 1.0);
    let hue = Brush {
        name: "h".into(),
        kind: BrushKind::Art(ArtBrush { art: rect_art(10.0, 4.0), colorization: Colorization::HueShift { key }, ..Default::default() }),
    };
    let tints = hue.with_stroke_options(&map(json!({"colorization": {"method": "tints"}}))).unwrap();
    assert_eq!(tints.kind.colorization(), Colorization::Tints);
    let back = hue.stroke_options_that_differ(&map(json!({"colorization": {"method": "hueShift"}}))).unwrap();
    assert!(back.is_empty(), "the brush's own colorization: {back:?}");
    // Hue Shift without a key takes the art's main color, as brush.options gives it.
    let keyless = tints.stroke_options_that_differ(&map(json!({"colorization": {"method": "hueShift"}}))).unwrap();
    let want = art_colors(tints.art()).first().copied().unwrap_or(Color::BLACK);
    assert_eq!(keyless["colorization"]["method"], json!("hueShift"));
    assert_eq!(keyless["colorization"]["key"], serde_json::to_value(want).unwrap());
    // Settings that do not read draw the brush as it is.
    let doc = Document::new(1.0, 1.0);
    let path = line(20.0, 50.0, 220.0, 50.0);
    let odd = stroke_pieces(&doc, &brush, &path, &with_options(json!({"width": [1, 2]})));
    assert!(near(bounds(&odd).height(), 4.0, 1e-6));
}

#[test]
fn null_stroke_option_items_keep_the_items_the_stroke_has() {
    let map = |v: serde_json::Value| v.as_object().cloned().unwrap_or_default();
    // A scatter brush at fixed sizes, with no modes of its own: they follow its values.
    let b = Brush { name: "s".into(), kind: BrushKind::Scatter(Scatter { size: (100.0, 100.0), ..Default::default() }) };
    let BrushKind::Scatter(s) = b.with_stroke_options(&map(json!({"size": [null, 150]}))).unwrap().kind else { panic!("scatter") };
    assert_eq!(s.size, (100.0, 150.0), "the smallest size stays the brush's");
    // A stroke whose sizes vary keeps that random size mode when it sets only the spacing mode.
    let e = b.with_stroke_options(&map(json!({"size": [50, 80], "modes": [null, "pressure", null, null]}))).unwrap();
    let BrushKind::Scatter(s) = e.kind else { panic!("scatter") };
    assert_eq!(s.modes(), [Variation::Random, Variation::Pressure, Variation::Fixed, Variation::Fixed]);
    // The kept modes follow the values as they are kept within their ranges.
    let c = calli(0.0, 100.0, 3.0).with_stroke_options(&map(json!({"variation": [-2, 0, 0], "modes": [null, "pressure", null]}))).unwrap();
    let BrushKind::Calligraphic(c) = c.kind else { panic!("calligraphic") };
    assert_eq!(c.modes(), [Variation::Fixed, Variation::Pressure, Variation::Fixed]);
    // Modes that change nothing are no setting of the stroke's: they keep following its values.
    let same = b.stroke_options_that_differ(&map(json!({"modes": [null, null, null, null]}))).unwrap();
    assert!(same.is_empty(), "{same:?}");
}

#[test]
fn a_null_calligraphic_modes_setting_stays_when_another_value_changes() {
    let map = |v: serde_json::Value| v.as_object().cloned().unwrap_or_default();
    // A brush whose modes are its own, and a stroke whose modes follow its values instead.
    let mut b = calli(0.0, 100.0, 3.0);
    if let BrushKind::Calligraphic(c) = &mut b.kind {
        c.modes = Some([Variation::Pressure; 3]);
    }
    let got = b.stroke_options_that_differ(&map(json!({"modes": null, "angle": 30.0}))).unwrap();
    assert_eq!(serde_json::Value::Object(got), json!({"modes": null, "angle": 30.0}));
    let BrushKind::Calligraphic(c) = b.with_stroke_options(&map(json!({"modes": null, "angle": 30.0}))).unwrap().kind else { panic!("calligraphic") };
    assert_eq!(c.modes(), [Variation::Fixed; 3], "the stroke draws with the modes its values give");
    // A brush whose modes follow its values: a null `modes` is the brush's own and is left out.
    let plain = calli(0.0, 100.0, 3.0).stroke_options_that_differ(&map(json!({"modes": null}))).unwrap();
    assert!(plain.is_empty(), "{plain:?}");
}
