//! Caret stops inside ligatures: the caret, a click, arrow keys and the selection reach each
//! character a ligature glyph stands for, at the font's ligature carets (`GDEF`) or, without them,
//! at the glyph's advance divided equally.

use kurbo::{BezPath, Point, Vec2};
use vectorcraft_doc::{CharStyle, TextKind, TextObject};
use vectorcraft_geom::PathData;

use crate::test_fonts::{
    LIGATURE_CARET_FAMILY, LIGATURE_CARET_GLYPH, VARIABLE_LIGATURE_CARET_FAMILY, VERTICAL_LIGATURE_CARET_FAMILY, lam_alef_family, lam_alef_font,
    ligature_caret_font, placed_ligature_caret_font, proportional_vertical_ligature_caret_font, source_sans_gid, variable_ligature_caret_font,
    vertical_ligature_caret_font,
};
use crate::*;

/// A database of the bundled fonts and `font`.
fn db_with(font: Option<Vec<u8>>) -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(font.expect("the test font builds")), 1);
    db
}

/// Put a contour-point caret on the existing test ligature without changing its outline.
fn contour_caret_font(point_index: u16) -> Vec<u8> {
    use write_fonts::FontBuilder;
    use write_fonts::tables::gdef::{CaretValue, Gdef, LigCaretList, LigGlyph};
    use write_fonts::tables::layout::CoverageFormat1 as WriteCoverage;
    use write_fonts::types::{GlyphId16, Tag};

    let base = ligature_caret_font(None).expect("test font builds");
    let font = skrifa::FontRef::new(&base).expect("test font parses");
    let gid = source_sans_gid(LIGATURE_CARET_GLYPH).expect("Source Sans 3 has the glyph");
    let mut builder = FontBuilder::new();
    let list = LigCaretList::new(WriteCoverage::new(vec![GlyphId16::new(gid)]).into(), vec![LigGlyph::new(vec![CaretValue::format_2(point_index)])]);
    builder.add_table(&Gdef::new(None, None, Some(list), None)).expect("GDEF builds");
    for record in font.table_directory.table_records() {
        let tag = Tag::new(&record.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(record.tag()).expect("base table reads").as_bytes());
        }
    }
    builder.build()
}

#[test]
fn a_contour_point_caret_uses_its_unhinted_x_and_y() {
    use skrifa::raw::TableProvider;
    use skrifa::raw::tables::glyf::Glyph;

    let base = ligature_caret_font(None).expect("test font builds");
    let font = skrifa::FontRef::new(&base).expect("test font parses");
    let gid = u32::from(source_sans_gid(LIGATURE_CARET_GLYPH).expect("Source Sans 3 has the glyph"));
    let outline = font.loca(None).expect("loca reads").get(skrifa::GlyphId::new(gid), &font.glyf().expect("glyf reads")).expect("glyph reads");
    let Glyph::Simple(simple) = outline.into_glyph().expect("glyph has an outline") else { panic!("W is simple") };
    let (point_index, point) =
        simple.points().enumerate().find(|(_, p)| p.x > 0 && p.x < 500 && p.y > -500 && p.y < 1000).expect("W has an interior contour point");
    let db = db_with(Some(contour_caret_font(u16::try_from(point_index).unwrap())));
    let face = db.face(LIGATURE_CARET_FAMILY, "Regular").unwrap();
    assert_eq!(face.ligature_carets(gid, 1), Some(vec![f64::from(point.x)]));
    assert_eq!(face.vertical_ligature_carets(gid, 1, 1000.0, 1500.0), Some(vec![1000.0 - f64::from(point.y)]));

    let bad = db_with(Some(contour_caret_font(u16::MAX)));
    assert_eq!(bad.face(LIGATURE_CARET_FAMILY, "Regular").unwrap().ligature_carets(gid, 1), None);
}

/// `count` glyph ids of [`contour_caret_font`] other than the ligature's.
fn spare_gids(count: usize) -> Vec<u16> {
    let ligature = source_sans_gid(LIGATURE_CARET_GLYPH).expect("Source Sans 3 has the glyph");
    (1..).filter(|g| *g != ligature).take(count).collect()
}

/// [`contour_caret_font`], whose caret is the ligature's point 0, with its `glyf` and `loca`
/// replaced: each `(gid, data)` in `glyphs` gives that glyph's `glyf` data, and every other glyph
/// is empty.
fn glyf_caret_font(glyphs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    use skrifa::raw::TableProvider;
    use write_fonts::FontBuilder;
    use write_fonts::types::Tag;

    let base = contour_caret_font(0);
    let font = skrifa::FontRef::new(&base).expect("test font parses");
    let num_glyphs = font.maxp().expect("maxp reads").num_glyphs();
    let short_loca = font.head().expect("head reads").index_to_loc_format() == 0;
    let mut glyf = Vec::new();
    let mut offsets = vec![0usize];
    for gid in 0..num_glyphs {
        if let Some((_, data)) = glyphs.iter().find(|(g, _)| *g == gid) {
            glyf.extend(data);
            // Keep every offset even for the short `loca` format.
            if glyf.len() % 2 == 1 {
                glyf.push(0);
            }
        }
        offsets.push(glyf.len());
    }
    let loca: Vec<u8> = if short_loca {
        offsets.iter().flat_map(|o| u16::try_from(o / 2).expect("short offset").to_be_bytes()).collect()
    } else {
        offsets.iter().flat_map(|o| u32::try_from(*o).expect("long offset").to_be_bytes()).collect()
    };
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"glyf"), glyf);
    builder.add_raw(Tag::new(b"loca"), loca);
    for record in font.table_directory.table_records() {
        let tag = Tag::new(&record.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(record.tag()).expect("base table reads").as_bytes());
        }
    }
    builder.build()
}

/// [`glyf_caret_font`] whose ligature is a composite of eight copies of a glyph that is itself
/// eight copies of the next, 15 levels down to a glyph with no contours.
fn composite_fan_caret_font() -> Vec<u8> {
    const FAN: usize = 8;
    let ligature = source_sans_gid(LIGATURE_CARET_GLYPH).expect("Source Sans 3 has the glyph");
    let levels = spare_gids(15);
    let composite = |child: u16| {
        let mut data = vec![0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0];
        for i in 0..FAN {
            // ARGS_ARE_XY_VALUES, plus MORE_COMPONENTS on all but the last; byte offsets of 0.
            let flags: u16 = if i + 1 < FAN { 0x0022 } else { 0x0002 };
            data.extend(flags.to_be_bytes());
            data.extend(child.to_be_bytes());
            data.extend([0, 0]);
        }
        data
    };
    let mut glyphs = vec![(ligature, composite(levels[0]))];
    for (level, gid) in levels.iter().enumerate() {
        match levels.get(level + 1) {
            Some(child) => glyphs.push((*gid, composite(*child))),
            // No contours, a zero bounding box and no instructions.
            None => glyphs.push((*gid, vec![0; 12])),
        }
    }
    glyf_caret_font(&glyphs)
}

