//! What a face's `GSUB` table substitutes for a glyph: the alternates of one glyph (the Glyphs
//! panel's Show › Alternates for Current Selection), the feature that gives a glyph no character
//! maps to (Show › Entire Font) and glyph names. Read once for a face, the first time one is asked
//! for, and kept with it ([`FontFace`]).
//!
//! A feature of alternates (an alternate substitution, lookup type 3; `salt`, `aalt`, `swsh`)
//! takes the alternate its value says: with the feature at value `N`, a glyph in its coverage is
//! replaced by the `N`th glyph of its set (the first at value 1, none at 0 or past the set), which
//! is how the shaper reads it. A feature of single substitutions (type 1; `ss01`, `smcp`) replaces
//! the glyph at any value from 1.
//! <https://learn.microsoft.com/en-us/typography/opentype/spec/gsub#lookuptype-3-alternate-substitution-subtable>
//! <https://learn.microsoft.com/en-us/typography/opentype/spec/features_ae#tag-aalt>

use std::collections::{HashMap, HashSet};

use skrifa::MetadataProvider;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::gsub::{AlternateSubstFormat1, LigatureSubstFormat1, MultipleSubstFormat1, SingleSubst, SubstitutionSubtables};

use super::FontFace;

/// Caps on what a damaged or very large `GSUB` table can make a face read: feature records,
/// lookups per feature and in all, subtables per lookup and in all, glyphs per alternate set, and
/// glyphs and substitutions listed in all.
pub(super) const MAX_FEATURES: usize = 4096;
const MAX_FEATURE_LOOKUPS: usize = 256;
const MAX_LOOKUPS: usize = 4096;
const MAX_SUBTABLES: usize = 256;
const MAX_SUBTABLE_READS: usize = 16384;
pub(super) const MAX_SET: usize = 512;
const MAX_ENTRIES: usize = 1 << 18;

/// The other discretionary features that substitute one glyph for another (historical forms,
/// justification alternates, alternate annotation forms, ornaments, titling), after `cv99`, in
/// tag order.
const OTHER_FEATURES: [&[u8; 4]; 5] = [b"hist", b"jalt", b"nalt", b"ornm", b"titl"];

/// Where the substitutions of feature `tag` come in a glyph's list: `aalt`, `salt`, `ss01` to
/// `ss20`, `swsh`, `cv01` to `cv99`, then [`OTHER_FEATURES`]. `None` for a feature that is not
/// listed.
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

/// The rank of the features that give no listed alternates but reach glyphs ([`FontFace::feature_reaching`]).
const UNLISTED: (u8, u8) = (9, 0);

/// How a feature gives its glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubstitutionKind {
    /// One glyph, whatever the feature's value is from 1 (a single substitution, type 1).
    Single,
    /// A set of glyphs, the one at the feature's value (an alternate substitution, type 3).
    Alternate,
}

/// What one feature substitutes for a glyph ([`FontFace::substitutions`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Substitution {
    /// The feature's tag (`salt`).
    pub feature: [u8; 4],
    pub kind: SubstitutionKind,
    /// The glyphs, in the font's order: the one glyph of a single substitution, or the alternate
    /// set (the feature at value `N` gives the `N`th).
    pub glyphs: Vec<u32>,
}

impl Substitution {
    /// The value the feature takes to give `glyph` (`salt=2` for 2), `None` when it never gives it.
    pub fn value_for(&self, glyph: u32) -> Option<u16> {
        match self.kind {
            SubstitutionKind::Single => (self.glyphs.first() == Some(&glyph)).then_some(1),
            SubstitutionKind::Alternate => self.glyphs.iter().position(|g| *g == glyph).and_then(|i| u16::try_from(i + 1).ok()),
        }
    }
}

type Tag = [u8; 4];

