//! Font database: bundled OFL fonts, user fonts, the installed system fonts (cataloged once, loaded
//! on demand), outline cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use kurbo::BezPath;
use skrifa::instance::{Location, LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::FileRef;
use skrifa::string::StringId;
use skrifa::{GlyphId, MetadataProvider, Tag};

use crate::GlyphImage;
use crate::colr::ColourContent;
use vectorcraft_doc::color::Color;

/// The family used when a requested family is unknown (Illustrator's Myriad Pro analogue).
pub const FALLBACK_FAMILY: &str = "Source Sans 3";

/// The fonts every database starts with (feature `bundled-fonts`, on by default).
#[cfg(feature = "bundled-fonts")]
static BUNDLED: &[&[u8]] = &[
    include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf"),
    include_bytes!("../../../assets/fonts/SourceSans3-Semibold.ttf"),
    include_bytes!("../../../assets/fonts/SourceSans3-Bold.ttf"),
    include_bytes!("../../../assets/fonts/SourceSans3-It.ttf"),
    include_bytes!("../../../assets/fonts/SourceSerif4-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"),
];
/// Without `bundled-fonts` a database starts with none: the embedder adds its fonts
/// ([`FontDb::add_font`]); until then text has no glyphs.
#[cfg(not(feature = "bundled-fonts"))]
static BUNDLED: &[&[u8]] = &[];

/// An axis setting of a variable font: the axis tag (`wght`) and its value in user units (700).
pub type Variation = ([u8; 4], f32);

/// Caps on what a (possibly damaged) variable font can make the database read: axes per font and
/// named instances listed per face.
const MAX_AXES: usize = 64;
const MAX_INSTANCES: usize = 512;

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
struct Instances {
    /// The id of each face and setting.
    ids: HashMap<InstanceKey, u32>,
    /// What each id is: the face it was instanced from and its axis settings.
    recipes: HashMap<u32, (u32, Vec<Variation>)>,
    /// The instanced faces built, by id; dropped all at once when [`MAX_LIVE_INSTANCES`] are held.
    live: HashMap<u32, Arc<FontFace>>,
}

/// A face instanced at axis settings: the id of the face, and the settings by tag, as bits.
type InstanceKey = (u32, Vec<([u8; 4], u32)>);

/// How many instanced faces [`Instances::live`] holds at most.
pub(crate) const MAX_LIVE_INSTANCES: usize = 256;

#[derive(Clone)]
enum FontBytes {
    Static(&'static [u8]),
    Owned(Arc<Vec<u8>>),
}

/// One loaded font face.
pub struct FontFace {
    /// Every name the face answers to (normalized), in every language its name table has.
    keys: FaceKeys,
    /// Its kind, for font menus' filters.
    pub traits: FontTraits,
    id: u32,
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
    bytes: FontBytes,
    index: u32,
    /// The file the face was read from (cataloged system fonts); `None` for the bundled fonts and
    /// fonts added as bytes.
    path: Option<std::path::PathBuf>,
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
    variations: Vec<Variation>,
    /// [`Self::variations`] in the font's normalized design space (outlines and metrics).
    location: Location,
    /// [`Self::variations`] for the shaper (advances, kerning, feature variations).
    pub(crate) instance: Option<harfrust::ShaperInstance>,
    /// [`Self::ideographic_centre`], read once: layout asks for it per glyph.
    ideographic_centre: std::sync::OnceLock<f64>,
    /// [`Self::icf_margins`], read once.
    icf_margins: std::sync::OnceLock<IcfMargins>,
    /// Has `sbix` or `CBDT` strikes: colour bitmap glyphs ([`FontDb::glyph_image`]).
    colour_bitmaps: bool,
    /// Has `COLR` colour glyphs ([`FontDb::glyph_colour`]).
    colour_layers: bool,
    /// Has an `SVG ` table: OpenType SVG glyphs ([`FontDb::glyph_colour`]).
    svg_glyphs: bool,
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

/// One installed family found by the system font scan: its name and the file of each style.
#[derive(Debug, Default)]
struct CatalogFamily {
    name: String,
    /// Non-English family name from the name table, when the scan found one.
    native: Option<String>,
    faces: Vec<CatalogFace>,
    /// The family's kind (from its first face found); variable when any face found is.
    traits: Option<FontTraits>,
}

/// One style of an installed family: a face, or a named instance of a variable font.
#[derive(Debug)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct CatalogFace {
    style: String,
    path: PathBuf,
    /// Weight and italic as the style lists them (by its name, or a named instance's axes).
    weight: f32,
    italic: bool,
}

/// The installed fonts by ASCII-lowercased family name (lookups ignore ASCII case, as
/// [`FontDb::face`] does). Always empty on wasm, which has no system fonts.
type Catalog = HashMap<String, CatalogFamily>;

/// The installed faces' (family, style) by normalized PostScript name ([`norm`]).
type PostScriptNames = HashMap<String, (String, String)>;

/// Maps the family and style [`FontDb::canonical`] was asked for to its answer, for names that
/// are neither a loaded nor an installed family.
type CanonicalNames = HashMap<(String, String), (String, String)>;

/// [`FontDb::glyph_image`]'s answers by face and glyph.
type GlyphImages = GlyphCache<(u32, u32), Arc<GlyphImage>>;
/// [`FontDb::glyph_colour`]'s answers by face and glyph, and the text's fill and stroke colours for
/// glyphs that paint with them.
type GlyphColours = GlyphCache<(u32, u32, Option<TextColours>), ColourContent>;
/// The text's fill and stroke colours, as a cache key ([`Color::key`]).
type TextColours = ([u32; 5], Option<[u32; 5]>);

/// What a glyph's colour content (an image, layers) takes of memory, in bytes.
trait Weight {
    fn bytes(&self) -> usize;
}

impl Weight for Arc<GlyphImage> {
    fn bytes(&self) -> usize {
        self.blob.bytes.len()
    }
}

impl Weight for ColourContent {
    fn bytes(&self) -> usize {
        match self {
            ColourContent::Image(i) => i.bytes(),
            // Roughly what a path node with its outline and fill takes.
            ColourContent::Art(art) => {
                let mut nodes = 0;
                art.walk(&mut |_| nodes += 1);
                nodes * 512
            }
        }
    }
}

/// The colour content of glyphs ([`FontDb::glyph_image`], [`FontDb::glyph_colour`]) kept by
/// face, glyph and what else it depends on, and its bytes in all.
struct GlyphCache<K, V> {
    kept: HashMap<K, Option<V>>,
    bytes: usize,
}

impl<K, V> Default for GlyphCache<K, V> {
    fn default() -> Self {
        Self { kept: HashMap::new(), bytes: 0 }
    }
}

impl<K: std::hash::Hash + Eq, V: Weight> GlyphCache<K, V> {
    /// Keeps `value` (or that there is none), first letting go of all it keeps when that would
    /// take more than [`IMAGE_CACHE_MAX`] glyphs or [`IMAGE_CACHE_BYTES`]. A glyph it keeps
    /// already, which another thread looked up meanwhile, stays as it is.
    fn keep(&mut self, key: K, value: Option<V>) {
        if self.kept.contains_key(&key) {
            return;
        }
        let bytes = value.as_ref().map_or(0, Weight::bytes);
        if self.kept.len() >= IMAGE_CACHE_MAX || self.bytes.saturating_add(bytes) > IMAGE_CACHE_BYTES {
            self.kept.clear();
            self.bytes = 0;
        }
        self.bytes += bytes;
        self.kept.insert(key, value);
    }
}

/// Process-wide font database.
pub struct FontDb {
    faces: RwLock<Vec<Arc<FontFace>>>,
    outlines: Mutex<HashMap<(u32, u32), Arc<BezPath>>>,
    /// [`FontDb::glyph_image`]'s answers per (face, glyph), misses too.
    images: Mutex<GlyphImages>,
    colours: Mutex<GlyphColours>,
    catalog: RwLock<Catalog>,
    /// The installed faces by PostScript name (documents name fonts so: PDF, EPS, .ai).
    postscript: RwLock<PostScriptNames>,
    /// Where the system font scan looks.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    font_paths: FontPaths,
    /// The installed fonts' other names (localized, legacy, full, PostScript) by normalized name.
    aliases: RwLock<HashMap<String, Vec<Alias>>>,
    /// Keeps [`FontDb::canonical`]'s answers for names other than the loaded and installed
    /// families until fonts are added or rescanned.
    canonical_cache: Mutex<CanonicalNames>,
    /// Set once the font folders have been scanned. Lookups by family name wait for the first
    /// scan, so what they find never depends on what ran before them.
    #[cfg(not(target_arch = "wasm32"))]
    cataloged: std::sync::OnceLock<()>,
    /// Families (lowercased) whose installed files were all loaded to find other versions of their
    /// faces (see [`FontDb::versions`]); a rescan forgets them.
    #[cfg(not(target_arch = "wasm32"))]
    version_scanned: RwLock<std::collections::HashSet<String>>,
    /// [`FontDb::family_list`], built on demand and dropped when fonts are added or rescanned.
    family_cache: Mutex<Option<Arc<[String]>>>,
    menu_cache: Mutex<Option<MenuFamilies>>,
    generation: AtomicU64,
    /// System fallback state: characters no system font covers.
    #[cfg(not(target_arch = "wasm32"))]
    sys: Mutex<SysFallback>,
    /// What the last scan read, to tell when fonts were installed or removed since.
    #[cfg(not(target_arch = "wasm32"))]
    stamp: Mutex<Option<ScanStamp>>,
    /// Faces instanced at the axis settings of type's Variable Font sliders ([`FontDb::instance`]).
    instances: Mutex<Instances>,
}

/// Where a database's system font scan looks: folders (read with their subfolders) and font files.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
enum FontPaths {
    Fixed(Vec<PathBuf>),
    /// The platform's ([`system_font_dirs`]), looked up again by each scan: fonts registered and
    /// font folders added since count too.
    System,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
impl FontPaths {
    fn get(&self) -> Vec<PathBuf> {
        match self {
            FontPaths::Fixed(paths) => paths.clone(),
            FontPaths::System => system_font_dirs(),
        }
    }
}

/// What a system font scan read: the paths it started from, and when each folder it listed (and
/// font file named by itself) last changed. Installing or removing a font changes its folder's.
#[cfg(not(target_arch = "wasm32"))]
struct ScanStamp {
    paths: Vec<PathBuf>,
    modified: Vec<(PathBuf, Option<std::time::SystemTime>)>,
}

#[cfg(not(target_arch = "wasm32"))]
fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct SysFallback {
    enabled: bool,
    misses: std::collections::HashSet<char>,
    /// Counts the scans of the font folders: a search that a scan overtook doesn't record its miss.
    scans: u64,
}

/// Fonts made for Thai, tried first (when installed) for Thai the font in use lacks: Windows
/// (Leelawadee UI; Tahoma before Windows 8), macOS (Thonburi, Sukhumvit Set) and Linux (Noto,
/// TLWG).
pub const THAI_FAMILIES: &[&str] = &[
    "Leelawadee UI",
    "Leelawadee",
    "Tahoma",
    "Thonburi",
    "Sukhumvit Set",
    "Noto Sans Thai",
    "Noto Sans Thai UI",
    "Noto Sans Thai Looped",
    "Waree",
    "Loma",
    "Garuda",
];

/// The families tried first for character `c`, in a script whose text only fonts made for it set
/// well (Thai, Lao, Khmer, Myanmar); none for other characters.
fn script_families(c: char) -> &'static [&'static str] {
    use unicode_script::{Script, UnicodeScript};
    match c.script() {
        Script::Thai => THAI_FAMILIES,
        Script::Lao => &["Lao UI", "Leelawadee UI", "Lao Sangam MN", "Noto Sans Lao", "Noto Sans Lao UI", "Phetsarath OT"],
        Script::Khmer => &["Khmer UI", "Khmer Sangam MN", "Noto Sans Khmer", "Noto Sans Khmer UI", "Khmer OS"],
        Script::Myanmar => &["Myanmar Text", "Myanmar Sangam MN", "Noto Sans Myanmar", "Noto Sans Myanmar UI", "Padauk"],
        _ => &[],
    }
}

/// Families tried (when installed) for characters the loaded fonts lack: CJK, symbols, emoji.
#[cfg(not(target_arch = "wasm32"))]
const SYSTEM_FALLBACKS: &[&str] = &[
    "Helvetica Neue",
    "Arial",
    "Segoe UI",
    "Noto Sans",
    "DejaVu Sans",
    "PingFang SC",
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Apple SD Gothic Neo",
    "Heiti SC",
    "STHeiti",
    "Microsoft YaHei",
    "Yu Gothic",
    "Malgun Gothic",
    "Noto Sans CJK SC",
    "Noto Sans CJK JP",
    "Arial Unicode MS",
    "Apple Symbols",
    "Segoe UI Symbol",
    "Noto Sans Symbols",
    "Noto Sans Symbols2",
    "Noto Emoji",
    "Segoe UI Emoji",
    "Apple Color Emoji",
    "Noto Color Emoji",
];

static NEXT_ID: AtomicU32 = AtomicU32::new(1);
const OUTLINE_CACHE_MAX: usize = 50_000;
/// The most glyph images [`FontDb::glyph_image`] keeps (each a PNG of some 10–60 KB).
const IMAGE_CACHE_MAX: usize = 4_096;
/// The most PNG bytes it keeps: a font can make each one up to [`crate::bitmap::MAX_PNG_BYTES`].
const IMAGE_CACHE_BYTES: usize = 64 << 20;
/// How many answers [`FontDb::canonical`] keeps at most. When it has kept that many, it clears
/// them before it keeps the next one.
const CANONICAL_CACHE_MAX: usize = 4_096;

fn name(font: &skrifa::FontRef<'_>, ids: &[StringId]) -> Option<String> {
    ids.iter().find_map(|id| font.localized_strings(*id).english_or_first().map(|s| s.to_string()).filter(|s| !s.is_empty()))
}

/// The most distinct legacy family (and style) names of a font used as keys.
const MAX_LEGACY_NAMES: usize = 64;

/// Every localized string of `id`, with its language.
fn localized(font: &skrifa::FontRef<'_>, id: StringId) -> Vec<(Option<String>, String)> {
    font.localized_strings(id).map(|s| (s.language().map(str::to_owned), s.to_string())).filter(|(_, s)| !s.is_empty()).collect()
}

/// Whether a name-table language tag is English (Illustrator's "Show Font Names in English").
fn is_english_lang(lang: Option<&str>) -> bool {
    lang.is_some_and(|l| {
        let l = l.to_ascii_lowercase();
        l == "en" || l == "eng" || l.starts_with("en-") || l.starts_with("en_")
    })
}

/// A non-English family label from the name table, for Preferences › Type › Show Font Names in
/// English (off). Prefers a record whose language isn't English; else any label that differs from
/// `english` and isn't plain ASCII. `None` when the font has no other name.
fn pick_native_label(english: &str, pairs: &[(Option<String>, String)]) -> Option<String> {
    pairs
        .iter()
        .find(|(lang, s)| !s.eq_ignore_ascii_case(english) && !is_english_lang(lang.as_deref()))
        .or_else(|| pairs.iter().find(|(_, s)| !s.eq_ignore_ascii_case(english) && !s.is_ascii()))
        .map(|(_, s)| s.clone())
}

/// The family's native-language name from the font (typographic, else legacy), when it has one.
fn native_family_name(font: &skrifa::FontRef<'_>, english: &str) -> Option<String> {
    pick_native_label(english, &localized(font, StringId::TYPOGRAPHIC_FAMILY_NAME))
        .or_else(|| pick_native_label(english, &localized(font, StringId::FAMILY_NAME)))
}

/// Every localized string of `id`.
fn all_names(font: &skrifa::FontRef<'_>, id: StringId) -> Vec<String> {
    localized(font, id).into_iter().map(|(_, s)| s).collect()
}

/// The names a face answers to besides its (English) family and style, normalized ([`norm`]):
/// its family in every language (typographic, else legacy: `ヒラギノ角ゴシック`), its style likewise
/// (`ミディアム`), its legacy family names (`Hiragino Sans W3`) with their legacy styles, paired per
/// language record (the Mac and Windows records of a face differ), its full names (name ID 4),
/// and its PostScript names (`HiraginoSans-W3`, as PDF and `.ai` files name fonts).
#[derive(Clone, Debug, Default)]
struct FaceKeys {
    families: Vec<String>,
    styles: Vec<String>,
    legacy: Vec<(String, String)>,
    full_names: Vec<String>,
    postscript: Vec<String>,
}

impl FaceKeys {
    fn of(font: &skrifa::FontRef<'_>, family: &str, style: &str) -> Self {
        let (typo_family, typo_style) = (all_names(font, StringId::TYPOGRAPHIC_FAMILY_NAME), all_names(font, StringId::TYPOGRAPHIC_SUBFAMILY_NAME));
        let (mut legacy_family, mut legacy_style) = (localized(font, StringId::FAMILY_NAME), localized(font, StringId::SUBFAMILY_NAME));
        // Records repeat (platforms, encodings) and are paired below: once each, and no more than a
        // font has any use for, or the pairs multiply.
        for names in [&mut legacy_family, &mut legacy_style] {
            names.sort();
            names.dedup();
            names.truncate(MAX_LEGACY_NAMES);
        }
        // Without typographic names, the legacy ones are the family's.
        let family_names: Vec<&str> = if typo_family.is_empty() {
            legacy_family.iter().map(|(_, f)| f.as_str()).collect()
        } else {
            typo_family.iter().map(String::as_str).collect()
        };
        let mut families: Vec<String> = std::iter::once(family).chain(family_names).map(norm).collect();
        // A legacy style only names the face beside its legacy family when the font has a
        // typographic style: every weight of `Hiragino Sans` is `Regular` in its own legacy family.
        let style_names: Vec<&str> = if typo_style.is_empty() {
            legacy_style.iter().map(|(_, s)| s.as_str()).collect()
        } else {
            typo_style.iter().map(String::as_str).collect()
        };
        let mut styles: Vec<String> = std::iter::once(style).chain(style_names).map(norm).collect();
        let mut legacy: Vec<(String, String)> = if typo_family.is_empty() {
            vec![]
        } else {
            legacy_family
                .iter()
                .flat_map(|(lf, f)| legacy_style.iter().filter(move |(ls, _)| ls == lf).map(move |(_, s)| (norm(f), norm(s))))
                .filter(|(f, s)| !f.is_empty() && !s.is_empty())
                .collect()
        };
        let mut postscript: Vec<String> = all_names(font, StringId::POSTSCRIPT_NAME).iter().map(|n| norm(n)).collect();
        let mut full_names: Vec<String> = all_names(font, StringId::FULL_NAME).iter().map(|n| norm(n)).collect();
        for v in [&mut families, &mut styles, &mut full_names, &mut postscript] {
            v.retain(|k| !k.is_empty());
            v.sort();
            v.dedup();
        }
        legacy.sort();
        legacy.dedup();
        FaceKeys { families, styles, legacy, full_names, postscript }
    }
}

/// A font's kind, for filtering font menus: the classification of its OS/2 table (IBM family
/// class, PANOSE) and, for CJK fonts whose tables seldom say, its names (明朝, ゴシック, 丸…).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontClass {
    /// Serif (Latin) or Mincho (CJK).
    Serif,
    /// Sans serif (Latin) or Gothic (CJK).
    Sans,
    /// Rounded sans / Maru Gothic.
    Rounded,
    /// Script, handwriting, brush (楷書・行書・教科書…).
    Script,
    Monospaced,
    /// Decorative, display, symbol and pictorial fonts.
    Decorative,
    #[default]
    Other,
}

impl FontClass {
    /// The kinds a font menu filters by (all but `Other`).
    pub const ALL: [FontClass; 6] =
        [FontClass::Serif, FontClass::Sans, FontClass::Rounded, FontClass::Script, FontClass::Monospaced, FontClass::Decorative];
}

/// What a font menu filters on: whether a family sets Japanese or Thai, its kind, and whether it
/// is a variable font.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontTraits {
    pub japanese: bool,
    /// Its character map has Thai (ก and า).
    pub thai: bool,
    pub class: FontClass,
    /// The face has variation axes (an `fvar` table of at most 64 axes); for a family, one of
    /// its faces has.
    pub variable: bool,
}

