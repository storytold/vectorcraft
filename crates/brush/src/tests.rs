use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{BezPath, Point, Rect, shapes};

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
    Brush { name: "c".into(), kind: BrushKind::Calligraphic(Calligraphic { angle, roundness, size, variation: [0.0; 3] }) }
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
    let out = stroke_pieces(&calli(90.0, 20.0, 10.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    assert_eq!(out.len(), 1);
    let b = bounds(&out);
    assert!(near(b.height(), 10.0, 0.05), "{b:?}");
    assert!(near(b.x0, -1.0, 0.05) && near(b.x1, 101.0, 0.05), "caps add half the minor axis: {b:?}");
}

#[test]
fn calligraphic_flat_nib_is_thin_along_its_angle() {
    let out = stroke_pieces(&calli(0.0, 20.0, 10.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    assert!(near(bounds(&out).height(), 2.0, 0.05), "{:?}", bounds(&out));
}

#[test]
fn calligraphic_width_at_45_degrees_matches_ellipse_support() {
    let (a, b) = (5.0_f64, 1.0_f64);
    let out = stroke_pieces(&calli(45.0, 20.0, 10.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    let expect = 2.0 * ((a * a + b * b) / 2.0).sqrt();
    assert!(near(bounds(&out).height(), expect, 0.05), "{} vs {expect}", bounds(&out).height());
    // A vertical stroke with the same nib has the same width (45° is symmetric).
    let v = stroke_pieces(&calli(45.0, 20.0, 10.0), &line(0.0, 0.0, 0.0, 100.0), &stroke(1.0));
    assert!(near(bounds(&v).width(), expect, 0.05));
}

#[test]
fn calligraphic_scales_with_stroke_weight() {
    let out = stroke_pieces(&calli(90.0, 100.0, 3.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(2.0));
    assert!(near(bounds(&out).height(), 6.0, 0.05));
}

#[test]
fn round_nib_is_direction_independent() {
    let out = stroke_pieces(&calli(0.0, 100.0, 4.0), &line(0.0, 0.0, 60.0, 80.0), &stroke(1.0));
    let b = bounds(&out);
    // Round caps: the bounds grow by the radius on every side.
    assert!(near(b.x0, -2.0, 0.05) && near(b.y1, 82.0, 0.05), "{b:?}");
}

#[test]
fn closed_calligraphic_makes_a_ring() {
    let bp = shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let out = stroke_pieces(&calli(0.0, 100.0, 6.0), &bp, &stroke(1.0));
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
    let out = stroke_pieces(&art_brush(rect_art(10.0, 4.0), ArtScale::Stretch), &line(20.0, 50.0, 220.0, 50.0), &stroke(1.0));
    let b = bounds(&out);
    assert!(near(b.x0, 20.0, 1e-6) && near(b.x1, 220.0, 1e-6), "{b:?}");
    assert!(near(b.height(), 4.0, 1e-6));
    // 3 pt stroke → 3× wider.
    let out = stroke_pieces(&art_brush(rect_art(10.0, 4.0), ArtScale::Stretch), &line(20.0, 50.0, 220.0, 50.0), &stroke(3.0));
    assert!(near(bounds(&out).height(), 12.0, 1e-6));
}

#[test]
fn art_brush_arc_length_mapping_is_linear() {
    // A marker square at art x ∈ [5, 6] of a 10 wide art lands at 50..60 % of the path.
    let marker =
        Node::path(NodeId(0), shapes::rectangle(Rect::new(5.0, -1.0, 6.0, 1.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let frame = rect_art(10.0, 2.0);
    let mut spacer = frame.clone();
    spacer.appearance = Appearance::default();
    let art = Node::group(NodeId(0), vec![std::sync::Arc::new(spacer), std::sync::Arc::new(marker)]);
    let out = stroke_pieces(&art_brush(art, ArtScale::Stretch), &line(0.0, 0.0, 300.0, 0.0), &stroke(1.0));
    let g = &out[0];
    let m = g.children().unwrap()[1].geometric_bounds().unwrap();
    assert!(near(m.x0, 150.0, 1e-6) && near(m.x1, 180.0, 1e-6), "{m:?}");
}

#[test]
fn art_brush_proportional_keeps_aspect() {
    let out = stroke_pieces(&art_brush(rect_art(100.0, 4.0), ArtScale::Proportional), &line(0.0, 0.0, 200.0, 0.0), &stroke(1.0));
    assert!(near(bounds(&out).height(), 8.0, 1e-6));
}

#[test]
fn art_brush_between_guides_keeps_ends() {
    // Head (last 20 % of a 100 pt art) keeps its 20 pt length on a 300 pt path.
    let head =
        Node::path(NodeId(0), shapes::rectangle(Rect::new(80.0, -4.0, 100.0, 4.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let shaft = rect_art(80.0, 2.0);
    let art = Node::group(NodeId(0), vec![std::sync::Arc::new(shaft), std::sync::Arc::new(head)]);
    let out = stroke_pieces(&art_brush(art, ArtScale::BetweenGuides { start: 0.0, end: 0.8 }), &line(0.0, 0.0, 300.0, 0.0), &stroke(1.0));
    let h = out[0].children().unwrap()[1].geometric_bounds().unwrap();
    assert!(near(h.x0, 280.0, 1e-6) && near(h.x1, 300.0, 1e-6), "{h:?}");
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
    let out = stroke_pieces(&art_brush(rect_art(10.0, 4.0), ArtScale::Stretch), &bp_open, &stroke(1.0));
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
    let out = stroke_pieces(&scatter_brush(100.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
    assert_eq!(out.len(), 10);
    let centres: Vec<f64> = out.iter().map(|n| n.geometric_bounds().unwrap().center().x).collect();
    for (i, c) in centres.iter().enumerate() {
        assert!(near(*c, 5.0 + 10.0 * i as f64, 1e-6), "{centres:?}");
    }
    let out = stroke_pieces(&scatter_brush(200.0), &line(0.0, 0.0, 100.0, 0.0), &stroke(1.0));
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
    let a1 = stroke_pieces(&b, &p, &stroke(1.0));
    let a2 = stroke_pieces(&b, &p, &stroke(1.0));
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
    assert_eq!(stroke_pieces(&pattern_brush(PatternFit::Stretch, false), &p, &stroke(1.0)).len(), 10);
    let p2 = line(0.0, 0.0, 104.0, 0.0);
    let st = stroke_pieces(&pattern_brush(PatternFit::Stretch, false), &p2, &stroke(1.0));
    assert_eq!(st.len(), 10);
    assert!(near(bounds(&st).width(), 104.0, 1e-6), "stretch fills the path exactly");
    let sp = stroke_pieces(&pattern_brush(PatternFit::AddSpace, false), &line(0.0, 0.0, 109.0, 0.0), &stroke(1.0));
    assert_eq!(sp.len(), 10);
    assert!(near(sp[0].geometric_bounds().unwrap().width(), 10.0, 1e-6), "add space keeps tile size");
    assert_eq!(stroke_pieces(&pattern_brush(PatternFit::Stretch, false), &p, &stroke(2.0)).len(), 5);
}

#[test]
fn pattern_places_corner_tiles() {
    let sq = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath();
    let out = stroke_pieces(&pattern_brush(PatternFit::Stretch, true), &sq, &stroke(1.0));
    let corners = out
        .iter()
        .filter(
            |n| matches!(&n.kind, NodeKind::Path { path, .. } if path.subpaths[0].anchors.len() == 4 && n.geometric_bounds().unwrap().width() < 7.0),
        )
        .count();
    assert_eq!(corners, 4);
    // Sides lose half a corner tile at each end: 94 pt per side → 9 tiles each.
    assert_eq!(out.len() - corners, 36);
}

#[test]
fn bristle_makes_translucent_strands() {
    let b = Brush { name: "b".into(), kind: BrushKind::Bristle(Bristle::default()) };
    let out = stroke_pieces(&b, &line(0.0, 0.0, 200.0, 0.0), &stroke(1.0));
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
    let out = stroke_pieces(&b, &line(0.0, 0.0, 50.0, 0.0), &stroke(1.0));
    assert_eq!(out[0].appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    let mut white = rect_art(1.0, 1.0);
    white.appearance.set_fill(Paint::solid(Color::WHITE));
    colorize(&mut white, Colorization::Tints, &Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert_eq!(white.appearance.fill_paint().color().unwrap().to_rgb(), [1.0, 1.0, 1.0]);
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
        let out = stroke_pieces(b, &bp, &stroke(1.0));
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
