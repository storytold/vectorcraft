//! Fonts built in code for tests, so no font file is committed: a small variable font (#296).
//! Compiled for this crate's tests and, with the `test-fonts` feature, for other crates' tests.

use std::sync::atomic::{AtomicBool, Ordering};

use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::Glyph;
use skrifa::{GlyphId, MetadataProvider};
use write_fonts::FontBuilder;
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, InstanceRecord, VariationAxisRecord};
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar, Tent};
use write_fonts::types::{F2Dot14, Fixed, NameId, Tag};

/// Whether faces use and keep shaping plans ([`keep_shape_plans`]).
static KEEP_SHAPE_PLANS: AtomicBool = AtomicBool::new(true);

/// Sets whether font faces use and keep shaping plans, for the whole process. When `keep` is false,
/// every shaped segment makes a plan of its own, so a test can compare what kept plans produce with
/// what its own plan produces. Plans a face already holds stay and are used again when it is true.
pub fn keep_shape_plans(keep: bool) {
    KEEP_SHAPE_PLANS.store(keep, Ordering::SeqCst);
}

pub(crate) fn keeps_shape_plans() -> bool {
    KEEP_SHAPE_PLANS.load(Ordering::SeqCst)
}

std::thread_local! {
    /// How many layouts this thread has started ([`layouts_on_this_thread`]).
    static LAYOUTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub(crate) fn count_layout() {
    LAYOUTS.with(|n| n.set(n.get() + 1));
}

/// How many layouts this thread has started.
pub fn layouts_on_this_thread() -> u64 {
    LAYOUTS.with(std::cell::Cell::get)
}

/// Drop every layout kept for a first frame, so that a test can draw what laying out again draws.
pub fn drop_kept_layouts() {
    crate::kept::drop_kept_layouts();
}

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
/// a name and one named like another (`BOLD`), which style lists leave out. Its version (name ID 5) is
/// `Version 1.000`. `None` when it can't
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
        (5, "Version 1.000"),
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

/// The family of [`unlisted_variable_font`].
pub const UNLISTED_VARIABLE_FAMILY: &str = "Unlisted Varitest Sans";

/// A variable face with no GSUB: `a.a` is an unencoded alternate named for `a`, but the Glyphs
/// panel has no listed alternates. The weight axis comes from [`variable_font`].
pub fn unlisted_variable_font() -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;

    let bytes = variable_font()?;
    let font = skrifa::FontRef::new(&bytes).ok()?;
    let mappings = [' ', 'a', 'l']
        .into_iter()
        .map(|c| Some((c, write_fonts::types::GlyphId::new(font.charmap().map(c)?.to_u32()))))
        .collect::<Option<Vec<_>>>()?;
    let cmap = Cmap::from_mappings(mappings).ok()?;
    let name = name_table(&[
        (1, UNLISTED_VARIABLE_FAMILY),
        (2, "Regular"),
        (4, "Unlisted Varitest Sans Regular"),
        (5, "Version 1.000"),
        (6, "UnlistedVaritestSans-Regular"),
        (256, "Weight"),
    ])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    builder.add_table(&cmap).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if tag != Tag::new(b"GSUB") && !builder.contains(tag) {
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

/// The family of [`height_font`] with its heights.
pub const HEIGHT_FAMILY: &str = "Heighttest Sans";

/// The family of [`height_font`] without them.
pub const HEIGHTLESS_FAMILY: &str = "Heightlesstest Sans";

/// The cap height and x height [`height_font`] gives, in font units (1,000 to the em).
pub const HEIGHT_CAP: i16 = 700;
pub const HEIGHT_X: i16 = 500;

/// The height (and the width across a vertical line) of [`height_font`]'s ideographic character
/// face, in ems.
pub const HEIGHT_ICF: f64 = 0.9;

/// The family of [`height_outside_font`].
pub const HEIGHT_OUTSIDE_FAMILY: &str = "Heightoutsidetest Sans";

/// The cap height and x height [`height_outside_font`] gives, in font units (1,000 to the em):
/// 2.5 em and 0.05 em, outside the 0.1 to 2 ems a font size measures.
pub const HEIGHT_OUTSIDE_CAP: i16 = 2500;
pub const HEIGHT_OUTSIDE_X: i16 = 50;

/// The bundled Source Sans 3 Regular as family [`HEIGHT_FAMILY`], with the cap height
/// [`HEIGHT_CAP`] and x height [`HEIGHT_X`] in its OS/2 table and a `BASE` table whose Han
/// baselines put the ideographic character face [`HEIGHT_ICF`] em high inside the em box (`ideo`
/// -120, `idtp` 880, `icfb` -70, `icft` 830). Without `heights`: family [`HEIGHTLESS_FAMILY`],
/// with a version 1 OS/2 table, which holds neither height, and no `BASE` table. `None` when it
/// can't be built.
pub fn height_font(heights: bool) -> Option<Vec<u8>> {
    if heights { heights_font(HEIGHT_FAMILY, Some((HEIGHT_X, HEIGHT_CAP)), true) } else { heights_font(HEIGHTLESS_FAMILY, None, false) }
}

/// The bundled Source Sans 3 Regular as family [`HEIGHT_OUTSIDE_FAMILY`], with the cap height
/// [`HEIGHT_OUTSIDE_CAP`] and x height [`HEIGHT_OUTSIDE_X`] in its OS/2 table and no `BASE`
/// table. `None` when it can't be built.
pub fn height_outside_font() -> Option<Vec<u8>> {
    heights_font(HEIGHT_OUTSIDE_FAMILY, Some((HEIGHT_OUTSIDE_X, HEIGHT_OUTSIDE_CAP)), false)
}

/// The bundled Source Sans 3 Regular as `family`: with the x height and cap height `heights` in
/// its OS/2 table, else a version 1 OS/2 table, which holds neither; with `base`, the `BASE`
/// table of [`height_font`].
fn heights_font(family: &str, heights: Option<(i16, i16)>, base: bool) -> Option<Vec<u8>> {
    use write_fonts::tables::base::{Axis, Base, BaseCoord, BaseScript, BaseScriptList, BaseScriptRecord, BaseTagList, BaseValues};

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let full = format!("{family} Regular");
    let postscript = format!("{}-Regular", family.replace(' ', ""));
    let name = name_table(&[(1, family), (2, "Regular"), (4, &full), (6, &postscript)])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    if base {
        let tags = BaseTagList::new([b"icfb", b"icft", b"ideo", b"idtp"].map(Tag::new).to_vec());
        let values = BaseValues::new(2, [-70, 830, -120, 880].map(BaseCoord::format_1).to_vec());
        let hani = BaseScriptRecord::new(Tag::new(b"hani"), BaseScript::new(Some(values), None, vec![]));
        let axis = Axis::new(Some(tags), BaseScriptList::new(vec![hani]));
        builder.add_table(&Base::new(Some(axis), None)).ok()?;
    }
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if builder.contains(tag) {
            continue;
        }
        let mut data = font.table_data(r.tag())?.as_bytes().to_vec();
        if tag == Tag::new(b"OS/2") {
            if let Some((x, cap)) = heights {
                // sxHeight and sCapHeight, at bytes 86 and 88 of a version 2 or later table.
                data.get_mut(86..90)?.copy_from_slice(&[x.to_be_bytes(), cap.to_be_bytes()].concat());
            } else {
                // Version 1 ends before sxHeight.
                data.truncate(86);
                data.get_mut(0..2)?.copy_from_slice(&1_u16.to_be_bytes());
            }
        }
        builder.add_raw(tag, data);
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

/// The family of [`full_width_font`].
pub const FULL_WIDTH_FAMILY: &str = "Fullwidthtest Sans";

/// The ideographs and Japanese punctuation [`full_width_font`] sets one em wide.
pub const FULL_WIDTH_CHARS: &str = "一二三四五六「」、。";

/// A test-only cmap over bundled Source Sans 3: [`FULL_WIDTH_CHARS`] use the one-em-wide glyph of
/// U+2318, so Japanese composition is tested without OS fonts; Latin letters and the space keep
/// their own glyphs.
pub fn full_width_font() -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let wide = write_fonts::types::GlyphId::new(charmap.map('\u{2318}')?.to_u32());
    let latin = ('A'..='Z').chain('a'..='z').chain([' ']);
    let mut mappings: Vec<(char, write_fonts::types::GlyphId)> = FULL_WIDTH_CHARS.chars().map(|c| (c, wide)).collect();
    for c in latin {
        mappings.push((c, write_fonts::types::GlyphId::new(charmap.map(c)?.to_u32())));
    }
    let cmap = Cmap::from_mappings(mappings).ok()?;
    let name = name_table(&[(1, FULL_WIDTH_FAMILY), (2, "Regular"), (4, "Fullwidthtest Sans Regular"), (6, "FullwidthtestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_table(&cmap).ok()?;
    builder.add_raw(Tag::new(b"name"), name);
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

/// The family of [`conjunct_font`].
pub const CONJUNCT_FAMILY: &str = "Conjuncttest Sans";

/// A test-only cmap over bundled Source Sans 3: Devanagari Ka uses K, the halant uses a period and
/// the vowel sign I (drawn before its syllable) uses i. The `GSUB` table's `dev2` script has an
/// `aalt` single substitution of K by k and a `salt` one of K by i, which list k and the vowel
/// sign's glyph as alternates of Ka, and a `half` ligature of K and the period into k (Ka's half
/// form). Family [`CONJUNCT_FAMILY`]. `None` when it can't be built.
pub fn conjunct_font() -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;
    use write_fonts::tables::gsub::{
        Gsub, Ligature, LigatureSet, LigatureSubstFormat1, SingleSubstFormat2, SubstitutionLookup, SubstitutionLookupList,
    };
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let mapping = [('क', 'K'), ('\u{094D}', '.'), ('\u{093F}', 'i')];
    let mappings = mapping.into_iter().map(|(c, source)| Some((c, write_fonts::types::GlyphId::new(charmap.map(source)?.to_u32()))));
    let cmap = Cmap::from_mappings(mappings.collect::<Option<Vec<_>>>()?).ok()?;
    let family = CONJUNCT_FAMILY;
    let name = name_table(&[(1, family), (2, "Regular"), (4, "Conjuncttest Sans Regular"), (6, "ConjuncttestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_table(&cmap).ok()?;
    builder.add_raw(Tag::new(b"name"), name);
    let gid = |c| Some(GlyphId16::new(u16::try_from(charmap.map(c)?.to_u32()).ok()?));
    let ka = gid('K')?;
    let aalt = SingleSubstFormat2::new(CoverageFormat1::new(vec![ka]).into(), vec![gid('k')?]);
    let half =
        LigatureSubstFormat1::new(CoverageFormat1::new(vec![ka]).into(), vec![LigatureSet::new(vec![Ligature::new(gid('k')?, vec![gid('.')?])])]);
    let salt = SingleSubstFormat2::new(CoverageFormat1::new(vec![ka]).into(), vec![gid('i')?]);
    // Features sorted by tag, as OpenType requires: `aalt`, `half`, then `salt`.
    let features = FeatureList::new(vec![
        FeatureRecord::new(Tag::new(b"aalt"), Feature::new(None, vec![0])),
        FeatureRecord::new(Tag::new(b"half"), Feature::new(None, vec![1])),
        FeatureRecord::new(Tag::new(b"salt"), Feature::new(None, vec![2])),
    ]);
    let lookups = SubstitutionLookupList::new(vec![
        SubstitutionLookup::Single(Lookup::new(Default::default(), vec![aalt.into()])),
        SubstitutionLookup::Ligature(Lookup::new(Default::default(), vec![half])),
        SubstitutionLookup::Single(Lookup::new(Default::default(), vec![salt.into()])),
    ]);
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"dev2"), Script::new(Some(LangSys::new(vec![0, 1, 2])), vec![]))]);
    builder.add_table(&Gsub::new(scripts, features, lookups)).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`matra_font`].
pub const MATRA_FAMILY: &str = "Matratest Sans";

/// A test-only cmap over bundled Source Sans 3: Devanagari Ka uses a, the vowel sign I (drawn
/// before its consonant) uses i and the nukta uses the period. The `GSUB` table's `dev2` script has
/// a `nukt` ligature of Ka and the nukta into N, and a `salt` single substitution of Ka by b.
/// Family [`MATRA_FAMILY`]. `None` when it can't be built.
pub fn matra_font() -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;
    use write_fonts::tables::gsub::{
        Gsub, Ligature, LigatureSet, LigatureSubstFormat1, SingleSubstFormat2, SubstitutionLookup, SubstitutionLookupList,
    };
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let mapping = [('क', 'a'), ('\u{093F}', 'i'), ('\u{093C}', '.')];
    let mappings = mapping.into_iter().map(|(c, source)| Some((c, write_fonts::types::GlyphId::new(charmap.map(source)?.to_u32()))));
    let cmap = Cmap::from_mappings(mappings.collect::<Option<Vec<_>>>()?).ok()?;
    let family = MATRA_FAMILY;
    let name = name_table(&[(1, family), (2, "Regular"), (4, "Matratest Sans Regular"), (5, "Version 1.000"), (6, "MatratestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_table(&cmap).ok()?;
    builder.add_raw(Tag::new(b"name"), name);
    let gid = |c| Some(GlyphId16::new(u16::try_from(charmap.map(c)?.to_u32()).ok()?));
    let ka = gid('a')?;
    let ligature = LigatureSet::new(vec![Ligature::new(gid('N')?, vec![gid('.')?])]);
    let nukt = LigatureSubstFormat1::new(CoverageFormat1::new(vec![ka]).into(), vec![ligature]);
    let salt = SingleSubstFormat2::new(CoverageFormat1::new(vec![ka]).into(), vec![gid('b')?]);
    // Features sorted by tag, as OpenType requires: `nukt`, then `salt`.
    let features = FeatureList::new(vec![
        FeatureRecord::new(Tag::new(b"nukt"), Feature::new(None, vec![0])),
        FeatureRecord::new(Tag::new(b"salt"), Feature::new(None, vec![1])),
    ]);
    let lookups = SubstitutionLookupList::new(vec![
        SubstitutionLookup::Ligature(Lookup::new(Default::default(), vec![nukt])),
        SubstitutionLookup::Single(Lookup::new(Default::default(), vec![salt.into()])),
    ]);
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"dev2"), Script::new(Some(LangSys::new(vec![0, 1])), vec![]))]);
    builder.add_table(&Gsub::new(scripts, features, lookups)).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`lam_alef_font`] whose lam-alef ligature is in `feature`.
pub fn lam_alef_family(feature: &[u8; 4]) -> &'static str {
    match feature {
        b"liga" => "Lamtest Liga",
        b"rlig" => "Lamtest Rlig",
        _ => "Lamtest Other",
    }
}

/// The glyph [`lam_alef_font`]'s `salt` gives its lam + alef ligature (H): M.
pub const LAM_ALEF_SALT: char = 'M';

/// A test-only Arabic cmap over bundled Source Sans 3: lam uses l, alef I, alef with hamza above
/// J, beh b and fatha the combining acute (a mark in the font's `GDEF`). The `GSUB` table's `arab`
/// script has one ligature lookup, under `feature`, that skips marks and joins lam + alef into H
/// and lam + alef with hamza above into K, and a `salt` lookup that turns H into
/// [`LAM_ALEF_SALT`]. Older fonts put the mandatory lam-alef ligature in `liga` rather than
/// `rlig`; this builds either, without an OS font. `None` when it can't be built.
pub fn lam_alef_font(feature: &[u8; 4]) -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;
    use write_fonts::tables::gsub::{
        Gsub, Ligature, LigatureSet, LigatureSubstFormat1, SingleSubstFormat2, SubstitutionLookup, SubstitutionLookupList,
    };
    use write_fonts::tables::layout::{
        CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, LookupFlag, Script, ScriptList, ScriptRecord,
    };
    use write_fonts::types::GlyphId16;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let mapping = [('\u{0644}', 'l'), ('\u{0627}', 'I'), ('\u{0623}', 'J'), ('\u{0628}', 'b'), ('\u{064E}', '\u{0301}')];
    let mappings = mapping.into_iter().map(|(c, source)| Some((c, write_fonts::types::GlyphId::new(charmap.map(source)?.to_u32()))));
    let cmap = Cmap::from_mappings(mappings.collect::<Option<Vec<_>>>()?).ok()?;
    let family = lam_alef_family(feature);
    let full = format!("{family} Regular");
    let postscript = format!("{}-Regular", family.replace(' ', ""));
    let name = name_table(&[(1, family), (2, "Regular"), (4, &full), (6, &postscript)])?;
    let mut builder = FontBuilder::new();
    builder.add_table(&cmap).ok()?;
    builder.add_raw(Tag::new(b"name"), name);
    let gid = |c| Some(GlyphId16::new(u16::try_from(charmap.map(c)?.to_u32()).ok()?));
    let coverage = CoverageFormat1::new(vec![gid('l')?]);
    let ligatures = vec![Ligature::new(gid('H')?, vec![gid('I')?]), Ligature::new(gid('K')?, vec![gid('J')?])];
    let subst = LigatureSubstFormat1::new(coverage.into(), vec![LigatureSet::new(ligatures)]);
    let salt = SingleSubstFormat2::new(CoverageFormat1::new(vec![gid('H')?]).into(), vec![gid(LAM_ALEF_SALT)?]);
    // Features sorted by tag, as OpenType requires: `liga` / `rlig`, then `salt`.
    let features = FeatureList::new(vec![
        FeatureRecord::new(Tag::new(feature), Feature::new(None, vec![0])),
        FeatureRecord::new(Tag::new(b"salt"), Feature::new(None, vec![1])),
    ]);
    let lookups = SubstitutionLookupList::new(vec![
        SubstitutionLookup::Ligature(Lookup::new(LookupFlag::IGNORE_MARKS, vec![subst])),
        SubstitutionLookup::Single(Lookup::new(Default::default(), vec![salt.into()])),
    ]);
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"arab"), Script::new(Some(LangSys::new(vec![0, 1])), vec![]))]);
    builder.add_table(&Gsub::new(scripts, features, lookups)).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`alternates_font`] with an `aalt` feature, and without one.
