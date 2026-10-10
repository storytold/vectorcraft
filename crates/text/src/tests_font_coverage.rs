//! Original generated SFNT/cmap probes: compare against the complete FontRef's Charmap.
//! Layout reference: https://learn.microsoft.com/en-us/typography/opentype/spec/cmap
//! Collection reference: https://learn.microsoft.com/en-us/typography/opentype/spec/otff

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use skrifa::MetadataProvider;
use skrifa::raw::FileRef;

use crate::fontdb::{file_font_coverage, probe_font_coverage, sfnt_of};
use crate::{embed, test_fonts};

struct CountReads<'a> {
    inner: Cursor<&'a [u8]>,
    ranges: Vec<Range<u64>>,
    bytes: usize,
}

impl<'a> CountReads<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { inner: Cursor::new(data), ranges: vec![], bytes: 0 }
    }
}

impl Read for CountReads<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let start = self.inner.position();
        let n = self.inner.read(out)?;
        self.ranges.push(start..start + n as u64);
        self.bytes += n;
        Ok(n)
    }
}

impl Seek for CountReads<'_> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(from)
    }
}

fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_be_bytes(data[at..at + 2].try_into().unwrap())
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(data[at..at + 4].try_into().unwrap())
}

/// A complete, independently read original file, with names, maxp and an unused body table.
fn font(cmap: &[u8], body_len: usize) -> Vec<u8> {
    let name = test_fonts::name_table(&[(1, "Coverage Probe"), (2, "Regular")]).unwrap();
    let maxp = [0, 0, 0x50, 0, 0, 16]; // maxp 0.5, sixteen glyphs
    let body = vec![0xcd; body_len];
    sfnt_of(&[(b"cmap", cmap), (b"maxp", &maxp), (b"name", &name), (b"zzzz", &body)]).unwrap()
}

fn full_coverage(data: &[u8], c: char) -> bool {
    let faces = match FileRef::new(data).unwrap() {
        FileRef::Font(_) => 1,
        FileRef::Collection(c) => c.len(),
    };
    (0..faces).any(|i| skrifa::FontRef::from_index(data, i).unwrap().charmap().map(c).is_some())
}

fn full_glyph(data: &[u8], c: char) -> Option<u32> {
    skrifa::FontRef::new(data).unwrap().charmap().map(c).map(|g| g.to_u32())
}

fn probe(data: &[u8], c: char) -> (Option<bool>, CountReads<'_>) {
    let mut reader = CountReads::new(data);
    let result = probe_font_coverage(&mut reader, data.len() as u64, c);
    (result, reader)
}

fn assert_coverage(data: &[u8], cases: &[(char, Option<u32>)]) {
    for &(c, glyph) in cases {
        assert_eq!(full_glyph(data, c), glyph, "original full Charmap for {c:?}");
        assert_eq!(probe(data, c).0, Some(glyph.is_some()), "live compact probe for {c:?}");
    }
    // Check every original mapped scalar, in addition to the explicit independent glyph oracle.
    let original = skrifa::FontRef::new(data).unwrap();
    for (cp, glyph) in original.charmap().mappings().filter(|(_, g)| g.to_u32() != 0) {
        if let Some(c) = char::from_u32(cp) {
            assert_eq!(full_glyph(data, c), Some(glyph.to_u32()));
            assert_eq!(probe(data, c).0, Some(true), "mapped scalar {cp:x}");
        }
    }
}

fn encoding(platform: u16, id: u16, subtable: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for v in [0_u16, 1, platform, id] {
        out.extend(v.to_be_bytes());
    }
    out.extend(12_u32.to_be_bytes());
    out.extend(subtable);
    out
}

fn one_encoding(cmap: &[u8], id: u16) -> Vec<u8> {
    let record = (0..usize::from(u16_at(cmap, 2))).map(|i| 4 + i * 8).find(|&at| u16_at(cmap, at + 2) == id).unwrap();
    let start = u32_at(cmap, record + 4) as usize;
    let len = match u16_at(cmap, start) {
        4 => usize::from(u16_at(cmap, start + 2)),
        12 => u32_at(cmap, start + 4) as usize,
        other => panic!("generated subtable {other}"),
    };
    encoding(u16_at(cmap, record), id, &cmap[start..start + len])
}

fn two_encodings(first: &[u8], second: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 2];
    out.extend(&first[4..8]);
    out.extend(20_u32.to_be_bytes());
    out.extend(&second[4..8]);
    out.extend((20_u32 + first.len() as u32 - 12).to_be_bytes());
    out.extend(&first[12..]);
    out.extend(&second[12..]);
    out
}