/// A contour-point caret on a composite's point uses the component offset the outline loader
/// uses. The ligature is one component, a glyph with a single point at (50, 0), turned a quarter
/// turn and scaled by 1.5 with a two-by-two matrix and placed at (100, 0). With
/// SCALED_COMPONENT_OFFSET the offset's x scales by the length of the x basis, 1.5, so the point
/// lands at x = 150; with UNSCALED_COMPONENT_OFFSET as well the offset stays (100, 0).
#[test]
fn a_contour_point_caret_on_a_composite_takes_its_scaled_offset() {
    let gid = source_sans_gid(LIGATURE_CARET_GLYPH).expect("Source Sans 3 has the glyph");
    let [leaf] = spare_gids(1)[..] else { panic!("one spare glyph") };
    // One on-curve point at (50, 0) with word coordinates, and no instructions.
    let mut point = vec![0, 1, 0, 0, 0, 0, 0, 50, 0, 0, 0, 0, 0, 0, 0x01];
    point.extend(50i16.to_be_bytes());
    point.extend(0i16.to_be_bytes());
    let composite = |offset_flags: u16| {
        // ARG_1_AND_2_ARE_WORDS, ARGS_ARE_XY_VALUES and WE_HAVE_A_TWO_BY_TWO.
        let flags = 0x0083 | offset_flags;
        let mut data = vec![0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0];
        data.extend(flags.to_be_bytes());
        data.extend(leaf.to_be_bytes());
        data.extend(100i16.to_be_bytes());
        data.extend(0i16.to_be_bytes());
        // xx = 0, yx = 1.5, xy = -1.5, yy = 0 in F2Dot14.
        for f2dot14 in [0x0000u16, 0x6000, 0xa000, 0x0000] {
            data.extend(f2dot14.to_be_bytes());
        }
        data
    };
    let caret = |offset_flags: u16| {
        let db = db_with(Some(glyf_caret_font(&[(gid, composite(offset_flags)), (leaf, point.clone())])));
        db.face(LIGATURE_CARET_FAMILY, "Regular").unwrap().ligature_carets(u32::from(gid), 1)
    };
    // SCALED_COMPONENT_OFFSET, then with UNSCALED_COMPONENT_OFFSET too.
    assert_eq!(caret(0x0800), Some(vec![150.0]));
    assert_eq!(caret(0x1800), Some(vec![100.0]));
}

/// A contour-point caret on a composite that expands to billions of glyphs with no points gives
/// no caret, and quickly, rather than reading every one of them.
#[test]
fn a_contour_point_caret_on_a_huge_empty_composite_gives_up() {
    let gid = u32::from(source_sans_gid(LIGATURE_CARET_GLYPH).expect("Source Sans 3 has the glyph"));
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let db = db_with(Some(composite_fan_caret_font()));
        let carets = db.face(LIGATURE_CARET_FAMILY, "Regular").map(|face| face.ligature_carets(gid, 1));
        let _ = send.send(carets);
    });
    let carets = receive.recv_timeout(std::time::Duration::from_secs(30)).expect("the caret lookup finishes");
    assert_eq!(carets, Some(None));
}

fn point(text: &str, family: &str) -> TextObject {
    TextObject::point(Point::ZERO, text, CharStyle { font_family: family.into(), size: 100.0, ..CharStyle::default() })
}

/// The caret's x for byte `b`.
fn caret_x(l: &TextLayout, b: usize) -> f64 {
    caret_position(l, b).0.x
}

/// The x of a click at `at` (0 to 1) along glyph `g`'s advance from its left, on its baseline.
fn along(g: &PositionedGlyph, at: f64) -> Point {
    Point::new(g.origin.x + g.advance * at, g.origin.y)
}

/// A font whose caret list for a ligature holds a number of carets other than the one asked for
/// gives none.
#[test]
fn a_caret_list_of_another_length_gives_no_carets() {
    let db = db_with(ligature_caret_font(Some(150)));
    let face = db.face(LIGATURE_CARET_FAMILY, "Regular").expect("the test font is added");
    let gid = u32::from(source_sans_gid(LIGATURE_CARET_GLYPH).expect("Source Sans 3 has the glyph"));
    assert_eq!(face.ligature_carets(gid, 1), Some(vec![150.0]));
    assert_eq!(face.ligature_carets(gid, 2), None);
    assert_eq!(face.ligature_carets(gid, 0), None);
}

/// A variable font's ligature caret with a variation index moves with the face's axis settings: by
/// none at the default weight, by the whole delta at the weight its region peaks at.
#[test]
fn a_variable_ligature_caret_takes_its_delta() {
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(variable_ligature_caret_font(150, 100).expect("the test font builds")), 2, "Regular and Bold");
    let gid = u32::from(source_sans_gid(LIGATURE_CARET_GLYPH).expect("Source Sans 3 has the glyph"));
    let regular = db.face(VARIABLE_LIGATURE_CARET_FAMILY, "Regular").expect("the test font is added");
    let bold = db.face(VARIABLE_LIGATURE_CARET_FAMILY, "Bold").expect("the test font is added");
    assert_eq!(regular.ligature_carets(gid, 1), Some(vec![150.0]));
    assert_eq!(bold.ligature_carets(gid, 1), Some(vec![250.0]));
}

