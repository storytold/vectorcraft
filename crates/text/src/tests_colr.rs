//! COLR colour glyphs (v0 layers, v1 paints): read as trees of document nodes in font units, or
//! as an image for paints VectorCraft lacks, and laid out with the type.

use skrifa::MetadataProvider;
use skrifa::color::ColorGlyphFormat;

use super::test_fonts::{COLOUR_CHAR, COLR_SWEEP_CHAR, colr_font};

#[test]
fn colr_fonts_build_and_read() {
    for v1 in [false, true] {
        let bytes = colr_font(v1).expect("the test font builds");
        let font = skrifa::FontRef::new(&bytes).unwrap();
        let glyph = |ch| font.color_glyphs().get(font.charmap().map(ch).unwrap());
        let format = glyph(COLOUR_CHAR).map(|g| g.format());
        assert!(matches!((v1, format), (false, Some(ColorGlyphFormat::ColrV0)) | (true, Some(ColorGlyphFormat::ColrV1))), "v1 {v1}");
        assert_eq!(glyph(COLR_SWEEP_CHAR).is_some(), v1, "only v1 has the sweep glyph");
    }
}

use std::sync::Arc;

use skrifa::instance::LocationRef;
use vectorcraft_doc::color::{BlendMode, Color, GradientKind, Paint};
use vectorcraft_doc::{Node, NodeKind};

use super::colr::{ColourContent, colour_glyph};
use super::test_fonts::{COLR_GREEN, COLR_RED};

const FOREGROUND: Color = Color::Rgb { r: 0.25, g: 0.5, b: 0.75 };

/// [`colour_glyph`] of `ch` in `font`.
fn content(font: &[u8], ch: char) -> Option<ColourContent> {
    let font = skrifa::FontRef::new(font).unwrap();
    let gid = font.charmap().map(ch).unwrap().to_u32();
    colour_glyph(&font, LocationRef::default(), 1000.0, gid, FOREGROUND).0
}

fn art(font: &[u8], ch: char) -> Arc<Node> {
    match content(font, ch) {
        Some(ColourContent::Art(a)) => a,
        _ => panic!("vector art expected"),
    }
}

fn children(n: &Node) -> &[Arc<Node>] {
    n.children().map(Vec::as_slice).unwrap_or_default()
}

fn fill(n: &Node) -> Paint {
    n.appearance.fill_paint()
}

fn rgb([r, g, b, _]: [u8; 4]) -> Color {
    Color::rgb8(r, g, b)
}

#[test]
fn v0_layers_are_paths_in_paint_order() {
    let root = art(&colr_font(false).unwrap(), COLOUR_CHAR);
    assert!(!root.isolate, "a group without blending needs no layer of its own");
    let layers = children(&root);
    assert_eq!(layers.len(), 2, "{layers:?}");
    assert!(layers.iter().all(|l| matches!(l.kind, NodeKind::Path { .. })));
    assert_eq!(fill(&layers[0]), Paint::solid(rgb(COLR_RED)));
    assert_eq!(fill(&layers[1]), Paint::solid(FOREGROUND), "palette index 0xFFFF is the foreground");
    // In font units, y down: the `O` stands above the baseline.
    let b = layers[0].geometric_bounds().unwrap();
    assert!(b.y1 <= 20.0 && b.y0 < -500.0 && b.width() > 400.0, "{b:?}");
}

