use super::{Range, ToolKey};

/// Map Alt+arrows to one type step. Horizontal ←/→ changes kerning at a caret or
/// tracking on a range, ↑/↓ changes leading, and Shift+↑/↓ changes baseline shift.
/// Vertical ↑/↓ (with or without Shift) changes kerning or tracking, ←/→ changes
/// leading, and Shift+→/← changes baseline shift.
pub fn step_key(key: ToolKey, shift: bool, caret: bool, vertical: bool) -> Option<(&'static str, f64)> {
    let shift = shift && !(vertical && matches!(key, ToolKey::Up | ToolKey::Down));
    let key = if vertical {
        match key {
            ToolKey::Up => ToolKey::Left,
            ToolKey::Down => ToolKey::Right,
            ToolKey::Right => ToolKey::Up,
            ToolKey::Left => ToolKey::Down,
            other => other,
        }
    } else {
        key
    };
    let sign = |up: bool| if up { 1.0 } else { -1.0 };
    Some(match (key, shift) {
        (ToolKey::Left | ToolKey::Right, false) => (if caret { "kerning" } else { "tracking" }, sign(key == ToolKey::Right)),
        (ToolKey::Up | ToolKey::Down, false) => ("leading", sign(key == ToolKey::Down)),
        (ToolKey::Up | ToolKey::Down, true) => ("baselineShift", sign(key == ToolKey::Up)),
        _ => return None,
    })
}

/// Byte range in `s` of the characters `r` (IME ranges count characters; carets count bytes).
/// `None` when `r` runs past the end of `s` or backwards.
pub(super) fn char_range_to_bytes(s: &str, r: Range<usize>) -> Option<Range<usize>> {
    let byte = |c: usize| if c == s.chars().count() { Some(s.len()) } else { s.char_indices().nth(c).map(|(i, _)| i) };
    let (a, b) = (byte(r.start)?, byte(r.end)?);
    (a <= b).then_some(a..b)
}

/// Lengths of the common prefix (at most `max_prefix`) and suffix of `a` and `b` (on char
/// boundaries, not overlapping).
pub(super) fn common_affixes(a: &str, b: &str, max_prefix: usize) -> (usize, usize) {
    let mut p = 0;
    for ((i, x), y) in a.char_indices().zip(b.chars()) {
        if x != y || i + x.len_utf8() > max_prefix {
            break;
        }
        p = i + x.len_utf8();
    }
    let max_s = (a.len() - p).min(b.len() - p);
    let mut s = 0;
    for (x, y) in a[p..].chars().rev().zip(b[p..].chars().rev()) {
        if x != y || s + x.len_utf8() > max_s {
            break;
        }
        s += x.len_utf8();
    }
    (p, s)
}

/// Byte length of the borrowed plain-text segments.
pub(super) fn plain_len(segments: &[&str]) -> usize {
    segments.iter().map(|s| s.len()).sum()
}

/// Is `byte` a UTF-8 boundary in the concatenation of `segments`? Run edges are boundaries.
pub(super) fn plain_is_boundary(segments: &[&str], byte: usize) -> bool {
    let mut at = 0usize;
    for segment in segments {
        if byte == at {
            return true;
        }
        if byte < at + segment.len() {
            return segment.is_char_boundary(byte - at);
        }
        at += segment.len();
    }
    byte == at
}

/// Move `i` back to a character boundary in the concatenation of `segments` (deleting fewer
/// bytes rather than splitting UTF-8).
pub(super) fn plain_floor_boundary(segments: &[&str], mut i: usize) -> usize {
    i = i.min(plain_len(segments));
    while i > 0 && !plain_is_boundary(segments, i) {
        i -= 1;
    }
    i
}

/// Move `i` forward to a character boundary in the concatenation of `segments`.
pub(super) fn plain_ceil_boundary(segments: &[&str], mut i: usize) -> usize {
    let len = plain_len(segments);
    i = i.min(len);
    while i < len && !plain_is_boundary(segments, i) {
        i += 1;
    }
    i
}

/// The byte offset `n` characters before `at` (a boundary) in `segments`, stopping at the start of
/// the text. Walks the runs, so a large IME deletion count never copies the whole text.
pub(super) fn plain_chars_before(segments: &[&str], at: usize, n: usize) -> usize {
    if n == 0 {
        return at;
    }
    let mut remaining = n;
    let mut end = plain_len(segments);
    for segment in segments.iter().rev() {
        let start = end - segment.len();
        let local_end = at.min(end).saturating_sub(start);
        if local_end > 0 && segment.is_char_boundary(local_end) {
            let before = &segment[..local_end];
            let count = before.chars().count();
            if count >= remaining {
                return start + before.char_indices().rev().nth(remaining.saturating_sub(1)).map_or(0, |(i, _)| i);
            }
            remaining -= count;
        }
        end = start;
    }
    0
}

/// The byte offset `n` characters after `at` (a boundary) in `segments`, stopping at the end of
/// the text.
pub(super) fn plain_chars_after(segments: &[&str], at: usize, n: usize) -> usize {
    if n == 0 {
        return at;
    }
    let len = plain_len(segments);
    let mut remaining = n;
    let mut acc = 0usize;
    for segment in segments {
        let end = acc + segment.len();
        let local_start = at.saturating_sub(acc);
        if end > at && segment.is_char_boundary(local_start) {
            let after = &segment[local_start..];
            let count = after.chars().count();
            if count >= remaining {
                return acc + local_start + after.char_indices().nth(remaining).map_or(after.len(), |(i, _)| i);
            }
            remaining -= count;
        }
        acc = end;
    }
    len
}
