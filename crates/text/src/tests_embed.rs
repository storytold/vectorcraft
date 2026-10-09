//! Embedded fonts: licence flags and subsets a viewer can use.

use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::types::Tag;
use skrifa::raw::{FontData, FontRead, TableProvider};

use super::embed::Embedding;
use super::*;

#[test]
fn fs_type_flags_decide_the_embedding() {
    assert_eq!(Embedding::from_fs_type(0), Embedding::Subset, "installable");
    assert_eq!(Embedding::from_fs_type(0x0004), Embedding::Subset, "preview & print");
    assert_eq!(Embedding::from_fs_type(0x0008), Embedding::Subset, "editable");
    assert_eq!(Embedding::from_fs_type(0x0002), Embedding::Forbidden, "restricted");
    assert_eq!(Embedding::from_fs_type(0x0006), Embedding::Subset, "the most permissive bit wins");
    assert_eq!(Embedding::from_fs_type(0x0200), Embedding::Forbidden, "bitmaps only");
    assert_eq!(Embedding::from_fs_type(0x0108), Embedding::Whole, "no subsetting");
}

/// The bundled Source Sans 3 Regular renamed `family` (13 characters, as long as the original
/// name) with OS/2 `fsType` set to `fs_type`.
fn licensed_font(family: &str, fs_type: u16) -> Vec<u8> {
    let mut data = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf").to_vec();
    let table = |data: &[u8], tag: &[u8; 4]| {
        let f = skrifa::FontRef::new(data).unwrap();
        let r = f.table_directory.table_records().iter().find(|r| r.tag().to_be_bytes() == *tag).unwrap();
        (r.offset() as usize, r.length() as usize)
    };
    let (os2, _) = table(&data, b"OS/2");
    data[os2 + 8..os2 + 10].copy_from_slice(&fs_type.to_be_bytes());
    let (name, len) = table(&data, b"name");
    assert_eq!(family.len(), "Source Sans 3".len());
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16("Source Sans 3"), utf16(family)), (b"Source Sans 3".to_vec(), family.as_bytes().to_vec())] {
        let mut i = name;
        while let Some(at) = data[i..name + len].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    data
}

/// A font file goes with a document (File → Package) only when exports may embed it: not when
/// its license is restricted or allows bitmaps only.
#[test]
fn fonts_exports_may_not_embed_are_not_embeddable() {
    let db = FontDb::with_font_dirs(vec![]);
    for (family, fs_type, embeddable) in [("Bitmap Sans 3", 0x0200, false), ("Lowbit Sans 3", 0x0003, false), ("Wholed Sans 3", 0x0100, true)] {
        assert!(db.add_font(licensed_font(family, fs_type)) > 0, "{family}");
        let face = db.face(family, "Regular").unwrap();
        assert_eq!((face.family.as_str(), face.embeddable()), (family, embeddable), "fsType {fs_type:#06x}");
    }
}

#[test]
fn a_subset_holds_only_the_glyphs_used_and_maps_their_characters() {
    let face = FontDb::global().face("Source Sans 3", "Regular").unwrap();
    assert_eq!(face.embedding(), Embedding::Subset);
    let e = face.embed(&['H', 'e', 'l', 'l', 'o', ' ']).unwrap();
    assert!(e.subset && !e.cff);
    assert!(e.data.len() * 20 < face.data().len(), "{} of {} bytes", e.data.len(), face.data().len());
    let f = skrifa::FontRef::new(&e.data).unwrap();
    // .notdef and the five characters' glyphs.
    assert_eq!(f.maxp().unwrap().num_glyphs(), 6);
    let map = f.charmap();
    let metrics = f.glyph_metrics(Size::unscaled(), LocationRef::default());
    for c in ['H', 'e', 'l', 'o', ' '] {
        let g = map.map(c).unwrap_or_else(|| panic!("{c} mapped"));
        assert_eq!(metrics.advance_width(g).unwrap() as f64, face.advance(face.glyph_for(c)), "{c}: same advance");
    }
    assert!(map.map('x').is_none(), "unused characters are left out");
    let h = f.outline_glyphs().get(map.map('H').unwrap()).unwrap();
    let mut pen = Commands::default();
    h.draw(skrifa::outline::DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).unwrap();
    assert!(pen.0 > 4, "H keeps its outline");
    // The tables a web viewer needs, the licence flags kept and the file checksum right.
    for t in [b"cmap", b"OS/2", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"post", b"glyf", b"loca"] {
        assert!(f.table_data(Tag::new(t)).is_some(), "{}", String::from_utf8_lossy(t));
    }
    let os2 = f.os2().unwrap();
    assert_eq!(os2.fs_type(), face.skrifa().unwrap().os2().unwrap().fs_type());
    assert_eq!((os2.us_first_char_index(), os2.us_last_char_index()), (' ' as u16, 'o' as u16));
    let (words, rest) = e.data.as_chunks::<4>();
    assert!(rest.is_empty());
    assert_eq!(words.iter().fold(0u32, |s, w| s.wrapping_add(u32::from_be_bytes(*w))), 0xb1b0_afba);
}

#[test]
fn cmaps_map_runs_and_characters_beyond_the_basic_plane() {
    let face = FontDb::global().face("Inter", "Regular").unwrap();
    let e = face.embed(&['A', 'B', 'z']).unwrap();
    let f = skrifa::FontRef::new(&e.data).unwrap();
    let cmap = f.cmap().unwrap();
    let (a, b, z) = (cmap.map_codepoint('A').unwrap(), cmap.map_codepoint('B').unwrap(), cmap.map_codepoint('z').unwrap());
    assert!(a != b && b != z && a.to_u32() != 0);
    assert_eq!(cmap.map_codepoint('C'), None);
    // No bundled face maps characters beyond the BMP: the table is checked on its own.
    let raw = embed::cmap(&[(0x41, 1), (0x42, 2), (0x1f600, 3)]).unwrap();
    let cmap = skrifa::raw::tables::cmap::Cmap::read(FontData::new(&raw)).unwrap();
    assert_eq!(cmap.encoding_records().len(), 2, "format 4 and format 12");
    for (c, g) in [('A', 1), ('B', 2), ('\u{1f600}', 3)] {
        assert_eq!(cmap.map_codepoint(c).map(|g| g.to_u32()), Some(g), "{c}");
    }
    assert_eq!(cmap.map_codepoint('C'), None);
    assert!(embed::cmap(&[]).is_none());
}

/// Counts outline commands.
#[derive(Default)]
struct Commands(usize);

impl skrifa::outline::OutlinePen for Commands {
    fn move_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn line_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn close(&mut self) {}
}
