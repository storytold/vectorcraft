use super::build::base_values;
use super::names::{localized, name};
use super::traits::{FaceKeys, FontTraits};
use super::{Arc, GlyphId, HashMap, Location, LocationRef, MetadataProvider, Size, StringId};

/// The family used when a requested family is unknown (Illustrator's Myriad Pro analogue).
pub const FALLBACK_FAMILY: &str = "Source Sans 3";

/// The fonts every database starts with (feature `bundled-fonts`, on by default).
#[cfg(feature = "bundled-fonts")]
pub(super) static BUNDLED: &[&[u8]] = &[
    include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSans3-Semibold.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSans3-Bold.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSans3-It.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSerif4-Regular.ttf"),
    include_bytes!("../../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf"),
];

/// Without `bundled-fonts` a database starts with none: the embedder adds its fonts
/// ([`FontDb::add_font`]); until then text has no glyphs.
#[cfg(not(feature = "bundled-fonts"))]
pub(super) static BUNDLED: &[&[u8]] = &[];

/// An axis setting of a variable font: the axis tag (`wght`) and its value in user units (700).
pub type Variation = ([u8; 4], f32);

/// Caps on what a (possibly damaged) variable font can make the database read: axes per font and
/// named instances listed per face.
pub(super) const MAX_AXES: usize = 64;

pub(super) const MAX_INSTANCES: usize = 512;

/// One axis of a variable font's design space, as its `fvar` table lists it: the tag (`wght`), its
/// name in the font's name table (`Weight`; the tag when it has none) and its range and default,
/// in the axis's own units.
#[derive(Clone, Debug, PartialEq)]
pub struct VariationAxis {
    pub tag: [u8; 4],
    pub name: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
    /// The font asks for the axis not to be shown to users (the `fvar` hidden flag).
    pub hidden: bool,
}

/// Faces of variable fonts instanced at axis settings their named instances don't have
/// ([`FontDb::instance`]). Each setting of a face keeps its id for the life of the process, so a
/// laid-out glyph's face can be found again ([`FontDb::face_by_id`]) after the built face was
/// dropped to bound the memory the cache holds.
#[derive(Default)]
pub(super) struct Instances {
    /// The id of each face and setting.
    pub(super) ids: HashMap<InstanceKey, u32>,
    /// What each id is: the face it was instanced from and its axis settings.
    pub(super) recipes: HashMap<u32, (u32, Vec<Variation>)>,
    /// The instanced faces built, by id; dropped all at once when [`MAX_LIVE_INSTANCES`] are held.
    pub(super) live: HashMap<u32, Arc<FontFace>>,
}

/// A face instanced at axis settings: the id of the face, and the settings by tag, as bits.
pub(super) type InstanceKey = (u32, Vec<([u8; 4], u32)>);

/// How many instanced faces [`Instances::live`] holds at most.
pub(crate) const MAX_LIVE_INSTANCES: usize = 256;

#[derive(Clone)]
pub(super) enum FontBytes {
    Static(&'static [u8]),
    Owned(Arc<Vec<u8>>),
}

/// One loaded font face.
pub struct FontFace {
    /// Every name the face answers to (normalized), in every language its name table has.
    pub(super) keys: FaceKeys,
    /// Its kind, for font menus' filters.
    pub traits: FontTraits,
    pub(super) id: u32,
    /// Typographic family name (e.g. "Source Sans 3").
    pub family: String,
    /// Non-English family name from the name table (`ヒラギノ角ゴシック`), when the font has one.
    /// Used when Preferences › Type › Show Font Names in English is off.
    pub native_family: Option<String>,
    /// Typographic style name (e.g. "Semibold", "Italic").
    pub style: String,
    /// The font's version string (name ID 5, e.g. "Version 7.200"; empty when it has none). Two
    /// installed versions of one family and style are told apart by it (see [`FontDb::face_version`]).
    pub version: String,
    /// usWeightClass-style weight (400 = regular).
    pub weight: f32,
    pub italic: bool,
    pub(super) bytes: FontBytes,
    pub(super) index: u32,
    /// The file the face was read from (cataloged system fonts); `None` for the bundled fonts and
    /// fonts added as bytes.
    pub(super) path: Option<std::path::PathBuf>,
    pub(crate) upem: f64,
    /// Ascender in font units (positive = up).
    pub(crate) ascent: f64,
    /// Descender in font units (positive = down).
    pub(crate) descent: f64,
    /// Cap height and x height in font units (estimated from the ascent when the font has no OS/2
    /// values).
    pub(crate) cap_height: f64,
    pub(crate) x_height: f64,
    /// Underline and strikeout: (offset from the baseline up to the line's top, thickness), in
    /// font units (the `post` and OS/2 tables', else a common placement).
    pub(crate) underline: (f64, f64),
    pub(crate) strikeout: (f64, f64),
    pub(crate) shaper: harfrust::ShaperData,
    /// The axis settings of the variable font's named instance this face is (user units: `wght`
    /// 700); empty for a static face or a variable font's default instance.
    pub(super) variations: Vec<Variation>,
    /// [`Self::variations`] in the font's normalized design space (outlines and metrics).
    pub(super) location: Location,
    /// [`Self::variations`] for the shaper (advances, kerning, feature variations).
    pub(crate) instance: Option<harfrust::ShaperInstance>,
    /// [`Self::ideographic_centre`], read once: layout asks for it per glyph.
    pub(super) ideographic_centre: std::sync::OnceLock<f64>,
    /// [`Self::icf_margins`], read once.
    pub(super) icf_margins: std::sync::OnceLock<IcfMargins>,
    /// Has `sbix` or `CBDT` strikes: colour bitmap glyphs ([`FontDb::glyph_image`]).
    pub(super) colour_bitmaps: bool,
    /// Has `COLR` colour glyphs ([`FontDb::glyph_colour`]).
    pub(super) colour_layers: bool,
    /// Has an `SVG ` table: OpenType SVG glyphs ([`FontDb::glyph_colour`]).
    pub(super) svg_glyphs: bool,
}

/// Where the ideographic character face (ICF) lies inside the ideographic em box: its distance
/// from each edge of the em box, in ems. `top` and `bottom` along a horizontal line, `right` and
/// `left` across a vertical one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IcfMargins {
    pub top: f64,
    pub bottom: f64,
    pub right: f64,
    pub left: f64,
}

