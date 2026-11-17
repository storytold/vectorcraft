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
    /// The face's glyph categories ([`FontFace::glyph_categories`]), read the first time they are
    /// asked for.
    categories: std::sync::OnceLock<Vec<GlyphCategory>>,
}

/// A kind of glyph that the Glyphs panel's Show menu lists by OpenType feature
/// ([`FontFace::glyph_categories`]). The kinds, the features in each and their order are
/// VectorCraft's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlyphCategoryKind {
    /// `liga`, `dlig` and `hlig`.
    Ligatures,
    /// `swsh` and `cswh`.
    Swashes,
    /// `salt`.
    StylisticAlternates,
    /// One of `ss01` to `ss20`, by its number.
    StylisticSet(u8),
    /// `smcp` and `c2sc`.
    SmallCaps,
    /// `ornm`.
    Ornaments,
    /// `frac`, `afrc`, `numr` and `dnom`.
    Fractions,
    /// `lnum`, `onum`, `pnum` and `tnum`.
    Figures,
}

impl GlyphCategoryKind {
    /// Every kind in the order the Show menu lists them, the stylistic sets by number.
    pub fn all() -> Vec<Self> {
        let sets = (1..=20).map(Self::StylisticSet);
        [Self::Ligatures, Self::Swashes, Self::StylisticAlternates]
            .into_iter()
            .chain(sets)
            .chain([Self::SmallCaps, Self::Ornaments, Self::Fractions, Self::Figures])
            .collect()
    }

    /// The `GSUB` features whose substitutions give the kind's glyphs.
    pub fn features(self) -> Vec<[u8; 4]> {
        match self {
            Self::Ligatures => vec![*b"liga", *b"dlig", *b"hlig"],
            Self::Swashes => vec![*b"swsh", *b"cswh"],
            Self::StylisticAlternates => vec![*b"salt"],
            Self::StylisticSet(n) => {
                let [a, b] = [b'0' + n / 10 % 10, b'0' + n % 10];
                vec![[b's', b's', a, b]]
            }
            Self::SmallCaps => vec![*b"smcp", *b"c2sc"],
            Self::Ornaments => vec![*b"ornm"],
            Self::Fractions => vec![*b"frac", *b"afrc", *b"numr", *b"dnom"],
            Self::Figures => vec![*b"lnum", *b"onum", *b"pnum", *b"tnum"],
        }
    }

    /// The kind's id: `ligatures`, `swashes`, `stylisticAlternates`, `ss01` to `ss20`, `smallCaps`,
    /// `ornaments`, `fractions` or `figures`.
    pub fn id(self) -> String {
        match self {
            Self::Ligatures => "ligatures".into(),
            Self::Swashes => "swashes".into(),
            Self::StylisticAlternates => "stylisticAlternates".into(),
            Self::StylisticSet(n) => format!("ss{n:02}"),
            Self::SmallCaps => "smallCaps".into(),
            Self::Ornaments => "ornaments".into(),
            Self::Fractions => "fractions".into(),
            Self::Figures => "figures".into(),
        }
    }

    /// The kind whose [`Self::id`] is `id`.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::all().into_iter().find(|k| k.id() == id)
    }
}

/// The glyphs of one [`GlyphCategoryKind`] that a face has ([`FontFace::glyph_categories`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphCategory {
    pub kind: GlyphCategoryKind,
    /// A stylistic set's name from the font's `name` table, when the font gives one.
    pub name: Option<String>,
    /// The glyphs the kind's features give, each once, by glyph id.
    pub glyphs: Vec<u32>,
}