/// A face's substitutions, by the glyph they replace and by the glyph they give.
#[derive(Default)]
pub(super) struct GsubIndex {
    /// The listed features' ([`feature_rank`]) substitutions of each glyph, by feature rank and
    /// lookup.
    by_input: HashMap<u32, Vec<Substitution>>,
    /// The first feature, listed ones first, that gives each glyph: by a single, multiple,
    /// alternate or ligature substitution, and the glyph it replaces (not known for a ligature).
    reached: HashMap<u32, (Tag, Option<u32>)>,
}

/// The glyph single substitution `t` puts in place of each glyph its coverage holds.
fn single_pairs(t: &SingleSubst<'_>, room: usize) -> Vec<(u32, u32)> {
    match t {
        SingleSubst::Format1(t) => {
            let Ok(coverage) = t.coverage() else { return vec![] };
            // The delta is added modulo 65536.
            let delta = i32::from(t.delta_glyph_id());
            coverage.iter().take(room).map(|g| (g.to_u32(), (g.to_u32() as i32 + delta).rem_euclid(0x1_0000) as u32)).collect()
        }
        SingleSubst::Format2(t) => {
            let Ok(coverage) = t.coverage() else { return vec![] };
            let out = t.substitute_glyph_ids();
            coverage.iter().zip(out).take(room).map(|(g, to)| (g.to_u32(), u32::from(to.get().to_u16()))).collect()
        }
    }
}

/// The alternate set of each glyph alternate substitution `t`'s coverage holds.
fn alternate_sets(t: &AlternateSubstFormat1<'_>, room: usize) -> Vec<(u32, Vec<u32>)> {
    let Ok(coverage) = t.coverage() else { return vec![] };
    let sets = t.alternate_sets();
    coverage
        .iter()
        .enumerate()
        .take(room)
        .filter_map(|(i, g)| {
            let set = sets.get(i).ok()?;
            Some((g.to_u32(), set.alternate_glyph_ids().iter().take(MAX_SET).map(|g| u32::from(g.get().to_u16())).collect()))
        })
        .collect()
}

/// The glyphs multiple substitution `t` puts in place of its covered glyphs, and the ligature
/// glyphs of ligature substitution `l`, as (glyph, glyphs it becomes).
fn multiple_pairs(t: &MultipleSubstFormat1<'_>, room: usize) -> Vec<(u32, Vec<u32>)> {
    let Ok(coverage) = t.coverage() else { return vec![] };
    let sequences = t.sequences();
    coverage
        .iter()
        .enumerate()
        .take(room)
        .filter_map(|(i, g)| {
            let sequence = sequences.get(i).ok()?;
            Some((g.to_u32(), sequence.substitute_glyph_ids().iter().map(|g| u32::from(g.get().to_u16())).collect()))
        })
        .collect()
}

fn ligature_glyphs(t: &LigatureSubstFormat1<'_>, room: usize) -> Vec<u32> {
    let Ok(coverage) = t.coverage() else { return vec![] };
    let sets = t.ligature_sets();
    let mut out = vec![];
    for (i, _) in coverage.iter().enumerate().take(room) {
        let Ok(set) = sets.get(i) else { continue };
        out.extend(set.ligatures().iter().flatten().take(room).map(|l| u32::from(l.ligature_glyph().to_u16())));
        if out.len() >= room {
            break;
        }
    }
    out
}