#[test]
fn v1_paints_become_gradients_clips_and_isolated_groups() {
    let root = art(&colr_font(true).unwrap(), COLOUR_CHAR);
    // The clip box clips the whole glyph.
    let [clipped] = children(&root) else { panic!("{:?}", children(&root)) };
    assert!(matches!(clipped.kind, NodeKind::Group { clip: true, .. }), "{:?}", clipped.kind);
    let layers = &children(clipped)[1..];
    assert_eq!(layers.len(), 3, "{layers:?}");
    let gradient = |n: &Node| match fill(n) {
        Paint::Gradient(g) => g,
        p => panic!("a gradient: {p:?}"),
    };
    let linear = gradient(&layers[0]);
    assert_eq!(linear.gradient.kind, GradientKind::Linear);
    let geom = linear.geom.unwrap();
    assert!((geom.start.x - 0.0).abs() < 1e-6 && (geom.end.x - 500.0).abs() < 1e-6, "{geom:?}");
    let radial = gradient(&layers[1]);
    assert_eq!(radial.gradient.kind, GradientKind::Radial);
    // Scaled by a half about the origin, y down: centre (125, -150), radius 150.
    let geom = radial.geom.unwrap();
    assert!((geom.start.x - 125.0).abs() < 1e-3 && (geom.start.y + 150.0).abs() < 1e-3, "{geom:?}");
    assert!(((geom.end - geom.start).hypot() - 150.0).abs() < 1e-3, "{geom:?}");
    // The composite: an isolated group, the backdrop, then the source multiplied over it.
    let composite = &layers[2];
    assert!(composite.isolate);
    let [backdrop, source] = children(composite) else { panic!("{:?}", children(composite)) };
    assert_eq!(fill(backdrop), Paint::solid(rgb(COLR_RED)));
    let source = children(source).first().cloned().unwrap_or_else(|| source.clone());
    assert_eq!(fill(&source), Paint::solid(rgb(COLR_GREEN)));
    assert!(children(composite)[1].blend == BlendMode::Multiply, "{:?}", children(composite)[1].blend);
}

#[test]
fn a_sweep_gradient_makes_the_glyph_an_image() {
    let Some(ColourContent::Image(image)) = content(&colr_font(true).unwrap(), COLR_SWEEP_CHAR) else { panic!("an image") };
    assert!(image.blob.bytes.starts_with(b"\x89PNG"));
    // The `O` is some 600 units tall: at 256 px per em, some 150 px.
    assert!((100..300).contains(&image.height), "{}", image.height);
    // Its pixels map back to the glyph's place in font units, y down.
    let b = image.xf.transform_rect_bbox(image.rect());
    assert!(b.y1 <= 20.0 && b.y0 < -500.0, "{b:?}");
}

#[test]
fn a_colr_graph_that_paints_itself_is_no_colour_glyph() {
    use write_fonts::tables::colr::{BaseGlyphList, BaseGlyphPaint, Colr, Paint};
    use write_fonts::types::GlyphId16;
    let base = skrifa::FontRef::new(&colr_font(false).unwrap()).unwrap().charmap().map(COLOUR_CHAR).unwrap().to_u32();
    let base = GlyphId16::new(u16::try_from(base).unwrap());
    let mut colr = Colr::new(0, None, None, 0);
    colr.base_glyph_list = Some(BaseGlyphList::new(1, vec![BaseGlyphPaint::new(base, Paint::colr_glyph(base))])).into();
    let font = super::test_fonts::colr_font_with("Colrtest Loop", &colr).unwrap();
    assert!(content(&font, COLOUR_CHAR).is_none());
}

#[test]
fn a_colr_graph_nested_past_the_cap_is_no_colour_glyph() {
    use write_fonts::tables::colr::{BaseGlyphList, BaseGlyphPaint, Colr, Paint};
    use write_fonts::types::{F2Dot14, GlyphId16};
    let font = skrifa::FontRef::new(&colr_font(false).unwrap()).unwrap().charmap().map(COLOUR_CHAR).unwrap().to_u32();
    let base = GlyphId16::new(u16::try_from(font).unwrap());
    let mut paint = Paint::glyph(Paint::solid(0, F2Dot14::from_f32(1.0)), base);
    for _ in 0..200 {
        paint = Paint::scale_uniform(paint, F2Dot14::from_f32(1.0));
    }
    let mut colr = Colr::new(0, None, None, 0);
    colr.base_glyph_list = Some(BaseGlyphList::new(1, vec![BaseGlyphPaint::new(base, paint)])).into();
    let font = super::test_fonts::colr_font_with("Colrtest Deep", &colr).unwrap();
    assert!(content(&font, COLOUR_CHAR).is_none());
}

// Layout: colour glyphs are laid out with their layers, which move with the type.

use kurbo::Point as KPoint;
use vectorcraft_doc::{CharStyle, TextObject};

use super::test_fonts::colr_family;
use super::{FontDb, layout};

fn colr_db(v1: bool) -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(colr_font(v1).expect("the test font builds")), 1);
    db
}

