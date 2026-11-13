//! Threaded text: one story flowing through several area-type frames in order.
//!
//! Each frame keeps its own slice of the story as ordinary runs, so rendering, export and hit
//! testing need nothing special; after an edit the engine joins the slices and re-distributes them
//! with [`distribute`].

use vectorcraft_doc::text::ListCounts;
use vectorcraft_doc::{CharStyle, ParaStyle, TextKind, TextObject, TextRun};

use crate::edit::{floor_char, normalize, runs_len};
use crate::{FontDb, layout};

/// The story: every frame's runs joined in thread order.
pub fn story(frames: &[&TextObject]) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = frames.iter().flat_map(|f| f.runs.iter().cloned()).collect();
    normalize(&mut runs);
    runs
}

/// The story's paragraph styles, one per story paragraph. A frame whose slice ends inside a
/// paragraph shares that paragraph with the next frame (its first one): the earlier frame's style
/// wins unless `later_wins[k]` says frame `k`'s first paragraph should (it was just restyled), or
/// the earlier frame's part of the paragraph is empty (its slice ends with a paragraph break).
/// An empty frame that was just restyled keeps its style against unedited empty frames after it.
pub fn story_paras(frames: &[&TextObject], later_wins: &[bool]) -> Vec<ParaStyle> {
    let mut out: Vec<ParaStyle> = vec![];
    let mut prev_empty_tail = true;
    let mut empty_tail_restyled = false;
    for (k, f) in frames.iter().enumerate() {
        let text = f.plain_text();
        let restyled = later_wins.get(k).copied().unwrap_or(false);
        let mut styles = f.paragraph_styles().into_iter();
        if k > 0
            && let Some(first) = styles.next()
            && (restyled || (prev_empty_tail && !(text.is_empty() && empty_tail_restyled)))
            && let Some(last) = out.last_mut()
        {
            *last = first;
        }
        out.extend(styles);
        // An empty frame has no paragraph boundary of its own. Keep the preceding frame's
        // boundary state so another empty frame cannot replace a paragraph's edited style.
        if !text.is_empty() {
            prev_empty_tail = text.ends_with('\n') || out.is_empty();
            empty_tail_restyled = false;
        } else if restyled {
            empty_tail_restyled = true;
        }
    }
    out
}

/// Paragraph styles of the slice `runs` of a story starting at story paragraph `first`.
fn slice_paras(paras: &[ParaStyle], first: usize, runs: &[TextRun]) -> Vec<ParaStyle> {
    let n = 1 + runs.iter().map(|r| r.text.bytes().filter(|&b| b == b'\n').count()).sum::<usize>();
    let fallback = paras.last().cloned().unwrap_or_default();
    (first..first + n).map(|i| paras.get(i).cloned().unwrap_or_else(|| fallback.clone())).collect()
}

/// How many bytes of `probe`'s runs fit in it (whole lines; a following break space or paragraph
/// break stays with the line that ends there, but on a path, where a space with no room
/// overflows as any character does, it starts the next frame), and for area type that overflows,
/// where its first line that does not fit starts.
fn fit(db: &FontDb, probe: &TextObject) -> (usize, Option<usize>) {
    #[cfg(test)]
    FIT_BYTES.set(FIT_BYTES.get() + runs_len(&probe.runs));
    let lay = layout(db, probe);
    let text: String = probe.runs.iter().map(|r| r.text.as_str()).collect();
    if !lay.overflow {
        return (text.len(), None);
    }
    let mut end = lay.lines.last().map_or(0, |l| l.end);
    if lay.on_path {
        return (end, None);
    }
    let unplaced = end;
    let bytes = text.as_bytes();
    while end < bytes.len() && bytes[end] == b' ' {
        end += 1;
    }
    if end < bytes.len() && bytes[end] == b'\n' {
        end += 1;
    }
    (end, Some(unplaced))
}

