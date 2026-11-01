//! Paragraph panel › Insert Kashidas: justified lines lengthen their Arabic words with kashidas.
//!
//! [`plan`] picks, in each Arabic word of a justified line, the place between its last pair of
//! joined letters, and shares the line's room among those places, each kashida up to its
//! [`Kashida::max_em`]. The layout draws each kashida as tatweels (U+0640) in the face of the
//! letter after it, and the word spaces take the room the kashidas leave.
//!
//! Behavior reference (public documentation):
//! <https://web.archive.org/web/20260906105741/https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/justify-text-using-kashida-and-hyphenation.html>
//! and <https://web.archive.org/web/20250816144754/https://helpx.adobe.com/illustrator/using/arabic-hebrew.html>.

use std::ops::Range;

use vectorcraft_doc::text::Kashida;

use crate::shape::SGlyph;

/// The tatweel (kashida) character.
pub(crate) const TATWEEL: char = '\u{0640}';

/// How a character joins its neighbors in cursive Arabic, from the Unicode Character Database's
/// `ArabicShaping.txt` for the Arabic block (U+0600 to U+06FF). Other characters do not join,
/// except the zero width joiner, and the nonspacing and enclosing marks and format characters
/// that the file does not list, which joining goes across.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Joining {
    /// Joins the letters on both sides.
    Dual,
    /// Joins the letter before it only.
    Right,
    /// Joins both sides without changing its own form (the tatweel, the zero width joiner).
    Causing,
    /// Skipped over: joining goes on across it (nonspacing and enclosing marks, format
    /// characters).
    Transparent,
    None,
}

fn joining(c: char) -> Joining {
    use unicode_general_category::{GeneralCategory, get_general_category};
    match c {
        '\u{0620}'
        | '\u{0626}'
        | '\u{0628}'
        | '\u{062A}'..='\u{062E}'
        | '\u{0633}'..='\u{063F}'
        | '\u{0641}'..='\u{0647}'
        | '\u{0649}'
        | '\u{064A}' => Joining::Dual,
        '\u{066E}' | '\u{066F}' | '\u{0678}'..='\u{0687}' | '\u{069A}'..='\u{06BF}' | '\u{06C1}' | '\u{06C2}' | '\u{06CC}' | '\u{06CE}' => {
            Joining::Dual
        }
        '\u{06D0}' | '\u{06D1}' | '\u{06FA}'..='\u{06FC}' | '\u{06FF}' => Joining::Dual,
        '\u{0622}'..='\u{0625}'
        | '\u{0627}'
        | '\u{0629}'
        | '\u{062F}'..='\u{0632}'
        | '\u{0648}'
        | '\u{0671}'..='\u{0673}'
        | '\u{0675}'..='\u{0677}' => Joining::Right,
        '\u{0688}'..='\u{0699}'
        | '\u{06C0}'
        | '\u{06C3}'..='\u{06CB}'
        | '\u{06CD}'
        | '\u{06CF}'
        | '\u{06D2}'
        | '\u{06D3}'
        | '\u{06D5}'
        | '\u{06EE}'
        | '\u{06EF}' => Joining::Right,
        TATWEEL | '\u{200D}' => Joining::Causing,
        '\u{0600}'..='\u{0605}' | '\u{06DD}' | '\u{200C}' => Joining::None,
        _ if matches!(get_general_category(c), GeneralCategory::NonspacingMark | GeneralCategory::EnclosingMark | GeneralCategory::Format) => {
            Joining::Transparent
        }
        _ => Joining::None,
    }
}

/// Is the cluster of glyph `a` joined to the cluster of glyph `b` that follows it in `text`: both
/// right to left in one face, `a`'s last character other than a mark joining the letter after it,
/// and `b` starting with a letter that joins the one before it?
fn joined(text: &str, a: &SGlyph, b: &SGlyph) -> bool {
    if a.face.id() != b.face.id() || a.level != b.level || !a.level.is_rtl() || a.byte + a.len != b.byte {
        return false;
    }
    let last = text.get(a.byte..a.byte + a.len).and_then(|s| s.chars().rev().find(|&c| joining(c) != Joining::Transparent));
    let first = text.get(b.byte..b.byte + b.len).and_then(|s| s.chars().next());
    matches!(last.map(joining), Some(Joining::Dual | Joining::Causing))
        && matches!(first.map(joining), Some(Joining::Dual | Joining::Right | Joining::Causing))
}