impl FontTraits {
    /// From a face's OS/2 and `fvar` tables (when it has them) and names.
    fn of(font: &skrifa::FontRef<'_>, keys: &FaceKeys) -> Self {
        use skrifa::raw::TableProvider;
        let os2 = font.os2().ok();
        let names: Vec<&str> = keys.families.iter().chain(&keys.postscript).map(String::as_str).collect();
        let named = |words: &[&str]| names.iter().any(|n| words.iter().any(|w| n.contains(w)));
        let has_japanese_name = keys.families.iter().any(|k| k.chars().any(|c| matches!(c as u32, 0x3040..=0x30FF | 0x4E00..=0x9FFF)));
        let (class_id, panose, ranges2, codepages) = match &os2 {
            Some(o) => (o.s_family_class(), o.panose_10().to_vec(), o.ul_unicode_range_2(), o.ul_code_page_range_1().unwrap_or(0)),
            None => (0, vec![], 0, 0),
        };
        // Hiragana (bit 49) or Katakana (bit 50) in the Unicode ranges, or the JIS code page.
        let japanese = ranges2 & (0b11 << 17) != 0 || codepages & (1 << 17) != 0 || has_japanese_name;
        let thai = ['\u{0E01}', '\u{0E32}'].iter().all(|c| font.charmap().map(*c).is_some());
        let family_type = panose.first().copied().unwrap_or(0);
        let serif_style = panose.get(1).copied().unwrap_or(0);
        let proportion = panose.get(3).copied().unwrap_or(0);
        let ibm = (class_id >> 8) as u8;
        let class = if named(&["丸", "maru", "rounded", "round"]) || japanese && named(&["jun"]) {
            FontClass::Rounded
        } else if named(&[
            "楷書",
            "行書",
            "草書",
            "隷書",
            "教科書",
            "毛筆",
            "筆",
            "kaisho",
            "gyosho",
            "sosho",
            "reisho",
            "kyokasho",
            "brush",
            "script",
            "hand",
            "kaiti",
            "xingkai",
            "libian",
        ]) {
            FontClass::Script
        } else if named(&["mono", "code", "courier", "menlo", "consol"]) {
            FontClass::Monospaced
        } else if named(&[
            "明朝",
            "mincho",
            "ryumin",
            "hiramin",
            "yumin",
            "kozmin",
            "serif",
            "songti",
            "simsun",
            "stsong",
            "fangsong",
            "myungjo",
            "times",
            "georgia",
            "garamond",
            "baskerville",
            "bodoni",
            "caslon",
            "palatino",
            "didot",
            "century",
            "cambria",
            "minion",
            "charter",
            "hoefler",
        ]) && !named(&["sansserif", "sans"])
        {
            FontClass::Serif
        } else if named(&[
            "ゴシック",
            "gothic",
            "goth",
            "sans",
            "kakugo",
            "kaku",
            "heiti",
            "helvetica",
            "arial",
            "grotesk",
            "grotesque",
            "futura",
            "gill",
            "verdana",
            "tahoma",
            "frutiger",
            "avenir",
            "geneva",
            "lucidagrande",
            "inter",
            "roboto",
            "optima",
            "pingfang",
        ]) {
            FontClass::Sans
        } else if family_type == 2 && proportion == 9 {
            FontClass::Monospaced
        } else if family_type == 3 || ibm == 10 {
            FontClass::Script
        } else if family_type == 4 || family_type == 5 || ibm == 9 || ibm == 12 {
            FontClass::Decorative
        } else if matches!(ibm, 1..=5 | 7) || family_type == 2 && (2..=10).contains(&serif_style) {
            FontClass::Serif
        } else if ibm == 8 || family_type == 2 && (11..=13).contains(&serif_style) {
            FontClass::Sans
        } else {
            FontClass::Other
        };
        let variable = (1..=MAX_AXES).contains(&font.axes().len());
        FontTraits { japanese, thai, class, variable }
    }
}

/// What a name other than a family's own one stands for ([`FontDb::canonical`]).
#[derive(Clone, Debug)]
struct Alias {
    /// The family's own name.
    family: String,
    /// The face the name names, for legacy family names, full names and PostScript names.
    style: Option<String>,
    /// A legacy family name's legacy style (`Regular` beside `Hiragino Sans W3`).
    paired: Option<String>,
}

impl Alias {
    /// The aliases of a face `family` / `style` with `keys`, by normalized name.
    fn of(family: &str, style: &str, keys: &FaceKeys) -> Vec<(String, Alias)> {
        #[cfg(test)]
        ALIAS_LISTS.with(|c| c.set(c.get() + 1));
        let mut v: Vec<(String, Alias)> =
            keys.families.iter().map(|k| (k.clone(), Alias { family: family.to_string(), style: None, paired: None })).collect();
        v.extend(
            keys.legacy
                .iter()
                .map(|(f, s)| (f.clone(), Alias { family: family.to_string(), style: Some(style.to_string()), paired: Some(s.clone()) })),
        );
        v.extend(
            keys.postscript
                .iter()
                .chain(&keys.full_names)
                .map(|k| (k.clone(), Alias { family: family.to_string(), style: Some(style.to_string()), paired: None })),
        );
        v
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: counts the faces whose aliases ([`Alias::of`]) were listed on this thread.
    static ALIAS_LISTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Tests: returns how many faces' aliases were listed on this thread.
#[cfg(test)]
pub(crate) fn alias_lists() -> u64 {
    ALIAS_LISTS.with(std::cell::Cell::get)
}

#[cfg(test)]
thread_local! {
    /// Tests: run on this thread each time it publishes faces to a database, to look the database
    /// up at exactly that point (as another thread could).
    static ON_FACES_PUBLISHED: std::cell::RefCell<Option<Box<dyn FnMut()>>> = const { std::cell::RefCell::new(None) };
}

/// Tests: run `hook` each time this thread publishes faces to a database.
#[cfg(test)]
pub(crate) fn on_faces_published(hook: Option<Box<dyn FnMut()>>) {
    ON_FACES_PUBLISHED.with(|h| *h.borrow_mut() = hook);
}

#[cfg(test)]
fn faces_published() {
    // Taken out while it runs, so a hook that adds faces itself doesn't run again.
    if let Some(mut hook) = ON_FACES_PUBLISHED.with(|h| h.borrow_mut().take()) {
        hook();
        ON_FACES_PUBLISHED.with(|h| {
            h.borrow_mut().get_or_insert(hook);
        });
    }
}

/// How a requested family and style resolved ([`FontDb::resolve`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontMatch {
    /// The face of that family and style (by any of its names, in any language).
    Exact,
    /// The family is there but not the style: its closest style stands in.
    Style,
    /// The family is unknown: the fallback family stands in.
    Missing,
}

impl FontMatch {
    pub fn as_str(self) -> &'static str {
        match self {
            FontMatch::Exact => "exact",
            FontMatch::Style => "substitute",
            FontMatch::Missing => "missing",
        }
    }
}

/// A face's (family, style) names.
fn face_names(f: &skrifa::FontRef<'_>) -> Option<(String, String)> {
    let family = name(f, &[StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME])?;
    let style = name(f, &[StringId::TYPOGRAPHIC_SUBFAMILY_NAME, StringId::SUBFAMILY_NAME]).unwrap_or_else(|| "Regular".into());
    Some((family, style))
}

/// One style a font file offers: one of its faces, or a named instance of a variable font
/// (Figtree's `Figtree[wght].ttf` holds Light, the default, and Regular to Black as instances).
#[derive(Clone, Debug)]
pub(crate) struct FaceStyle {
    family: String,
    /// See [`FontFace::native_family`].
    native_family: Option<String>,
    style: String,
    keys: FaceKeys,
    /// A named instance's axis settings (user units); empty for the face itself.
    variations: Vec<Variation>,
    /// A named instance's weight (its `wght` setting); `None` for the face itself (its OS/2 table
    /// says) or an instance of a font without a weight axis.
    weight: Option<f32>,
    /// A named instance is italic (by its `ital` or `slnt` setting, or its name).
    italic: bool,
    /// Its kind, for font menus' filters (the face's, shared by its named instances).
    traits: FontTraits,
}

