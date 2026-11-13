use vectorcraft_doc::{CharStyle, TextKind, TextObject, WarichuAlign};

use super::SGlyph;
use crate::shape::Wari;

/// What [`warichu`] reads from a glyph's character style ([`CharStyle::warichu`] and Warichu
/// Settings): whether it is marked, its Lines, Line Gap (points) and Scale (percent), its
/// Alignment, the size it is set at (already scaled by [`warichu_sizes`]) and whether its leading
/// is Auto.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WariStyle {
    pub marked: bool,
    pub lines: u8,
    pub gap: f64,
    pub scale: f64,
    pub align: WarichuAlign,
    pub size: f64,
    pub auto_leading: bool,
}

impl WariStyle {
    pub(super) fn of(st: &CharStyle) -> Self {
        Self {
            marked: st.warichu,
            lines: st.warichu_lines,
            gap: st.warichu_gap,
            scale: st.warichu_scale,
            align: st.warichu_align,
            size: st.size,
            auto_leading: st.leading.is_none(),
        }
    }
    /// Do glyphs in styles `self` and `other` belong to one block: both marked, with the same
    /// settings and size, and both with Auto leading or both without?
    fn same_block(&self, other: &Self) -> bool {
        self.marked
            && other.marked
            && self.lines == other.lines
            && self.gap == other.gap
            && self.scale == other.scale
            && self.align == other.align
            && self.size == other.size
            && self.auto_leading == other.auto_leading
    }
}

/// Warichu Settings Scale as a factor of the size, kept to the range Warichu Settings takes
/// ([`vectorcraft_doc::text::WARICHU_SCALE`]); 50% when it is not a number.
fn scale_factor(percent: f64) -> f64 {
    let range = vectorcraft_doc::text::WARICHU_SCALE;
    if percent.is_nan() { 0.5 } else { percent.clamp(*range.start(), *range.end()) / 100.0 }
}

/// Are characters in style `st` set as warichu (in `vertical` type or not)? In vertical type a
/// character marked Tate-chu-yoko too is set across the column at its own size instead.
pub(super) fn stacked(st: &CharStyle, vertical: bool) -> bool {
    st.warichu && !(vertical && st.tate_chu_yoko)
}

/// `t` with each warichu run set at its Warichu Settings Scale of its size. An explicit leading
/// stays in points, and [`place_block`] and the paragraph's Auto Leading give an Auto leading the
/// run's own size ([`line_size`]), so that the block's line keeps its height. `t` itself when no
/// run is marked, and for type on a path, which sets warichu characters at their own size.
pub(super) fn warichu_sizes(t: &TextObject) -> std::borrow::Cow<'_, TextObject> {
    let stacked = |st: &CharStyle| stacked(st, t.vertical);
    if matches!(t.kind, TextKind::OnPath { .. }) || !t.runs.iter().any(|r| stacked(&r.style)) {
        return std::borrow::Cow::Borrowed(t);
    }
    let mut out = t.clone();
    for r in out.runs.iter_mut().filter(|r| stacked(&r.style)) {
        r.style.size *= scale_factor(r.style.warichu_scale);
    }
    std::borrow::Cow::Owned(out)
}

/// The size of the line characters in style `st` sit in, from the styles [`warichu_sizes`] sets
/// (in `vertical` type or not): a warichu character's size without its Warichu Settings Scale, any
/// other character's own size. For flowed type, not type on a path.
pub(super) fn line_size(st: &CharStyle, vertical: bool) -> f64 {
    if stacked(st, vertical) { WariStyle::of(st).parent_size() } else { st.size }
}

impl WariStyle {
    /// The size a warichu block's text has without its Warichu Settings Scale: the size of the
    /// line it sits in.
    pub(super) fn parent_size(&self) -> f64 {
        self.size / scale_factor(self.scale)
    }
}

