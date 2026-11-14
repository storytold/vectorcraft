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

/// The licence flags (OS/2 `fsType`) of `a` and `b` that allow less: a restricted font beats one
/// that may be embedded whole, which beats one that may be embedded as a subset. A font without
/// flags reads as print and preview (4), as a font built from it does.
fn stricter(a: Option<u16>, b: Option<u16>) -> Option<u16> {
    use vectorcraft_text::embed::Embedding;
    let rank = |f: Option<u16>| match Embedding::from_fs_type(f.unwrap_or(4)) {
        Embedding::Subset => 0,
        Embedding::Whole => 1,
        Embedding::Forbidden => 2,
    };
    if rank(b) > rank(a) { b } else { a }
}

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
            // Two programs of one family and style share one font here: it takes the stricter
            // of their licence flags, so a restricted one's glyphs never read as permitted.
            if let Some(g) = self.fonts.get_mut(i) {
                g.info.fs_type = stricter(g.info.fs_type, info().fs_type);
            }
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

    /// The fonts built from gathered glyphs, stored under their content keys. Only text that
    /// actually references one admits a runtime face, before it is respaced.
    pub fn keep(self, images: &mut HashMap<String, ImageBlob>) -> HashMap<(String, String), String> {
        let mut kept = HashMap::new();
        for f in self.fonts {
            let Some(bytes) = build_file_font(&f.info, &f.glyphs) else { continue };
            let blob = ImageBlob::new(FILE_FONT_MIME, bytes);
            let base = blob.content_key();
            let mut key = base.clone();
            let mut suffix = 1;
            while images.get(&key).is_some_and(|held| held != &blob) {
                key = format!("{base}-{suffix}");
                suffix += 1;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Two programs of one family and style are one font here, and its flags are the stricter
    /// of theirs whichever came first (CodeRabbit, #4569).
    #[test]
    fn two_programs_of_one_font_keep_the_stricter_licence() {
        for (first, second, want) in [
            (Some(4), Some(2), Some(2)),
            (Some(2), Some(4), Some(2)),
            (None, Some(0x0200), Some(0x0200)),
            (Some(4), Some(0x0108), Some(0x0108)),
            (Some(0x0108), Some(2), Some(2)),
            (Some(8), Some(4), Some(8)),
            (None, None, None),
        ] {
            let mut g = FileFontGlyphs::default();
            let info = |fs_type| {
                move || FileFontInfo { family: "F".into(), style: "R".into(), postscript: "F-R".into(), ascent: 800.0, descent: -200.0, fs_type }
            };
            let a = g.font("F", "R", info(first));
            let b = g.font("F", "R", info(second));
            assert_eq!(a, b);
            assert_eq!(g.fonts[a].info.fs_type, want, "{first:?} then {second:?}");
        }
    }
}