/// The styles of a face: the face itself, then its named instances when it is a variable font.
pub(crate) fn face_styles(f: &skrifa::FontRef<'_>) -> Vec<FaceStyle> {
    let Some((family, style)) = face_names(f) else { return vec![] };
    let native_family = native_family_name(f, &family);
    let mut keys = FaceKeys::of(f, &family, &style);
    let traits = FontTraits::of(f, &keys);
    let (mut instances, renamed) = named_instances(f, &family, &style, &mut keys, native_family.clone());
    for i in &mut instances {
        i.traits = traits;
    }
    // A variable font's default face renders at its `wght` default, whatever its name table and
    // OS/2 table say (Noto Sans Hebrew: named `Light`, weight class 300, default instance Regular
    // at 400): weigh it so, and name it by its default instance when that is another weight.
    let weight = f
        .axes()
        .iter()
        .find(|a| a.tag().to_be_bytes() == *b"wght")
        .map(|a| a.default_value())
        .filter(|w| w.is_finite())
        .map(|w| w.clamp(1.0, 1000.0));
    let style = renamed.unwrap_or(style);
    let face = FaceStyle { family, native_family, style, keys, variations: vec![], weight, italic: false, traits };
    std::iter::once(face).chain(instances).collect()
}

/// The named instances of a variable font face (its `fvar` table) as styles of the face's family,
/// named by their subfamily names, without the default instance (the face itself) and without
/// names already taken. The default instance's name, when it isn't the face's style name (a face
/// `Regular` whose default instance is `Book`), names the face too (`keys`). Damaged tables give
/// fewer instances or none; the counts read are capped.
fn named_instances(
    f: &skrifa::FontRef<'_>,
    family: &str,
    style: &str,
    keys: &mut FaceKeys,
    native_family: Option<String>,
) -> (Vec<FaceStyle>, Option<String>) {
    let axes = f.axes();
    if axes.is_empty() || axes.len() > MAX_AXES {
        return (vec![], None);
    }
    let tags: Vec<Tag> = axes.iter().map(|a| a.tag()).collect();
    let defaults: Vec<f32> = axes.iter().map(|a| a.default_value()).collect();
    let is_default = |coords: &[f32]| coords.len() == defaults.len() && coords.iter().zip(&defaults).all(|(c, d)| (c - d).abs() < 1e-3);
    // The face renders as its default instance. When that instance is another weight than the
    // name table says (`Light` face, Regular default instance), the instance names the face.
    let renamed = tags.iter().any(|t| t.to_be_bytes() == *b"wght").then(|| {
        f.named_instances()
            .iter()
            .take(MAX_INSTANCES)
            .filter(|inst| is_default(&inst.user_coords().take(MAX_AXES).collect::<Vec<f32>>()))
            .find_map(|inst| name(f, &[inst.subfamily_name_id()]))
            .filter(|d| style_weight(d) != style_weight(style))
    });
    let renamed = renamed.flatten();
    // Instances without a PostScript name of their own are named `<prefix>-<style>` (Adobe
    // Technical Note #5902), as PDF and `.ai` files name them.
    let prefix = name(f, &[StringId::VARIATIONS_POSTSCRIPT_NAME_PREFIX]).unwrap_or_else(|| family.to_string());
    if let Some(new) = &renamed {
        let old = norm(style);
        keys.styles.retain(|k| *k != old);
        keys.styles.push(norm(new));
    }
    let mut taken = vec![norm(renamed.as_deref().unwrap_or(style))];
    let mut out = Vec::new();
    for inst in f.named_instances().iter().take(MAX_INSTANCES) {
        let coords: Vec<f32> = inst.user_coords().take(MAX_AXES).collect();
        if coords.len() != tags.len() || coords.iter().any(|c| !c.is_finite()) {
            continue;
        }
        let Some(style) = name(f, &[inst.subfamily_name_id()]) else { continue };
        let ns = norm(&style);
        if ns.is_empty() || taken.contains(&ns) {
            continue;
        }
        // The default instance is the face itself.
        if coords.iter().zip(&defaults).all(|(c, d)| (c - d).abs() < 1e-3) {
            keys.styles.push(ns.clone());
            keys.postscript.push(norm(&format!("{prefix}{style}")));
            taken.push(ns);
            continue;
        }
        taken.push(ns);
        let axis = |tag: &[u8; 4]| tags.iter().zip(&coords).find(|(t, _)| t.to_be_bytes() == *tag).map(|(_, v)| *v);
        let weight = axis(b"wght").map(|w| w.clamp(1.0, 1000.0));
        let italic = style_italic(&style) || axis(b"ital").is_some_and(|v| v >= 0.5) || axis(b"slnt").is_some_and(|v| v != 0.0);
        let mut styles: Vec<String> = std::iter::once(style.clone()).chain(all_names(f, inst.subfamily_name_id())).map(|s| norm(&s)).collect();
        let mut postscript: Vec<String> = inst.postscript_name_id().map(|id| all_names(f, id)).unwrap_or_default().iter().map(|n| norm(n)).collect();
        postscript.push(norm(&format!("{prefix}{style}")));
        for v in [&mut styles, &mut postscript] {
            v.retain(|k| !k.is_empty());
            v.sort();
            v.dedup();
        }
        // Name ID 4 describes the default face, not every named instance of its variable font.
        let keys = FaceKeys { families: keys.families.clone(), styles, legacy: vec![], full_names: vec![], postscript };
        let variations = tags.iter().zip(&coords).map(|(t, v)| (t.to_be_bytes(), *v)).collect();
        out.push(FaceStyle {
            family: family.to_string(),
            native_family: native_family.clone(),
            style,
            keys,
            variations,
            weight,
            italic,
            traits: FontTraits::default(),
        });
    }
    (out, renamed)
}

/// The most faces read from a (possibly damaged) collection.
const MAX_FACES: u32 = 256;

/// Parse every face in `data` (a font file or collection) and the named instances of its variable
/// ones. Returns each style with the index of its face.
fn enumerate_faces(data: &[u8]) -> Vec<(u32, FaceStyle)> {
    let count = match FileRef::new(data) {
        Ok(FileRef::Font(_)) => 1,
        Ok(FileRef::Collection(c)) => c.len().min(MAX_FACES),
        Err(_) => 0,
    };
    (0..count)
        .filter_map(|i| Some((i, skrifa::FontRef::from_index(data, i).ok()?)))
        .flat_map(|(i, f)| face_styles(&f).into_iter().map(move |s| (i, s)))
        .collect()
}

/// The font files held by the file at `path`: its contents, or for a suitcase font the fonts in
/// its resource fork ([`crate::suitcase`]). None when it can't be read.
#[cfg(not(target_arch = "wasm32"))]
fn font_files(path: &Path) -> Vec<Vec<u8>> {
    if crate::suitcase::is_suitcase(path) {
        return crate::suitcase::read_fonts(path);
    }
    std::fs::read(path).into_iter().collect()
}

/// The tables holding glyphs the font engine draws: TrueType, CFF, CFF2 and VARC outlines, and
/// `sbix` and `CBDT` colour bitmaps. A face with none of them (outlines in Apple's `hvgl` table,
/// monochrome bitmaps only) draws nothing.
#[cfg(not(target_arch = "wasm32"))]
const OUTLINE_TABLES: [&[u8; 4]; 6] = [b"glyf", b"CFF ", b"CFF2", b"VARC", b"sbix", b"CBDT"];

/// The styles of every face in the font file at `path` ([`face_styles`]), reading only its table
/// directories and `name`, `fvar` and `OS/2` tables: a scan opens hundreds of font files, many of
/// them megabytes long. Faces without outlines the font engine draws (no [`OUTLINE_TABLES`] table)
/// are left out.
#[cfg(not(target_arch = "wasm32"))]
fn file_face_names(path: &Path) -> Vec<FaceStyle> {
    file_face_names_within(path, MAX_FACES, u64::MAX)
}

/// [`file_face_names`], reading at most `max_faces` faces and `max_read` bytes of the file in all.
/// A suitcase font's resource fork is read whole when it is at most `max_read` bytes long, and the
/// limits then apply to each of its fonts; a longer fork gives none.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn file_face_names_within(path: &Path, max_faces: u32, max_read: u64) -> Vec<FaceStyle> {
    if crate::suitcase::is_suitcase(path) {
        if crate::suitcase::fork_len(path) > max_read {
            return vec![];
        }
        return crate::suitcase::read_fonts(path)
            .into_iter()
            .flat_map(|font| reader_face_names_within(std::io::Cursor::new(font), max_faces, max_read))
            .collect();
    }
    match std::fs::File::open(path) {
        Ok(file) => reader_face_names_within(file, max_faces, max_read),
        Err(_) => vec![],
    }
}

/// [`file_face_names_within`] for the font file `file` reads.
#[cfg(not(target_arch = "wasm32"))]
fn reader_face_names_within(mut file: impl std::io::Read + std::io::Seek, max_faces: u32, max_read: u64) -> Vec<FaceStyle> {
    use std::io::SeekFrom;
    /// The largest table read from a (possibly damaged) file.
    const MAX_NAME_TABLE: u32 = 1 << 20;
    let mut read = 0u64;
    let mut read_at = |offset: u64, len: usize| -> Option<Vec<u8>> {
        read = read.checked_add(u64::try_from(len).ok()?).filter(|r| *r <= max_read)?;
        let mut buf = vec![0; len];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut buf).ok()?;
        Some(buf)
    };
    let be32 = |b: &[u8], at: usize| b.get(at..at + 4).and_then(|s| s.try_into().ok()).map(u32::from_be_bytes);
    let Some(head) = read_at(0, 12) else { return vec![] };
    // A collection lists where each face's table directory starts.
    let starts: Vec<u32> = if head.starts_with(b"ttcf") {
        let n = be32(&head, 8).unwrap_or(0).min(max_faces) as usize;
        read_at(12, n * 4).map(|b| b.as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes(*c)).collect()).unwrap_or_default()
    } else {
        vec![0]
    };
    starts
        .into_iter()
        .filter_map(|start| {
            let dir = read_at(start.into(), 12)?;
            let tables = u16::from_be_bytes(dir.get(4..6)?.try_into().ok()?) as usize;
            let records = read_at(u64::from(start) + 12, tables * 16)?;
            if !records.as_chunks::<16>().0.iter().any(|r| OUTLINE_TABLES.iter().any(|t| r.starts_with(*t))) {
                return None;
            }
            let mut table = |tag: &[u8; 4]| -> Option<Vec<u8>> {
                let rec: &[u8] = records.as_chunks::<16>().0.iter().find(|r| r.starts_with(tag))?;
                let (offset, len) = (be32(rec, 8)?, be32(rec, 12)?);
                if len > MAX_NAME_TABLE {
                    return None;
                }
                read_at(offset.into(), len as usize)
            };
            let name = table(b"name")?;
            // A variable font's named instances are styles too.
            let fvar = table(b"fvar");
            // Its OS/2 table (the font's classification, for the menus' filters), when it has one.
            let os2 = table(b"OS/2");
            // A font holding just these tables, to read them as the font itself would.
            let mut tables: Vec<(&[u8; 4], &[u8])> = vec![(b"name", &name)];
            if let Some(fvar) = &fvar {
                tables.push((b"fvar", fvar));
            }
            if let Some(os2) = &os2 {
                tables.push((b"OS/2", os2));
            }
            let font = sfnt_of(&tables)?;
            Some(face_styles(&skrifa::FontRef::new(&font).ok()?))
        })
        .flatten()
        .collect()
}

/// Whether the file at `path` is a font file by its extension (`.ttf`, `.otf`, `.ttc` or `.otc`,
/// in any case), as the font scan reads the files in a folder.
pub fn is_font_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["ttf", "otf", "ttc", "otc"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Whether the file at `path` is a suitcase font ([`crate::suitcase`]): on macOS, a file with
/// nothing in its data and its fonts in its resource fork. Never on the web.
pub fn is_suitcase(path: &Path) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::suitcase::is_suitcase(path)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        false
    }
}

/// A font a document names that isn't available, to look for in font files ([`WantedFonts`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WantedFont {
    pub family: String,
    pub style: String,
    /// When the family is installed but not the style ([`FontMatch::Style`]): the family of the
    /// face [`FontDb::resolve`] gives instead. `None` when the family is missing.
    pub installed: Option<String>,
}

/// Fonts to look for in font files: [`WantedFonts::provided_by`] tells which of them a file
/// provides.
#[derive(Debug, Default)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct WantedFonts {
    fonts: Vec<WantedFont>,
    /// The indexes of the fonts by every name a face could answer to them by: the family as given
    /// and the installed family (ASCII-lowercased), and the family normalized ([`norm`]).
    by_name: HashMap<String, Vec<usize>>,
}

impl WantedFonts {
    /// The most faces and bytes of one file a search reads.
    #[cfg(not(target_arch = "wasm32"))]
    const SEARCH_FACES: u32 = 64;
    #[cfg(not(target_arch = "wasm32"))]
    const SEARCH_READ: u64 = 8 << 20;

    pub fn new(fonts: &[WantedFont]) -> Self {
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, w) in fonts.iter().enumerate() {
            let mut names = vec![w.family.to_ascii_lowercase(), norm(&w.family)];
            names.extend(w.installed.as_ref().map(|c| c.to_ascii_lowercase()));
            names.sort();
            names.dedup();
            for n in names.into_iter().filter(|n| !n.is_empty()) {
                by_name.entry(n).or_default().push(i);
            }
        }
        Self { fonts: fonts.to_vec(), by_name }
    }

    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    /// The indexes of the fonts that the font file at `path` provides: copied into a folder the
    /// font scan reads, the file makes [`FontDb::resolve`] find each of them exactly, in one of its
    /// faces or a variable font's named instances. The file is read as the scan reads it (its
    /// table directories and its `name`, `fvar` and `OS/2` tables), at most 64 faces and 8 MB of
    /// it, when it is a font file by its extension ([`is_font_file`]) or a suitcase font whose
    /// resource fork is at most 8 MB long. Sorted; none on the web.
    pub fn provided_by(&self, path: &Path) -> Vec<usize> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !is_font_file(path) && !crate::suitcase::is_suitcase(path) {
                return Vec::new();
            }
            let faces = file_face_names_within(path, Self::SEARCH_FACES, Self::SEARCH_READ);
            self.provided_by_faces(&faces)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            Vec::new()
        }
    }

    /// [`Self::provided_by`] for the faces of a file.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn provided_by_faces(&self, faces: &[FaceStyle]) -> Vec<usize> {
        let mut candidates: Vec<usize> = Vec::new();
        for f in faces {
            let names = std::iter::once(f.family.to_ascii_lowercase())
                .chain(f.keys.families.iter().cloned())
                .chain(f.keys.legacy.iter().map(|(family, _)| family.clone()))
                .chain(f.keys.full_names.iter().cloned())
                .chain(f.keys.postscript.iter().cloned());
            for n in names {
                candidates.extend(self.by_name.get(&n).into_iter().flatten());
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates.retain(|i| self.fonts.get(*i).is_some_and(|w| provides(w, faces)));
        candidates
    }
}