fn colr_style(v1: bool) -> CharStyle {
    CharStyle { font_family: colr_family(v1).into(), size: 100.0, ..CharStyle::default() }
}

#[test]
fn a_colr_glyph_lays_out_its_layers_with_an_empty_outline() {
    let db = colr_db(false);
    let l = layout(&db, &TextObject::point(KPoint::new(10.0, 50.0), &format!("{COLOUR_CHAR}F"), colr_style(false)));
    let (g, f) = (&l.glyphs[0], &l.glyphs[1]);
    assert!(g.outline.elements().is_empty(), "the layers replace the outline");
    assert_eq!(g.colour.as_deref().map(|c| children(c).len()), Some(2));
    assert!(g.image.is_none());
    assert!(f.colour.is_none() && !f.outline.elements().is_empty(), "other glyphs keep their outlines");
    // In text space at 100 pt: the `O` layer stands on the baseline (y 0), some 70 pt tall.
    let placed = g.placed_colour(kurbo::Affine::IDENTITY).unwrap();
    let b = placed.geometric_bounds().unwrap();
    assert!(b.y1.abs() < 3.0 && b.height() > 60.0 && b.height() < 80.0, "{b:?}");
}

#[test]
fn the_foreground_layer_takes_the_runs_fill_and_black_for_a_gradient_fill() {
    let db = colr_db(false);
    let foreground = |fill: Paint| {
        let st = CharStyle { fill, ..colr_style(false) };
        let l = layout(&db, &TextObject::point(KPoint::ZERO, &COLOUR_CHAR.to_string(), st));
        let colour = l.glyphs[0].colour.clone().unwrap();
        children(&colour)[1].appearance.fill_paint()
    };
    let blue = Color::rgb8(0, 0, 200);
    assert_eq!(foreground(Paint::solid(blue)), Paint::solid(blue));
    assert_eq!(foreground(Paint::solid(FOREGROUND)), Paint::solid(FOREGROUND), "a new fill colour, a new glyph");
    let gradient = Paint::Gradient(Box::new(vectorcraft_doc::color::GradientPaint::new(Default::default())));
    assert_eq!(foreground(gradient), Paint::solid(Color::BLACK));
}

/// Fills that look alike but separate differently (a grey on the black plate, the same RGB grey on
/// all four) each paint the foreground layer in their own colour.
#[test]
fn fills_that_only_look_alike_each_paint_the_foreground_layer() {
    let db = colr_db(false);
    let foreground = |fill: Color| {
        let st = CharStyle { fill: Paint::solid(fill), ..colr_style(false) };
        let l = layout(&db, &TextObject::point(KPoint::ZERO, &COLOUR_CHAR.to_string(), st));
        let colour = l.glyphs[0].colour.clone().unwrap();
        children(&colour)[1].appearance.fill_paint()
    };
    let grey = Color::gray(0.5);
    let rgb = Color::rgb8(128, 128, 128);
    assert_eq!(grey.to_rgba8(1.0), rgb.to_rgba8(1.0), "they look alike");
    assert_eq!(foreground(grey), Paint::solid(grey));
    assert_eq!(foreground(rgb), Paint::solid(rgb));
}

#[test]
fn layout_bounds_take_in_the_colour_layers() {
    let db = colr_db(true);
    let l = layout(&db, &TextObject::point(KPoint::new(10.0, 50.0), &COLOUR_CHAR.to_string(), colr_style(true)));
    let g = &l.glyphs[0];
    let b = g.placed_colour(kurbo::Affine::IDENTITY).unwrap().geometric_bounds().unwrap();
    assert!(l.bounds.contains_rect(b.inset(-0.01)) || l.bounds.contains_rect(b), "{:?} holds {b:?}", l.bounds);
    // The `O` layer is wider than the `E` the glyph advances by.
    assert!(b.width() > g.advance * 0.9, "{b:?}");
}