#[cfg(test)]
thread_local! {
    /// The bytes of text [`fit`] has laid out on this thread ([`fit_bytes`]).
    static FIT_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The bytes of text [`fit`] has laid out on this thread.
#[cfg(test)]
fn fit_bytes() -> usize {
    FIT_BYTES.get()
}

#[cfg(test)]
thread_local! {
    /// The bytes of frames' own text copied on this thread to measure them ([`probe_copied`]).
    static PROBE_COPIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The bytes of frames' own text copied on this thread to measure them.
#[cfg(test)]
fn probe_copied() -> usize {
    PROBE_COPIED.get()
}

/// A copy of `frame` to measure text in, its own text counted as copied ([`probe_copied`]).
fn copy_to_measure(frame: &TextObject) -> TextObject {
    #[cfg(test)]
    PROBE_COPIED.set(PROBE_COPIED.get() + runs_len(&frame.runs));
    frame.clone()
}

/// The fewest bytes of story a frame is measured with before the measured text grows.
const MIN_MEASURE: usize = 512;

/// What is left of a story while [`distribute_continuing_levels`] hands it out, `len` bytes
/// borrowed from the story's runs: the runs from `run` on, the first of them without its first
/// `skip` bytes. While `raw` they are all the runs given (`run` and `skip` are 0), empty runs
/// included. Once a frame has taken a share they are the story after it as
/// [`crate::edit::slice_runs`] cuts it: no empty runs, and a run cut after its first character
/// without its glyph choice.
#[derive(Clone, Copy)]
struct Rest<'a> {
    runs: &'a [TextRun],
    run: usize,
    skip: usize,
    raw: bool,
    len: usize,
}

impl<'a> Rest<'a> {
    /// The runs' pieces in order: each piece's text, its run and whether the run was cut before it.
    fn pieces(self) -> impl Iterator<Item = (&'a str, &'a TextRun, bool)> {
        self.runs.get(self.run..).unwrap_or_default().iter().enumerate().filter_map(move |(k, r)| {
            let at = if k == 0 { self.skip } else { 0 };
            let text = r.text.get(at..)?;
            (self.raw || !text.is_empty()).then_some((text, r, at > 0))
        })
    }

    /// A piece's style: its run's, without the glyph choice when the run was cut before it.
    fn style(r: &TextRun, cut: bool) -> CharStyle {
        let mut style = r.style.clone();
        if cut {
            style.glyph_alternate = None;
        }
        style
    }

    /// [`crate::edit::slice_runs`] of the rest's runs.
    fn slice(self, start: usize, end: usize) -> Vec<TextRun> {
        let mut out = vec![];
        let mut off = 0;
        for (text, r, cut) in self.pieces() {
            let (rs, re) = (off, off + text.len());
            if rs >= end {
                break;
            }
            let (a, b) = (start.max(rs), end.min(re));
            if a < b {
                let from = floor_char(text, a - rs);
                let t = &text[from..floor_char(text, b - rs)];
                if !t.is_empty() {
                    out.push(TextRun { text: t.to_string(), style: Self::style(r, cut || from > 0), inline: r.inline.clone() });
                }
            }
            off = re;
        }
        out
    }

    /// The rest's runs before byte `end`, which follows a paragraph break, as a frame is measured
    /// with them: every run that starts before it (an empty one too, while the runs are the ones
    /// given), the last one cut there.
    fn before(self, end: usize) -> Vec<TextRun> {
        let mut out = vec![];
        let mut off = 0;
        for (text, r, cut) in self.pieces() {
            if off >= end {
                break;
            }
            let t = text.get(..end - off).unwrap_or(text);
            out.push(TextRun { text: t.to_string(), style: Self::style(r, cut), inline: r.inline.clone() });
            off += text.len();
        }
        out
    }

    /// [`crate::edit::style_at`] of the rest's runs.
    fn style_at(self, byte: usize) -> CharStyle {
        let mut off = 0;
        let mut last = None;
        let mut first = None;
        for (text, r, cut) in self.pieces() {
            first = first.or(Some((r, cut)));
            let end = off + text.len();
            if byte < end || (byte == end && byte == off) {
                return Self::style(r, cut);
            }
            if !text.is_empty() {
                last = Some((r, cut));
                if byte == end {
                    return Self::style(r, cut);
                }
            }
            off = end;
        }
        last.or(first).map(|(r, cut)| Self::style(r, cut)).unwrap_or_default()
    }

    /// The end of the first paragraph break at or after byte `from`, or None when the rest has no
    /// break there.
    fn break_after(self, from: usize) -> Option<usize> {
        let mut off = 0;
        for (text, _, _) in self.pieces() {
            let at = from.saturating_sub(off);
            if let Some(i) = text.as_bytes().get(at..).and_then(|b| b.iter().position(|&b| b == b'\n')) {
                return Some(off + at + i + 1);
            }
            off += text.len();
        }
        None
    }

    /// All of the rest's runs.
    fn all(self) -> Vec<TextRun> {
        if self.raw { self.runs.to_vec() } else { self.slice(0, self.len) }
    }

    /// The rest after its first `n` bytes, as a slice of the story: the run that byte falls in
    /// cut there (back to a character boundary, as [`crate::edit::slice_runs`] cuts it).
    fn after(self, n: usize) -> Self {
        let mut run = self.run;
        let mut at = self.skip + n;
        while let Some(r) = self.runs.get(run)
            && at >= r.text.len()
            && run + 1 < self.runs.len()
        {
            at -= r.text.len();
            run += 1;
        }
        let skip = self.runs.get(run).map_or(0, |r| floor_char(&r.text, at));
        let cut_back = at - skip;
        Self { runs: self.runs, run, skip, raw: false, len: self.len + cut_back - n }
    }
}

/// How many bytes of `rest` fit in `frame`, measured as `probe` makes the frame for some runs (with
/// their paragraph styles, the list counts and the continuation mark).
///
/// Area type without Shrink Text to Fit whose lines align to the top is first measured with the
/// whole paragraphs of the rest that end at or after byte `want`, and then with twice as many bytes
/// each time, while the rest goes on past them. The layout places lines in order and stops at the
/// first line that does not fit, and each paragraph is shaped and broken on its own text. So when
/// that line starts before the last paragraph break measured, the frame holds the same bytes with
/// the whole rest. Other type, and a rest that ends first, is measured with the whole rest.
fn fit_rest(db: &FontDb, probe: impl Fn(Vec<TextRun>) -> TextObject, frame: &TextObject, rest: Rest<'_>, mut want: usize) -> usize {
    let by_paragraph = matches!(frame.kind, TextKind::Area { .. })
        && frame.area.fit.min_scale().is_none()
        && frame.area.vertical_align == vectorcraft_doc::VerticalAlign::Top;
    if by_paragraph {
        while let Some(end) = rest.break_after(want.saturating_sub(1)).filter(|&end| end < rest.len) {
            let (n, unplaced) = fit(db, &probe(rest.before(end)));
            if unplaced.is_some_and(|u| u + 1 < end) {
                return n;
            }
            want = end.saturating_mul(2);
        }
    }
    fit(db, &probe(rest.all())).0
}

/// A frame's share of a story: its runs, its paragraphs' styles (one per paragraph) and the
/// numbered list paragraphs counted before its first one, at level 1 and at the levels past it
/// ([`TextObject::list_count`], [`TextObject::list_level_counts`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub runs: Vec<TextRun>,
    pub paras: Vec<ParaStyle>,
    pub list_count: u32,
    pub list_level_counts: Vec<u32>,
    /// The first paragraph continues from the frame before
    /// ([`TextObject::first_para_continues`]).
    pub continues: bool,
}