/// Whether the faces of a font file, cataloged by a rescan, make [`FontDb::resolve`] find `w`
/// exactly in one of them: [`FontDb::canonical`] and [`FontDb::find`] over these faces, as they
/// would run once the file is cataloged.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn provides(w: &WantedFont, faces: &[FaceStyle]) -> bool {
    let sk = norm(&w.style);
    // A face of `family` with the style among its style names, as `find` looks for it once the
    // family's files are loaded.
    let styled = |family: &str| faces.iter().any(|f| f.family.eq_ignore_ascii_case(family) && f.keys.styles.contains(&sk));
    let named_as_given = faces.iter().any(|f| f.family.eq_ignore_ascii_case(&w.family));
    if let Some(installed) = &w.installed {
        // A family cataloged by the name as given is found by it.
        if named_as_given && !w.family.eq_ignore_ascii_case(installed) {
            return styled(&w.family);
        }
        // The installed family is loaded: a face of it is loaded from the file only when its own
        // style is the one asked for (`is_cataloged`).
        return faces.iter().any(|f| f.family.eq_ignore_ascii_case(installed) && norm(&f.style) == sk);
    }
    if named_as_given {
        return styled(&w.family);
    }
    let key = norm(&w.family);
    if key.is_empty() {
        return false;
    }
    // Another of a family's names keeps the style asked for.
    if let Some(f) = faces.iter().find(|f| f.keys.families.contains(&key)) {
        return styled(&f.family);
    }
    // A legacy family with its legacy style, a full name or a PostScript name names its face
    // whatever style is asked for.
    if faces.iter().any(|f| {
        f.keys.legacy.iter().any(|(family, style)| *family == key && *style == sk)
            || f.keys.full_names.contains(&key)
            || f.keys.postscript.contains(&key)
    }) {
        return true;
    }
    // A legacy family with another style: that style of the family.
    faces.iter().find(|f| f.keys.legacy.iter().any(|(family, _)| *family == key)).is_some_and(|f| styled(&f.family))
}

/// A font file holding `tables` (tag, data), sorted by tag as table directories are.
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) fn sfnt_of(tables: &[(&[u8; 4], &[u8])]) -> Option<Vec<u8>> {
    let mut tables = tables.to_vec();
    tables.sort_by_key(|t| *t.0);
    let n = u16::try_from(tables.len()).ok()?;
    let mut font = Vec::new();
    font.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    for v in [n, 0, 0, 0] {
        font.extend_from_slice(&v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        font.extend_from_slice(*tag);
        for v in [0, u32::try_from(offset).ok()?, u32::try_from(data.len()).ok()?] {
            font.extend_from_slice(&v.to_be_bytes());
        }
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    Some(font)
}

/// Lists the font files the platform's font service knows, for [`set_platform_font_files`].
pub type PlatformFontFiles = fn() -> Vec<String>;

/// What lists the platform's font files, set by the app ([`set_platform_font_files`]).
#[cfg(not(target_arch = "wasm32"))]
static PLATFORM_FONT_FILES: std::sync::OnceLock<PlatformFontFiles> = std::sync::OnceLock::new();

/// Have [`system_font_dirs`] add the font files `list` gives (full paths) outside the font
/// folders, asked again by each scan and each check for installed fonts
/// ([`FontDb::installed_fonts_changed`]). The desktop app lists DirectWrite's system font
/// collection on Windows: fonts a font service such as Adobe Fonts loads in place, from files
/// outside the font folders and unknown to the registry (#579). On macOS it lists the fonts
/// CoreText's font manager has available, which apps and font managers can register from any
/// folder. Only the first call counts.
pub fn set_platform_font_files(list: PlatformFontFiles) {
    #[cfg(not(target_arch = "wasm32"))]
    // A second call keeps the first lister, as documented.
    let _ = PLATFORM_FONT_FILES.set(list);
    #[cfg(target_arch = "wasm32")]
    let _ = list;
}

/// Folders the user adds to the font scan (Preferences › Type › Additional Fonts Folder, #683),
/// read with their subfolders after the platform's.
#[cfg(not(target_arch = "wasm32"))]
static USER_FONT_DIRS: std::sync::RwLock<Vec<PathBuf>> = std::sync::RwLock::new(Vec::new());

/// Have [`system_font_dirs`] also read `dirs` (#683), so the fonts in them are listed and used as
/// if installed. The next check for installed fonts ([`FontDb::installed_fonts_changed`]) then
/// reports the change, and a scan catalogs them.
pub fn set_user_font_dirs(dirs: Vec<PathBuf>) {
    set_user_font_dirs_confined(dirs, false);
}

/// [`set_user_font_dirs`]; `confined` when a caller confined to some folders (automation) chose
/// them: the scan then reads only what is inside them, not files or folders that links in them
/// lead to elsewhere, since the caller could plant such a link.
pub fn set_user_font_dirs_confined(dirs: Vec<PathBuf>, confined: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        *USER_FONT_DIRS.write().unwrap_or_else(std::sync::PoisonError::into_inner) = dirs;
        USER_FONT_DIRS_CONFINED.store(confined, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (dirs, confined);
}

/// Whether [`USER_FONT_DIRS`] were chosen by a confined caller ([`set_user_font_dirs_confined`]).
#[cfg(not(target_arch = "wasm32"))]
static USER_FONT_DIRS_CONFINED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The folders, as found on disk, that the scan reads only the inside of
/// ([`set_user_font_dirs_confined`]).
#[cfg(not(target_arch = "wasm32"))]
fn confined_font_dirs() -> Vec<PathBuf> {
    if !USER_FONT_DIRS_CONFINED.load(std::sync::atomic::Ordering::Relaxed) {
        return vec![];
    }
    user_font_dirs().iter().filter_map(|d| std::fs::canonicalize(d).ok()).collect()
}

/// The folders [`set_user_font_dirs`] added (none on wasm).
pub fn user_font_dirs() -> Vec<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    return USER_FONT_DIRS.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    #[cfg(target_arch = "wasm32")]
    Vec::new()
}

/// VectorCraft's own Fonts folder, set by the app ([`set_app_font_dir`]).
#[cfg(not(target_arch = "wasm32"))]
static APP_FONT_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Have [`system_font_dirs`] read `dir` with its subfolders: VectorCraft's own Fonts folder, which
/// `text.addFontFiles` copies font files into. The desktop app and `vectorcraft-cli` set it before
/// the first scan, as they install [`set_platform_font_files`]. A folder that doesn't exist yet is
/// read once it does. Only the first call counts. Ignored on wasm.
pub fn set_app_font_dir(dir: PathBuf) {
    #[cfg(not(target_arch = "wasm32"))]
    // A second call keeps the first folder, as documented.
    let _ = APP_FONT_DIR.set(dir);
    #[cfg(target_arch = "wasm32")]
    let _ = dir;
}

/// The folder [`set_app_font_dir`] set: none before that, and none on wasm.
pub fn app_font_dir() -> Option<&'static Path> {
    #[cfg(not(target_arch = "wasm32"))]
    return APP_FONT_DIR.get().map(PathBuf::as_path);
    #[cfg(target_arch = "wasm32")]
    None
}

/// The platform's font folders (the system's and the user's), VectorCraft's own Fonts folder
/// ([`set_app_font_dir`]), the font files known to the platform outside them (on Windows those
/// registered with it, and those [`set_platform_font_files`] lists) and the folders the user added
/// ([`set_user_font_dirs`]), scanned by [`FontDb::global`].
pub fn system_font_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_arch = "wasm32") {
        return dirs;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts", "/Network/Library/Fonts"].map(Into::into));
        if let Some(h) = &home {
            dirs.push(h.join("Library/Fonts"));
        }
        // Fonts macOS downloads on demand (Yu Gothic, Yu Mincho, Tsukushi Maru Gothic…): one
        // asset folder per font, under a catalog folder whose number changes with macOS.
        for e in std::fs::read_dir("/System/Library/AssetsV2").into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with("com_apple_MobileAsset_Font") {
                dirs.push(e.path());
            }
        }
    } else if cfg!(windows) {
        let root = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
        dirs.push(root.join("Fonts"));
        // Fonts installed for the current user only (what Install does without administrator rights).
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(l).join("Microsoft\\Windows\\Fonts"));
        }
        #[cfg(windows)]
        dirs.extend(registered_font_files(&dirs));
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(Into::into));
        // The XDG data folders' `fonts` (fontconfig's user folder; Flatpak, Snap, NixOS and Guix
        // list their own data folders), relative paths ignored as the XDG spec says.
        let xdg = |var| std::env::var_os(var).map(|v| std::env::split_paths(&v).filter(|p| p.is_absolute()).collect::<Vec<_>>()).unwrap_or_default();
        let data_home = xdg("XDG_DATA_HOME").into_iter().next().or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        dirs.extend(data_home.iter().cloned().chain(xdg("XDG_DATA_DIRS")).map(|d| d.join("fonts")));
        if let Some(h) = &home {
            dirs.push(h.join(".fonts"));
        }
        // In a Flatpak sandbox, the host's fonts (system, local and the user's).
        dirs.extend(["/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts"].map(Into::into));
        // The folders fontconfig is set up with outside those: packages add their own (TeX fonts
        // under `/usr/share/texmf/fonts`, #1529), and `fc-list` and other apps show their fonts.
        let config_home = xdg("XDG_CONFIG_HOME").into_iter().next().or_else(|| home.as_ref().map(|h| h.join(".config")));
        let places = crate::fontconfig::Places { home: home.clone(), data_home, config_home };
        for dir in crate::fontconfig::font_dirs(&crate::fontconfig::root(), &places) {
            if !dirs.iter().any(|known| dir.starts_with(known)) {
                dirs.push(dir);
            }
        }
    }
    dirs.extend(app_font_dir().map(Path::to_path_buf));
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(list) = PLATFORM_FONT_FILES.get() {
        let files = fonts_outside(list(), &dirs);
        dirs.extend(files);
    }
    dirs.extend(user_font_dirs());
    dirs
}

/// The registry key listing the fonts installed on Windows (for all users under the local
/// machine's key, for the current user under theirs): one value per face, holding its file's
/// name in the Windows font folder or its full path.
#[cfg(windows)]
const REGISTERED_FONTS: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Fonts";

/// The font files registered with Windows outside `font_dirs` (fonts installed as shortcuts to
/// where they are, or by programs into their own folders), which apps find by the registry.
#[cfg(windows)]
fn registered_font_files(font_dirs: &[PathBuf]) -> Vec<PathBuf> {
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};
    let values = [LOCAL_MACHINE, CURRENT_USER].into_iter().filter_map(|root| root.open(REGISTERED_FONTS).ok()).flat_map(|key| {
        // A value that isn't a string isn't a font file.
        key.values().map(|vs| vs.filter_map(|(_, v)| String::try_from(v).ok()).collect::<Vec<_>>()).unwrap_or_default()
    });
    fonts_outside(values, font_dirs)
}

/// The full paths among `registered` (font registrations' data) that aren't in `font_dirs`
/// (ignoring case, as Windows paths do), sorted and deduplicated. File names alone are files in
/// the Windows font folder, which is scanned anyway. The count is capped.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn fonts_outside(registered: impl IntoIterator<Item = String>, font_dirs: &[PathBuf]) -> Vec<PathBuf> {
    const MAX_REGISTERED: usize = 1 << 16;
    let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let dirs: Vec<PathBuf> = font_dirs.iter().map(|d| lower(d)).collect();
    let mut files: Vec<PathBuf> = registered
        .into_iter()
        .take(MAX_REGISTERED)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && !dirs.iter().any(|d| lower(p).starts_with(d)))
        .collect();
    files.sort();
    files.dedup();
    files
}

/// `face` at the axis settings `variations` (user units), as face `id`: its names, kind, italic
/// and file, and its weight the `wght` setting when there is one.
fn instanced(face: &FontFace, variations: Vec<Variation>, id: u32) -> Option<FontFace> {
    let weight = variations.iter().find(|(t, _)| t == b"wght").map_or(face.weight, |(_, v)| *v);
    let spec = FaceStyle {
        family: face.family.clone(),
        native_family: face.native_family.clone(),
        style: face.style.clone(),
        keys: face.keys.clone(),
        variations,
        weight: Some(weight),
        italic: face.italic,
        traits: face.traits,
    };
    let mut f = make_face(face.bytes.clone(), face.index, spec, face.path.clone())?;
    f.id = id;
    Some(f)
}

