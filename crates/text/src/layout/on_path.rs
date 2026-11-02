use super::bidi::{is_rtl, para_bidi_with, visual_order};
use super::breaking::{Metrics, optical_hangs, optical_hangs_rtl, optical_share, roman_share};
use super::flow::list_markers;
use super::region::in_range;
use super::{
    Affine, ArcPath, BezPath, Ctx, GlyphBend, Justify, LineInfo, ListMarker, ParamCurve, PathAlign, PathData, PathEffect, PathEl, Point, Range,
    SGlyph, Shape, TextObject, Vec2, style_metrics,
};

/// The distance along type on a path each glyph takes: its advance measured `spacing` points above
/// the path (Type on a Path Options › Spacing), so glyphs close up round the outside of a curve and
/// open up round the inside; just the advances without spacing. Glyphs start `from` along the path.
fn path_steps(ap: &ArcPath, glyphs: &[SGlyph], from: f64, spacing: f64) -> Vec<f64> {
    let mut s = from;
    glyphs
        .iter()
        .map(|g| {
            let mut step = g.adv;
            if spacing != 0.0 && g.adv > 1e-9 {
                // How far the path turns across the glyph: positive bending away from its top.
                let ((_, a), (_, b)) = (ap.at(s), ap.at(s + g.adv));
                let curvature = a.cross(b).atan2(a.dot(b)) / g.adv;
                step = g.adv / (1.0 + curvature * spacing).clamp(0.25, 4.0);
            }
            s += step;
            step
        })
        .collect()
}

/// How closely a bent glyph's curves follow the path (text space units).
const BEND_TOLERANCE: f64 = 0.01;

/// The most pieces one segment of a glyph's outline is cut into to bend it.
const BEND_PIECES: usize = 32;

/// The most elements a bent glyph outline may have; beyond that the glyph stays straight.
const BEND_MAX_ELEMENTS: usize = 8192;

/// `outline` (text space, placed straight by `pre`, which takes the glyph's pen space there) bent
/// along `ap` as `bend` says ([`GlyphBend`]); `None` where the path doesn't turn under the glyph
/// (it stays as it is, exactly), or the bent outline would be too large or not finite.
fn bend_outline(outline: &BezPath, pre: Affine, ap: &ArcPath, bend: &GlyphBend) -> Option<BezPath> {
    let GlyphBend { s, step, advance, rise } = *bend;
    if outline.elements().is_empty() || !(advance > 1e-9 && step.is_finite() && step > 0.0 && s.is_finite() && rise.is_finite()) {
        return None;
    }
    // How far the path turns under the glyph; none: it lies straight already.
    let ((_, a), (_, b)) = (ap.at(s), ap.at(s + step));
    let turn = a.cross(b).atan2(a.dot(b)).abs();
    if !(turn.is_finite() && turn > 1e-9) {
        return None;
    }
    let det = pre.determinant();
    if !(det.is_finite() && det.abs() > 1e-12) {
        return None;
    }
    let to_pen = pre.inverse();
    let k = step / advance;
    let map = |q: Point| {
        let (p, d) = ap.at(s + q.x * k);
        p + Vec2::new(-d.y, d.x) * (q.y + rise)
    };
    // Pieces short enough that mapping a piece's control points stays within the tolerance: the
    // error of a piece of length L under curvature c, at height y, is about L² c (1 + |y| c) / 8.
    let curvature = turn / step;
    let pen = to_pen * outline.clone();
    let height = pen.bounding_box().y0.abs().max(pen.bounding_box().y1.abs()) + rise.abs();
    let piece = ((8.0 * BEND_TOLERANCE) / (curvature * (1.0 + height * curvature))).sqrt();
    let pieces = |a: Point, b: Point| -> usize {
        let n = ((b - a).hypot() / piece).ceil();
        if n.is_finite() { (n as usize).clamp(1, BEND_PIECES) } else { BEND_PIECES }
    };
    let mut out = BezPath::new();
    let (mut start, mut cur) = (Point::ZERO, Point::ZERO);
    let line = |out: &mut BezPath, from: Point, to: Point| {
        let n = pieces(from, to);
        for i in 1..=n {
            out.line_to(map(from.lerp(to, i as f64 / n as f64)));
        }
    };
    for el in pen.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                out.move_to(map(p));
                (start, cur) = (p, p);
            }
            PathEl::LineTo(p) => {
                line(&mut out, cur, p);
                cur = p;
            }
            PathEl::QuadTo(p1, p2) => {
                let c = kurbo::QuadBez::new(cur, p1, p2).raise();
                let n = pieces(cur, p1).max(pieces(p1, p2));
                for i in 0..n {
                    let q = c.subsegment(i as f64 / n as f64..(i + 1) as f64 / n as f64);
                    out.curve_to(map(q.p1), map(q.p2), map(q.p3));
                }
                cur = p2;
            }
            PathEl::CurveTo(p1, p2, p3) => {
                let c = kurbo::CubicBez::new(cur, p1, p2, p3);
                let n = pieces(cur, p1).max(pieces(p1, p2)).max(pieces(p2, p3));
                for i in 0..n {
                    let q = c.subsegment(i as f64 / n as f64..(i + 1) as f64 / n as f64);
                    out.curve_to(map(q.p1), map(q.p2), map(q.p3));
                }
                cur = p3;
            }
            PathEl::ClosePath => {
                if (cur - start).hypot() > 1e-12 {
                    line(&mut out, cur, start);
                }
                out.close_path();
                cur = start;
            }
        }
        if out.elements().len() > BEND_MAX_ELEMENTS {
            return None;
        }
    }
    let finite = |p: Point| p.x.is_finite() && p.y.is_finite();
    out.elements()
        .iter()
        .all(|el| match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => finite(p),
            PathEl::QuadTo(a, b) => finite(a) && finite(b),
            PathEl::CurveTo(a, b, c) => finite(a) && finite(b) && finite(c),
            PathEl::ClosePath => true,
        })
        .then_some(out)
}

