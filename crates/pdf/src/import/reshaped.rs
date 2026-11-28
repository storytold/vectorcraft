//! Type in a script whose letters reorder and join as they are set (#5235). A PDF draws such text
//! as glyphs already shaped: a vowel sign before the consonant it follows in reading order, a
//! conjunct as one glyph that the file may not name, a mark placed over its base. Read glyph by
//! glyph, the characters don't always make the text that was set, and type made from them draws
//! other glyphs in other places. A line of such type is therefore set again and compared with what
//! the file drew; one that would look different keeps the file's glyphs as outlines.

use kurbo::{Rect, Shape};
use vectorcraft_doc::TextObject;

/// Does `text` have a character of a script whose letters reorder and join into conjuncts: the
/// Brahmic scripts of U+0900 to U+0DFF (Devanagari, Bengali, Gurmukhi, Gujarati, Oriya, Tamil,
/// Telugu, Kannada, Malayalam, Sinhala)?
pub(super) fn reshapes(text: &str) -> bool {
    text.chars().any(|c| ('\u{0900}'..='\u{0DFF}').contains(&c))
}

/// Would type `t` draw its glyphs where the file drew them (`ink`: each glyph's ink box in
/// document space)? As many glyphs with ink, each within 3 % of the type's size of one of the
/// file's.
pub(super) fn draws_as(t: &TextObject, ink: &[Rect]) -> bool {
    let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let mut set: Vec<Rect> =
        laid.glyphs.iter().filter(|g| !g.outline.elements().is_empty()).map(|g| (t.xf * g.outline.clone()).bounding_box()).collect();
    if set.len() != ink.len() {
        return false;
    }
    let size = t.runs.iter().map(|r| r.style.size).fold(0.0, f64::max);
    let tol = size * 0.03;
    if !tol.is_finite() || set.iter().chain(ink).any(|r| !(r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite())) {
        return false;
    }
    set.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    let mut used = vec![false; set.len()];
    // Each of the file's glyphs takes one the type draws in its place: looked for among those
    // whose left edge is as near.
    ink.iter().all(|r| {
        let from = set.partition_point(|s| s.x0 < r.x0 - tol);
        let found = set
            .iter()
            .zip(used.iter_mut())
            .skip(from)
            .take_while(|(s, _)| s.x0 <= r.x0 + tol)
            .find(|(s, used)| !**used && (s.y0 - r.y0).abs() <= tol && (s.x1 - r.x1).abs() <= tol && (s.y1 - r.y1).abs() <= tol);
        match found {
            Some((_, used)) => {
                *used = true;
                true
            }
            None => false,
        }
    })
}
