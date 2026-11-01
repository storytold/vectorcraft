//! Colour bitmap glyphs (`sbix`, `CBDT`): laid out as images in place of outlines, in the box
//! their strike gives, moving with the type.

use kurbo::{Point, Rect};
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use vectorcraft_doc::CharStyle;

use crate::test_fonts::{COLOUR_CBDT_BEARINGS, COLOUR_CHAR, COLOUR_PPEM, colour_bitmap_family, colour_bitmap_font};
use crate::*;

fn db(sbix: bool) -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(colour_bitmap_font(sbix).expect("the test font builds")), 1);
    db
}

fn style(sbix: bool) -> CharStyle {
    CharStyle { font_family: colour_bitmap_family(sbix).into(), size: 100.0, ..CharStyle::default() }
}

/// The text-space box of glyph `g`'s image.
fn image_box(g: &PositionedGlyph) -> Rect {
    let (image, xf) = g.placed_image().expect("the glyph is an image");
    xf.transform_rect_bbox(image.rect())
}

fn assert_near(a: Rect, b: Rect) {
    let d = [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1];
    assert!(d.iter().all(|d| d.abs() < 1e-6), "{a:?} is not {b:?}");
}

/// The left side bearing of [`COLOUR_CHAR`] in the bundled Source Sans 3 (font units of 1000).
fn lsb() -> f64 {
    let font = skrifa::FontRef::new(include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf")).unwrap();
    let gid = font.charmap().map(COLOUR_CHAR).unwrap();
    f64::from(font.glyph_metrics(Size::unscaled(), LocationRef::default()).left_side_bearing(gid).unwrap())
}

#[test]
fn an_sbix_glyph_is_its_largest_strike_one_em_wide_sitting_on_the_shifted_baseline() {
    let db = db(true);
    let l = layout(&db, &TextObject::point(Point::new(10.0, 50.0), &COLOUR_CHAR.to_string(), style(true)));
    let g = &l.glyphs[0];
    assert!(g.outline.elements().is_empty(), "the image replaces the outline");
    let image = g.image.as_ref().unwrap();
    assert_eq!((image.width, image.height), (u32::from(COLOUR_PPEM), u32::from(COLOUR_PPEM)), "the largest strike");
    // From the glyph's left side bearing, its bottom 0.128 em below the baseline.
    let x = g.origin.x + lsb() * 0.1;
    assert_near(image_box(g), Rect::new(x, g.origin.y - 87.2, x + 100.0, g.origin.y + 12.8));
    assert!(l.bounds.contains_rect(image_box(g)), "the bounds hold the image: {:?}", l.bounds);
}

#[test]
fn a_cbdt_glyph_sits_where_its_bearings_put_it() {
    let db = db(false);
    let l = layout(&db, &TextObject::point(Point::new(10.0, 50.0), &COLOUR_CHAR.to_string(), style(false)));
    let g = &l.glyphs[0];
    assert!(g.outline.elements().is_empty());
    let (bx, by) = COLOUR_CBDT_BEARINGS;
    let px = 100.0 / f64::from(COLOUR_PPEM);
    let (x, top) = (g.origin.x + f64::from(bx) * px, g.origin.y - f64::from(by) * px);
    assert_near(image_box(g), Rect::new(x, top, x + 100.0, top + 100.0));
}

#[test]
fn a_bitmap_glyph_scales_with_the_type() {
    let db = db(false);
    let st = CharStyle { h_scale: 200.0, ..style(false) };
    let l = layout(&db, &TextObject::point(Point::ZERO, &COLOUR_CHAR.to_string(), st));
    let b = image_box(&l.glyphs[0]);
    assert!((b.width() - 200.0).abs() < 1e-6 && (b.height() - 100.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn a_bitmap_glyph_moves_when_type_snaps_to_pixels() {
    let db = db(false);
    let mut l = layout(&db, &TextObject::point(Point::ZERO, &COLOUR_CHAR.to_string(), style(false)));
    let (before, origin) = (image_box(&l.glyphs[0]), l.glyphs[0].origin);
    assert!(l.snap_to_pixels(Affine::translate((0.3, 0.4))));
    let g = &l.glyphs[0];
    assert!(g.origin != origin);
    assert_near(image_box(g), before + (g.origin - origin));
}

#[test]
fn other_glyphs_of_a_colour_font_keep_their_outlines() {
    let db = db(true);
    let l = layout(&db, &TextObject::point(Point::ZERO, "F", style(true)));
    assert!(l.glyphs[0].image.is_none());
    assert!(!l.glyphs[0].outline.elements().is_empty());
}

/// A strike PNG past the caps, or whose header doesn't read, gives the glyph no image: it is not
/// decoded (a font is untrusted input).
#[test]
fn a_bitmap_past_the_caps_or_without_a_header_is_no_image() {
    use crate::bitmap::{MAX_PNG_BYTES, MAX_SIDE};
    use crate::test_fonts::{COLOUR_LARGE, colour_bitmap_font_with, solid_png};
    let good = solid_png(COLOUR_PPEM.into(), COLOUR_LARGE);
    // A header claiming a side past the cap (the IHDR width).
    let mut wide = good.clone();
    wide[16..20].copy_from_slice(&(MAX_SIDE + 1).to_be_bytes());
    // More bytes than a glyph's PNG may have.
    let mut long = good.clone();
    long.resize(MAX_PNG_BYTES + 1, 0);
    let headless = good[8..].to_vec();
    for sbix in [true, false] {
        for (what, png) in [("wide", &wide), ("long", &long), ("headless", &headless), ("good", &good)] {
            let db = FontDb::with_font_dirs(vec![]);
            assert_eq!(db.add_font(colour_bitmap_font_with(sbix, png).unwrap()), 1);
            let l = layout(&db, &TextObject::point(Point::ZERO, &COLOUR_CHAR.to_string(), style(sbix)));
            assert_eq!(l.glyphs[0].image.is_some(), what == "good", "{what} (sbix {sbix})");
        }
    }
}

/// A colour bitmap font without outline tables (Noto Color Emoji has `CBDT` but no `glyf`) is an
/// installed font like any other.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_bitmap_only_font_file_is_cataloged() {
    use crate::fontdb::file_face_names_within;
    let full = colour_bitmap_font(false).unwrap();
    let font = skrifa::FontRef::new(&full).unwrap();
    let mut builder = write_fonts::FontBuilder::new();
    for r in font.table_directory.table_records() {
        if ![*b"glyf", *b"loca"].contains(&r.tag().to_be_bytes()) {
            builder.add_raw(write_fonts::types::Tag::new(&r.tag().to_be_bytes()), font.table_data(r.tag()).unwrap().as_bytes());
        }
    }
    let dir = std::env::temp_dir().join(format!("vc-bitmaps-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Bitmaps.ttf");
    std::fs::write(&file, builder.build()).unwrap();
    let faces = file_face_names_within(&file, 4, u64::MAX);
    assert_eq!(faces.len(), 1, "the face is cataloged");
}

/// The system's colour emoji font, when installed: an emoji draws as an image, and a ZWJ sequence
/// as one glyph.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn an_installed_colour_emoji_font_draws_emoji_as_images() {
    let db = FontDb::with_font_dirs(system_font_dirs());
    let Some(family) = ["Apple Color Emoji", "Noto Color Emoji"].into_iter().find(|f| db.has_family(f)) else {
        eprintln!("no colour emoji font installed: skipped");
        return;
    };
    let st = CharStyle { font_family: family.into(), size: 40.0, ..CharStyle::default() };
    let l = layout(&db, &TextObject::point(Point::ZERO, "\u{1F600}", st.clone()));
    assert!(l.glyphs[0].image.is_some(), "{family} draws 😀 as an image");
    let l = layout(&db, &TextObject::point(Point::ZERO, "\u{1F469}\u{200D}\u{1F4BB}", st));
    assert_eq!(l.glyphs.iter().filter(|g| g.image.is_some()).count(), 1, "the sequence is one glyph");
}
