//! Fonts built in code for tests, so no font file is committed: a small variable font (#296).
//! Compiled for this crate's tests and, with the `test-fonts` feature, for other crates' tests.

use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::Glyph;
use skrifa::{GlyphId, MetadataProvider};
use write_fonts::FontBuilder;
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, InstanceRecord, VariationAxisRecord};
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar, Tent};
use write_fonts::types::{F2Dot14, Fixed, NameId, Tag};

/// The family of [`variable_font`].
pub const VARIABLE_FAMILY: &str = "Varitest Sans";

/// The characters [`variable_font`] widens towards its heaviest weight (simple glyphs in Source Sans
/// 3; the composite ones, like `H`, would need their components varied).
pub const VARIABLE_CHARS: &str = "lIn";

/// How much wider [`VARIABLE_CHARS`] are at the heaviest weight (outlines and advances).
pub const VARIABLE_WIDEN: f32 = 0.25;

/// The bundled Source Sans 3 Regular as a variable font of family [`VARIABLE_FAMILY`]: one weight
/// axis (400–700, default 400, the face itself being Regular) whose maximum makes the glyphs of
/// [`VARIABLE_CHARS`] [`VARIABLE_WIDEN`] wider (outlines and advances, through `gvar`), with the
/// named instances Normal (the default, the face Regular), SemiBold (550) and Bold (700), plus an instance without
/// a name and one named like another (`BOLD`), which style lists leave out. `None` when it can't
/// be built.
pub fn variable_font() -> Option<Vec<u8>> {
    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let (glyf, loca) = (font.glyf().ok()?, font.loca(None).ok()?);
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let charmap = font.charmap();
    let widened: Vec<GlyphId> = VARIABLE_CHARS.chars().filter_map(|c| charmap.map(c)).collect();
    let mut variations = Vec::new();
    for g in 0..u32::from(font.maxp().ok()?.num_glyphs()) {
        let gid = write_fonts::types::GlyphId::new(g);
        let id = GlyphId::new(g);
        let deltas = if widened.contains(&id) {
            let Ok(Some(Glyph::Simple(s))) = loca.get_glyf(id, &glyf) else { return None };
            let wider = |x: f32| (x * VARIABLE_WIDEN).round() as i16;
            let mut d: Vec<GlyphDelta> = s.points().map(|p| GlyphDelta::required(wider(f32::from(p.x)), 0)).collect();
            // The phantom points: left side bearing, advance, top, bottom.
            let advance = wider(metrics.advance_width(id)?);
            d.extend([(0, 0), (advance, 0), (0, 0), (0, 0)].map(|(x, y)| GlyphDelta::required(x, y)));
            vec![GlyphDeltas::new(vec![Tent::new(F2Dot14::from_f32(1.0), None)], d)]
        } else {
            vec![]
        };
        variations.push(GlyphVariations::new(gid, deltas));
    }
    let gvar = Gvar::new(variations, 1).ok()?;
    let axis = VariationAxisRecord {
        axis_tag: Tag::new(b"wght"),
        min_value: Fixed::from_i32(400),
        default_value: Fixed::from_i32(400),
        max_value: Fixed::from_i32(700),
        flags: 0,
        axis_name_id: NameId::new(256),
    };
    let instance = |name: u16, wght: i32| InstanceRecord {
        subfamily_name_id: NameId::new(name),
        flags: 0,
        coordinates: vec![Fixed::from_i32(wght)],
        post_script_name_id: None,
    };
    let instances = vec![instance(257, 400), instance(258, 550), instance(259, 700), instance(260, 600), instance(261, 650)];
    let fvar = Fvar::new(AxisInstanceArrays::new(vec![axis], instances));
    let name = name_table(&[
        (1, VARIABLE_FAMILY),
        (2, "Regular"),
        (4, "Varitest Sans Regular"),
        (6, "VaritestSans-Regular"),
        (256, "Weight"),
        (257, "Normal"),
        (258, "SemiBold"),
        (259, "Bold"),
        (261, "BOLD"),
    ])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    builder.add_table(&fvar).ok()?.add_table(&gvar).ok()?;
    // The rest of Source Sans 3's tables as they are.
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`two_axis_font`].
pub const TWO_AXIS_FAMILY: &str = "Biaxtest Sans";

/// How much narrower [`two_axis_font`] makes [`VARIABLE_CHARS`] at its narrowest width.
pub const TWO_AXIS_NARROW: f32 = 0.2;