/// Read `face`'s glyph categories ([`FontFace::glyph_categories`]). The caps of [`GsubIndex::read`]
/// bound what a damaged table makes it read, and all the lookups together give at most
/// `MAX_ENTRIES` glyphs.
fn read_categories(face: &FontFace) -> Vec<GlyphCategory> {
    let Some(font) = face.skrifa() else { return vec![] };
    let Ok(gsub) = font.gsub() else { return vec![] };
    let (Ok(features), Ok(lookups)) = (gsub.feature_list(), gsub.lookup_list()) else { return vec![] };
    let glyphs = font.maxp().map_or(0, |m| u32::from(m.num_glyphs()));
    let kinds = GlyphCategoryKind::all();
    // The lookups of each kind's features, in every script's feature records.
    let mut wanted: Vec<Vec<u16>> = vec![vec![]; kinds.len()];
    let mut total = 0_usize;
    for record in features.feature_records().iter().take(MAX_FEATURES) {
        let tag = record.feature_tag().to_be_bytes();
        let Some(k) = kinds.iter().position(|kind| kind.features().contains(&tag)) else { continue };
        let Ok(feature) = record.feature(features.offset_data()) else { continue };
        for i in feature.lookup_list_indices().iter().take(MAX_FEATURE_LOOKUPS) {
            if total >= MAX_LOOKUPS {
                break;
            }
            if !wanted[k].contains(&i.get()) {
                wanted[k].push(i.get());
                total += 1;
            }
        }
    }
    let all = lookups.lookups();
    let mut reads = MAX_SUBTABLE_READS;
    // What each lookup gives, read once when several kinds share it.
    let mut read: HashMap<u16, Vec<u32>> = HashMap::new();
    // The glyphs all the lookups read together may give: at most `MAX_ENTRIES`.
    let mut left = MAX_ENTRIES;
    let mut outputs = |lookup: u16, reads: &mut usize| -> Vec<u32> {
        if let Some(out) = read.get(&lookup) {
            return out.clone();
        }
        // A lookup not read yet once the read budget is spent gives nothing, and is not cached.
        if *reads == 0 || left == 0 {
            return vec![];
        }
        // Each subtable adds only what still fits in what is left of the glyph budget.
        let cap = left;
        let mut out: Vec<u32> = vec![];
        // The glyphs an earlier subtable of the lookup covers: OpenType applies the first subtable
        // that covers a glyph, so a later one's substitution of it is never made.
        let mut covered: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let mut first = |from: u32| covered.insert(from);
        if let Ok(subtables) = all.get(usize::from(lookup)).and_then(|l| l.subtables()) {
            let n = MAX_SUBTABLES.min(*reads);
            let room = |out: &Vec<u32>| cap.saturating_sub(out.len());
            match subtables {
                SubstitutionSubtables::Single(s) => {
                    for t in s.iter().take(n).inspect(|_| *reads -= 1).flatten() {
                        let r = room(&out);
                        out.extend(single_pairs(&t, r).into_iter().filter(|(from, _)| first(*from)).map(|(_, to)| to).take(r));
                        if room(&out) == 0 {
                            break;
                        }
                    }
                }
                SubstitutionSubtables::Alternate(s) => {
                    for t in s.iter().take(n).inspect(|_| *reads -= 1).flatten() {
                        let r = room(&out);
                        out.extend(alternate_sets(&t, r).into_iter().filter(|(from, _)| first(*from)).flat_map(|(_, set)| set).take(r));
                        if room(&out) == 0 {
                            break;
                        }
                    }
                }
                SubstitutionSubtables::Multiple(s) => {
                    for t in s.iter().take(n).inspect(|_| *reads -= 1).flatten() {
                        let r = room(&out);
                        out.extend(multiple_pairs(&t, r).into_iter().filter(|(from, _)| first(*from)).flat_map(|(_, to)| to).take(r));
                        if room(&out) == 0 {
                            break;
                        }
                    }
                }
                SubstitutionSubtables::Ligature(s) => {
                    for t in s.iter().take(n).inspect(|_| *reads -= 1).flatten() {
                        let r = room(&out);
                        out.extend(ligature_glyphs(&t, r).into_iter().take(r));
                        if room(&out) == 0 {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        left = left.saturating_sub(out.len());
        read.insert(lookup, out.clone());
        out
    };
    let names = face.stylistic_sets();
    let mut categories = vec![];
    for (kind, lookups) in kinds.into_iter().zip(wanted) {
        let mut found: Vec<u32> = vec![];
        for lookup in lookups {
            let room = MAX_ENTRIES.saturating_sub(found.len());
            found.extend(outputs(lookup, &mut reads).into_iter().filter(|g| *g != 0 && *g < glyphs).take(room));
            if found.len() >= MAX_ENTRIES {
                break;
            }
        }
        found.sort_unstable();
        found.dedup();
        found.truncate(MAX_ENTRIES);
        if found.is_empty() {
            continue;
        }
        let name = match kind {
            GlyphCategoryKind::StylisticSet(n) => names.iter().find(|s| s.number == n).and_then(|s| s.name.clone()),
            _ => None,
        };
        categories.push(GlyphCategory { kind, name, glyphs: found });
    }
    categories
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

/// Does `features` (a `GSUB` or `GPOS` feature list) have feature `tag` with at least one lookup?
/// At most [`MAX_FEATURES`] feature records are read.
fn has_feature(features: &skrifa::raw::tables::layout::FeatureList<'_>, tag: [u8; 4]) -> bool {
    features
        .feature_records()
        .iter()
        .take(MAX_FEATURES)
        .filter(|r| r.feature_tag().to_be_bytes() == tag)
        .any(|r| r.feature(features.offset_data()).is_ok_and(|f| !f.lookup_list_indices().is_empty()))
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

    /// Does the face's `GSUB` table have feature `tag` with at least one lookup? At most
    /// [`MAX_FEATURES`] feature records are read; false when the face has no `GSUB` table or it
    /// cannot be read.
    pub fn has_substitution_feature(&self, tag: [u8; 4]) -> bool {
        let Some(font) = self.skrifa() else { return false };
        font.gsub().ok().and_then(|g| g.feature_list().ok()).is_some_and(|features| has_feature(&features, tag))
    }

    /// Does the face's `GPOS` table have feature `tag` with at least one lookup? At most
    /// [`MAX_FEATURES`] feature records are read; false when the face has no `GPOS` table or it
    /// cannot be read.
    pub fn has_positioning_feature(&self, tag: [u8; 4]) -> bool {
        let Some(font) = self.skrifa() else { return false };
        font.gpos().ok().and_then(|g| g.feature_list().ok()).is_some_and(|features| has_feature(&features, tag))
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

    /// Do [`Self::substitutions`] of glyph `gid` give a glyph other than `gid` itself? It reads the
    /// face's index without copying the substitutions, so the Glyphs panel asks it for each cell it
    /// draws.
    pub fn has_alternates(&self, gid: u32) -> bool {
        self.gsub_index().by_input.get(&gid).is_some_and(|subs| subs.iter().any(|s| s.glyphs.iter().any(|&g| g != gid)))
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

    /// The kinds of glyph the face's `GSUB` table gives ([`GlyphCategoryKind`]), in the order of
    /// [`GlyphCategoryKind::all`], each with the glyphs that the single, alternate, multiple and
    /// ligature substitutions of its features put in place of others (in extension lookups too),
    /// each glyph once by id, `.notdef` and ids past the face's glyph count left out. A single,
    /// alternate or multiple substitution of a glyph an earlier subtable of its lookup covers is
    /// left out, since that substitution is never made. A kind
    /// without glyphs is left out. A stylistic set takes its name from the font's `name` table,
    /// when the font gives one. Contextual lookups are not followed, nor is the choice of a
    /// variable font's instance of `FeatureVariations`. Read once for the face; empty when it has
    /// no `GSUB` table or the table cannot be read.
    pub fn glyph_categories(&self) -> &[GlyphCategory] {
        self.gsub_index().categories.get_or_init(|| read_categories(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test font lists ligatures, stylistic set 2 with its name, small caps and figures, by
    /// glyph id; its `cswh` feature has no lookups, so it lists no swashes, and small caps leave out
    /// the C of a substitution an earlier subtable shadows.
    #[test]
    fn categories_list_the_glyphs_each_kind_of_feature_gives() {
        use crate::test_fonts::{CATEGORIES_FAMILY, categories_font, source_sans_gid};
        let db = super::super::FontDb::with_font_dirs(vec![]);
        assert_eq!(db.add_font(categories_font().expect("the test font builds")), 1);
        let face = db.face(CATEGORIES_FAMILY, "Regular").expect("the test face");
        let g = |c| u32::from(source_sans_gid(c).expect("a glyph"));
        let mut figures = vec![g('7'), g('8')];
        figures.sort_unstable();
        let found: Vec<(String, Option<String>, Vec<u32>)> =
            face.glyph_categories().iter().map(|c| (c.kind.id(), c.name.clone(), c.glyphs.clone())).collect();
        assert_eq!(
            found,
            [
                ("ligatures".to_string(), None, vec![g('L')]),
                ("ss02".to_string(), Some("Round Dots".to_string()), vec![g('G')]),
                ("smallCaps".to_string(), None, vec![g('B')]),
                ("figures".to_string(), None, figures),
            ]
        );
        assert!(std::ptr::eq(face.glyph_categories(), face.glyph_categories()), "read once");
    }

    /// A face whose `GSUB` table has no features has no categories.
    #[test]
    fn a_face_without_features_has_no_categories() {
        use crate::test_fonts::{HALF_FORM_FAMILY, half_form_font};
        let db = super::super::FontDb::with_font_dirs(vec![]);
        assert_eq!(db.add_font(half_form_font(false).expect("the test font builds")), 1);
        let face = db.face(HALF_FORM_FAMILY, "Regular").expect("the test face");
        assert!(face.glyph_categories().is_empty());
    }

    #[test]
    fn kinds_round_trip_through_their_ids() {
        for kind in GlyphCategoryKind::all() {
            assert_eq!(GlyphCategoryKind::from_id(&kind.id()), Some(kind));
        }
        assert_eq!(GlyphCategoryKind::StylisticSet(7).features(), [*b"ss07"]);
        assert_eq!(GlyphCategoryKind::StylisticSet(20).features(), [*b"ss20"]);
        assert_eq!(GlyphCategoryKind::from_id("ss21"), None);
        assert_eq!(GlyphCategoryKind::from_id("Ligatures"), None);
    }
}