/// Format 4 uses its glyphIdArray, rather than only idDelta segments.
fn indirect_format4() -> Vec<u8> {
    let words: [u16; 18] = [4, 36, 0, 4, 4, 1, 0, 0x42, 0xffff, 0, 0x41, 0xffff, 0, 1, 4, 0, 3, 9];
    encoding(3, 1, &words.into_iter().flat_map(u16::to_be_bytes).collect::<Vec<_>>())
}

/// Original collections use absolute table offsets, including all physical faces.
fn collection(faces: &[Vec<u8>], version: u32) -> Vec<u8> {
    let extra = if version == 0x0002_0000 { 12 } else { 0 };
    let mut out = b"ttcf".to_vec();
    out.extend(version.to_be_bytes());
    out.extend((faces.len() as u32).to_be_bytes());
    let mut start = 12 + 4 * faces.len() + extra;
    for face in faces {
        out.extend((start as u32).to_be_bytes());
        start += face.len();
    }
    out.resize(out.len() + extra, 0); // no TTC v2 signature
    for face in faces {
        let start = out.len() as u32;
        let mut face = face.clone();
        for table in 0..usize::from(u16_at(&face, 4)) {
            let at = 12 + table * 16 + 8;
            let offset = u32_at(&face, at) + start;
            face[at..at + 4].copy_from_slice(&offset.to_be_bytes());
        }
        out.extend(face);
    }
    out
}

#[test]
fn unicode_format4_and_its_glyph_array_match_the_full_charmap() {
    let cmap = embed::cmap(&[(0x41, 1), (0x42, 2), (0x3a9, 7)]).unwrap();
    assert_eq!(u16_at(&cmap, 2), 1, "format 4 only");
    let original = font(&cmap, 128);
    assert_coverage(&original, &[('A', Some(1)), ('B', Some(2)), ('Ω', Some(7)), ('C', None), ('\u{1f600}', None)]);
    assert_coverage(&font(&indirect_format4(), 128), &[('A', Some(3)), ('B', Some(9)), ('C', None)]);
}

#[test]
fn format12_and_mixed_unicode_tables_keep_supplementary_mappings() {
    let cmap = embed::cmap(&[(0x41, 1), (0x42, 2), (0x1f600, 3)]).unwrap();
    assert_eq!(u16_at(&cmap, 2), 2, "format 4 plus 12");
    for cmap in [cmap.clone(), one_encoding(&cmap, 10)] {
        let original = font(&cmap, 128);
        assert_coverage(&original, &[('A', Some(1)), ('B', Some(2)), ('\u{1f600}', Some(3)), ('C', None), ('\u{1f601}', None)]);
    }
}

#[test]
fn encoding_selection_and_zero_glyphs_match_the_original_charmap() {
    let bmp = one_encoding(&embed::cmap(&[(0x41, 1)]).unwrap(), 1);
    let full = one_encoding(&embed::cmap(&[(0x42, 2), (0x1f600, 3)]).unwrap(), 10);
    // The records disagree: preserve the full Charmap's selection, rather than unioning maps.
    let original = font(&two_encodings(&bmp, &full), 128);
    for c in ['A', 'B', '\u{1f600}', 'C'] {
        assert_eq!(probe(&original, c).0, Some(full_coverage(&original, c)), "original encoding selection for {c:?}");
    }
    let zero = embed::cmap(&[(0x41, 0), (0x42, 1), (0x1f600, 0)]).unwrap();
    let original = font(&zero, 128);
    assert_eq!(full_glyph(&original, 'B'), Some(1));
    for c in ['A', 'B', '\u{1f600}'] {
        assert_eq!(probe(&original, c).0, Some(full_coverage(&original, c)), "original zero-glyph policy for {c:?}");
    }
}

#[test]
fn windows_symbol_aliases_use_charmap_rather_than_unicode_only_lookup() {
    let raw = embed::cmap(&[(0xf041, 3), (0xf042, 9)]).unwrap();
    let mut symbol = one_encoding(&raw, 1);
    symbol[6..8].copy_from_slice(&0_u16.to_be_bytes()); // Windows symbol encoding
    let original = font(&symbol, 128);
    assert_coverage(&original, &[('\u{f041}', Some(3)), ('\u{f042}', Some(9)), ('C', None)]);
    for c in ['A', 'B'] {
        assert_eq!(probe(&original, c).0, Some(full_coverage(&original, c)), "original symbol alias for {c:?}");
    }
}

#[test]
fn cff_sfnt_header_does_not_change_the_cmap_oracle() {
    let mut original = font(&embed::cmap(&[(0x41, 1)]).unwrap(), 128);
    original[..4].copy_from_slice(b"OTTO");
    assert_coverage(&original, &[('A', Some(1)), ('B', None)]);
}