/// Warichu: each stretch of marked glyphs `g` (shaped at their Warichu Settings Scale of their
/// size, [`warichu_sizes`]) is set as one block of its Lines inside its line. The block takes the
/// length of its longest line along the line and does not break across lines; its first glyph
/// advances by that length and the others by none, as a tate-chu-yoko block's do, and every glyph
/// keeps its place in the block ([`Wari`]). A stretch takes the settings of its first glyph, and a
/// glyph with other settings, another size or the other kind of leading (Auto or a number) starts
/// a new block. A tab, a forced line break and a tate-chu-yoko glyph are not part of a block and
/// end it; digits marked Warichu are not set as automatic tate-chu-yoko. In horizontal type each
/// line's characters are set in their bidi order, so right-to-left text reads from the right. The
/// Insert Aki before the block's first character and after its last is space before and after the
/// block.
///
/// The text is split between the lines where the line lengths come nearest an even share of the
/// whole, at places where a line may break when there are enough of them (after a space or a CJK
/// character), and between any two characters otherwise. A block has at most as many lines as
/// characters. The lines are stacked one em of the warichu size apart plus the Line Gap, the stack
/// centered across the line on the center of the line's ideographic em box (in vertical type, the
/// column's center line), the first line above (in vertical type, to the right). Each line is set
/// along the block by its Alignment; Auto sets the lines as Justify with Last Line Aligned Left
/// does. These are VectorCraft's own rules for the parts public documentation does not describe.
/// <https://helpx.adobe.com/illustrator/using/formatting-asian-characters.html>
pub(super) fn warichu(g: &mut [SGlyph], (vertical, whole): (bool, bool), style: impl Fn(&SGlyph) -> WariStyle) {
    let styles: Vec<WariStyle> = g.iter().map(&style).collect();
    let member =
        |i: usize| g.get(i).is_some_and(|gl| !matches!(gl.ch, '\u{2028}' | '\t') && gl.tcy.is_none()) && styles.get(i).is_some_and(|s| s.marked);
    let members: Vec<bool> = (0..g.len()).map(member).collect();
    let mut i = 0;
    while i < g.len() {
        let Some(first) = styles.get(i).copied().filter(|_| members.get(i).copied().unwrap_or(false)) else {
            i += 1;
            continue;
        };
        let mut end = i + 1;
        while members.get(end).copied().unwrap_or(false) && styles.get(end).is_some_and(|s| first.same_block(s)) {
            end += 1;
        }
        if let Some(block) = g.get_mut(i..end) {
            place_block(block, &first, (vertical, whole));
        }
        i = end;
    }
}

/// The clusters of `g` (glyphs of one character, by their byte), as glyph index ranges.
fn clusters(g: &[SGlyph]) -> Vec<std::ops::Range<usize>> {
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, gl) in g.iter().enumerate() {
        match out.last_mut() {
            Some(last) if i > 0 && g.get(i - 1).is_some_and(|p| p.byte == gl.byte && p.len > 0 && gl.len > 0) => last.end = i + 1,
            _ => out.push(i..i + 1),
        }
    }
    out
}

/// Where the block's text is split into `lines` lines: the cluster indices each later line starts
/// at, nearest an even share of the whole length `total` (`ends` holds the length up to the end
/// of each cluster), at places where a line may break when they give every line its share, and
/// between any two characters otherwise.
fn splits(g: &[SGlyph], clusters: &[std::ops::Range<usize>], ends: &[f64], lines: usize) -> Vec<usize> {
    let count = clusters.len();
    let may_break = |c: usize| clusters.get(c - 1).and_then(|r| g.get(r.end - 1)).is_some_and(SGlyph::break_after);
    let preferred: Vec<usize> = (1..count).filter(|&c| may_break(c)).collect();
    let out = nearest_splits(&preferred, ends, lines, count);
    if out.len() + 1 >= lines { out } else { nearest_splits(&(1..count).collect::<Vec<_>>(), ends, lines, count) }
}

