//! Stroke geometry shared by the canvas, the PDF/SVG exporters and Outline Stroke, so every
//! consumer paints the same shapes:
//!
//! - [`stroke_pieces`]: the centre line (trimmed under the arrowheads) and the arrowheads as
//!   filled outlines (hollow kinds are rings), placed by [`ArrowAlign`](vectorcraft_doc::ArrowAlign);
//! - [`dash`]: the dash pattern, exact or fitted to corners and path ends; zero-length dashes
//!   become [`Dot`]s that a round or projecting cap turns into discs or squares ([`dot_outline`]);
//! - [`width_outline`]: variable-width (profile) strokes, with the stroke's joins at corners;
//! - [`line_outline`]: the line part of a stroke as one filled outline (profile, dashes, dots,
//!   caps and joins);
//! - [`for_writer`] and [`outline_region`]: strokes for the SVG/PDF writers and Outline Stroke;
//! - [`gradient_slices`] and [`gradient_meshes`]: gradients laid along or across a stroke.

mod arrow;
mod dash;
mod gradient;
mod outline;
mod width;

use std::borrow::Cow;

use kurbo::{BezPath, ParamCurve, ParamCurveNearest, PathEl, PathSeg, Shape};
use vectorcraft_doc::{LineCap, LineJoin, StrokeAlign, StrokeLayer};
pub(crate) use vectorcraft_geom::bez::{kink, segments, subpaths, tangent, unit};

pub use arrow::Arrow;
pub use dash::{Dashed, Dot, dash, dot_outline};
pub use gradient::{Slice, WrittenSlices, gradient_meshes, gradient_slices, written_slices};
pub use outline::{OUTLINE_TOL, Written, WrittenShape, for_writer, is_plain, outline_region};
pub use width::width_outline;

/// Arc-length accuracy for dashing and trimming (document points).
const ARCLEN_ACCURACY: f64 = 1e-6;

/// The geometry one stroke paints, in the coordinate space of its path.
#[derive(Clone, Debug)]
pub struct StrokePieces<'a> {
    /// The centre line to stroke: the path with its ends trimmed under the arrowheads (borrowed
    /// when nothing is trimmed). Empty when the heads cover the whole path.
    pub line: Cow<'a, BezPath>,
    /// The arrowheads, start first. Closed ends get none.
    pub heads: Vec<Arrow>,
}

/// The centre line and arrowheads of stroke `st` along `bp`.
pub fn stroke_pieces<'a>(bp: &'a BezPath, st: &StrokeLayer) -> StrokePieces<'a> {
    if st.start_arrow.is_none() && st.end_arrow.is_none() {
        return StrokePieces { line: Cow::Borrowed(bp), heads: vec![] };
    }
    arrow::pieces(bp, st)
}

/// Does `bp` end with a `ClosePath`?
pub fn is_closed(bp: &BezPath) -> bool {
    bp.elements().last().is_some_and(|e| matches!(e, PathEl::ClosePath))
}

/// The width actually stroked: inside/outside strokes of closed paths are drawn twice as wide
/// and clipped to one side; open paths always stroke centred.
pub fn aligned_width(st: &StrokeLayer, closed: bool) -> f64 {
    match st.align {
        StrokeAlign::Inside | StrokeAlign::Outside if closed => st.width * 2.0,
        _ => st.width,
    }
}

/// How far a cap reaches past the end of a stroke of `width`.
pub fn cap_extent(cap: LineCap, width: f64) -> f64 {
    match cap {
        LineCap::Butt => 0.0,
        LineCap::Round | LineCap::Square => width / 2.0,
    }
}

/// The stroke style (cap, join, miter limit) of `st` at `width`, without dashes.
pub fn style(st: &StrokeLayer, width: f64) -> kurbo::Stroke {
    kurbo::Stroke::new(width)
        .with_join(match st.join {
            LineJoin::Miter => kurbo::Join::Miter,
            LineJoin::Round => kurbo::Join::Round,
            LineJoin::Bevel => kurbo::Join::Bevel,
        })
        .with_caps(match st.cap {
            LineCap::Butt => kurbo::Cap::Butt,
            LineCap::Round => kurbo::Cap::Round,
            LineCap::Square => kurbo::Cap::Square,
        })
        .with_miter_limit(st.miter_limit)
}