#[test]
fn ttc_negatives_cover_all_faces_and_later_unnamed_faces_can_match() {
    let a = font(&embed::cmap(&[(0x41, 1)]).unwrap(), 128);
    let cmap = embed::cmap(&[(0x1f600, 3)]).unwrap();
    let unnamed = sfnt_of(&[(b"cmap", &cmap)]).unwrap();
    for version in [0x0001_0000, 0x0002_0000] {
        let original = collection(&[a.clone(), unnamed.clone()], version);
        for c in ['A', '\u{1f600}', 'Z'] {
            assert_eq!(probe(&original, c).0, Some(full_coverage(&original, c)), "TTC version {version:x}, {c:?}");
        }
        assert!(full_coverage(&original, '\u{1f600}'), "the last physical face, without a name table");
        let (_, reads) = probe(&original, 'Z');
        let last_face = u32_at(&original, 16) as u64;
        assert!(reads.ranges.iter().any(|r| r.start == last_face), "a negative inspected the second directory");
    }
}

#[test]
fn distinct_missing_characters_read_only_directories_and_cmap() {
    let body_len = 2 << 20;
    let original = font(&embed::cmap(&[(0x41, 1)]).unwrap(), body_len);
    let body_start = (original.len() - body_len) as u64;
    let mut total = 0;
    for c in ['\u{e101}', '\u{e102}'] {
        assert!(!full_coverage(&original, c), "independent absent-character oracle");
        let (result, reads) = probe(&original, c);
        assert_eq!(result, Some(false));
        let cmap_start = u32_at(&original, 20) as u64;
        let cmap_len = u32_at(&original, 24) as u64;
        assert_eq!(reads.ranges, [0..12, 12..76, cmap_start..cmap_start + cmap_len], "only header, four directory records and cmap");
        assert_eq!(reads.bytes, 12 + 4 * 16 + cmap_len as usize);
        assert!(reads.bytes < 1024, "{} probe bytes instead of {} full bytes", reads.bytes, original.len());
        assert!(reads.ranges.iter().all(|r| r.end <= body_start), "no unused body bytes");
        total += reads.bytes;
    }
    assert!(total < 2048);
    assert!(original.len() > 2 << 20, "the full body is material, not a timing oracle");
}

struct TempFont(PathBuf);

impl TempFont {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("vc-coverage-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path.join("original.ttf"))
    }
}

impl Drop for TempFont {
    fn drop(&mut self) {
        // A private test fixture only; cleanup must not hide an assertion failure.
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

#[test]
fn same_path_names_length_and_mtime_do_not_hide_live_replacement() {
    let a = font(&embed::cmap(&[(0x41, 1)]).unwrap(), 128);
    let b = font(&embed::cmap(&[(0x42, 1)]).unwrap(), 128);
    assert_eq!(a.len(), b.len());
    for replace_path in [false, true] {
        let fixture = TempFont::new();
        std::fs::write(&fixture.0, &a).unwrap();
        let before = std::fs::metadata(&fixture.0).unwrap();
        let modified = before.modified().unwrap();
        assert_eq!(file_font_coverage(&fixture.0, 'B'), Some(false));
        let destination = if replace_path { fixture.0.with_extension("replacement") } else { fixture.0.clone() };
        std::fs::write(&destination, &b).unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&destination).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(modified)).unwrap();
        drop(file);
        if replace_path {
            // Completed replacement, with no platform-specific atomic-rename assumptions.
            std::fs::remove_file(&fixture.0).unwrap();
            std::fs::rename(&destination, &fixture.0).unwrap();
        }
        let after = std::fs::metadata(&fixture.0).unwrap();
        assert_eq!(after.len(), before.len());
        assert_eq!(after.modified().unwrap(), modified);
        let live = std::fs::read(&fixture.0).unwrap();
        assert_eq!(full_glyph(&live, 'B'), Some(1));
        assert_eq!(full_glyph(&live, 'A'), None);
        assert_eq!(file_font_coverage(&fixture.0, 'B'), Some(true));
        assert_eq!(file_font_coverage(&fixture.0, 'A'), Some(false));
    }
}