pub fn alternates_family(aalt: bool) -> &'static str {
    if aalt { "Alttest Sans" } else { "Alttest Sans Noaalt" }
}

/// Four glyphs of the bundled Source Sans 3 Regular that its `cmap` maps no character to, the
/// lowest ids first: the unencoded alternates of [`alternates_font`]. `None` when there are not
/// four.
pub fn unencoded_glyphs() -> Option<[u32; 4]> {
    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let mapped: std::collections::HashSet<u32> = font.charmap().mappings().map(|(_, g)| g.to_u32()).collect();
    let glyphs = u32::from(font.maxp().ok()?.num_glyphs());
    let found: Vec<u32> = (1..glyphs).filter(|g| !mapped.contains(g)).take(4).collect();
    found.try_into().ok()
}

/// The family of [`crowded_alternates_font`].
pub const CROWDED_ALTERNATES_FAMILY: &str = "Crowdtest Sans";

/// The bundled Source Sans 3 Regular, its `cmap` kept, with a `GSUB` table whose 17 `cv01` feature
/// records each list a single substitution of `a` by `c` 256 times, followed by a `salt` record
/// whose single substitution gives `b`. Family [`CROWDED_ALTERNATES_FAMILY`]. `None` when it can't
/// be built.
pub fn crowded_alternates_font() -> Option<Vec<u8>> {
    use write_fonts::tables::gsub::{Gsub, SingleSubst, SingleSubstFormat2, SubstitutionLookup, SubstitutionLookupList};
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let family = CROWDED_ALTERNATES_FAMILY;
    let name = name_table(&[(1, family), (2, "Regular"), (4, "Crowdtest Sans Regular"), (5, "Version 1.000"), (6, "CrowdtestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    let gid = |c| Some(GlyphId16::new(u16::try_from(charmap.map(c)?.to_u32()).ok()?));
    let a = gid('a')?;
    let single = |to: GlyphId16| {
        SubstitutionLookup::Single(Lookup::new(
            Default::default(),
            vec![SingleSubst::from(SingleSubstFormat2::new(CoverageFormat1::new(vec![a]).into(), vec![to]))],
        ))
    };
    let lookups = SubstitutionLookupList::new(vec![single(gid('c')?), single(gid('b')?)]);
    let mut records: Vec<FeatureRecord> = (0..17).map(|_| FeatureRecord::new(Tag::new(b"cv01"), Feature::new(None, vec![0; 256]))).collect();
    records.push(FeatureRecord::new(Tag::new(b"salt"), Feature::new(None, vec![1])));
    let indices = (0..records.len() as u16).collect();
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(indices)), vec![]))]);
    builder.add_table(&Gsub::new(scripts, FeatureList::new(records), lookups)).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The bundled Source Sans 3 Regular, its `cmap` kept, with a `GSUB` table of alternates for `a`
