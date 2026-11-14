use std::sync::Arc;

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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileFonts {
    /// The font kept for each family and style the runs name.
    pub fonts: Vec<FileFont>,
    /// The text as the file had it ([`text_hash`] of the plain text).
    pub text: u64,
    /// Validated runtime resources; serialized keys alone never authorize a font lookup.
    #[serde(skip)]
    resources: Vec<(String, u32, std::sync::Weak<dyn FileFontFace>)>,
}

impl FileFonts {
    /// `fonts` for type whose text is `text`.
    pub fn new(fonts: Vec<FileFont>, text: &str) -> Self {
        Self { fonts, text: text_hash([text]), resources: vec![] }
    }

    /// The runtime face bound to this reference by its owning document.
    pub fn face(&self, key: &str) -> Option<Arc<dyn FileFontFace>> {
        self.resources.iter().find(|(k, _, _)| k == key).and_then(|(_, _, face)| face.upgrade())
    }

    /// Replace validated bindings, returning whether their runtime identity changed.
    pub fn bind(&mut self, resources: Vec<(String, Arc<dyn FileFontFace>)>) -> bool {
        let changed = self.resources.len() != resources.len()
            || self.resources.iter().zip(&resources).any(|((a, id, _), (b, face))| a != b || *id != face.id());
        self.resources = resources.into_iter().map(|(key, face)| (key, face.id(), Arc::downgrade(&face))).collect();
        changed
    }

    /// Remap serialized and validated runtime resource keys together when art changes namespaces.
    /// The callback applies the same mapping to each occurrence of an original key.
    pub fn map_keys(&mut self, mut map: impl FnMut(&mut String)) {
        for font in &mut self.fonts {
            map(&mut font.key);
        }
        for (key, _, _) in &mut self.resources {
            map(key);
        }
    }
}

impl PartialEq for FileFonts {
    fn eq(&self, other: &Self) -> bool {
        self.fonts == other.fonts
            && self.text == other.text
            && self.resources.len() == other.resources.len()
            && self.resources.iter().zip(&other.resources).all(|((a, aid, _), (b, bid, _))| a == b && aid == bid)
    }
}

/// A runtime file-font resource owned by image blobs, undo snapshots and clipboard resources.
/// The text engine implements this interface without making the document depend on it.
pub trait FileFontFace: Send + Sync {
    fn id(&self) -> u32;
    fn bytes(&self) -> &[u8];
}

impl std::fmt::Debug for dyn FileFontFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileFontFace").field("id", &self.id()).finish()
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
