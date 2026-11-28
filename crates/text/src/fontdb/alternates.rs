//! The alternate glyphs a face's `GSUB` table gives one character: the Glyphs panel's Show ›
//! Alternates for Current Selection. The application finds the character's default glyph in the
//! `cmap` and lists what the features that substitute one glyph for another give it, as the
//! OpenType registry describes `aalt` ([`FontFace::substitutions`] reads them).
//! <https://learn.microsoft.com/en-us/typography/opentype/spec/features_ae#tag-aalt>
//! <https://learn.microsoft.com/en-us/typography/opentype/spec/features_pt>

use skrifa::MetadataProvider;
use skrifa::raw::TableProvider;
use vectorcraft_doc::GlyphChoice;

use super::FontFace;
use super::gsub::Substitution;

/// One glyph [`FontFace::alternates`] lists: its id and the feature it was found in (`aalt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphAlternate {
    pub gid: u32,
    pub feature: [u8; 4],
}

/// The most glyphs [`FontFace::alternates`] lists.
const MAX_ALTERNATES: usize = 1024;

/// The alternates of glyph `default` that its substitutions `subs` ([`FontFace::substitutions`])
/// list, in their order, each glyph once with the first feature that gives it: the default glyph
/// itself only when `aalt` gives it.
pub(crate) fn listed(subs: &[Substitution], default: u32) -> Vec<GlyphAlternate> {
    let mut out: Vec<GlyphAlternate> = vec![];
    for substitution in subs {
        for &gid in &substitution.glyphs {
            let wanted = gid != default || substitution.feature == *b"aalt";
            if wanted && out.len() < MAX_ALTERNATES && !out.iter().any(|a| a.gid == gid) {
                out.push(GlyphAlternate { gid, feature: substitution.feature });
            }
        }
    }
    out
}

impl FontFace {
    /// The face's alternate glyphs for `c`, from its `GSUB` table: the glyphs single substitutions
    /// (lookup type 1) and alternate substitutions (type 3, the whole set in the font's order), in
    /// extension lookups (type 7) too, put in place of `c`'s default glyph
    /// ([`FontFace::substitutions`]). `aalt` comes first, then `salt`, `ss01` to `ss20`, `swsh`,
    /// `cv01` to `cv99`, `hist`, `jalt`, `nalt`, `ornm` and `titl`; within a feature its lookups go
    /// in lookup order. A glyph found again is left out. The default glyph is listed only when
    /// `aalt` lists it. Ligatures and contextual substitutions are not read. Empty when the face
    /// does not map `c`, has no `GSUB` table, or the table cannot be read; glyph ids past the face's
    /// glyph count are left out.
    pub fn alternates(&self, c: char) -> Vec<GlyphAlternate> {
        let Some(default) = self.skrifa().and_then(|f| f.charmap().map(c)).map(|g| g.to_u32()) else { return vec![] };
        listed(&self.substitutions(default), default)
    }

    /// The glyphs no character maps (its `cmap` gives no character for them), by glyph id, without
    /// `.notdef`: the positional forms, alternates and ligatures the `GSUB` table reaches and any
    /// other glyph of the font. Read once for the face.
    pub fn unencoded_glyphs(&self) -> std::sync::Arc<[u32]> {
        self.unencoded
            .get_or_init(|| {
                let Some(font) = self.skrifa() else { return std::sync::Arc::from([]) };
                let mapped: std::collections::HashSet<u32> = font.charmap().mappings().map(|(_, g)| g.to_u32()).collect();
                let count = font.maxp().map_or(0, |m| u32::from(m.num_glyphs()));
                (1..count).filter(|g| !mapped.contains(g)).collect()
            })
            .clone()
    }

    /// Is glyph `gid` one no character maps and no feature reaches, whose name does not give it
    /// to another letter than `c` (`a.salt` for `a` in a font with no `salt`; `A.salt` is not
    /// `a`'s)? Such a glyph is not among [`Self::alternates`], yet it can be drawn for `c` as a
    /// glyph choice ([`Self::choice`]).
    pub fn is_unlisted_alternate(&self, c: char, gid: u32) -> bool {
        self.unencoded_glyphs().binary_search(&gid).is_ok()
            && self.feature_reaching(gid).is_none()
            && self.name_owner(gid).is_none_or(|owner| owner == c)
    }

    /// The character a glyph's name says it belongs to: the one whose glyph carries the name up
    /// to its first `.` (`A` for `A.salt`, `s` for `s.ss1`; the lowest character when several map
    /// that glyph). `None` when the glyph has no name, or no character maps a glyph of that name.
    fn name_owner(&self, gid: u32) -> Option<char> {
        let base = self.glyph_name(gid)?;
        let base = base.split('.').next().unwrap_or_default();
        let names = self.named_chars.get_or_init(|| {
            let Some(font) = self.skrifa() else { return Default::default() };
            let mut by_char: Vec<(char, u32)> = font.charmap().mappings().filter_map(|(c, g)| Some((char::from_u32(c)?, g.to_u32()))).collect();
            by_char.sort_unstable();
            let mut out = std::collections::HashMap::new();
            for (c, g) in by_char {
                if let Some(name) = self.glyph_name(g) {
                    out.entry(name).or_insert(c);
                }
            }
            out
        });
        names.get(base).copied()
    }

    /// How many glyphs the face has (its `maxp` table; 0 when it can't be read).
    pub fn glyph_count(&self) -> u32 {
        self.skrifa().and_then(|f| f.maxp().ok()).map_or(0, |m| u32::from(m.num_glyphs()))
    }

    /// Does this face draw `choice` for character `c`? It does when the choice is for `c`, names
    /// this face's PostScript name and version, and its glyph id is one of the face's glyphs.
    pub fn draws(&self, choice: &GlyphChoice, c: char) -> bool {
        choice.ch == c && choice.version == self.version && choice.gid < self.glyph_count() && choice.font == self.postscript_name()
    }

    /// `c` drawn with glyph `gid` of this face ([`CharStyle::glyph_alternate`]).
    ///
    /// [`CharStyle::glyph_alternate`]: vectorcraft_doc::CharStyle::glyph_alternate
    pub fn choice(&self, c: char, gid: u32) -> GlyphChoice {
        GlyphChoice { ch: c, gid, font: self.postscript_name(), version: self.version.clone() }
    }
}
