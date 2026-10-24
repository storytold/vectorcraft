//! OpenType SVG glyphs: layout asks the reader the app registers (the SVG importer, a layer up)
//! for each glyph's art, and carries it as the glyph's colour content.

use std::sync::Arc;

use kurbo::{Point, Rect, Shape};
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Node, NodeId, TextObject};
use vectorcraft_geom::PathData;

use super::test_fonts::{SVG_CHAR, source_sans_gid, svg_font};

/// The family of the SVG test fonts here (each database holds one).
const FAMILY: &str = "Svgtest Glyphs";
use super::{FontDb, layout, set_svg_glyph_reader};

/// Stands in for the SVG importer, which this crate can't use (it is a layer up): a square one em
/// wide in the fill, stroked when there is a stroke, for a document that has the glyph's element.
fn reader(document: &[u8], gid: u32, _upem: f64, fill: Color, stroke: Option<Color>) -> Option<(Node, bool)> {
    let text = std::str::from_utf8(document).ok()?;
    if !text.contains(&format!("id=\"glyph{gid}\"")) {
        return None;
    }
    let coloured = text.contains("context-");
    let path = PathData::from_bezpath(&Rect::new(0.0, -700.0, 1000.0, 0.0).to_path(0.1));
    let stroke = stroke.map_or(Paint::None, Paint::solid);
    let art = Node::group(NodeId(u64::MAX), vec![Arc::new(Node::path(NodeId(u64::MAX), path, Appearance::basic(Paint::solid(fill), stroke, 10.0)))]);
    Some((art, coloured))
}

fn gid(ch: char) -> u16 {
    source_sans_gid(ch).unwrap()
}

/// A font database with the SVG test font of `document` for [`SVG_CHAR`]'s glyph.
fn db(document: &str) -> FontDb {
    set_svg_glyph_reader(reader);
    let db = FontDb::with_font_dirs(vec![]);
    let g = gid(SVG_CHAR);
    assert_eq!(db.add_font(svg_font(FAMILY, &[(g, g, document)]).unwrap()), 1);
    db
}

fn style(fill: Color) -> CharStyle {
    CharStyle { font_family: FAMILY.into(), size: 100.0, fill: Paint::solid(fill), ..CharStyle::default() }
}

fn document(body: &str) -> String {
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><g id=\"glyph{}\">{body}</g></svg>", gid(SVG_CHAR))
}

#[test]
fn an_svg_glyph_is_what_the_reader_makes_of_its_document() {
    let db = db(&document("<path fill=\"context-fill\" d=\"M0 0h500v-500z\"/>"));
    let blue = Color::rgb8(0, 0, 200);
    let l = layout(&db, &TextObject::point(Point::ZERO, &format!("{SVG_CHAR}F"), style(blue)));
    let (g, f) = (&l.glyphs[0], &l.glyphs[1]);
    assert!(g.outline.elements().is_empty(), "the art replaces the outline");
    let art = g.placed_colour(kurbo::Affine::IDENTITY).expect("the reader's art");
    let mut fills = vec![];
    art.walk(&mut |n| fills.push(n.appearance.fill_paint()));
    assert!(fills.contains(&Paint::solid(blue)), "in the run's fill: {fills:?}");
    // One em at 100 pt.
    let b = art.geometric_bounds().unwrap();
    assert!((b.width() - 100.0).abs() < 1e-6 && (b.y0 + 70.0).abs() < 1e-6, "{b:?}");
    assert!(f.colour.is_none() && !f.outline.elements().is_empty(), "glyphs without documents keep their outlines");
}

#[test]
fn a_glyph_the_reader_makes_nothing_of_keeps_its_outline() {
    let db = db("<svg xmlns=\"http://www.w3.org/2000/svg\"><g id=\"glyph1\"/></svg>");
    let l = layout(&db, &TextObject::point(Point::ZERO, &SVG_CHAR.to_string(), style(Color::BLACK)));
    assert!(l.glyphs[0].colour.is_none() && !l.glyphs[0].outline.elements().is_empty());
}

#[test]
fn a_document_without_text_colours_is_read_once_for_every_fill() {
    let fonts = db(&document("<path fill=\"red\" d=\"M0 0h500v-500z\"/>"));
    let art = |fill| layout(&fonts, &TextObject::point(Point::ZERO, &SVG_CHAR.to_string(), style(fill))).glyphs[0].colour.clone().unwrap();
    assert!(Arc::ptr_eq(&art(Color::BLACK), &art(Color::rgb8(0, 0, 200))));
    let fonts = db(&document("<path fill=\"context-fill\" d=\"M0 0h500v-500z\"/>"));
    let art = |fill| layout(&fonts, &TextObject::point(Point::ZERO, &SVG_CHAR.to_string(), style(fill))).glyphs[0].colour.clone().unwrap();
    assert!(!Arc::ptr_eq(&art(Color::BLACK), &art(Color::rgb8(0, 0, 200))), "context-fill follows the fill");
}