/// The tatweel of `g`'s face and its advance in points at `g`'s scale; None when the face has
/// no tatweel.
pub(crate) fn tatweel(g: &SGlyph) -> Option<(u32, f64)> {
    let gid = g.face.glyph_for(TATWEEL);
    let advance = g.face.advance(gid) * g.sx;
    (gid != 0 && advance.is_finite() && advance > 0.0).then_some((gid, advance))
}

/// The most tatweels one Stylistic kashida is drawn with.
pub(crate) const MAX_TATWEELS: f64 = 64.0;

/// The kashidas of the glyphs `line` (indices into `sg`, a paragraph's glyphs in logical order;
/// `text` is the plain text) of a justified line with `room` points to fill: for each Arabic
/// word with joined letters, at its last pair of them, the byte offset of the cluster after the kashida and
/// the kashida's length, in text order. Words end at spaces, inline graphics, and characters other than letters
/// and marks (punctuation, digits, symbols). Every kashida takes an equal share of `room`, up to
/// `kind`'s longest in ems of the letter after it (as wide as its horizontal scale draws it); a
/// Stylistic kashida is a whole number of tatweels, at most [`MAX_TATWEELS`], and a word whose
/// share is shorter than one tatweel gets none. A line of words without joined letters, or whose
/// font has no tatweel, gets none.
pub(crate) fn plan(text: &str, sg: &[SGlyph], line: Range<usize>, kind: Kashida, room: f64) -> Vec<(usize, f64)> {
    if kind == Kashida::None || !room.is_finite() || room <= 0.0 {
        return vec![];
    }
    // The first glyph of the cluster after each word's last joint.
    let mut picks: Vec<usize> = Vec::new();
    let mut word: Option<usize> = None;
    let mut prev: Option<usize> = None;
    let mut j = line.start;
    while j < line.end {
        let Some(g) = sg.get(j) else { break };
        let end = (j + 1..line.end).find(|&e| sg.get(e).is_none_or(|n| n.byte != g.byte)).unwrap_or(line.end);
        let first = text.get(g.byte..g.byte + g.len).and_then(|s| s.chars().next());
        let not_a_letter = first.is_some_and(|c| !c.is_alphabetic() && joining(c) != Joining::Transparent);
        if g.is_space() || g.inline.is_some() || not_a_letter {
            picks.extend(word.take());
            prev = None;
        } else {
            if prev.and_then(|p| sg.get(p)).is_some_and(|a| joined(text, a, g)) && tatweel(g).is_some() {
                word = Some(j);
            }
            prev = Some(j);
        }
        j = end;
    }
    picks.extend(word);
    if picks.is_empty() {
        return vec![];
    }
    let share = room / picks.len() as f64;
    let mut out = Vec::with_capacity(picks.len());
    for g in picks.iter().filter_map(|&k| sg.get(k)) {
        let Some((_, t)) = tatweel(g) else { continue };
        let longest = kind.max_em() * g.face.units_per_em() * g.sx;
        let mut len = share.min(longest);
        if kind == Kashida::Stylistic {
            len = (len / t).floor().min(MAX_TATWEELS) * t;
        }
        if len.is_finite() && len > 1e-6 {
            out.push((g.byte, len));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Beh joins on both sides, alef and waw only to the letter before them, hamza to neither;
    /// marks and the format characters the file does not list are skipped, the number signs and
    /// the end of ayah it lists join neither side, and the tatweel and the zero width joiner join
    /// both sides.
    #[test]
    fn arabic_letters_join_as_arabic_shaping_lists_them() {
        assert_eq!(joining('ب'), Joining::Dual);
        assert_eq!(joining('ي'), Joining::Dual);
        assert_eq!(joining('ک'), Joining::Dual);
        assert_eq!(joining('ا'), Joining::Right);
        assert_eq!(joining('و'), Joining::Right);
        assert_eq!(joining('ة'), Joining::Right);
        assert_eq!(joining('ء'), Joining::None);
        assert_eq!(joining('\u{064E}'), Joining::Transparent);
        assert_eq!(joining('\u{061C}'), Joining::Transparent);
        assert_eq!(joining('\u{0600}'), Joining::None);
        assert_eq!(joining('\u{0605}'), Joining::None);
        assert_eq!(joining('\u{06DD}'), Joining::None);
        assert_eq!(joining(TATWEEL), Joining::Causing);
        assert_eq!(joining('\u{200D}'), Joining::Causing);
        assert_eq!(joining('\u{200C}'), Joining::None);
        assert_eq!(joining('a'), Joining::None);
    }
}
