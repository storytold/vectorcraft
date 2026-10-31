//! Threaded text: one story flowing through several area-type frames in order.
//!
//! Each frame keeps its own slice of the story as ordinary runs, so rendering, export and hit
//! testing need nothing special; after an edit the engine joins the slices and re-distributes them
//! with [`distribute`].

use vectorcraft_doc::{ParaStyle, TextObject, TextRun};

use crate::edit::{normalize, runs_len, slice_runs, style_at};
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
pub fn story_paras(frames: &[&TextObject], later_wins: &[bool]) -> Vec<ParaStyle> {
    let mut out: Vec<ParaStyle> = vec![];
    let mut prev_empty_tail = true;
    for (k, f) in frames.iter().enumerate() {
        let mut styles = f.paragraph_styles().into_iter();
        if k > 0
            && let Some(first) = styles.next()
            && (prev_empty_tail || later_wins.get(k).copied().unwrap_or(false))
            && let Some(last) = out.last_mut()
        {
            *last = first;
        }
        out.extend(styles);
        let text = f.plain_text();
        prev_empty_tail = text.is_empty() || text.ends_with('\n') || out.is_empty();
    }
    out
}

/// Paragraph styles of the slice `runs` of a story starting at story paragraph `first`.
fn slice_paras(paras: &[ParaStyle], first: usize, runs: &[TextRun]) -> Vec<ParaStyle> {
    let n = 1 + runs.iter().map(|r| r.text.bytes().filter(|&b| b == b'\n').count()).sum::<usize>();
    let fallback = paras.last().cloned().unwrap_or_default();
    (first..first + n).map(|i| paras.get(i).cloned().unwrap_or_else(|| fallback.clone())).collect()
}

/// How many bytes of `runs` fit in `frame` (whole lines; a following break space or paragraph
/// break stays with the line that ends there, but on a path, where a space with no room
/// overflows as any character does, it starts the next frame).
fn fit(db: &FontDb, frame: &TextObject, runs: &[TextRun]) -> usize {
    let mut probe = frame.clone();
    probe.runs = runs.to_vec();
    let lay = layout(db, &probe);
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
    if !lay.overflow {
        return text.len();
    }
    let mut end = lay.lines.last().map_or(0, |l| l.end);
    if lay.on_path {
        return end;
    }
    let bytes = text.as_bytes();
    while end < bytes.len() && bytes[end] == b' ' {
        end += 1;
    }
    if end < bytes.len() && bytes[end] == b'\n' {
        end += 1;
    }
    end
}

/// A frame's share of a story: its runs, its paragraphs' styles (one per paragraph) and the
/// numbered list paragraphs counted before its first one ([`TextObject::list_count`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub runs: Vec<TextRun>,
    pub paras: Vec<ParaStyle>,
    pub list_count: u32,
    /// The first paragraph continues from the frame before
    /// ([`TextObject::first_para_continues`]).
    pub continues: bool,
}

/// The count of numbered list paragraphs after the paragraphs of `runs` (styles `paras`, one per
/// paragraph) but the last one, counted on from `count` as the layout counts them
/// ([`vectorcraft_doc::text::ParaList::next_marker`]). A slice's last paragraph also starts the
/// next frame, which counts it.
fn count_list_paragraphs(runs: &[TextRun], paras: &[ParaStyle], mut count: u32) -> u32 {
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
    let mut pieces: Vec<&str> = text.split('\n').collect();
    pieces.pop();
    for (piece, para) in pieces.iter().zip(paras) {
        vectorcraft_doc::text::ParaList::next_marker(para.list, piece.is_empty(), &mut count);
    }
    count
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
/// on from the frames before it. Each frame's share is measured with that mark and that count.
pub fn distribute(db: &FontDb, frames: &[&TextObject], runs: &[TextRun], paras: &[ParaStyle]) -> Vec<Slice> {
    distribute_continuing(db, frames, runs, paras, false, 0)
}

/// [`distribute`] for `runs` that follow text before them: the first frame's share is measured,
/// and marked, as continuing a paragraph when `continued` is set, and its numbered list paragraphs
/// are counted on from `list_count`.
pub fn distribute_continuing(
    db: &FontDb,
    frames: &[&TextObject],
    runs: &[TextRun],
    paras: &[ParaStyle],
    continued: bool,
    mut list_count: u32,
) -> Vec<Slice> {
    let mut rest = runs.to_vec();
    let mut out = Vec::with_capacity(frames.len());
    let mut next_para = 0;
    let mut continues = continued;
    for (i, f) in frames.iter().enumerate() {
        let first_para = next_para;
        let len = runs_len(&rest);
        let own = [f.para.clone()];
        let (paras, first_para) = if paras.is_empty() { (&own[..], 0) } else { (paras, first_para) };
        let n = if i + 1 == frames.len() {
            len
        } else {
            let mut probe = (*f).clone();
            probe.runs = rest.clone();
            probe.set_paragraph_styles(slice_paras(paras, first_para, &rest));
            probe.list_count = list_count;
            probe.first_para_continues = continues;
            fit(db, &probe, &rest)
        };
        let mut head = slice_runs(&rest, 0, n);
        if head.is_empty() {
            head.push(TextRun { text: String::new(), style: style_at(&rest, n), inline: None });
        }
        let tail = slice_runs(&rest, n, len);
        rest = if tail.is_empty() { vec![TextRun { text: String::new(), style: style_at(&rest, len), inline: None }] } else { tail };
        let head_paras = slice_paras(paras, first_para, &head);
        next_para += head_paras.len().saturating_sub(1);
        let next_count = count_list_paragraphs(&head, &head_paras, list_count);
        let starts_continued = continues;
        continues = continues_after(head.iter().rev().find_map(|r| r.text.bytes().last()), continues);
        out.push(Slice { runs: head, paras: head_paras, list_count, continues: starts_continued });
        list_count = next_count;
    }
    out
}

#[cfg(test)]
mod tests;
