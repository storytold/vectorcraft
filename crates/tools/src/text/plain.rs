use super::{ImeSurrounding, Range, ToolKey};

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

/// Append the bytes `start..end` of the concatenation of `segments` to `out`.
fn copy_plain_range(segments: &[&str], start: usize, end: usize, out: &mut String) {
    let mut at = 0usize;
    for segment in segments {
        let segment_end = at + segment.len();
        let from = start.max(at);
        let to = end.min(segment_end);
        if from < to {
            let (a, b) = (from - at, to - at);
            if segment.is_char_boundary(a) && segment.is_char_boundary(b) {
                out.push_str(&segment[a..b]);
            }
        }
        if segment_end >= end {
            break;
        }
        at = segment_end;
    }
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

/// The logical IME surrounding window: the pre-edit is removed and the caret/anchor offsets
/// are remapped around it. `preedit` has already been validated against `segments`.
pub(super) fn ime_surrounding_window(
    segments: &[&str],
    preedit: Option<Range<usize>>,
    len: usize,
    caret: usize,
    anchor: usize,
    max_bytes: usize,
) -> ImeSurrounding {
    let removed = preedit.as_ref().map_or(0, |r| r.end - r.start);
    let logical_len = len - removed;
    let to_logical = |byte: usize| -> usize {
        match &preedit {
            Some(r) if byte > r.start && byte < r.end => r.start,
            Some(r) if byte >= r.end => byte - removed,
            _ => byte,
        }
    };
    let cursor = to_logical(caret);
    let anchor = to_logical(anchor);
    let selection_start = cursor.min(anchor);
    let selection_end = cursor.max(anchor);
    let selection_len = selection_end - selection_start;
    if selection_len > max_bytes {
        return ImeSurrounding::Unavailable("the selection is wider than the IME surrounding-text limit");
    }
    let budget = max_bytes - selection_len;
    let before_available = selection_start;
    let after_available = logical_len - selection_end;
    let mut before = (budget / 2).min(before_available);
    let mut after = (budget - before).min(after_available);
    before = (budget - after).min(before_available);
    after = (budget - before).min(after_available);
    let mut window_start = selection_start - before;
    let mut window_end = selection_end + after;
    let logical_boundary = |byte: usize| -> bool {
        match &preedit {
            Some(r) if byte == r.start => true,
            Some(r) if byte < r.start => plain_is_boundary(segments, byte),
            Some(_) => plain_is_boundary(segments, byte + removed),
            None => plain_is_boundary(segments, byte),
        }
    };
    while window_start < selection_start && !logical_boundary(window_start) {
        window_start += 1;
    }
    while selection_end < window_end && !logical_boundary(window_end) {
        window_end -= 1;
    }
    let to_original = |byte: usize| -> usize {
        match &preedit {
            Some(r) if byte > r.start => byte + removed,
            _ => byte,
        }
    };
    let original_start = to_original(window_start);
    let original_end = to_original(window_end);
    let mut text = String::with_capacity(window_end - window_start);
    match &preedit {
        Some(r) => {
            if window_start < r.start {
                let first_end = original_end.min(r.start);
                if original_start < first_end {
                    copy_plain_range(segments, original_start, first_end, &mut text);
                }
            }
            if r.start < window_end {
                let second_start = r.end.max(original_start);
                if second_start < original_end {
                    copy_plain_range(segments, second_start, original_end, &mut text);
                }
            }
        }
        None => {
            if original_start < original_end {
                copy_plain_range(segments, original_start, original_end, &mut text);
            }
        }
    }
    debug_assert_eq!(text.len(), window_end - window_start, "removing the pre-edit must preserve the logical window length");
    ImeSurrounding::Available { text, cursor: cursor - window_start, anchor: anchor - window_start }
}
