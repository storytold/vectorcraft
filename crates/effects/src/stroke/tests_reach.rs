//! The document's stroke bounds and clicks (`vectorcraft_doc` reach) against the geometry the
//! stroke really paints: every painted point is inside the visual bounds and hits.

use kurbo::{BezPath, Point, Rect, Shape};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{ArrowAlign, Arrowhead, Dash, LineCap, LineJoin, StrokeAlign, StrokeLayer, WidthProfile};
use vectorcraft_geom::hit::fill_contains;
use vectorcraft_geom::{FillRule, PathData};

use super::outline_region;

fn paths() -> Vec<BezPath> {
    let mut curve = BezPath::new();
    curve.move_to((10.0, 60.0));
    curve.curve_to((30.0, 0.0), (70.0, 120.0), (90.0, 40.0));
    let mut zigzag = BezPath::new();
    zigzag.move_to((10.0, 20.0));
    zigzag.line_to((40.0, 70.0));
    zigzag.line_to((55.0, 15.0));
    zigzag.line_to((90.0, 60.0));
    let mut tri = BezPath::new();
    tri.move_to((10.0, 10.0));
    tri.line_to((90.0, 30.0));
    tri.line_to((10.0, 50.0));
    tri.close_path();
    vec![curve, zigzag, tri, Rect::new(20.0, 20.0, 80.0, 70.0).to_path(0.1)]
}

/// Check `st` along `bp`: the painted region's box is inside the bounds, and points of the
/// region (on a 1.5 pt grid) hit within a quarter point, as the painted outlines are flattened.
/// Variable widths are drawn straight between their offsets at the path's samples and inner
/// corners, which beside sharp corners can paint up to about a point past the exact width there.
fn check(bp: &BezPath, st: &StrokeLayer, what: &str) {
    let region = outline_region(&PathData::from_bezpath(bp), FillRule::NonZero, st).to_bezpath();
    if region.elements().is_empty() {
        return;
    }
    let painted = region.bounding_box();
    let bounds = st.bounds_on(bp, bp.bounding_box());
    assert!(bounds.inflate(0.05, 0.05).contains_rect(painted), "{what}: paints {painted:?} outside {bounds:?}");
    let tol = if st.profile.is_some() { 1.0 } else { 0.25 };
    // Off the outline's own coordinates, so no sample sits level with a vertex.
    let (mut x, mut checked) = (painted.x0 + 0.371, 0);
    while x <= painted.x1 {
        let mut y = painted.y0 + 0.293;
        while y <= painted.y1 {
            let p = Point::new(x, y);
            if fill_contains(&region, FillRule::EvenOdd, p) {
                assert!(st.hits(bp, FillRule::NonZero, p, tol), "{what}: painted {p:?} doesn't hit");
                checked += 1;
            }
            y += 1.5;
        }
        x += 1.5;
    }
    assert!(checked > 0, "{what}: no samples");
}

#[test]
fn a_butt_capped_open_path_is_measured_where_it_paints() {
    let mut vertical = BezPath::new();
    vertical.move_to((50.0, 10.0));
    vertical.line_to((50.0, 90.0));
    let mut diagonal = BezPath::new();
    diagonal.move_to((10.0, 10.0));
    diagonal.line_to((40.0, 50.0));
    let mut arc = BezPath::new();
    arc.move_to((10.0, 50.0));
    arc.curve_to((10.0, 22.4), (32.4, 0.0), (60.0, 0.0));
    let mut stroke = StrokeLayer::new(Paint::solid(Color::BLACK), 4.0);
    stroke.cap = LineCap::Butt;
    for (name, bp) in [("vertical", vertical), ("diagonal", diagonal), ("arc", arc)] {
        let painted = outline_region(&PathData::from_bezpath(&bp), FillRule::NonZero, &stroke).to_bezpath().bounding_box();
        let bounds = stroke.bounds_on(&bp, bp.bounding_box());
        for (got, want) in [(bounds.x0, painted.x0), (bounds.y0, painted.y0), (bounds.x1, painted.x1), (bounds.y1, painted.y1)] {
            assert!((got - want).abs() < 0.05, "{name}: measured {bounds:?}, paints {painted:?}");
        }
    }
}

