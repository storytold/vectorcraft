use super::bidi::{is_rtl, para_bidi_with, visual_order};
use super::breaking::{Metrics, optical_hangs, optical_hangs_rtl, optical_share, roman_share};
use super::flow::list_markers;
use super::region::in_range;
use super::{
    Affine, ArcPath, BezPath, Ctx, GlyphBend, Justify, LineInfo, ListMarker, ParamCurve, PathAlign, PathData, PathEffect, PathEl, Point, Range,
    SGlyph, Shape, TextObject, Vec2, style_metrics,
};
use crate::fontdb::PathAlignHeights;
use unicode_bidi::{BidiClass, BidiInfo};

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

/// Glyphs in a row, each taking its step: how far the row takes the line (`sum`), and how far past
/// where its first glyph starts the farthest of their ends is (`far`).
#[derive(Clone, Copy)]
struct Reach {
    sum: f64,
    far: f64,
}

impl Reach {
    /// No glyphs.
    const NONE: Self = Self { sum: 0.0, far: f64::NEG_INFINITY };

    /// One glyph.
    fn step(step: f64) -> Self {
        Self { sum: step, far: step }
    }

    /// The glyphs of `self` followed by those of `next`.
    fn then(self, next: Self) -> Self {
        Self { sum: self.sum + next.sum, far: self.far.max(self.sum + next.far) }
    }
}

/// The reach of the glyphs at some of `n` positions in a row (a segment tree): a glyph is put at
/// its position in O(log n), and the reach of the glyphs in a range of positions is found in O(log n).
struct Reaches {
    size: usize,
    nodes: Vec<Reach>,
}

impl Reaches {
    fn new(n: usize) -> Self {
        let size = n.next_power_of_two();
        Self { size, nodes: vec![Reach::NONE; 2 * size] }
    }

    fn node(&self, k: usize) -> Reach {
        self.nodes.get(k).copied().unwrap_or(Reach::NONE)
    }

    /// Puts a glyph that takes `step` at position `at`.
    fn put(&mut self, at: usize, step: f64) {
        let mut k = self.size + at;
        let Some(leaf) = self.nodes.get_mut(k) else { return };
        *leaf = Reach::step(step);
        while k > 1 {
            k /= 2;
            let joined = self.node(2 * k).then(self.node(2 * k + 1));
            if let Some(node) = self.nodes.get_mut(k) {
                *node = joined;
            }
        }
    }

    /// The reach of the glyphs put at positions `range`, in order.
    fn of(&self, range: Range<usize>) -> Reach {
        let (mut left, mut right) = (Reach::NONE, Reach::NONE);
        let (mut a, mut b) = (range.start.min(self.size) + self.size, range.end.min(self.size) + self.size);
        while a < b {
            if a % 2 == 1 {
                left = left.then(self.node(a));
                a += 1;
            }
            if b % 2 == 1 {
                b -= 1;
                right = self.node(b).then(right);
            }
            (a, b) = (a / 2, b / 2);
        }
        left.then(right)
    }
}

/// Whether a glyph whose character has bidi class `class` goes to its paragraph's level when the
/// line ends with it and the characters after it like it (UAX #9 L1): whitespace and isolate
/// controls, and the embedding controls and boundary neutrals, which `unicode_bidi` treats the same.
fn trails(class: BidiClass) -> bool {
    use unicode_bidi::BidiClass::{BN, FSI, LRE, LRI, LRO, PDF, PDI, RLE, RLI, RLO, WS};
    matches!(class, WS | FSI | LRI | RLI | PDI | RLE | LRE | RLO | LRO | PDF | BN)
}

