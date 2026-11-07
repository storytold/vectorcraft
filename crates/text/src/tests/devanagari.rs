//! Devanagari shaping: conjuncts, reph, pre-base matras and nukta forms, with their source clusters
//! and caret stops. The expected forms come from Microsoft's OpenType Devanagari shaping specification
//! (https://learn.microsoft.com/en-us/typography/script-development/devanagari) and the Unicode
//! Standard's Devanagari section (https://www.unicode.org/versions/latest/core-spec/chapter-12/).
//! No font is committed: the tests use an installed face and print SKIPPED when none is found.
use crate::*;
use kurbo::{BezPath, Point};
use std::sync::Arc;
use vectorcraft_doc::{CharStyle, TextKind};
use vectorcraft_geom::PathData;

fn devanagari_test_face(test: &str) -> Option<Arc<FontFace>> {
    let db = FontDb::global();
    // The face must have OpenType (GSUB) Devanagari lookups. An AAT face such as macOS's Devanagari MT
    // shapes with `morx`, and हिन्दी comes out as three clusters because the conjunct न्दी is split in two.
    let opentype = |face: &FontFace| {
        use skrifa::raw::TableProvider;
        face.skrifa()
            .and_then(|f| f.gsub().ok())
            .and_then(|gsub| gsub.script_list().ok())
            .is_some_and(|list| list.script_records().iter().any(|r| matches!(&r.script_tag().to_be_bytes(), b"dev2" | b"deva")))
    };
    let face = ["Kohinoor Devanagari", "ITF Devanagari", "Devanagari Sangam MN", "Noto Sans Devanagari", "Nirmala UI", "Mangal"]
        .into_iter()
        .filter_map(|name| db.face(name, "Regular").filter(|face| face.family == name))
        .find(|face| {
            ['क', 'ष', 'र', 'ह', 'न', 'द', '\u{094D}', '\u{093F}', '\u{0940}', '\u{093C}'].iter().all(|&c| face.covers(c)) && opentype(face)
        });
    if let Some(face) = &face {
        let path = face.path().map_or_else(|| "bundled font".to_string(), |p| p.display().to_string());
        eprintln!("ASSERTIONS RUN: {test}; Devanagari font {} {} ({path})", face.family, face.style);
    } else {
        eprintln!("SKIPPED: {test}; no installed candidate font shapes Devanagari through GSUB");
    }
    face
}

fn devanagari_layout(face: &FontFace, text: &str) -> TextLayout {
    let st = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size: 20.0, ..CharStyle::default() };
    layout(FontDb::global(), &TextObject::point(Point::ZERO, text, st))
}

fn source_clusters(layout: &TextLayout) -> Vec<(usize, usize)> {
    let mut clusters: Vec<_> = layout.glyphs.iter().map(|glyph| (glyph.byte, glyph.len)).collect();
    clusters.dedup();
    clusters
}

/// The left and right edges of the glyphs of the cluster that starts at `byte`.
fn cluster_extent(layout: &TextLayout, byte: usize) -> (f64, f64) {
    let glyphs = layout.glyphs.iter().filter(|g| g.byte == byte);
    glyphs.fold((f64::INFINITY, f64::NEG_INFINITY), |(left, right), g| (left.min(g.origin.x), right.max(g.origin.x + g.advance)))
}

#[test]
fn ka_virama_ssa_forms_one_conjunct_glyph() {
    let Some(face) = devanagari_test_face("ka_virama_ssa_forms_one_conjunct_glyph") else { return };
    // `akhn` substitutes Ka + halant + Ssa with the KaSsa ligature.
    let l = devanagari_layout(&face, "क्ष");
    assert_eq!(l.glyphs.len(), 1, "{:?}", l.glyphs);
    let g = &l.glyphs[0];
    assert_eq!((g.byte, g.len), (0, "क्ष".len()));
    assert!(!['क', 'ष', '\u{094D}'].iter().any(|&c| face.glyph_for(c) == g.gid), "conjunct drawn with a nominal glyph: {}", g.gid);
}

#[test]
fn reph_is_drawn_after_its_base_consonant() {
    let Some(face) = devanagari_test_face("reph_is_drawn_after_its_base_consonant") else { return };
    // Ra + halant before the base consonant becomes the reph (`rphf`), reordered after the base.
    let s = "र्क";
    let l = devanagari_layout(&face, s);
    let gids: Vec<u32> = l.glyphs.iter().map(|g| g.gid).collect();
    assert_eq!(gids.len(), 2, "{:?}", l.glyphs);
    assert_eq!(gids[0], face.glyph_for('क'), "the base consonant comes first: {gids:?}");
    assert!(![face.glyph_for('र'), face.glyph_for('\u{094D}'), face.glyph_for('क')].contains(&gids[1]), "no reph glyph: {gids:?}");
    assert!(l.glyphs[1].origin.x >= l.glyphs[0].origin.x);
    assert_eq!(source_clusters(&l), vec![(0, s.len())]);
}