#[test]
fn unsupported_or_ambiguous_structures_request_the_original_read() {
    let cmap = embed::cmap(&[(0x41, 1)]).unwrap();
    let original = font(&cmap, 128);
    let mut unknown_sfnt = original.clone();
    unknown_sfnt[..4].copy_from_slice(b"????");
    let mut unknown_ttc = collection(std::slice::from_ref(&original), 0x0001_0000);
    unknown_ttc[4..8].copy_from_slice(&0x0003_0000_u32.to_be_bytes());
    let mut unsorted = original.clone();
    let first = unsorted[12..28].to_vec();
    let second = unsorted[28..44].to_vec();
    unsorted[12..28].copy_from_slice(&second);
    unsorted[28..44].copy_from_slice(&first);
    let duplicate = sfnt_of(&[(b"cmap", &cmap), (b"cmap", &cmap)]).unwrap();
    let missing = sfnt_of(&[(b"name", &test_fonts::name_table(&[(1, "Missing Cmap")]).unwrap())]).unwrap();
    for data in [unknown_sfnt, unknown_ttc, unsorted, duplicate, missing] {
        assert_eq!(probe(&data, 'Z').0, None, "uncertainty never becomes a negative");
    }
    let unknown_cmap = encoding(3, 1, &[0, 99, 0, 4]);
    assert_eq!(probe(&font(&unknown_cmap, 128), 'Z').0, None);
    let known = one_encoding(&cmap, 1);
    let unsupported = encoding(0, 5, &[0, 99, 0, 4]);
    assert_eq!(probe(&font(&two_encodings(&known, &unsupported), 128), 'Z').0, None, "unsupported second encoding is not discarded");
    let later_unknown = collection(&[original, font(&unsupported, 128)], 0x0001_0000);
    assert_eq!(probe(&later_unknown, 'Z').0, None, "an unsupported later face is not discarded");
    let mut bad_range = indirect_format4();
    bad_range[12 + 28..12 + 30].copy_from_slice(&2_u16.to_be_bytes());
    assert_eq!(probe(&font(&bad_range, 128), 'Z').0, None, "glyph array range overlaps its header");
    let mut bad_groups = one_encoding(&embed::cmap(&[(0x1f600, 3)]).unwrap(), 10);
    bad_groups[12 + 12..12 + 16].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(probe(&font(&bad_groups, 128), 'Z').0, None, "truncated group array");
}

#[test]
fn face_table_and_cmap_limits_never_truncate_to_a_negative() {
    let mapped = font(&embed::cmap(&[(0x42, 1)]).unwrap(), 0);
    let absent = font(&embed::cmap(&[(0x41, 1)]).unwrap(), 0);
    let mut faces = vec![absent; 256];
    faces.push(mapped);
    let original = collection(&faces, 0x0001_0000);
    assert!(full_coverage(&original, 'B'), "face 257 maps B");
    assert_eq!(probe(&original, 'B').0, None, "over-limit collection is not truncated");

    let cmap = embed::cmap(&[(0x41, 1)]).unwrap();
    let tags: Vec<[u8; 4]> = (0..256).map(|i| format!("z{i:03}").into_bytes().try_into().unwrap()).collect();
    let mut tables: Vec<(&[u8; 4], &[u8])> = tags.iter().map(|t| (t, &[][..])).collect();
    tables.push((b"cmap", &cmap));
    let original = sfnt_of(&tables).unwrap();
    assert_eq!(full_glyph(&original, 'A'), Some(1));
    assert_eq!(probe(&original, 'Z').0, None, "257 tables take the full path");

    let mut huge = cmap;
    huge.resize((1 << 20) + 1, 0);
    let original = font(&huge, 0);
    assert_eq!(full_glyph(&original, 'A'), Some(1));
    let (result, reads) = probe(&original, 'Z');
    assert_eq!(result, None);
    assert!(reads.bytes < 1024, "over-limit cmap is not allocated/read");
}

#[test]
fn total_read_budget_does_not_discard_a_later_mapping() {
    let mut absent = embed::cmap(&[(0x41, 1)]).unwrap();
    absent.resize(1 << 20, 0);
    let mut mapped = embed::cmap(&[(0x42, 1)]).unwrap();
    mapped.resize(1 << 20, 0);
    let mut faces = vec![font(&absent, 0); 4];
    faces.push(font(&mapped, 0));
    let original = collection(&faces, 0x0001_0000);
    assert!(full_coverage(&original, 'B'));
    let (result, reads) = probe(&original, 'B');
    assert_eq!(result, None, "a partial collection cannot authorize a negative");
    assert!(reads.bytes <= 4 << 20, "bounded total probe bytes");
}

#[test]
fn bounds_and_read_failures_are_uncertain_and_can_be_retried() {
    let original = font(&embed::cmap(&[(0x41, 1)]).unwrap(), 128);
    let mut outside = original.clone();
    outside[12 + 8..12 + 12].copy_from_slice(&u32::MAX.to_be_bytes()); // cmap offset
    assert_eq!(probe(&outside, 'Z').0, None);
    let end = (u32_at(&original, 20) + u32_at(&original, 24)) as usize;
    let mut short = CountReads::new(&original[..end - 1]);
    assert_eq!(probe_font_coverage(&mut short, original.len() as u64, 'Z'), None, "read_exact failure");
    assert_eq!(probe(&original, 'Z').0, Some(false), "a fresh repaired reader is retried");
    assert_eq!(probe(&original, 'A').0, Some(true));
}

#[cfg(not(target_arch = "wasm32"))]
mod fontdb_integration {
    use super::{AtomicU64, Ordering, PathBuf, collection, embed, encoding, file_font_coverage, test_fonts};
    use crate::fontdb::{FallbackTestIo, FontDb};
    use std::sync::Arc;
    use write_fonts::{FontBuilder, types::Tag};

