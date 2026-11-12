//! Operators the legacy `.ai` writer ([`crate::legacy_ai`]) and the editing data writer
//! ([`crate::ai_editing`]) both write: path points, guides and line style (Adobe Illustrator File
//! Format Specification, version 7.0, §5.3, §5.4 and §6), and the colors they write in place of
//! the paint they can't hold.

use std::fmt::Write;

use vectorcraft_color::{Color, Gradient};
use vectorcraft_doc::{LAYER_COLORS, LayerColor, LineCap, LineJoin, StrokeLayer};
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

/// A guide (§6): for each subpath, `(N) *`, its points, `(N) *`. In `editing` data (see
/// [`crate::ai_editing`]) a subpath is its points and `(N) *` alone, `(n) *` when closed (the
/// painting operator that closes it first).
pub(crate) fn guide(s: &mut String, xf: Affine, path: &PathData, editing: bool) {
    for sp in path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()) {
        if !editing {
            s.push_str("(N) *\n");
        }
        subpath(s, xf, sp);
        s.push_str(if editing && sp.closed { "(n) *\n" } else { "(N) *\n" });
    }
}

/// The colour index an `.ai` layer's `Lb` gives preset `preset` (its position in
/// [`LAYER_COLORS`]). The file numbers Magenta, Cyan and Yellow 4, 5 and 6, where
/// [`LAYER_COLORS`] has Yellow, Magenta and Cyan; every other preset has its own position (#6502,
/// from the index and RGB pairs of the `Lb` records in files users sent). A reader takes the index
/// and ignores the RGB beside it, so the index decides the colour.
pub(crate) fn ai_layer_index(preset: u8) -> u8 {
    match preset {
        4 => 6,
        5 => 4,
        6 => 5,
        i => i,
    }
}

/// The preset (position in [`LAYER_COLORS`]) of the colour index `index` in an `.ai` layer's
/// `Lb`: the inverse of [`ai_layer_index`].
pub(crate) fn preset_of_ai_layer_index(index: u8) -> u8 {
    match index {
        6 => 4,
        4 => 5,
        5 => 6,
        i => i,
    }
}

/// The colour index and RGB an `.ai` layer's `Lb` writes for `color`: a preset's index
/// ([`ai_layer_index`]) with its RGB, or `-1` with a custom colour's RGB.
pub(crate) fn layer_color(color: LayerColor) -> (i64, [u8; 3]) {
    match color {
        LayerColor::Preset(i) => match LAYER_COLORS.get(usize::from(i)) {
            Some((_, rgb)) => (i64::from(ai_layer_index(i)), *rgb),
            None => (-1, [79, 128, 255]),
        },
        LayerColor::Custom(rgb) => (-1, rgb),
    }
}

pub(crate) fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// `color` as a reader reads it back from the values written for it ([`ps::written`]): RGB values
/// clamped to 0..1, L to 0..100 and a and b to -128..127, each to four decimals; other colors
/// unchanged.
pub(crate) fn as_written(color: Color) -> Color {
    let w = |v: f32| ps::written(f64::from(v)) as f32;
    match color {
        Color::Rgb { r, g, b } => Color::rgb(w(unit(r)), w(unit(g)), w(unit(b))),
        Color::Lab { l, a, b } => Color::lab(w(l.clamp(0.0, 100.0)), w(a.clamp(-128.0, 127.0)), w(b.clamp(-128.0, 127.0))),
        c => c,
    }
}

/// The RGB values written for `color`, clamped to 0..1: an RGB color's own values (in the working
/// RGB space, as a reader reads them back), any other color's display RGB.
pub(crate) fn rgb_values(color: Color) -> [f32; 3] {
    match color {
        Color::Rgb { r, g, b } => [r, g, b].map(unit),
        c => c.to_rgb().map(unit),
    }
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
