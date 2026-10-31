//! The fonts a PDF carries (embeds) for type whose fonts aren't available here: the glyphs it draws
//! in each, gathered as its pages are read, become one font per family and style, kept with the
//! document ([`vectorcraft_doc::FileFonts`]), that draws that type as the file did until its text
//! is edited. A file embeds its fonts so that it shows in them wherever it is read (Acrobat Help,
//! PDF fonts, <https://helpx.adobe.com/acrobat/using/pdf-fonts.html>).
//!
//! Each glyph comes as the interpreter draws it (an outline in thousandths of an em, whatever the
//! font program: Type 1, CFF or TrueType, simple or CID-keyed) with the width the file gives it, for
//! the character the file says it shows (its `ToUnicode` map, or what the import reads from the
//! font program). The first glyph of each character is kept: a font that draws one character with
//! two glyphs (a vertical form, an alternate) shows the first one for both.

use std::collections::{HashMap, HashSet};

use vectorcraft_doc::{FILE_FONT_MIME, ImageBlob};
use vectorcraft_text::{FileFontInfo, FileGlyph, MAX_FILE_GLYPHS, build_file_font};

/// The most glyphs gathered from one file, in all its fonts (an outline is some hundreds of bytes;
/// a CJK document uses a few thousand characters).
const MAX_GLYPHS: usize = 200_000;

/// The glyphs gathered so far.
#[derive(Default)]
pub(crate) struct FileFontGlyphs {
    fonts: Vec<Gathered>,
    /// Each font (family and style) by its index in `fonts`.
    index: HashMap<(String, String), usize>,
    /// Glyphs gathered, in all.
    count: usize,
}

/// The glyphs of one family and style.
struct Gathered {
    family: String,
    style: String,
    info: FileFontInfo,
    glyphs: Vec<FileGlyph>,
    chars: HashSet<char>,
}

impl FileFontGlyphs {
    /// The font of `family` and `style` (made with `info` the first time): its index.
    pub fn font(&mut self, family: &str, style: &str, info: impl FnOnce() -> FileFontInfo) -> usize {
        if let Some(&i) = self.index.get(&(family.to_string(), style.to_string())) {
            return i;
        }
        let i = self.fonts.len();
        self.fonts.push(Gathered { family: family.to_string(), style: style.to_string(), info: info(), glyphs: vec![], chars: HashSet::new() });
        self.index.insert((family.to_string(), style.to_string()), i);
        i
    }

    /// Does font `font` still want a glyph for `ch` (none gathered yet, and room for it)?
    pub fn wants(&self, font: usize, ch: char) -> bool {
        self.count < MAX_GLYPHS && self.fonts.get(font).is_some_and(|f| f.glyphs.len() < MAX_FILE_GLYPHS && !f.chars.contains(&ch))
    }

    /// Gather `glyph` for font `font`.
    pub fn add(&mut self, font: usize, glyph: FileGlyph) {
        if !self.wants(font, glyph.ch) {
            return;
        }
        if let Some(f) = self.fonts.get_mut(font) {
            f.chars.insert(glyph.ch);
            f.glyphs.push(glyph);
            self.count += 1;
        }
    }

    /// The fonts built from the glyphs gathered, among `images` (each under its content's key,
    /// MIME [`FILE_FONT_MIME`]) and kept by the font database for their type to be drawn in →
    /// each font's key by its family and style. Fonts that can't be built are left out: their type
    /// shows in the fallback font, as before.
    pub fn keep(self, images: &mut HashMap<String, ImageBlob>) -> HashMap<(String, String), String> {
        let db = vectorcraft_text::FontDb::global();
        let mut kept = HashMap::new();
        for f in self.fonts {
            let Some(bytes) = build_file_font(&f.info, &f.glyphs) else { continue };
            let blob = ImageBlob::new(FILE_FONT_MIME, bytes);
            let key = blob.content_key();
            if !db.keep_file_font(&key, blob.bytes.clone()) {
                continue;
            }
            images.entry(key.clone()).or_insert(blob);
            kept.insert((f.family, f.style), key);
        }
        kept
    }
}

/// What a font program the file embeds (`data`, a TrueType or OpenType file, or a bare CFF font)
/// says of the font's licence (OS/2 `fsType`) and its ascent and descent (thousandths of an em).
pub(crate) fn program_facts(data: &[u8], index: u32) -> (Option<u16>, Option<(f64, f64)>) {
    use skrifa::MetadataProvider;
    use skrifa::raw::TableProvider;
    let Ok(f) = skrifa::FontRef::from_index(data, index) else { return (None, None) };
    let fs_type = f.os2().ok().map(|o| o.fs_type());
    let m = f.metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    let k = 1000.0 / f64::from(m.units_per_em.max(1));
    let (ascent, descent) = (f64::from(m.ascent) * k, f64::from(m.descent) * k);
    let metrics = (ascent.is_finite() && descent.is_finite() && ascent > 0.0 && descent <= 0.0).then_some((ascent, descent));
    (fs_type, metrics)
}
