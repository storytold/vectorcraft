//! Fonts carried by exported documents (SVG `@font-face`): what a font's licence allows (its
//! OS/2 `fsType` flags) and font files holding only the characters a document uses.

use std::borrow::Cow;

use skrifa::MetadataProvider;
use skrifa::raw::TableProvider;
use skrifa::raw::types::Tag;

use crate::FontFace;

/// What a font's licence lets a document do with it (OS/2 `fsType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Embedding {
    /// The font may be embedded, as a subset.
    Subset,
    /// The font may be embedded whole only (no subsetting).
    Whole,
    /// Restricted licence, or bitmap embedding only: the outlines may not be embedded.
    Forbidden,
}

impl Embedding {
    /// The permission an OS/2 `fsType` value grants. Bits 0–3 are the usage permissions (none:
    /// installable; 2: restricted; 4: preview and print; 8: editable), the most permissive one
    /// winning when several are set; bit 8 forbids subsetting and bit 9 allows bitmaps only.
    pub fn from_fs_type(fs: u16) -> Self {
        let restricted = fs & 0x0002 != 0 && fs & 0x000c == 0;
        if restricted || fs & 0x0200 != 0 {
            Self::Forbidden
        } else if fs & 0x0100 != 0 {
            Self::Whole
        } else {
            Self::Subset
        }
    }
}

/// A font file a document carries.
#[derive(Clone, Debug)]
pub struct EmbeddedFont {
    /// An OpenType file of the one face (`cmap`, `OS/2` and the other tables a viewer needs).
    pub data: Vec<u8>,
    /// CFF outlines (`font/otf`) rather than TrueType ones (`font/ttf`).
    pub cff: bool,
    /// Only the glyphs of the characters asked for; false: the whole face, as its licence asks
    /// (or when it can't be subset).
    pub subset: bool,
}

const CMAP: Tag = Tag::new(b"cmap");
const OS2: Tag = Tag::new(b"OS/2");
const HEAD: Tag = Tag::new(b"head");

impl FontFace {
    /// What the face's licence allows (a face without an `OS/2` table states no restriction).
    pub fn embedding(&self) -> Embedding {
        self.skrifa().and_then(|f| f.os2().ok()).map_or(Embedding::Subset, |os2| Embedding::from_fs_type(os2.fs_type()))
    }

    /// The face as a font file to embed: the glyphs of `chars` alone when its licence allows
    /// subsetting, else the whole face. Ligatures, kerning and other layout features are left out
    /// of subsets (viewers draw each character's own glyph). A named instance of a variable font
    /// ([`FontFace::variations`]) is embedded as a static font of that instance (the variable file
    /// would show its default instance), CFF2 outlines becoming TrueType ones. `None` when the
    /// licence forbids embedding or the font can't be read.
    pub fn embed(&self, chars: &[char]) -> Option<EmbeddedFont> {
        let font = self.skrifa()?;
        let cff = [b"CFF ", b"CFF2"].iter().any(|t| font.table_data(Tag::new(t)).is_some());
        let whole = || Some(EmbeddedFont { data: sfnt(font.table_directory.sfnt_version(), tables(&font))?, cff, subset: false });
        let instance = !self.variations().is_empty();
        match self.embedding() {
            Embedding::Forbidden => None,
            // Every glyph of the instance, keeping their ids.
            Embedding::Whole if instance => {
                let all: Vec<char> = font.charmap().mappings().filter_map(|(c, _)| char::from_u32(c)).collect();
                subset(self, &font, &all, true).map(|(data, cff)| EmbeddedFont { data, cff, subset: false })
            }
            Embedding::Whole => whole(),
            Embedding::Subset => match subset(self, &font, chars, false) {
                Some((data, cff)) => Some(EmbeddedFont { data, cff, subset: true }),
                // Outlines the subsetter can't read: what the licence allows anyway (a named
                // instance can't be embedded whole, which would show the default instance).
                None if !instance => whole(),
                None => None,
            },
        }
    }
}

/// Every table of `font`, by tag.
fn tables<'a>(font: &skrifa::FontRef<'a>) -> Vec<(Tag, Cow<'a, [u8]>)> {
    font.table_directory.table_records().iter().filter_map(|r| font.table_data(r.tag()).map(|d| (r.tag(), Cow::Borrowed(d.as_bytes())))).collect()
}