/// The bundled Source Sans 3 Regular as a variable font of family [`TWO_AXIS_FAMILY`] with two axes:
/// weight (`wght` 400–700, default 400), whose maximum makes the glyphs of [`VARIABLE_CHARS`]
/// [`VARIABLE_WIDEN`] wider, and width (`wdth` 75–100, default 100), whose minimum makes them
/// [`TWO_AXIS_NARROW`] narrower (outlines and advances, through `gvar`). Its named instances are
/// Regular (the default), Bold (`wght` 700) and Condensed (`wdth` 75). `None` when it can't be
/// built.
pub fn two_axis_font() -> Option<Vec<u8>> {
    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let (glyf, loca) = (font.glyf().ok()?, font.loca(None).ok()?);
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let charmap = font.charmap();
    let varied: Vec<GlyphId> = VARIABLE_CHARS.chars().filter_map(|c| charmap.map(c)).collect();
    let mut variations = Vec::new();
    for g in 0..u32::from(font.maxp().ok()?.num_glyphs()) {
        let id = GlyphId::new(g);
        let mut deltas = vec![];
        if varied.contains(&id) {
            let Ok(Some(Glyph::Simple(s))) = loca.get_glyf(id, &glyf) else { return None };
            let advance = metrics.advance_width(id)?;
            // At each axis's far end: every point and the advance phantom point moved by `k` of their x.
            let moved = |k: f32| {
                let by = |x: f32| (x * k).round() as i16;
                let mut d: Vec<GlyphDelta> = s.points().map(|p| GlyphDelta::required(by(f32::from(p.x)), 0)).collect();
                d.extend([(0, 0), (by(advance), 0), (0, 0), (0, 0)].map(|(x, y)| GlyphDelta::required(x, y)));
                d
            };
            let peak = |w: f32, d: f32| vec![Tent::new(F2Dot14::from_f32(w), None), Tent::new(F2Dot14::from_f32(d), None)];
            deltas.push(GlyphDeltas::new(peak(1.0, 0.0), moved(VARIABLE_WIDEN)));
            deltas.push(GlyphDeltas::new(peak(0.0, -1.0), moved(-TWO_AXIS_NARROW)));
        }
        variations.push(GlyphVariations::new(write_fonts::types::GlyphId::new(g), deltas));
    }
    let gvar = Gvar::new(variations, 2).ok()?;
    let axis = |tag: &[u8; 4], min: i32, default: i32, max: i32, name: u16| VariationAxisRecord {
        axis_tag: Tag::new(tag),
        min_value: Fixed::from_i32(min),
        default_value: Fixed::from_i32(default),
        max_value: Fixed::from_i32(max),
        flags: 0,
        axis_name_id: NameId::new(name),
    };
    let instance = |name: u16, wght: i32, wdth: i32| InstanceRecord {
        subfamily_name_id: NameId::new(name),
        flags: 0,
        coordinates: vec![Fixed::from_i32(wght), Fixed::from_i32(wdth)],
        post_script_name_id: None,
    };
    let axes = vec![axis(b"wght", 400, 400, 700, 256), axis(b"wdth", 75, 100, 100, 257)];
    let fvar = Fvar::new(AxisInstanceArrays::new(axes, vec![instance(258, 400, 100), instance(259, 700, 100), instance(260, 400, 75)]));
    let name = name_table(&[
        (1, TWO_AXIS_FAMILY),
        (2, "Regular"),
        (4, "Biaxtest Sans Regular"),
        (6, "BiaxtestSans-Regular"),
        (256, "Weight"),
        (257, "Width"),
        (258, "Regular"),
        (259, "Bold"),
        (260, "Condensed"),
    ])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    builder.add_table(&fvar).ok()?.add_table(&gvar).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// A `name` table of Windows English (US) records (name id, string).
pub fn name_table(records: &[(u16, &str)]) -> Option<Vec<u8>> {
    let mut records = records.to_vec();
    records.sort_by_key(|r| r.0);
    let n = u16::try_from(records.len()).ok()?;
    let (mut head, mut strings) = (Vec::new(), Vec::new());
    for v in [0, n, 6 + 12 * n] {
        head.extend_from_slice(&v.to_be_bytes());
    }
    for (id, s) in records {
        let bytes: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [3, 1, 0x0409, id, u16::try_from(bytes.len()).ok()?, u16::try_from(strings.len()).ok()?] {
            head.extend_from_slice(&v.to_be_bytes());
        }
        strings.extend(bytes);
    }
    head.extend(strings);
    Some(head)
}

/// The family of [`vertical_font`].
pub const VERTICAL_FAMILY: &str = "Vertitest Sans";

/// The character whose vertical metrics [`vertical_font`] sets apart: 1.4 em down the column, its
/// vertical origin 1.1 em above the baseline.
pub const VERTICAL_TALL: char = '§';

/// The bundled Source Sans 3 Regular as family [`VERTICAL_FAMILY`] with vertical metrics (`vhea`,
/// `vmtx`, and `VORG` when `vorg`): every glyph one em down the column with its vertical origin at
/// the usual 0.88 em, except [`VERTICAL_TALL`] (1.4 em, origin 1.1 em). Without `VORG` the origin
/// comes from each glyph's top and top side bearing, as in TrueType fonts. `None` when it can't be
/// built.
pub fn vertical_font(vorg: bool) -> Option<Vec<u8>> {
    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let n = font.maxp().ok()?.num_glyphs();
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let tall = font.charmap().map(VERTICAL_TALL)?;
    let cell = |g: GlyphId| -> (u16, i16) { if g == tall { (1400, 1100) } else { (1000, 880) } };
    let be16 = |v: &mut Vec<u8>, x: i16| v.extend_from_slice(&x.to_be_bytes());
    // vmtx: (advance height, top side bearing = origin − the glyph's top) for every glyph.
    let mut vmtx = Vec::with_capacity(4 * usize::from(n));
    for g in 0..u32::from(n) {
        let id = GlyphId::new(g);
        let (advance, origin) = cell(id);
        let top = metrics.bounds(id).map_or(0.0, |b| b.y_max).round() as i16;
        vmtx.extend_from_slice(&advance.to_be_bytes());
        be16(&mut vmtx, origin - top);
    }
    // vhea 1.1: ascent/descent of the em box across the column, then the metrics' count.
    let mut vhea = Vec::with_capacity(36);
    vhea.extend_from_slice(&0x0001_1000_u32.to_be_bytes());
    for x in [500, -500, 0] {
        be16(&mut vhea, x);
    }
    vhea.extend_from_slice(&1400_u16.to_be_bytes());
    for x in [0, 0, 1400, 1, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut vhea, x);
    }
    vhea.extend_from_slice(&n.to_be_bytes());
    let name = name_table(&[(1, VERTICAL_FAMILY), (2, "Regular"), (4, "Vertitest Sans Regular"), (6, "VertitestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    builder.add_raw(Tag::new(b"vhea"), vhea);
    builder.add_raw(Tag::new(b"vmtx"), vmtx);
    if vorg {
        // VORG 1.0: the default origin, then the one glyph that differs.
        let mut t = Vec::new();
        for x in [1, 0, 880, 1] {
            be16(&mut t, x);
        }
        t.extend_from_slice(&u16::try_from(tall.to_u32()).ok()?.to_be_bytes());
        be16(&mut t, 1100);
        builder.add_raw(Tag::new(b"VORG"), t);
    }
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`vertical_punctuation_font`].
pub const VERTICAL_PUNCTUATION_FAMILY: &str = "Punctuationtest Sans";

/// A test-only cmap over bundled Source Sans 3: fullwidth !/? use the ASCII glyphs,
/// 「 uses L and ー uses a hyphen. Optionally `vert` substitutes I/J for !/?, so tests
/// exercise both nominal fallbacks and actual vertical substitutions without OS fonts.
pub fn vertical_punctuation_font(alternates: bool) -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;
    use write_fonts::tables::gsub::{Gsub, SingleSubstFormat2, SubstitutionLookup, SubstitutionLookupList};
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let mapping = [('！', '!'), ('？', '?'), ('「', 'L'), ('ー', '-'), ('!', '!'), ('?', '?'), ('I', 'I'), ('J', 'J'), ('L', 'L'), ('-', '-')];
    let mappings = mapping.into_iter().map(|(c, source)| Some((c, write_fonts::types::GlyphId::new(charmap.map(source)?.to_u32()))));
    let cmap = Cmap::from_mappings(mappings.collect::<Option<Vec<_>>>()?).ok()?;
    let family = VERTICAL_PUNCTUATION_FAMILY;
    let name = name_table(&[(1, family), (2, "Regular"), (4, "Punctuationtest Sans Regular"), (6, "PunctuationtestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_table(&cmap).ok()?;
    builder.add_raw(Tag::new(b"name"), name);
    if alternates {
        let gid = |c| Some(GlyphId16::new(u16::try_from(charmap.map(c)?.to_u32()).ok()?));
        let mut pairs = [(gid('!')?, gid('I')?), (gid('?')?, gid('J')?)];
        pairs.sort_by_key(|p| p.0);
        let coverage = CoverageFormat1::new(pairs.iter().map(|p| p.0).collect());
        let subst = SingleSubstFormat2::new(coverage.into(), pairs.iter().map(|p| p.1).collect());
        let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(vec![0])), vec![]))]);
        let features = FeatureList::new(vec![FeatureRecord::new(Tag::new(b"vert"), Feature::new(None, vec![0]))]);
        let lookups = SubstitutionLookupList::new(vec![SubstitutionLookup::Single(Lookup::new(Default::default(), vec![subst.into()]))]);
        builder.add_table(&Gsub::new(scripts, features, lookups)).ok()?;
    }
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if tag != Tag::new(b"GSUB") && !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`half_form_font`].
pub const HALF_FORM_FAMILY: &str = "Halftest Sans";

