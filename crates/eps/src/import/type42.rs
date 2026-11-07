//! Type 42 fonts' glyphs drawn from the TrueType font they carry, for the type a composite font
//! shows: its glyphs are picked by glyph index, often in a subset font without a character map,
//! so they can't be set again as type in an installed font. The font is the `sfnts` strings
//! joined, each string of odd length ending in a padding byte that isn't part of it (Adobe
//! Technical Note #5012, "The Type 42 Font Format Specification", on the `sfnts` array); a glyph
//! name maps to its glyph index through `CharStrings`.

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use std::cell::OnceCell;
use std::collections::HashMap;

use skrifa::{FontRef, GlyphId, MetadataProvider};
use vectorcraft_geom::BezPath;

use super::obj::Obj;

/// Most bytes of TrueType data a Type 42 font may carry.
pub(super) const MAX_FONT: usize = 32 << 20;

/// The TrueType font of a Type 42 font's `sfnts` strings, or `None` when an element isn't a
/// string or the font would be larger than [`MAX_FONT`].
pub(super) fn font_data(sfnts: &[Obj]) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    for s in sfnts {
        let Obj::Str(s) = s else { return None };
        let s = s.borrow();
        let len = s.len() - s.len() % 2;
        if data.len() + len > MAX_FONT {
            return None;
        }
        data.extend_from_slice(s.get(..len).unwrap_or_default());
    }
    Some(data)
}

/// The TrueType font a Type 42 font carries, with the character each glyph stands for (read
/// when first asked).
pub(super) struct Font {
    pub data: Vec<u8>,
    chars: OnceCell<HashMap<u32, char>>,
}

impl Font {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data, chars: OnceCell::new() }
    }

    /// The character glyph `gid` stands for: the one the font's character map maps to it
    /// (the lowest, when it maps several), else the one its `post` table's glyph name says
    /// (`uni20AC`, a single character). `None` for a glyph neither names, and for one a symbol
    /// font maps only from its private-use codes (U+F000–U+F0FF), which aren't characters.
    pub fn char_of(&self, gid: u32) -> Option<char> {
        let chars = self.chars.get_or_init(|| {
            let mut m = HashMap::new();
            if let Ok(font) = FontRef::new(&self.data) {
                for (c, g) in font.charmap().mappings() {
                    // Not a symbol font's private-use codes, controls or the soft hyphen, which
                    // doesn't show (a hyphen glyph mapped from it is the hyphen's).
                    let Some(c) = char::from_u32(c).filter(|c| !('\u{F000}'..='\u{F0FF}').contains(c) && !c.is_control() && *c != '\u{AD}') else {
                        continue;
                    };
                    m.entry(g.to_u32()).and_modify(|e: &mut char| *e = (*e).min(c)).or_insert(c);
                }
            }
            m
        });
        if let Some(c) = chars.get(&gid) {
            return Some(*c);
        }
        let font = FontRef::new(&self.data).ok()?;
        let name = font.glyph_names().get(GlyphId::new(gid))?;
        let c = super::text::glyph_char(name.as_str());
        (c != '\u{2022}' || name.as_str() == "bullet").then_some(c)
    }

    /// The glyph the font's character map gives `c`.
    pub fn gid_of(&self, c: char) -> Option<u32> {
        let font = FontRef::new(&self.data).ok()?;
        font.charmap().map(c).map(|g| g.to_u32()).filter(|g| *g != 0)
    }
}

/// Glyph `gid` of TrueType font `data`: its outline and advance width, in ems (y up), or `None`
/// when the font can't be read.
pub(super) fn glyph(data: &[u8], gid: u32) -> Option<(BezPath, f64)> {
    let font = FontRef::new(data).ok()?;
    let upem = f64::from(font.metrics(Size::unscaled(), LocationRef::default()).units_per_em);
    if upem <= 0.0 {
        return None;
    }
    let id = GlyphId::new(gid);
    let mut pen = Pen(BezPath::new(), 1.0 / upem);
    if gid != 0
        && let Some(g) = font.outline_glyphs().get(id)
    {
        // A glyph that can't be drawn draws nothing; its width still moves the next one.
        let _ = g.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen);
    }
    let advance = font.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(id).map_or(0.0, f64::from) / upem;
    Some((pen.0, if advance.is_finite() { advance } else { 0.0 }))
}

/// The advance width of glyph `gid` of TrueType font `data`, in ems (0 when it can't be read).
pub(super) fn advance(data: &[u8], gid: u32) -> f64 {
    let Ok(font) = FontRef::new(data) else { return 0.0 };
    let upem = f64::from(font.metrics(Size::unscaled(), LocationRef::default()).units_per_em);
    let w = font.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(GlyphId::new(gid)).map_or(0.0, f64::from) / upem;
    if w.is_finite() { w } else { 0.0 }
}

/// A pen drawing an outline scaled from font units to ems.
pub(super) struct Pen(pub BezPath, pub f64);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x) * self.1, f64::from(y) * self.1));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x) * self.1, f64::from(y) * self.1));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let s = self.1;
        self.0.quad_to((f64::from(cx0) * s, f64::from(cy0) * s), (f64::from(x) * s, f64::from(y) * s));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let s = self.1;
        self.0.curve_to((f64::from(cx0) * s, f64::from(cy0) * s), (f64::from(cx1) * s, f64::from(cy1) * s), (f64::from(x) * s, f64::from(y) * s));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}
