//! Colour bitmap glyphs (emoji): drawn as their images, which the type's fill doesn't paint.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Point;
use vectorcraft_text::test_fonts::{COLOUR_CHAR, COLOUR_LARGE, colour_bitmap_family, colour_bitmap_font};

use crate::*;

/// 100 pt [`COLOUR_CHAR`] in the colour bitmap test font (`sbix` or `CBDT`), filled blue, its
/// baseline at (20, 120), on a white 160×160 page.
fn render(sbix: bool, opts: RenderOptions) -> Rendered {
    let _adding = crate::tests_fontchange::adding_fonts();
    vectorcraft_text::FontDb::global().add_font(colour_bitmap_font(sbix).unwrap());
    let mut d = Document::new(160.0, 160.0);
    let st = CharStyle {
        font_family: colour_bitmap_family(sbix).into(),
        size: 100.0,
        fill: Paint::solid(Color::rgb(0.0, 0.0, 1.0)),
        ..Default::default()
    };
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 120.0), &COLOUR_CHAR.to_string(), st))));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    Renderer::new().render(&d, 160, 160, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..opts })
}

#[test]
fn a_bitmap_glyph_draws_its_largest_strike_not_the_fill() {
    for sbix in [true, false] {
        let img = render(sbix, RenderOptions::default());
        // Inside the em square the image covers (the CBDT one from 75 above the baseline to 25
        // below, the sbix one from 87.2 above to 12.8 below), away from its edges.
        assert_eq!(img.pixel(70, 80), COLOUR_LARGE, "sbix {sbix}");
        let blue = (0..160).flat_map(|y| (0..160).map(move |x| (x, y))).any(|(x, y)| img.pixel(x, y)[2] > 200 && img.pixel(x, y)[0] < 50);
        assert!(!blue, "the fill doesn't paint the glyph (sbix {sbix})");
    }
}

#[test]
fn outline_mode_draws_a_bitmap_glyph_as_its_box() {
    let img = render(false, RenderOptions { outline: true, ..Default::default() });
    assert_eq!(img.pixel(70, 80), [255, 255, 255, 255], "no pixels inside");
    // The box's left edge at x 32.5 (the bearing of 2 px of 6.25 pt), from y 45 to 145.
    assert!((31..=34).any(|x| img.pixel(x, 100)[0] < 128), "the box's edge shows");
}

/// 100 pt [`COLOUR_CHAR`] in the `CBDT` test font with `effects`, on a white 160×160 page.
fn render_with(effects: Vec<vectorcraft_doc::Effect>) -> Rendered {
    let _adding = crate::tests_fontchange::adding_fonts();
    vectorcraft_text::FontDb::global().add_font(colour_bitmap_font(false).unwrap());
    let mut d = Document::new(160.0, 160.0);
    let st = CharStyle { font_family: colour_bitmap_family(false).into(), size: 100.0, ..Default::default() };
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 120.0), &COLOUR_CHAR.to_string(), st))));
    let mut appearance = (*n.appearance).clone();
    appearance.effects = effects;
    n.appearance = appearance.into();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    Renderer::new().render(&d, 160, 160, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

fn effect(id: &str, params: serde_json::Value) -> vectorcraft_doc::Effect {
    vectorcraft_doc::Effect { id: id.into(), params, visible: true }
}

/// Type with a geometry effect is drawn as its outlines: its emoji go with them, as images that
/// hold their own pixels (here moved 40 pt by a Transform effect).
#[test]
fn bitmap_glyphs_go_with_type_that_a_geometry_effect_reshapes() {
    let img = render_with(vec![effect("distort.transform", serde_json::json!({"moveH": 40}))]);
    assert_eq!(img.pixel(110, 80), COLOUR_LARGE, "moved with the type");
    assert_eq!(img.pixel(50, 80), [255, 255, 255, 255], "not where it was");
}

/// A colour adjustment leaves the emoji of reshaped type in their colours, as it does those of
/// type, and as exports and Expand Appearance draw them.
#[test]
fn a_colour_adjustment_leaves_the_emoji_of_reshaped_type_as_they_are() {
    let moved = effect("distort.transform", serde_json::json!({"moveH": 40}));
    let img = render_with(vec![moved, effect("adjust.hueSaturation", serde_json::json!({"hue": 120}))]);
    assert_eq!(img.pixel(110, 80), COLOUR_LARGE);
}