    /// Original two-glyph TrueType font: empty .notdef and a 100-unit square, 600-unit advances.
    /// No bundled font tables or binary assets are copied.
    fn complete_font(family: &str, cmap: &[u8]) -> Vec<u8> {
        let mut head: Vec<u8> = [0x0001_0000_u32, 0x0001_0000, 0, 0x5f0f_3cf5].into_iter().flat_map(u32::to_be_bytes).collect();
        head.extend([0_u16, 1000].into_iter().flat_map(u16::to_be_bytes));
        head.extend([0; 16]); // created/modified
        head.extend([0_i16, 0, 100, 100, 0, 8, 2, 0, 0].into_iter().flat_map(i16::to_be_bytes));

        let mut hhea = 0x0001_0000_u32.to_be_bytes().to_vec();
        hhea.extend([800_i16, -200, 0, 600, 0, 500, 100, 1, 0, 0, 0, 0, 0, 0, 0, 2].into_iter().flat_map(i16::to_be_bytes));
        let mut maxp = 0x0001_0000_u32.to_be_bytes().to_vec();
        maxp.extend([2_u16, 4, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0].into_iter().flat_map(u16::to_be_bytes));
        let mut glyf: Vec<u8> = [1_i16, 0, 0, 100, 100, 3, 0].into_iter().flat_map(i16::to_be_bytes).collect();
        glyf.extend([1; 4]); // four on-curve points, no instructions
        glyf.extend([0_i16, 100, 0, -100, 0, 0, 100, 0].into_iter().flat_map(i16::to_be_bytes));
        let loca = [0_u16, 0, (glyf.len() / 2) as u16].into_iter().flat_map(u16::to_be_bytes).collect();
        let hmtx = [600_u16, 0, 600, 0].into_iter().flat_map(u16::to_be_bytes).collect();

        let mut os2 = vec![0; 78]; // OS/2 v0, regular, installable embedding
        for (at, value) in [(2, 600_u16), (4, 400), (6, 5), (62, 0x40), (66, 0xffff), (68, 800), (70, (-200_i16) as u16), (74, 800), (76, 200)] {
            os2[at..at + 2].copy_from_slice(&value.to_be_bytes());
        }
        os2[58..62].copy_from_slice(b"VCIT");
        let mut post = vec![0; 32];
        post[..4].copy_from_slice(&0x0003_0000_u32.to_be_bytes());
        let ps = format!("{}-Regular", family.replace(' ', ""));
        let name = test_fonts::name_table(&[(1, family), (2, "Regular"), (4, family), (6, &ps)]).unwrap();
        let mut builder = FontBuilder::new();
        for (tag, data) in [
            (b"head", head),
            (b"hhea", hhea),
            (b"maxp", maxp),
            (b"glyf", glyf),
            (b"loca", loca),
            (b"hmtx", hmtx),
            (b"OS/2", os2),
            (b"post", post),
            (b"name", name),
            (b"cmap", cmap.to_vec()),
            (b"zzzz", vec![0xcd; 16 << 10]),
        ] {
            builder.add_raw(Tag::new(tag), data);
        }
        builder.build()
    }

    /// Valid full-reader mapping deliberately outside the compact probe's format 4/12 subset.
    fn format13(c: char) -> Vec<u8> {
        let mut table: Vec<u8> = [13_u16, 0].into_iter().flat_map(u16::to_be_bytes).collect();
        table.extend([28_u32, 0, 1, c as u32, c as u32, 1].into_iter().flat_map(u32::to_be_bytes));
        encoding(3, 10, &table)
    }

    struct DbFixture(PathBuf);

