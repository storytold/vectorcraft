//! A face's list of mapped characters (the Glyphs panel's grid): read from the font once for the
//! face, and the same list, in the same order, as a fresh read of the font's `cmap`.

use std::collections::BTreeMap;
use std::sync::Arc;

use skrifa::MetadataProvider;

use crate::test_fonts::{CMAPLESS_FAMILY, LARGE_CMAP_FAMILY, VARIABLE_FAMILY, cmapless_font, large_cmap_font, variable_font};
use crate::*;

/// The characters `face` maps, read from its `cmap` each time: every mapping with a valid code point,
/// sorted by code point, one per character.
fn read_chars(face: &FontFace) -> Vec<(char, u32)> {
    let Some(f) = face.skrifa() else { return vec![] };
    let mut v: Vec<(char, u32)> = f.charmap().mappings().filter_map(|(cp, g)| char::from_u32(cp).map(|c| (c, g.to_u32()))).collect();
    v.sort_unstable_by_key(|x| x.0);
    v.dedup_by_key(|x| x.0);
    v
}

/// A database of the bundled fonts, the test variable font, a font with a large `cmap` and one with
/// no `cmap`.
fn db() -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    assert!(db.add_font(variable_font().expect("the variable test font builds")) > 1, "its named instances are faces");
    assert_eq!(db.add_font(large_cmap_font(50_000).expect("the large test font builds")), 1);
    assert_eq!(db.add_font(cmapless_font().expect("the cmapless test font builds")), 1);
    db
}

/// Faces of every kind the panel browses: static faces, a variable font's named instance and an
/// instance at other axis settings, a large `cmap`, and no `cmap`.
fn faces(db: &FontDb) -> Vec<Arc<FontFace>> {
    let mut faces: Vec<Arc<FontFace>> = [
        (FALLBACK_FAMILY, "Regular"),
        (FALLBACK_FAMILY, "Bold"),
        ("Inter", "Regular"),
        ("JetBrains Mono", "Regular"),
        (VARIABLE_FAMILY, "Regular"),
        (VARIABLE_FAMILY, "Bold"),
        (LARGE_CMAP_FAMILY, "Regular"),
        (CMAPLESS_FAMILY, "Regular"),
    ]
    .into_iter()
    .map(|(family, style)| db.face(family, style).unwrap_or_else(|| panic!("{family} {style} is loaded")))
    .collect();
    let regular = db.face(VARIABLE_FAMILY, "Regular").expect("the variable face");
    let axes: BTreeMap<String, f64> = [("wght".to_string(), 620.0)].into_iter().collect();
    let instance = db.instance(&regular, &axes);
    assert_ne!(instance.id(), regular.id(), "a new instance");
    faces.push(instance);
    faces
}

#[test]
fn the_character_list_is_the_fonts_cmap_read_once() {
    let db = db();
    let faces = faces(&db);
    for face in &faces {
        let list = face.mapped_chars();
        assert_eq!(&list[..], read_chars(face).as_slice(), "{face:?}");
        assert_eq!(face.chars(), read_chars(face), "{face:?}");
        assert!(Arc::ptr_eq(&list, &face.mapped_chars()), "{face:?}: read once");
    }
    let large = db.face(LARGE_CMAP_FAMILY, "Regular").expect("the large face");
    assert_eq!(large.mapped_chars().len(), 50_000);
    let none = db.face(CMAPLESS_FAMILY, "Regular").expect("the cmapless face");
    assert!(none.mapped_chars().is_empty(), "a font with no cmap maps no character");
    // Each face keeps its own list: an instance of a variable font shares its characters, not the
    // list read for another face.
    for (i, a) in faces.iter().enumerate() {
        for b in &faces[i + 1..] {
            assert!(a.mapped_chars().is_empty() || !Arc::ptr_eq(&a.mapped_chars(), &b.mapped_chars()), "{a:?} and {b:?}");
        }
    }
}

/// Times one face's character list read from the font each time against the list read once, for the
/// bundled Source Sans 3 Regular and a font mapping 50,000 characters. Run with
/// `cargo test --release -p vectorcraft-text --lib character_list_timing -- --ignored --nocapture`.
#[test]
#[ignore = "manual timing probe; wall-clock performance depends on machine load"]
fn character_list_timing() {
    const CALLS: usize = 100;
    let db = db();
    for (family, style) in [(FALLBACK_FAMILY, "Regular"), (LARGE_CMAP_FAMILY, "Regular")] {
        let face = db.face(family, style).expect("the face");
        let mut read = vec![];
        let mut kept = vec![];
        for _ in 0..7 {
            let start = std::time::Instant::now();
            let mut n = 0;
            for _ in 0..CALLS {
                n += std::hint::black_box(read_chars(&face)).len();
            }
            read.push(start.elapsed().as_secs_f64() * 1000.0 / CALLS as f64);
            let start = std::time::Instant::now();
            let mut m = 0;
            for _ in 0..CALLS {
                m += std::hint::black_box(face.mapped_chars()).len();
            }
            kept.push(start.elapsed().as_secs_f64() * 1000.0 / CALLS as f64);
            assert_eq!(n, m);
        }
        read.sort_by(f64::total_cmp);
        kept.sort_by(f64::total_cmp);
        println!(
            "{family} {style}, {} characters: read each time median {:.4} ms, read once median {:.6} ms per call; samples {read:.4?} and {kept:.6?}",
            face.mapped_chars().len(),
            read[3],
            kept[3]
        );
    }
}