#[test]
fn a_face_with_colr_and_sbix_draws_colr() {
    use write_fonts::FontBuilder;
    use write_fonts::types::Tag;
    let colr = colr_font(false).unwrap();
    let sbix = super::test_fonts::colour_bitmap_font(true).unwrap();
    let sbix = skrifa::FontRef::new(&sbix).unwrap();
    let font = skrifa::FontRef::new(&colr).unwrap();
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"sbix"), sbix.table_data(skrifa::Tag::new(b"sbix")).unwrap().as_bytes());
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag()).unwrap().as_bytes());
        }
    }
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(builder.build()), 1);
    let l = layout(&db, &TextObject::point(KPoint::ZERO, &COLOUR_CHAR.to_string(), colr_style(false)));
    assert!(l.glyphs[0].colour.is_some() && l.glyphs[0].image.is_none(), "vectors over bitmaps");
}

/// A v1 font whose glyph of `base` is `paint`, given the glyph id of `O`, its layer list one
/// red fill.
fn v1_font(family: &str, base: char, paint: impl Fn(write_fonts::types::GlyphId16) -> ColrPaint) -> Vec<u8> {
    use write_fonts::tables::colr::{BaseGlyphList, BaseGlyphPaint, Colr, LayerList};
    use write_fonts::types::GlyphId16;
    let bytes = colr_font(false).unwrap();
    let font = skrifa::FontRef::new(&bytes).unwrap();
    let gid = |ch| GlyphId16::new(u16::try_from(font.charmap().map(ch).unwrap().to_u32()).unwrap());
    let mut colr = Colr::new(0, None, None, 0);
    colr.base_glyph_list = Some(BaseGlyphList::new(1, vec![BaseGlyphPaint::new(gid(base), paint(gid('O')))])).into();
    colr.layer_list = Some(LayerList::new(1, vec![solid(0)])).into();
    super::test_fonts::colr_font_with(family, &colr).unwrap()
}

use write_fonts::tables::colr::Paint as ColrPaint;

fn solid(index: u16) -> ColrPaint {
    ColrPaint::solid(index, write_fonts::types::F2Dot14::from_f32(1.0))
}

fn sweep() -> ColrPaint {
    use write_fonts::tables::colr::{ColorLine, ColorStop, Extend};
    use write_fonts::types::{F2Dot14, FWord};
    let one = F2Dot14::from_f32(1.0);
    let line = ColorLine::new(Extend::Pad, 2, vec![ColorStop::new(F2Dot14::from_f32(0.0), 0, one), ColorStop::new(one, 1, one)]);
    ColrPaint::sweep_gradient(line, FWord::new(250), FWord::new(300), F2Dot14::from_f32(0.0), one)
}

/// A glyph filled with a composite of solid colours is vector art: the composite's fills fill
/// the glyph around them.
#[test]
fn a_glyph_filled_with_a_composite_is_vector_art() {
    use write_fonts::tables::colr::CompositeMode;
    let font = v1_font("Colrtest Composite", COLOUR_CHAR, |o| ColrPaint::glyph(ColrPaint::composite(solid(2), CompositeMode::Multiply, solid(0)), o));
    assert!(matches!(content(&font, COLOUR_CHAR), Some(ColourContent::Art(_))));
}

/// A colour of its own filling a (TrueType, quadratic) glyph's area is that glyph filled: one
/// path, not a clipping group around a copy of it.
#[test]
fn a_glyph_filled_with_one_colour_is_one_path() {
    let font = v1_font("Colrtest Flood", COLOUR_CHAR, |o| ColrPaint::glyph(ColrPaint::colr_layers(1, 0), o));
    let root = art(&font, COLOUR_CHAR);
    let [only] = children(&root) else { panic!("{:?}", children(&root).iter().map(|c| c.kind_label()).collect::<Vec<_>>()) };
    assert!(matches!(only.kind, NodeKind::Path { .. }), "{}", only.kind_label());
    assert_eq!(fill(only), Paint::solid(rgb(COLR_RED)));
}

/// A sweep glyph in a font without clip boxes, its base glyph without an outline (the space's,
/// as COLRv1 fonts make them): its image is as large as what it paints.
#[test]
fn a_sweep_glyph_without_a_clip_box_or_outline_is_an_image_of_its_paint() {
    let font = v1_font("Colrtest Bare", ' ', |o| ColrPaint::glyph(sweep(), o));
    let Some(ColourContent::Image(image)) = content(&font, ' ') else { panic!("an image") };
    assert!((100..300).contains(&image.height), "{}", image.height);
}