    impl DbFixture {
        fn new() -> Self {
            // Honor the caller's TMPDIR; ordinary test runs use Rust's default temporary directory.
            let root = std::env::temp_dir();
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = root.join(format!("vc-fontdb-integration-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, data: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, data).unwrap();
            path
        }
    }

    impl Drop for DbFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn db_definite_misses_skip_full_reads_and_repeated_misses_are_cached() {
        let fixture = DbFixture::new();
        let data = complete_font("Db Absent", &embed::cmap(&[(0x41, 1)]).unwrap());
        let path = fixture.write("absent.ttf", &data);
        let db = FontDb::for_fallback_test(vec![path.clone()]);
        assert_eq!(db.load_system_fonts(), 1);
        assert!(db.has_family("Db Absent"));
        assert!(!db.is_loaded("Db Absent"));
        assert!(db.face_covering('Z').is_none());
        let first = FallbackTestIo { probes: vec![(path.clone(), 'Z')], full_reads: vec![] };
        assert_eq!(db.fallback_test_io(), first);
        assert!(db.face_covering('Z').is_none());
        assert_eq!(db.fallback_test_io(), first, "same scalar does not retry the generic scan");
        assert!(db.face_covering('Y').is_none());
        assert_eq!(db.fallback_test_io(), FallbackTestIo { probes: vec![(path.clone(), 'Z'), (path, 'Y')], full_reads: vec![] });
        assert!(!db.is_loaded("Db Absent"));
    }

    #[test]
    fn db_positive_and_uncertain_hits_keep_the_original_full_load() {
        for (cmap, expected_probe) in [(embed::cmap(&[(0x42, 1)]).unwrap(), Some(true)), (format13('B'), None)] {
            let fixture = DbFixture::new();
            let data = complete_font("Db Hit", &cmap);
            let path = fixture.write("hit.ttf", &data);
            assert_eq!(file_font_coverage(&path, 'B'), expected_probe, "fixture takes the intended compact route");
            let db = FontDb::for_fallback_test(vec![path.clone()]);
            assert_eq!(db.load_system_fonts(), 1);
            assert!(!db.is_loaded("Db Hit"));
            let face = db.face_covering('B').expect("generic fallback loads the original named face");
            assert_eq!((&*face.family, &*face.style), ("Db Hit", "Regular"));
            assert_eq!(face.path(), Some(path.as_path()));
            assert_eq!(face.face_index(), 0);
            assert_eq!(face.file_data(), data);
            assert_eq!(face.glyph_for('B'), 1);
            assert_eq!(face.glyph_for('A'), 0);
            assert_eq!(face.advance(1), 600.0);
            assert!(db.outline(&face, 1).elements().len() >= 4, "the complete original font has a usable outline");
            assert!(db.is_loaded("Db Hit"));
            let io = FallbackTestIo { probes: vec![(path.clone(), 'B')], full_reads: vec![(path, 'B', data.len())] };
            assert_eq!(db.fallback_test_io(), io);
            assert!(Arc::ptr_eq(&face, &db.face_covering('B').unwrap()));
            assert!(Arc::ptr_eq(&face, &db.face("Db Hit", "Regular").unwrap()));
            assert_eq!(db.fallback_test_io(), io, "already loaded selection needs no generic I/O");
        }
    }

    #[test]
    fn db_uncertain_misses_read_the_full_body_without_loading_a_face() {
        let fixture = DbFixture::new();
        let data = complete_font("Db Uncertain", &format13('A'));
        let path = fixture.write("uncertain.ttf", &data);
        assert_eq!(super::full_glyph(&data, 'A'), Some(1), "valid format 13, not a malformed fixture");
        assert_eq!(file_font_coverage(&path, 'Z'), None);
        let db = FontDb::for_fallback_test(vec![path.clone()]);
        assert_eq!(db.load_system_fonts(), 1);
        assert!(db.face_covering('Z').is_none());
        let first = FallbackTestIo { probes: vec![(path.clone(), 'Z')], full_reads: vec![(path.clone(), 'Z', data.len())] };
        assert_eq!(db.fallback_test_io(), first);
        assert!(db.face_covering('Z').is_none());
        assert_eq!(db.fallback_test_io(), first);
        assert!(db.face_covering('Y').is_none());
        assert_eq!(
            db.fallback_test_io(),
            FallbackTestIo {
                probes: vec![(path.clone(), 'Z'), (path.clone(), 'Y')],
                full_reads: vec![(path.clone(), 'Z', data.len()), (path, 'Y', data.len())],
            }
        );
        assert!(!db.is_loaded("Db Uncertain"), "a full-reader miss is not a font load");
    }

    #[test]
    fn db_generic_order_deduplicates_ttc_paths_and_stops_after_first_hit() {
        let fixture = DbFixture::new();
        let absent = embed::cmap(&[(0x41, 1)]).unwrap();
        let negative = collection(&[complete_font("Db Absent One", &absent), complete_font("Db Absent Two", &absent)], 0x0001_0000);
        let first = complete_font("Db First", &embed::cmap(&[(0x3a9, 1)]).unwrap());
        let later = complete_font("Db Later", &embed::cmap(&[(0x3a9, 1)]).unwrap());
        let a = fixture.write("a-absent.ttc", &negative);
        let b = fixture.write("b-first.ttf", &first);
        let z = fixture.write("z-later.ttf", &later);
        // Reverse input order; the two named TTC faces also contribute duplicate catalog paths.
        let db = FontDb::for_fallback_test(vec![z.clone(), b.clone(), a.clone()]);
        assert_eq!(db.load_system_fonts(), 4);
        let face = db.face_covering('Ω').unwrap();
        assert_eq!(face.family, "Db First");
        assert_eq!(face.path(), Some(b.as_path()));
        assert_eq!(face.file_data(), first);
        let io = FallbackTestIo { probes: vec![(a.clone(), 'Ω'), (b.clone(), 'Ω')], full_reads: vec![(b, 'Ω', first.len())] };
        assert_eq!(db.fallback_test_io(), io, "negative TTC is probed once, later candidate is not touched");
        for family in ["Db Absent One", "Db Absent Two", "Db Later"] {
            assert!(!db.is_loaded(family));
        }
        let other = db.face("Db Later", "Regular").unwrap();
        assert_eq!(other.path(), Some(z.as_path()));
        assert_eq!(other.file_data(), later);
        assert!(Arc::ptr_eq(&face, &db.face_covering('Ω').unwrap()), "equal traits retain loaded order");
        assert!(Arc::ptr_eq(&other, &db.fallback_for('Ω', face.id()).unwrap()), "excluded loaded face allows the next one");
        assert_eq!(db.fallback_test_io(), io, "named-family load and loaded selection do not enter the generic loop");
    }

    #[test]
    fn db_generic_ttc_later_face_hit_loads_the_original_named_faces() {
        let fixture = DbFixture::new();
        let data = collection(
            &[
                complete_font("Db Ttc First", &embed::cmap(&[(0x41, 1)]).unwrap()),
                complete_font("Db Ttc Second", &embed::cmap(&[(0x3a9, 1)]).unwrap()),
            ],
            0x0002_0000,
        );
        let path = fixture.write("faces.ttc", &data);
        let db = FontDb::for_fallback_test(vec![path.clone()]);
        assert_eq!(db.load_system_fonts(), 2);
        let face = db.face_covering('Ω').unwrap();
        assert_eq!(face.family, "Db Ttc Second");
        assert_eq!(face.face_index(), 1);
        assert_eq!(face.path(), Some(path.as_path()));
        assert_eq!(face.file_data(), data);
        assert_eq!(face.glyph_for('Ω'), 1);
        assert!(db.is_loaded("Db Ttc First"));
        assert!(db.is_loaded("Db Ttc Second"));
        let io = FallbackTestIo { probes: vec![(path.clone(), 'Ω')], full_reads: vec![(path, 'Ω', data.len())] };
        assert_eq!(db.fallback_test_io(), io);
        assert_eq!(db.face_covering('A').unwrap().family, "Db Ttc First");
        assert_eq!(db.fallback_test_io(), io);
    }

    #[test]
    fn db_completed_replacement_rescan_retries_cached_scalar_miss() {
        let a = complete_font("Db Replacement", &embed::cmap(&[(0x41, 1), (0x44, 1)]).unwrap());
        let b = complete_font("Db Replacement", &embed::cmap(&[(0x42, 1), (0x45, 1)]).unwrap());
        assert_eq!(a.len(), b.len());
        for replace_path in [false, true] {
            let fixture = DbFixture::new();
            let path = fixture.write("replacement.ttf", &a);
            let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
            let db = FontDb::for_fallback_test(vec![path.clone()]);
            assert_eq!(db.load_system_fonts(), 1);
            assert!(db.face_covering('B').is_none());
            let miss = FallbackTestIo { probes: vec![(path.clone(), 'B')], full_reads: vec![] };
            assert_eq!(db.fallback_test_io(), miss);
            let destination = if replace_path { fixture.write("new.ttf", &b) } else { fixture.write("replacement.ttf", &b) };
            let file = std::fs::OpenOptions::new().write(true).open(&destination).unwrap();
            file.set_times(std::fs::FileTimes::new().set_modified(modified)).unwrap();
            drop(file);
            if replace_path {
                // Completed replacement only: no assumption about concurrent/atomic rename.
                std::fs::remove_file(&path).unwrap();
                std::fs::rename(&destination, &path).unwrap();
            }
            assert_eq!(std::fs::metadata(&path).unwrap().len(), a.len() as u64);
            assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), modified);
            assert_eq!(std::fs::read(&path).unwrap(), b);
            assert!(db.face_covering('B').is_none(), "completed replacement alone keeps the cached scalar miss");
            assert_eq!(db.fallback_test_io(), miss);
            let generation = db.generation();
            assert_eq!(db.load_system_fonts(), 1);
            assert!(db.generation() > generation);
            let live = db.face_covering('B').expect("catalog rescan must retry the cached scalar against the completed replacement");
            assert_eq!(live.path(), Some(path.as_path()));
            assert_eq!(live.file_data(), b);
            assert_eq!(live.glyph_for('B'), 1);
            assert_eq!(live.glyph_for('A'), 0);
            let io = FallbackTestIo { probes: vec![(path.clone(), 'B'), (path.clone(), 'B')], full_reads: vec![(path.clone(), 'B', b.len())] };
            assert_eq!(db.fallback_test_io(), io);
            assert!(Arc::ptr_eq(&live, &db.face_covering('B').unwrap()), "repeated scalar reuses the retried loaded face");
            assert!(Arc::ptr_eq(&live, &db.face_covering('E').unwrap()), "the other mapped scalar uses loaded replacement bytes");
            assert_eq!(db.fallback_test_io(), io);
            let fresh = FontDb::for_fallback_test(vec![path.clone()]);
            assert_eq!(fresh.face_covering('B').unwrap().file_data(), b);
            assert_eq!(fresh.fallback_test_io(), FallbackTestIo { probes: vec![(path.clone(), 'B')], full_reads: vec![(path, 'B', b.len())] });
        }
    }

