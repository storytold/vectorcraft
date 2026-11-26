//! Dynamic Text's Block for area type ([`TextObject::dynamic_text`] on area type): each line as
//! typed is set to the frame's width by changing its font size, after every edit
//! ([`super::refresh_bounds`]).

use vectorcraft_doc::{TextKind, TextObject, TextRun};

/// The font size range a line's first character is fitted within, in document points (the
/// Character panel's).
const SIZES: (f64, f64) = (0.1, 1296.0);

/// Most passes a refit runs to correct lines that still wrap after the first fit; each pass measures
/// the lines and runs at most one more area layout.
pub(crate) const BLOCK_PASSES: usize = 4;

/// One line as typed: `start..content` is its text, `content..end` the `\n` or U+2028 that ends it
/// (empty for the last line). `para` is the index of its paragraph, and `para_start` whether it
/// starts that paragraph.
#[derive(Clone, Copy, Debug)]
struct TypedLine {
    start: usize,
    content: usize,
    end: usize,
    para: usize,
    para_start: bool,
}

/// The lines as typed: the text split after each `\n` and U+2028.
fn typed_lines(text: &str) -> Vec<TypedLine> {
    let mut out = vec![];
    let (mut start, mut para, mut para_start) = (0, 0, true);
    for (i, c) in text.char_indices() {
        if c == '\n' || c == '\u{2028}' {
            let end = i + c.len_utf8();
            out.push(TypedLine { start, content: i, end, para, para_start });
            start = end;
            para_start = c == '\n';
            if c == '\n' {
                para += 1;
            }
        }
    }
    out.push(TypedLine { start, content: text.len(), end: text.len(), para, para_start });
    out
}

/// The stored size of the character at `byte` (the run that holds it, not the one before it).
pub(crate) fn size_at(runs: &[TextRun], byte: usize) -> f64 {
    let mut off = 0;
    for r in runs {
        let end = off + r.text.len();
        if (off..end).contains(&byte) {
            return r.style.size;
        }
        off = end;
    }
    vectorcraft_text::edit::style_at(runs, byte).size
}

/// The stored size of each line's first character, in one pass over the runs.
fn first_sizes(runs: &[TextRun], lines: &[TypedLine]) -> Vec<f64> {
    let mut out = Vec::with_capacity(lines.len());
    let (mut ri, mut off) = (0, 0);
    for l in lines {
        while let Some(r) = runs.get(ri) {
            if l.start < off + r.text.len() {
                break;
            }
            off += r.text.len();
            ri += 1;
        }
        out.push(runs.get(ri).map_or_else(|| size_at(runs, l.start), |r| r.style.size));
    }
    out
}

/// Each line's content width (trailing spaces left out) by its first byte, with `t` laid out as
/// point type, where no line wraps, with the word spacing `t`'s layout uses.
fn line_widths(db: &vectorcraft_text::FontDb, t: &TextObject) -> std::collections::HashMap<usize, f64> {
    let mut probe = t.clone();
    for p in probe.para_styles_mut() {
        p.justification = t.layout_justification(p);
    }
    probe.kind = TextKind::Point;
    let laid = vectorcraft_text::layout(db, &probe);
    let mut out = std::collections::HashMap::with_capacity(laid.lines.len());
    for m in &laid.lines {
        out.entry(m.start).or_insert(m.x1 - m.x0);
    }
    out
}

/// The room along its lines that a frame of `t`'s text needs for Block to keep the widest line as
/// typed at its present size: that line's width plus its paragraph's indents, as [`block_fit`]
/// measures them, and a margin of under half a size step so that the line is not made a step
/// smaller; with the widest line's first byte and its paragraph's index. A line whose negative
/// indents take more than its width needs its own width. None when no line has a width.
pub(crate) fn needed_room(t: &TextObject) -> Option<(f64, usize, usize)> {
    let points = t.style_scale().map(|s| s.points).filter(|p| p.is_finite() && *p > 1e-12)?;
    let text = t.plain_text();
    let lines = typed_lines(&text);
    let widths = line_widths(vectorcraft_text::FontDb::global(), t);
    let sizes = first_sizes(&t.runs, &lines);
    lines
        .iter()
        .zip(&sizes)
        .filter_map(|(l, size)| {
            let width = widths.get(&l.start).copied().filter(|w| w.is_finite() && *w > 1e-9)?;
            let size = size * points;
            if l.content == l.start || !(size.is_finite() && size > 0.0) {
                return None;
            }
            let para = t.para_at(l.para);
            let first = if l.para_start { para.first_line_indent } else { 0.0 };
            let indents = para.laid_out_left_indent() + para.right_indent + first;
            // Negative indents that take more than the line's width leave the line no room at its
            // size; the frame is then as wide as the line itself, and Block makes it larger.
            let room = if width + indents > 1e-9 { width + indents } else { width };
            // A tenth of a point more size widens the line by 0.1 * width / size.
            Some((room + 0.04 * width / size, l.start, l.para))
        })
        .filter(|r| r.0.is_finite() && r.0 > 1e-9)
        .reduce(|a, b| if b.0 > a.0 { b } else { a })
}

