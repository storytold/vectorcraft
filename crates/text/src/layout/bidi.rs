use super::*;

/// The bidirectional resolution (UAX #9) of paragraph `text`, its base direction `direction` or,
/// without one, from its first strong character. `None` for the usual paragraph with nothing right
/// to left in it (no right-to-left character or direction), which skips the algorithm.
pub(crate) fn para_bidi(text: &str, direction: Option<ParaDirection>) -> Option<BidiInfo<'_>> {
    use unicode_bidi::BidiClass::{AL, AN, FSI, R, RLE, RLI, RLO};
    let rtl_set = direction == Some(ParaDirection::RightToLeft);
    if !rtl_set && !text.chars().any(|c| matches!(unicode_bidi::bidi_class(c), R | AL | AN | RLE | RLO | RLI | FSI)) {
        return None;
    }
    let level = direction.map(|d| if d == ParaDirection::RightToLeft { Level::rtl() } else { Level::ltr() });
    Some(BidiInfo::new(text, level))
}

/// [`para_bidi`] for a paragraph whose characters in `set` (byte ranges of `text`, in order and not
/// overlapping, from [`Ctx::char_directions`]) have a Character Direction: each range is resolved
/// as a Unicode bidi embedding, as if an LRE or RLE stood before it and a PDF after it (UAX #9 X2,
/// X3, X7). The levels, classes and paragraphs come back for the bytes of `text`. The paragraph's own
/// direction is unchanged: the first strong character is found inside embeddings too (P2).
pub(crate) fn para_bidi_with<'a>(text: &'a str, direction: Option<ParaDirection>, set: &[(Range<usize>, ParaDirection)]) -> Option<BidiInfo<'a>> {
    let plain = para_bidi(text, direction);
    if set.is_empty() || (plain.is_none() && set.iter().all(|(_, d)| *d == ParaDirection::LeftToRight)) {
        // A left-to-right embedding in left-to-right text takes an even level: its order is the same.
        return plain;
    }
    // The text with the embedding controls, and where each byte of `text` is in it.
    let mut marked = String::with_capacity(text.len() + set.len() * 6);
    let mut at_byte: Vec<usize> = Vec::with_capacity(text.len());
    let mut copy = |marked: &mut String, from: usize, to: usize| {
        if let Some(s) = text.get(from..to) {
            at_byte.extend(marked.len()..marked.len() + s.len());
            marked.push_str(s);
        }
    };
    let mut done = 0;
    for (r, d) in set {
        let (a, b) = (r.start.max(done).min(text.len()), r.end.min(text.len()));
        if a >= b {
            continue;
        }
        copy(&mut marked, done, a);
        marked.push(if *d == ParaDirection::RightToLeft { '\u{202B}' } else { '\u{202A}' });
        copy(&mut marked, a, b);
        marked.push('\u{202C}');
        done = b;
    }
    copy(&mut marked, done, text.len());
    let level = direction.map(|d| if d == ParaDirection::RightToLeft { Level::rtl() } else { Level::ltr() });
    let info = BidiInfo::new(&marked, level);
    let back = |i: usize| at_byte.partition_point(|&m| m < i);
    let paragraphs =
        info.paragraphs.iter().map(|p| unicode_bidi::ParagraphInfo { range: back(p.range.start)..back(p.range.end), level: p.level }).collect();
    Some(BidiInfo {
        text,
        original_classes: at_byte.iter().filter_map(|&i| info.original_classes.get(i).copied()).collect(),
        levels: at_byte.iter().filter_map(|&i| info.levels.get(i).copied()).collect(),
        paragraphs,
    })
}

/// Does the paragraph run right to left ([`para_bidi`])?
pub(crate) fn is_rtl(bidi: Option<&BidiInfo<'_>>) -> bool {
    bidi.and_then(|b| b.paragraphs.first()).is_some_and(|p| p.level.is_rtl())
}

/// The visual order of a line's `glyphs` (indices into them), whole shaping clusters reordered
/// (UAX #9 L1–L2) for the line `line` (bytes of the text) of the paragraph starting at byte
/// `paragraph_start`; their own order when nothing is right to left (`bidi` None).
pub(super) fn visual_order(bidi: Option<&BidiInfo<'_>>, paragraph_start: usize, line: Range<usize>, glyphs: &[SGlyph]) -> Vec<usize> {
    let Some((bidi, para)) = bidi.filter(|b| b.has_rtl()).and_then(|b| Some((b, b.paragraphs.first()?))) else {
        return (0..glyphs.len()).collect();
    };
    if glyphs.is_empty() {
        return vec![];
    }
    let levels = bidi.reordered_levels(para, line.start.saturating_sub(paragraph_start)..line.end.saturating_sub(paragraph_start));
    // A list marker (length 0) is a cluster of its own.
    let mut clusters: Vec<Range<usize>> = Vec::new();
    for (i, g) in glyphs.iter().enumerate() {
        if i > 0 && glyphs.get(i - 1).is_some_and(|p| p.byte == g.byte && (p.len == 0) == (g.len == 0)) {
            if let Some(c) = clusters.last_mut() {
                c.end = i + 1;
            }
        } else {
            clusters.push(i..i + 1);
        }
    }
    // A list marker is at the paragraph's level, and at a right-to-left one its tab is drawn
    // before (left of) its characters.
    let cluster_levels: Vec<_> = clusters
        .iter()
        .filter_map(|c| glyphs.get(c.start))
        .map(|g| if g.len == 0 { g.level } else { levels.get(g.byte.saturating_sub(paragraph_start)).copied().unwrap_or(g.level) })
        .collect();
    let reversed = |i: usize| glyphs.get(clusters.get(i).map_or(0, |c| c.start)).is_some_and(|g| g.len == 0 && g.level.is_rtl());
    unicode_bidi::BidiInfo::reorder_visual(&cluster_levels)
        .into_iter()
        .filter_map(|i| clusters.get(i).map(|c| (c.clone(), reversed(i))))
        .flat_map(|(c, rev)| {
            let mut v: Vec<usize> = c.collect();
            if rev {
                v.reverse();
            }
            v
        })
        .collect()
}
