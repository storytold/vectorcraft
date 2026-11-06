//! The alternate glyphs a face's `GSUB` table gives one character: the Glyphs panel's Show ›
//! Alternates for Current Selection. The application finds the character's default glyph in the
//! `cmap` and looks it up in the coverage of each lookup of the features that substitute one glyph
//! for another, as the OpenType registry describes `aalt`.
//! <https://learn.microsoft.com/en-us/typography/opentype/spec/features_ae#tag-aalt>
//! <https://learn.microsoft.com/en-us/typography/opentype/spec/features_pt>

use skrifa::MetadataProvider;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::gsub::{AlternateSubstFormat1, SingleSubst, SubstitutionSubtables};
use vectorcraft_doc::GlyphChoice;

use super::FontFace;

/// One glyph [`FontFace::alternates`] lists: its id and the feature it was found in (`aalt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphAlternate {
    pub gid: u32,
    pub feature: [u8; 4],
}

/// Caps on what a damaged or very large `GSUB` table can make [`FontFace::alternates`] read:
/// feature records, lookups per feature and in all, subtables per lookup and in all, glyphs per
/// alternate set and glyphs listed in all.
const MAX_FEATURES: usize = 4096;
const MAX_FEATURE_LOOKUPS: usize = 256;
const MAX_LOOKUPS: usize = 4096;
const MAX_SUBTABLES: usize = 256;
const MAX_SUBTABLE_READS: usize = 16384;
const MAX_SET: usize = 512;
const MAX_ALTERNATES: usize = 1024;

/// The other discretionary features that substitute one glyph for another (historical forms,
/// justification alternates, alternate annotation forms, ornaments, titling), after `cv99`, in
/// tag order.
const OTHER_FEATURES: [&[u8; 4]; 5] = [b"hist", b"jalt", b"nalt", b"ornm", b"titl"];

/// Where the glyphs of feature `tag` come in the list: `aalt`, `salt`, `ss01` to `ss20`, `swsh`,
/// `cv01` to `cv99`, then [`OTHER_FEATURES`]. `None` for a feature that is not listed.
fn feature_rank(tag: [u8; 4]) -> Option<(u8, u8)> {
    let number = |t: &[u8]| std::str::from_utf8(t).ok().filter(|s| s.bytes().all(|b| b.is_ascii_digit())).and_then(|s| s.parse::<u8>().ok());
    match &tag {
        b"aalt" => Some((0, 0)),
        b"salt" => Some((1, 0)),
        [b's', b's', ..] => number(&tag[2..]).filter(|n| (1..=20).contains(n)).map(|n| (2, n)),
        b"swsh" => Some((3, 0)),
        [b'c', b'v', ..] => number(&tag[2..]).filter(|n| (1..=99).contains(n)).map(|n| (4, n)),
        _ => OTHER_FEATURES.iter().position(|t| **t == tag).map(|i| (5, i as u8)),
    }
}