/// and nothing else. With [`unencoded_glyphs`] `[u1, u2, u3, u4]`: `aalt` (alternate
/// substitution) lists a, u1 and b; `cv01` (single substitution) gives c; `salt` (single) gives b;
/// `smcp` (single) gives d; `ss01` (single, a glyph-id delta) gives u2; `ss02` (single, in an
/// extension lookup) gives u4; and `swsh` (alternate substitution) lists u3 and u1. Without `aalt`
/// the font has the others only. Family [`alternates_family`]. `None` when it can't be built.
pub fn alternates_font(aalt: bool) -> Option<Vec<u8>> {
    use write_fonts::tables::gsub::{
        AlternateSet, AlternateSubstFormat1, ExtensionSubstFormat1, ExtensionSubtable, Gsub, SingleSubst, SingleSubstFormat1, SingleSubstFormat2,
        SubstitutionLookup, SubstitutionLookupList,
    };
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let family = alternates_family(aalt);
    let full = format!("{family} Regular");
    let postscript = format!("{}-Regular", family.replace(' ', ""));
    let name = name_table(&[(1, family), (2, "Regular"), (4, &full), (5, "Version 1.000"), (6, &postscript)])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    let id = |g: u32| Some(GlyphId16::new(u16::try_from(g).ok()?));
    let gid = |c| id(charmap.map(c)?.to_u32());
    let [u1, u2, u3, u4] = unencoded_glyphs()?.map(id);
    let (u1, u2, u3, u4, a) = (u1?, u2?, u3?, u4?, gid('a')?);
    let cover = || CoverageFormat1::new(vec![a]);
    let single = |to: GlyphId16| SingleSubst::from(SingleSubstFormat2::new(cover().into(), vec![to]));
    let lookup = |subtable: SingleSubst| SubstitutionLookup::Single(Lookup::new(Default::default(), vec![subtable]));
    let alternates = |set: Vec<GlyphId16>| {
        SubstitutionLookup::Alternate(Lookup::new(Default::default(), vec![AlternateSubstFormat1::new(cover().into(), vec![AlternateSet::new(set)])]))
    };
    let delta = i16::try_from(i32::from(u2.to_u16()) - i32::from(a.to_u16())).ok()?;
    let extension = ExtensionSubtable::Single(ExtensionSubstFormat1::new(1, single(u4)));
    // Lookups: 0 aalt, 1 cv01, 2 salt, 3 smcp, 4 ss01, 5 ss02, 6 swsh.
    let lookups = SubstitutionLookupList::new(vec![
        alternates(vec![a, u1, gid('b')?]),
        lookup(single(gid('c')?)),
        lookup(single(gid('b')?)),
        lookup(single(gid('d')?)),
        lookup(SingleSubst::from(SingleSubstFormat1::new(cover().into(), delta))),
        SubstitutionLookup::Extension(Lookup::new(Default::default(), vec![extension])),
        alternates(vec![u3, u1]),
    ]);
    // Features sorted by tag, as OpenType requires.
    let tags: [&[u8; 4]; 7] = [b"aalt", b"cv01", b"salt", b"smcp", b"ss01", b"ss02", b"swsh"];
    let records: Vec<FeatureRecord> = tags
        .iter()
        .zip(0u16..)
        .filter(|(t, _)| aalt || **t != b"aalt")
        .map(|(t, i)| FeatureRecord::new(Tag::new(t), Feature::new(None, vec![i])))
        .collect();
    let indices = (0..records.len() as u16).collect();
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(indices)), vec![]))]);
    builder.add_table(&Gsub::new(scripts, FeatureList::new(records), lookups)).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`capped_candidates_font`].