/// A test-only cmap over bundled Source Sans 3: Devanagari Ka uses K, the halant uses a period, Ssa
/// uses S and the zero width joiner uses the space. The `GSUB` table lists the `dev2` script either
/// way. With `half`, its `half` feature substitutes k for K followed by the period (Ka's half form);
/// without it, the script has no features and Ka has no half form. `None` when it can't be built.
pub fn half_form_font(half: bool) -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;
    use write_fonts::tables::gsub::{Gsub, Ligature, LigatureSet, LigatureSubstFormat1, SubstitutionLookup, SubstitutionLookupList};
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let mapping = [('क', 'K'), ('\u{094D}', '.'), ('ष', 'S'), ('\u{200D}', ' ')];
    let mappings = mapping.into_iter().map(|(c, source)| Some((c, write_fonts::types::GlyphId::new(charmap.map(source)?.to_u32()))));
    let cmap = Cmap::from_mappings(mappings.collect::<Option<Vec<_>>>()?).ok()?;
    let family = HALF_FORM_FAMILY;
    let name = name_table(&[(1, family), (2, "Regular"), (4, "Halftest Sans Regular"), (6, "HalftestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_table(&cmap).ok()?;
    builder.add_raw(Tag::new(b"name"), name);
    let (features, lookups) = if half {
        let gid = |c| Some(GlyphId16::new(u16::try_from(charmap.map(c)?.to_u32()).ok()?));
        let coverage = CoverageFormat1::new(vec![gid('K')?]);
        let subst = LigatureSubstFormat1::new(coverage.into(), vec![LigatureSet::new(vec![Ligature::new(gid('k')?, vec![gid('.')?])])]);
        let features = FeatureList::new(vec![FeatureRecord::new(Tag::new(b"half"), Feature::new(None, vec![0]))]);
        let lookups = SubstitutionLookupList::new(vec![SubstitutionLookup::Ligature(Lookup::new(Default::default(), vec![subst]))]);
        (features, lookups)
    } else {
        (FeatureList::new(vec![]), SubstitutionLookupList::new(vec![]))
    };
    let langsys = LangSys::new(if half { vec![0] } else { vec![] });
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"dev2"), Script::new(Some(langsys), vec![]))]);
    builder.add_table(&Gsub::new(scripts, features, lookups)).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`pair_kerning_font`].