/// A transform that collapses the glyph (scale 0, from a font) draws nothing in its image,
/// without panicking.
#[test]
fn a_collapsed_sweep_glyph_draws_nothing() {
    use write_fonts::tables::colr::{BaseGlyphList, BaseGlyphPaint, Clip, ClipBox, ClipList, Colr};
    use write_fonts::types::{F2Dot14, FWord, GlyphId16};
    let bytes = colr_font(false).unwrap();
    let read = skrifa::FontRef::new(&bytes).unwrap();
    let gid = |ch| GlyphId16::new(u16::try_from(read.charmap().map(ch).unwrap().to_u32()).unwrap());
    let mut colr = Colr::new(0, None, None, 0);
    let flat = ColrPaint::scale_uniform(ColrPaint::glyph(sweep(), gid('O')), F2Dot14::from_f32(0.0));
    colr.base_glyph_list = Some(BaseGlyphList::new(1, vec![BaseGlyphPaint::new(gid(COLOUR_CHAR), flat)])).into();
    // A clip box, so the image is made.
    let clip = ClipBox::format_1(FWord::new(0), FWord::new(0), FWord::new(500), FWord::new(500));
    colr.clip_list = Some(ClipList::new(1, 1, vec![Clip::new(gid(COLOUR_CHAR), gid(COLOUR_CHAR), clip)])).into();
    let font = super::test_fonts::colr_font_with("Colrtest Flat", &colr).unwrap();
    let Some(ColourContent::Image(image)) = content(&font, COLOUR_CHAR) else { panic!("an image") };
    assert!(image.blob.bytes.starts_with(b"\x89PNG"));
}