/// Lay type on a path out along `path` (text space), between its brackets.
pub(super) fn on_path(cx: &mut Ctx<'_>, paras: &[Range<usize>], t: &TextObject, path: &PathData) {
    cx.out.on_path = true;
    // Type on a path is one line: the first paragraph's attributes align it.
    let para = t.para_at(0);
    // The first paragraph's direction aligns the line (Auto) and sets the caret's.
    let mut rtl = None;
    let mut shaped = Vec::new();
    let markers = list_markers(t, paras);
    for (pi, pr) in paras.iter().enumerate() {
        let text = cx.text;
        let bidi = para_bidi_with(text.get(pr.clone()).unwrap_or_default(), para.direction, &cx.char_directions(pr));
        rtl.get_or_insert(is_rtl(bidi.as_ref()) && !cx.vertical);
        let mut glyphs = cx.shape_para(pr.clone(), bidi.as_ref());
        // Bullets and numbers: a list paragraph's text starts after its marker here too, unless
        // the paragraph continues from the frame before in its thread.
        if pi > 0 || !t.first_para_continues {
            let marker_text = markers.get(pi).and_then(Option::as_deref);
            cx.prepend_marker(pr, t.para_at(pi), marker_text, is_rtl(bidi.as_ref()) && !cx.vertical, &mut glyphs);
        }
        shaped.push((pr.clone(), bidi.filter(|b| b.has_rtl() && !cx.vertical), glyphs));
    }
    let rtl = rtl.unwrap_or(false);
    // The text before byte `cut` in visual order, as area type reorders each line it breaks.
    let visual = |cut: usize| {
        let mut sg = Vec::new();
        for (pr, bidi, glyphs) in &shaped {
            let n = glyphs.iter().take_while(|g| g.byte < cut).count();
            let line = glyphs.get(..n).unwrap_or_default();
            let end = glyphs.get(n).map_or(pr.end, |g| g.byte);
            sg.extend(visual_order(bidi.as_ref(), pr.start, pr.start..end, line).into_iter().filter_map(|i| line.get(i).cloned()));
        }
        sg
    };
    let text_len = cx.text.len();
    let mut sg = visual(text_len);
    // A path out of range holds no text, as a path with no length holds none.
    let ap = if in_range(&path.to_bezpath()) { ArcPath::new(path) } else { ArcPath::default() };
    let m = Metrics::max(&sg).map(|m| (m.asc, m.desc)).unwrap_or_else(|| {
        let s = style_metrics(cx.db, cx.style_at(0));
        (s.0, s.1)
    });
    if ap.is_empty() {
        cx.out.overflow = !sg.is_empty();
        cx.out.lines.push(LineInfo {
            rtl,
            baseline: 0.0,
            x0: 0.0,
            x1: 0.0,
            ascent: m.0,
            descent: m.1,
            first_baseline: 0.0,
            start: 0,
            end: 0,
            glyph_start: 0,
            glyph_end: 0,
            avail: (0.0, 0.0),
            region: 0,
        });
        return;
    }
    let centre = path.bounds().unwrap_or_default().center();
    let (from, to) = t.kind.path_span().unwrap_or((0.0, 1.0));
    let s_start = from * ap.len();
    let avail = (to - from) * ap.len();
    let spacing = if t.path_spacing.is_finite() { t.path_spacing } else { 0.0 };
    // Optical Margin Alignment and Roman Hanging Punctuation: the brackets are the margins, and the
    // glyphs at the line's ends hang outside them by `hang_l` and `hang_r`, measured along the path.
    // The line is aligned in a span that much longer at each end, and the glyphs that hang at the
    // end (`hung_r` of them) do not make it overflow past the end bracket. On an open path a hang
    // stops at the path's end. On a closed path the two hangs together take at most the stretch
    // between the end and start brackets: the end where the line is placed takes its share first.
    // A centered line's ends each get up to half of it, and either end takes what the other leaves.
    // The hangs are measured on the steps the glyphs take where the line is placed. A right-to-left
    // line is read from right to left ([`optical_hangs_rtl`]); in vertical type `sg` is in text order.
    let hanging = t.optical_margin || para.roman_hanging;
    let hangs = |sg: &[SGlyph], steps: &[f64]| {
        if !hanging {
            return (0.0, 0.0, 0);
        }
        let share: fn(&SGlyph) -> f64 = if t.optical_margin { optical_share } else { roman_share };
        // A list marker (the glyphs of length 0) does not hang: it is counted with no width.
        let line: Vec<(&SGlyph, f64)> = sg.iter().zip(steps).map(|(g, &step)| (g, if g.len == 0 { 0.0 } else { step })).collect();
        let (l, r, _, hung_r) = if rtl { optical_hangs_rtl(&line, share) } else { optical_hangs(&line, share) };
        let (l, r) = if ap.is_closed() {
            let gap = (ap.len() - avail).max(0.0);
            match para.justify {
                j if matches!(j, Justify::Right | Justify::JustifyRight) || (rtl && j == Justify::Auto) => {
                    let r = r.min(gap);
                    (l.min(gap - r), r)
                }
                Justify::Center | Justify::JustifyCenter => {
                    let l = l.min(gap - r.min(gap * 0.5));
                    (l, r.min(gap - l))
                }
                _ => {
                    let l = l.min(gap);
                    (l, r.min(gap - l))
                }
            }
        } else {
            (l.min(s_start), r.min((ap.len() - s_start - avail).max(0.0)))
        };
        (l, r, hung_r)
    };
    // The glyphs `sg` arranged on the path: their steps, where the first starts, and the hangs.
    let arrange = |sg: &[SGlyph]| {
        let mut steps = path_steps(&ap, sg, s_start, spacing);
        let w: f64 = steps.iter().sum();
        let place = |(l, r, _): (f64, f64, usize)| {
            let (lo, room) = (s_start - l, avail + l + r);
            match para.justify {
                Justify::Auto if rtl => lo + (room - w).max(0.0),
                Justify::Center | Justify::JustifyCenter => lo + ((room - w) * 0.5).max(0.0),
                Justify::Right | Justify::JustifyRight => lo + (room - w).max(0.0),
                _ => lo,
            }
        };
        let mut hung = hangs(sg, &steps);
        let mut s0 = place(hung);
        if spacing != 0.0 && s0 != s_start {
            // Spaced from where the alignment puts them, the curve under them is another. That
            // changes the hanging glyphs' steps, so the line is placed again from the new steps
            // until it stays where it is (at most 8 times).
            steps = path_steps(&ap, sg, s0, spacing);
            for _ in 0..8 {
                hung = hangs(sg, &steps);
                let moved = place(hung);
                if (moved - s0).abs() < 1e-9 {
                    break;
                }
                s0 = moved;
                steps = path_steps(&ap, sg, s0, spacing);
            }
        }
        (steps, s0, hung)
    };
    // How many of the arranged glyphs fit before one runs past the end bracket (with the hang).
    let fitting = |steps: &[f64], s0: f64, (hang_l, hang_r, hung_r): (f64, f64, usize)| {
        let (lo, room) = (s_start - hang_l, avail + hang_l + hang_r);
        let mut x = 0.0;
        steps
            .iter()
            .enumerate()
            .take_while(|&(k, &step)| {
                let end = if k + hung_r >= steps.len() { room } else { room - hang_r };
                let fit = s0 + x + step - lo <= end + 1e-6;
                x += step;
                fit
            })
            .count()
    };
    let (mut steps, mut s0, mut hung) = arrange(&sg);
    // Where the text that fits ends: before the first glyph that doesn't, which threaded text
    // flows on from, as from the last line of area type.
    let mut fits = text_len;
    // Right-to-left text that overflows keeps the start of its text, not what comes first in visual
    // order: the longest start, in reading order, whose glyphs fit in visual order is laid out.
    if shaped.iter().any(|(_, bidi, _)| bidi.is_some()) && fitting(&steps, s0, hung) < sg.len() {
        let mut starts: Vec<usize> = shaped.iter().flat_map(|(_, _, glyphs)| glyphs.iter().map(|g| g.byte)).collect();
        starts.dedup();
        // The text before `starts[fit]` fits; before `starts[over]` (the whole text at the end) it
        // doesn't.
        let (mut fit, mut over) = (0, starts.len());
        while over - fit > 1 {
            let mid = (fit + over) / 2;
            let sg = visual(starts.get(mid).copied().unwrap_or(text_len));
            let (steps, s0, hung) = arrange(&sg);
            if fitting(&steps, s0, hung) == sg.len() {
                fit = mid;
            } else {
                over = mid;
            }
        }
        cx.out.overflow = true;
        fits = starts.get(fit).copied().unwrap_or(text_len);
        sg = visual(fits);
        (steps, s0, hung) = arrange(&sg);
    }
    let fit = fitting(&steps, s0, hung);
    if fit < sg.len() {
        cx.out.overflow = true;
        fits = sg.get(fit..).and_then(|rest| rest.iter().map(|g| g.byte).min()).unwrap_or(text_len);
    }
    // Align to Path: how far down (glyph space) the type moves to run its ascender, centre or
    // descender along the path instead of its baseline.
    let rise = match t.path_align {
        PathAlign::Baseline => 0.0,
        PathAlign::Ascender => m.0,
        PathAlign::Descender => -m.1,
        PathAlign::Center => (m.0 - m.1) * 0.5,
    };
    let mut x = 0.0;
    for (g, &step) in sg.iter().zip(&steps).take(fit) {
        let s = s0 + x;
        let (p, dir) = ap.at(s + step * 0.5);
        let angle = dir.y.atan2(dir.x);
        let half = Affine::translate((-g.adv * 0.5, rise));
        // Glyph space: x along the advance, y down from the baseline; `pre` maps it onto the path.
        let pre = match t.path_effect {
            PathEffect::Rainbow => Affine::translate(p.to_vec2()) * Affine::rotate(angle) * half,
            // x axis along the tangent, y axis stays vertical.
            PathEffect::Skew => Affine::translate(p.to_vec2()) * Affine::new([dir.x, dir.y, 0.0, 1.0, 0.0, 0.0]) * half,
            // x axis stays horizontal (facing the path's direction), y axis perpendicular to the path.
            PathEffect::Ribbon3d => {
                let sx = if dir.x < 0.0 { -1.0 } else { 1.0 };
                Affine::translate(p.to_vec2()) * Affine::new([sx, 0.0, -dir.y * sx, dir.x * sx, 0.0, 0.0]) * half
            }
            PathEffect::StairStep => Affine::translate(ap.at(s).0.to_vec2()) * Affine::translate((0.0, rise)),
            // x axis along the tangent; vertical edges point at the path's centre (kept on the glyph's
            // up side, and never closer than ~17° to the baseline so glyphs stay legible).
            PathEffect::Gravity => {
                let n = Vec2::new(dir.y, -dir.x);
                let mut up = p - centre;
                up = if up.hypot() < 1e-9 { n } else { up / up.hypot() };
                if up.dot(n) < 0.0 {
                    up = -up;
                }
                if up.dot(n) < 0.3 {
                    let along = up - n * up.dot(n);
                    up = n * 0.3 + along / along.hypot().max(1e-9) * (1.0 - 0.09f64).sqrt();
                }
                Affine::translate(p.to_vec2()) * Affine::new([dir.x, dir.y, -up.x, -up.y, 0.0, 0.0]) * half
            }
        };
        // The caret's baseline moves with the type.
        let origin = p - dir * (g.adv * 0.5) + Vec2::new(-dir.y, dir.x) * rise;
        // A list marker's glyphs (the only glyphs here with no length) are in a row at the byte of
        // their paragraph's start.
        if g.len == 0 {
            let at = cx.out.glyphs.len();
            let starts_paragraph = |b: usize| paras.iter().position(|p| p.start == b);
            match cx.out.markers.last_mut() {
                Some(m) if m.glyphs.end == at && cx.out.glyphs.get(m.glyphs.start).is_some_and(|h| h.byte == g.byte) => m.glyphs.end = at + 1,
                _ => {
                    if let Some(text) = starts_paragraph(g.byte).and_then(|pi| markers.get(pi)).and_then(Option::as_ref) {
                        cx.out.markers.push(ListMarker { glyphs: at..at + 1, tab: at, text: text.clone() });
                    }
                }
            }
            if let Some(m) = cx.out.markers.last_mut().filter(|m| m.glyphs.end == at + 1 && g.ch == '\t') {
                m.tab = at;
            }
        }
        cx.emit(g, pre, origin, angle, g.adv, 0);
        // A glyph of a script whose letters join is bent along a curved path (Rainbow), so its
        // joining strokes meet its neighbours' instead of standing off the curve.
        if t.path_effect == PathEffect::Rainbow && g.cursive && !cx.vertical && g.inline.is_none() {
            let bend = GlyphBend { s, step, advance: g.adv, rise };
            if let Some(last) = cx.out.glyphs.last_mut()
                && last.image.is_none()
                && last.colour.is_none()
                && let Some(bent) = bend_outline(&last.outline, pre, &ap, &bend)
            {
                last.outline = bent;
                last.bend = Some(bend);
            }
        }
        x += step;
    }
    let (ps, _) = ap.at(s0);
    let (pe, _) = ap.at(s0 + x);
    cx.out.lines.push(LineInfo {
        rtl,
        baseline: ps.y,
        x0: ps.x,
        x1: pe.x,
        ascent: m.0,
        descent: m.1,
        first_baseline: 0.0,
        start: 0,
        end: fits,
        glyph_start: 0,
        glyph_end: cx.out.glyphs.len(),
        avail: (0.0, ap.len()),
        region: 0,
    });
}