/// Measures every start of the text in reading order, shortest first, in O(n log n) for n glyphs:
/// `visit(end, reach)` gets the byte each start ends at (a glyph's byte; the whole text is not
/// visited) and the reach of its glyphs in its own visual order when each glyph `g` takes `step(g)`.
///
/// The visual order of a start of a paragraph keeps the order its glyphs have in the whole
/// paragraph (UAX #9 L2 reorders by the levels between two glyphs only), except for the glyphs
/// that L1 sets to the paragraph's level at the end of the start: those come after the rest in a
/// left-to-right paragraph and before it, last first, in a right-to-left one. The glyphs are put at
/// their positions in the whole text's visual order one start after another.
fn each_start(shaped: &[(Range<usize>, Option<BidiInfo<'_>>, Vec<SGlyph>)], step: impl Fn(&SGlyph) -> f64, mut visit: impl FnMut(usize, Reach)) {
    let total = shaped.iter().map(|(_, _, glyphs)| glyphs.len()).sum();
    let mut reaches = Reaches::new(total);
    let mut last_byte = None;
    let mut para_at = 0;
    for (pr, bidi, glyphs) in shaped {
        // Where each glyph of the paragraph is in the whole text's visual order.
        let mut at = vec![para_at; glyphs.len()];
        for (v, i) in visual_order(bidi.as_ref(), pr.start, pr.clone(), glyphs).into_iter().enumerate() {
            if let Some(p) = at.get_mut(i) {
                *p = para_at + v;
            }
        }
        let para_rtl = bidi.as_ref().and_then(|b| b.paragraphs.first()).map(|p| p.level.is_rtl());
        // The glyphs at the end of the start so far that L1 sets to the paragraph's level, their
        // reach in visual order, and the reach of the cluster being added to them.
        let mut held: Vec<(usize, f64)> = Vec::new();
        let (mut trailing, mut cluster) = (Reach::NONE, Reach::NONE);
        let mut prev: Option<&SGlyph> = None;
        for (g, &p) in glyphs.iter().zip(&at) {
            // A cluster is a glyph and those after it at its byte (a list marker's apart).
            let joins = prev.is_some_and(|q| q.byte == g.byte && (q.len == 0) == (g.len == 0));
            if !joins {
                trailing = match para_rtl {
                    Some(true) => cluster.then(trailing),
                    _ => trailing.then(cluster),
                };
                cluster = Reach::NONE;
            }
            if last_byte != Some(g.byte) {
                // The start that ends before this glyph.
                let reach = match para_rtl {
                    _ if held.is_empty() => reaches.of(0..total),
                    Some(true) => reaches.of(0..para_at).then(trailing).then(reaches.of(para_at..total)),
                    _ => reaches.of(0..total).then(trailing),
                };
                visit(g.byte, reach);
                last_byte = Some(g.byte);
            }
            let s = step(g);
            // Every character of the glyph's cluster is one that trails.
            let from = g.byte.saturating_sub(pr.start);
            let classes = bidi.as_ref().and_then(|b| b.original_classes.get(from..from + g.len));
            if para_rtl.is_some() && g.len > 0 && classes.is_some_and(|c| c.iter().copied().all(trails)) {
                held.push((p, s));
                cluster = cluster.then(Reach::step(s));
            } else {
                for (q, s) in held.drain(..) {
                    reaches.put(q, s);
                }
                (trailing, cluster) = (Reach::NONE, Reach::NONE);
                reaches.put(p, s);
            }
            prev = Some(g);
        }
        // The whole paragraph is in its own visual order.
        for (q, s) in held.drain(..) {
            reaches.put(q, s);
        }
        para_at += glyphs.len();
    }
}

/// The end (a byte) of the longest start of the text, in reading order, whose glyphs fit when each
/// takes its advance: `fits(w, far)` says whether glyphs in visual order that take `w` along the
/// path, the farthest of their ends `far` past where the first starts, fit. `first` is the first
/// glyph's byte, where nothing is laid out. A start can fit where a shorter one doesn't, after a
/// glyph that takes the line back, so every start is measured ([`each_start`]).
fn longest_fitting_start(shaped: &[(Range<usize>, Option<BidiInfo<'_>>, Vec<SGlyph>)], first: usize, fits: impl Fn(f64, f64) -> bool) -> usize {
    let mut longest = first;
    each_start(
        shaped,
        |g| g.adv,
        |byte, reach| {
            if fits(reach.sum, reach.far) {
                longest = byte;
            }
        },
    );
    longest
}

/// The ends (bytes) of the starts of the text, in reading order, shortest first, that can fit on a
/// path of length `len` with `spacing`: those whose glyphs, in their own visual order, reach at most
/// `len` past where the first starts when each takes the least step it can. A glyph's step is at
/// least a quarter of its advance with spacing, and its advance otherwise ([`path_steps`]); a line
/// starts no earlier than its span with the hang at its start, and every glyph that fits ends within
/// that span with both hangs, which is no longer than the path. The start before the first glyph,
/// where nothing is laid out, is always one of them.
fn starts_that_can_fit(shaped: &[(Range<usize>, Option<BidiInfo<'_>>, Vec<SGlyph>)], spacing: f64, len: f64) -> Vec<usize> {
    let least = |g: &SGlyph| if spacing != 0.0 && g.adv > 1e-9 { g.adv * 0.25 } else { g.adv };
    let mut starts = Vec::new();
    each_start(shaped, least, |byte, reach| {
        // A little room for sums added in another order than where the glyphs are placed.
        if reach.far <= len + 1e-5 || reach.far.is_nan() {
            starts.push(byte);
        }
    });
    starts
}

/// The most starts of overflowing right-to-left type on a path that are arranged one by one, from
/// the longest, when a glyph takes the line back ([`longest_start_tried`]).
const PATH_START_TRIES: usize = 32;

/// The longest of `starts` (ends of starts, shortest first, the first of which fits) for which
/// `fits` holds, arranging at most `tries` of them one by one, from the longest. When none of those
/// fits, a binary search among the rest finds one that fits with O(log n) more arrangements; as a
/// longer start can fit where a shorter one doesn't, that one is not always the longest that fits.
/// `None` when there are no starts.
fn longest_start_tried(starts: &[usize], tries: usize, mut fits: impl FnMut(usize) -> bool) -> Option<usize> {
    let rest = starts.len().saturating_sub(tries);
    if let Some(&byte) = starts.get(rest..).unwrap_or_default().iter().rev().find(|&&byte| fits(byte)) {
        return Some(byte);
    }
    // `starts[fit]` fits; `starts[over]` was tried or doesn't.
    let (mut fit, mut over) = (0, rest);
    while over > fit + 1 {
        let mid = fit + (over - fit) / 2;
        if starts.get(mid).is_some_and(|&byte| fits(byte)) {
            fit = mid;
        } else {
            over = mid;
        }
    }
    starts.get(fit).copied()
}

#[cfg(test)]
thread_local! {
    /// How many times type on a path arranged glyphs on its path on this thread ([`path_arrangements`]).
    static ARRANGEMENTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times type on a path arranged glyphs on its path on this thread.
#[cfg(test)]
pub(crate) fn path_arrangements() -> usize {
    ARRANGEMENTS.get()
}

#[cfg(test)]
thread_local! {
    /// Set while [`layout_trying_each_start`] lays text out.
    static EACH_START: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// [`layout`](super::layout) arranging every start of overflowing right-to-left type on a path in
/// turn, from the longest, until one fits: the reference the search for the longest start is
/// compared with.
#[cfg(test)]
pub(crate) fn layout_trying_each_start(db: &crate::fontdb::FontDb, t: &TextObject) -> crate::TextLayout {
    EACH_START.set(true);
    let l = super::layout(db, t);
    EACH_START.set(false);
    l
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
        let mut glyphs = cx.shape_para(pr.clone(), bidi.as_ref(), t.para_at(pi));
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
        let s = style_metrics(cx.face_at(0).as_deref(), cx.style_at(0));
        (s.0, s.1)
    });
    // Vertical type: the caret and the selection span the column, half its em on each side of the
    // line Align to Path puts the baseline on.
    let m = if cx.vertical {
        let half = sg.iter().map(|g| g.face.units_per_em() * g.sx * 0.5).fold(f64::NAN, f64::max);
        let half = if half.is_finite() { half } else { cx.style_at(0).size * 0.5 };
        (half, half)
    } else {
        m
    };
    if ap.is_empty() {
        cx.out.overflow = !sg.is_empty();
        cx.out.lines.push(LineInfo {
            rtl,
            baseline: 0.0,
            x0: 0.0,
            x1: 0.0,
            trailing: 0.0,
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
    // Where a line that takes `w` along the path starts, aligned in the span with the hangs.
    let align = |w: f64, (l, r, _): (f64, f64, usize)| {
        let (lo, room) = (s_start - l, avail + l + r);
        match para.justify {
            Justify::Auto if rtl => lo + (room - w).max(0.0),
            Justify::Center | Justify::JustifyCenter => lo + ((room - w) * 0.5).max(0.0),
            Justify::Right | Justify::JustifyRight => lo + (room - w).max(0.0),
            _ => lo,
        }
    };
    // The glyphs `sg` arranged on the path: their steps, where the first starts, and the hangs.
    let arrange = |sg: &[SGlyph]| {
        #[cfg(test)]
        ARRANGEMENTS.set(ARRANGEMENTS.get() + 1);
        let mut steps = path_steps(&ap, sg, s_start, spacing);
        // Where the line starts when its glyphs take `steps`.
        let place = |steps: &[f64], hung| align(steps.iter().sum(), hung);
        let mut hung = hangs(sg, &steps);
        let mut s0 = place(&steps, hung);
        if spacing != 0.0 && s0 != s_start {
            // Spaced from where the alignment puts them, the curve under them is another. That
            // changes the glyphs' steps, so what the line takes and what hangs, and the line is
            // placed again from the new steps until it stays where it is (at most 8 times).
            steps = path_steps(&ap, sg, s0, spacing);
            for _ in 0..8 {
                hung = hangs(sg, &steps);
                let moved = place(&steps, hung);
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
    // What the whole text takes along the path and what the span holds: Dynamic Text's first guess.
    cx.out.path_fill = Some((steps.iter().sum(), avail + hung.0 + hung.1));
    // Where the text that fits ends: before the first glyph that doesn't, which threaded text
    // flows on from, as from the last line of area type.
    let mut fits = text_len;
    // Right-to-left text that overflows keeps the start of its text, not what comes first in visual
    // order: the longest start, in reading order, whose glyphs fit in visual order is laid out.
    if shaped.iter().any(|(_, bidi, _)| bidi.is_some()) && fitting(&steps, s0, hung) < sg.len() {
        // Whether the text before byte `cut` fits, arranged as it would be laid out.
        let fits_before = |cut: usize| {
            let sg = visual(cut);
            let (steps, s0, hung) = arrange(&sg);
            fitting(&steps, s0, hung) == sg.len()
        };
        // The byte at which each start of the text ends, in reading order.
        let mut starts: Vec<usize> = shaped.iter().flat_map(|(_, _, glyphs)| glyphs).map(|g| g.byte).collect();
        starts.dedup();
        let first = starts.first().copied().unwrap_or(text_len);
        // On one straight segment the path doesn't turn, so spacing changes no step.
        let straight = matches!(path.to_bezpath().elements(), [PathEl::MoveTo(_), PathEl::LineTo(_)]);
        #[cfg(test)]
        let each_start = EACH_START.get();
        #[cfg(not(test))]
        let each_start = false;
        fits = if each_start {
            starts.iter().rev().copied().find(|&byte| fits_before(byte)).unwrap_or(first)
        } else if (spacing == 0.0 || straight) && !hanging {
            // Each glyph takes its advance wherever the line is placed.
            longest_fitting_start(&shaped, first, |w, far| align(w, (0.0, 0.0, 0)) - s_start + far <= avail + 1e-6)
        } else if sg.iter().all(|g| g.adv >= 0.0) {
            // No glyph takes the line back: a start fits when a longer one does. The text before
            // `starts[fit]` fits; before `starts[over]` (the whole text at the end) it doesn't.
            let (mut fit, mut over) = (0, starts.len());
            while over - fit > 1 {
                let mid = (fit + over) / 2;
                if fits_before(starts.get(mid).copied().unwrap_or(text_len)) {
                    fit = mid;
                } else {
                    over = mid;
                }
            }
            starts.get(fit).copied().unwrap_or(text_len)
        } else {
            // A glyph that takes the line back can make a start fit where a shorter one doesn't:
            // the starts that can fit the path however their glyphs are spaced are tried from the
            // longest, at most `PATH_START_TRIES` of them before a binary search over the rest.
            longest_start_tried(&starts_that_can_fit(&shaped, spacing, ap.len()), PATH_START_TRIES, &fits_before).unwrap_or(first)
        };
        cx.out.overflow = true;
        sg = visual(fits);
        (steps, s0, hung) = arrange(&sg);
    }
    let fit = fitting(&steps, s0, hung);
    if fit < sg.len() {
        cx.out.overflow = true;
        fits = sg.get(fit..).and_then(|rest| rest.iter().map(|g| g.byte).min()).unwrap_or(text_len);
    }
    // Align to Path: how far down (glyph space) a glyph moves to run the top of its em box, half the
    // height of the top of its H or the bottom of its font's bounding box along the path instead of its
    // baseline, each glyph by its own face, size and vertical scale. In vertical type the glyph
    // moves half its em (with its horizontal scale) for Ascender and Descender, and stays for Center.
    // An inline graphic moves as a character of its run would, by its run's vertical scale in both.
    let vertical = cx.vertical;
    let rise_of = |g: &SGlyph| {
        let em = g.face.units_per_em() * if vertical { g.sx } else { g.sy };
        let h = if vertical { PathAlignHeights { top: 0.5, bottom: -0.5, center: 0.0 } } else { g.face.path_align_heights() };
        match t.path_align {
            PathAlign::Baseline => 0.0,
            PathAlign::Ascender => h.top * em,
            PathAlign::Descender => h.bottom * em,
            PathAlign::Center => h.center * em,
        }
    };
    let mut x = 0.0;
    for (g, &step) in sg.iter().zip(&steps).take(fit) {
        let rise = rise_of(g);
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
        trailing: 0.0,
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

#[cfg(test)]
mod tests {
    use super::longest_start_tried;

    /// The longest start that fits is found among the ones tried; past the tries, a binary search
    /// finds one that fits, with as many more tries as halvings of the rest.
    #[test]
    fn starts_are_tried_from_the_longest_then_searched() {
        let starts: Vec<usize> = (0..1000).collect();
        // Every start up to 600 fits, and so does 990.
        let fits = |byte: usize| byte <= 600 || byte == 990;
        let mut tried = 0;
        let found = longest_start_tried(&starts, 32, |byte| {
            tried += 1;
            fits(byte)
        });
        assert_eq!((found, tried), (Some(990), 10));
        tried = 0;
        let found = longest_start_tried(&starts, 8, |byte| {
            tried += 1;
            fits(byte)
        });
        assert_eq!(found, Some(600));
        assert!(tried <= 8 + 10, "{tried} tries");
        // With fewer starts than tries, each is tried.
        assert_eq!(longest_start_tried(&starts[..5], 32, |byte| byte == 0), Some(0));
        assert_eq!(longest_start_tried(&[], 32, |_| true), None);
    }
}