/// Each paint documents have no form for makes the glyph an image.
#[test]
fn paints_documents_lack_make_the_glyph_an_image() {
    use write_fonts::tables::colr::{ColorLine, ColorStop, CompositeMode, Extend};
    use write_fonts::types::{F2Dot14, FWord, UfWord};
    let one = F2Dot14::from_f32(1.0);
    let line = |extend| ColorLine::new(extend, 2, vec![ColorStop::new(F2Dot14::from_f32(0.0), 0, one), ColorStop::new(one, 1, one)]);
    let w = FWord::new;
    type GlyphPaint<'a> = Box<dyn Fn(write_fonts::types::GlyphId16) -> ColrPaint + 'a>;
    let cases: [(&str, GlyphPaint<'_>); 3] = [
        ("Colrtest Xor", Box::new(|o| ColrPaint::composite(ColrPaint::glyph(solid(2), o), CompositeMode::Xor, ColrPaint::glyph(solid(0), o)))),
        (
            "Colrtest Repeat",
            Box::new(move |o| ColrPaint::glyph(ColrPaint::linear_gradient(line(Extend::Repeat), w(0), w(0), w(100), w(0), w(0), w(100)), o)),
        ),
        (
            "Colrtest Circles",
            Box::new(move |o| {
                ColrPaint::glyph(ColrPaint::radial_gradient(line(Extend::Pad), w(200), w(300), UfWord::new(50), w(250), w(300), UfWord::new(300)), o)
            }),
        ),
    ];
    for (family, paint) in cases {
        let font = v1_font(family, COLOUR_CHAR, paint);
        assert!(matches!(content(&font, COLOUR_CHAR), Some(ColourContent::Image(_))), "{family}");
    }
}

/// A radial gradient whose start circle is around its centre is vector art, its stops moved out.
#[test]
fn a_radial_gradient_from_a_circle_around_its_centre_is_vector_art() {
    use write_fonts::tables::colr::{ColorLine, ColorStop, Extend};
    use write_fonts::types::{F2Dot14, FWord, UfWord};
    let one = F2Dot14::from_f32(1.0);
    let line = ColorLine::new(Extend::Pad, 2, vec![ColorStop::new(F2Dot14::from_f32(0.0), 0, one), ColorStop::new(one, 1, one)]);
    let w = FWord::new;
    let font = v1_font("Colrtest Ring", COLOUR_CHAR, |o| {
        ColrPaint::glyph(ColrPaint::radial_gradient(line.clone(), w(250), w(300), UfWord::new(100), w(250), w(300), UfWord::new(400)), o)
    });
    let root = art(&font, COLOUR_CHAR);
    let Paint::Gradient(g) = fill(&children(&root)[0]) else { panic!("a gradient") };
    let offsets: Vec<f32> = g.gradient.stops.iter().map(|s| s.offset).collect();
    assert_eq!(offsets, [0.25, 1.0], "the stops start a quarter of the way out");
}

/// A glyph whose layers don't use the foreground colour is read once, whatever the type's fill.
#[test]
fn a_glyph_without_the_foreground_colour_is_read_once_for_every_fill() {
    let tree = |v1: bool, fill: Color| {
        let db = colr_db(v1);
        let st = CharStyle { fill: Paint::solid(fill), ..colr_style(v1) };
        let first = layout(&db, &TextObject::point(KPoint::ZERO, &COLOUR_CHAR.to_string(), st.clone())).glyphs[0].colour.clone().unwrap();
        let st = CharStyle { fill: Paint::solid(FOREGROUND), ..st };
        let other = layout(&db, &TextObject::point(KPoint::ZERO, &COLOUR_CHAR.to_string(), st)).glyphs[0].colour.clone().unwrap();
        Arc::ptr_eq(&first, &other)
    };
    assert!(tree(true, Color::rgb8(0, 0, 200)), "v1's glyph uses only its palette");
    assert!(!tree(false, Color::rgb8(0, 0, 200)), "v0's foreground layer follows the fill");
}

/// A variable font's COLR glyph draws its layers at the face's instance: at Bold, the layer's `l`
/// is as much wider as the font widens it.
#[test]
fn a_variable_fonts_colr_layers_follow_its_instance() {
    use crate::test_fonts::{COLR_VARIABLE_FAMILY, VARIABLE_WIDEN, colr_variable_font};
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(colr_variable_font().expect("the test font builds")), 3, "Regular, SemiBold and Bold");
    let width = |style: &str| {
        let st = CharStyle { font_family: COLR_VARIABLE_FAMILY.into(), font_style: style.into(), size: 100.0, ..CharStyle::default() };
        let l = layout(&db, &TextObject::point(KPoint::ZERO, &COLOUR_CHAR.to_string(), st));
        l.glyphs[0].placed_colour(kurbo::Affine::IDENTITY).unwrap().geometric_bounds().unwrap().width()
    };
    let (regular, bold) = (width("Regular"), width("Bold"));
    assert!((bold / regular - 1.0 - f64::from(VARIABLE_WIDEN)).abs() < 0.02, "{regular} → {bold}");
}

/// On a path, a colour glyph's layers turn with it: along a path going down, the tall `l` layer
/// lies on its side, across the path.
#[test]
fn a_colour_glyph_on_a_path_turns_with_it() {
    use crate::test_fonts::{COLR_VARIABLE_FAMILY, colr_variable_font};
    use vectorcraft_doc::text::TextKind;
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(colr_variable_font().expect("the test font builds"));
    let st = CharStyle { font_family: COLR_VARIABLE_FAMILY.into(), size: 100.0, ..CharStyle::default() };
    let mut t = TextObject::point(KPoint::ZERO, &COLOUR_CHAR.to_string(), st);
    let upright = layout(&db, &t).glyphs[0].placed_colour(kurbo::Affine::IDENTITY).unwrap().geometric_bounds().unwrap();
    let down = kurbo::Shape::to_path(&kurbo::Line::new((0.0, 0.0), (0.0, 400.0)), 0.1);
    t.kind = TextKind::OnPath { path: vectorcraft_geom::PathData::from_bezpath(&down), start: 0.0, end: None };
    let b = layout(&db, &t).glyphs[0].placed_colour(kurbo::Affine::IDENTITY).unwrap().geometric_bounds().unwrap();
    assert!(upright.height() > 2.0 * upright.width(), "upright, the `l` is tall: {upright:?}");
    assert!((b.width() - upright.height()).abs() < 1.0 && (b.height() - upright.width()).abs() < 1.0, "on its side: {b:?}");
    assert!(b.y0 >= -1.0 && b.y1 <= 400.0, "along the path: {b:?}");
}