impl FontFace {
    /// The face's alternate glyphs for `c`, from its `GSUB` table: the glyphs single substitutions
    /// (lookup type 1) and alternate substitutions (type 3, the whole set in the font's order), in
    /// extension lookups (type 7) too, put in place of `c`'s default glyph. `aalt` comes first,
    /// then `salt`, `ss01` to `ss20`, `swsh`, `cv01` to `cv99`, `hist`, `jalt`, `nalt`, `ornm` and
    /// `titl`; within a feature its lookups go in lookup order. A glyph found again is left out.
    /// The default glyph is listed only when `aalt` lists it. Ligatures and contextual
    /// substitutions are not read. Empty when the face does not map `c`, has no `GSUB` table, or
    /// the table cannot be read; glyph ids past the face's glyph count are left out.
    pub fn alternates(&self, c: char) -> Vec<GlyphAlternate> {
        let mut out: Vec<GlyphAlternate> = vec![];
        let Some(f) = self.skrifa() else { return out };
        let Some(default) = f.charmap().map(c) else { return out };
        let Ok(gsub) = f.gsub() else { return out };
        let (Ok(features), Ok(lookups)) = (gsub.feature_list(), gsub.lookup_list()) else { return out };
        let glyphs = f.maxp().map_or(0, |m| u32::from(m.num_glyphs()));
        // The lookups of the listed features, by rank and lookup index.
        // The listed feature records are read in rank order, so the cap on lookups in all leaves
        // out the lowest ranked ones.
        let records = features.feature_records();
        let mut ranked: Vec<((u8, u8), usize, [u8; 4])> = records
            .iter()
            .take(MAX_FEATURES)
            .enumerate()
            .filter_map(|(i, r)| {
                let tag = r.feature_tag().to_be_bytes();
                feature_rank(tag).map(|rank| (rank, i, tag))
            })
            .collect();
        ranked.sort_unstable_by_key(|r| (r.0, r.1));
        let mut wanted: Vec<((u8, u8), u16, [u8; 4])> = vec![];
        for (rank, i, tag) in ranked {
            let room = MAX_LOOKUPS.saturating_sub(wanted.len());
            if room == 0 {
                break;
            }
            let Some(record) = records.get(i) else { continue };
            let Ok(feature) = record.feature(features.offset_data()) else { continue };
            wanted.extend(feature.lookup_list_indices().iter().take(MAX_FEATURE_LOOKUPS.min(room)).map(|i| (rank, i.get(), tag)));
        }
        wanted.sort_unstable_by_key(|w| (w.0, w.1));
        wanted.dedup_by_key(|w| (w.0, w.1));
        let mut push = |gid: u32, feature: [u8; 4]| {
            let listed = gid != default.to_u32() || feature == *b"aalt";
            if listed && gid < glyphs && out.len() < MAX_ALTERNATES && !out.iter().any(|a| a.gid == gid) {
                out.push(GlyphAlternate { gid, feature });
            }
        };
        let all = lookups.lookups();
        let mut reads = MAX_SUBTABLE_READS;
        for (_, index, tag) in wanted {
            if reads == 0 {
                break;
            }
            let Ok(subtables) = all.get(usize::from(index)).and_then(|l| l.subtables()) else { continue };
            let n = MAX_SUBTABLES.min(reads);
            // A glyph takes the first subtable of a lookup whose coverage holds it.
            match subtables {
                SubstitutionSubtables::Single(s) => {
                    if let Some(gid) = s.iter().take(n).inspect(|_| reads -= 1).flatten().find_map(|t| single(&t, default.to_u32())) {
                        push(gid, tag);
                    }
                }
                SubstitutionSubtables::Alternate(s) => {
                    if let Some(set) = s.iter().take(n).inspect(|_| reads -= 1).flatten().find_map(|t| alternate_set(&t, default.to_u32())) {
                        set.into_iter().for_each(|gid| push(gid, tag));
                    }
                }
                _ => {}
            }
        }
        out
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

/// The glyph single substitution `t` puts in place of `gid`, when its coverage holds `gid`.
fn single(t: &SingleSubst<'_>, gid: u32) -> Option<u32> {
    match t {
        SingleSubst::Format1(t) => {
            t.coverage().ok()?.get(skrifa::GlyphId::new(gid))?;
            // The delta is added modulo 65536.
            Some((gid as i32 + i32::from(t.delta_glyph_id())).rem_euclid(0x1_0000) as u32)
        }
        SingleSubst::Format2(t) => {
            let i = t.coverage().ok()?.get(skrifa::GlyphId::new(gid))?;
            t.substitute_glyph_ids().get(usize::from(i)).map(|g| u32::from(g.get().to_u16()))
        }
    }
}

/// The alternate set of alternate substitution `t` for `gid`, when its coverage holds `gid`.
fn alternate_set(t: &AlternateSubstFormat1<'_>, gid: u32) -> Option<Vec<u32>> {
    let i = t.coverage().ok()?.get(skrifa::GlyphId::new(gid))?;
    let set = t.alternate_sets().get(usize::from(i)).ok()?;
    Some(set.alternate_glyph_ids().iter().take(MAX_SET).map(|g| u32::from(g.get().to_u16())).collect())
}
