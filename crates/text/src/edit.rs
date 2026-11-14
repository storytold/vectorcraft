//! Rich-text editing on style runs (range replace, range styling with run splitting) and
//! character/word/paragraph navigation on plain text. Shared by the engine commands
//! (`text.editRange`, `text.setRangeStyle`) and the Type tool.

use std::ops::Range;

use vectorcraft_doc::text::INLINE_CHAR;
use vectorcraft_doc::{CharStyle, TextRun};

/// Is `s` exactly one object replacement character (an inline graphic run's text)?
pub fn is_inline_text(s: &str) -> bool {
    let mut c = s.chars();
    c.next() == Some(INLINE_CHAR) && c.next().is_none()
}

/// Total byte length of `runs`.
pub fn runs_len(runs: &[TextRun]) -> usize {
    runs.iter().map(|r| r.text.len()).sum()
}

/// Clamp `b` to `s.len()` and back to a char boundary.
pub fn floor_char(s: &str, b: usize) -> usize {
    let mut b = b.min(s.len());
    while b > 0 && !s.is_char_boundary(b) {
        b -= 1;
    }
    b
}

/// Style of the character at `byte` (the character before it when `byte` is at a run end or the
/// text end), used for inserted text.
pub fn style_at(runs: &[TextRun], byte: usize) -> CharStyle {
    let mut off = 0;
    let mut last = None;
    for r in runs {
        let end = off + r.text.len();
        if byte < end || (byte == end && byte == off) {
            return r.style.clone();
        }
        if !r.text.is_empty() {
            last = Some(&r.style);
            if byte == end {
                return r.style.clone();
            }
        }
        off = end;
    }
    last.or(runs.first().map(|r| &r.style)).cloned().unwrap_or_default()
}

/// Style for text typed over `start..end` (Illustrator: the first replaced character's style, or
/// the character before the caret), without a glyph choice ([`CharStyle::glyph_alternate`]):
/// typed characters take their default glyphs.
pub fn insertion_style(runs: &[TextRun], start: usize, end: usize) -> CharStyle {
    let mut style = 'found: {
        if end > start {
            // The character at `start` (not the one before it).
            let mut off = 0;
            for r in runs {
                if start < off + r.text.len() {
                    break 'found r.style.clone();
                }
                off += r.text.len();
            }
        }
        style_at(runs, start)
    };
    style.glyph_alternate = None;
    style
}

/// Merge adjacent runs with equal styles and drop empty runs (keeping one run so the object
/// remembers its style). Inline graphic runs never merge: each stays one [`INLINE_CHAR`]; one
/// whose text is anything else (malformed input) becomes plain text. A run with a glyph choice
/// ([`CharStyle::glyph_alternate`]) is not merged into the run before it, and loses the choice
/// when its text does not start with the character the choice is for.
pub fn normalize(runs: &mut Vec<TextRun>) {
    let first_style = runs.first().map(|r| r.style.clone());
    let mut out: Vec<TextRun> = Vec::with_capacity(runs.len());
    for mut r in runs.drain(..) {
        if r.text.is_empty() {
            continue;
        }
        if r.inline.is_some() && !is_inline_text(&r.text) {
            r.inline = None;
        }
        if r.style.glyph_alternate.as_ref().is_some_and(|c| !r.text.starts_with(c.ch)) {
            r.style.glyph_alternate = None;
        }
        match out.last_mut() {
            Some(l) if l.style == r.style && l.inline.is_none() && r.inline.is_none() && r.style.glyph_alternate.is_none() => {
                l.text.push_str(&r.text)
            }
            _ => out.push(r),
        }
    }
    if out.is_empty() {
        out.push(TextRun { text: String::new(), style: first_style.unwrap_or_default(), inline: None });
    }
    *runs = out;
}

/// Split runs so that `byte` falls on a run boundary; returns the index of the run starting at
/// `byte` (== `runs.len()` at the end).
fn split_at(runs: &mut Vec<TextRun>, byte: usize) -> usize {
    let mut off = 0;
    for i in 0..runs.len() {
        let len = runs[i].text.len();
        if byte == off {
            return i;
        }
        if byte < off + len {
            let at = floor_char(&runs[i].text, byte - off);
            let tail = runs[i].text.split_off(at);
            // A glyph choice is for the run's first character, which stays in the head.
            let style = CharStyle { glyph_alternate: None, ..runs[i].style.clone() };
            // An inline graphic is one character: a split lands before it (`at` == 0), the empty
            // head is dropped by `normalize` and the tail keeps the graphic.
            let inline = runs[i].inline.clone();
            runs.insert(i + 1, TextRun { text: tail, style, inline });
            return i + 1;
        }
        off += len;
    }
    runs.len()
}