pub const PAIR_KERNING_FAMILY: &str = "Pairtest Sans";

/// The bundled Source Sans 3 Regular as family [`PAIR_KERNING_FAMILY`], its `GPOS` replaced by two
/// `kern` pairs, A followed by V and S followed by S: each takes 60 units off the first glyph's
/// advance (the pair's first value record) and moves the second 100 units left (its second).
/// `None` when it can't be built.
pub fn pair_kerning_font() -> Option<Vec<u8>> {
    use write_fonts::tables::gpos::{Gpos, PairPos, PairSet, PairValueRecord, PositionLookup, PositionLookupList, ValueRecord};
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let gid = |c| Some(GlyphId16::new(u16::try_from(charmap.map(c)?.to_u32()).ok()?));
    let pair = |second| PairValueRecord::new(second, ValueRecord::new().with_x_advance(-60), ValueRecord::new().with_x_placement(-100));
    let mut pairs = [(gid('A')?, gid('V')?), (gid('S')?, gid('S')?)];
    pairs.sort_by_key(|p| p.0);
    let coverage = CoverageFormat1::new(pairs.iter().map(|p| p.0).collect());
    let pos = PairPos::format_1(coverage.into(), pairs.iter().map(|p| PairSet::new(vec![pair(p.1)])).collect());
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(vec![0])), vec![]))]);
    let features = FeatureList::new(vec![FeatureRecord::new(Tag::new(b"kern"), Feature::new(None, vec![0]))]);
    let lookups = PositionLookupList::new(vec![PositionLookup::Pair(Lookup::new(Default::default(), vec![pos]))]);
    let family = PAIR_KERNING_FAMILY;
    let name = name_table(&[(1, family), (2, "Regular"), (4, "Pairtest Sans Regular"), (6, "PairtestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_table(&Gpos::new(scripts, features, lookups)).ok()?;
    builder.add_raw(Tag::new(b"name"), name);
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if tag != Tag::new(b"kern") && !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`twin_font`]: two versions of one family and style.
pub const TWIN_FAMILY: &str = "Twintest Sans";

/// The version strings of [`twin_font`]'s older and newer face.
pub const TWIN_VERSIONS: [&str; 2] = ["Version 1.000", "Version 2.000"];

/// A face of family [`TWIN_FAMILY`], style Regular, in version [`TWIN_VERSIONS`]`[0]` (the bundled
/// Source Sans 3 Regular) or, `newer`, `[1]` (the bundled Inter Regular): one name, two sets of
/// glyphs, as a font library that keeps an old version beside a new one has. `None` when it can't
/// be built.
pub fn twin_font(newer: bool) -> Option<Vec<u8>> {
    const OLDER: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    const NEWER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
    let font = skrifa::FontRef::new(if newer { NEWER } else { OLDER }).ok()?;
    let version = TWIN_VERSIONS[usize::from(newer)];
    let name = name_table(&[(1, TWIN_FAMILY), (2, "Regular"), (4, "Twintest Sans Regular"), (5, version), (6, "TwintestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The character whose glyph [`colour_bitmap_font`] draws as a colour bitmap.
pub const COLOUR_CHAR: char = 'E';

/// The colour of [`colour_bitmap_font`]'s bitmap in its largest strike (RGBA); the `sbix` font's
/// smaller strike is [`COLOUR_SMALL`].
pub const COLOUR_LARGE: [u8; 4] = [255, 0, 0, 255];

/// The colour of the `sbix` font's smaller strike.
pub const COLOUR_SMALL: [u8; 4] = [0, 0, 255, 255];

/// The pixels per em of [`colour_bitmap_font`]'s largest strike: its square bitmap is one em wide.
pub const COLOUR_PPEM: u8 = 16;

/// Where [`colour_bitmap_font`]`(false)`'s (`CBDT`) bitmap sits, in pixels: its left edge right of
/// the glyph's origin, its top edge above the baseline.
pub const COLOUR_CBDT_BEARINGS: (i8, i8) = (2, 12);

/// The family of [`colour_bitmap_font`] (`sbix`, or `CBDT`).
pub fn colour_bitmap_family(sbix: bool) -> &'static str {
    if sbix { "Sbixtest Emoji" } else { "Cbdttest Emoji" }
}

/// The bundled Source Sans 3 Regular as family [`colour_bitmap_family`], [`COLOUR_CHAR`] a solid
/// square colour bitmap one em wide: in `sbix` strikes of [`COLOUR_PPEM`] ([`COLOUR_LARGE`]) and
/// half that ([`COLOUR_SMALL`]), each with no origin offset, or (`sbix` false) in one `CBDT`
/// strike of [`COLOUR_PPEM`] ([`COLOUR_LARGE`], bearings [`COLOUR_CBDT_BEARINGS`]). Its outline
/// stays too. Tables written from the OpenType specification. `None` when it can't be built.
pub fn colour_bitmap_font(sbix: bool) -> Option<Vec<u8>> {
    colour_bitmap_font_with(sbix, &solid_png(COLOUR_PPEM.into(), COLOUR_LARGE))
}

/// [`colour_bitmap_font`] with `png` as the bitmap of its largest strike (a malformed one, say).
pub fn colour_bitmap_font_with(sbix: bool, png: &[u8]) -> Option<Vec<u8>> {
    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let glyphs = font.maxp().ok()?.num_glyphs();
    let gid = u16::try_from(font.charmap().map(COLOUR_CHAR)?.to_u32()).ok()?;
    let family = colour_bitmap_family(sbix);
    let ps = family.replace(' ', "");
    let name = name_table(&[(1, family), (2, "Regular"), (4, &format!("{family} Regular")), (6, &format!("{ps}-Regular"))])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    if sbix {
        let small = solid_png((COLOUR_PPEM / 2).into(), COLOUR_SMALL);
        builder.add_raw(Tag::new(b"sbix"), sbix_table(glyphs, gid, &[(COLOUR_PPEM / 2, &small), (COLOUR_PPEM, png)]));
    } else {
        let (cblc, cbdt) = cbdt_tables(gid, png);
        builder.add_raw(Tag::new(b"CBLC"), cblc);
        builder.add_raw(Tag::new(b"CBDT"), cbdt);
    }
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// An `sbix` table of `strikes` (ppem, PNG), each holding a PNG for glyph `gid` alone.
fn sbix_table(glyphs: u16, gid: u16, strikes: &[(u8, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(1u16.to_be_bytes()); // version
    out.extend(1u16.to_be_bytes()); // flags
    out.extend((strikes.len() as u32).to_be_bytes());
    let header = 8 + 4 * strikes.len();
    let mut data = Vec::new();
    for (ppem, png) in strikes {
        out.extend(((header + data.len()) as u32).to_be_bytes());
        let offsets = 4 + 4 * (usize::from(glyphs) + 1);
        let mut strike = Vec::new();
        strike.extend(u16::from(*ppem).to_be_bytes());
        strike.extend(72u16.to_be_bytes()); // ppi
        // Glyph data offsets: empty but for `gid`'s (origin offsets 0, `png `, the PNG).
        let record = 8 + png.len();
        for g in 0..=glyphs {
            let at = offsets + if g > gid { record } else { 0 };
            strike.extend((at as u32).to_be_bytes());
        }
        strike.extend([0, 0, 0, 0]);
        strike.extend(b"png ");
        strike.extend(*png);
        data.extend(strike);
    }
    out.extend(data);
    out
}

/// `CBLC` and `CBDT` tables of one strike of [`COLOUR_PPEM`] holding `png` for glyph `gid`
/// (index subtable format 1, glyph data format 17).
fn cbdt_tables(gid: u16, png: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let ppem = COLOUR_PPEM;
    let mut cbdt = Vec::new();
    cbdt.extend(3u16.to_be_bytes());
    cbdt.extend(0u16.to_be_bytes());
    // Small glyph metrics: height, width, bearing x, bearing y, advance.
    let (bx, by) = COLOUR_CBDT_BEARINGS;
    cbdt.extend([ppem, ppem, bx as u8, by as u8, ppem]);
    cbdt.extend((png.len() as u32).to_be_bytes());
    cbdt.extend(png);
    let mut cblc = Vec::new();
    cblc.extend(3u16.to_be_bytes());
    cblc.extend(0u16.to_be_bytes());
    cblc.extend(1u32.to_be_bytes()); // numSizes
    let array = 8 + 48;
    let subtable = 8;
    let index_size = subtable + 8 + 2 * 4;
    cblc.extend((array as u32).to_be_bytes());
    cblc.extend((index_size as u32).to_be_bytes());
    cblc.extend(1u32.to_be_bytes()); // numberOfIndexSubTables
    cblc.extend(0u32.to_be_bytes()); // colorRef
    // Horizontal and vertical line metrics.
    let line = [ppem as i8, 0, ppem as i8, 1, 0, 0, 0, 0, 0, 0, 0, 0].map(|v| v as u8);
    cblc.extend(line);
    cblc.extend(line);
    cblc.extend(gid.to_be_bytes());
    cblc.extend(gid.to_be_bytes());
    cblc.extend([ppem, ppem, 32, 1]); // ppem x, ppem y, bit depth, flags (horizontal)
    // The index subtable array, then the subtable: format 1, image format 17, its data at 4.
    cblc.extend(gid.to_be_bytes());
    cblc.extend(gid.to_be_bytes());
    cblc.extend((subtable as u32).to_be_bytes());
    cblc.extend(1u16.to_be_bytes());
    cblc.extend(17u16.to_be_bytes());
    cblc.extend(4u32.to_be_bytes());
    cblc.extend(0u32.to_be_bytes());
    cblc.extend(((cbdt.len() - 4) as u32).to_be_bytes());
    (cblc, cbdt)
}

/// A `side`×`side` PNG of one colour (RGBA), written uncompressed (stored deflate blocks).
pub fn solid_png(side: u32, rgba: [u8; 4]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut c = !0u32;
        for b in bytes {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    }
    // Each row: filter type 0 (none), then its pixels.
    let row: Vec<u8> = std::iter::once(0).chain(rgba.repeat(side as usize)).collect();
    let raw = row.repeat(side as usize);
    let (mut a, mut b) = (1u32, 0u32);
    for v in &raw {
        a = (a + u32::from(*v)) % 65521;
        b = (b + a) % 65521;
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(0xffff).collect();
    for (i, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(i + 1 == blocks.len()));
        let n = block.len() as u16;
        zlib.extend(n.to_le_bytes());
        zlib.extend((!n).to_le_bytes());
        zlib.extend(*block);
    }
    zlib.extend(((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(side.to_be_bytes());
    ihdr.extend(side.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]); // 8 bits, RGBA
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// The palette of [`colr_font`] (palette 0, RGBA): its indices 0, 1 and 2.
pub const COLR_RED: [u8; 4] = [255, 0, 0, 255];
pub const COLR_BLUE: [u8; 4] = [0, 0, 255, 255];
pub const COLR_GREEN: [u8; 4] = [0, 255, 0, 255];

/// The character whose glyph [`colr_font`]`(true)` paints with a sweep gradient.
pub const COLR_SWEEP_CHAR: char = 'S';

/// The family of [`colr_font`] (COLRv1, or v0).
pub fn colr_family(v1: bool) -> &'static str {
    if v1 { "Colrtest Paints" } else { "Colrtest Layers" }
}

/// The glyph ids of `chars` in the bundled Source Sans 3.
fn source_sans_gids<const N: usize>(chars: [char; N]) -> Option<[u16; N]> {
    let font = skrifa::FontRef::new(SOURCE_SANS).ok()?;
    let mut gids = [0; N];
    for (gid, ch) in gids.iter_mut().zip(chars) {
        *gid = u16::try_from(font.charmap().map(ch)?.to_u32()).ok()?;
    }
    Some(gids)
}

const SOURCE_SANS: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");

/// The bundled Source Sans 3 Regular as family [`colr_family`], with `COLR` and `CPAL` tables
/// written from the OpenType specification, palette 0 [`COLR_RED`], [`COLR_BLUE`], [`COLR_GREEN`].
/// v0: [`COLOUR_CHAR`] is the outline of `O` in red, then that of `I` in the foreground colour
/// (palette index 0xFFFF). v1: [`COLOUR_CHAR`] is, in a clip box, `O` filled with a linear
/// gradient red → blue, `I` scaled by a half about the origin with a radial gradient blue → green,
/// and `-` in green composited with Multiply over `-` in red; [`COLR_SWEEP_CHAR`] is `O` with a
/// sweep gradient red → blue. Their outlines stay. `None` when it can't be built.
pub fn colr_font(v1: bool) -> Option<Vec<u8>> {
    use write_fonts::tables::colr::{
        BaseGlyph, BaseGlyphList, BaseGlyphPaint, Clip, ClipBox, ClipList, ColorLine, ColorStop, Colr, CompositeMode, Extend, Layer, LayerList, Paint,
    };
    use write_fonts::types::{FWord, GlyphId16, UfWord};
    let [base, sweep, o, i, dash] = source_sans_gids([COLOUR_CHAR, COLR_SWEEP_CHAR, 'O', 'I', '-'])?;
    let (base, sweep, o, i, dash) = (GlyphId16::new(base), GlyphId16::new(sweep), GlyphId16::new(o), GlyphId16::new(i), GlyphId16::new(dash));
    let colr = if v1 {
        let one = F2Dot14::from_f32(1.0);
        let line = |from: u16, to: u16| {
            ColorLine::new(Extend::Pad, 2, vec![ColorStop::new(F2Dot14::from_f32(0.0), from, one), ColorStop::new(one, to, one)])
        };
        let w = FWord::new;
        let layers = vec![
            Paint::glyph(Paint::linear_gradient(line(0, 1), w(0), w(0), w(500), w(0), w(0), w(500)), o),
            Paint::scale_uniform(
                Paint::glyph(Paint::radial_gradient(line(1, 2), w(250), w(300), UfWord::new(0), w(250), w(300), UfWord::new(300)), i),
                F2Dot14::from_f32(0.5),
            ),
            Paint::composite(Paint::glyph(Paint::solid(2, one), dash), CompositeMode::Multiply, Paint::glyph(Paint::solid(0, one), dash)),
        ];
        let mut colr = Colr::new(0, None, None, 0);
        colr.layer_list = Some(LayerList::new(layers.len() as u32, layers)).into();
        let mut glyphs = vec![
            BaseGlyphPaint::new(base, Paint::colr_layers(3, 0)),
            BaseGlyphPaint::new(sweep, Paint::glyph(Paint::sweep_gradient(line(0, 1), w(250), w(300), F2Dot14::from_f32(0.0), one), o)),
        ];
        glyphs.sort_by_key(|g| g.glyph_id);
        colr.base_glyph_list = Some(BaseGlyphList::new(glyphs.len() as u32, glyphs)).into();
        let clip = ClipBox::format_1(w(-50), w(-250), w(750), w(800));
        colr.clip_list = Some(ClipList::new(1, 1, vec![Clip::new(base, base, clip)])).into();
        colr
    } else {
        Colr::new(1, Some(vec![BaseGlyph::new(base, 0, 2)]), Some(vec![Layer::new(o, 0), Layer::new(i, 0xFFFF)]), 2)
    };
    colr_font_with(colr_family(v1), &colr)
}

/// The bundled Source Sans 3 Regular as `family` with `colr` and [`colr_font`]'s palette.
pub fn colr_font_with(family: &str, colr: &write_fonts::tables::colr::Colr) -> Option<Vec<u8>> {
    with_colr(SOURCE_SANS, family_names(family, &[])?, colr)
}

/// The family of [`colr_variable_font`].
pub const COLR_VARIABLE_FAMILY: &str = "Colrtest Variable";

/// [`variable_font`] as family [`COLR_VARIABLE_FAMILY`], with a `COLR` (v0) table in which
/// [`COLOUR_CHAR`] is the outline of `l` (one of [`VARIABLE_CHARS`]) in red, so its layer widens
/// with the weight. `None` when it can't be built.
pub fn colr_variable_font() -> Option<Vec<u8>> {
    use write_fonts::tables::colr::{BaseGlyph, Colr, Layer};
    use write_fonts::types::GlyphId16;
    let [base, l] = source_sans_gids([COLOUR_CHAR, 'l'])?;
    let colr = Colr::new(1, Some(vec![BaseGlyph::new(GlyphId16::new(base), 0, 1)]), Some(vec![Layer::new(GlyphId16::new(l), 0)]), 1);
    let axis_names = [(256, "Weight"), (257, "Normal"), (258, "SemiBold"), (259, "Bold")];
    with_colr(&variable_font()?, family_names(COLR_VARIABLE_FAMILY, &axis_names)?, &colr)
}

/// A `name` table for Regular of `family`, with `more` records.
fn family_names(family: &str, more: &[(u16, &str)]) -> Option<Vec<u8>> {
    let ps = family.replace(' ', "");
    let (full, postscript) = (format!("{family} Regular"), format!("{ps}-Regular"));
    let mut records = vec![(1, family), (2, "Regular"), (4, full.as_str()), (6, postscript.as_str())];
    records.extend_from_slice(more);
    name_table(&records)
}

/// Font `base` with the `name` table `name`, `colr` and [`colr_font`]'s palette.
fn with_colr(base: &[u8], name: Vec<u8>, colr: &write_fonts::tables::colr::Colr) -> Option<Vec<u8>> {
    use write_fonts::tables::cpal::{ColorRecord, Cpal};
    let font = skrifa::FontRef::new(base).ok()?;
    let records = [COLR_RED, COLR_BLUE, COLR_GREEN].map(|[r, g, b, a]| ColorRecord::new(b, g, r, a)).to_vec();
    let cpal = Cpal::new(3, 1, 3, Some(records), vec![0]);
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    builder.add_table(colr).ok()?;
    builder.add_table(&cpal).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The character whose glyph [`svg_font`]'s first document draws.
pub const SVG_CHAR: char = 'E';

/// The glyph id of `ch` in the bundled Source Sans 3 (and the fonts made from it).
pub fn source_sans_gid(ch: char) -> Option<u16> {
    source_sans_gids([ch]).map(|[g]| g)
}

/// The bundled Source Sans 3 Regular as `family` with an `SVG ` table written from the OpenType
/// specification: `documents` are (first glyph, last glyph, SVG document), in glyph order. Its
/// outlines stay. `None` when it can't be built.
pub fn svg_font(family: &str, documents: &[(u16, u16, &str)]) -> Option<Vec<u8>> {
    let font = skrifa::FontRef::new(SOURCE_SANS).ok()?;
    let ps = family.replace(' ', "");
    let name = name_table(&[(1, family), (2, "Regular"), (4, &format!("{family} Regular")), (6, &format!("{ps}-Regular"))])?;
    // The header (version, the document list's offset, reserved), then the document list: its
    // records (first and last glyph, the document's offset from the list and length), then the
    // documents.
    let records = documents.len();
    let mut list = (u16::try_from(records).ok()?).to_be_bytes().to_vec();
    let mut offset = 2 + 12 * records;
    for (first, last, doc) in documents {
        list.extend(first.to_be_bytes());
        list.extend(last.to_be_bytes());
        list.extend((u32::try_from(offset).ok()?).to_be_bytes());
        list.extend((u32::try_from(doc.len()).ok()?).to_be_bytes());
        offset += doc.len();
    }
    for (_, _, doc) in documents {
        list.extend(doc.as_bytes());
    }
    let mut svg = vec![0, 0, 0, 0, 0, 10, 0, 0, 0, 0];
    svg.extend(list);
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    builder.add_raw(Tag::new(b"SVG "), svg);
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}
