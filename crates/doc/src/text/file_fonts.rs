use serde::{Deserialize, Serialize};

/// The MIME type of a font a file carried ([`FileFonts`]), kept among the document's image blobs
/// ([`crate::Document::images`]).
pub const FILE_FONT_MIME: &str = "font/ttf";

/// The most fonts of [`FileFonts`] that draw one type object (a file's list may be longer: the rest
/// are left out).
pub const MAX_FILE_FONTS: usize = 64;

/// The fonts a file carried for type whose fonts aren't available here (a PDF's embedded fonts),
/// kept with the document among its image blobs ([`crate::Document::images`], MIME
/// [`FILE_FONT_MIME`]). A file embeds its fonts so that readers see its text in its own fonts
/// rather than substitutes (Acrobat Help, PDF fonts,
/// <https://helpx.adobe.com/acrobat/using/pdf-fonts.html>); editing text in a font that isn't
/// installed sets it in a substitute (Acrobat Help, Edit text in PDFs,
/// <https://helpx.adobe.com/acrobat/using/edit-text-pdfs1.html>). So the type is drawn in these,
/// as the file drew it, while its text is the one the file had and a font it names isn't
/// available: once its text is edited (or that font is installed) it takes the font it names, or
/// its substitute. Its runs keep naming the file's fonts, so they stay listed as missing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileFonts {
    /// The font kept for each family and style the runs name.
    pub fonts: Vec<FileFont>,
    /// The text as the file had it ([`text_hash`] of the plain text).
    pub text: u64,
}

impl FileFonts {
    /// `fonts` for type whose text is `text`.
    pub fn new(fonts: Vec<FileFont>, text: &str) -> Self {
        Self { fonts, text: text_hash([text]) }
    }
}

/// One font of [`FileFonts`]: the family and style type names it by, and its blob.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFont {
    pub family: String,
    pub style: String,
    /// The font's blob among the document's image blobs.
    pub key: String,
}

/// A hash of the text `parts` make one after another (64-bit FNV-1a of its UTF-8 bytes): the same
/// on every machine and in every version, as a saved [`FileFonts::text`] needs.
pub fn text_hash<'a>(parts: impl IntoIterator<Item = &'a str>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in parts.into_iter().flat_map(str::bytes) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}