/// Replace `start..end` of the plain text with `insert`, styled like the replaced text (see
/// [`insertion_style`]). Offsets are clamped to char boundaries. Returns the caret after the
/// insertion.
pub fn replace_range(runs: &mut Vec<TextRun>, start: usize, end: usize, insert: &str) -> usize {
    let style = insertion_style(runs, start, end);
    replace_range_styled(runs, start, end, &[TextRun { text: insert.to_string(), style, inline: None }])
}

/// Replace `start..end` with styled runs (paste with formatting). Returns the caret after them.
pub fn replace_range_styled(runs: &mut Vec<TextRun>, start: usize, end: usize, insert: &[TextRun]) -> usize {
    let plain: String = runs.iter().map(|r| r.text.as_str()).collect();
    let (mut a, mut b) = (floor_char(&plain, start), floor_char(&plain, end));
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    // Taken before the splice: deleting all of the text leaves no run for `normalize` to take the
    // style from, and the next typed text would get the default one (#2119).
    let keep = insertion_style(runs, a, b);
    let i = split_at(runs, a);
    let j = split_at(runs, b);
    runs.splice(i..j, insert.iter().cloned());
    if runs.is_empty() {
        runs.push(TextRun { text: String::new(), style: keep, inline: None });
    }
    normalize(runs);
    a + insert.iter().map(|r| r.text.len()).sum::<usize>()
}

/// Apply `f` to the character style of `start..end`, splitting runs at the range ends.
pub fn style_range(runs: &mut Vec<TextRun>, start: usize, end: usize, mut f: impl FnMut(&mut CharStyle)) {
    let plain: String = runs.iter().map(|r| r.text.as_str()).collect();
    let (mut a, mut b) = (floor_char(&plain, start), floor_char(&plain, end));
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    if a == b {
        // Empty text: style the (single, empty) run so the next typed text uses it.
        if plain.is_empty() {
            for r in runs.iter_mut() {
                f(&mut r.style);
            }
        }
        return;
    }
    let i = split_at(runs, a);
    let j = split_at(runs, b);
    for r in &mut runs[i..j] {
        f(&mut r.style);
    }
    normalize(runs);
}

/// Styled copy of `start..end` (for copy/cut with formatting).
pub fn slice_runs(runs: &[TextRun], start: usize, end: usize) -> Vec<TextRun> {
    let mut out = vec![];
    let mut off = 0;
    for r in runs {
        let (rs, re) = (off, off + r.text.len());
        let (a, b) = (start.max(rs), end.min(re));
        if a < b {
            let t = &r.text[floor_char(&r.text, a - rs)..floor_char(&r.text, b - rs)];
            if t.is_empty() {
                off = re;
                continue;
            }
            let mut style = r.style.clone();
            // A glyph choice is for the run's first character.
            if floor_char(&r.text, a - rs) > 0 {
                style.glyph_alternate = None;
            }
            out.push(TextRun { text: t.to_string(), style, inline: r.inline.clone() });
        }
        off = re;
    }
    out
}

// ---------- navigation on plain text ----------

pub fn prev_char(s: &str, i: usize) -> usize {
    let i = floor_char(s, i);
    s[..i].char_indices().next_back().map(|(k, _)| k).unwrap_or(0)
}

pub fn next_char(s: &str, i: usize) -> usize {
    let i = floor_char(s, i);
    s[i..].chars().next().map(|c| i + c.len_utf8()).unwrap_or(s.len())
}

/// Does the character at byte `i` of `s` start a line by itself: the first character of the text,
/// of a paragraph, or after a forced line break (U+2028)?
pub fn starts_line(s: &str, i: usize) -> bool {
    i == 0 || s.get(..i).is_some_and(|before| before.ends_with(['\n', '\u{2028}']))
}

/// The character whose kerning the Type tool's caret at byte `i` of `s` sets (the Kerning field,
/// Alt+arrows, Reset Tracking and Kerning), and whether that kerning is the space before it
/// ([`vectorcraft_doc::CharStyle::kerning_before`]) or after it
/// ([`vectorcraft_doc::CharStyle::kerning`]). Inside a line it is the space after the character
/// before the caret. At the start of a line ([`starts_line`]) no character on the line comes before
/// the caret, so it is the space before the character after it (#5111). None at the start of an
/// empty line, where no character follows either.
pub fn caret_kerning_target(s: &str, i: usize) -> Option<(Range<usize>, bool)> {
    let i = floor_char(s, i);
    if !starts_line(s, i) {
        return Some((prev_char(s, i)..i, false));
    }
    let c = s.get(i..)?.chars().next()?;
    (c != '\n' && c != '\u{2028}').then(|| (i..i + c.len_utf8(), true))
}

/// The start of the grapheme cluster before `i` (Left arrow): a Thai consonant with its vowel and
/// tone marks, an accented letter or an emoji sequence is passed over whole.
pub fn prev_cluster(s: &str, i: usize) -> usize {
    use unicode_segmentation::GraphemeCursor;
    let i = floor_char(s, i);
    GraphemeCursor::new(i, s.len(), true).prev_boundary(s, 0).ok().flatten().unwrap_or(0)
}