#[test]
fn beveled_corners_are_measured_where_they_paint() {
    let mut tip = BezPath::new();
    tip.move_to((0.0, 100.0));
    tip.line_to((20.0, 0.0));
    tip.line_to((40.0, 100.0));
    tip.close_path();
    let mut chevron = BezPath::new();
    chevron.move_to((0.0, 100.0));
    chevron.line_to((20.0, 0.0));
    chevron.line_to((40.0, 100.0));
    // A rectangle with no width turns back at its top and bottom.
    let flat = vectorcraft_geom::shapes::rectangle(Rect::new(3.0, 100.0, 3.0, 150.0)).to_bezpath();
    let ps = paths();
    for (name, bp) in [("tip", &tip), ("chevron", &chevron), ("zigzag", &ps[1]), ("triangle", &ps[2]), ("flat", &flat)] {
        for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
            for (join, limit) in [(LineJoin::Bevel, 10.0), (LineJoin::Miter, 1.0), (LineJoin::Miter, 2.0), (LineJoin::Miter, 10.0)] {
                let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 20.0);
                (st.cap, st.join, st.miter_limit) = (cap, join, limit);
                let painted = outline_region(&PathData::from_bezpath(bp), FillRule::NonZero, &st).to_bezpath().bounding_box();
                let bounds = st.bounds_on(bp, bp.bounding_box());
                for (got, want) in [(bounds.x0, painted.x0), (bounds.y0, painted.y0), (bounds.x1, painted.x1), (bounds.y1, painted.y1)] {
                    assert!((got - want).abs() < 0.05, "{name} {cap:?} {join:?} {limit}: measured {bounds:?}, paints {painted:?}");
                }
            }
        }
    }
}

#[test]
fn bounds_and_clicks_cover_what_every_stroke_paints() {
    let profiles = [None, Some(WidthProfile::lens()), Some(WidthProfile { points: vec![(0.0, 2.0, 0.5), (0.6, 0.8, 1.6), (1.0, 1.2, 0.3)] })];
    for (pi, bp) in paths().iter().enumerate() {
        for align in [StrokeAlign::Center, StrokeAlign::Inside, StrokeAlign::Outside] {
            for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
                for (join, limit) in [(LineJoin::Miter, 10.0), (LineJoin::Miter, 2.0), (LineJoin::Round, 10.0), (LineJoin::Bevel, 10.0)] {
                    for (fi, profile) in profiles.iter().enumerate() {
                        for dash in [None, Some(Dash { pattern: vec![7.0, 4.0], offset: 0.0, align_corners: false })] {
                            let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 6.0);
                            (st.align, st.cap, st.join, st.miter_limit, st.profile, st.dash) =
                                (align, cap, join, limit, profile.clone(), dash.clone());
                            check(bp, &st, &format!("path {pi} {align:?} {cap:?} {join:?} {limit} profile {fi} dashed {}", dash.is_some()));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn bounds_and_clicks_cover_every_arrowhead() {
    let bp = &paths()[0];
    for kind in Arrowhead::ALL {
        for align in [ArrowAlign::Extend, ArrowAlign::Tip] {
            for (scale, cap) in [(50.0, LineCap::Round), (100.0, LineCap::Butt), (300.0, LineCap::Square)] {
                let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 3.0);
                (st.start_arrow, st.end_arrow, st.arrow_align, st.arrow_scale, st.cap) = (Some(kind), Some(kind), align, (scale, scale), cap);
                check(bp, &st, &format!("{kind:?} {align:?} {scale}%"));
            }
        }
    }
}