#[test]
fn colr_comes_before_svg() {
    use write_fonts::FontBuilder;
    use write_fonts::types::Tag;
    set_svg_glyph_reader(reader);
    let colr = super::test_fonts::colr_font(false).unwrap();
    let colr = skrifa::FontRef::new(&colr).unwrap();
    let g = gid(SVG_CHAR);
    let svg = svg_font(FAMILY, &[(g, g, &document(""))]).unwrap();
    let svg = skrifa::FontRef::new(&svg).unwrap();
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"SVG "), svg.table_data(skrifa::Tag::new(b"SVG ")).unwrap().as_bytes());
    for r in colr.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, colr.table_data(r.tag()).unwrap().as_bytes());
        }
    }
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(builder.build()), 1);
    let st = CharStyle { font_family: super::test_fonts::colr_family(false).into(), size: 100.0, ..CharStyle::default() };
    let l = layout(&db, &TextObject::point(Point::ZERO, &SVG_CHAR.to_string(), st));
    // The COLR glyph has two layers; the reader's art one path.
    let colour = l.glyphs[0].colour.clone().unwrap();
    assert_eq!(colour.children().map(Vec::len), Some(2), "COLR's layers");
}

/// A glyph with COLR layers that don't read (one paints itself) keeps its outline, also where the
/// font has an SVG document for it: COLR glyphs never draw SVG art, so every writer agrees on what
/// it is.
#[test]
fn a_glyph_whose_colr_layers_dont_read_keeps_its_outline() {
    use write_fonts::FontBuilder;
    use write_fonts::tables::colr::{BaseGlyphList, BaseGlyphPaint, Colr, Paint};
    use write_fonts::types::{GlyphId16, Tag};
    set_svg_glyph_reader(reader);
    let g = gid(SVG_CHAR);
    let mut colr = Colr::new(0, None, None, 0);
    colr.base_glyph_list = Some(BaseGlyphList::new(1, vec![BaseGlyphPaint::new(GlyphId16::new(g), Paint::colr_glyph(GlyphId16::new(g)))])).into();
    let looped = super::test_fonts::colr_font_with(FAMILY, &colr).unwrap();
    let looped = skrifa::FontRef::new(&looped).unwrap();
    let svg = svg_font(FAMILY, &[(g, g, &document(""))]).unwrap();
    let svg = skrifa::FontRef::new(&svg).unwrap();
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"SVG "), svg.table_data(skrifa::Tag::new(b"SVG ")).unwrap().as_bytes());
    for r in looped.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, looped.table_data(r.tag()).unwrap().as_bytes());
        }
    }
    let fonts = FontDb::with_font_dirs(vec![]);
    assert_eq!(fonts.add_font(builder.build()), 1);
    let l = layout(&fonts, &TextObject::point(Point::ZERO, &SVG_CHAR.to_string(), style(Color::BLACK)));
    assert!(l.glyphs[0].colour.is_none() && !l.glyphs[0].outline.elements().is_empty());
    assert!(!fonts.is_svg_glyph(l.glyphs[0].font_id, l.glyphs[0].gid));
}

/// A glyph in the text's stroke colour follows the stroke: added later, it shows.
#[test]
fn a_glyph_in_the_texts_stroke_follows_it() {
    let fonts = db(&document("<path fill=\"context-fill\" stroke=\"context-stroke\" d=\"M0 0h500v-500z\"/>"));
    let stroke_of = |stroke: Paint| {
        let st = CharStyle { stroke, stroke_width: 2.0, ..style(Color::BLACK) };
        let art = layout(&fonts, &TextObject::point(Point::ZERO, &SVG_CHAR.to_string(), st)).glyphs[0].colour.clone().unwrap();
        let mut strokes = vec![];
        art.walk(&mut |n| strokes.push(n.appearance.stroke_paint()));
        strokes
    };
    assert!(!stroke_of(Paint::None).iter().any(|p| *p != Paint::None), "no stroke at first");
    let red = Paint::solid(Color::rgb8(255, 0, 0));
    assert!(stroke_of(red.clone()).contains(&red), "the stroke added");
}