/// The font's ligature caret is where the caret, a click, arrow keys and the selection stop inside
/// the ligature, not the middle of the glyph.
#[test]
fn the_font_ligature_caret_places_the_inner_stop() {
    let db = db_with(ligature_caret_font(Some(150)));
    let face = db.face(LIGATURE_CARET_FAMILY, "Regular").expect("the test font is added");
    let gid = u32::from(source_sans_gid(LIGATURE_CARET_GLYPH).expect("Source Sans 3 has the glyph"));
    let l = layout(&db, &point("fi", LIGATURE_CARET_FAMILY));
    assert_eq!(l.glyphs.len(), 1, "f + i is one glyph: {:?}", l.glyphs);
    let g = &l.glyphs[0];
    assert_eq!((g.gid, g.len), (gid, 2));
    let at = 150.0 / face.advance(gid);
    assert!(at < 0.4, "the caret is well left of the middle: {at}");
    assert_eq!(g.carets.len(), 1);
    assert_eq!(g.carets[0].0, 1);
    assert!((g.carets[0].1 - at).abs() < 1e-9, "{:?} against {at}", g.carets);
    assert!((caret_x(&l, 1) - g.advance * at).abs() < 1e-6);
    // A click left or right of the caret goes to it rather than to either end.
    assert_eq!(hit_byte(&l, along(g, at * 0.4)), 0);
    assert_eq!(hit_byte(&l, along(g, at * 1.4)), 1);
    assert_eq!(hit_byte(&l, along(g, 0.9)), 2);
    // Arrow keys stop at it both ways, and the f alone is selected up to it.
    assert_eq!(caret_horizontal(&l, 0, true), 1);
    assert_eq!(caret_horizontal(&l, 1, true), 2);
    assert_eq!(caret_horizontal(&l, 2, false), 1);
    let q = selection_quads(&l, 0, 1);
    assert_eq!(q.len(), 1);
    assert!((q[0][1].x - q[0][0].x - g.advance * at).abs() < 1e-6, "{q:?}");
}

/// The font's placement of a ligature (`GPOS` x placement) moves its caret with the glyph.
#[test]
fn a_placed_ligature_keeps_its_caret_on_the_glyph() {
    let at = |font: Option<Vec<u8>>| {
        let db = db_with(font);
        let l = layout(&db, &point("fi", LIGATURE_CARET_FAMILY));
        let g = l.glyphs.iter().find(|g| g.len == 2).expect("f + i is one glyph").clone();
        (caret_x(&l, 1) - g.origin.x, g.outline.bounding_box().x0 - g.origin.x, g.width_start, hit_byte(&l, Point::new(caret_x(&l, 1) + 1.0, 0.0)))
    };
    let (caret, ink, start, _) = at(ligature_caret_font(Some(150)));
    let (placed_caret, placed_ink, placed_start, hit) = at(placed_ligature_caret_font(150, 100));
    // 100 units at 100 pt in a 1000-unit em.
    assert!(start.abs() < 1e-9 && (placed_start - 10.0).abs() < 1e-9, "{start} {placed_start}");
    assert!((placed_ink - ink - 10.0).abs() < 1e-6, "the glyph moves right: {placed_ink} against {ink}");
    assert!((placed_caret - caret - 10.0).abs() < 1e-6, "the caret moves with it: {placed_caret} against {caret}");
    assert_eq!(hit, 1);
}

/// A ligature whose font lists no carets for it divides its advance equally among its characters.
#[test]
fn a_ligature_without_font_carets_is_divided_equally() {
    let db = db_with(ligature_caret_font(None));
    let l = layout(&db, &point("fi", LIGATURE_CARET_FAMILY));
    assert_eq!(l.glyphs.len(), 1);
    let g = &l.glyphs[0];
    assert_eq!(g.carets, vec![(1, 0.5)]);
    assert!((caret_x(&l, 1) - g.advance * 0.5).abs() < 1e-6);
    assert_eq!(hit_byte(&l, along(g, 0.4)), 1);
    assert_eq!(hit_byte(&l, along(g, 0.2)), 0);
    // A caret value outside the glyph is not used either.
    let db = db_with(ligature_caret_font(Some(-20)));
    let l = layout(&db, &point("fi", LIGATURE_CARET_FAMILY));
    assert_eq!(l.glyphs[0].carets, vec![(1, 0.5)]);
}

/// Tracking adds space after a ligature's own width: the font's caret stays where the font puts it
/// in the glyph, and the caret, a click and the selection use that place, left to right and right
/// to left.
#[test]
fn tracking_does_not_move_a_ligature_caret() {
    let db = db_with(ligature_caret_font(Some(150)));
    let face = db.face(LIGATURE_CARET_FAMILY, "Regular").expect("the test font is added");
    let mut t = point("fi", LIGATURE_CARET_FAMILY);
    t.runs[0].style.tracking = 50.0;
    let l = layout(&db, &t);
    assert_eq!(l.glyphs.len(), 1, "f + i stays one glyph at this tracking: {:?}", l.glyphs);
    let g = &l.glyphs[0];
    let k = 100.0 / face.units_per_em();
    let width = face.advance(g.gid) * k;
    assert!((g.width - width).abs() < 1e-9, "{} against {width}", g.width);
    assert!((g.advance - width - 5.0).abs() < 1e-9, "the advance takes in 5 pt of tracking: {}", g.advance);
    let x = g.origin.x + 150.0 * k;
    assert!((caret_x(&l, 1) - x).abs() < 1e-6, "{} against {x}", caret_x(&l, 1));
    assert_eq!(hit_byte(&l, Point::new(g.origin.x + 150.0 * k * 0.45, -10.0)), 0);
    assert_eq!(hit_byte(&l, Point::new(x + 0.3, -10.0)), 1);
    let q = selection_quads(&l, 0, 1);
    assert_eq!(q.len(), 1);
    assert!((q[0][1].x - x).abs() < 1e-6, "{q:?}");
    // Right to left the tracking lies to the glyph's right, before its first character: the stop
    // between lam and alef stays in the middle of the glyph's own width.
    let db = db_with(lam_alef_font(b"rlig"));
    let mut t = point("\u{0644}\u{0627}", lam_alef_family(b"rlig"));
    t.runs[0].style.tracking = 50.0;
    let l = layout(&db, &t);
    let g = l.glyphs.iter().find(|g| g.len == 4).expect("lam + alef is one glyph");
    assert!(g.rtl && g.advance > g.width + 4.0, "{} {}", g.advance, g.width);
    let mid = g.origin.x + g.width * 0.5;
    assert!((caret_x(&l, 2) - mid).abs() < 1e-6, "{} against {mid}", caret_x(&l, 2));
    let q = selection_quads(&l, 2, 4);
    assert_eq!(q.len(), 1, "{q:?}");
    let (x0, x1) = (q[0][0].x.min(q[0][1].x), q[0][0].x.max(q[0][1].x));
    assert!((x0 - g.origin.x).abs() < 1e-6 && (x1 - mid).abs() < 1e-6, "{q:?}");
}