/// The size a line set at `size` points and `width` wide takes to fill `avail`, its width growing
/// by `slope` a point of size (`width / size` when the whole line follows its size): rounded down
/// to tenths and kept within [`SIZES`]. A line that fits and would not fit a tenth larger keeps its
/// size rounded down to tenths, and None when that is its size, so fitting again changes nothing.
/// Room taken entirely by indents gives the smallest size.
fn refit(size: f64, width: f64, avail: f64, slope: f64) -> Option<f64> {
    if width <= avail + 1e-6 && width + 0.1 * slope > avail {
        let rounded = tenths(size);
        return ((rounded - size).abs() > 1e-9).then_some(rounded);
    }
    Some(tenths(size + (avail.max(0.0) - width) / slope))
}

/// A size in document points rounded down to a whole number of tenths and kept within [`SIZES`].
fn tenths(points: f64) -> f64 {
    ((points * 10.0 + 1e-6).floor() / 10.0).clamp(SIZES.0, SIZES.1)
}

/// Fit area type with Dynamic Text (Block) on: with the word gap as every paragraph's Desired word
/// spacing ([`TextObject::layout_justification`]; the paragraphs keep their own values), each line
/// as typed gets the size (its first character's, in steps of 0.1 pt within 0.1 to
/// 1,296 pt, the line's other characters keeping their proportions to it) at which its width
/// equals the frame's width less the inset, the gutters and the paragraph's indents, and each
/// character's leading is its size plus the line gap. An empty line or one of only spaces keeps
/// its size. Nothing changes when the frame has no width along the lines or the type's transform
/// has no size. A line that still wraps at its first fit is made smaller, in at most
/// [`BLOCK_PASSES`] more passes. True when the text changed.
pub(crate) fn block_fit(t: &mut TextObject) -> bool {
    if !t.dynamic_text {
        return false;
    }
    let TextKind::Area { frame } = &t.kind else { return false };
    let (_, line_gap) = t.area.block_gaps();
    let Some(b) = frame.bounds().filter(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite())) else { return false };
    let Some(points) = t.style_scale().map(|s| s.points).filter(|p| p.is_finite() && *p > 1e-12) else { return false };
    let before = t.runs.clone();

    // The room along a line in one cell of the frame, before indents.
    let a = &t.area;
    let (along, cells, gutter) = if t.vertical { (b.height(), a.rows, a.row_gutter.unwrap_or(a.gutter)) } else { (b.width(), a.columns, a.gutter) };
    let cells = cells.max(1) as f64;
    let cell = (along - 2.0 * a.inset.max(0.0) - gutter.max(0.0) * (cells - 1.0)) / cells;
    if !(cell.is_finite() && cell > 1e-9) {
        return t.runs != before;
    }

    let db = vectorcraft_text::FontDb::global();
    let text = t.plain_text();
    let lines = typed_lines(&text);
    let widths = line_widths(db, t);
    let sizes = first_sizes(&t.runs, &lines);
    // Per line: (factor applied to its sizes, the room it fills, the last size measured and its
    // width), or None to keep its size.
    let mut fits: Vec<Option<(f64, f64, f64, f64)>> = lines
        .iter()
        .zip(&sizes)
        .map(|(l, size)| {
            if l.content == l.start {
                return None;
            }
            let width = widths.get(&l.start).copied().filter(|w| w.is_finite() && *w > 1e-9)?;
            let para = t.para_at(l.para);
            let first = if l.para_start { para.first_line_indent } else { 0.0 };
            let avail = cell - para.laid_out_left_indent() - para.right_indent - first;
            let size = size * points;
            if !(size.is_finite() && size > 0.0) {
                return None;
            }
            Some((refit(size, width, avail, width / size).map_or(1.0, |n| n / size), avail, size, width))
        })
        .collect();

    let gap = line_gap / points;
    let min_leading = SIZES.0 / points;
    // Always from the sizes the refit started with, so each pass sets a line's size once: the runs
    // are split where the lines end and each piece scaled by its line's factor, in one pass.
    let apply = |t: &mut TextObject, from: &[TextRun], fits: &[Option<(f64, f64, f64, f64)>]| {
        let mut out = Vec::with_capacity(from.len() + lines.len());
        let (mut off, mut li) = (0, 0);
        for r in from {
            let end = off + r.text.len();
            let mut at = off;
            loop {
                while lines.get(li).is_some_and(|l| l.end <= at && li + 1 < lines.len()) {
                    li += 1;
                }
                let stop = lines.get(li).map_or(end, |l| l.end.clamp(at, end));
                let stop = if stop == at { end } else { stop };
                let f = fits.get(li).copied().flatten().map_or(1.0, |(f, ..)| f);
                let mut piece = TextRun { text: r.text.get(at - off..stop - off).unwrap_or_default().to_string(), ..r.clone() };
                if at > off {
                    piece.style.glyph_alternate = None;
                }
                let st = &mut piece.style;
                st.size *= f;
                st.baseline_shift *= f;
                st.tcy_up_down *= f;
                st.tcy_left_right *= f;
                st.warichu_gap *= f;
                st.leading = Some((st.size + gap).max(min_leading));
                if let Some(art) = &mut piece.inline {
                    art.baseline_shift *= f;
                }
                out.push(piece);
                at = stop;
                if at >= end {
                    break;
                }
            }
            off = end;
        }
        vectorcraft_text::edit::normalize(&mut out);
        t.runs = out;
    };
    let start = t.runs.clone();
    apply(t, &start, &fits);

    // Each pass measures the lines again: a line whose width did not follow its size (a tab to a
    // fixed stop) is fitted again from its measured width, and when every line measures right, a
    // line the frame's shape or a list marker still wraps is made smaller, by what the line the
    // layout gave it holds.
    // Per line: the smallest size measured too wide, which a later pass never goes back to.
    let mut too_wide = vec![f64::INFINITY; lines.len()];
    for _ in 0..BLOCK_PASSES {
        let now_widths = line_widths(db, t);
        let mut again = false;
        for (((l, fit), size), wide) in lines.iter().zip(fits.iter_mut()).zip(&sizes).zip(too_wide.iter_mut()) {
            let Some((f, avail, last_size, last_width)) = fit else { continue };
            let size = size * points;
            let now = size * *f;
            let Some(width) = now_widths.get(&l.start).copied().filter(|w| w.is_finite() && *w > 1e-9) else { continue };
            if width > *avail + 1e-6 {
                *wide = wide.min(now);
            }
            // The slope between the last two sizes measured, where they differ: a line whose width
            // has a part that stays as the size changes is fitted in one step.
            let slope = Some((width - *last_width) / (now - *last_size))
                .filter(|s| (now - *last_size).abs() > 1e-9 && s.is_finite() && *s > 1e-9)
                .unwrap_or(width / now);
            (*last_size, *last_width) = (now, width);
            let next = refit(now, width, *avail, slope).map(|n| if n >= *wide - 1e-9 { tenths(*wide - 0.1) } else { n });
            if let Some(next) = next.filter(|n| (n - now).abs() > 1e-9) {
                *f = next / size;
                again = true;
            }
        }
        if !again {
            let laid = vectorcraft_text::layout(db, t);
            // Per typed line: how many laid-out lines it took, and the room of its first.
            let mut parts: Vec<(usize, f64)> = vec![(0, 0.0); lines.len()];
            for m in &laid.lines {
                let i = lines.partition_point(|l| l.start <= m.start).saturating_sub(1);
                if let Some(p) = parts.get_mut(i) {
                    if p.0 == 0 {
                        p.1 = m.avail.1 - m.avail.0;
                    }
                    p.0 += 1;
                }
            }
            for (((l, fit), (count, room)), size) in lines.iter().zip(fits.iter_mut()).zip(&parts).zip(&sizes) {
                let Some((f, ..)) = fit else { continue };
                if *count < 2 {
                    continue;
                }
                let size = size * points;
                let now = size * *f;
                if now <= SIZES.0 + 1e-9 {
                    continue;
                }
                let width = now_widths.get(&l.start).copied().unwrap_or(f64::INFINITY);
                let smaller = if room.is_finite() && *room > 0.0 && *room < width { tenths(now * room / width) } else { now };
                *f = smaller.min(tenths(now - 0.1)) / size;
                again = true;
            }
        }
        if !again {
            break;
        }
        apply(t, &start, &fits);
    }
    t.runs != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_lines_split_after_returns_and_forced_breaks() {
        let l = typed_lines("ab\ncd\u{2028}e\n");
        let spans: Vec<_> = l.iter().map(|l| (l.start, l.content, l.end, l.para, l.para_start)).collect();
        assert_eq!(spans, vec![(0, 2, 3, 0, true), (3, 5, 8, 1, true), (8, 9, 10, 1, false), (10, 10, 10, 2, true)]);
        assert_eq!(typed_lines("").len(), 1);
    }

    #[test]
    fn a_fitting_line_keeps_its_size_in_tenths() {
        // 100 pt wide at 12 pt, 100.5 pt of room: a tenth larger would not fit.
        assert_eq!(refit(12.0, 100.0, 100.5, 100.0 / 12.0), None);
        assert_eq!(refit(12.34, 100.0, 100.5, 100.0 / 12.34), Some(12.3), "a size between tenths is rounded down");
        // Room for twice the width: twice the size.
        assert_eq!(refit(12.0, 100.0, 200.0, 100.0 / 12.0), Some(24.0));
        // A part of the width that stays (a slope below width / size) needs more size.
        assert_eq!(refit(12.0, 100.0, 150.0, 5.0), Some(22.0));
    }

    #[test]
    fn sizes_round_down_to_tenths_within_the_range() {
        assert_eq!(tenths(23.99), 23.9);
        assert_eq!(tenths(24.0 - 1e-9), 24.0);
        assert_eq!(tenths(0.01), 0.1);
        assert_eq!(tenths(5000.0), 1296.0);
    }
}