/// The end of the grapheme cluster at `i` (Right arrow, Delete): see [`prev_cluster`].
pub fn next_cluster(s: &str, i: usize) -> usize {
    use unicode_segmentation::GraphemeCursor;
    let i = floor_char(s, i);
    GraphemeCursor::new(i, s.len(), true).next_boundary(s, 0).ok().flatten().unwrap_or(s.len())
}

/// Part of a word: a letter, digit or mark (Thai vowel and tone marks, combining accents), an
/// underscore or an apostrophe.
fn is_word_char(c: char) -> bool {
    use unicode_general_category::{GeneralCategory as G, get_general_category};
    c.is_alphanumeric() || matches!(c, '_' | '\'' | '’') || matches!(get_general_category(c), G::NonspacingMark | G::SpacingMark | G::EnclosingMark)
}

/// Is byte `b` of `s` (with `breaks`, its [`crate::shape::unspaced_word_breaks`]) a boundary
/// between two words of Thai, Lao, Khmer or Myanmar, which no space separates?
fn inner_word_break(s: &str, breaks: &[usize], b: usize) -> bool {
    use crate::shape::is_unspaced_script;
    breaks.binary_search(&b).is_ok()
        && s.get(b..).and_then(|t| t.chars().next()).is_some_and(is_unspaced_script)
        && s.get(..b).and_then(|t| t.chars().next_back()).is_some_and(is_unspaced_script)
}

/// Start of the previous word (Cmd/Alt+Left). Thai (Lao, Khmer, Myanmar) words end where their
/// script's word boundaries are, spaces or not.
pub fn prev_word(s: &str, i: usize) -> usize {
    let i = floor_char(s, i);
    let breaks = crate::shape::unspaced_word_breaks(s);
    let chars: Vec<(usize, char)> = s[..i].char_indices().collect();
    let mut k = chars.len();
    while k > 0 && !is_word_char(chars[k - 1].1) {
        k -= 1;
    }
    let end = k;
    while k > 0 && is_word_char(chars[k - 1].1) && !(k < end && inner_word_break(s, &breaks, chars[k].0)) {
        k -= 1;
    }
    chars.get(k).map(|c| c.0).unwrap_or(i)
}

/// End of the next word (Cmd/Alt+Right): see [`prev_word`].
pub fn next_word(s: &str, i: usize) -> usize {
    let i = floor_char(s, i);
    let breaks = crate::shape::unspaced_word_breaks(s);
    let mut it = s[i..].char_indices().peekable();
    while it.peek().is_some_and(|(_, c)| !is_word_char(*c)) {
        it.next();
    }
    let start = it.peek().map(|(k, _)| i + k);
    while it.peek().is_some_and(|(k, c)| is_word_char(*c) && !(Some(i + k) != start && inner_word_break(s, &breaks, i + k))) {
        it.next();
    }
    it.peek().map(|(k, _)| i + k).unwrap_or(s.len())
}

/// The word (or run of spaces / single punctuation) at `i` (double-click selection).
pub fn word_at(s: &str, i: usize) -> Range<usize> {
    let i = floor_char(s, i);
    let (next, prev) = (s[i..].chars().next(), s[..i].chars().next_back());
    // At a word's end, the word wins over the following space or punctuation.
    let (c, at) = match (next, prev) {
        (Some(n), Some(p)) if !is_word_char(n) && is_word_char(p) => (p, i - p.len_utf8()),
        (Some(n), _) => (n, i),
        (None, Some(p)) => (p, i - p.len_utf8()),
        (None, None) => return i..i,
    };
    let class = |c: char| {
        if is_word_char(c) {
            0
        } else if c.is_whitespace() && c != '\n' {
            1
        } else {
            2
        }
    };
    let k = class(c);
    if k == 2 {
        return at..at + c.len_utf8();
    }
    // Thai (Lao, Khmer, Myanmar) words also end where their script's word boundaries are.
    let breaks = if k == 0 { crate::shape::unspaced_word_breaks(s) } else { vec![] };
    let mut a = at;
    for (p, d) in s[..at].char_indices().rev() {
        if class(d) != k || inner_word_break(s, &breaks, a) {
            break;
        }
        a = p;
    }
    let mut b = at;
    for (p, d) in s[at..].char_indices() {
        if class(d) != k || (b > at && inner_word_break(s, &breaks, b)) {
            break;
        }
        b = at + p + d.len_utf8();
    }
    a..b
}

/// The paragraph containing `i`, excluding its `\n` (triple-click selection).
pub fn paragraph_at(s: &str, i: usize) -> Range<usize> {
    let i = floor_char(s, i);
    let a = s[..i].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let b = s[i..].find('\n').map(|p| i + p).unwrap_or(s.len());
    a..b
}
