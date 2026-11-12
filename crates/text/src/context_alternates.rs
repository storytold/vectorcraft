//! The alternate glyphs of one character as it stands in its text: the glyph the shaper makes for
//! it in its word (a medial letter's medial form, not its isolated one), the alternates the font
//! gives that glyph, and for each the OpenType feature value that draws it on that character alone
//! (`salt=2`, `ss03`). Applying that feature to the character's range swaps the glyph in place
//! while the text stays real Unicode and the word is shaped as one piece, so the letters still
//! join. Adobe's Glyphs panel does the same for its Show › Alternates for Current Selection:
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/special-characters-glyphs/replace-characters-with-alternate-glyphs.html>

use kurbo::Point;
use vectorcraft_doc::{CharStyle, TextObject};

use crate::{FontDb, GlyphAlternate, OtFeatures, edit, layout};

/// The most characters on each side of the selected one that are shaped with it: the word it is in.
const MAX_WORD: usize = 256;

/// The most alternates tried against the text.
const MAX_CANDIDATES: usize = 512;

/// Does feature `tag` (as a style lists it, with its prefix and value) choose a glyph among a
/// glyph's alternates: `aalt`, `salt`, `swsh` or a stylistic set `ss01` to `ss20`?
pub fn is_alternate_feature(tag: &str) -> bool {
    let name = OtFeatures::tag_name(tag);
    matches!(name, "aalt" | "salt" | "swsh") || (name.len() == 4 && name.starts_with("ss") && OtFeatures::known_tag(name))
}

/// The tags to change a character's features by, as `featureChanges` takes them, so that it has
/// `tag` as its only feature that chooses an alternate: each such feature of `features` turned
/// off, and then `tag` (`salt=2`) when it is given. Empty when the character already has exactly
/// that.
pub fn alternate_changes(features: &[String], tag: Option<&str>) -> Vec<String> {
    let mut changes: Vec<String> = vec![];
    for t in features.iter().filter(|t| is_alternate_feature(t) && !t.starts_with('-')) {
        let off = format!("-{}", OtFeatures::tag_name(t));
        if tag.is_none_or(|n| OtFeatures::tag_name(n) != OtFeatures::tag_name(t)) && !changes.contains(&off) {
            changes.push(off);
        }
    }
    if let Some(tag) = tag {
        let now = OtFeatures::from_tags(features.iter().map(String::as_str));
        let wanted = OtFeatures::from_tags([tag]);
        let name = OtFeatures::tag_name(tag);
        if now.feature_value(name) != wanted.feature_value(name) {
            changes.push(tag.to_string());
        }
    }
    changes
}

/// One glyph [`alternates_in_context`] lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextAlternate {
    pub gid: u32,
    /// The feature that lists the glyph among the default glyph's alternates (`aalt` first).
    pub feature: [u8; 4],
    /// The feature tag that draws the glyph on the character in its word (`salt=2`), when one does.
    pub apply: Option<String>,
}

/// What [`alternates_in_context`] finds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextAlternates {
    /// The face that supplied the glyph ([`crate::PositionedGlyph::font_id`]).
    pub font_id: u32,
    /// The glyph the shaper makes for the character with none of its own alternate features: its
    /// form in its position.
    pub default: u32,
    /// The glyph it is drawn with now.
    pub current: u32,
    /// The alternates of [`Self::default`] in the font's order (`aalt`, `salt`, `ss01`… as
    /// [`crate::FontFace::alternates`]), without it unless `aalt` lists it.
    pub alternates: Vec<ContextAlternate>,
}

/// The glyph of the layout `text` that draws the character at `byte`, as (face id, glyph id).
fn glyph_at(db: &FontDb, text: &TextObject, byte: usize) -> Option<(u32, u32, usize)> {
    layout(db, text).glyphs.into_iter().find(|g| g.byte <= byte && byte < g.byte.saturating_add(g.len)).map(|g| (g.font_id, g.gid, g.len))
}