pub const CAPPED_CANDIDATES_FAMILY: &str = "Captest Sans";

/// The bundled Source Sans 3 Regular, its `cmap` kept, with a `GSUB` table for `a` of an `aalt`
/// alternate substitution that lists `b` 512 times (the longest set read: 512 values to try) and a
/// `salt` single substitution that gives `c`. Family [`CAPPED_CANDIDATES_FAMILY`]. `None` when it
/// can't be built.
pub fn capped_candidates_font() -> Option<Vec<u8>> {
    use write_fonts::tables::gsub::{
        AlternateSet, AlternateSubstFormat1, Gsub, SingleSubst, SingleSubstFormat2, SubstitutionLookup, SubstitutionLookupList,
    };
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let name =
        name_table(&[(1, CAPPED_CANDIDATES_FAMILY), (2, "Regular"), (4, "Captest Sans Regular"), (5, "Version 1.000"), (6, "CaptestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    let gid = |c| Some(GlyphId16::new(u16::try_from(charmap.map(c)?.to_u32()).ok()?));
    let a = gid('a')?;
    let cover = || CoverageFormat1::new(vec![a]);
    let alternates = AlternateSubstFormat1::new(cover().into(), vec![AlternateSet::new(vec![gid('b')?; 512])]);
    let salt = SingleSubst::from(SingleSubstFormat2::new(cover().into(), vec![gid('c')?]));
    let lookups = SubstitutionLookupList::new(vec![
        SubstitutionLookup::Alternate(Lookup::new(Default::default(), vec![alternates])),
        SubstitutionLookup::Single(Lookup::new(Default::default(), vec![salt])),
    ]);
    let records =
        vec![FeatureRecord::new(Tag::new(b"aalt"), Feature::new(None, vec![0])), FeatureRecord::new(Tag::new(b"salt"), Feature::new(None, vec![1]))];
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(vec![0, 1])), vec![]))]);
    builder.add_table(&Gsub::new(scripts, FeatureList::new(records), lookups)).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`joining_alternates_font`].
pub const JOINING_ALTERNATES_FAMILY: &str = "Jointest Sans";

/// Beh, teh, noon and alef: the letters [`joining_alternates_font`] maps.
pub const JOINING_LETTERS: [char; 4] = ['\u{0628}', '\u{062A}', '\u{0646}', '\u{0627}'];

/// A test-only Arabic font over the bundled Source Sans 3, whose glyphs stand for the positional
/// forms of beh, teh, noon and alef ([`JOINING_LETTERS`]), each a Latin letter's outline: isolated
/// b t n I, initial B T N, medial v u m, final V U M A (alef joins to the right only). Its `arab`
/// `GSUB` has `init`, `medi` and `fina` single substitutions from the isolated glyphs, `salt`
/// alternate substitutions of the medial teh u (w, x, y: three alternates), of the final teh U
/// (W, X), of the isolated teh t (E) and of the medial beh v (z), and `ss01` single substitutions
/// u to w, U to W, t to E and v to z. A set differs by position, as in a font that draws each
/// letter's alternates for the position they join in. `cv01` gives the medial noon m the glyph d,
/// an alternate that has no style tag. Family [`JOINING_ALTERNATES_FAMILY`]. `None` when it can't
/// be built.
pub fn joining_alternates_font() -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;
    use write_fonts::tables::gsub::{
        AlternateSet, AlternateSubstFormat1, Gsub, SingleSubst, SingleSubstFormat2, SubstitutionLookup, SubstitutionLookupList,
    };
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    let font = skrifa::FontRef::new(SOURCE_SANS).ok()?;
    let charmap = font.charmap();
    let gid = |c: char| Some(GlyphId16::new(u16::try_from(charmap.map(c)?.to_u32()).ok()?));
    let family = JOINING_ALTERNATES_FAMILY;
    let full = format!("{family} Regular");
    let postscript = format!("{}-Regular", family.replace(' ', ""));
    let name = name_table(&[(1, family), (2, "Regular"), (4, &full), (5, "Version 1.000"), (6, &postscript)])?;
    let mapping = [(JOINING_LETTERS[0], 'b'), (JOINING_LETTERS[1], 't'), (JOINING_LETTERS[2], 'n'), (JOINING_LETTERS[3], 'I'), (' ', ' ')];
    let mappings = mapping.into_iter().map(|(c, source)| Some((c, write_fonts::types::GlyphId::new(charmap.map(source)?.to_u32()))));
    let cmap = Cmap::from_mappings(mappings.collect::<Option<Vec<_>>>()?).ok()?;
    let mut builder = FontBuilder::new();
    builder.add_table(&cmap).ok()?;
    builder.add_raw(Tag::new(b"name"), name);
    // A single substitution by (glyph, replacement) pairs, sorted by glyph id as coverage needs.
    let single = |pairs: &[(char, char)]| {
        let mut ids = pairs.iter().map(|(from, to)| Some((gid(*from)?, gid(*to)?))).collect::<Option<Vec<_>>>()?;
        ids.sort_by_key(|p| p.0.to_u16());
        let from = ids.iter().map(|p| p.0).collect();
        let to = ids.iter().map(|p| p.1).collect();
        Some(SubstitutionLookup::Single(Lookup::new(
            Default::default(),
            vec![SingleSubst::from(SingleSubstFormat2::new(CoverageFormat1::new(from).into(), to))],
        )))
    };
    let alternates = |sets: &[(char, &[char])]| {
        let mut ids = sets
            .iter()
            .map(|(from, set)| Some((gid(*from)?, set.iter().map(|c| gid(*c)).collect::<Option<Vec<_>>>()?)))
            .collect::<Option<Vec<_>>>()?;
        ids.sort_by_key(|p| p.0.to_u16());
        let from = ids.iter().map(|p| p.0).collect();
        let sets = ids.into_iter().map(|p| AlternateSet::new(p.1)).collect();
        Some(SubstitutionLookup::Alternate(Lookup::new(
            Default::default(),
            vec![AlternateSubstFormat1::new(CoverageFormat1::new(from).into(), sets)],
        )))
    };
    // Lookups: 0 init, 1 medi, 2 fina, 3 salt, 4 ss01, 5 cv01.
    let lookups = SubstitutionLookupList::new(vec![
        single(&[('b', 'B'), ('t', 'T'), ('n', 'N')])?,
        single(&[('b', 'v'), ('t', 'u'), ('n', 'm')])?,
        single(&[('b', 'V'), ('t', 'U'), ('n', 'M'), ('I', 'A')])?,
        alternates(&[('u', &['w', 'x', 'y']), ('U', &['W', 'X']), ('t', &['E']), ('v', &['z'])])?,
        single(&[('u', 'w'), ('U', 'W'), ('t', 'E'), ('v', 'z')])?,
        single(&[('m', 'd')])?,
    ]);
    // Features sorted by tag, as OpenType requires: cv01, fina, init, medi, salt, ss01.
    let records: Vec<FeatureRecord> = [(b"cv01", 5), (b"fina", 2), (b"init", 0), (b"medi", 1), (b"salt", 3), (b"ss01", 4)]
        .into_iter()
        .map(|(tag, lookup)| FeatureRecord::new(Tag::new(tag), Feature::new(None, vec![lookup])))
        .collect();
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"arab"), Script::new(Some(LangSys::new(vec![0, 1, 2, 3, 4, 5])), vec![]))]);
    builder.add_table(&Gsub::new(scripts, FeatureList::new(records), lookups)).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`aliased_joining_font`].
pub const ALIASED_JOINING_FAMILY: &str = "Alias Jointest Sans";

