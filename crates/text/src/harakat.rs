//! Arabic marks (harakat) styled apart from their letters.
//!
//! A designer colours the harakat apart from the letters (or hides them with no fill) without
//! breaking the letters' joining or the marks' attachment: runs that differ only in paint
//! ([`paint_only`]) are not a shaping boundary at a harakat ([`paint_only_mark_boundary`]), so the
//! letter and its marks are still shaped together, glyph for glyph as before. When the character-
//! level shaping pass matches the monotone pass ([`own_chars`]), a glyph may take a later
//! in-cluster character's run only when it is positively a mark: the face's GDEF GlyphClassDef
//! names it class 3 (mark) and that character is a harakat. Anything else—including fonts with no
//! GDEF, AAT/`morx` faces where the class is unavailable, ligatures, and multi-component
//! glyphs—keeps the shaping cluster's first character. Conservative means a mark may fail to take
//! its colour, but a base letter must never take a mark's colour. AAT/`morx` fonts may still leave
//! a mark on a ligature (for example Geeza Pro's Allah ligature) painted as its letter. Renderers
//! and exporters read the glyph's run as they always did, so they need no change. The text itself
//! is never changed.

use skrifa::GlyphId;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::gdef::GlyphClassDef;
use vectorcraft_doc::CharStyle;

use crate::fontdb::FontFace;

/// Is `c` an Arabic combining mark: harakat (fatha, damma, kasra, tanwin, shadda, sukun), the
/// superscript alef, Quranic annotation marks, and the extended-A marks?
pub(crate) fn is_harakat(c: char) -> bool {
    matches!(
        c as u32,
        0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x06DF..=0x06E4
            | 0x06E7..=0x06E8
            | 0x06EA..=0x06ED
            | 0x08D3..=0x08FF
    )
}

/// Do character styles `a` and `b` differ, if at all, only in paint: fill, stroke and how the
/// stroke is drawn, overprint, and the character style name? Those are not read by the shaper
/// and change no glyph, advance or position. Stricter than [`crate::shape`]'s `shapes_alike`
/// (which also ignores baseline shift, tracking, underline, …).
pub(crate) fn paint_only(a: &CharStyle, b: &CharStyle) -> bool {
    let mut b = b.clone();
    b.fill = a.fill.clone();
    b.stroke = a.stroke.clone();
    b.stroke_width = a.stroke_width;
    b.overprint_fill = a.overprint_fill;
    b.overprint_stroke = a.overprint_stroke;
    b.stroke_cap = a.stroke_cap;
    b.stroke_join = a.stroke_join;
    b.stroke_miter_limit = a.stroke_miter_limit;
    b.stroke_dash = a.stroke_dash.clone();
    b.stroke_align = a.stroke_align;
    b.style_name = a.style_name.clone();
    *a == b
}

/// Is the run boundary at byte `at` of `text`, between styles `before` and `after`, one that
/// stays inside a shaping cluster: they differ only in paint and a harakat is on either side?
pub(crate) fn paint_only_mark_boundary(text: &str, at: usize, before: &CharStyle, after: &CharStyle) -> bool {
    let next = text.get(at..).and_then(|s| s.chars().next());
    let previous = text.get(..at).and_then(|s| s.chars().next_back());
    (next.is_some_and(is_harakat) || previous.is_some_and(is_harakat)) && paint_only(before, after)
}

/// The byte ranges of the harakat in `text[range]`, each run of consecutive harakat as one range.
/// The range is normalised as [`crate::edit::style_range`] takes one: its ends clamped to the
/// text, each moved back to the start of the character it falls in, and put in order.
#[cfg(test)]
pub(crate) fn harakat_ranges(text: &str, range: std::ops::Range<usize>) -> Vec<std::ops::Range<usize>> {
    let (mut start, mut end) = (crate::edit::floor_char(text, range.start), crate::edit::floor_char(text, range.end));
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    let Some(part) = text.get(start..end) else {
        return vec![];
    };
    let mut out: Vec<std::ops::Range<usize>> = vec![];
    for (i, c) in part.char_indices().filter(|(_, c)| is_harakat(*c)) {
        let (a, b) = (start + i, start + i + c.len_utf8());
        match out.last_mut() {
            Some(last) if last.end == a => last.end = b,
            _ => out.push(a..b),
        }
    }
    out
}

/// Does `face`'s GDEF GlyphClassDef name glyph `gid` as class 3 (mark)? False when the face has
/// no GDEF, no class def, or the class is unavailable (typical of AAT/`morx` paths).
pub(crate) fn is_gdef_mark(face: &FontFace, gid: u32) -> bool {
    let Some(font) = face.skrifa() else {
        return false;
    };
    let Ok(gdef) = font.gdef() else {
        return false;
    };
    let Some(Ok(classes)) = gdef.glyph_class_def() else {
        return false;
    };
    GlyphClassDef::new(classes.get(GlyphId::new(gid))) == GlyphClassDef::Mark
}

/// Each glyph's own character, from the shaping `by_char` at character cluster level of the
/// glyphs `raw` (glyph id, cluster, x advance, x and y offset; `by_char` with the y advance after
/// the x advance), the segment ending at byte `end`, and `text` for character identity.
/// `is_gdef_mark(gid)` is true only when the face's GDEF GlyphClassDef names that glyph class 3.
/// Empty unless `by_char` sets the same glyphs the same way (glyph id, advance and offsets, glyph
/// by glyph): repeated glyph ids alone could hide a changed correspondence. A glyph whose character
/// lies outside its own shaping cluster of `raw` (from its cluster value up to the next larger one,
/// else `end`) gets its cluster's first character instead. A later in-cluster character is kept
/// only when the glyph is a GDEF mark and that character is a harakat; otherwise the cluster start
/// is kept (conservative: a mark may miss its colour; a base must never take a mark's). `raw`'s
/// clusters are never changed. These are HarfBuzz shaping clusters, which can span more than one
/// grapheme (a ligature).
pub(crate) fn own_chars(
    raw: &[(u32, u32, i32, i32, i32)],
    by_char: &[(u32, u32, i32, i32, i32, i32)],
    end: usize,
    text: &str,
    is_gdef_mark: impl Fn(u32) -> bool,
) -> Vec<u32> {
    let same = by_char.len() == raw.len() && by_char.iter().zip(raw).all(|(c, r)| c.0 == r.0 && c.2 == r.2 && c.4 == r.3 && c.5 == r.4);
    if !same {
        return vec![];
    }
    let mut starts: Vec<u32> = raw.iter().map(|g| g.1).collect();
    starts.sort_unstable();
    starts.dedup();
    let cluster_end = |cl: u32| starts.get(starts.partition_point(|&c| c <= cl)).map_or(end, |&c| c as usize);
    by_char
        .iter()
        .zip(raw)
        .map(|(c, r)| {
            let in_cluster = r.1 <= c.1 && (c.1 as usize) < cluster_end(r.1);
            if !in_cluster {
                return r.1;
            }
            if c.1 == r.1 {
                return c.1;
            }
            // Later in-cluster character: only a positively identified mark glyph on a harakat.
            let mark_char = text.get(c.1 as usize..).and_then(|s| s.chars().next()).is_some_and(is_harakat);
            if is_gdef_mark(r.0) && mark_char { c.1 } else { r.1 }
        })
        .collect()
}