/// [`splits`] among the cluster indices `candidates` of a block of `count` clusters.
fn nearest_splits(candidates: &[usize], ends: &[f64], lines: usize, count: usize) -> Vec<usize> {
    let total = ends.last().copied().unwrap_or(0.0);
    let mut out = Vec::with_capacity(lines.saturating_sub(1));
    let mut previous = 0;
    for k in 1..lines {
        let target = total * k as f64 / lines as f64;
        // Leave a cluster and a place to split for each line still to come.
        let last = count.saturating_sub(lines - k);
        let later = |c: usize| candidates.len() - candidates.partition_point(|&x| x <= c) >= lines - 1 - k;
        let best = candidates.iter().copied().filter(|&c| c > previous && c <= last && later(c)).min_by(|&a, &b| {
            (ends.get(a - 1).copied().unwrap_or(0.0) - target).abs().total_cmp(&(ends.get(b - 1).copied().unwrap_or(0.0) - target).abs())
        });
        let Some(c) = best else { break };
        out.push(c);
        previous = c;
    }
    out
}

/// Set one warichu block `g` in the settings `s` of its first glyph, in `vertical` type or not.
/// With `whole` (System Layout on or Fractional Widths off) the block's length is a whole number
/// of points, at least one, as each character's advance is ([`super::whole_character_widths`]).
fn place_block(g: &mut [SGlyph], s: &WariStyle, (vertical, whole): (bool, bool)) {
    let Some(first) = g.first() else { return };
    let start = first.byte;
    let ic = first.face.ideographic_centre();
    let (ascent, descent) = (first.ascent, first.descent);
    let clusters = clusters(g);
    // Each glyph's advance without its Insert Aki.
    let own = |gl: &SGlyph| gl.adv - gl.aki;
    let widths: Vec<f64> = clusters.iter().map(|r| g.get(r.clone()).map_or(0.0, |c| c.iter().map(own).sum())).collect();
    let ends: Vec<f64> = widths
        .iter()
        .scan(0.0, |sum, w| {
            *sum += w;
            Some(*sum)
        })
        .collect();
    let lines = usize::from(s.lines.clamp(*vectorcraft_doc::text::WARICHU_LINES.start(), *vectorcraft_doc::text::WARICHU_LINES.end()))
        .min(clusters.len())
        .max(1);
    let mut starts = vec![0];
    starts.extend(splits(g, &clusters, &ends, lines));
    starts.push(clusters.len());
    let rows: Vec<std::ops::Range<usize>> = starts.windows(2).filter_map(|w| Some(*w.first()?..*w.get(1)?)).filter(|r| !r.is_empty()).collect();
    // Every character keeps its advance in the block, a space too, so each has its place for the
    // caret and the selection.
    let length = |r: &std::ops::Range<usize>| widths.get(r.clone()).map_or(0.0, |w| w.iter().sum::<f64>());
    let block = rows.iter().map(length).fold(0.0, f64::max);
    let block = if whole && block.is_finite() && block > 0.0 { block.round().max(1.0) } else { block };
    let em = s.size;
    let gap = if s.gap.is_finite() { s.gap.clamp(0.0, vectorcraft_doc::MAX_COORD) } else { 0.0 };
    let count = rows.len();
    let height = count as f64 * em + count.saturating_sub(1) as f64 * gap;
    let top = -ic * s.parent_size() - height * 0.5;
    // Every glyph's place: (along the block, across the line, its own advance).
    let mut places = vec![(0.0, 0.0, 0.0); g.len()];
    for (k, row) in rows.iter().enumerate() {
        let baseline = top + k as f64 * (em + gap) + (ic + 0.5) * em;
        let room = (block - length(row)).max(0.0);
        let last = k + 1 == count;
        let gaps = row.len().saturating_sub(1);
        let align = match (s.align, last) {
            (WarichuAlign::Auto | WarichuAlign::JustifyLeft, true) => WarichuAlign::Left,
            (WarichuAlign::JustifyCenter, true) => WarichuAlign::Center,
            (WarichuAlign::JustifyRight, true) => WarichuAlign::Right,
            (WarichuAlign::Auto, false) => WarichuAlign::JustifyAll,
            (a, _) => a,
        };
        let (mut pen, between) = match align {
            WarichuAlign::Center => (room * 0.5, 0.0),
            WarichuAlign::Right => (room, 0.0),
            WarichuAlign::Left => (0.0, 0.0),
            _ if gaps > 0 => (0.0, room / gaps as f64),
            _ => (0.0, 0.0),
        };
        // The row's clusters in the order they are drawn: by their bidi levels in horizontal type
        // (right-to-left text reads from the right), in reading order down a vertical column.
        let order: Vec<usize> = if vertical {
            row.clone().collect()
        } else {
            let levels: Vec<unicode_bidi::Level> =
                row.clone().map(|c| clusters.get(c).and_then(|r| g.get(r.start)).map_or(unicode_bidi::Level::ltr(), |gl| gl.level)).collect();
            unicode_bidi::BidiInfo::reorder_visual(&levels).into_iter().map(|i| row.start + i).collect()
        };
        for (n, &c) in order.iter().enumerate() {
            let Some(range) = clusters.get(c) else { continue };
            let mut inside = 0.0;
            for i in range.clone() {
                let adv = g.get(i).map_or(0.0, own);
                if let Some(p) = places.get_mut(i) {
                    *p = (pen + inside, baseline, adv);
                }
                inside += adv;
            }
            pen += widths.get(c).copied().unwrap_or(0.0) + if n + 1 < order.len() { between } else { 0.0 };
        }
    }
    let r = scale_factor(s.scale);
    let bottom = top + height;
    // Insert Aki: the space before the block's first character and after its last, in reading
    // order, is space before and after the block, on the sides the block's direction (its first
    // character's) sets; inside the block it takes no part. A cluster holds the space on its left
    // (above, in vertical type) in its glyphs' `aki_shift`, and the rest of the `aki` its last glyph
    // carries on its right (below).
    let sides = |r: Option<&std::ops::Range<usize>>| {
        let Some(c) = r.and_then(|r| g.get(r.clone())) else { return (0.0, 0.0) };
        let left = c.first().map_or(0.0, |gl| gl.aki_shift);
        let right = c.iter().map(|gl| gl.aki).sum::<f64>() - left;
        let rtl = !vertical && c.first().is_some_and(|gl| gl.level.is_rtl());
        // (before, after) in reading order.
        if rtl { (right, left) } else { (left, right) }
    };
    let (before, _) = sides(clusters.first());
    let (_, after) = sides(clusters.last());
    let finite = |v: f64| {
        if !v.is_finite() {
            0.0
        } else if whole {
            v.round().max(0.0)
        } else {
            v.max(0.0)
        }
    };
    let (before, after) = (finite(before), finite(after));
    let rtl = !vertical && g.first().is_some_and(|gl| gl.level.is_rtl());
    let (inset, outset) = if rtl { (after, before) } else { (before, after) };
    let length = inset + block + outset;
    let glyphs = g.len();
    for (i, (gl, (along, across, advance))) in g.iter_mut().zip(places).enumerate() {
        let em = gl.face.units_per_em() * gl.sy / r;
        gl.wari = Some(Wari { start, pen: if i == 0 { 0.0 } else { length }, along: inset + along, across, advance, em });
        gl.adv = if i == 0 { length } else { 0.0 };
        gl.tracking = 0.0;
        gl.kern_shift -= gl.aki_shift;
        gl.aki_shift = 0.0;
        gl.aki = if i == 0 { inset + outset } else { 0.0 };
        // No line breaks inside the block; the line may break before and after it.
        if i + 1 < glyphs {
            gl.keeps_next = true;
        }
        // The line keeps the height of the block's own size, and holds the whole stack.
        gl.ascent = (ascent / r).max(-top);
        gl.descent = (descent / r).max(bottom);
        if s.auto_leading {
            gl.leading /= r;
        }
        // The first baseline (Cap Height, x Height) measures the line's own size too.
        gl.cap /= r;
        gl.xh /= r;
    }
}