/// [`joining_alternates_font`] with Latin `B` and the Arabic initial beh presentation form
/// U+FE91 both mapped to beh's initial glyph. Only the presentation form should insert beh.
pub fn aliased_joining_font() -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;

    let bytes = joining_alternates_font()?;
    let font = skrifa::FontRef::new(&bytes).ok()?;
    let source = skrifa::FontRef::new(SOURCE_SANS).ok()?;
    let initial = write_fonts::types::GlyphId::new(source.charmap().map('B')?.to_u32());
    let mut mappings =
        font.charmap().mappings().filter_map(|(c, g)| Some((char::from_u32(c)?, write_fonts::types::GlyphId::new(g.to_u32())))).collect::<Vec<_>>();
    mappings.extend([('B', initial), ('\u{FE91}', initial)]);
    let cmap = Cmap::from_mappings(mappings).ok()?;
    let name = name_table(&[
        (1, ALIASED_JOINING_FAMILY),
        (2, "Regular"),
        (4, "Alias Jointest Sans Regular"),
        (5, "Version 1.000"),
        (6, "AliasJointestSans-Regular"),
    ])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    builder.add_table(&cmap).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family and private-use character of [`contextual_fallback_font`].
pub const CONTEXTUAL_FALLBACK_FAMILY: &str = "Contextual Fallbacktest Sans";
pub const CONTEXTUAL_FALLBACK_CHAR: char = '\u{F6084}';

/// A fallback fixture with coverage no bundled face shares: `ccmp` draws its private-use
/// character (n) as m, whose `cv01` alternate d has no applicable character-style feature.
pub fn contextual_fallback_font() -> Option<Vec<u8>> {
    use write_fonts::tables::gsub::{Gsub, SingleSubst, SingleSubstFormat2, SubstitutionLookup, SubstitutionLookupList};
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    let base = remapped_source_sans(
        CONTEXTUAL_FALLBACK_FAMILY,
        "ContextualFallbacktestSans-Regular",
        &[(CONTEXTUAL_FALLBACK_CHAR, 'n'), ('a', 'a'), (' ', ' ')],
    )?;
    let font = skrifa::FontRef::new(&base).ok()?;
    let [n, m, d] = source_sans_gids(['n', 'm', 'd'])?;
    let single = |from, to| {
        SubstitutionLookup::Single(Lookup::new(
            Default::default(),
            vec![SingleSubst::from(SingleSubstFormat2::new(CoverageFormat1::new(vec![GlyphId16::new(from)]).into(), vec![GlyphId16::new(to)]))],
        ))
    };
    let lookups = SubstitutionLookupList::new(vec![single(n, m), single(m, d)]);
    let features = FeatureList::new(vec![
        FeatureRecord::new(Tag::new(b"ccmp"), Feature::new(None, vec![0])),
        FeatureRecord::new(Tag::new(b"cv01"), Feature::new(None, vec![1])),
    ]);
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(vec![0, 1])), vec![]))]);
    let mut builder = FontBuilder::new();
    builder.add_table(&Gsub::new(scripts, features, lookups)).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`ligature_caret_font`].
pub const LIGATURE_CARET_FAMILY: &str = "Carettest Sans";

/// The glyph [`ligature_caret_font`]'s `liga` gives for f + i: W, wider than either letter.
pub const LIGATURE_CARET_GLYPH: char = 'W';

/// A test-only cmap over bundled Source Sans 3 for f, i and the space, whose `GSUB` `liga` joins f +
/// i into [`LIGATURE_CARET_GLYPH`]. With `caret`, a `GDEF` ligature caret list puts that glyph's one
/// caret at x = `caret` font units; without it the font has no `GDEF` table. `None` when it can't be
/// built.
pub fn ligature_caret_font(caret: Option<i16>) -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;
    use write_fonts::tables::gdef::{CaretValue, Gdef, LigCaretList, LigGlyph};
    use write_fonts::tables::gsub::{Gsub, Ligature, LigatureSet, LigatureSubstFormat1, SubstitutionLookup, SubstitutionLookupList};
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let mapping = [('f', 'f'), ('i', 'i'), (' ', ' ')];
    let mappings = mapping.into_iter().map(|(c, source)| Some((c, write_fonts::types::GlyphId::new(charmap.map(source)?.to_u32()))));
    let cmap = Cmap::from_mappings(mappings.collect::<Option<Vec<_>>>()?).ok()?;
    let family = LIGATURE_CARET_FAMILY;
    let name = name_table(&[(1, family), (2, "Regular"), (4, "Carettest Sans Regular"), (6, "CarettestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_table(&cmap).ok()?;
    builder.add_raw(Tag::new(b"name"), name);
    let gid = |c| Some(GlyphId16::new(u16::try_from(charmap.map(c)?.to_u32()).ok()?));
    let ligature = gid(LIGATURE_CARET_GLYPH)?;
    let subst = LigatureSubstFormat1::new(
        CoverageFormat1::new(vec![gid('f')?]).into(),
        vec![LigatureSet::new(vec![Ligature::new(ligature, vec![gid('i')?])])],
    );
    let features = FeatureList::new(vec![FeatureRecord::new(Tag::new(b"liga"), Feature::new(None, vec![0]))]);
    let lookups = SubstitutionLookupList::new(vec![SubstitutionLookup::Ligature(Lookup::new(Default::default(), vec![subst]))]);
    let langsys = || Script::new(Some(LangSys::new(vec![0])), vec![]);
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), langsys()), ScriptRecord::new(Tag::new(b"latn"), langsys())]);
    builder.add_table(&Gsub::new(scripts, features, lookups)).ok()?;
    if let Some(x) = caret {
        let carets = LigCaretList::new(CoverageFormat1::new(vec![ligature]).into(), vec![LigGlyph::new(vec![CaretValue::format_1(x)])]);
        builder.add_table(&Gdef::new(None, None, Some(carets), None)).ok()?;
    }
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if tag != Tag::new(b"GDEF") && !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`vertical_ligature_caret_font`].
pub const VERTICAL_LIGATURE_CARET_FAMILY: &str = "Vertical Carettest Sans";

/// [`ligature_caret_font`] with its one caret at y = `caret` font units, as family
/// [`VERTICAL_LIGATURE_CARET_FAMILY`], with vertical metrics (`vhea`, `vmtx` and `VORG`): every glyph
/// one em down the column with its vertical origin at 0.88 em. `None` when it can't be built.
pub fn vertical_ligature_caret_font(caret: i16) -> Option<Vec<u8>> {
    let base = ligature_caret_font(Some(caret))?;
    let font = skrifa::FontRef::new(&base).ok()?;
    let n = font.maxp().ok()?.num_glyphs();
    let be16 = |v: &mut Vec<u8>, x: i16| v.extend_from_slice(&x.to_be_bytes());
    // vmtx: (advance height, top side bearing) for every glyph; VORG gives the origin.
    let mut vmtx = Vec::with_capacity(4 * usize::from(n));
    for _ in 0..n {
        vmtx.extend_from_slice(&1000_u16.to_be_bytes());
        be16(&mut vmtx, 0);
    }
    // vhea 1.1: ascent/descent of the em box across the column, then the metrics' count.
    let mut vhea = Vec::with_capacity(36);
    vhea.extend_from_slice(&0x0001_1000_u32.to_be_bytes());
    for x in [500, -500, 0] {
        be16(&mut vhea, x);
    }
    vhea.extend_from_slice(&1000_u16.to_be_bytes());
    for x in [0, 0, 1000, 1, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut vhea, x);
    }
    vhea.extend_from_slice(&n.to_be_bytes());
    // VORG 1.0: the default origin and no glyph that differs.
    let mut vorg = Vec::new();
    for x in [1, 0, 880, 0] {
        be16(&mut vorg, x);
    }
    let family = VERTICAL_LIGATURE_CARET_FAMILY;
    let name = name_table(&[(1, family), (2, "Regular"), (4, "Vertical Carettest Sans Regular"), (6, "VerticalCarettestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    builder.add_raw(Tag::new(b"vhea"), vhea);
    builder.add_raw(Tag::new(b"vmtx"), vmtx);
    builder.add_raw(Tag::new(b"VORG"), vorg);
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// [`vertical_ligature_caret_font`] with its one caret at y = `caret` font units, f + i joined by
/// `ccmp` (which shaping applies top to bottom too, unlike `liga`), and its `GPOS` replaced by a
/// `vpal` single adjustment that changes the ligature glyph's advance down the column by
/// `y_advance` font units. `None` when it can't be built.
pub fn proportional_vertical_ligature_caret_font(caret: i16, y_advance: i16) -> Option<Vec<u8>> {
    use write_fonts::tables::gpos::{Gpos, PositionLookup, PositionLookupList, SinglePos, ValueRecord};
    use write_fonts::tables::gsub::{Gsub, Ligature, LigatureSet, LigatureSubstFormat1, SubstitutionLookup, SubstitutionLookupList};
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    let base = vertical_ligature_caret_font(caret)?;
    let font = skrifa::FontRef::new(&base).ok()?;
    let gid = |c| Some(GlyphId16::new(source_sans_gid(c)?));
    let ligature = gid(LIGATURE_CARET_GLYPH)?;
    let langsys = || Script::new(Some(LangSys::new(vec![0])), vec![]);
    let scripts = || ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), langsys()), ScriptRecord::new(Tag::new(b"latn"), langsys())]);
    let subst = LigatureSubstFormat1::new(
        CoverageFormat1::new(vec![gid('f')?]).into(),
        vec![LigatureSet::new(vec![Ligature::new(ligature, vec![gid('i')?])])],
    );
    let features = FeatureList::new(vec![FeatureRecord::new(Tag::new(b"ccmp"), Feature::new(None, vec![0]))]);
    let lookups = SubstitutionLookupList::new(vec![SubstitutionLookup::Ligature(Lookup::new(Default::default(), vec![subst]))]);
    let gsub = Gsub::new(scripts(), features, lookups);
    let pos = SinglePos::format_1(CoverageFormat1::new(vec![ligature]).into(), ValueRecord::new().with_y_advance(y_advance));
    let features = FeatureList::new(vec![FeatureRecord::new(Tag::new(b"vpal"), Feature::new(None, vec![0]))]);
    let lookups = PositionLookupList::new(vec![PositionLookup::Single(Lookup::new(Default::default(), vec![pos]))]);
    let mut builder = FontBuilder::new();
    builder.add_table(&gsub).ok()?;
    builder.add_table(&Gpos::new(scripts(), features, lookups)).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`variable_ligature_caret_font`].