/// The filled outline of the line part of `st` along `line` (usually [`StrokePieces::line`]) at
/// `width` (see [`aligned_width`]), flattened/fitted to `tol`: the dash pattern with its dots and
/// the width profile (each dash and dot as wide as the profile where it sits along the path), with
/// the stroke's caps and joins. Fill it with the non-zero rule. Arrowheads are not included.
pub fn line_outline(line: &BezPath, st: &StrokeLayer, width: f64, tol: f64) -> BezPath {
    if line.elements().is_empty() || width <= 0.0 || !width.is_finite() {
        return BezPath::new();
    }
    let tol = tol.max(1e-4);
    let dashed = st.dash.as_ref().and_then(|d| dash(line, d));
    match (st.profile.as_ref(), dashed) {
        (Some(profile), None) => width_outline(line, width, profile, st, tol),
        (Some(profile), Some(d)) => {
            let mut out = width::outline_spans(&d.path, &d.spans, width, profile, st, tol);
            for dot in &d.dots {
                // The dot spans the profile's left and right widths there.
                let (l, r) = profile.at(dot.t);
                let (l, r) = (l.max(0.0), r.max(0.0));
                let at = dot.at + width::left(dot.dir) * (width * (l - r) / 4.0);
                out.extend(dot_outline(&[Dot { at, ..*dot }], width * (l + r) / 2.0, st.cap, tol).iter());
            }
            out
        }
        (None, Some(d)) => {
            let mut out = kurbo::stroke(d.path.iter(), &style(st, width), &kurbo::StrokeOpts::default(), tol);
            out.extend(dot_outline(&d.dots, width, st.cap, tol).iter());
            out
        }
        (None, None) => {
            let mut out = plain_outline(line, &style(st, width), tol);
            out.extend(dot_outline(&point_dots(line), width, st.cap, tol).iter());
            out
        }
    }
}

/// The subpaths of `line` that never leave their first point (`m` then `L` to the same point, as
/// `.ai` files write dots, #1613), as dots: kurbo strokes nothing for them, where PDF and SVG
/// viewers paint the cap. They lie along the x axis, which a projecting cap's square follows
/// (SVG 2, "Path data": zero-length path segments).
fn point_dots(line: &BezPath) -> Vec<Dot> {
    let els = line.elements();
    subpaths(line)
        .into_iter()
        .filter_map(|(r, _)| {
            let sub = els.get(r)?;
            let at = sub.first()?.end_point()?;
            let still = sub.iter().all(|e| match e {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => *p == at,
                PathEl::QuadTo(a, b) => *a == at && *b == at,
                PathEl::CurveTo(a, b, c) => *a == at && *b == at && *c == at,
                PathEl::ClosePath => true,
            });
            still.then_some(Dot { at, dir: kurbo::Vec2::new(1.0, 0.0), t: 0.0 })
        })
        .collect()
}

/// `line` stroked with `style`, to fill with the non-zero rule.
///
/// kurbo offsets a closed subpath to both sides, as two contours. Once half the weight is more
/// than the shape can hold (its inradius), the inner one has crossed the middle and comes out
/// the far side: wound against the outer contour it would cut a hole where the pen in fact
/// overlaps itself (#1396). Such a contour is left out, so the stroke paints solid.
// shortcut: closed subpaths of plain strokes only. An open path that turns back on itself inside
// a wider stroke (and dashes on such a shape) can still cancel; fix it in the offsetting if reported.
fn plain_outline(line: &BezPath, style: &kurbo::Stroke, tol: f64) -> BezPath {
    let stroke = |els: &[PathEl]| kurbo::stroke(els.iter().copied(), style, &kurbo::StrokeOpts::default(), tol);
    let els = line.elements();
    // Only a stroke at least as wide as a closed subpath's bounds can cross over: everything
    // else takes the one call it always did.
    let may_cross = |sub: &[PathEl]| {
        let b = BezPath::from_vec(sub.to_vec()).bounding_box();
        style.width >= b.width().min(b.height())
    };
    let subs = subpaths(line);
    if !subs.iter().any(|(r, closed)| *closed && els.get(r.clone()).is_some_and(may_cross)) {
        return stroke(els);
    }
    let reach = style.width / 2.0 - 2.0 * tol;
    let mut out = BezPath::new();
    for (r, closed) in subs {
        let Some(sub) = els.get(r) else { continue };
        let outline = stroke(sub);
        if !(closed && may_cross(sub)) {
            out.extend(outline.iter());
            continue;
        }
        let sub = BezPath::from_vec(sub.to_vec());
        for (c, _) in subpaths(&outline) {
            let Some(contour) = outline.elements().get(c) else { continue };
            if reaches_edge(contour, &sub, reach) {
                out.extend(contour.iter().copied());
            }
        }
    }
    out
}

/// Whether some point of `contour` lies at least `reach` from `path`: an offset contour that
/// never does is nowhere the edge of the stroke.
fn reaches_edge(contour: &[PathEl], path: &BezPath, reach: f64) -> bool {
    let far = |p: kurbo::Point| path.segments().all(|s| s.nearest(p, 1e-6).distance_sq >= reach * reach);
    kurbo::segments(contour.iter().copied()).any(|s| far(s.eval(0.0)) || far(s.eval(0.5)))
}

/// Append `seg` to `out` (which must already have a current point at its start).
pub(crate) fn push_seg(out: &mut BezPath, seg: &PathSeg) {
    match seg {
        PathSeg::Line(l) => out.line_to(l.p1),
        PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
        PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_arrows;
#[cfg(test)]
mod tests_fit;
#[cfg(test)]
mod tests_gradient;
#[cfg(test)]
mod tests_reach;