/// `face` reduced to the glyphs of `chars` (every glyph, keeping their ids, when `all`), with a
/// `cmap` mapping them and the face's `OS/2`; a named instance instanced. Returns the font and
/// whether its outlines are CFF ones.
fn subset(face: &FontFace, font: &skrifa::FontRef<'_>, chars: &[char], all: bool) -> Option<(Vec<u8>, bool)> {
    let charmap = font.charmap();
    let mut remapper = subsetter::GlyphRemapper::new();
    if all {
        for gid in 0..font.maxp().ok()?.num_glyphs() {
            remapper.remap(gid);
        }
    }
    let mut map: Vec<(u32, u16)> = chars
        .iter()
        .filter_map(|c| {
            let gid = u16::try_from(charmap.map(*c)?.to_u32()).ok().filter(|g| *g != 0)?;
            Some((*c as u32, remapper.remap(gid)))
        })
        .collect();
    map.sort_unstable();
    map.dedup_by_key(|m| m.0);
    let data = if face.variations().is_empty() {
        subsetter::subset(face.data(), face.index(), &remapper).ok()?
    } else {
        let coords: Vec<(subsetter::Tag, f32)> = face.variations().iter().map(|(t, v)| (subsetter::Tag::new(t), *v)).collect();
        subsetter::subset_with_variations(face.data(), face.index(), &coords, &remapper).ok()?
    };
    let sub = skrifa::FontRef::new(&data).ok()?;
    let cff = [b"CFF ", b"CFF2"].iter().any(|t| sub.table_data(Tag::new(t)).is_some());
    let mut t: Vec<(Tag, Cow<'_, [u8]>)> = tables(&sub).into_iter().filter(|(tag, _)| *tag != CMAP && *tag != OS2).collect();
    t.push((CMAP, Cow::Owned(cmap(&map)?)));
    if let Some(os2) = font.table_data(OS2) {
        t.push((OS2, Cow::Owned(os2_for(os2.as_bytes(), &map))));
    }
    Some((sfnt(sub.table_directory.sfnt_version(), t)?, cff))
}

/// The face's `OS/2` table with its first and last character fitted to the subset's.
fn os2_for(os2: &[u8], map: &[(u32, u16)]) -> Vec<u8> {
    let mut out = os2.to_vec();
    let bmp = |c: u32| u16::try_from(c).unwrap_or(u16::MAX).to_be_bytes();
    if let (Some(first), Some(last), Some(slot)) = (map.first(), map.last(), out.get_mut(64..68)) {
        slot[..2].copy_from_slice(&bmp(first.0));
        slot[2..].copy_from_slice(&bmp(last.0));
    }
    out
}

/// Runs of consecutive characters mapped to consecutive glyphs: (first char, last char, first
/// glyph).
fn runs(map: &[(u32, u16)]) -> Vec<(u32, u32, u16)> {
    let mut out: Vec<(u32, u32, u16)> = Vec::new();
    for &(c, g) in map {
        match out.last_mut() {
            Some((start, end, g0)) if c == *end + 1 && u32::from(g) == u32::from(*g0) + (c - *start) => *end = c,
            _ => out.push((c, c, g)),
        }
    }
    out
}

/// A `cmap` table mapping `map`'s characters (sorted, unique) to their glyphs: a format 4
/// subtable for the Basic Multilingual Plane and, when characters lie beyond it (or it would be
/// too long), a format 12 one for all of them. `None` when there is nothing to map.
pub(crate) fn cmap(map: &[(u32, u16)]) -> Option<Vec<u8>> {
    let all = runs(map);
    let last = all.last()?.1;
    let mut bmp: Vec<(u32, u32, u16)> = runs(&map.iter().copied().filter(|(c, _)| *c < 0xffff).collect::<Vec<_>>());
    // The closing segment every format 4 subtable ends with.
    bmp.push((0xffff, 0xffff, 0));
    let f4_len = 16 + 8 * bmp.len();
    let f4 = f4_len <= usize::from(u16::MAX);
    let f12 = !f4 || last >= 0xffff;
    let mut subtables: Vec<(u16, Vec<u8>)> = Vec::new();
    if f4 {
        let seg_x2 = u16::try_from(bmp.len() * 2).ok()?;
        let sel = bmp.len().ilog2() as u16;
        let range = 2u16.checked_pow(u32::from(sel))?.checked_mul(2)?;
        let mut s = Vec::with_capacity(f4_len);
        for v in [4, u16::try_from(f4_len).ok()?, 0, seg_x2, range, sel, seg_x2 - range] {
            s.extend(v.to_be_bytes());
        }
        let u = |c: u32| u16::try_from(c).unwrap_or(u16::MAX);
        s.extend(bmp.iter().flat_map(|(_, end, _)| u(*end).to_be_bytes()));
        s.extend(0u16.to_be_bytes());
        s.extend(bmp.iter().flat_map(|(start, _, _)| u(*start).to_be_bytes()));
        // The closing segment maps to glyph 0: 0xffff + 1.
        s.extend(bmp.iter().flat_map(|(start, end, g)| if *end == 0xffff { 1u16 } else { g.wrapping_sub(u(*start)) }.to_be_bytes()));
        s.extend(bmp.iter().flat_map(|_| 0u16.to_be_bytes()));
        subtables.push((1, s));
    }
    if f12 {
        let n = u32::try_from(all.len()).ok()?;
        let mut s = Vec::with_capacity(16 + 12 * all.len());
        s.extend(12u16.to_be_bytes());
        s.extend(0u16.to_be_bytes());
        for v in [16 + 12 * n, 0, n] {
            s.extend(v.to_be_bytes());
        }
        for (start, end, g) in &all {
            for v in [*start, *end, u32::from(*g)] {
                s.extend(v.to_be_bytes());
            }
        }
        subtables.push((10, s));
    }
    // Header and encoding records (Windows platform: 1 = BMP, 10 = full repertoire).
    let mut out = Vec::new();
    out.extend(0u16.to_be_bytes());
    out.extend(u16::try_from(subtables.len()).ok()?.to_be_bytes());
    let mut offset = 4 + 8 * subtables.len();
    for (encoding, s) in &subtables {
        out.extend(3u16.to_be_bytes());
        out.extend(encoding.to_be_bytes());
        out.extend(u32::try_from(offset).ok()?.to_be_bytes());
        offset += s.len();
    }
    for (_, s) in subtables {
        out.extend(s);
    }
    Some(out)
}

/// The sum of `data` as big-endian 32-bit words (zero-padded), as table directories record it.
fn checksum(data: &[u8]) -> u32 {
    let (words, rest) = data.as_chunks::<4>();
    let mut last = [0u8; 4];
    last[..rest.len()].copy_from_slice(rest);
    words.iter().chain([&last]).fold(0u32, |s, w| s.wrapping_add(u32::from_be_bytes(*w)))
}

/// The most bytes of a font file written.
const MAX_FONT_BYTES: usize = 256 << 20;

/// An OpenType file of `tables` (unique tags) with checksums and the `head` checksum adjustment.
fn sfnt(version: u32, mut tables: Vec<(Tag, Cow<'_, [u8]>)>) -> Option<Vec<u8>> {
    tables.sort_by_key(|t| t.0);
    tables.dedup_by_key(|t| t.0);
    let n = u16::try_from(tables.len()).ok().filter(|n| *n > 0)?;
    let sel = n.ilog2() as u16;
    let range = 2u16.checked_pow(u32::from(sel))?.checked_mul(16)?;
    // Tables may alias one range of the input: the file is no larger than a font file may be.
    let size = tables.iter().try_fold(12 + 16 * tables.len(), |sum, t| sum.checked_add(t.1.len().checked_add(3)?))?;
    if size > MAX_FONT_BYTES {
        return None;
    }
    let mut out = Vec::with_capacity(size);
    out.extend(version.to_be_bytes());
    for v in [n, range, sel, n.checked_mul(16)? - range] {
        out.extend(v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    let mut head_at = None;
    for (tag, data) in &mut tables {
        if *tag == HEAD {
            // The adjustment is summed as zero; it is set once the whole file is written.
            data.to_mut().get_mut(8..12)?.fill(0);
            head_at = Some(offset + 8);
        }
        out.extend(tag.to_be_bytes());
        out.extend(checksum(data).to_be_bytes());
        out.extend(u32::try_from(offset).ok()?.to_be_bytes());
        out.extend(u32::try_from(data.len()).ok()?.to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    if let Some(at) = head_at {
        let adjust = 0xb1b0_afba_u32.wrapping_sub(checksum(&out));
        out.get_mut(at..at + 4)?.copy_from_slice(&adjust.to_be_bytes());
    }
    Some(out)
}