pub const VARIABLE_LIGATURE_CARET_FAMILY: &str = "Varicaret Sans";

/// [`ligature_caret_font`] as a variable font of family [`VARIABLE_LIGATURE_CARET_FAMILY`]: one
/// weight axis (400–700, default 400) with the named instances Regular (400) and Bold (700), and a
/// `GDEF` ligature caret list whose one caret is a format 3 value at x = `caret` font units with a
/// variation index into the `GDEF` item variation store that moves it by `delta` at 700. `None`
/// when it can't be built.
pub fn variable_ligature_caret_font(caret: i16, delta: i16) -> Option<Vec<u8>> {
    use write_fonts::tables::gdef::{CaretValue, Gdef, LigCaretList, LigGlyph};
    use write_fonts::tables::layout::{CoverageFormat1, DeviceOrVariationIndex};
    use write_fonts::tables::variations::{ItemVariationData, ItemVariationStore, RegionAxisCoordinates, VariationRegion, VariationRegionList};
    use write_fonts::types::GlyphId16;

    let base = ligature_caret_font(None)?;
    let font = skrifa::FontRef::new(&base).ok()?;
    let ligature = GlyphId16::new(source_sans_gid(LIGATURE_CARET_GLYPH)?);
    let value = CaretValue::format_3(caret, DeviceOrVariationIndex::variation_index(0, 0));
    let carets = LigCaretList::new(CoverageFormat1::new(vec![ligature]).into(), vec![LigGlyph::new(vec![value])]);
    let mut gdef = Gdef::new(None, None, Some(carets), None);
    let region = VariationRegion::new(vec![RegionAxisCoordinates::new(F2Dot14::from_f32(0.0), F2Dot14::from_f32(1.0), F2Dot14::from_f32(1.0))]);
    // One delta set of one 16-bit delta for the one region.
    let data = ItemVariationData::new(1, 1, vec![0], delta.to_be_bytes().to_vec());
    gdef.item_var_store = Some(ItemVariationStore::new(VariationRegionList::new(1, vec![region]), vec![Some(data)])).into();
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
    let fvar = Fvar::new(AxisInstanceArrays::new(vec![axis], vec![instance(257, 400), instance(258, 700)]));
    let family = VARIABLE_LIGATURE_CARET_FAMILY;
    let name = name_table(&[
        (1, family),
        (2, "Regular"),
        (4, "Varicaret Sans Regular"),
        (6, "VaricaretSans-Regular"),
        (256, "Weight"),
        (257, "Regular"),
        (258, "Bold"),
    ])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    builder.add_table(&gdef).ok()?.add_table(&fvar).ok()?;
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// [`ligature_caret_font`] with its one caret at x = `caret` font units and a `GPOS` `kern` single
/// adjustment that places the ligature glyph `x` font units right of its pen position. `None` when
/// it can't be built.
pub fn placed_ligature_caret_font(caret: i16, x: i16) -> Option<Vec<u8>> {
    use write_fonts::tables::gpos::{Gpos, PositionLookup, PositionLookupList, SinglePos, ValueRecord};
    use write_fonts::tables::layout::{CoverageFormat1, Feature, FeatureList, FeatureRecord, LangSys, Lookup, Script, ScriptList, ScriptRecord};
    use write_fonts::types::GlyphId16;

    let base = ligature_caret_font(Some(caret))?;
    let font = skrifa::FontRef::new(&base).ok()?;
    let ligature = GlyphId16::new(source_sans_gid(LIGATURE_CARET_GLYPH)?);
    let pos = SinglePos::format_1(CoverageFormat1::new(vec![ligature]).into(), ValueRecord::new().with_x_placement(x));
    let features = FeatureList::new(vec![FeatureRecord::new(Tag::new(b"kern"), Feature::new(None, vec![0]))]);
    let lookups = PositionLookupList::new(vec![PositionLookup::Single(Lookup::new(Default::default(), vec![pos]))]);
    let langsys = || Script::new(Some(LangSys::new(vec![0])), vec![]);
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), langsys()), ScriptRecord::new(Tag::new(b"latn"), langsys())]);
    let mut builder = FontBuilder::new();
    builder.add_table(&Gpos::new(scripts, features, lookups)).ok()?;
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

/// The family of [`no_pair_kerning_font`].
pub const NO_PAIR_KERNING_FAMILY: &str = "No Pairtest Sans";

