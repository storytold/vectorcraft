//! Colour bitmap glyphs (emoji): outlined text writes their images; live text leaves them to the
//! viewer, and embedded fonts don't carry them.

use skrifa::raw::TableProvider;
use vectorcraft_doc::{CharStyle, Document, Node, NodeKind, TextObject};
use vectorcraft_geom::Point;
use vectorcraft_svg::{ExportOptions, export_full};
use vectorcraft_testkit::format::base64_decode;
use vectorcraft_text::test_fonts::{COLOUR_CHAR, colour_bitmap_family, colour_bitmap_font};

/// [`COLOUR_CHAR`] and an outline glyph in the `CBDT` test font, 24 pt.
fn doc() -> Document {
    vectorcraft_text::FontDb::global().add_font(colour_bitmap_font(false).unwrap());
    let mut d = Document::new(200.0, 100.0);
    let st = CharStyle { font_family: colour_bitmap_family(false).into(), size: 24.0, ..CharStyle::default() };
    let mut n =
        Node::new(vectorcraft_doc::NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 50.0), &format!("{COLOUR_CHAR}F"), st))));
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

#[test]
fn outlined_text_writes_bitmap_glyphs_as_images() {
    let svg = export_full(&doc(), &ExportOptions { outline_text: true, ..Default::default() }, None).svg;
    assert_eq!(svg.matches("<image ").count(), 1, "{svg}");
    assert!(svg.contains("width=\"16\" height=\"16\"") && svg.contains("xlink:href=\"data:image/png;base64,"), "{svg}");
    assert!(svg.contains("<path"), "the other glyph is an outline");
}

#[test]
fn embedded_fonts_leave_bitmap_glyphs_to_the_viewer() {
    let out = export_full(&doc(), &ExportOptions { embed_fonts: true, ..Default::default() }, None);
    let data = out.svg.split("base64,").nth(1).unwrap().split(')').next().unwrap();
    let font = base64_decode(data).unwrap();
    let cmap = skrifa::FontRef::new(&font).unwrap().cmap().unwrap();
    assert!(cmap.map_codepoint('F').is_some());
    assert!(cmap.map_codepoint(COLOUR_CHAR).is_none(), "its subset would hold no bitmap");
    assert!(out.warnings.iter().any(|w| w.contains("emoji")), "{:?}", out.warnings);
    assert!(out.svg.contains(&format!("{COLOUR_CHAR}F")), "the character stays text");
}

/// `text` in the COLR test font (v1 or v0), 24 pt, filled blue.
fn colr_doc(v1: bool, text: impl std::fmt::Display) -> Document {
    use vectorcraft_text::test_fonts::{colr_family, colr_font};
    vectorcraft_text::FontDb::global().add_font(colr_font(v1).unwrap());
    let mut d = Document::new(200.0, 100.0);
    let fill = vectorcraft_doc::color::Paint::solid(vectorcraft_doc::color::Color::rgb8(0, 0, 255));
    let st = CharStyle { font_family: colr_family(v1).into(), size: 24.0, fill, ..CharStyle::default() };
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 50.0), &text.to_string(), st))));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

#[test]
fn outlined_text_writes_colr_glyphs_as_vector_art() {
    let svg = export_full(&colr_doc(true, COLOUR_CHAR), &ExportOptions { outline_text: true, ..Default::default() }, None).svg;
    assert!(!svg.contains("<image "), "{svg}");
    assert!(svg.contains("<linearGradient") && svg.contains("<radialGradient"), "{svg}");
    assert!(svg.contains("<clipPath"), "the clip box: {svg}");
    assert!(svg.contains("mix-blend-mode:multiply") || svg.contains("mix-blend-mode: multiply"), "{svg}");
    let v0 = export_full(&colr_doc(false, COLOUR_CHAR), &ExportOptions { outline_text: true, ..Default::default() }, None).svg;
    assert!(v0.contains("#f00") || v0.contains("#ff0000"), "the red layer: {v0}");
    assert!(v0.contains("#00f") || v0.contains("#0000ff"), "the foreground layer in the fill: {v0}");
}

#[test]
fn outlined_text_writes_a_sweep_glyph_as_its_image() {
    use vectorcraft_text::test_fonts::COLR_SWEEP_CHAR;
    let svg = export_full(&colr_doc(true, COLR_SWEEP_CHAR), &ExportOptions { outline_text: true, ..Default::default() }, None).svg;
    assert_eq!(svg.matches("<image ").count(), 1, "{svg}");
}

#[test]
fn embedded_fonts_leave_colr_glyphs_to_the_viewer() {
    let out = export_full(&colr_doc(false, COLOUR_CHAR), &ExportOptions { embed_fonts: true, ..Default::default() }, None);
    assert!(!out.svg.contains("base64,"), "no subset without its colour layers: {}", out.svg);
    assert!(out.warnings.iter().any(|w| w.contains("emoji")), "{:?}", out.warnings);
    assert!(out.svg.contains(COLOUR_CHAR), "the character stays text");
}

/// Outlined text stacks as the canvas paints type: a run's colour glyphs under its paint, so a
/// plain character set over a colour one stays on top.
#[test]
fn outlined_colour_glyphs_stay_under_their_runs_paint() {
    let outlined = ExportOptions { outline_text: true, ..Default::default() };
    let svg = export_full(&doc(), &outlined, None).svg;
    assert!(svg.find("<image ").unwrap() < svg.find("<path").unwrap(), "the emoji, then the run: {svg}");
    let svg = export_full(&colr_doc(false, format!("{COLOUR_CHAR}F")), &outlined, None).svg;
    let first = &svg[svg.find("<path").unwrap()..];
    assert!(first[..first.find('>').unwrap()].contains("f00"), "the red layer first: {svg}");
}