fn make_face(bytes: FontBytes, index: u32, spec: FaceStyle, path: Option<std::path::PathBuf>) -> Option<FontFace> {
    let data: &[u8] = match &bytes {
        FontBytes::Static(b) => b,
        FontBytes::Owned(v) => v.as_slice(),
    };
    let f = skrifa::FontRef::from_index(data, index).ok()?;
    let FaceStyle { family, native_family, style, keys, variations, weight, italic, traits } = spec;
    let version = name(&f, &[StringId::VERSION_STRING]).unwrap_or_default();
    // A named instance: outlines, metrics and advances at its axis settings.
    let location = if variations.is_empty() { Location::default() } else { f.axes().location(variations.iter().map(|(t, v)| (Tag::new(t), *v))) };
    let m = f.metrics(Size::unscaled(), &location);
    let a = f.attributes();
    let hb = harfrust::FontRef::from_index(data, index).ok()?;
    let shaper = harfrust::ShaperData::new(&hb);
    let instance =
        (!variations.is_empty()).then(|| harfrust::ShaperInstance::from_variations(&hb, variations.iter().map(|(t, v)| (harfrust::Tag::new(t), *v))));
    let upem = m.units_per_em.max(1) as f64;
    let x_height = m.x_height.map(|v| v as f64).filter(|v| *v > 0.0).unwrap_or(m.ascent as f64 * 0.5);
    let decoration = |d: Option<skrifa::metrics::Decoration>, fallback: (f64, f64)| {
        d.map(|d| (f64::from(d.offset), f64::from(d.thickness))).filter(|(o, t)| o.is_finite() && *t > 0.0 && t.is_finite()).unwrap_or(fallback)
    };
    let underline = decoration(m.underline, (-0.075 * upem, 0.05 * upem));
    let strikeout = decoration(m.strikeout, (x_height * 0.5 + 0.025 * upem, 0.05 * upem));
    let colour_bitmaps = crate::bitmap::has_colour_bitmaps(&f);
    let colour_layers = crate::colr::has_colour_layers(&f);
    let svg_glyphs = crate::svg_glyph::has_svg_glyphs(&f);
    Some(FontFace {
        keys,
        traits,
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        family,
        native_family,
        style,
        version,
        weight: weight.unwrap_or(a.weight.value()),
        italic: italic || !matches!(a.style, skrifa::attribute::Style::Normal),
        variations,
        location,
        instance,
        upem,
        ascent: m.ascent as f64,
        descent: -(m.descent as f64),
        cap_height: m.cap_height.map(|v| v as f64).filter(|v| *v > 0.0).unwrap_or(m.ascent as f64 * 0.72),
        x_height,
        underline,
        strikeout,
        shaper,
        bytes,
        index,
        path,
        ideographic_centre: std::sync::OnceLock::new(),
        icf_margins: std::sync::OnceLock::new(),
        colour_bitmaps,
        colour_layers,
        svg_glyphs,
    })
}

/// An axis of a BASE table: its default baseline values for Han ideographs (`hani`), else kana
/// (`kana`), else its first script, by tag, in font units.
fn base_values(axis: skrifa::raw::tables::base::Axis<'_>) -> Option<HashMap<[u8; 4], f64>> {
    let tags = axis.base_tag_list()?.ok()?;
    let list = axis.base_script_list().ok()?;
    let records = list.base_script_records();
    let record =
        [b"hani", b"kana"].into_iter().find_map(|t| records.iter().find(|r| r.base_script_tag() == Tag::new(t))).or_else(|| records.first())?;
    let values = record.base_script(list.offset_data()).ok()?.base_values()?.ok()?;
    let mut out = HashMap::new();
    for (tag, coord) in tags.baseline_tags().iter().zip(values.base_coords().iter()) {
        if let Ok(c) = coord {
            out.insert(tag.get().to_be_bytes(), f64::from(c.coordinate()));
        }
    }
    Some(out)
}

/// `s` lowercased, without anything but letters and digits.
fn norm_chars(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase)
}

fn norm(s: &str) -> String {
    norm_chars(s).collect()
}

/// Weight implied by a style name (400 = regular). Japanese faces number their weights W0–W9
/// (Hiragino, Morisawa…), W6 weighing 600 as its weight class does (#2873).
pub fn style_weight(style: &str) -> f32 {
    let numbered = style.split(|c: char| !c.is_ascii_alphanumeric()).find_map(|part| match part.as_bytes() {
        [b'W' | b'w', d @ b'0'..=b'9'] => Some(f32::from(d - b'0') * 100.0),
        _ => None,
    });
    if let Some(w) = numbered {
        return w.max(100.0);
    }
    let s = norm(style);
    const TABLE: &[(&str, f32)] = &[
        ("extralight", 200.0),
        ("ultralight", 200.0),
        ("semibold", 600.0),
        ("demibold", 600.0),
        ("extrabold", 800.0),
        ("ultrabold", 800.0),
        ("hairline", 100.0),
        ("thin", 100.0),
        ("light", 300.0),
        ("medium", 500.0),
        ("bold", 700.0),
        ("black", 900.0),
        ("heavy", 900.0),
    ];
    TABLE.iter().find(|(k, _)| s.contains(k)).map(|(_, w)| *w).unwrap_or(400.0)
}

fn style_italic(style: &str) -> bool {
    let s = norm(style);
    s.contains("italic") || s.contains("oblique") || s == "it"
}

/// How far `face` is from the style asked for: its weight difference, and a penalty for the wrong
/// slant. The penalty is less than the weight gap between Regular and Bold (and Bold and Black),
/// so Bold Italic asked of a family with Bold and Italic faces only gets the Bold (#2491).
fn style_distance(face: &FontFace, weight: f32, italic: bool) -> f32 {
    (face.weight - weight).abs() + if face.italic != italic { 250.0 } else { 0.0 }
}

/// [`FontDb::menu_family_list`], with the [`FontDb::family_list`] it was built from.
type MenuFamilies = (Arc<[String]>, Arc<[String]>);

impl FontDb {
    /// A database holding the bundled fonts, whose system font scan reads `font_dirs` (folders,
    /// read with their subfolders, and font files).
    pub fn with_font_dirs(font_dirs: Vec<PathBuf>) -> Self {
        Self::new(FontPaths::Fixed(font_dirs))
    }

    /// Same scan paths as [`FontDb::global`] (platform lister and font folders), for tests.
    #[cfg(test)]
    pub(crate) fn for_system_scan_test() -> Self {
        Self::new(FontPaths::System)
    }

    fn new(font_paths: FontPaths) -> Self {
        let mut faces = Vec::new();
        // Bundled fonts first, then explicitly supplied craft-fonts faces: Japanese (Mincho
        // first), Arabic, Thai, then remaining faces in manifest order. Existing script fallback
        // priorities stay ahead of the additional families users can now pick by name.
        let craft = crate::craft_fonts::document_fonts().into_iter().map(|f| f.bytes);
        for data in BUNDLED.iter().copied().chain(craft) {
            for (i, spec) in enumerate_faces(data) {
                if let Some(f) = make_face(FontBytes::Static(data), i, spec, None) {
                    faces.push(Arc::new(f));
                }
            }
        }
        Self {
            faces: RwLock::new(faces),
            outlines: Mutex::new(HashMap::new()),
            images: Mutex::new(GlyphCache::default()),
            colours: Mutex::new(GlyphCache::default()),
            catalog: RwLock::new(Catalog::new()),
            postscript: RwLock::new(PostScriptNames::new()),
            font_paths,
            aliases: RwLock::new(HashMap::new()),
            canonical_cache: Mutex::new(CanonicalNames::new()),
            #[cfg(not(target_arch = "wasm32"))]
            cataloged: std::sync::OnceLock::new(),
            #[cfg(not(target_arch = "wasm32"))]
            version_scanned: RwLock::new(std::collections::HashSet::new()),
            family_cache: Mutex::new(None),
            menu_cache: Mutex::new(None),
            generation: AtomicU64::new(0),
            #[cfg(not(target_arch = "wasm32"))]
            sys: Mutex::new(SysFallback { enabled: true, ..Default::default() }),
            #[cfg(not(target_arch = "wasm32"))]
            stamp: Mutex::new(None),
            instances: Mutex::new(Instances::default()),
        }
    }

