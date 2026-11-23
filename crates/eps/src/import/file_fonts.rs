//! The fonts an EPS carries (embeds) for type whose fonts aren't available here, kept with the
//! document as PDF import keeps a PDF's (#4568): the glyphs its type is shown in, gathered as the
//! program runs, become one font per family and style ([`vectorcraft_text::build_file_font`]),
//! which draws that type as the file draws it until its text is edited
//! ([`vectorcraft_doc::FileFonts`]).
//!
//! Each glyph comes from the font program the file carries (a Type 1 font's charstrings, a Type 42
//! font's TrueType glyph): its outline and advance, for the character its code shows. The first
//! glyph of each character is kept; a string that uses a different glyph for a character is drawn
//! as outlines instead.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use vectorcraft_doc::{FILE_FONT_MIME, FileFont, FileFonts, ImageBlob, Node, NodeKind};
use vectorcraft_text::{FileFontInfo, FileGlyph, MAX_FILE_GLYPHS, build_file_font};

/// The most glyphs gathered from one file, in all its fonts (an outline is some hundreds of bytes).
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
    /// Each character's glyph, by its index in `glyphs`.
    chars: HashMap<char, usize>,
}

impl FileFontGlyphs {
    /// The font of `family` and `style` (made with `info` the first time): its index.
    pub fn font(&mut self, family: &str, style: &str, info: impl FnOnce() -> FileFontInfo) -> usize {
        if let Some(&i) = self.index.get(&(family.to_string(), style.to_string())) {
            return i;
        }
        let i = self.fonts.len();
        self.fonts.push(Gathered { family: family.to_string(), style: style.to_string(), info: info(), glyphs: vec![], chars: HashMap::new() });
        self.index.insert((family.to_string(), style.to_string()), i);
        i
    }

    /// Can `glyphs` all be drawn in font `font`: no character among them has a glyph there, or
    /// earlier in `glyphs`, that differs in outline or advance (a font keeps one glyph per
    /// character), and the font has room for the characters it doesn't have yet? Each glyph is
    /// compared with the one its character has, once, as PDF import does.
    pub fn fits(&self, font: usize, glyphs: &[FileGlyph]) -> bool {
        let Some(f) = self.fonts.get(font) else { return false };
        let mut new: HashMap<char, &FileGlyph> = HashMap::new();
        for g in glyphs {
            let kept = f.chars.get(&g.ch).and_then(|&k| f.glyphs.get(k)).or_else(|| new.get(&g.ch).copied());
            match kept {
                Some(k) if k.outline != g.outline || k.advance != g.advance => return false,
                Some(_) => {}
                None => {
                    new.insert(g.ch, g);
                }
            }
        }
        f.glyphs.len().saturating_add(new.len()) <= MAX_FILE_GLYPHS && self.count.saturating_add(new.len()) <= MAX_GLYPHS
    }

    /// Gather `glyph` for font `font`, unless it already has one for its character or is full.
    pub fn add(&mut self, font: usize, glyph: FileGlyph) {
        if self.count >= MAX_GLYPHS {
            return;
        }
        if let Some(f) = self.fonts.get_mut(font)
            && f.glyphs.len() < MAX_FILE_GLYPHS
            && !f.chars.contains_key(&glyph.ch)
        {
            f.chars.insert(glyph.ch, f.glyphs.len());
            f.glyphs.push(glyph);
            self.count += 1;
        }
    }

    /// The fonts built from the glyphs gathered, stored in `images` under their content keys:
    /// family and style → the font's key.
    pub fn keep(self, images: &mut BTreeMap<String, ImageBlob>) -> HashMap<(String, String), String> {
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

/// The type among `nodes` (and in their groups) shown in fonts the file carries ([`mark`]), in the
/// families and styles `kept` holds fonts for (family and style → the font's blob in `images`):
/// drawn in them while its text is the file's, and laid out again for its bounds. Type in a font
/// the file only names keeps the fallback font, also where its family and style are the same.
pub(crate) fn keep_file_fonts(nodes: &mut [Arc<Node>], kept: &HashMap<(String, String), String>, images: &BTreeMap<String, ImageBlob>) {
    if kept.is_empty() {
        return;
    }
    let db = vectorcraft_text::FontDb::global();
    for n in nodes.iter_mut() {
        if n.children().is_some_and(|c| !c.is_empty())
            && let Some(children) = Arc::make_mut(n).children_mut()
        {
            keep_file_fonts(children, kept, images);
        }
        let NodeKind::Text(t) = &n.kind else { continue };
        if t.file_fonts.is_none() {
            continue;
        }
        let mut fonts: Vec<FileFont> = vec![];
        for r in &t.runs {
            let (family, style) = (&r.style.font_family, &r.style.font_style);
            if let Some(key) = kept.get(&(family.clone(), style.clone()))
                && !fonts.iter().any(|f| f.family == *family && f.style == *style)
            {
                fonts.push(FileFont { family: family.clone(), style: style.clone(), key: key.clone() });
            }
        }
        if let NodeKind::Text(t) = &mut Arc::make_mut(n).kind {
            if fonts.is_empty() {
                t.file_fonts = None;
                continue;
            }
            t.file_fonts = Some(Box::new(FileFonts::new(fonts, &t.plain_text())));
            db.bind_file_fonts(t, |key| images.get(key));
            t.cached_bounds = Some(vectorcraft_text::layout(db, t).bounds);
        }
    }
}

/// Mark type `t` as shown in a font the file carries, so that [`keep_file_fonts`] gives it the font
/// kept for its family and style (the key is filled in then).
pub(crate) fn mark(t: &mut vectorcraft_doc::TextObject) {
    let fonts = t.runs.first().map(|r| FileFont { family: r.style.font_family.clone(), style: r.style.font_style.clone(), key: String::new() });
    t.file_fonts = Some(Box::new(FileFonts::new(fonts.into_iter().collect(), &t.plain_text())));
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    fn glyph(ch: char) -> FileGlyph {
        FileGlyph { ch, outline: kurbo::Rect::new(0.0, 0.0, 400.0, 500.0).to_path(0.1), advance: 600.0 }
    }

    /// A font full of glyphs takes no new character, so a string that needs one doesn't fit (it
    /// comes in as outlines rather than as type the kept font can't draw); its own characters
    /// still fit.
    #[test]
    fn a_full_font_takes_no_new_character() {
        let mut g = FileFontGlyphs::default();
        let f = g.font("F", "Regular", FileFontInfo::default);
        let chars: Vec<char> = (0x10000u32..).filter_map(char::from_u32).take(MAX_FILE_GLYPHS).collect();
        for &c in &chars {
            g.add(f, glyph(c));
        }
        assert!(g.fits(f, &[glyph(chars[0]), glyph(chars[1])]));
        assert!(!g.fits(f, &[glyph(chars[0]), glyph('A')]));
    }

    /// A long string of one character compares each glyph with that character's once, not with
    /// every one before it: a quarter of a million of them fit at once.
    #[test]
    fn a_long_string_of_one_character_fits_in_linear_time() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut g = FileFontGlyphs::default();
            let f = g.font("F", "Regular", FileFontInfo::default);
            tx.send(g.fits(f, &vec![glyph('A'); 250_000])).ok();
        });
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(20)), Ok(true));
    }
}