impl std::fmt::Debug for FontFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FontFace({} {})", self.family, self.style)
    }
}

impl FontFace {
    pub(crate) fn data(&self) -> &[u8] {
        match &self.bytes {
            FontBytes::Static(b) => b,
            FontBytes::Owned(v) => v.as_slice(),
        }
    }
    pub(crate) fn skrifa(&self) -> Option<skrifa::FontRef<'_>> {
        skrifa::FontRef::from_index(self.data(), self.index).ok()
    }
    pub(crate) fn hb(&self) -> Option<harfrust::FontRef<'_>> {
        harfrust::FontRef::from_index(self.data(), self.index).ok()
    }
    /// The face's index in its font file (collections hold several).
    pub(crate) fn index(&self) -> u32 {
        self.index
    }
    /// Where in a variable font's design space the face's outlines and metrics are taken (the
    /// default location for a static face).
    pub(crate) fn location(&self) -> LocationRef<'_> {
        LocationRef::from(&self.location)
    }
    /// The axis settings of the variable font's named instance this face is (`wght` 700 for
    /// Bold), in user units; empty for a static face and a variable font's default instance.
    /// The face's file holds the whole variable font: exporters that embed it pass these on.
    pub fn variations(&self) -> &[Variation] {
        &self.variations
    }
    /// The variation axes of the face's font (its `fvar` table, at most 64); none for a static
    /// face.
    pub fn axes(&self) -> Vec<VariationAxis> {
        let Some(f) = self.skrifa() else { return vec![] };
        f.axes()
            .iter()
            .take(MAX_AXES)
            .map(|a| {
                let tag = a.tag().to_be_bytes();
                let fallback = || String::from_utf8_lossy(&tag).trim().to_string();
                let name = name(&f, &[a.name_id()]).unwrap_or_else(fallback);
                VariationAxis { tag, name, min: a.min_value(), default: a.default_value(), max: a.max_value(), hidden: a.is_hidden() }
            })
            .collect()
    }
    /// The face's setting of axis `tag`: its named instance's ([`Self::variations`]), else the
    /// axis default. `None` when the font has no such axis.
    pub fn axis_value(&self, tag: [u8; 4]) -> Option<f32> {
        let axis = self.axes().into_iter().find(|a| a.tag == tag)?;
        Some(self.variations.iter().find(|(t, _)| *t == tag).map_or(axis.default, |(_, v)| *v))
    }
    /// Unique id of this face within the process.
    pub fn id(&self) -> u32 {
        self.id
    }
    /// Does the face map `c` to a glyph?
    pub fn covers(&self, c: char) -> bool {
        self.skrifa().is_some_and(|f| f.charmap().map(c).is_some())
    }
    /// Units per em.
    pub fn units_per_em(&self) -> f64 {
        self.upem
    }
    /// (ascent, descent) in font units, both positive.
    pub fn vertical_metrics(&self) -> (f64, f64) {
        (self.ascent, self.descent)
    }
    /// The height of the lowercase letters in font units, positive (estimated from the ascent
    /// when the font has no OS/2 value).
    pub fn x_height(&self) -> f64 {
        self.x_height
    }
    /// The line gap the font asks for between lines, in font units (from the same table as
    /// [`vertical_metrics`](Self::vertical_metrics); 0 when it has none).
    pub fn line_gap(&self) -> f64 {
        self.skrifa().map_or(0.0, |f| f64::from(f.metrics(Size::unscaled(), self.location()).leading)).max(0.0)
    }
    /// Every mapped character and its glyph id, sorted by code point (the Glyphs panel).
    pub fn chars(&self) -> Vec<(char, u32)> {
        let Some(f) = self.skrifa() else { return vec![] };
        let mut v: Vec<(char, u32)> = f.charmap().mappings().filter_map(|(cp, g)| char::from_u32(cp).map(|c| (c, g.to_u32()))).collect();
        v.sort_unstable_by_key(|x| x.0);
        v.dedup_by_key(|x| x.0);
        v
    }
    /// Advance width of glyph `gid` in font units.
    pub fn advance(&self, gid: u32) -> f64 {
        self.skrifa()
            .and_then(|f| f.glyph_metrics(Size::unscaled(), self.location()).advance_width(GlyphId::new(gid)))
            .map(|a| a as f64)
            .unwrap_or(self.upem * 0.5)
    }
    /// Glyph `gid` set upright in vertical type, from the font's vertical metrics: (its advance down
    /// the column, the height above the baseline of its vertical origin, the top of its cell), in
    /// font units. The origin is the `VORG` table's, else the glyph's top plus its top side bearing.
    /// `None` when the font has no vertical metrics (`vhea`/`vmtx`), or they make no sense.
    pub fn vertical_glyph(&self, gid: u32) -> Option<(f64, f64)> {
        use skrifa::raw::TableProvider;
        let f = self.skrifa()?;
        let vmtx = f.vmtx().ok()?;
        let g = GlyphId::new(gid);
        let advance = f64::from(vmtx.advance(g)?);
        let origin = match f.vorg() {
            Ok(vorg) => f64::from(vorg.vertical_origin_y(g)),
            Err(_) => {
                let top = f.glyph_metrics(Size::unscaled(), self.location()).bounds(g).map_or(self.ascent, |b| f64::from(b.y_max));
                top + f64::from(vmtx.side_bearing(g)?)
            }
        };
        let em = self.upem;
        // A glyph's cell is somewhere between a tenth of an em and a few ems, its top within a few
        // ems of the baseline: anything else is a damaged table.
        (advance > em * 0.1 && advance < em * 4.0 && origin.abs() < em * 4.0).then_some((advance, origin))
    }
    /// Height above the baseline of the centre of the face's ideographic em box, in ems: from its
    /// vertical metrics (an ideograph's cell), else the usual 0.38 (the box running from 0.12 em
    /// below the baseline to 0.88 em above it).
    pub fn ideographic_centre(&self) -> f64 {
        *self.ideographic_centre.get_or_init(|| {
            let gid = ['国', 'あ', '一'].into_iter().map(|c| self.glyph_for(c)).find(|g| *g != 0);
            gid.and_then(|g| self.vertical_glyph(g)).map_or(0.38, |(advance, origin)| (origin - advance * 0.5) / self.upem)
        })
    }
    /// The ideographic character face as margins inside the em box ([`IcfMargins`]): from the
    /// font's BASE table as the OpenType baseline tags define it (`icfb`, `icft` against `ideo`,
    /// `idtp`), else, as that definition allows, from the bounds of some ideographs and kana
    /// averaged, else none (the ICF is the em box).
    /// <https://learn.microsoft.com/en-us/typography/opentype/spec/baselinetags>
    pub fn icf_margins(&self) -> IcfMargins {
        *self.icf_margins.get_or_init(|| self.icf_from_base().or_else(|| self.icf_from_glyphs()).unwrap_or_default())
    }

    /// [`Self::icf_margins`] from the BASE table: the ICF's bottom edge `icfb` (top edge `icft`, else
    /// as far below the em box top) against the em box `ideo` .. `idtp` (else the OS/2 typographic
    /// descender, and one em above it); across vertical lines, `icfb` and `icft` of the vertical
    /// axis against 0 .. `idtp` (else the horizontal margin, and one em).
    pub(crate) fn icf_from_base(&self) -> Option<IcfMargins> {
        use skrifa::raw::TableProvider;
        let f = self.skrifa()?;
        let base = f.base().ok()?;
        let em = self.upem;
        let h = base_values(base.horiz_axis()?.ok()?)?;
        let icfb = *h.get(b"icfb")?;
        let em_bottom = match h.get(b"ideo") {
            Some(v) => *v,
            None => f64::from(f.os2().ok()?.s_typo_descender()),
        };
        let em_top = h.get(b"idtp").copied().unwrap_or(em_bottom + em);
        let margin = icfb - em_bottom;
        let icf_top = h.get(b"icft").copied().unwrap_or(em_top - margin);
        let v = base.vert_axis().and_then(Result::ok).and_then(base_values).unwrap_or_default();
        let icf_left = v.get(b"icfb").copied().unwrap_or(margin);
        let em_right = v.get(b"idtp").copied().unwrap_or(em);
        let icf_right = v.get(b"icft").copied().unwrap_or(em_right - icf_left);
        let m = IcfMargins { top: (em_top - icf_top) / em, bottom: margin / em, right: (em_right - icf_right) / em, left: icf_left / em };
        // Inside the em box, short of its middle: anything else is a damaged table.
        [m.top, m.bottom, m.right, m.left].iter().all(|x| (0.0..0.5).contains(x)).then_some(m)
    }

    /// [`Self::icf_margins`] from the ink of some ideographs and kana: each edge's margin to the em
    /// box (its centre [`Self::ideographic_centre`], one em high and wide), averaged.
    pub(crate) fn icf_from_glyphs(&self) -> Option<IcfMargins> {
        let f = self.skrifa()?;
        let metrics = f.glyph_metrics(Size::unscaled(), self.location());
        let em = self.upem;
        let (em_bottom, em_top) = ((self.ideographic_centre() - 0.5) * em, (self.ideographic_centre() + 0.5) * em);
        let boxes: Vec<_> = ['国', '東', '永', '書', 'あ', 'ア']
            .into_iter()
            .map(|c| self.glyph_for(c))
            .filter(|g| *g != 0)
            .filter_map(|g| metrics.bounds(GlyphId::new(g)))
            .collect();
        if boxes.is_empty() {
            return None;
        }
        let n = boxes.len() as f64;
        let mean = |v: &dyn Fn(&skrifa::metrics::BoundingBox) -> f32| boxes.iter().map(|b| f64::from(v(b))).sum::<f64>() / n;
        let m = IcfMargins {
            top: (em_top - mean(&|b| b.y_max)) / em,
            bottom: (mean(&|b| b.y_min) - em_bottom) / em,
            right: (em - mean(&|b| b.x_max)) / em,
            left: mean(&|b| b.x_min) / em,
        };
        [m.top, m.bottom, m.right, m.left].iter().all(|x| (0.0..0.5).contains(x)).then_some(m)
    }

    /// Glyph id for `c` (0 = .notdef).
    pub fn glyph_for(&self, c: char) -> u32 {
        self.skrifa().and_then(|f| f.charmap().map(c)).map(|g| g.to_u32()).unwrap_or(0)
    }
    /// The file the face was read from (cataloged system fonts).
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }
    /// The bytes of the face's font file (a collection holds several faces).
    pub fn file_data(&self) -> &[u8] {
        self.data()
    }
    /// The embedding permissions of the OS/2 table (`fsType`; 0, installable, when it has none).
    pub fn fs_type(&self) -> u16 {
        use skrifa::raw::TableProvider;
        self.skrifa().and_then(|f| f.os2().ok()).map_or(0, |t| t.fs_type())
    }
    /// May the font file be copied along with a document? Its license allows embedding its
    /// outlines, as exports check it ([`Self::embedding`]).
    pub fn embeddable(&self) -> bool {
        self.embedding() != crate::embed::Embedding::Forbidden
    }
    /// The face's index in its font file ([`Self::file_data`]; collections hold several).
    pub fn face_index(&self) -> u32 {
        self.index
    }
    /// The face's PostScript name, as PostScript files name fonts (`SourceSans3-Bold`): name ID 6;
    /// for a named instance of a variable font (whose name ID 6 is the default instance's), its
    /// prefix and style (`SourceSans3-Semibold`). Without one, its family and style.
    pub fn postscript_name(&self) -> String {
        let squeeze = |s: &str| -> String { s.chars().filter(|c| c.is_ascii_graphic() && !"[](){}<>/%".contains(*c)).collect() };
        let own = self.skrifa().and_then(|f| localized(&f, StringId::POSTSCRIPT_NAME).into_iter().map(|(_, s)| s).next()).map(|s| squeeze(&s));
        match own.filter(|s| !s.is_empty()) {
            Some(name) if self.variations.is_empty() => name,
            own => {
                let prefix = own.as_deref().and_then(|n| n.split('-').next()).map_or_else(|| squeeze(&self.family), str::to_string);
                format!("{prefix}-{}", squeeze(&self.style))
            }
        }
    }
    /// Are the face's outlines TrueType (`glyf`), not PostScript (`CFF `)?
    pub fn is_truetype(&self) -> bool {
        use skrifa::raw::TableProvider;
        self.skrifa().is_some_and(|f| f.glyf().is_ok())
    }
}