    /// Enable or disable the lazy system-font fallback for characters the loaded fonts lack
    /// (native only; on by default).
    pub fn set_system_fallback(&self, on: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.sys.lock().unwrap_or_else(|e| e.into_inner()).enabled = on;
        }
        #[cfg(target_arch = "wasm32")]
        let _ = on;
    }

    /// Process-wide database: the bundled fonts, then the installed system fonts, cataloged the
    /// first time a lookup by family name needs them (or ahead of time by
    /// [`FontDb::scan_in_background`]).
    pub fn global() -> &'static FontDb {
        static DB: std::sync::OnceLock<FontDb> = std::sync::OnceLock::new();
        DB.get_or_init(|| FontDb::new(FontPaths::System))
    }

    fn read_faces(&self) -> std::sync::RwLockReadGuard<'_, Vec<Arc<FontFace>>> {
        self.faces.read().unwrap_or_else(|e| e.into_inner())
    }

    fn read_catalog(&self) -> std::sync::RwLockReadGuard<'_, Catalog> {
        self.ensure_catalog();
        self.catalog.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Fonts were added or the catalog changed: cached family lists and [`FontDb::canonical`]'s
    /// answers are stale.
    fn changed(&self) {
        *self.family_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.canonical_cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// A number that changes whenever the available fonts do (fonts added, system fonts
    /// rescanned): lists built from [`FontDb::family_list`] are current while it doesn't.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Family names available (loaded plus installed system fonts), sorted and deduplicated.
    pub fn families(&self) -> Vec<String> {
        self.family_list().to_vec()
    }

    /// [`FontDb::families`], shared: cheap to call every frame.
    pub fn family_list(&self) -> Arc<[String]> {
        let catalog = self.read_catalog();
        let mut cache = self.family_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(list) = cache.as_ref() {
            return list.clone();
        }
        let mut v: Vec<String> = self.read_faces().iter().map(|f| f.family.clone()).collect();
        v.extend(catalog.values().map(|c| c.name.clone()));
        v.sort_by_key(|a| a.to_lowercase());
        v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let list: Arc<[String]> = v.into();
        *cache = Some(list.clone());
        list
    }

    /// The families font menus and lists show: [`FontDb::family_list`] without the system's hidden
    /// families, whose names start with "." (macOS keeps ".SF NS", ".LastResort" and others for its
    /// own interface, and Mac apps don't list them). Those still resolve by name, for documents and
    /// fallbacks that name them. Cheap to call every frame.
    pub fn menu_family_list(&self) -> Arc<[String]> {
        let all = self.family_list();
        let mut cache = self.menu_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((from, list)) = cache.as_ref()
            && Arc::ptr_eq(from, &all)
        {
            return list.clone();
        }
        let list: Arc<[String]> = all.iter().filter(|f| !f.starts_with('.')).cloned().collect();
        *cache = Some((all, list.clone()));
        list
    }

    /// How a font menu labels `family`: the English name when `english` is on (Preferences › Type ›
    /// Show Font Names in English, the default), else the font's native-language name when it has
    /// one. The document and commands still use the English family ([`Self::canonical`]).
    pub fn family_display_name(&self, family: &str, english: bool) -> String {
        let (family, _) = self.canonical(family, "");
        if english {
            return family;
        }
        if let Some(native) = self.read_faces().iter().find(|f| f.family.eq_ignore_ascii_case(&family)).and_then(|f| f.native_family.clone()) {
            return native;
        }
        if let Some(native) = self.read_catalog().get(&family.to_ascii_lowercase()).and_then(|c| c.native.clone()) {
            return native;
        }
        family
    }

    /// Style names available for `family`: upright styles by weight, then italics.
    pub fn styles(&self, family: &str) -> Vec<String> {
        let (family, _) = self.canonical(family, "");
        let family = family.as_str();
        let mut v: Vec<(bool, f32, String)> = Vec::new();
        // Each style once, as its first loaded version describes it (versions can disagree on weight).
        for f in self.read_faces().iter().filter(|f| f.family.eq_ignore_ascii_case(family)) {
            if !v.iter().any(|(_, _, s)| s.eq_ignore_ascii_case(&f.style)) {
                v.push((f.italic, f.weight, f.style.clone()));
            }
        }
        if let Some(c) = self.read_catalog().get(&family.to_ascii_lowercase()) {
            for cf in &c.faces {
                if !v.iter().any(|(_, _, s)| s.eq_ignore_ascii_case(&cf.style)) {
                    v.push((cf.italic, cf.weight, cf.style.clone()));
                }
            }
        }
        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
        v.dedup_by(|a, b| a.2 == b.2);
        v.into_iter().map(|t| t.2).collect()
    }

    /// Add a user font (TTF/OTF/TTC bytes). Returns the number of faces added (0 if unparseable or
    /// every face was already present).
    pub fn add_font(&self, bytes: Vec<u8>) -> usize {
        self.add_font_from(bytes, None)
    }

    /// [`FontDb::add_font`] for the bytes of the file at `path` (when known).
    fn add_font_from(&self, bytes: Vec<u8>, path: Option<&Path>) -> usize {
        self.add_fonts([(bytes, path.map(Path::to_path_buf))])
    }

    /// Add the faces of `files` (each file's bytes, and the file when known) all at once: a lookup
    /// on another thread finds all of them or none, never a family with only some of its styles
    /// loaded, whose nearest style it would take instead (Arial Bold Italic, the first of Arial's
    /// files, for upright Regular text). Returns the number of faces added.
    fn add_fonts(&self, files: impl IntoIterator<Item = (Vec<u8>, Option<PathBuf>)>) -> usize {
        // Another version of a family and style is kept: a document may name it (the first one
        // loaded stays the one the family and style alone resolve to).
        let same = |f: &FontFace, family: &str, style: &str, version: &str| {
            f.family.eq_ignore_ascii_case(family) && f.style.eq_ignore_ascii_case(style) && f.version == version
        };
        let mut made: Vec<FontFace> = Vec::new();
        for (bytes, path) in files {
            let data = Arc::new(bytes);
            for (i, spec) in enumerate_faces(&data) {
                let version = skrifa::FontRef::from_index(&data, i).ok().and_then(|f| name(&f, &[StringId::VERSION_STRING])).unwrap_or_default();
                let present = |f: &FontFace| same(f, &spec.family, &spec.style, &version);
                if self.read_faces().iter().any(|f| present(f)) || made.iter().any(present) {
                    continue;
                }
                if let Some(f) = make_face(FontBytes::Owned(data.clone()), i, spec, path.clone()) {
                    made.push(f);
                }
            }
        }
        let mut faces = self.faces.write().unwrap_or_else(|e| e.into_inner());
        let before = faces.len();
        for f in made {
            // Another thread may have added the same face meanwhile.
            if !faces.iter().any(|g| same(g, &f.family, &f.style, &f.version)) {
                faces.push(Arc::new(f));
            }
        }
        let added = faces.len() - before;
        drop(faces);
        #[cfg(test)]
        faces_published();
        if added > 0 {
            self.changed();
        }
        added
    }

    /// Catalog the installed fonts unless that has been done: the first caller scans the font
    /// folders, any other waits for that scan to finish. A no-op on wasm.
    fn ensure_catalog(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.cataloged.get_or_init(|| {
            self.scan_font_dirs();
        });
    }

    /// Catalog the installed fonts on a background thread, so the first lookup by family name
    /// (opening a file, the font menus) doesn't wait for the scan. A no-op once they are
    /// cataloged, and on wasm.
    pub fn scan_in_background(&'static self) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.cataloged.get().is_none() {
            // A failed spawn leaves the scan to the first lookup that needs it.
            let _ = std::thread::Builder::new().name("font-scan".into()).spawn(move || self.ensure_catalog());
        }
    }

    /// Scan the font folders again (fonts installed or removed since), cataloging the faces
    /// found (native only). Font data is loaded when a cataloged family is first resolved. Faces
    /// loaded from files the scan no longer finds are dropped. Returns the number of faces
    /// cataloged.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_system_fonts(&self) -> usize {
        // The first scan, or a rescan once it is done (never both at once).
        let mut first = None;
        self.cataloged.get_or_init(|| first = Some(self.scan_font_dirs()));
        first.unwrap_or_else(|| self.scan_font_dirs())
    }

    /// Whether fonts were installed or removed since the last scan (a font folder or font file it
    /// read changed, or a new one is to be read), for [`FontDb::load_system_fonts`] to catalog
    /// them. Costs a look at each folder the scan read, not a scan. `false` before the first scan
    /// and on wasm.
    pub fn installed_fonts_changed(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let paths = self.font_paths.get();
            let stamp = self.stamp.lock().unwrap_or_else(|e| e.into_inner());
            stamp.as_ref().is_some_and(|s| s.paths != paths || s.modified.iter().any(|(p, t)| modified(p) != *t))
        }
        #[cfg(target_arch = "wasm32")]
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn scan_font_dirs(&self) -> usize {
        // Files may have been installed since: other versions are looked for again.
        self.version_scanned.write().unwrap_or_else(|e| e.into_inner()).clear();
        let mut catalog = Catalog::new();
        let mut aliases: HashMap<String, Vec<Alias>> = HashMap::new();
        let mut postscript = PostScriptNames::new();
        let mut n = 0;
        let paths = self.font_paths.get();
        let mut modified_at = Vec::new();
        // Each file with whether it was named by itself rather than found in a folder.
        let mut files = Vec::new();
        let mut stack = paths.clone();
        // Each folder once, however links lead back to it.
        let mut visited = std::collections::HashSet::new();
        let confined = confined_font_dirs();
        let inside_confined = |p: &Path| std::fs::canonicalize(p).is_ok_and(|r| confined.iter().any(|root| r.starts_with(root)));
        while let Some(d) = stack.pop() {
            if !visited.insert(std::fs::canonicalize(&d).unwrap_or_else(|_| d.clone())) {
                continue;
            }
            // In a confined folder, a link is followed only to somewhere inside one.
            let in_confined = !confined.is_empty() && inside_confined(&d);
            // Before listing it: a font installed meanwhile changes it after this.
            modified_at.push((d.clone(), modified(&d)));
            // Not a folder: a font file named by itself (or nothing, yet).
            let Ok(rd) = std::fs::read_dir(&d) else {
                files.push((d, true));
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if in_confined && e.file_type().is_ok_and(|t| t.is_symlink()) && !inside_confined(&p) {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else {
                    files.push((p, false));
                }
            }
        }
        for (p, named) in files {
            // A file named by itself is a font whatever its name (font services keep fonts in
            // files without an extension); one of a folder's only with a font's extension, or
            // a suitcase font (whose fonts are in its resource fork, whatever its name).
            if !named && !is_font_file(&p) && !crate::suitcase::is_suitcase(&p) {
                continue;
            }
            for FaceStyle { family, native_family, style, keys, weight, italic, traits, .. } in file_face_names(&p) {
                for (k, a) in Alias::of(&family, &style, &keys) {
                    aliases.entry(k).or_default().push(a);
                }
                // `keys.postscript` is normalized ([`norm`]), as `by_postscript_name` looks it up.
                for ps in &keys.postscript {
                    postscript.insert(ps.clone(), (family.clone(), style.clone()));
                }
                let entry = catalog.entry(family.to_ascii_lowercase()).or_default();
                let kind = entry.traits.get_or_insert(traits);
                kind.variable |= traits.variable;
                if entry.name.is_empty() {
                    entry.name = family;
                }
                if entry.native.is_none() {
                    entry.native = native_family;
                }
                let (weight, italic) = (weight.unwrap_or_else(|| style_weight(&style)), italic || style_italic(&style));
                entry.faces.push(CatalogFace { style, path: p.clone(), weight, italic });
                n += 1;
            }
        }
        log::debug!("cataloged {n} system font faces");
        let installed: std::collections::HashSet<PathBuf> = catalog.values().flat_map(|c| c.faces.iter().map(|cf| cf.path.clone())).collect();
        *self.stamp.lock().unwrap_or_else(|e| e.into_inner()) = Some(ScanStamp { paths, modified: modified_at });
        *self.catalog.write().unwrap_or_else(|e| e.into_inner()) = catalog;
        *self.aliases.write().unwrap_or_else(|e| e.into_inner()) = aliases;
        *self.postscript.write().unwrap_or_else(|e| e.into_inner()) = postscript;
        // Faces loaded from a file this scan did not catalog (the font was removed, or its folder
        // is no longer read) are dropped with their outlines. A family with no face left and no
        // cataloged file leaves the lists, and type set in it resolves to the fallback font. A face
        // without a file (bundled, or added as bytes) stays.
        let mut dropped = std::collections::HashSet::new();
        self.faces.write().unwrap_or_else(|e| e.into_inner()).retain(|f| {
            let keep = f.path.as_ref().is_none_or(|p| installed.contains(p));
            if !keep {
                dropped.insert(f.id);
            }
            keep
        });
        if !dropped.is_empty() {
            log::debug!("dropped {} font faces whose files are no longer installed", dropped.len());
            self.outlines.lock().unwrap_or_else(|e| e.into_inner()).retain(|(id, _), _| !dropped.contains(id));
        }
        // A font installed since may cover the characters no font did: they are looked for again.
        {
            let mut sys = self.sys.lock().unwrap_or_else(|e| e.into_inner());
            sys.scans = sys.scans.wrapping_add(1);
            sys.misses.clear();
        }
        self.changed();
        n
    }

    /// The family's own name and the style for a name given in a document: any of a family's
    /// names (`ヒラギノ角ゴシック` is Hiragino Sans), a legacy family name (`Hiragino Sans W6` with its
    /// legacy style names the W6 face), a full name (name ID 4) or a PostScript name
    /// (`HiraginoSans-W6`, as PDF and `.ai` files name fonts). Case and anything but letters and
    /// digits are ignored. An unknown name is returned unchanged.
    pub fn canonical(&self, family: &str, style: &str) -> (String, String) {
        if self.is_loaded(family) || self.read_catalog().contains_key(&family.to_ascii_lowercase()) {
            return (family.to_string(), style.to_string());
        }
        // The lock is held until the answer is kept, and [`FontDb::changed`] waits for it: an
        // answer found from the fonts before a change is cleared by that change.
        let mut cache = self.canonical_cache.lock().unwrap_or_else(|e| e.into_inner());
        let key = (family.to_string(), style.to_string());
        if let Some(answer) = cache.get(&key) {
            return answer.clone();
        }
        let answer = self.canonical_alias(family, style);
        if cache.len() >= CANONICAL_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, answer.clone());
        answer
    }

    /// Finds [`FontDb::canonical`]'s answer for a name that is neither a loaded nor an installed
    /// family among the loaded faces' and the installed fonts' other names.
    fn canonical_alias(&self, family: &str, style: &str) -> (String, String) {
        let key = norm(family);
        let mut found: Vec<Alias> =
            self.read_faces().iter().flat_map(|f| Alias::of(&f.family, &f.style, &f.keys)).filter(|(k, _)| *k == key).map(|(_, a)| a).collect();
        if let Some(v) = self.aliases.read().unwrap_or_else(|e| e.into_inner()).get(&key) {
            found.extend(v.iter().cloned());
        }
        let ns = norm(style);
        // A family and style split from a PostScript name by an importer that didn't have the font
        // (`Honoka-Mincho` → Honoka, Mincho): once it is installed, the face of that PostScript
        // name, which is its own family and style (ほのか明朝, Regular).
        if found.is_empty() && !ns.is_empty() {
            let joined = format!("{key}{ns}");
            let face = |a: &Alias| a.paired.is_none() && a.style.is_some();
            let ps =
                self.read_faces().iter().flat_map(|f| Alias::of(&f.family, &f.style, &f.keys)).find(|(k, a)| *k == joined && face(a)).map(|(_, a)| a);
            let ps =
                ps.or_else(|| self.aliases.read().unwrap_or_else(|e| e.into_inner()).get(&joined).and_then(|v| v.iter().find(|a| face(a)).cloned()));
            if let Some(a) = ps {
                return (a.family, a.style.unwrap_or_else(|| style.to_string()));
            }
        }
        // A family name keeps the style asked for; a legacy pair names its face; a full or
        // PostScript name its face whatever style is asked for.
        if let Some(a) = found.iter().find(|a| a.style.is_none()) {
            return (a.family.clone(), style.to_string());
        }
        if let Some(a) = found.iter().find(|a| a.paired.as_ref().is_some_and(|p| *p == ns) || a.paired.is_none()) {
            return (a.family.clone(), a.style.clone().unwrap_or_else(|| style.to_string()));
        }
        match found.first() {
            Some(a) => (a.family.clone(), style.to_string()),
            None => (family.to_string(), style.to_string()),
        }
    }

    /// The kind of `family` (whether it sets Japanese, its class), for font menu filters: from a
    /// loaded face, else from the system font scan. Variable when any of its faces, loaded or
    /// scanned, is: a family may install static files beside a variable one. Cheap: no font file
    /// is read, and a loaded family doesn't start the scan.
    pub fn family_traits(&self, family: &str) -> FontTraits {
        let loaded = {
            let faces = self.read_faces();
            let mut of_family = faces.iter().filter(|f| f.family.eq_ignore_ascii_case(family));
            of_family.next().map(|first| FontTraits { variable: first.traits.variable || of_family.any(|f| f.traits.variable), ..first.traits })
        };
        let scanned = |catalog: &Catalog| catalog.get(&family.to_ascii_lowercase()).and_then(|c| c.traits);
        match loaded {
            None => scanned(&self.read_catalog()).unwrap_or_default(),
            Some(traits) if traits.variable => traits,
            Some(traits) => {
                let variable = scanned(&self.catalog.read().unwrap_or_else(|e| e.into_inner())).is_some_and(|t| t.variable);
                FontTraits { variable, ..traits }
            }
        }
    }

    /// [`face`](Self::face) by any of the family's names ([`canonical`](Self::canonical)), and how
    /// it matched.
    pub fn resolve(&self, family: &str, style: &str) -> Option<(Arc<FontFace>, FontMatch)> {
        let (family, style) = self.canonical(family, style);
        let f = self.face(&family, &style)?;
        let m = if !f.family.eq_ignore_ascii_case(&family) {
            FontMatch::Missing
        } else if norm(&f.style) == norm(&style) || f.keys.styles.contains(&norm(&style)) {
            FontMatch::Exact
        } else {
            FontMatch::Style
        };
        Some((f, m))
    }

    /// The installed face whose PostScript name is `name` (`HiraginoSans-W3`; any case), loaded
    /// from the system fonts if need be. `None` when no installed font has the name.
    pub fn find_postscript(&self, name: &str) -> Option<Arc<FontFace>> {
        let key = norm(name);
        if key.is_empty() {
            return None;
        }
        let loaded = |db: &FontDb| db.read_faces().iter().find(|f| f.keys.postscript.contains(&key)).cloned();
        if let Some(f) = loaded(self) {
            return Some(f);
        }
        self.ensure_catalog();
        let face = self.aliases.read().unwrap_or_else(|e| e.into_inner()).get(&key).and_then(|v| {
            v.iter().find(|a| a.paired.is_none() && a.style.is_some()).map(|a| (a.family.clone(), a.style.clone().unwrap_or_default()))
        });
        let (family, style) = face?;
        self.face(&family, &style)?;
        loaded(self)
    }

    /// The (family, style) of the installed face whose PostScript name is `name` (ignoring ASCII
    /// case). Documents name fonts this way, and a family can hold hyphens
    /// ("Rounded-X-Mplus-1c-black" is Rounded-X M+ 1c, black), so the name can't be split.
    pub fn by_postscript_name(&self, name: &str) -> Option<(String, String)> {
        self.ensure_catalog();
        self.postscript.read().unwrap_or_else(|e| e.into_inner()).get(&norm(name)).cloned()
    }

    /// Load the files of the installed `family`, adding all their faces at once
    /// ([`Self::add_fonts`]). Returns whether the family is loaded now: by this call, or by another
    /// thread meanwhile.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_cataloged(&self, family: &str) -> bool {
        let mut paths: Vec<PathBuf> =
            self.read_catalog().get(&family.to_ascii_lowercase()).map(|c| c.faces.iter().map(|cf| cf.path.clone()).collect()).unwrap_or_default();
        paths.sort();
        paths.dedup();
        let files = paths.into_iter().flat_map(|p| font_files(&p).into_iter().map(move |data| (data, Some(p.clone()))));
        self.add_fonts(files) > 0 || self.is_loaded(family)
    }

    /// Resolve a family + style to a face, falling back to the closest style of the family, then to
    /// Source Sans 3 Regular. Installed system fonts are found by name whatever ran before. `None`
    /// only if no font at all is loaded (the bundled fonts failed to parse, or were left out with
    /// the `bundled-fonts` feature and none was added), in which case text has no glyphs.
    pub fn face(&self, family: &str, style: &str) -> Option<Arc<FontFace>> {
        let (family, style) = self.canonical(family, style);
        let (family, style) = (family.as_str(), style.as_str());
        let found = self.find(family, style);
        if found.as_ref().is_some_and(|f| norm(&f.style) == norm(style)) {
            return found;
        }
        // The family, or this style of it, is installed but not loaded yet. Looked for again
        // whether or not this call loaded it: another thread may have just done so (then this
        // load adds nothing, but the face is there).
        #[cfg(not(target_arch = "wasm32"))]
        if found.is_none() || self.is_cataloged(family, style) {
            self.load_cataloged(family);
            if let Some(f) = self.find(family, style) {
                return Some(f);
            }
        }
        found
            .or_else(|| self.find(FALLBACK_FAMILY, style))
            .or_else(|| self.find(FALLBACK_FAMILY, "Regular"))
            .or_else(|| self.read_faces().first().cloned())
    }

    /// [`FontDb::face`] in version `version` of the family and style (a [`FontFace::version`]), when
    /// that version is installed; otherwise (or without a version) the face [`FontDb::face`] gives.
    pub fn face_version(&self, family: &str, style: &str, version: Option<&str>) -> Option<Arc<FontFace>> {
        let face = self.face(family, style)?;
        let Some(version) = version.filter(|v| *v != face.version) else { return Some(face) };
        let same =
            |f: &&Arc<FontFace>| f.family.eq_ignore_ascii_case(&face.family) && f.style.eq_ignore_ascii_case(&face.style) && f.version == version;
        if let Some(f) = self.read_faces().iter().find(same) {
            return Some(f.clone());
        }
        // Not loaded: the family's installed files may hold it (the catalog doesn't list versions).
        self.load_family_files(&face.family);
        let found = self.read_faces().iter().find(same).cloned();
        Some(found.unwrap_or(face))
    }

    /// Every installed version of `face`'s family and style, `face` itself first.
    pub fn versions(&self, face: &Arc<FontFace>) -> Vec<Arc<FontFace>> {
        self.load_family_files(&face.family);
        let mut out = vec![face.clone()];
        out.extend(
            self.read_faces()
                .iter()
                .filter(|f| f.family.eq_ignore_ascii_case(&face.family) && f.style.eq_ignore_ascii_case(&face.style) && f.id != face.id)
                .cloned(),
        );
        out
    }

    /// Load the installed files of `family` once (other versions of faces already loaded from
    /// elsewhere); a no-op on wasm, where no fonts are installed.
    fn load_family_files(&self, family: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let key = family.to_ascii_lowercase();
            if self.version_scanned.write().unwrap_or_else(|e| e.into_inner()).insert(key) {
                self.load_cataloged(family);
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = family;
    }

    /// Is `style` of `family` among the installed fonts?
    #[cfg(not(target_arch = "wasm32"))]
    fn is_cataloged(&self, family: &str, style: &str) -> bool {
        let ns = norm(style);
        self.read_catalog().get(&family.to_ascii_lowercase()).is_some_and(|c| c.faces.iter().any(|cf| norm(&cf.style) == ns))
    }

    /// The loaded face with [`FontFace::id`] `id` (the face a laid-out glyph came from), an
    /// instanced face ([`Self::instance`]) included.
    pub fn face_by_id(&self, id: u32) -> Option<Arc<FontFace>> {
        let loaded = self.read_faces().iter().find(|f| f.id == id).cloned();
        loaded.or_else(|| self.instance_by_id(id))
    }

    /// The face of character style `st`: [`Self::face_version`] of its font, instanced at its
    /// Variable Font settings ([`Self::instance`]).
    pub fn style_face(&self, st: &vectorcraft_doc::CharStyle) -> Option<Arc<FontFace>> {
        let face = self.face_version(&st.font_family, &st.font_style, st.font_version.as_deref())?;
        Some(self.instance(&face, &st.font_axes))
    }

    /// `face` with the axis settings `axes` (by tag, in the axes' own units) in place of its own,
    /// each clamped to its axis's range: its outlines, metrics and advances taken there, and its
    /// file embedded instanced there. Tags the font has no axis for are left out. `face` itself
    /// when that changes no setting, and for a static face.
    pub fn instance(&self, face: &Arc<FontFace>, axes: &std::collections::BTreeMap<String, f64>) -> Arc<FontFace> {
        if axes.is_empty() {
            return face.clone();
        }
        let font_axes = face.axes();
        let mut variations = face.variations.clone();
        for (tag, value) in axes {
            let Some(axis) = font_axes.iter().find(|a| a.tag.as_slice() == tag.as_bytes()) else { continue };
            if !value.is_finite() {
                continue;
            }
            let v = (*value as f32).clamp(axis.min.min(axis.max), axis.max.max(axis.min));
            match variations.iter_mut().find(|(t, _)| *t == axis.tag) {
                Some(setting) => setting.1 = v,
                None => variations.push((axis.tag, v)),
            }
        }
        // Every axis at the setting it has in `face`: nothing to instance.
        let at = |vs: &[Variation], a: &VariationAxis| vs.iter().find(|(t, _)| *t == a.tag).map_or(a.default, |(_, v)| *v);
        if font_axes.iter().all(|a| at(&variations, a) == at(&face.variations, a)) {
            return face.clone();
        }
        variations.sort_by_key(|(t, _)| *t);
        let key = (face.id, variations.iter().map(|(t, v)| (*t, v.to_bits())).collect::<Vec<_>>());
        let mut inst = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        let id = match inst.ids.get(&key) {
            Some(id) => *id,
            None => {
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                inst.ids.insert(key, id);
                inst.recipes.insert(id, (face.id, variations.clone()));
                id
            }
        };
        if let Some(f) = inst.live.get(&id) {
            return f.clone();
        }
        let Some(built) = instanced(face, variations, id) else { return face.clone() };
        let built = Arc::new(built);
        if inst.live.len() >= MAX_LIVE_INSTANCES {
            inst.live.clear();
        }
        inst.live.insert(id, built.clone());
        built
    }

    /// The instanced face with id `id` ([`Self::instance`]), built again when it was dropped.
    fn instance_by_id(&self, id: u32) -> Option<Arc<FontFace>> {
        let (base, variations) = {
            let inst = self.instances.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(f) = inst.live.get(&id) {
                return Some(f.clone());
            }
            inst.recipes.get(&id).cloned()?
        };
        let base = self.read_faces().iter().find(|f| f.id == base).cloned()?;
        let built = Arc::new(instanced(&base, variations, id)?);
        let mut inst = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        if inst.live.len() >= MAX_LIVE_INSTANCES {
            inst.live.clear();
        }
        Some(inst.live.entry(id).or_insert(built).clone())
    }

    /// The available family `name` names, ignoring case and anything but letters and digits, as
    /// PostScript names write families ("MicrosoftYaHei" is Microsoft YaHei).
    pub fn find_family(&self, name: &str) -> Option<String> {
        self.family_list().iter().find(|f| norm_chars(f).eq(norm_chars(name))).cloned()
    }

    /// Is `family` available (loaded, or installed on the system)?
    pub fn has_family(&self, family: &str) -> bool {
        let (family, _) = self.canonical(family, "");
        self.is_loaded(&family) || self.read_catalog().contains_key(&family.to_ascii_lowercase())
    }

    pub(crate) fn is_loaded(&self, family: &str) -> bool {
        self.read_faces().iter().any(|f| f.family.eq_ignore_ascii_case(family))
    }

    pub(crate) fn find(&self, family: &str, style: &str) -> Option<Arc<FontFace>> {
        let faces = self.read_faces();
        let cands: Vec<&Arc<FontFace>> = faces.iter().filter(|f| f.family.eq_ignore_ascii_case(family)).collect();
        if cands.is_empty() {
            return None;
        }
        let ns = norm(style);
        if let Some(f) = cands.iter().find(|f| norm(&f.style) == ns) {
            return Some((*f).clone());
        }
        // Another of a face's style names: in another language, or a variable font's default
        // instance's name.
        if let Some(f) = cands.iter().find(|f| f.keys.styles.contains(&ns)) {
            return Some((*f).clone());
        }
        let (tw, ti) = (style_weight(style), style_italic(style));
        cands.iter().min_by(|a, b| style_distance(a, tw, ti).total_cmp(&style_distance(b, tw, ti))).map(|f| (*f).clone())
    }

    /// [`fallback_for_style`](Self::fallback_for_style) for upright Regular text.
    pub(crate) fn fallback_for(&self, c: char, exclude: u32) -> Option<Arc<FontFace>> {
        self.fallback_for_style(c, exclude, 400.0, false)
    }

    /// First family (fallback family first, then load order) with a face that covers `c`, in that
    /// family's face nearest `weight` and `italic`; on native, system fonts are loaded lazily the
    /// first time no loaded face covers a character (#2491).
    pub(crate) fn fallback_for_style(&self, c: char, exclude: u32, weight: f32, italic: bool) -> Option<Arc<FontFace>> {
        // Thai (Lao, Khmer, Myanmar) takes a font made for it, when one is installed, rather than
        // whichever face covering it happened to load first (a display face, say).
        let prefer = script_families(c);
        if let Some(f) = self.family_fallback(c, exclude, prefer, weight, italic) {
            return Some(f);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.load_families(c, prefer)
            && let Some(f) = self.family_fallback(c, exclude, prefer, weight, italic)
        {
            return Some(f);
        }
        if let Some(f) = self.loaded_fallback(c, exclude, weight, italic) {
            return Some(f);
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.system_fallback(c);
        // Another lookup may have loaded the face while this one searched. Loading no new
        // faces does not mean no face covers the character (as with `face`'s catalog lookup).
        self.loaded_fallback(c, exclude, weight, italic)
    }

    /// A face that covers `c`, for text drawn outside the canvas (the app's own UI): the loaded
    /// fonts first, then (native) an installed one, loaded on demand. Characters no font covers
    /// are remembered, so they are looked for once.
    pub fn face_covering(&self, c: char) -> Option<Arc<FontFace>> {
        self.fallback_for(c, 0)
    }

    /// The loaded face of the first of `families` that covers `c`, nearest `weight` and `italic`.
    fn family_fallback(&self, c: char, exclude: u32, families: &[&str], weight: f32, italic: bool) -> Option<Arc<FontFace>> {
        let faces = self.read_faces();
        families.iter().find_map(|fam| {
            faces
                .iter()
                .filter(|f| f.id != exclude && f.family.eq_ignore_ascii_case(fam) && f.covers(c))
                .min_by(|a, b| style_distance(a, weight, italic).total_cmp(&style_distance(b, weight, italic)))
                .cloned()
        })
    }

    /// Load the first of the installed `families` not loaded yet that covers `c` (with system
    /// fallback on). Returns whether one was loaded.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_families(&self, c: char, families: &[&str]) -> bool {
        if families.is_empty() || !self.sys.lock().unwrap_or_else(|e| e.into_inner()).enabled {
            return false;
        }
        families.iter().any(|fam| !self.is_loaded(fam) && self.load_cataloged(fam) && self.family_fallback(c, 0, &[fam], 400.0, false).is_some())
    }

    /// The first loaded family (the fallback family, then load order) with a face covering `c`,
    /// and in it the face nearest `weight` and `italic`.
    fn loaded_fallback(&self, c: char, exclude: u32, weight: f32, italic: bool) -> Option<Arc<FontFace>> {
        let faces = self.read_faces();
        let mut families: Vec<&str> = Vec::new();
        for f in faces.iter().filter(|f| f.id != exclude && f.covers(c)) {
            if !families.iter().any(|fam| fam.eq_ignore_ascii_case(&f.family)) {
                families.push(&f.family);
            }
        }
        // Stable: the fallback family first, the others in load order.
        families.sort_by_key(|fam| !fam.eq_ignore_ascii_case(FALLBACK_FAMILY));
        let fam = families.first()?;
        faces
            .iter()
            .filter(|f| f.id != exclude && f.family.eq_ignore_ascii_case(fam) && f.covers(c))
            .min_by(|a, b| style_distance(a, weight, italic).total_cmp(&style_distance(b, weight, italic)))
            .cloned()
    }

    /// Load a system font covering `c` (preferred fallback families first, then any cataloged
    /// file under 40 MB). Returns true if one was loaded. Misses are remembered.
    #[cfg(not(target_arch = "wasm32"))]
    fn system_fallback(&self, c: char) -> bool {
        if c.is_control() || c.is_whitespace() {
            return false;
        }
        {
            let sys = self.sys.lock().unwrap_or_else(|e| e.into_inner());
            if !sys.enabled || sys.misses.contains(&c) {
                return false;
            }
        }
        // The first scan done before its count is read, so the first miss is remembered too.
        self.ensure_catalog();
        let scans = self.sys.lock().unwrap_or_else(|e| e.into_inner()).scans;
        let covered = |db: &FontDb| db.read_faces().iter().any(|f| f.covers(c));
        for fam in SYSTEM_FALLBACKS {
            if !self.is_loaded(fam) && self.load_cataloged(fam) && covered(self) {
                return true;
            }
        }
        let mut paths: Vec<PathBuf> = self.read_catalog().values().flat_map(|c| c.faces.iter().map(|cf| cf.path.clone())).collect();
        paths.sort();
        paths.dedup();
        for p in paths {
            if std::fs::metadata(&p).map(|m| m.len() > 40 << 20).unwrap_or(true) {
                continue;
            }
            for data in font_files(&p) {
                let hit =
                    enumerate_faces(&data).iter().any(|(i, _)| skrifa::FontRef::from_index(&data, *i).is_ok_and(|f| f.charmap().map(c).is_some()));
                if hit && self.add_font_from(data, Some(&p)) > 0 && covered(self) {
                    return true;
                }
            }
        }
        let mut sys = self.sys.lock().unwrap_or_else(|e| e.into_inner());
        if sys.scans == scans {
            sys.misses.insert(c);
        }
        false
    }

    /// Glyph outline in font units, y-down (flipped), cached per (face, glyph).
    pub fn outline(&self, face: &FontFace, gid: u32) -> Arc<BezPath> {
        let key = (face.id, gid);
        if let Some(p) = self.outlines.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return p.clone();
        }
        let mut pen = FlipPen(BezPath::new());
        if let Some(f) = face.skrifa()
            && let Some(g) = f.outline_glyphs().get(GlyphId::new(gid))
        {
            let _ = g.draw(DrawSettings::unhinted(Size::unscaled(), face.location()), &mut pen);
        }
        let p = Arc::new(pen.0);
        let mut cache = self.outlines.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= OUTLINE_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, p.clone());
        p
    }

    /// Glyph `gid`'s colour bitmap (`sbix`, `CBDT`) in `face`, cached per (face, glyph).
    pub fn glyph_image(&self, face: &FontFace, gid: u32) -> Option<Arc<GlyphImage>> {
        if !face.colour_bitmaps {
            return None;
        }
        let key = (face.id, gid);
        if let Some(i) = self.images.lock().unwrap_or_else(|e| e.into_inner()).kept.get(&key) {
            return i.clone();
        }
        let image = face.skrifa().and_then(|f| crate::bitmap::glyph_image(&f, face.upem, gid)).map(Arc::new);
        self.images.lock().unwrap_or_else(|e| e.into_inner()).keep(key, image.clone());
        image
    }

    /// Is glyph `gid` of face `face_id` drawn from its SVG document (it has one, and no COLR
    /// layers, which come first)? Writers that draw a font's own glyphs can draw COLR and bitmap
    /// glyphs, not always SVG ones.
    pub fn is_svg_glyph(&self, face_id: u32, gid: u32) -> bool {
        let Some(face) = self.face_by_id(face_id) else { return false };
        face.skrifa().is_some_and(|f| svg_drawn(&face, &f, gid) && crate::svg_glyph::has_document(&f, gid))
    }

    /// Glyph `gid`'s colour content in `face`, from its COLR layers or else its SVG document, in
    /// the text's `foreground` (fill) and `stroke` colours where it uses them; cached per (face,
    /// glyph), and per text colours for the glyphs that paint with them.
    pub(crate) fn glyph_colour(&self, face: &FontFace, gid: u32, foreground: Color, stroke: Option<Color>) -> Option<ColourContent> {
        if !face.colour_layers && !face.svg_glyphs {
            return None;
        }
        let f = face.skrifa()?;
        // COLR first; SVG for glyphs without COLR layers (those whose layers don't read keep their
        // outlines, as in fonts without SVG).
        let svg = svg_drawn(face, &f, gid);
        // Kept without colours, or by the text's colours it uses: the fill, and for SVG art the
        // stroke too.
        let coloured = Some((foreground.key(), stroke.filter(|_| svg).map(|c| c.key())));
        {
            let cache = self.colours.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(c) = [None, coloured].iter().find_map(|k| cache.kept.get(&(face.id, gid, *k))) {
                return c.clone();
            }
        }
        let (content, used) = if svg {
            let (art, used) = crate::svg_glyph::svg_glyph(&f, gid, face.upem, foreground, stroke);
            (art.map(ColourContent::Art), used)
        } else {
            crate::colr::colour_glyph(&f, face.location(), face.upem, gid, foreground)
        };
        let key = (face.id, gid, if used { coloured } else { None });
        self.colours.lock().unwrap_or_else(|e| e.into_inner()).keep(key, content.clone());
        content
    }
}

