//! Where each text object of an editing copy is framed: the `/Art` path an `/AI11Text` object
//! carries (its area type's frame, or the path type on a path runs along), by the story and frame
//! the object shows. The text document's frame of a copied object keeps its original's path, so
//! read alone it puts every copy on the original; the object's own `/Art` path is where the file
//! places it, in the art space's path operators.
//!
//! An `/AI11Text` object is written as `/AI11Text :` … `n /FrameIndex ,` `n /StoryIndex ,`
//! `/Art :` (its path) `;` … `/ConfiningPath ,` (point type has no `/Art`), its lines plain or each
//! led by `%_`.

use std::collections::HashMap;

use kurbo::{BezPath, Point};

use super::find;
use super::lex::number;

/// The most text objects read.
const MAX_OBJECTS: usize = 100_000;
/// How far past `/AI11Text :` an object's `/Art` path may end (bytes).
const MAX_OBJECT: usize = 256 << 10;
const OBJECT: &[u8] = b"/AI11Text :";

/// The `/Art` path of each text object of `data` (an editing copy's document text), in art space
/// (y up), by its story and frame. Objects without a readable path are left out.
pub(crate) fn text_frames(data: &[u8]) -> HashMap<(u32, usize), BezPath> {
    let mut out = HashMap::new();
    let mut at = 0;
    while let Some(i) = data.get(at..).and_then(|d| find(d, OBJECT)) {
        let start = at + i;
        at = start + OBJECT.len();
        if out.len() >= MAX_OBJECTS {
            break;
        }
        let Some(seg) = data.get(start..(start + MAX_OBJECT).min(data.len())) else { break };
        // The object ends where the next one starts.
        let seg = seg.get(OBJECT.len()..).and_then(|rest| find(rest, OBJECT)).and_then(|j| seg.get(..j + OBJECT.len())).unwrap_or(seg);
        let (Some(art), Some(end)) = (find(seg, b"/Art :"), find(seg, b"/ConfiningPath")) else { continue };
        if end < art {
            continue;
        }
        let Some(head) = seg.get(..art) else { continue };
        // The type in a drawn look is a story of another text document (see `Reader::text_slot`).
        if number_before(head, b"/FreeUndo") == Some(1.0) {
            continue;
        }
        let (Some(story), frame) = (number_before(head, b"/StoryIndex"), number_before(head, b"/FrameIndex").unwrap_or(0.0)) else { continue };
        if !(0.0..f64::from(u32::MAX)).contains(&story) || !(0.0..1e6).contains(&frame) {
            continue;
        }
        if let Some(p) = seg.get(art + 6..end).and_then(path) {
            out.entry((story as u32, frame as usize)).or_insert(p);
        }
    }
    out
}

/// The words of `text`, without the `%_` that leads lines inside dictionaries.
fn words(text: &[u8]) -> impl Iterator<Item = &[u8]> {
    text.split(|b| b.is_ascii_whitespace()).map(|w| w.strip_prefix(b"%_").unwrap_or(w)).filter(|w| !w.is_empty())
}

/// The number written just before `key` (`49 /StoryIndex`) in `head`.
fn number_before(head: &[u8], key: &[u8]) -> Option<f64> {
    let k = find(head, key)?;
    number(words(head.get(..k)?).last()?)
}

/// A path in the format's path operators (`m`, `l`/`L`, `c`/`C`, `v`/`V`, `y`/`Y`; any other
/// operator ends the numbers before it).
fn path(text: &[u8]) -> Option<BezPath> {
    let mut bp = BezPath::new();
    let mut nums: Vec<f64> = vec![];
    let mut cur: Option<Point> = None;
    for w in words(text) {
        if let Some(v) = number(w) {
            nums.push(v);
            continue;
        }
        let p = |i: usize| -> Option<Point> { Some(Point::new(*nums.get(i)?, *nums.get(i + 1)?)) };
        let n = nums.len();
        match w {
            b"m" if n >= 2 => {
                let q = p(n - 2)?;
                bp.move_to(q);
                cur = Some(q);
            }
            b"l" | b"L" if n >= 2 && cur.is_some() => {
                let q = p(n - 2)?;
                bp.line_to(q);
                cur = Some(q);
            }
            b"c" | b"C" if n >= 6 && cur.is_some() => {
                let q = p(n - 2)?;
                bp.curve_to(p(n - 6)?, p(n - 4)?, q);
                cur = Some(q);
            }
            b"v" | b"V" if n >= 4 => {
                let c0 = cur?;
                let q = p(n - 2)?;
                bp.curve_to(c0, p(n - 4)?, q);
                cur = Some(q);
            }
            b"y" | b"Y" if n >= 4 && cur.is_some() => {
                let q = p(n - 2)?;
                bp.curve_to(p(n - 4)?, q, q);
                cur = Some(q);
            }
            _ => {}
        }
        nums.clear();
    }
    (bp.elements().len() >= 2).then_some(bp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    #[test]
    fn each_text_object_has_its_own_frame_by_story_and_frame() {
        let doc = b"/AI11Text :\r0 /FreeUndo ,\r0 /FrameIndex ,\r49 /StoryIndex ,\r/Art :\rX=\r7 As\r10 20 m\r110 20 L\r110 80 L\rN\rX+\r;\r/ConfiningPath ,\r;\r\
            /AI11Text :\r%_0 /FreeUndo ,\r%_1 /FrameIndex ,\r%_60 /StoryIndex ,\r%_/Art :\r%_X=\r%_300 20 m\r%_340 20 350 30 360 40 c\r%_N\r%_;\r%_/ConfiningPath ,\r%_;\r\
            /AI11Text :\r1 /FreeUndo ,\r0 /FrameIndex ,\r33 /StoryIndex ,\r2 /TextAntialiasing ,\r;\r";
        let f = text_frames(doc);
        assert_eq!(f.len(), 2, "point type has no frame: {f:?}");
        assert_eq!(f[&(49, 0)].bounding_box(), kurbo::Rect::new(10.0, 20.0, 110.0, 80.0));
        assert_eq!(f[&(60, 1)].elements().len(), 2, "a curve, led by %_");
        // The type in a drawn look counts its stories in another text document: its frame isn't
        // that of the text object which has the same number in this one.
        let look = String::from_utf8_lossy(doc).replacen("0 /FreeUndo", "1 /FreeUndo", 1).replacen("10 20 m", "500 20 m", 1);
        let own = "/AI11Text :\r0 /FreeUndo ,\r0 /FrameIndex ,\r49 /StoryIndex ,\r/Art :\r1 2 m\r3 2 L\r3 4 L\rN\r;\r/ConfiningPath ,\r;\r";
        let f = text_frames(format!("{look}{own}").as_bytes());
        assert_eq!(f[&(49, 0)].bounding_box(), kurbo::Rect::new(1.0, 2.0, 3.0, 4.0));
        assert!(text_frames(b"/AI11Text : garbage").is_empty());
    }
}