    #[test]
    fn db_rescan_does_not_replace_an_already_loaded_family_style() {
        let fixture = DbFixture::new();
        let a = complete_font("Db Loaded", &embed::cmap(&[(0x41, 1)]).unwrap());
        let b = complete_font("Db Loaded", &embed::cmap(&[(0x42, 1)]).unwrap());
        let path = fixture.write("loaded.ttf", &a);
        let db = FontDb::for_fallback_test(vec![path.clone()]);
        let old = db.face_covering('A').unwrap();
        assert_eq!(old.file_data(), a);
        fixture.write("loaded.ttf", &b);
        assert_eq!(db.load_system_fonts(), 1);
        let io = db.fallback_test_io();
        assert!(Arc::ptr_eq(&old, &db.face_covering('A').unwrap()));
        assert_eq!(db.fallback_test_io(), io, "loaded face wins before inspecting its replacement");
        assert!(db.face_covering('B').is_none(), "existing family/style prevents loading replacement bytes");
        let io = FallbackTestIo {
            probes: vec![(path.clone(), 'A'), (path.clone(), 'B')],
            full_reads: vec![(path.clone(), 'A', a.len()), (path.clone(), 'B', b.len())],
        };
        assert_eq!(db.fallback_test_io(), io);
        assert_eq!(db.face("Db Loaded", "Regular").unwrap().file_data(), a);
        assert!(db.face_covering('B').is_none());
        assert_eq!(db.fallback_test_io(), io);
        let fresh = FontDb::for_fallback_test(vec![path.clone()]);
        assert_eq!(fresh.face_covering('B').unwrap().file_data(), b);
        assert_eq!(fresh.fallback_test_io(), FallbackTestIo { probes: vec![(path.clone(), 'B')], full_reads: vec![(path, 'B', b.len())] });
    }