impl GsubIndex {
    /// Read `face`'s `GSUB` table. Empty when the face has none or it cannot be read; glyph ids past
    /// the face's glyph count are left out; the caps above bound what a damaged table makes it read.
    fn read(face: &FontFace) -> Self {
        let mut index = Self::default();
        let Some(font) = face.skrifa() else { return index };
        let Ok(gsub) = font.gsub() else { return index };
        let (Ok(features), Ok(lookups)) = (gsub.feature_list(), gsub.lookup_list()) else { return index };
        let glyphs = font.maxp().map_or(0, |m| u32::from(m.num_glyphs()));
        // The lookups of the features, by rank and lookup index. The feature records are read in
        // rank order, so the cap on lookups in all leaves out the lowest ranked ones.
        let records = features.feature_records();
        let mut ranked: Vec<((u8, u8), usize, [u8; 4])> = records
            .iter()
            .take(MAX_FEATURES)
            .enumerate()
            .map(|(i, r)| {
                let tag = r.feature_tag().to_be_bytes();
                (feature_rank(tag).unwrap_or(UNLISTED), i, tag)
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
        wanted.sort_by_key(|w| (w.0, w.1));
        wanted.dedup_by_key(|w| (w.0, w.1));
        let all = lookups.lookups();
        let mut reads = MAX_SUBTABLE_READS;
        let mut entries = 0_usize;
        for (rank, lookup, tag) in wanted {
            if reads == 0 || entries >= MAX_ENTRIES {
                break;
            }
            let Ok(subtables) = all.get(usize::from(lookup)).and_then(|l| l.subtables()) else { continue };
            let n = MAX_SUBTABLES.min(reads);
            let listed = rank != UNLISTED;
            // A glyph takes the first subtable of a lookup whose coverage holds it.
            let mut seen: HashSet<u32> = HashSet::new();
            let mut note = |index: &mut Self, input: u32, kind: SubstitutionKind, out: Vec<u32>| {
                if !seen.insert(input) {
                    return;
                }
                let out: Vec<u32> = out.into_iter().filter(|g| *g < glyphs).collect();
                for g in &out {
                    if *g != input && index.reached.len() < MAX_ENTRIES {
                        index.reached.entry(*g).or_insert((tag, Some(input)));
                    }
                }
                if listed && !out.is_empty() && entries < MAX_ENTRIES {
                    entries += 1;
                    index.by_input.entry(input).or_default().push(Substitution { feature: tag, kind, glyphs: out });
                }
            };
            match subtables {
                SubstitutionSubtables::Single(s) => {
                    for t in s.iter().take(n).inspect(|_| reads -= 1).flatten() {
                        for (input, to) in single_pairs(&t, MAX_ENTRIES) {
                            note(&mut index, input, SubstitutionKind::Single, vec![to]);
                        }
                    }
                }
                SubstitutionSubtables::Alternate(s) => {
                    for t in s.iter().take(n).inspect(|_| reads -= 1).flatten() {
                        for (input, set) in alternate_sets(&t, MAX_ENTRIES) {
                            note(&mut index, input, SubstitutionKind::Alternate, set);
                        }
                    }
                }
                SubstitutionSubtables::Multiple(s) => {
                    let mut covered: HashSet<u32> = HashSet::new();
                    for t in s.iter().take(n).inspect(|_| reads -= 1).flatten() {
                        for (input, to) in multiple_pairs(&t, MAX_ENTRIES) {
                            // Not listed as alternates, only reached.
                            if covered.insert(input) {
                                for g in to.into_iter().filter(|g| *g < glyphs && *g != input) {
                                    if index.reached.len() < MAX_ENTRIES {
                                        index.reached.entry(g).or_insert((tag, Some(input)));
                                    }
                                }
                            }
                        }
                    }
                }
                SubstitutionSubtables::Ligature(s) => {
                    for t in s.iter().take(n).inspect(|_| reads -= 1).flatten() {
                        for g in ligature_glyphs(&t, MAX_ENTRIES).into_iter().filter(|g| *g < glyphs) {
                            if index.reached.len() < MAX_ENTRIES {
                                index.reached.entry(g).or_insert((tag, None));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        index
    }
}

/// The most substitutions [`FontFace::glyph_origin`] follows back.
const MAX_ORIGIN_STEPS: usize = 8;

/// The character a glyph comes from ([`FontFace::glyph_origin`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphOrigin {
    pub ch: char,
    /// The features from the glyph back to the character's glyph, the glyph's own first.
    pub features: Vec<[u8; 4]>,
}

impl GlyphOrigin {
    /// The place in a word that the glyph's features give it: the first of `init`, `medi`, `fina`
    /// and `isol` among them, `None` when none is (a letter that does not join).
    pub fn position(&self) -> Option<crate::JoiningPosition> {
        self.features.iter().find_map(|f| match f {
            b"init" => Some(crate::JoiningPosition::Initial),
            b"medi" => Some(crate::JoiningPosition::Medial),
            b"fina" => Some(crate::JoiningPosition::Final),
            b"isol" => Some(crate::JoiningPosition::Isolated),
            _ => None,
        })
    }
}

/// Where a letter stands in a word, as the `init`, `medi`, `fina` and `isol` features say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoiningPosition {
    Initial,
    Medial,
    Final,
    Isolated,
}

impl FontFace {
    fn gsub_index(&self) -> &GsubIndex {
        self.gsub.get_or_init(|| GsubIndex::read(self))
    }

    /// What the features `aalt`, `salt`, `ss01` to `ss20`, `swsh`, `cv01` to `cv99`, `hist`,
    /// `jalt`, `nalt`, `ornm` and `titl` of the face's `GSUB` table substitute for glyph `gid`, in
    /// that order of features and within a feature in lookup order: the glyph of each single
    /// substitution (lookup type 1) and the whole set of each alternate substitution (type 3), in
    /// extension lookups (type 7) too, as the shaper reads them. A feature gives the glyph its
    /// first substitution (the first subtable of the lookup whose coverage holds it). Ligatures and
    /// contextual substitutions are not read, nor is the choice of a variable font's instance of
    /// `FeatureVariations`. Empty when the face has no `GSUB` table or it cannot be read; glyph ids
    /// past the face's glyph count are left out.
    pub fn substitutions(&self, gid: u32) -> Vec<Substitution> {
        self.gsub_index().by_input.get(&gid).cloned().unwrap_or_default()
    }

    /// The feature of the `GSUB` table that gives glyph `gid` as a substitute (by a single,
    /// multiple, alternate or ligature substitution), a listed feature ([`Self::substitutions`])
    /// before another (`init`, `liga`), if any does. Not read through a contextual lookup.
    pub fn feature_reaching(&self, gid: u32) -> Option<[u8; 4]> {
        self.gsub_index().reached.get(&gid).map(|r| r.0)
    }

    /// Where glyph `gid` comes from: the character whose glyph the `GSUB` table's substitutions
    /// lead to it from, and the features on the way, `gid`'s own first (`aalt`, then `fina` for
    /// `yehfarsi.fina.alt2`, whose character is U+06CC). The chain follows the first substitution
    /// that gives each glyph back to a glyph none gives, as far as [`MAX_ORIGIN_STEPS`] steps; a
    /// presentation-form character that maps a glyph in the chain does not stop it, and the
    /// character is the lowest one outside the Arabic presentation forms that maps the glyph it
    /// ends on (the lowest of all when only presentation forms do). `None` when no character maps
    /// that glyph (a glyph no feature reaches, a ligature).
    pub fn glyph_origin(&self, gid: u32) -> Option<GlyphOrigin> {
        let index = self.gsub_index();
        let mut features = vec![];
        let mut at = gid;
        for _ in 0..MAX_ORIGIN_STEPS {
            let Some((tag, from)) = index.reached.get(&at) else { break };
            features.push(*tag);
            at = (*from)?;
        }
        let font = self.skrifa()?;
        let presentation = |c: u32| (0xFB50..=0xFDFF).contains(&c) || (0xFE70..=0xFEFF).contains(&c);
        let mut chars: Vec<u32> = font.charmap().mappings().filter(|(_, g)| g.to_u32() == at).map(|(c, _)| c).collect();
        chars.sort_by_key(|c| (presentation(*c), *c));
        let ch = chars.first().copied().and_then(char::from_u32)?;
        Some(GlyphOrigin { ch, features })
    }

    /// The name of glyph `gid` from the font's `post` or `CFF` table: `None` for a glyph past the
    /// face's glyphs, or when the font has no names (the ids are then `gid12`).
    pub fn glyph_name(&self, gid: u32) -> Option<String> {
        let font = self.skrifa()?;
        let name = font.glyph_names().get(skrifa::GlyphId::new(gid))?;
        (!name.is_synthesized()).then(|| name.as_str().to_string())
    }
}