impl Slice {
    /// The list counts the slice's frame starts from ([`TextObject::list_counts`]).
    pub fn list_counts(&self) -> ListCounts {
        ListCounts::from_parts(self.list_count, &self.list_level_counts)
    }
}

/// The counts of numbered list paragraphs at each level after the paragraphs of `runs` (styles
/// `paras`, one per paragraph) but the last one, counted on from `counts` as the layout counts them
/// ([`vectorcraft_doc::text::ParaList::next_marker`]). A slice's last paragraph also starts the
/// next frame, which counts it.
fn count_list_paragraphs(runs: &[TextRun], paras: &[ParaStyle], mut counts: ListCounts) -> ListCounts {
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
    let mut pieces: Vec<&str> = text.split('\n').collect();
    pieces.pop();
    for (piece, para) in pieces.iter().zip(paras) {
        vectorcraft_doc::text::ParaList::next_marker(para.list, piece.is_empty(), &mut counts);
    }
    counts
}

/// Whether the frame after one whose share ends with byte `before` starts inside a paragraph: it
/// does unless that byte is a paragraph break. After an empty share (`None`) it takes `was`, the
/// mark of the frame with the empty share.
fn continues_after(before: Option<u8>, was: bool) -> bool {
    before.map_or(was, |b| b != b'\n')
}

/// For each of `frames` in thread order, whether its first paragraph continues from the frames
/// before it ([`TextObject::first_para_continues`]): the text of those frames is not empty and
/// does not end with a paragraph break.
pub fn continued(frames: &[&TextObject]) -> Vec<bool> {
    let mut on = false;
    frames
        .iter()
        .map(|f| {
            let this = on;
            on = continues_after(f.runs.iter().rev().find_map(|r| r.text.bytes().last()), on);
            this
        })
        .collect()
}