    #[test]
    fn db_rescan_does_not_republish_an_in_flight_miss() {
        let fixture = DbFixture::new();
        let absent = complete_font("Db Race Absent", &embed::cmap(&[(0x41, 1)]).unwrap());
        let covering = complete_font("Db Race Covering", &embed::cmap(&[(0x42, 1)]).unwrap());
        let a = fixture.write("a-absent.ttf", &absent);
        let db = Arc::new(FontDb::for_fallback_test(vec![fixture.0.clone()]));
        assert_eq!(db.load_system_fonts(), 1);

        let (reached, resume) = db.pause_next_fallback_miss_for_test('B');
        let worker_db = db.clone();
        let worker = std::thread::spawn(move || worker_db.face_covering('B'));
        reached.recv_timeout(std::time::Duration::from_secs(10)).expect("old-catalog sweep reaches the publication barrier");
        let old_io = FallbackTestIo { probes: vec![(a.clone(), 'B')], full_reads: vec![] };
        assert_eq!(db.fallback_test_io(), old_io);
        assert!(!db.is_loaded("Db Race Absent"));

        // Install a separate ordinary font only after the old sweep has completed.
        let b = fixture.write("b-covering.ttf", &covering);
        let generation = db.generation();
        assert_eq!(db.load_system_fonts(), 2);
        assert!(db.generation() > generation);
        assert!(db.has_family("Db Race Covering"));
        assert!(!db.is_loaded("Db Race Covering"));
        assert_eq!(db.fallback_test_io(), old_io, "rescan catalogs without a generic lookup");

        resume.send(()).unwrap();
        assert!(worker.join().unwrap().is_none(), "the earlier sweep still returns its own miss");
        let face = db.face_covering('B').expect("a pre-rescan sweep must not suppress the newly cataloged font");
        assert_eq!(face.family, "Db Race Covering");
        assert_eq!(face.path(), Some(b.as_path()));
        assert_eq!(face.file_data(), covering);
        assert_eq!(face.glyph_for('B'), 1);
        assert_eq!(face.glyph_for('A'), 0);
        let io = FallbackTestIo { probes: vec![(a.clone(), 'B'), (a, 'B'), (b.clone(), 'B')], full_reads: vec![(b, 'B', covering.len())] };
        assert_eq!(db.fallback_test_io(), io, "same scalar retries against the rescanned catalog");
        assert!(Arc::ptr_eq(&face, &db.face_covering('B').unwrap()));
        assert_eq!(db.fallback_test_io(), io, "loaded selection adds no generic I/O");
    }
}
