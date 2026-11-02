//! Operators the legacy `.ai` writer ([`crate::legacy_ai`]) and the editing data writer
//! ([`crate::ai_editing`]) both write: path points, guides and line style (Adobe Illustrator File
//! Format Specification, version 7.0, §5.3, §5.4 and §6), and the colors they write in place of
//! the paint they can't hold.

use std::fmt::Write;

use vectorcraft_color::{Color, Gradient};
use vectorcraft_doc::{LineCap, LineJoin, StrokeLayer};
use vectorcraft_geom::{Affine, Anchor, AnchorKind, PathData, Point, SubPath};

use crate::ps;

/// One subpath's points, taken into art space by `xf`: `m`, then a line or curve to each point
/// (lower case: the point is smooth, upper case: a corner, §5.3.2), and for a closed one the
/// segment back to the start, which says whether the start is smooth or a corner (as the
/// specification's examples close paths: the start point again, then the painting operator).
pub(crate) fn subpath(s: &mut String, xf: Affine, sp: &SubPath) {
    let Some(first) = sp.anchors.first() else { return };
    let pt = |s: &mut String, p: Point| {
        let p = xf * p;
        ps::push_nums(s, &[p.x, p.y]);
    };
    pt(s, first.p);
    s.push_str("m\n");
    let segment = |s: &mut String, a: &Anchor, b: &Anchor| {
        let straight = a.h_out == a.p && b.h_in == b.p;
        let smooth = b.kind == AnchorKind::Smooth;
        if straight {
            pt(s, b.p);
            s.push_str(if smooth { "l\n" } else { "L\n" });
        } else {
            pt(s, a.h_out);
            pt(s, b.h_in);
            pt(s, b.p);
            s.push_str(if smooth { "c\n" } else { "C\n" });
        }
    };
    for pair in sp.anchors.windows(2) {
        if let [a, b] = pair {
            segment(s, a, b);
        }
    }
    if sp.closed
        && sp.anchors.len() > 1
        && let Some(last) = sp.anchors.last()
    {
        segment(s, last, first);
    }
}

/// A guide (§6): for each subpath, `(N) *`, its points, `(N) *`; with `closing`, a closed
/// subpath ends `(n) *` (the painting operator that closes it first).
pub(crate) fn guide(s: &mut String, xf: Affine, path: &PathData, closing: bool) {
    for sp in path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()) {
        s.push_str("(N) *\n");
        subpath(s, xf, sp);
        s.push_str(if closing && sp.closed { "(n) *\n" } else { "(N) *\n" });
    }
}

pub(crate) fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// The average colour of a gradient's stops (in RGB).
pub(crate) fn average(g: &Gradient) -> Color {
    let n = g.stops.len().max(1) as f32;
    let sum = g.stops.iter().map(|s| s.color.to_rgb()).fold([0.0f32; 3], |a, c| [a[0] + c[0], a[1] + c[1], a[2] + c[2]]);
    Color::rgb(unit(sum[0] / n), unit(sum[1] / n), unit(sum[2] / n))
}

/// The stroke's width, cap, join, miter limit and dashes: `w J j M d`.
pub(crate) fn line_style(s: &mut String, st: &StrokeLayer) {
    let cap = match st.cap {
        LineCap::Butt => 0,
        LineCap::Round => 1,
        LineCap::Square => 2,
    };
    let join = match st.join {
        LineJoin::Miter => 0,
        LineJoin::Round => 1,
        LineJoin::Bevel => 2,
    };
    ps::push_num(s, st.width);
    let _ = write!(s, " w {cap} J {join} j ");
    ps::push_num(s, if st.miter_limit.is_finite() { st.miter_limit.max(1.0) } else { 10.0 });
    s.push_str(" M [");
    let dash = st.dash.as_ref().filter(|d| d.is_dashed());
    if let Some(d) = dash {
        ps::push_nums(s, &d.pattern);
        s.pop();
    }
    s.push_str("] ");
    ps::push_num(s, dash.map_or(0.0, |d| d.offset));
    s.push_str(" d\n");
}