/// The alternate glyphs of the character at `byte` of `text` as it stands in its word, from the
/// glyph the shaper makes for it there, and the feature value that draws each on that character
/// alone. The character's own alternate features (`salt`, `ss01`…) are left out when the default
/// glyph is found; each alternate's feature is tried on the character in its word, and the one
/// that draws that glyph there is its `apply` (`salt` before the stylistic sets, `swsh` and
/// `aalt`; a glyph no feature of the font draws in this position has none). `None` when `byte` is
/// not a character of the text or the layout holds no glyph for it. Only the word is shaped (up to
/// 256 characters each side), so the cost does not grow with the text.
pub fn alternates_in_context(db: &FontDb, text: &TextObject, byte: usize) -> Option<ContextAlternates> {
    let plain = text.plain_text();
    let ch = plain.get(byte..)?.chars().next()?;
    let end = byte + ch.len_utf8();
    let breaks = |c: char| c.is_whitespace() || matches!(c, '\u{2028}' | '\u{2029}');
    let start = plain[..byte].char_indices().rev().take(MAX_WORD).take_while(|(_, c)| !breaks(*c)).last().map_or(byte, |(i, _)| i);
    let stop = plain[end..].char_indices().take(MAX_WORD).take_while(|(_, c)| !breaks(*c)).last().map_or(end, |(i, c)| end + i + c.len_utf8());
    let mut probe = TextObject::point(Point::ZERO, "", CharStyle::default());
    probe.runs = edit::slice_runs(&text.runs, start, stop);
    probe.vertical = text.vertical;
    probe.para = text.para.clone();
    let (a, b) = (byte - start, end - start);
    let (_, current, _) = glyph_at(db, &probe, a)?;
    let own = |probe: &TextObject, tag: Option<&str>| {
        let mut p = probe.clone();
        edit::style_range(&mut p.runs, a, b, |st| {
            st.glyph_alternate = None;
            st.features.retain(|t| !is_alternate_feature(t));
            st.features.extend(tag.map(str::to_string));
        });
        glyph_at(db, &p, a)
    };
    let (default_font, default, len) = own(&probe, None)?;
    let face = db.face_by_id(default_font)?;
    // A glyph that more than the character makes (a ligature) has no alternates of its own here.
    let subs = if len == ch.len_utf8() { face.substitutions(default) } else { vec![] };
    let mut alternates: Vec<ContextAlternate> = vec![];
    let mut tried: Vec<(String, u32)> = vec![];
    for sub in &subs {
        let name = String::from_utf8_lossy(&sub.feature).to_string();
        let values = if sub.glyphs.is_empty() { 0 } else { u16::try_from(sub.glyphs.len()).unwrap_or(u16::MAX) };
        for value in 1..=values {
            let tag = if value == 1 { name.clone() } else { format!("{name}={value}") };
            if !is_alternate_feature(&tag) || !OtFeatures::known_tag(&tag) || tried.len() >= MAX_CANDIDATES || tried.iter().any(|(t, _)| *t == tag) {
                continue;
            }
            // A single substitution gives its glyph at any value: only the plain tag is tried.
            if sub.kind == crate::SubstitutionKind::Single && value > 1 {
                continue;
            }
            if let Some((font, gid, _)) = own(&probe, Some(&tag))
                && font == default_font
            {
                tried.push((tag, gid));
            }
        }
    }
    for GlyphAlternate { gid, feature } in crate::fontdb::listed(&subs, default) {
        let rank = |tag: &str| match OtFeatures::tag_name(tag) {
            "salt" => 0,
            "swsh" => 2,
            "aalt" => 3,
            _ => 1,
        };
        let mut drawing: Vec<&(String, u32)> = tried.iter().filter(|(_, g)| *g == gid).collect();
        drawing.sort_by_key(|(t, _)| rank(t));
        alternates.push(ContextAlternate { gid, feature, apply: drawing.first().map(|(t, _)| t.clone()) });
    }
    Some(ContextAlternates { font_id: default_font, default, current, alternates })
}