/// A font with the bundled Source Sans 3 outlines and no `GPOS` or `kern` table.
pub fn no_pair_kerning_font() -> Option<Vec<u8>> {
    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let family = NO_PAIR_KERNING_FAMILY;
    let name = name_table(&[(1, family), (2, "Regular"), (4, "No Pairtest Sans Regular"), (6, "NoPairtestSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !matches!(tag, t if t == Tag::new(b"name") || t == Tag::new(b"GPOS") || t == Tag::new(b"kern")) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`right_to_left_optical_font`].
pub const RIGHT_TO_LEFT_OPTICAL_FAMILY: &str = "Right-to-left Profile Sans";

/// Two Hebrew characters mapped to the bundled Source Sans 3 A and V outlines.
pub fn right_to_left_optical_font() -> Option<Vec<u8>> {
    remapped_source_sans(RIGHT_TO_LEFT_OPTICAL_FAMILY, "RtlProfileSans-Regular", &[('א', 'A'), ('ב', 'V')])
}

/// The family of [`ideograph_optical_font`].
pub const IDEOGRAPH_OPTICAL_FAMILY: &str = "Ideograph Profile Sans";

/// Two ideographs, two kana and a fullwidth letter mapped to the bundled Source Sans 3 A, V, T, o
/// and W outlines: shapes Optical would space, on characters that keep the font's spacing.
pub fn ideograph_optical_font() -> Option<Vec<u8>> {
    remapped_source_sans(IDEOGRAPH_OPTICAL_FAMILY, "IdeographProfileSans-Regular", &[('日', 'A'), ('本', 'V'), ('カ', 'T'), ('な', 'o'), ('Ｗ', 'W')])
}

/// A face named `family` that maps only the characters of `mapping` to the bundled Source Sans 3
/// outlines of their letters, with no `GPOS` or `kern` table.
fn remapped_source_sans(family: &str, postscript: &str, mapping: &[(char, char)]) -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let mappings = mapping.iter().map(|&(c, source)| Some((c, write_fonts::types::GlyphId::new(charmap.map(source)?.to_u32()))));
    let cmap = Cmap::from_mappings(mappings.collect::<Option<Vec<_>>>()?).ok()?;
    let full = format!("{family} Regular");
    let name = name_table(&[(1, family), (2, "Regular"), (4, full.as_str()), (6, postscript)])?;
    let mut builder = FontBuilder::new();
    builder.add_table(&cmap).ok()?;
    builder.add_raw(Tag::new(b"name"), name);
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if tag != Tag::new(b"GPOS") && tag != Tag::new(b"kern") && !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`tall_ideograph_font`].
pub const TALL_IDEOGRAPH_FAMILY: &str = "Tall Ideograph Sans";

/// A face that maps the ideograph 编 to the bundled Source Sans 3 `H` outline, which stands on the
/// baseline, with a taller ascent (1,058 units per 1,000) and descent (262) than Source Sans 3 in
/// its `hhea` and `OS/2` tables and no line gap: the vertical proportions of a CJK system face.
pub fn tall_ideograph_font() -> Option<Vec<u8>> {
    const ASCENT: i16 = 1058;
    const DESCENT: i16 = 262;
    let mut data = remapped_source_sans(TALL_IDEOGRAPH_FAMILY, "TallIdeographSans-Regular", &[('编', 'H')])?;
    let tables: Vec<(Tag, usize)> = {
        let font = skrifa::FontRef::new(&data).ok()?;
        font.table_directory.table_records().iter().map(|r| (Tag::new(&r.tag().to_be_bytes()), r.offset() as usize)).collect()
    };
    let (ascent, descent, gap) = (ASCENT.to_be_bytes(), (-DESCENT).to_be_bytes(), [0, 0]);
    for (tag, at) in tables {
        if tag == Tag::new(b"hhea") {
            // ascender, descender and lineGap at bytes 4 to 10.
            data.get_mut(at + 4..at + 10)?.copy_from_slice(&[ascent, descent, gap].concat());
        } else if tag == Tag::new(b"OS/2") {
            // sTypoAscender, sTypoDescender, sTypoLineGap, usWinAscent and usWinDescent at bytes 68 to 78.
            let win = [ASCENT.unsigned_abs().to_be_bytes(), DESCENT.unsigned_abs().to_be_bytes()];
            data.get_mut(at + 68..at + 78)?.copy_from_slice(&[ascent, descent, gap, win[0], win[1]].concat());
        }
    }
    Some(data)
}

/// A family of [`path_align_font`]: OS/2 cap height 660, H and O.
pub const PATH_ALIGN_ROMAN_FAMILY: &str = "Pathalign Roman Sans";
/// A family of [`path_align_font`]: no cap height, no H or O, and a `head` box above the baseline.
pub const PATH_ALIGN_BARE_FAMILY: &str = "Pathalign Bare Sans";
/// A family of [`path_align_font`]: a `BASE` table with `ideo` at -150 and no `idtp`.
pub const PATH_ALIGN_BASE_FAMILY: &str = "Pathalign Base Sans";
/// A family of [`path_align_font`]: Japanese code page, 永 on the ⌠ outline (ink -51 to 949).
pub const PATH_ALIGN_IDEOGRAPH_FAMILY: &str = "Pathalign Ideograph Sans";
/// A family of [`path_align_font`]: as [`PATH_ALIGN_IDEOGRAPH_FAMILY`], with ┼ on the | outline
/// (ink -250 to 750, one em).
pub const PATH_ALIGN_CROSS_FAMILY: &str = "Pathalign Cross Sans";
/// A family of [`path_align_font`]: Japanese code page, 永 on the H outline (ink 0 to 656), and a
/// typographic box from -100 to 900.
pub const PATH_ALIGN_TYPO_FAMILY: &str = "Pathalign Typo Sans";
/// A family of [`path_align_font`]: as [`PATH_ALIGN_TYPO_FAMILY`], with a typographic box of 0
/// to 0 and an `hhea` box from -100 to 900.
pub const PATH_ALIGN_HHEA_FAMILY: &str = "Pathalign Hhea Sans";
/// A family of [`path_align_font`]: as [`PATH_ALIGN_TYPO_FAMILY`], as a variable font with one
/// weight axis (400–700, named instances Regular and Bold) whose `MVAR` table raises the
/// typographic ascender and descender by 100 at 700: a box from 0 to 1,000 in Bold.
pub const PATH_ALIGN_VARIABLE_FAMILY: &str = "Pathalign Variable Sans";
/// A family of [`path_align_font`]: Japanese code page, with no reference ideograph and no ┼.
pub const PATH_ALIGN_NONE_FAMILY: &str = "Pathalign None Sans";
/// A family of [`path_align_font`]: no cap height, and H and O swapped (H top 668, O top 656).
pub const PATH_ALIGN_LOWER_FAMILY: &str = "Pathalign Lower Sans";
/// A family of [`path_align_font`]: OS/2 cap height 660, O and no H, and a `BASE` table as
/// [`PATH_ALIGN_BASE_FAMILY`]'s.
pub const PATH_ALIGN_CAPITAL_FAMILY: &str = "Pathalign Capital Sans";
/// A family of [`path_align_font`]: as [`PATH_ALIGN_IDEOGRAPH_FAMILY`], with a typographic box
/// from -101 to 1,002: margins of 53 and 50 round 永's ink.
pub const PATH_ALIGN_MARGINS_FAMILY: &str = "Pathalign Margins Sans";
/// A family of [`path_align_font`]: as [`PATH_ALIGN_IDEOGRAPH_FAMILY`], with a typographic box
/// from -101 to 1,003: margins of 54 and 50 round 永's ink.
pub const PATH_ALIGN_UNEVEN_FAMILY: &str = "Pathalign Uneven Sans";
/// A family of [`path_align_font`]: a `BASE` table with `ideo` at -150 and `romn` below it at
/// -200.
pub const PATH_ALIGN_DAMAGED_FAMILY: &str = "Pathalign Damaged Sans";

/// The bundled Source Sans 3 Regular (1,000 units to the em, typographic box -326 to 1,000, `head`
/// box bottom -295, H top 656, O top 668) without its `BASE` table, as one of the
/// `PATH_ALIGN_*_FAMILY` faces, which map x, and H and O except where the family says otherwise.
/// `None` when it can't be built.
pub fn path_align_font(family: &str) -> Option<Vec<u8>> {
    use write_fonts::tables::base::{Axis, Base, BaseCoord, BaseScript, BaseScriptList, BaseScriptRecord, BaseTagList, BaseValues};
    use write_fonts::tables::mvar::{Mvar, ValueRecord};
    use write_fonts::tables::variations::{ItemVariationData, ItemVariationStore, RegionAxisCoordinates, VariationRegion, VariationRegionList};
    use write_fonts::types::MajorMinor;

    let typo = family == PATH_ALIGN_TYPO_FAMILY || family == PATH_ALIGN_VARIABLE_FAMILY;
    let ideograph = [PATH_ALIGN_IDEOGRAPH_FAMILY, PATH_ALIGN_CROSS_FAMILY, PATH_ALIGN_MARGINS_FAMILY, PATH_ALIGN_UNEVEN_FAMILY].contains(&family);
    let japanese = typo || ideograph || [PATH_ALIGN_HHEA_FAMILY, PATH_ALIGN_NONE_FAMILY].contains(&family);
    let mut mapping = vec![('x', 'x')];
    match family {
        PATH_ALIGN_BARE_FAMILY => {}
        PATH_ALIGN_LOWER_FAMILY => mapping.extend([('H', 'O'), ('O', 'H')]),
        PATH_ALIGN_CAPITAL_FAMILY => mapping.push(('O', 'O')),
        _ => mapping.extend([('H', 'H'), ('O', 'O')]),
    }
    if ideograph {
        mapping.push(('永', '⌠'));
    }
    if family == PATH_ALIGN_CROSS_FAMILY {
        mapping.push(('┼', '|'));
    }
    if typo || family == PATH_ALIGN_HHEA_FAMILY {
        mapping.push(('永', 'H'));
    }
    let mut data = remapped_source_sans(family, &format!("{}-Regular", family.replace(' ', "")), &mapping)?;
    let tables: Vec<(Tag, usize)> = {
        let font = skrifa::FontRef::new(&data).ok()?;
        font.table_directory.table_records().iter().map(|r| (Tag::new(&r.tag().to_be_bytes()), r.offset() as usize)).collect()
    };
    for (tag, at) in tables {
        if tag == Tag::new(b"OS/2") {
            if japanese {
                // ulCodePageRange1 at bytes 78 to 82: bit 17, JIS/Japan.
                data.get_mut(at + 78..at + 82)?.copy_from_slice(&(1_u32 << 17).to_be_bytes());
            }
            if typo {
                // sTypoAscender and sTypoDescender at bytes 68 to 72.
                data.get_mut(at + 68..at + 72)?.copy_from_slice(&[900_i16.to_be_bytes(), (-100_i16).to_be_bytes()].concat());
            }
            if family == PATH_ALIGN_HHEA_FAMILY {
                data.get_mut(at + 68..at + 72)?.copy_from_slice(&[0, 0, 0, 0]);
            }
            if family == PATH_ALIGN_MARGINS_FAMILY || family == PATH_ALIGN_UNEVEN_FAMILY {
                let top: i16 = if family == PATH_ALIGN_MARGINS_FAMILY { 1002 } else { 1003 };
                data.get_mut(at + 68..at + 72)?.copy_from_slice(&[top.to_be_bytes(), (-101_i16).to_be_bytes()].concat());
            }
            if family == PATH_ALIGN_BARE_FAMILY || family == PATH_ALIGN_LOWER_FAMILY {
                // sCapHeight at bytes 88 to 90.
                data.get_mut(at + 88..at + 90)?.copy_from_slice(&0_i16.to_be_bytes());
            }
        } else if tag == Tag::new(b"hhea") && family == PATH_ALIGN_HHEA_FAMILY {
            // ascender and descender at bytes 4 to 8.
            data.get_mut(at + 4..at + 8)?.copy_from_slice(&[900_i16.to_be_bytes(), (-100_i16).to_be_bytes()].concat());
        } else if tag == Tag::new(b"head") && family == PATH_ALIGN_BARE_FAMILY {
            // yMin at bytes 38 to 40.
            data.get_mut(at + 38..at + 40)?.copy_from_slice(&10_i16.to_be_bytes());
        }
    }
    // Source Sans 3's own `BASE` table (`ideo` -170) is left out.
    let font = skrifa::FontRef::new(&data).ok()?;
    let mut builder = FontBuilder::new();
    if [PATH_ALIGN_BASE_FAMILY, PATH_ALIGN_CAPITAL_FAMILY, PATH_ALIGN_DAMAGED_FAMILY].contains(&family) {
        let romn = if family == PATH_ALIGN_DAMAGED_FAMILY { -200 } else { 0 };
        let tags = BaseTagList::new([b"ideo", b"romn"].map(Tag::new).to_vec());
        let values = BaseValues::new(1, [-150, romn].map(BaseCoord::format_1).to_vec());
        let latn = BaseScriptRecord::new(Tag::new(b"latn"), BaseScript::new(Some(values), None, vec![]));
        let axis = Axis::new(Some(tags), BaseScriptList::new(vec![latn]));
        builder.add_table(&Base::new(Some(axis), None)).ok()?;
    }
    if family == PATH_ALIGN_VARIABLE_FAMILY {
        let region = VariationRegion::new(vec![RegionAxisCoordinates::new(F2Dot14::from_f32(0.0), F2Dot14::from_f32(1.0), F2Dot14::from_f32(1.0))]);
        // Two delta sets (hasc, hdsc) of one 16-bit delta each for the one region.
        let data = ItemVariationData::new(2, 1, vec![0], [100_i16.to_be_bytes(), 100_i16.to_be_bytes()].concat());
        let store = ItemVariationStore::new(VariationRegionList::new(1, vec![region]), vec![Some(data)]);
        let records = vec![ValueRecord::new(Tag::new(b"hasc"), 0, 0), ValueRecord::new(Tag::new(b"hdsc"), 0, 1)];
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
        let fvar = Fvar::new(AxisInstanceArrays::new(vec![axis], vec![instance(257, 400), instance(258, 700)]));
        let name = name_table(&[
            (1, family),
            (2, "Regular"),
            (4, "Pathalign Variable Sans Regular"),
            (6, "PathalignVariableSans-Regular"),
            (256, "Weight"),
            (257, "Regular"),
            (258, "Bold"),
        ])?;
        builder.add_raw(Tag::new(b"name"), name);
        builder.add_table(&fvar).ok()?.add_table(&Mvar::new(MajorMinor::VERSION_1_0, Some(store), records)).ok()?;
    }
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) && tag != Tag::new(b"BASE") {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

/// The family of [`soft_hyphen_outline_font`].
pub const SOFT_HYPHEN_OUTLINE_FAMILY: &str = "Soft Hyphen Profile Sans";

/// A face that maps the invisible soft hyphen to the bundled hyphen outline.
pub fn soft_hyphen_outline_font() -> Option<Vec<u8>> {
    use write_fonts::tables::cmap::Cmap;

    const BASE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(BASE).ok()?;
    let charmap = font.charmap();
    let mappings = [('A', 'A'), ('V', 'V'), ('-', '-'), ('\u{00AD}', '-')]
        .into_iter()
        .map(|(c, source)| Some((c, write_fonts::types::GlyphId::new(charmap.map(source)?.to_u32()))));
    let cmap = Cmap::from_mappings(mappings.collect::<Option<Vec<_>>>()?).ok()?;
    let family = SOFT_HYPHEN_OUTLINE_FAMILY;
    let name = name_table(&[(1, family), (2, "Regular"), (4, "Soft Hyphen Profile Sans Regular"), (6, "SoftHyphenProfileSans-Regular")])?;
    let mut builder = FontBuilder::new();
    builder.add_table(&cmap).ok()?;
    builder.add_raw(Tag::new(b"name"), name);
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if tag != Tag::new(b"GPOS") && tag != Tag::new(b"kern") && !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag())?.as_bytes());
        }
    }
    Some(builder.build())
}

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