/// Is glyph `gid` of `face` (as `font`) drawn from an SVG document, if it has one: the face has
/// SVG glyphs, and the glyph no COLR layers, which come first.
fn svg_drawn(face: &FontFace, font: &skrifa::FontRef<'_>, gid: u32) -> bool {
    face.svg_glyphs && !crate::colr::has_colour_glyph(font, gid)
}

/// A pen drawing outlines y down, as text space has them.
pub(crate) struct FlipPen(pub(crate) BezPath);

impl OutlinePen for FlipPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x as f64, -y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((x as f64, -y as f64));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to((cx0 as f64, -cy0 as f64), (x as f64, -y as f64));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to((cx0 as f64, -cy0 as f64), (cx1 as f64, -cy1 as f64), (x as f64, -y as f64));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    /// A one-table font holding a name table from (platform, language, name id, string) records:
    /// Windows Unicode (UTF-16) or Mac Roman (ASCII here).
    fn name_font(records: &[(u16, u16, u16, &str)]) -> Vec<u8> {
        let (mut head, mut strings) = (vec![], vec![]);
        let n = records.len() as u16;
        for v in [0, n, 6 + 12 * n] {
            head.extend_from_slice(&v.to_be_bytes());
        }
        for &(platform, lang, id, s) in records {
            let bytes: Vec<u8> = if platform == 3 { s.encode_utf16().flat_map(u16::to_be_bytes).collect() } else { s.as_bytes().to_vec() };
            let encoding = if platform == 3 { 1 } else { 0 };
            for v in [platform, encoding, lang, id, bytes.len() as u16, strings.len() as u16] {
                head.extend_from_slice(&v.to_be_bytes());
            }
            strings.extend(bytes);
        }
        head.extend(strings);
        let mut font = vec![0, 1, 0, 0, 0, 1, 0, 16, 0, 0, 0, 0];
        font.extend_from_slice(b"name");
        for v in [0u32, 28, head.len() as u32] {
            font.extend_from_slice(&v.to_be_bytes());
        }
        font.extend(head);
        font
    }

    #[test]
    fn a_collection_lists_at_most_max_faces() {
        let mut font = name_font(&[(3, 0x0409, 1, "Fam"), (3, 0x0409, 2, "Regular")]);
        let n = 1000u32;
        let base = 12 + 4 * n;
        let table = u32::from_be_bytes(font[20..24].try_into().unwrap()) + base;
        font[20..24].copy_from_slice(&table.to_be_bytes());
        let mut ttc = b"ttcf".to_vec();
        for v in [0x0001_0000, n] {
            ttc.extend_from_slice(&u32::to_be_bytes(v));
        }
        for _ in 0..n {
            ttc.extend_from_slice(&base.to_be_bytes());
        }
        ttc.extend(font);
        let faces = enumerate_faces(&ttc);
        assert!(!faces.is_empty());
        assert!(faces.len() <= MAX_FACES as usize, "{} faces", faces.len());
    }

    fn keys(records: &[(u16, u16, u16, &str)]) -> (String, String, FaceKeys) {
        let font = name_font(records);
        let f = skrifa::FontRef::new(&font).unwrap();
        let (family, style) = face_names(&f).unwrap();
        let k = FaceKeys::of(&f, &family, &style);
        (family, style, k)
    }

    const EN: u16 = 0x0409;
    const JA: u16 = 0x0411;

    /// Laid out like the name table of Hiragino Sans W3.
    const HIRAGINO_W3: &[(u16, u16, u16, &str)] = &[
        (1, 0, 1, "Hiragino Sans"),
        (1, 0, 2, "W3"),
        (3, EN, 1, "Hiragino Sans W3"),
        (3, EN, 2, "Regular"),
        (3, JA, 1, "ヒラギノ角ゴシック W3"),
        (3, JA, 2, "Regular"),
        (3, EN, 6, "HiraginoSans-W3"),
        (3, EN, 16, "Hiragino Sans"),
        (3, JA, 16, "ヒラギノ角ゴシック"),
        (3, EN, 17, "W3"),
        (3, JA, 17, "W3"),
    ];

    #[test]
    fn a_face_answers_to_its_names_in_every_language() {
        let (family, style, k) = keys(HIRAGINO_W3);
        assert_eq!((family.as_str(), style.as_str()), ("Hiragino Sans", "W3"));
        let aliases = Alias::of(&family, &style, &k);
        let find = |name: &str| aliases.iter().filter(|(n, _)| *n == norm(name)).map(|(_, a)| a.clone()).collect::<Vec<_>>();
        assert!(find("ヒラギノ 角ゴシック").iter().any(|a| a.family == "Hiragino Sans" && a.style.is_none()), "a family name in Japanese");
        let legacy = find("Hiragino Sans W3");
        assert!(legacy.iter().any(|a| a.style.as_deref() == Some("W3") && a.paired.as_deref() == Some("regular")), "{legacy:?}");
        assert!(!legacy.iter().any(|a| a.paired.as_deref() == Some("w3")), "Mac family + Windows style is no pair");
        assert!(find("HiraginoSans-W3").iter().any(|a| a.style.as_deref() == Some("W3") && a.paired.is_none()), "the PostScript name");
    }

    /// #394 Preferences › Type › Show Font Names in English: pick the non-English name-table label.
    #[test]
    fn pick_native_label_prefers_a_non_english_name_record() {
        assert_eq!(
            pick_native_label("Hiragino Sans", &[(Some("en".into()), "Hiragino Sans".into()), (Some("ja".into()), "ヒラギノ角ゴシック".into())])
                .as_deref(),
            Some("ヒラギノ角ゴシック")
        );
        assert!(pick_native_label("Source Sans 3", &[(Some("en".into()), "Source Sans 3".into())]).is_none());
        let font = name_font(HIRAGINO_W3);
        let f = skrifa::FontRef::new(&font).unwrap();
        let (family, _) = face_names(&f).unwrap();
        assert_eq!(family, "Hiragino Sans");
        assert_eq!(native_family_name(&f, &family).as_deref(), Some("ヒラギノ角ゴシック"));
    }

    #[test]
    fn without_typographic_names_the_legacy_ones_are_the_familys() {
        let (family, style, k) = keys(&[(3, EN, 1, "Example Serif"), (3, EN, 2, "Bold Italic"), (3, JA, 1, "例セリフ"), (3, JA, 2, "太字斜体")]);
        assert_eq!((family.as_str(), style.as_str()), ("Example Serif", "Bold Italic"));
        assert!(k.families.contains(&norm("例セリフ")) && k.styles.contains(&norm("太字斜体")) && k.legacy.is_empty());
    }

    #[test]
    fn resolve_says_how_a_font_matched() {
        let db = FontDb::with_font_dirs(vec![]);
        let m = |f: &str, s: &str| db.resolve(f, s).map(|(face, m)| (face.family.clone(), face.style.clone(), m)).unwrap();
        assert_eq!(m("Source Sans 3", "Semibold"), ("Source Sans 3".into(), "Semibold".into(), FontMatch::Exact));
        assert_eq!(m("Source Sans 3", "Black").2, FontMatch::Style);
        assert_eq!(m("No Such Family", "Regular"), (FALLBACK_FAMILY.into(), "Regular".into(), FontMatch::Missing));
        // A PostScript name names its face, whatever style is asked for (text opened from a PDF).
        assert_eq!(m("SourceSans3-Semibold", "Regular"), ("Source Sans 3".into(), "Semibold".into(), FontMatch::Exact));
        let semi = db.find_postscript("sourcesans3-semibold").unwrap();
        assert_eq!(semi.style, "Semibold");
        assert!(db.find_postscript("NoSuchFont-Regular").is_none());
        assert!(db.has_family("SourceSans3-Regular"));
    }

    /// A PDF names a font by PostScript name; when it isn't installed the importer splits the name
    /// (`Honoka-Mincho` → Honoka, Mincho). Once a font of that PostScript name is installed, under
    /// a family name of its own (ほのか明朝), the split pair finds it.
    #[test]
    fn a_family_and_style_split_from_a_postscript_name_find_its_face() {
        let db = FontDb::with_font_dirs(vec![]);
        db.ensure_catalog();
        // An installed face whose family name shares nothing with its PostScript name.
        let alias = Alias { family: "Source Sans 3".into(), style: Some("Semibold".into()), paired: None };
        db.aliases.write().unwrap().insert(norm("Honoka-Mincho"), vec![alias]);
        db.changed();
        let m = db.resolve("Honoka", "Mincho").map(|(face, m)| (face.family.clone(), face.style.clone(), m)).unwrap();
        assert_eq!(m, ("Source Sans 3".into(), "Semibold".into(), FontMatch::Exact));
        // Unknown names still fall back.
        assert_eq!(db.resolve("Honoka", "Gothic").unwrap().1, FontMatch::Missing);
    }

    /// The database keeps at most [`CANONICAL_CACHE_MAX`] answers, and names asked for past that
    /// many still get the same answers.
    #[test]
    fn the_answers_kept_for_names_are_capped() {
        let db = FontDb::with_font_dirs(vec![]);
        for i in 0..=CANONICAL_CACHE_MAX {
            let name = format!("No Such Family {i}");
            assert_eq!(db.canonical(&name, "Regular"), (name.clone(), "Regular".into()));
        }
        assert!(db.canonical_cache.lock().unwrap().len() <= CANONICAL_CACHE_MAX);
        assert_eq!(db.canonical("SourceSans3-Semibold", "Regular"), ("Source Sans 3".into(), "Semibold".into()));
    }
}