/// The story `runs` (with paragraph styles `paras`, one per story paragraph; empty = each
/// frame's own) split across `frames` in order (the last frame takes any overflow). Empty slices
/// keep one empty run carrying the style at the split, so typing there has a style. A paragraph
/// split between frames has its style in both, and each slice that starts inside a paragraph is
/// marked as continuing it ([`Slice::continues`]). Each frame's numbered list paragraphs are counted
/// on from the frames before it, at each level. Each frame's share is measured with that mark and
/// those counts.
pub fn distribute(db: &FontDb, frames: &[&TextObject], runs: &[TextRun], paras: &[ParaStyle]) -> Vec<Slice> {
    distribute_continuing_levels(db, frames, runs, paras, false, ListCounts::default())
}

/// [`distribute`] for `runs` that follow text before them: the first frame's share is measured,
/// and marked, as continuing a paragraph when `continued` is set, and its numbered list paragraphs
/// at level 1 are counted on from `list_count` ([`distribute_continuing_levels`] takes the counts
/// of every level).
pub fn distribute_continuing(
    db: &FontDb,
    frames: &[&TextObject],
    runs: &[TextRun],
    paras: &[ParaStyle],
    continued: bool,
    list_count: u32,
) -> Vec<Slice> {
    distribute_continuing_levels(db, frames, runs, paras, continued, ListCounts::from_parts(list_count, &[]))
}

/// [`distribute_continuing`] with the first frame's numbered list paragraphs counted on from
/// `counts` at each level.
pub fn distribute_continuing_levels(
    db: &FontDb,
    frames: &[&TextObject],
    runs: &[TextRun],
    paras: &[ParaStyle],
    continued: bool,
    mut counts: ListCounts,
) -> Vec<Slice> {
    let mut story_rest = Rest { runs, run: 0, skip: 0, raw: true, len: runs_len(runs) };
    // Once the story is used up, what is left is one empty run with the style there.
    let mut used_up: Option<TextRun> = None;
    let mut out = Vec::with_capacity(frames.len());
    let mut next_para = 0;
    let mut continues = continued;
    let mut want = MIN_MEASURE;
    for (i, f) in frames.iter().enumerate() {
        let first_para = next_para;
        let empty_story: [TextRun; 1];
        let rest = match used_up.clone() {
            Some(empty) => {
                empty_story = [empty];
                Rest { runs: &empty_story, run: 0, skip: 0, raw: true, len: 0 }
            }
            None => story_rest,
        };
        let len = rest.len;
        let own = [f.para.clone()];
        let (paras, first_para) = if paras.is_empty() { (&own[..], 0) } else { (paras, first_para) };
        let n = if i + 1 == frames.len() {
            len
        } else {
            // The frame without its own text, copied once and then for each measurement.
            let mut base = copy_to_measure(f);
            base.runs = Vec::new();
            base.paras = Vec::new();
            let probe = |runs: Vec<TextRun>| {
                let mut probe = copy_to_measure(&base);
                let styles = slice_paras(paras, first_para, &runs);
                probe.runs = runs;
                probe.set_paragraph_styles(styles);
                probe.set_list_counts(counts);
                probe.first_para_continues = continues;
                probe
            };
            fit_rest(db, probe, f, rest, want)
        };
        want = n.saturating_mul(2).max(MIN_MEASURE);
        let mut head = rest.slice(0, n);
        if head.is_empty() {
            head.push(TextRun { text: String::new(), style: rest.style_at(n), inline: None });
        }
        if used_up.is_none() {
            if n >= len {
                used_up = Some(TextRun { text: String::new(), style: rest.style_at(len), inline: None });
            } else {
                story_rest = story_rest.after(n);
            }
        }
        let head_paras = slice_paras(paras, first_para, &head);
        next_para += head_paras.len().saturating_sub(1);
        let next_counts = count_list_paragraphs(&head, &head_paras, counts);
        let starts_continued = continues;
        continues = continues_after(head.iter().rev().find_map(|r| r.text.bytes().last()), continues);
        let (list_count, list_level_counts) = counts.parts();
        out.push(Slice { runs: head, paras: head_paras, list_count, list_level_counts, continues: starts_continued });
        counts = next_counts;
    }
    out
}

#[cfg(test)]
mod tests;
