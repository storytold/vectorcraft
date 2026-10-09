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

/// The family used when a requested family is unknown (Illustrator's Myriad Pro analogue).
pub const FALLBACK_FAMILY: &str = "Source Sans 3";

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

/// An axis setting of a variable font: the axis tag (`wght`) and its value in user units (700).
pub type Variation = ([u8; 4], f32);

/// Caps on what a (possibly damaged) variable font can make the database read: axes per font and
/// named instances listed per face.
const MAX_AXES: usize = 64;
const MAX_INSTANCES: usize = 512;

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
    /// Typographic style name (e.g. "Semibold", "Italic").
    pub style: String,
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
}

/// One installed family found by the system font scan: its name and the file of each style.
#[derive(Debug, Default)]
struct CatalogFamily {
    name: String,
    faces: Vec<CatalogFace>,
    /// The family's kind (from its first face found).
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

/// Process-wide font database.
pub struct FontDb {
    faces: RwLock<Vec<Arc<FontFace>>>,
    outlines: Mutex<HashMap<(u32, u32), Arc<BezPath>>>,
    catalog: RwLock<Catalog>,
    /// The installed faces by PostScript name (documents name fonts so: PDF, EPS, .ai).
    postscript: RwLock<PostScriptNames>,
    /// Where the system font scan looks.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    font_paths: FontPaths,
    /// The installed fonts' other names (localized, legacy, PostScript) by normalized name.
    aliases: RwLock<HashMap<String, Vec<Alias>>>,
    /// Set once the font folders have been scanned. Lookups by family name wait for the first
    /// scan, so what they find never depends on what ran before them.
    #[cfg(not(target_arch = "wasm32"))]
    cataloged: std::sync::OnceLock<()>,
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

fn name(font: &skrifa::FontRef<'_>, ids: &[StringId]) -> Option<String> {
    ids.iter().find_map(|id| font.localized_strings(*id).english_or_first().map(|s| s.to_string()).filter(|s| !s.is_empty()))
}

/// Every localized string of `id`, with its language.
fn localized(font: &skrifa::FontRef<'_>, id: StringId) -> Vec<(Option<String>, String)> {
    font.localized_strings(id).map(|s| (s.language().map(str::to_owned), s.to_string())).filter(|(_, s)| !s.is_empty()).collect()
}

/// Every localized string of `id`.
fn all_names(font: &skrifa::FontRef<'_>, id: StringId) -> Vec<String> {
    localized(font, id).into_iter().map(|(_, s)| s).collect()
}

/// The names a face answers to besides its (English) family and style, normalized ([`norm`]):
/// its family in every language (typographic, else legacy: `ヒラギノ角ゴシック`), its style likewise
/// (`ミディアム`), its legacy family names (`Hiragino Sans W3`) with their legacy styles, paired per
/// language record (the Mac and Windows records of a face differ), and its PostScript names
/// (`HiraginoSans-W3`, as PDF and `.ai` files name fonts).
#[derive(Clone, Debug, Default)]
struct FaceKeys {
    families: Vec<String>,
    styles: Vec<String>,
    legacy: Vec<(String, String)>,
    postscript: Vec<String>,
}

impl FaceKeys {
    fn of(font: &skrifa::FontRef<'_>, family: &str, style: &str) -> Self {
        let (typo_family, typo_style) = (all_names(font, StringId::TYPOGRAPHIC_FAMILY_NAME), all_names(font, StringId::TYPOGRAPHIC_SUBFAMILY_NAME));
        let (legacy_family, legacy_style) = (localized(font, StringId::FAMILY_NAME), localized(font, StringId::SUBFAMILY_NAME));
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
        for v in [&mut families, &mut styles, &mut postscript] {
            v.retain(|k| !k.is_empty());
            v.sort();
            v.dedup();
        }
        legacy.sort();
        legacy.dedup();
        FaceKeys { families, styles, legacy, postscript }
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

/// What a font menu filters on: whether a family sets Japanese, and its kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontTraits {
    pub japanese: bool,
    pub class: FontClass,
}

impl FontTraits {
    /// From a face's OS/2 table (when it has one) and names.
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
        FontTraits { japanese, class }
    }
}

/// What a name other than a family's own one stands for ([`FontDb::canonical`]).
#[derive(Clone, Debug)]
struct Alias {
    /// The family's own name.
    family: String,
    /// The face the name names, for legacy family names and PostScript names.
    style: Option<String>,
    /// A legacy family name's legacy style (`Regular` beside `Hiragino Sans W3`).
    paired: Option<String>,
}

impl Alias {
    /// The aliases of a face `family` / `style` with `keys`, by normalized name.
    fn of(family: &str, style: &str, keys: &FaceKeys) -> Vec<(String, Alias)> {
        let mut v: Vec<(String, Alias)> =
            keys.families.iter().map(|k| (k.clone(), Alias { family: family.to_string(), style: None, paired: None })).collect();
        v.extend(
            keys.legacy
                .iter()
                .map(|(f, s)| (f.clone(), Alias { family: family.to_string(), style: Some(style.to_string()), paired: Some(s.clone()) })),
        );
        v.extend(keys.postscript.iter().map(|k| (k.clone(), Alias { family: family.to_string(), style: Some(style.to_string()), paired: None })));
        v
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
    let mut keys = FaceKeys::of(f, &family, &style);
    let traits = FontTraits::of(f, &keys);
    let mut instances = named_instances(f, &family, &style, &mut keys);
    for i in &mut instances {
        i.traits = traits;
    }
    let face = FaceStyle { family, style, keys, variations: vec![], weight: None, italic: false, traits };
    std::iter::once(face).chain(instances).collect()
}

/// The named instances of a variable font face (its `fvar` table) as styles of the face's family,
/// named by their subfamily names, without the default instance (the face itself) and without
/// names already taken. The default instance's name, when it isn't the face's style name (a face
/// `Regular` whose default instance is `Book`), names the face too (`keys`). Damaged tables give
/// fewer instances or none; the counts read are capped.
fn named_instances(f: &skrifa::FontRef<'_>, family: &str, style: &str, keys: &mut FaceKeys) -> Vec<FaceStyle> {
    let axes = f.axes();
    if axes.is_empty() || axes.len() > MAX_AXES {
        return vec![];
    }
    let tags: Vec<Tag> = axes.iter().map(|a| a.tag()).collect();
    let defaults: Vec<f32> = axes.iter().map(|a| a.default_value()).collect();
    // Instances without a PostScript name of their own are named `<prefix>-<style>` (Adobe
    // Technical Note #5902), as PDF and `.ai` files name them.
    let prefix = name(f, &[StringId::VARIATIONS_POSTSCRIPT_NAME_PREFIX]).unwrap_or_else(|| family.to_string());
    let mut taken = vec![norm(style)];
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
        let keys = FaceKeys { families: keys.families.clone(), styles, legacy: vec![], postscript };
        let variations = tags.iter().zip(&coords).map(|(t, v)| (t.to_be_bytes(), *v)).collect();
        out.push(FaceStyle { family: family.to_string(), style, keys, variations, weight, italic, traits: FontTraits::default() });
    }
    out
}

/// Parse every face in `data` (a font file or collection) and the named instances of its variable
/// ones. Returns each style with the index of its face.
fn enumerate_faces(data: &[u8]) -> Vec<(u32, FaceStyle)> {
    let count = match FileRef::new(data) {
        Ok(FileRef::Font(_)) => 1,
        Ok(FileRef::Collection(c)) => c.len(),
        Err(_) => 0,
    };
    (0..count)
        .filter_map(|i| Some((i, skrifa::FontRef::from_index(data, i).ok()?)))
        .flat_map(|(i, f)| face_styles(&f).into_iter().map(move |s| (i, s)))
        .collect()
}

/// The tables holding outlines the font engine draws: TrueType, CFF, CFF2 and VARC. A face with
/// none of them (outlines in Apple's `hvgl` table, bitmaps only) draws nothing.
#[cfg(not(target_arch = "wasm32"))]
const OUTLINE_TABLES: [&[u8; 4]; 4] = [b"glyf", b"CFF ", b"CFF2", b"VARC"];

/// The styles of every face in the font file at `path` ([`face_styles`]), reading only its table
/// directories and `name`, `fvar` and `OS/2` tables: a scan opens hundreds of font files, many of
/// them megabytes long. Faces without outlines the font engine draws (no [`OUTLINE_TABLES`] table)
/// are left out.
#[cfg(not(target_arch = "wasm32"))]
fn file_face_names(path: &Path) -> Vec<FaceStyle> {
    /// The most faces read from a (possibly damaged) collection.
    const MAX_FACES: u32 = 256;
    file_face_names_within(path, MAX_FACES, u64::MAX)
}

/// [`file_face_names`], reading at most `max_faces` faces and `max_read` bytes of the file in all.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn file_face_names_within(path: &Path, max_faces: u32, max_read: u64) -> Vec<FaceStyle> {
    use std::io::{Read, Seek, SeekFrom};
    /// The largest table read from a (possibly damaged) file.
    const MAX_NAME_TABLE: u32 = 1 << 20;
    let Ok(mut file) = std::fs::File::open(path) else { return vec![] };
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
    /// it. Sorted; none on the web.
    pub fn provided_by(&self, path: &Path) -> Vec<usize> {
        #[cfg(not(target_arch = "wasm32"))]
        {
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
    // A legacy family with its legacy style, or a PostScript name, names its face whatever style
    // is asked for.
    if faces.iter().any(|f| f.keys.legacy.iter().any(|(family, style)| *family == key && *style == sk) || f.keys.postscript.contains(&key)) {
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
    #[cfg(not(target_arch = "wasm32"))]
    {
        *USER_FONT_DIRS.write().unwrap_or_else(std::sync::PoisonError::into_inner) = dirs;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = dirs;
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
        dirs.extend(data_home.into_iter().chain(xdg("XDG_DATA_DIRS")).map(|d| d.join("fonts")));
        if let Some(h) = &home {
            dirs.push(h.join(".fonts"));
        }
        // In a Flatpak sandbox, the host's fonts (system, local and the user's).
        dirs.extend(["/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts"].map(Into::into));
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

fn make_face(bytes: FontBytes, index: u32, spec: FaceStyle, path: Option<std::path::PathBuf>) -> Option<FontFace> {
    let data: &[u8] = match &bytes {
        FontBytes::Static(b) => b,
        FontBytes::Owned(v) => v.as_slice(),
    };
    let f = skrifa::FontRef::from_index(data, index).ok()?;
    let FaceStyle { family, style, keys, variations, weight, italic, traits } = spec;
    // A named instance: outlines, metrics and advances at its axis settings.
    let location = if variations.is_empty() { Location::default() } else { f.axes().location(variations.iter().map(|(t, v)| (Tag::new(t), *v))) };
    let m = f.metrics(Size::unscaled(), &location);
    let a = f.attributes();
    let hb = harfrust::FontRef::from_index(data, index).ok()?;
    let shaper = harfrust::ShaperData::new(&hb);
    let instance =
        (!variations.is_empty()).then(|| harfrust::ShaperInstance::from_variations(&hb, variations.iter().map(|(t, v)| (harfrust::Tag::new(t), *v))));
    Some(FontFace {
        keys,
        traits,
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        family,
        style,
        weight: weight.unwrap_or(a.weight.value()),
        italic: italic || !matches!(a.style, skrifa::attribute::Style::Normal),
        variations,
        location,
        instance,
        upem: m.units_per_em.max(1) as f64,
        ascent: m.ascent as f64,
        descent: -(m.descent as f64),
        cap_height: m.cap_height.map(|v| v as f64).filter(|v| *v > 0.0).unwrap_or(m.ascent as f64 * 0.72),
        x_height: m.x_height.map(|v| v as f64).filter(|v| *v > 0.0).unwrap_or(m.ascent as f64 * 0.5),
        shaper,
        bytes,
        index,
        path,
        ideographic_centre: std::sync::OnceLock::new(),
        icf_margins: std::sync::OnceLock::new(),
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

/// Weight implied by a style name (400 = regular).
pub fn style_weight(style: &str) -> f32 {
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

/// [`FontDb::menu_family_list`], with the [`FontDb::family_list`] it was built from.
type MenuFamilies = (Arc<[String]>, Arc<[String]>);

impl FontDb {
    /// A database holding the bundled fonts, whose system font scan reads `font_dirs` (folders,
    /// read with their subfolders, and font files).
    pub fn with_font_dirs(font_dirs: Vec<PathBuf>) -> Self {
        Self::new(FontPaths::Fixed(font_dirs))
    }

    fn new(font_paths: FontPaths) -> Self {
        let mut faces = Vec::new();
        // The bundled fonts, then the Japanese craft-fonts faces (when built with them), Mincho
        // first: fallbacks for Japanese text after the requested and bundled fonts.
        let craft = crate::craft_fonts::japanese_document_fonts().into_iter().map(|f| f.bytes);
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
            catalog: RwLock::new(Catalog::new()),
            postscript: RwLock::new(PostScriptNames::new()),
            font_paths,
            aliases: RwLock::new(HashMap::new()),
            #[cfg(not(target_arch = "wasm32"))]
            cataloged: std::sync::OnceLock::new(),
            family_cache: Mutex::new(None),
            menu_cache: Mutex::new(None),
            generation: AtomicU64::new(0),
            #[cfg(not(target_arch = "wasm32"))]
            sys: Mutex::new(SysFallback { enabled: true, ..Default::default() }),
            #[cfg(not(target_arch = "wasm32"))]
            stamp: Mutex::new(None),
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

    /// Fonts were added or the catalog changed: cached family lists are stale.
    fn changed(&self) {
        *self.family_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
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

    /// Style names available for `family`: upright styles by weight, then italics.
    pub fn styles(&self, family: &str) -> Vec<String> {
        let (family, _) = self.canonical(family, "");
        let family = family.as_str();
        let mut v: Vec<(bool, f32, String)> =
            self.read_faces().iter().filter(|f| f.family.eq_ignore_ascii_case(family)).map(|f| (f.italic, f.weight, f.style.clone())).collect();
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
        let data = Arc::new(bytes);
        let mut added = 0;
        for (i, spec) in enumerate_faces(&data) {
            if self.read_faces().iter().any(|f| f.family.eq_ignore_ascii_case(&spec.family) && f.style.eq_ignore_ascii_case(&spec.style)) {
                continue;
            }
            if let Some(f) = make_face(FontBytes::Owned(data.clone()), i, spec, path.map(Path::to_path_buf)) {
                self.faces.write().unwrap_or_else(|e| e.into_inner()).push(Arc::new(f));
                added += 1;
            }
        }
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
    /// found (native only). Font data is loaded when a cataloged family is first resolved.
    /// Returns the number of faces cataloged.
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
        while let Some(d) = stack.pop() {
            if !visited.insert(std::fs::canonicalize(&d).unwrap_or_else(|_| d.clone())) {
                continue;
            }
            // Before listing it: a font installed meanwhile changes it after this.
            modified_at.push((d.clone(), modified(&d)));
            // Not a folder: a font file named by itself (or nothing, yet).
            let Ok(rd) = std::fs::read_dir(&d) else {
                files.push((d, true));
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    files.push((p, false));
                }
            }
        }
        for (p, named) in files {
            // A file named by itself is a font whatever its name (font services keep fonts in
            // files without an extension); one of a folder's only with a font's extension.
            if !named && !is_font_file(&p) {
                continue;
            }
            for FaceStyle { family, style, keys, weight, italic, traits, .. } in file_face_names(&p) {
                for (k, a) in Alias::of(&family, &style, &keys) {
                    aliases.entry(k).or_default().push(a);
                }
                // `keys.postscript` is normalized ([`norm`]), as `by_postscript_name` looks it up.
                for ps in &keys.postscript {
                    postscript.insert(ps.clone(), (family.clone(), style.clone()));
                }
                let entry = catalog.entry(family.to_ascii_lowercase()).or_default();
                entry.traits.get_or_insert(traits);
                if entry.name.is_empty() {
                    entry.name = family;
                }
                let (weight, italic) = (weight.unwrap_or_else(|| style_weight(&style)), italic || style_italic(&style));
                entry.faces.push(CatalogFace { style, path: p.clone(), weight, italic });
                n += 1;
            }
        }
        log::debug!("cataloged {n} system font faces");
        *self.stamp.lock().unwrap_or_else(|e| e.into_inner()) = Some(ScanStamp { paths, modified: modified_at });
        *self.catalog.write().unwrap_or_else(|e| e.into_inner()) = catalog;
        *self.aliases.write().unwrap_or_else(|e| e.into_inner()) = aliases;
        *self.postscript.write().unwrap_or_else(|e| e.into_inner()) = postscript;
        self.changed();
        n
    }

    /// The family's own name and the style for a name given in a document: any of a family's
    /// names (`ヒラギノ角ゴシック` is Hiragino Sans), a legacy family name (`Hiragino Sans W6` with its
    /// legacy style names the W6 face) or a PostScript name (`HiraginoSans-W6`, as PDF and `.ai`
    /// files name fonts). Case and anything but letters and digits are ignored. A name nothing
    /// answers to comes back as it is.
    pub fn canonical(&self, family: &str, style: &str) -> (String, String) {
        if self.is_loaded(family) || self.read_catalog().contains_key(&family.to_ascii_lowercase()) {
            return (family.to_string(), style.to_string());
        }
        let key = norm(family);
        let mut found: Vec<Alias> =
            self.read_faces().iter().flat_map(|f| Alias::of(&f.family, &f.style, &f.keys)).filter(|(k, _)| *k == key).map(|(_, a)| a).collect();
        if let Some(v) = self.aliases.read().unwrap_or_else(|e| e.into_inner()).get(&key) {
            found.extend(v.iter().cloned());
        }
        let ns = norm(style);
        // A family name keeps the style asked for; a legacy pair names its face; a PostScript name
        // its face whatever style is asked for.
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
    /// loaded face, else from the system font scan. Cheap: no font file is read.
    pub fn family_traits(&self, family: &str) -> FontTraits {
        if let Some(f) = self.read_faces().iter().find(|f| f.family.eq_ignore_ascii_case(family)) {
            return f.traits;
        }
        self.read_catalog().get(&family.to_ascii_lowercase()).and_then(|c| c.traits).unwrap_or_default()
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

    /// Load the files of the installed `family`. Returns whether any face was added.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_cataloged(&self, family: &str) -> bool {
        let mut paths: Vec<PathBuf> =
            self.read_catalog().get(&family.to_ascii_lowercase()).map(|c| c.faces.iter().map(|cf| cf.path.clone()).collect()).unwrap_or_default();
        paths.sort();
        paths.dedup();
        let mut any = false;
        for p in paths {
            if let Ok(data) = std::fs::read(&p) {
                any |= self.add_font_from(data, Some(&p)) > 0;
            }
        }
        any
    }

    /// Resolve a family + style to a face, falling back to the closest style of the family, then to
    /// Source Sans 3 Regular. Installed system fonts are found by name whatever ran before. `None`
    /// only if no font at all is loaded (the bundled fonts failed to parse), in which case text has
    /// no glyphs.
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

    /// Is `style` of `family` among the installed fonts?
    #[cfg(not(target_arch = "wasm32"))]
    fn is_cataloged(&self, family: &str, style: &str) -> bool {
        let ns = norm(style);
        self.read_catalog().get(&family.to_ascii_lowercase()).is_some_and(|c| c.faces.iter().any(|cf| norm(&cf.style) == ns))
    }

    /// The loaded face with [`FontFace::id`] `id` (the face a laid-out glyph came from).
    pub fn face_by_id(&self, id: u32) -> Option<Arc<FontFace>> {
        self.read_faces().iter().find(|f| f.id == id).cloned()
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
        cands
            .iter()
            .min_by(|a, b| {
                let sa = (a.weight - tw).abs() + if a.italic != ti { 1000.0 } else { 0.0 };
                let sb = (b.weight - tw).abs() + if b.italic != ti { 1000.0 } else { 0.0 };
                sa.total_cmp(&sb)
            })
            .map(|f| (*f).clone())
    }

    /// First face (fallback family first, then load order) that covers `c`; on native, system
    /// fonts are loaded lazily the first time no loaded face covers a character.
    pub(crate) fn fallback_for(&self, c: char, exclude: u32) -> Option<Arc<FontFace>> {
        if let Some(f) = self.loaded_fallback(c, exclude) {
            return Some(f);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.system_fallback(c) {
            return self.loaded_fallback(c, exclude);
        }
        None
    }

    /// A face that covers `c`, for text drawn outside the canvas (the app's own UI): the loaded
    /// fonts first, then (native) an installed one, loaded on demand. Characters no font covers
    /// are remembered, so they are looked for once.
    pub fn face_covering(&self, c: char) -> Option<Arc<FontFace>> {
        self.fallback_for(c, 0)
    }

    fn loaded_fallback(&self, c: char, exclude: u32) -> Option<Arc<FontFace>> {
        let faces = self.read_faces();
        let mut order: Vec<&Arc<FontFace>> = faces.iter().filter(|f| f.id != exclude).collect();
        order.sort_by_key(|f| (!f.family.eq_ignore_ascii_case(FALLBACK_FAMILY), f.italic, (f.weight - 400.0).abs() as i32));
        order.into_iter().find(|f| f.covers(c)).cloned()
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
            let Ok(data) = std::fs::read(&p) else { continue };
            let hit = enumerate_faces(&data).iter().any(|(i, _)| skrifa::FontRef::from_index(&data, *i).is_ok_and(|f| f.charmap().map(c).is_some()));
            if hit && self.add_font_from(data, Some(&p)) > 0 && covered(self) {
                return true;
            }
        }
        self.sys.lock().unwrap_or_else(|e| e.into_inner()).misses.insert(c);
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
}

struct FlipPen(BezPath);

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