#[cfg(test)]
mod race_tests {
    use super::*;

    #[test]
    fn a_family_loaded_by_another_thread_meanwhile_is_still_found() {
        // Two threads ask for the same installed family at once: each must get it, not the
        // fallback (the second one's load finds every face already added).
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
        let db: &'static FontDb = Box::leak(Box::new(FontDb::with_font_dirs(vec![dir])));
        // Only the catalog knows Inter (the bundled faces are dropped).
        db.faces.write().unwrap().retain(|f| f.family != "Inter");
        let threads: Vec<_> = (0..8).map(|_| std::thread::spawn(move || db.face("Inter", "Regular").map(|f| f.family.clone()))).collect();
        for t in threads {
            assert_eq!(t.join().unwrap().as_deref(), Some("Inter"));
        }
    }
}

#[cfg(test)]
mod image_cache_tests {
    use super::*;

    #[test]
    fn the_glyph_image_cache_keeps_its_bytes_within_its_budget() {
        // Images as large as a font may make them: no test font has enough of those to fill the
        // budget, so they're made here.
        let image = || {
            let blob = vectorcraft_doc::ImageBlob::png(vec![0; crate::bitmap::MAX_PNG_BYTES]);
            Some(Arc::new(GlyphImage { blob, width: 1, height: 1, xf: kurbo::Affine::IDENTITY }))
        };
        let mut cache = GlyphCache::default();
        for gid in 0..20 {
            cache.keep((1, gid), image());
            assert!(cache.bytes <= IMAGE_CACHE_BYTES, "{} bytes after {} images", cache.bytes, gid + 1);
        }
        // The same glyph again, as when two threads look it up at once, counts once.
        cache.keep((1, 19), image());
        assert_eq!(cache.bytes, cache.kept.values().flatten().map(|i: &Arc<GlyphImage>| i.blob.bytes.len()).sum::<usize>());
        // A glyph without an image costs nothing, and is still kept.
        cache.keep((1, 99), None);
        assert!(cache.kept.contains_key(&(1, 99)));
    }
}