#[test]
fn pre_base_matra_is_drawn_before_its_consonant() {
    let Some(face) = devanagari_test_face("pre_base_matra_is_drawn_before_its_consonant") else { return };
    let s = "कि";
    let l = devanagari_layout(&face, s);
    assert_eq!(l.glyphs.len(), 2, "{:?}", l.glyphs);
    let (matra, ka) = (&l.glyphs[0], &l.glyphs[1]);
    assert_eq!(ka.gid, face.glyph_for('क'));
    assert_ne!(matra.gid, ka.gid);
    assert!(matra.origin.x < ka.origin.x, "the i matra is drawn to the left: {:?}", l.glyphs);
    assert_eq!(source_clusters(&l), vec![(0, s.len())]);
    // The caret stops at the syllable's ends: before the matra, and after the consonant.
    let (left, right) = cluster_extent(&l, 0);
    assert!((caret_position(&l, 0).0.x - left).abs() < 1e-9);
    assert!((caret_position(&l, s.len()).0.x - right).abs() < 1e-9);
}

#[test]
fn syllables_map_to_their_source_bytes_and_caret_stops() {
    let Some(face) = devanagari_test_face("syllables_map_to_their_source_bytes_and_caret_stops") else { return };
    // हि + न्दी: raw font clusters can subdivide a Unicode editing grapheme.
    let s = "हिन्दी";
    let l = devanagari_layout(&face, s);
    let second = "हि".len();
    let clusters = source_clusters(&l);
    assert_eq!(clusters.first(), Some(&(0, second)), "{clusters:?}");
    let mut end = 0;
    for &(byte, len) in &clusters {
        assert_eq!(byte, end, "source clusters must cover input without gaps or overlaps: {clusters:?}");
        assert!(len > 0, "empty source cluster: {clusters:?}");
        end = byte.checked_add(len).expect("source cluster range overflow");
        assert!(s.get(byte..end).is_some(), "source cluster must span valid UTF-8: {clusters:?}");
    }
    assert_eq!(end, s.len(), "source clusters must cover the whole input: {clusters:?}");
    // The first syllable's i matra is drawn before its consonant.
    assert_eq!(l.glyphs[1].gid, face.glyph_for('ह'), "{:?}", l.glyphs);
    assert!(l.glyphs[0].origin.x < l.glyphs[1].origin.x, "{:?}", l.glyphs);
    // Editing arrows follow Unicode graphemes, regardless of raw cluster subdivision.
    assert_eq!(edit::next_cluster(s, 0), second);
    assert_eq!(edit::next_cluster(s, second), s.len());
    assert_eq!(edit::prev_cluster(s, s.len()), second);
    assert_eq!(caret_horizontal_cluster(&l, s, 0, true), second);
    assert_eq!(caret_horizontal_cluster(&l, s, second, true), s.len());
    assert_eq!(caret_horizontal_cluster(&l, s, s.len(), false), second);
    assert!((caret_position(&l, second).0.x - cluster_extent(&l, 0).1).abs() < 1e-9);
}

#[test]
fn nukta_forms_match_the_precomposed_letters() {
    let Some(face) = devanagari_test_face("nukta_forms_match_the_precomposed_letters") else { return };
    // U+0958 is canonically equivalent to Ka + nukta and is drawn the same way.
    let composed = devanagari_layout(&face, "\u{0958}");
    let decomposed = devanagari_layout(&face, "\u{0915}\u{093C}");
    let shape = |l: &TextLayout| l.glyphs.iter().map(|g| (g.gid, g.advance, g.origin)).collect::<Vec<_>>();
    assert_eq!(shape(&composed), shape(&decomposed));
    assert_eq!(source_clusters(&decomposed), vec![(0, "\u{0915}\u{093C}".len())]);
    // The nukta joins its consonant: one glyph takes up room on the line.
    assert_eq!(decomposed.glyphs.iter().filter(|g| g.advance > 0.0).count(), 1, "{:?}", decomposed.glyphs);
}

#[test]
fn a_click_in_a_syllable_goes_to_its_nearer_end() {
    let Some(face) = devanagari_test_face("a_click_in_a_syllable_goes_to_its_nearer_end") else { return };
    let s = "कि";
    let l = devanagari_layout(&face, s);
    let (left, right) = cluster_extent(&l, 0);
    // This point lies just past the matra, still in the left half of the syllable.
    let matra_end = l.glyphs[0].origin.x + l.glyphs[0].advance;
    let near_left = matra_end + 0.1;
    assert!(near_left < (left + right) * 0.5, "the matra takes most of the syllable: {:?}", l.glyphs);
    let y = l.lines[0].baseline;
    assert_eq!(hit_byte(&l, Point::new(near_left, y)), 0);
    assert_eq!(hit_byte(&l, Point::new(right - 0.1, y)), s.len());

    // Clicks at the same places on a path go to the same ends.
    let mut line = BezPath::new();
    line.move_to((0.0, 50.0));
    line.line_to((500.0, 50.0));
    let st = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size: 20.0, ..CharStyle::default() };
    let mut t = TextObject::point(Point::ZERO, s, st);
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
    let p = layout(FontDb::global(), &t);
    assert!(p.on_path && p.glyphs.len() == 2, "{:?}", p.glyphs);
    let x0 = p.glyphs[0].origin.x;
    let matra_end = x0 + p.glyphs[0].advance;
    assert_eq!(hit_byte(&p, Point::new(matra_end + 0.1, 50.0)), 0);
    assert_eq!(hit_byte(&p, Point::new(p.glyphs[1].origin.x + p.glyphs[1].advance - 0.1, 50.0)), s.len());
}
