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

/// A frame's share of a story: its runs and its paragraphs' styles (one per paragraph).
#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub runs: Vec<TextRun>,
    pub paras: Vec<ParaStyle>,
}

/// The story `runs` (with paragraph styles `paras`, one per story paragraph; empty = each
/// frame's own) split across `frames` in order (the last frame takes any overflow). Empty slices
/// keep one empty run carrying the style at the split, so typing there has a style. A paragraph
/// split between frames has its style in both.
pub fn distribute(db: &FontDb, frames: &[&TextObject], runs: &[TextRun], paras: &[ParaStyle]) -> Vec<Slice> {
    let mut rest = runs.to_vec();
    let mut out = Vec::with_capacity(frames.len());
    let mut next_para = 0;
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
        out.push(Slice { runs: head, paras: head_paras });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::{CharStyle, Justify, TextKind};
    use vectorcraft_geom::{Affine, PathData, Rect, shapes};

    fn frame(x: f64) -> TextObject {
        let mut t = TextObject::point(vectorcraft_geom::Point::ZERO, "", CharStyle::default());
        t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(x, 0.0, x + 120.0, 60.0)) };
        t.xf = Affine::IDENTITY;
        t
    }

    /// Type on a straight path `length` long.
    fn path_frame(length: f64) -> TextObject {
        let mut t = TextObject::point(vectorcraft_geom::Point::ZERO, "", CharStyle::default());
        let line = kurbo::Line::new((0.0, 0.0), (length, 0.0));
        t.kind = TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Shape::to_path(&line, 0.1)), start: 0.0, end: None };
        t.xf = Affine::IDENTITY;
        t
    }

    /// Type on a path that overflows ends its line where its text stops fitting, as area type's
    /// last line does, so a story flows on from there through area type and type on a path alike.
    #[test]
    fn story_flows_across_area_type_and_type_on_a_path() {
        let db = FontDb::global();
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(6);
        let story = vec![TextRun { text: text.clone(), style: CharStyle::default(), inline: None }];
        let mut path = path_frame(100.0);
        path.runs = story.clone();
        let laid = layout(db, &path);
        let end = laid.lines.first().map_or(0, |l| l.end);
        assert!(laid.overflow && end > 0 && end < text.len(), "the path's line ends where its text stops: {end}");
        let (a, b, c) = (frame(0.0), path_frame(100.0), frame(400.0));
        let parts: Vec<Vec<TextRun>> = distribute(db, &[&a, &b, &c], &story, &[]).into_iter().map(|s| s.runs).collect();
        let lens: Vec<usize> = parts.iter().map(|p| runs_len(p)).collect();
        assert!(lens.iter().all(|l| *l > 0), "each frame takes a share: {lens:?}");
        let joined: String = parts.iter().flatten().map(|r| r.text.as_str()).collect();
        assert_eq!(joined, text, "no character lost or duplicated");
        let mut on_path = b.clone();
        on_path.runs = parts[1].clone();
        assert!(!layout(db, &on_path).overflow, "the path's share fits it");
    }

    /// A path that runs out of room at a space holds only what it shows: the space starts the
    /// next frame, as a space with no room on a path overflows.
    #[test]
    fn a_path_that_runs_out_at_a_space_leaves_it_to_the_next_frame() {
        let db = FontDb::global();
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(6);
        let story = vec![TextRun { text: text.clone(), style: CharStyle::default(), inline: None }];
        let mut at_space = 0;
        for length in (60..160).step_by(3) {
            let (a, b) = (path_frame(f64::from(length)), frame(400.0));
            let parts: Vec<Vec<TextRun>> = distribute(db, &[&a, &b], &story, &[]).into_iter().map(|s| s.runs).collect();
            let joined: String = parts.iter().flatten().map(|r| r.text.as_str()).collect();
            assert_eq!(joined, text, "no character lost or duplicated");
            let mut on_path = a.clone();
            on_path.runs = parts[0].clone();
            assert!(!layout(db, &on_path).overflow, "the path's share fits it ({length})");
            at_space += usize::from(parts[1].iter().map(|r| r.text.as_str()).collect::<String>().starts_with(' '));
        }
        assert!(at_space > 0, "some lengths end at a space");
    }

    /// A right-to-left story flows on from where its text stops fitting a path, in reading order.
    #[test]
    fn right_to_left_story_flows_on_from_a_path() {
        let db = FontDb::global();
        let text = "שלום עולם ".repeat(8);
        let story = vec![TextRun { text: text.clone(), style: CharStyle::default(), inline: None }];
        let (a, b) = (path_frame(100.0), frame(400.0));
        let parts: Vec<Vec<TextRun>> = distribute(db, &[&a, &b], &story, &[]).into_iter().map(|s| s.runs).collect();
        let lens: Vec<usize> = parts.iter().map(|p| runs_len(p)).collect();
        assert!(lens.iter().all(|l| *l > 0), "each frame takes a share: {lens:?}");
        let joined: String = parts.iter().flatten().map(|r| r.text.as_str()).collect();
        assert_eq!(joined, text, "no character lost or duplicated");
        let mut on_path = a.clone();
        on_path.runs = parts[0].clone();
        assert!(!layout(db, &on_path).overflow, "the path's share fits it");
    }

    /// A path with no length holds no text, so the story flows past it.
    #[test]
    fn story_flows_past_a_path_with_no_length() {
        let db = FontDb::global();
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(6);
        let story = vec![TextRun { text: text.clone(), style: CharStyle::default(), inline: None }];
        let (a, b, c) = (frame(0.0), path_frame(0.0), frame(400.0));
        let parts: Vec<Vec<TextRun>> = distribute(db, &[&a, &b, &c], &story, &[]).into_iter().map(|s| s.runs).collect();
        let lens: Vec<usize> = parts.iter().map(|p| runs_len(p)).collect();
        assert!(lens[0] > 0 && lens[1] == 0 && lens[2] > 0, "{lens:?}");
        let joined: String = parts.iter().flatten().map(|r| r.text.as_str()).collect();
        assert_eq!(joined, text, "no character lost or duplicated");
    }

    #[test]
    fn story_flows_across_frames_and_rejoins() {
        let db = FontDb::global();
        let (a, b, c) = (frame(0.0), frame(200.0), frame(400.0));
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(6) + "\nSecond paragraph here.";
        let story = vec![TextRun { text: text.clone(), style: CharStyle::default(), inline: None }];
        let parts: Vec<Vec<TextRun>> = distribute(db, &[&a, &b, &c], &story, &[]).into_iter().map(|s| s.runs).collect();
        let lens: Vec<usize> = parts.iter().map(|p| runs_len(p)).collect();
        assert!(lens[0] > 0 && lens[1] > 0, "{lens:?}");
        let joined: String = parts.iter().flatten().map(|r| r.text.as_str()).collect();
        assert_eq!(joined, text, "no character lost or duplicated");
        // Each non-last slice fits its frame.
        for (f, p) in [&a, &b].iter().zip(&parts) {
            let mut t = (*f).clone();
            t.runs = p.clone();
            assert!(!layout(db, &t).overflow);
        }
        // Lines never start with the break space.
        assert!(!parts[1][0].text.starts_with(' '));
    }

    #[test]
    fn paragraph_styles_flow_with_their_paragraphs() {
        let db = FontDb::global();
        let (a, b) = (frame(0.0), frame(200.0));
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(4) + "\nSecond paragraph.\nThird.";
        let story = vec![TextRun { text, style: CharStyle::default(), inline: None }];
        let style = |j| ParaStyle { justify: j, ..Default::default() };
        use vectorcraft_doc::Justify::*;
        let paras = vec![style(Left), style(Center), style(Right)];
        let parts = distribute(db, &[&a, &b], &story, &paras);
        assert!(!parts[0].runs.is_empty() && !parts[1].runs.is_empty());
        // The first paragraph overflows into the second frame: it is in both, with its style.
        let pa: Vec<Justify> = parts[0].paras.iter().map(|p| p.justify).collect();
        let pb: Vec<Justify> = parts[1].paras.iter().map(|p| p.justify).collect();
        assert_eq!(pa, [Left]);
        assert_eq!(pb, [Left, Center, Right]);
        // Joined again, the shared paragraph counts once.
        let mut fa = a.clone();
        fa.runs = parts[0].runs.clone();
        fa.set_paragraph_styles(parts[0].paras.clone());
        let mut fb = b.clone();
        fb.runs = parts[1].runs.clone();
        fb.set_paragraph_styles(parts[1].paras.clone());
        assert_eq!(story_paras(&[&fa, &fb], &[]), paras);
        // A frame whose shared first paragraph was restyled wins.
        fb.edit_paras(Some(0..1), |p| p.justify = JustifyAll);
        assert_eq!(story_paras(&[&fa, &fb], &[false, true])[0].justify, JustifyAll);
        assert_eq!(story_paras(&[&fa, &fb], &[false, false])[0].justify, Left);
    }

    #[test]
    fn short_story_leaves_later_frames_empty_with_a_style() {
        let db = FontDb::global();
        let (a, b) = (frame(0.0), frame(200.0));
        let st = CharStyle { size: 20.0, ..Default::default() };
        let parts: Vec<Vec<TextRun>> =
            distribute(db, &[&a, &b], &[TextRun { text: "Hi".into(), style: st, inline: None }], &[]).into_iter().map(|s| s.runs).collect();
        assert_eq!(parts[0][0].text, "Hi");
        assert_eq!((parts[1][0].text.as_str(), parts[1][0].style.size), ("", 20.0));
    }
}