/// The names of a [`named_copy`]: family, style, version string and PostScript name.
pub type CopyNames<'a> = (&'a str, &'a str, &'a str, &'a str);

/// The bundled Source Sans 3 Regular or, `inter`, the bundled Inter Regular (glyphs of other
/// widths) under `names`, as an installed copy of a font: a font library can keep copies whose
/// names differ only in case or punctuation, or whose family and style are one but whose PostScript
/// names or versions differ. `None` when it can't be built.
pub fn named_copy(inter: bool, (family, style, version, postscript): CopyNames<'_>) -> Option<Vec<u8>> {
    const SOURCE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
    let font = skrifa::FontRef::new(if inter { INTER } else { SOURCE }).ok()?;
    let full = format!("{family} {style}");
    let name = name_table(&[(1, family), (2, style), (4, &full), (5, version), (6, postscript)])?;
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

/// The family of [`numbered_font`]: styles numbered W3 and W6, and no Regular.
pub const NUMBERED_FAMILY: &str = "Numtest Mincho";

/// A face of family [`NUMBERED_FAMILY`] in `style` (`W3` or `W6`; the bundled Source Sans 3
/// Regular renamed), as families whose styles are numbered have, with no Regular among them.
/// `None` when it can't be built.
pub fn numbered_font(style: &str) -> Option<Vec<u8>> {
    const FACE: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");
    let font = skrifa::FontRef::new(FACE).ok()?;
    let full = format!("{NUMBERED_FAMILY} {style}");
    let postscript = format!("NumtestMincho-{style}");
    let name = name_table(&[(1, NUMBERED_FAMILY), (2, style), (4, &full), (5, "Version 1.000"), (6, &postscript)])?;
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
