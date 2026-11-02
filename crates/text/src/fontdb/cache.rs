use super::traits::FontTraits;
use super::{Arc, ColourContent, GlyphImage, HashMap, IMAGE_CACHE_BYTES, IMAGE_CACHE_MAX, PathBuf};

/// One installed family found by the system font scan: its name and the file of each style.
#[derive(Debug, Default)]
pub(super) struct CatalogFamily {
    pub(super) name: String,
    /// Non-English family name from the name table, when the scan found one.
    pub(super) native: Option<String>,
    pub(super) faces: Vec<CatalogFace>,
    /// The family's kind (from its first face found); variable when any face found is.
    pub(super) traits: Option<FontTraits>,
}

/// One style of an installed family: a face, or a named instance of a variable font.
#[derive(Debug)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) struct CatalogFace {
    pub(super) style: String,
    pub(super) path: PathBuf,
    /// Weight and italic as the style lists them (by its name, or a named instance's axes).
    pub(super) weight: f32,
    pub(super) italic: bool,
}

/// The installed fonts by ASCII-lowercased family name (lookups ignore ASCII case, as
/// [`FontDb::face`] does). Always empty on wasm, which has no system fonts.
pub(super) type Catalog = HashMap<String, CatalogFamily>;

/// The installed faces' (family, style) by normalized PostScript name ([`norm`]).
pub(super) type PostScriptNames = HashMap<String, (String, String)>;

/// Maps the family and style [`FontDb::canonical`] was asked for to its answer, for names that
/// are neither a loaded nor an installed family.
pub(super) type CanonicalNames = HashMap<(String, String), (String, String)>;

/// [`FontDb::glyph_image`]'s answers by face and glyph.
pub(super) type GlyphImages = GlyphCache<(u32, u32), Arc<GlyphImage>>;

/// [`FontDb::glyph_colour`]'s answers by face and glyph, and the text's fill and stroke colours for
/// glyphs that paint with them.
pub(super) type GlyphColours = GlyphCache<(u32, u32, Option<TextColours>), ColourContent>;

/// The text's fill and stroke colours, as a cache key ([`Color::key`]).
pub(super) type TextColours = ([u32; 5], Option<[u32; 5]>);

/// What a glyph's colour content (an image, layers) takes of memory, in bytes.
pub(super) trait Weight {
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
pub(super) struct GlyphCache<K, V> {
    pub(super) kept: HashMap<K, Option<V>>,
    pub(super) bytes: usize,
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
    pub(super) fn keep(&mut self, key: K, value: Option<V>) {
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