/// Negative kerning can end a ligature's advance before its font caret: the caret still stands at
/// the font's place, a click at the advance's end goes to the end of the ligature, and the last
/// character's selection runs between the two.
#[test]
fn negative_kerning_keeps_a_ligature_caret_past_the_advance() {
    let db = db_with(ligature_caret_font(Some(150)));
    let face = db.face(LIGATURE_CARET_FAMILY, "Regular").expect("the test font is added");
    let k = 100.0 / face.units_per_em();
    let gid = u32::from(source_sans_gid(LIGATURE_CARET_GLYPH).expect("Source Sans 3 has the glyph"));
    let width = face.advance(gid) * k;
    // Kerning (thousandths of an em at 100 pt, a tenth of a point each) that leaves an advance of
    // half the distance to the font caret.
    let kern = (150.0 * k * 0.5 - width) * 10.0;
    let mut t = point("fi", LIGATURE_CARET_FAMILY);
    t.runs[0].style.kerning = Some(kern);
    let l = layout(&db, &t);
    assert_eq!(l.glyphs.len(), 1, "f + i stays one glyph with this kerning: {:?}", l.glyphs);
    let g = &l.glyphs[0];
    assert!((g.width - width).abs() < 1e-9, "{} against {width}", g.width);
    assert!((g.advance - 150.0 * k * 0.5).abs() < 1e-6, "the advance ends before the caret: {}", g.advance);
    let x = g.origin.x + 150.0 * k;
    let end = g.origin.x + g.advance;
    assert!((caret_x(&l, 1) - x).abs() < 1e-6, "{} against {x}", caret_x(&l, 1));
    assert!((caret_x(&l, 2) - end).abs() < 1e-6, "{} against {end}", caret_x(&l, 2));
    assert_eq!(hit_byte(&l, Point::new(end, -10.0)), 2);
    assert_eq!(hit_byte(&l, Point::new(x + 0.3, -10.0)), 1);
    let q = selection_quads(&l, 1, 2);
    assert_eq!(q.len(), 1, "{q:?}");
    let (x0, x1) = (q[0][0].x.min(q[0][1].x), q[0][0].x.max(q[0][1].x));
    assert!((x0 - end).abs() < 1e-6 && (x1 - x).abs() < 1e-6, "{q:?}");
    // On a path, a click at the advance's end goes to the end of the ligature as well.
    let mut line = BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((1000.0, 0.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
    let l = layout(&db, &t);
    let g = l.glyphs.iter().find(|g| g.len == 2).expect("f + i is one glyph");
    let d = Vec2::new(g.angle.cos(), g.angle.sin());
    assert_eq!(hit_byte(&l, g.origin + d * g.advance), 2);
    assert_eq!(hit_byte(&l, g.origin + d * (g.width * 150.0 / face.advance(gid) + 0.3)), 1);
}

/// On a path, a ligature whose negative kerning ends its advance before its font caret is followed
/// by an i that covers the caret: a click at the caret goes to the caret, not to the i.
#[test]
fn a_click_at_a_ligature_caret_past_the_advance_on_a_path_reaches_it() {
    let db = db_with(ligature_caret_font(Some(150)));
    let face = db.face(LIGATURE_CARET_FAMILY, "Regular").expect("the test font is added");
    let k = 100.0 / face.units_per_em();
    let gid = u32::from(source_sans_gid(LIGATURE_CARET_GLYPH).expect("Source Sans 3 has the glyph"));
    let width = face.advance(gid) * k;
    let kern = (150.0 * k * 0.5 - width) * 10.0;
    let mut t = point("fii", LIGATURE_CARET_FAMILY);
    crate::edit::style_range(&mut t.runs, 0, 2, |st| st.kerning = Some(kern));
    let mut line = BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((1000.0, 0.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
    let l = layout(&db, &t);
    let g = l.glyphs.iter().find(|g| g.len == 2).expect("f + i is one glyph");
    let i = l.glyphs.iter().find(|h| h.byte == 2).expect("the second i");
    let d = Vec2::new(g.angle.cos(), g.angle.sin());
    let caret = g.origin + d * (g.width * 150.0 / face.advance(gid));
    let mid = |h: &PositionedGlyph| h.origin + d * (h.advance * 0.5);
    assert!((caret - mid(i)).hypot() < (caret - mid(g)).hypot(), "the i's middle is nearer the caret than the ligature's");
    assert_eq!(hit_byte(&l, caret + d * 0.3), 1);
}

/// Tracking of -1000 (one em to the left) leaves Source Serif 4's fi ligature, with standard
/// ligatures turned on explicitly, an advance of zero or less: the caret, a click and the selection
/// still reach the i at its place in the glyph's own width, across and on a path.
#[test]
fn a_ligature_without_advance_keeps_its_inner_stop() {
    let db = FontDb::with_font_dirs(vec![]);
    let mut t = point("fi", "Source Serif 4");
    t.runs[0].style.tracking = -1000.0;
    t.runs[0].style.features = vec!["liga".into()];
    let l = layout(&db, &t);
    let g = l.glyphs.iter().find(|g| g.len == 2).expect("f + i is one glyph");
    assert!(g.advance <= 0.0 && g.width > 0.0, "{} {}", g.advance, g.width);
    let &[(1, f)] = g.carets.as_slice() else { panic!("one inner stop: {:?}", g.carets) };
    let x = g.origin.x + g.width * f;
    let end = g.origin.x + g.advance;
    assert!((caret_x(&l, 1) - x).abs() < 1e-6, "{} against {x}", caret_x(&l, 1));
    assert!((caret_x(&l, 2) - end).abs() < 1e-6, "{} against {end}", caret_x(&l, 2));
    assert_eq!(hit_byte(&l, Point::new(x + 0.3, -10.0)), 1);
    assert_eq!(hit_byte(&l, Point::new(g.origin.x + 0.3, -10.0)), 0);
    let q = selection_quads(&l, 0, 1);
    assert_eq!(q.len(), 1, "{q:?}");
    let (x0, x1) = (q[0][0].x.min(q[0][1].x), q[0][0].x.max(q[0][1].x));
    assert!((x0 - g.origin.x).abs() < 1e-6 && (x1 - x).abs() < 1e-6, "{q:?} against {} {x}", g.origin.x);
    assert_eq!(caret_horizontal(&l, 0, true), 1);
    // On a path, a click past the inner stop goes to it, and the f alone is selected up to it.
    let mut line = BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((1000.0, 0.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
    let l = layout(&db, &t);
    let g = l.glyphs.iter().find(|g| g.len == 2).expect("f + i is one glyph");
    assert!(g.advance <= 0.0, "{}", g.advance);
    let d = Vec2::new(g.angle.cos(), g.angle.sin());
    assert_eq!(hit_byte(&l, g.origin + d * (g.width * f + 0.3)), 1);
    assert_eq!(hit_byte(&l, g.origin + d * 0.3), 0);
    let q = selection_quads(&l, 0, 1);
    assert_eq!(q.len(), 1, "{q:?}");
    assert!(((q[0][1] - q[0][0]).hypot() - g.width * f).abs() < 1e-6, "{q:?}");
}

/// Bundled Source Serif 4's fi ligature has a caret stop that a click and the arrow keys reach.
#[test]
fn source_serif_fi_and_ffi_have_inner_stops() {
    let db = FontDb::with_font_dirs(vec![]);
    for (s, stops) in [("fi", vec![1]), ("ffi", vec![1, 2])] {
        let l = layout(&db, &point(s, "Source Serif 4"));
        let lone: Vec<&PositionedGlyph> = l.glyphs.iter().filter(|g| g.len == s.len()).collect();
        if lone.is_empty() {
            // The font has no ligature for these letters (ffi in some versions).
            assert_ne!(s, "fi", "{:?}", l.glyphs);
            continue;
        }
        let g = lone[0];
        assert_eq!(g.carets.iter().map(|c| c.0).collect::<Vec<_>>(), stops, "{s}");
        for &(b, f) in &g.carets {
            let x = g.origin.x + g.advance * f;
            assert_eq!(hit_byte(&l, Point::new(x + 0.01, -10.0)), b, "{s} byte {b}");
            assert!((caret_x(&l, b) - x).abs() < 1e-6);
        }
        let mut b = 0;
        let mut seen = vec![];
        while b < s.len() {
            b = caret_horizontal(&l, b, true);
            seen.push(b);
        }
        assert_eq!(seen[..stops.len()], stops[..], "{s}");
    }
}

/// A right-to-left ligature (lam + alef, two bytes each) has its stop between the two characters,
/// never inside a character, and its first character is on its right.
#[test]
fn a_right_to_left_ligature_stops_between_its_characters() {
    let db = db_with(lam_alef_font(b"rlig"));
    let l = layout(&db, &point("\u{0644}\u{0627}", lam_alef_family(b"rlig")));
    let g = l.glyphs.iter().find(|g| g.len == 4).expect("lam + alef is one glyph");
    assert!(g.rtl);
    assert_eq!(g.carets, vec![(2, 0.5)]);
    // Right of the middle is the lam, left of it the alef.
    assert_eq!(hit_byte(&l, along(g, 0.9)), 0);
    assert_eq!(hit_byte(&l, along(g, 0.6)), 2);
    assert_eq!(hit_byte(&l, along(g, 0.1)), 4);
    assert!((caret_x(&l, 2) - (g.origin.x + g.advance * 0.5)).abs() < 1e-6);
    assert!(caret_x(&l, 0) > caret_x(&l, 2) && caret_x(&l, 2) > caret_x(&l, 4));
    // The lam alone is selected on the right half.
    let q = selection_quads(&l, 0, 2);
    assert_eq!(q.len(), 1, "{q:?}");
    let (x0, x1) = (q[0][0].x.min(q[0][1].x), q[0][0].x.max(q[0][1].x));
    assert!((x0 - (g.origin.x + g.advance * 0.5)).abs() < 1e-6 && (x1 - (g.origin.x + g.advance)).abs() < 1e-6, "{q:?}");
    // Arrow keys (visual stops) take the inner stop: right to left, left goes forward.
    assert_eq!(caret_horizontal_cluster(&l, "\u{0644}\u{0627}", 0, false), 2);
    assert_eq!(caret_horizontal_cluster(&l, "\u{0644}\u{0627}", 2, false), 4);
}

/// Right to left, manual kerning moves a ligature right and puts its space on the left: the inner
/// stop moves with the glyph, and the click, the caret and the selection of each character follow.
#[test]
fn manual_kerning_moves_a_right_to_left_ligature_stop_with_the_glyph() {
    let db = db_with(lam_alef_font(b"rlig"));
    let at = |kern: f64| {
        let mut t = point("\u{0644}\u{0627}", lam_alef_family(b"rlig"));
        t.runs[0].style.kerning = Some(kern);
        let l = layout(&db, &t);
        let g = l.glyphs.iter().find(|g| g.len == 4).expect("lam + alef is one glyph").clone();
        assert!(g.rtl);
        (l, g)
    };
    let (plain, p) = at(0.0);
    let (kerned, k) = at(100.0);
    // 100/1000 em at 100 pt.
    assert!((k.width_start - 10.0).abs() < 1e-9 && (k.advance - p.advance - 10.0).abs() < 1e-9, "{} {} {}", k.width_start, k.advance, p.advance);
    let ink = |g: &PositionedGlyph| g.outline.bounding_box().x0;
    assert!((ink(&k) - k.origin.x - (ink(&p) - p.origin.x) - 10.0).abs() < 1e-6, "the glyph moves right");
    let mid = k.origin.x + 10.0 + k.width * 0.5;
    assert!((caret_x(&kerned, 2) - ink(&k) - (caret_x(&plain, 2) - ink(&p))).abs() < 1e-6, "the stop moves with the glyph");
    assert!((caret_x(&kerned, 2) - mid).abs() < 1e-6, "{} against {mid}", caret_x(&kerned, 2));
    assert_eq!(hit_byte(&kerned, Point::new(mid + 1.0, k.origin.y)), 2);
    assert_eq!(hit_byte(&kerned, Point::new(mid + 4.0, k.origin.y)), 2);
    assert_eq!(hit_byte(&kerned, Point::new(k.origin.x + k.advance - 1.0, k.origin.y)), 0);
    let q = selection_quads(&kerned, 0, 2);
    assert_eq!(q.len(), 1, "{q:?}");
    let x0 = q[0][0].x.min(q[0][1].x);
    assert!((x0 - mid).abs() < 1e-6, "the lam alone is selected right of the stop: {q:?}");
}

/// A letter with a combining mark is one character: a glyph for it has no inner stop.
#[test]
fn a_letter_and_its_mark_has_no_inner_stop() {
    let db = FontDb::with_font_dirs(vec![]);
    let l = layout(&db, &point("e\u{0301}", "Source Sans 3"));
    assert!(l.glyphs.iter().all(|g| g.carets.is_empty()), "{:?}", l.glyphs);
}

/// On a path, a click on either half of the ligature goes to its nearer stop, and the selection of
/// one character covers its part of the glyph.
#[test]
fn a_ligature_on_a_path_has_its_inner_stop() {
    let db = db_with(ligature_caret_font(None));
    let mut line = BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((1000.0, 0.0));
    let mut t = point("fi", LIGATURE_CARET_FAMILY);
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
    let l = layout(&db, &t);
    assert!(l.on_path);
    let g = l.glyphs.iter().find(|g| g.len == 2).expect("f + i is one glyph");
    assert_eq!(g.carets, vec![(1, 0.5)]);
    let at = |f: f64| g.origin + Vec2::new(g.angle.cos(), g.angle.sin()) * (g.advance * f);
    assert_eq!(hit_byte(&l, at(0.1)), 0);
    assert_eq!(hit_byte(&l, at(0.4)), 1);
    assert_eq!(hit_byte(&l, at(0.95)), 2);
    let q = selection_quads(&l, 1, 2);
    assert_eq!(q.len(), 1);
    assert!(((q[0][1] - q[0][0]).hypot() - g.advance * 0.5).abs() < 1e-6, "{q:?}");
}

/// In vertical type, the inner stop of a turned Latin ligature lies down the column between its
/// ends.
#[test]
fn a_ligature_in_vertical_type_has_its_inner_stop() {
    let db = db_with(ligature_caret_font(None));
    let mut t = point("fi", LIGATURE_CARET_FAMILY);
    t.vertical = true;
    let l = layout(&db, &t);
    let Some(g) = l.glyphs.iter().find(|g| g.len == 2) else { panic!("f + i is one glyph: {:?}", l.glyphs) };
    assert_eq!(g.carets, vec![(1, 0.5)]);
    let y = |b| caret_position(&l, b).0.y;
    assert!(y(0) < y(1) && y(1) < y(2), "{} {} {}", y(0), y(1), y(2));
    let mid = caret_position(&l, 1);
    assert_eq!(hit_byte(&l, mid.0.midpoint(mid.1) + Vec2::new(0.0, 0.01)), 1);
}

/// In vertical type, tracking adds space after a turned ligature's own width as it does across:
/// the font's caret stays the same distance down the column from the ligature's start, and the
/// last character's selection runs from it to the end of the advance.
#[test]
fn tracking_does_not_move_a_ligature_caret_in_vertical_type() {
    let db = db_with(ligature_caret_font(Some(150)));
    let face = db.face(LIGATURE_CARET_FAMILY, "Regular").expect("the test font is added");
    let k = 100.0 / face.units_per_em();
    let at = |tracking: f64| {
        let mut t = point("fi", LIGATURE_CARET_FAMILY);
        t.vertical = true;
        t.runs[0].style.tracking = tracking;
        let l = layout(&db, &t);
        let Some(g) = l.glyphs.iter().find(|g| g.len == 2) else { panic!("f + i is one glyph: {:?}", l.glyphs) };
        let width = face.advance(g.gid) * k;
        assert!((g.width - width).abs() < 1e-9, "{} against {width}", g.width);
        let y = |b| caret_position(&l, b).0.y;
        let q = selection_quads(&l, 1, 2);
        assert_eq!(q.len(), 1, "{q:?}");
        let ys = q[0].map(|p| p.y);
        let (y0, y1) = (ys.iter().copied().fold(f64::INFINITY, f64::min), ys.iter().copied().fold(f64::NEG_INFINITY, f64::max));
        assert!((y0 - y(1)).abs() < 1e-6 && (y1 - y(2)).abs() < 1e-6, "{q:?} {} {}", y(1), y(2));
        (y(1) - y(0), y(2) - y(0), g.advance)
    };
    let (plain, plain_end, _) = at(0.0);
    let (tracked, tracked_end, advance) = at(50.0);
    assert!((plain - 150.0 * k).abs() < 1e-6, "{plain} against {}", 150.0 * k);
    assert!((tracked - plain).abs() < 1e-6, "{tracked} against {plain}");
    assert!((tracked_end - advance).abs() < 1e-6 && (tracked_end - plain_end - 5.0).abs() < 1e-6, "{tracked_end} {plain_end} {advance}");
}

/// A ligature standing upright in vertical type (Standard Vertical Roman Alignment) takes its font
/// caret as a y coordinate: the stop lies that far below the top of its cell, which runs from its
/// vertical origin (0.88 em) one em down the column. A caret above the cell is not used.
#[test]
fn an_upright_ligature_takes_its_caret_down_its_cell() {
    let at = |caret: i16| {
        let db = db_with(vertical_ligature_caret_font(caret));
        let mut t = point("fi", VERTICAL_LIGATURE_CARET_FAMILY);
        t.vertical = true;
        t.runs[0].style.standard_vertical_roman = true;
        let l = layout(&db, &t);
        let Some(g) = l.glyphs.iter().find(|g| g.len == 2) else { panic!("f + i is one glyph: {:?}", l.glyphs) };
        let face = db.face(VERTICAL_LIGATURE_CARET_FAMILY, "Regular").expect("the test font is added");
        let cell = 1000.0 * 100.0 / face.units_per_em();
        assert!((g.width - cell).abs() < 1e-9, "the glyph stands upright, one em down the column: {} against {cell}", g.width);
        let &[(1, f)] = g.carets.as_slice() else { panic!("one inner stop: {:?}", g.carets) };
        let y = |b| caret_position(&l, b).0.y;
        assert!(((y(1) - y(0)) - g.width * f).abs() < 1e-6, "{} {} against {f}", y(0), y(1));
        f
    };
    // 0.88 em at the top: y = 280 is 600 units down, y = -50 is 930 units down.
    assert!((at(280) - 0.6).abs() < 1e-9, "{}", at(280));
    assert!((at(-50) - 0.93).abs() < 1e-9, "{}", at(-50));
    assert!((at(900) - 0.5).abs() < 1e-9, "{}", at(900));
}

/// Horizontal type does not read Standard Vertical Roman Alignment or the tate-chu-yoko offsets, so
/// setting one of them on the f of "fi" keeps the ligature. Vertical type reads them per glyph, so
/// there the f is shaped apart from the i.
#[test]
fn vertical_only_settings_keep_a_horizontal_ligature_whole() {
    let db = db_with(vertical_ligature_caret_font(280));
    let marks: [fn(&mut CharStyle); 3] = [|s| s.standard_vertical_roman = true, |s| s.tcy_up_down = 10.0, |s| s.tcy_left_right = 10.0];
    for (i, mark) in marks.into_iter().enumerate() {
        for vertical in [false, true] {
            let mut t = point("fi", VERTICAL_LIGATURE_CARET_FAMILY);
            t.vertical = vertical;
            let plain = t.runs[0].style.clone();
            let mut marked = plain.clone();
            mark(&mut marked);
            t.runs = vec![
                vectorcraft_doc::TextRun { text: "f".into(), style: marked, inline: None },
                vectorcraft_doc::TextRun { text: "i".into(), style: plain, inline: None },
            ];
            let l = layout(&db, &t);
            let joined = l.glyphs.iter().any(|g| g.len == 2);
            assert_eq!(joined, !vertical, "setting {i}, vertical {vertical}: {:?}", l.glyphs);
        }
    }
}

/// On a path, an upright ligature in vertical type stands with its top toward the path's start, and
/// its cell runs one em from the pen, as in vertical point type, with or without the font's vertical
/// metrics. The inner stop lies at the middle of that cell, and the first character's selection ends
/// there.
#[test]
fn an_upright_ligature_on_a_path_takes_its_stop_in_its_turned_cell() {
    for (font, family) in [(ligature_caret_font(None), LIGATURE_CARET_FAMILY), (vertical_ligature_caret_font(900), VERTICAL_LIGATURE_CARET_FAMILY)] {
        let db = db_with(font);
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((1000.0, 0.0));
        let mut t = point("fi", family);
        t.vertical = true;
        t.runs[0].style.standard_vertical_roman = true;
        t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
        let l = layout(&db, &t);
        assert!(l.on_path);
        let Some(g) = l.glyphs.iter().find(|g| g.len == 2) else { panic!("f + i is one glyph: {:?}", l.glyphs) };
        assert_eq!(g.carets, vec![(1, 0.5)], "{family}");
        assert!((g.width - 100.0).abs() < 1e-9 && g.width_start.abs() < 1e-9, "{family}: {} {}", g.width, g.width_start);
        let x = |b| caret_position(&l, b).0.x - g.origin.x;
        assert!((x(1) - 50.0).abs() < 1e-6, "{family}: {}", x(1));
        let q = selection_quads(&l, 0, 1);
        assert_eq!(q.len(), 1, "{q:?}");
        assert!(((q[0][1] - q[0][0]).hypot() - 50.0).abs() < 1e-6, "{family}: {q:?}");
        // A baseline shift moves the turned glyph across the path, and leaves its stop in place.
        let ink = g.outline.bounding_box();
        t.runs[0].style.baseline_shift = 10.0;
        let l = layout(&db, &t);
        let Some(s) = l.glyphs.iter().find(|g| g.len == 2) else { panic!("f + i is one glyph: {:?}", l.glyphs) };
        let moved = s.outline.bounding_box().center() - ink.center();
        assert!((moved.x.abs()) < 1e-6 && (moved.y + 10.0).abs() < 1e-6, "{family}: the shift moves the glyph to the path's left: {moved:?}");
        let stop = caret_position(&l, 1).0.x - s.origin.x;
        assert!((stop - 50.0).abs() < 1e-6, "{family}: {stop}");
        // Horizontal Scale widens the glyph across the path; its stop stays inside the glyph.
        t.runs[0].style.baseline_shift = 0.0;
        t.runs[0].style.h_scale = 200.0;
        let l = layout(&db, &t);
        let Some(s) = l.glyphs.iter().find(|g| g.len == 2) else { panic!("f + i is one glyph: {:?}", l.glyphs) };
        let bounds = s.outline.bounding_box();
        let stop = caret_position(&l, 1).0.x - s.origin.x;
        assert!(bounds.x0 < stop && stop < bounds.x1 && stop > 0.0, "{family}: {stop} outside {bounds:?}");
        t.runs[0].style.h_scale = 100.0;
        // Skew on a slanted path: the stop stays inside the drawn glyph along the path.
        let mut slant = BezPath::new();
        slant.move_to((0.0, 0.0));
        slant.line_to((1000.0, 1000.0));
        t.runs[0].style.baseline_shift = 0.0;
        t.kind = TextKind::OnPath { path: PathData::from_bezpath(&slant), start: 0.0, end: None };
        t.path_effect = vectorcraft_doc::PathEffect::Skew;
        let l = layout(&db, &t);
        let Some(s) = l.glyphs.iter().find(|g| g.len == 2) else { panic!("f + i is one glyph: {:?}", l.glyphs) };
        let tangent = Vec2::new(s.angle.cos(), s.angle.sin());
        let ink = s.outline.elements().iter().filter_map(|el| el.end_point()).map(|p| (p - s.origin).dot(tangent));
        let (lo, hi) = ink.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), d| (lo.min(d), hi.max(d)));
        let (a, b) = caret_position(&l, 1);
        let stop = (a.midpoint(b) - s.origin).dot(tangent);
        assert!(lo < stop && stop < hi, "{family}: {stop} outside {lo}..{hi}");
    }
}

/// On a path, an upright ligature in vertical type reads along the path in order: the top of its
/// cell comes first, and its font caret (600 units down the cell) is drawn where the caret, a click
/// and the selection put the stop, on a path running either way with any path effect. A glyph lying
/// on its side is a quarter turn clockwise from that, as horizontal type on the path sets it.
#[test]
fn an_upright_ligature_on_a_path_reads_along_it_in_order() {
    use vectorcraft_doc::PathEffect;
    let db = db_with(vertical_ligature_caret_font(280));
    let mut t = point("fi", VERTICAL_LIGATURE_CARET_FAMILY);
    t.vertical = true;
    t.runs[0].style.standard_vertical_roman = true;
    for (to, effect) in [
        (1000.0, PathEffect::Rainbow),
        (-1000.0, PathEffect::Rainbow),
        (-1000.0, PathEffect::Skew),
        (-1000.0, PathEffect::Ribbon3d),
        (-1000.0, PathEffect::StairStep),
        (-1000.0, PathEffect::Gravity),
    ] {
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((to, 0.0));
        t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
        t.path_effect = effect;
        let l = layout(&db, &t);
        let Some(g) = l.glyphs.iter().find(|g| g.len == 2) else { panic!("f + i is one glyph: {:?}", l.glyphs) };
        let &[(1, f)] = g.carets.as_slice() else { panic!("one inner stop: {:?}", g.carets) };
        assert!((f - 0.6).abs() < 1e-9, "{f}");
        let tangent = Vec2::new(g.angle.cos(), g.angle.sin());
        let along = |p: Point| (p - g.origin).dot(tangent);
        // Font units, y down: the cell's top is 880 units above the baseline and its foot 120 below.
        let (top, foot) = (along(g.xf * Point::new(0.0, -880.0)), along(g.xf * Point::new(0.0, 120.0)));
        assert!(top < foot, "{to} {effect:?}: the top comes first along the path: {top} {foot}");
        let (start, width) = (g.width_start, g.width);
        assert!((top - start).abs() < 1e-6 && (foot - top - width).abs() < 1e-6, "{to} {effect:?}: {top} {foot} {start} {width}");
        let drawn = along(g.xf * Point::new(0.0, -280.0));
        assert!((drawn - (top + (foot - top) * 0.6)).abs() < 1e-6, "{to} {effect:?}: {drawn} against {top} {foot}");
        let (a, b) = caret_position(&l, 1);
        let caret = along(a.midpoint(b));
        assert!((caret - drawn).abs() < 1e-6, "{to} {effect:?}: the caret stands at the drawn stop: {caret} against {drawn}");
        assert_eq!(hit_byte(&l, g.origin + tangent * (drawn + 0.3)), 1, "{to} {effect:?}");
        assert_eq!(hit_byte(&l, g.origin + tangent * (drawn - 0.3)), 1, "{to} {effect:?}");
        assert_eq!(hit_byte(&l, g.origin + tangent * 0.3), 0, "{to} {effect:?}");
        let q = selection_quads(&l, 0, 1);
        assert_eq!(q.len(), 1, "{q:?}");
        let (q0, q1) = (along(q[0][0]), along(q[0][1]));
        assert!((q0.min(q1) - drawn.min(0.0)).abs() < 1e-6 && (q0.max(q1) - drawn.max(0.0)).abs() < 1e-6, "{to} {effect:?}: {q:?} against {drawn}");
    }
    // Without Standard Vertical Roman Alignment the letter lies on its side: its top faces the
    // path's left and its advance runs along the path.
    let mut line = BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((1000.0, 0.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
    t.path_effect = PathEffect::Rainbow;
    t.runs[0].style.standard_vertical_roman = false;
    let l = layout(&db, &t);
    let Some(s) = l.glyphs.iter().find(|g| g.len == 2) else { panic!("f + i is one glyph: {:?}", l.glyphs) };
    let tangent = Vec2::new(s.angle.cos(), s.angle.sin());
    let left = Vec2::new(tangent.y, -tangent.x);
    let (pen, up, ahead) = (s.xf * Point::ZERO, s.xf * Point::new(0.0, -500.0), s.xf * Point::new(500.0, 0.0));
    assert!((up - pen).dot(left) > 1.0 && (ahead - pen).dot(tangent) > 1.0, "{pen:?} {up:?} {ahead:?}");
}

/// On a path, an upright ligature whose cell Proportional Metrics shortens (`vpal`, from 1000 units
/// to 800) keeps its font caret (600 units down the cell) where it is drawn: the inner stop lies at
/// 600 / 800 of its width, and the caret stands at the drawn stop.
#[test]
fn an_upright_ligature_on_a_path_keeps_its_font_caret_on_a_proportional_cell() {
    use vectorcraft_doc::PathEffect;
    let db = db_with(proportional_vertical_ligature_caret_font(280, -200));
    let mut t = point("fi", VERTICAL_LIGATURE_CARET_FAMILY);
    t.vertical = true;
    t.runs[0].style.standard_vertical_roman = true;
    t.runs[0].style.proportional_metrics = true;
    let mut line = BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((1000.0, 0.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
    t.path_effect = PathEffect::Rainbow;
    let l = layout(&db, &t);
    let Some(g) = l.glyphs.iter().find(|g| g.len == 2) else { panic!("f + i is one glyph: {:?}", l.glyphs) };
    assert!((g.width - 80.0).abs() < 1e-6, "the cell is 800 units at 100 pt: {}", g.width);
    let &[(1, f)] = g.carets.as_slice() else { panic!("one inner stop: {:?}", g.carets) };
    assert!((f - 0.75).abs() < 1e-9, "{f}");
    let tangent = Vec2::new(g.angle.cos(), g.angle.sin());
    let along = |p: Point| (p - g.origin).dot(tangent);
    let (top, drawn) = (along(g.xf * Point::new(0.0, -880.0)), along(g.xf * Point::new(0.0, -280.0)));
    assert!((top - g.width_start).abs() < 1e-6 && (drawn - top - 60.0).abs() < 1e-6, "{top} {drawn} {}", g.width_start);
    let (a, b) = caret_position(&l, 1);
    let caret = along(a.midpoint(b));
    assert!((caret - drawn).abs() < 1e-6, "the caret stands at the drawn stop: {caret} against {drawn}");
}

/// Tracking that makes an upright ligature's advance negative leaves it upright: it keeps its
/// caret down its cell, and its cell starts where it does without tracking, with or without the
/// font's vertical metrics.
#[test]
fn an_upright_ligature_with_a_negative_advance_keeps_its_cell() {
    let at = |font: Option<Vec<u8>>, family: &str, tracking: f64| {
        let db = db_with(font);
        let mut t = point("fi", family);
        t.vertical = true;
        t.runs[0].style.standard_vertical_roman = true;
        t.runs[0].style.tracking = tracking;
        // Ligatures turned on by name stay on at any tracking.
        t.runs[0].style.features = vec!["+liga".into()];
        let l = layout(&db, &t);
        let Some(g) = l.glyphs.iter().find(|g| g.len == 2) else { panic!("f + i is one glyph: {:?}", l.glyphs) };
        let face = db.face(family, "Regular").expect("the test font is added");
        assert!(tracking == 0.0 || g.advance <= 0.0, "the tracking makes the advance negative: {}", g.advance);
        let cell = 1000.0 * 100.0 / face.units_per_em();
        assert!((g.width - cell).abs() < 1e-9, "the glyph stands upright, one em down the column: {} against {cell}", g.width);
        let y = |b| caret_position(&l, b).0.y;
        let &[(1, f)] = g.carets.as_slice() else { panic!("one inner stop: {:?}", g.carets) };
        assert!(((y(1) - y(0)) - g.width * f).abs() < 1e-6, "{} {} against {f}", y(0), y(1));
        (f, g.outline.bounding_box().y0 - y(0))
    };
    let vertical = |tracking| at(vertical_ligature_caret_font(280), VERTICAL_LIGATURE_CARET_FAMILY, tracking);
    let (f, top) = vertical(0.0);
    assert!((f - 0.6).abs() < 1e-9, "{f}");
    let (tracked, tracked_top) = vertical(-1500.0);
    assert!((tracked - f).abs() < 1e-9, "the caret is read down the cell: {tracked} against {f}");
    assert!((tracked_top - top).abs() < 1e-6, "{tracked_top} against {top}");
    let plain = |tracking| at(ligature_caret_font(None), LIGATURE_CARET_FAMILY, tracking);
    let (f, top) = plain(0.0);
    assert!((f - 0.5).abs() < 1e-9, "{f}");
    let (tracked, tracked_top) = plain(-1500.0);
    assert!((tracked - f).abs() < 1e-9, "{tracked} against {f}");
    assert!((tracked_top - top).abs() < 1e-6, "the cell starts where it does without tracking: {tracked_top} against {top}");
}
