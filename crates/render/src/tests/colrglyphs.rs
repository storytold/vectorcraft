//! COLR colour glyphs: drawn as their layers (or, for paints documents have no form for, their
//! image), which the type's fill doesn't paint, except in the layers in the foreground colour.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Point;
use vectorcraft_text::test_fonts::{COLOUR_CHAR, COLR_SWEEP_CHAR, colr_family, colr_font};

use crate::*;

/// 100 pt `text` in the COLR test font (v1 or v0), filled magenta, its baseline at (20, 120)
/// turned by `turn` about there, on a white 160×160 page.
fn render(v1: bool, text: char, turn: f64, opts: RenderOptions) -> Rendered {
    let _adding = crate::tests_fontchange::adding_fonts();
    vectorcraft_text::FontDb::global().add_font(colr_font(v1).unwrap());
    let mut d = Document::new(160.0, 160.0);
    let st = CharStyle { font_family: colr_family(v1).into(), size: 100.0, fill: Paint::solid(Color::rgb(1.0, 0.0, 1.0)), ..Default::default() };
    let mut t = TextObject::point(Point::new(20.0, 120.0), &text.to_string(), st);
    t.xf = Affine::rotate_about(turn, (20.0, 120.0)) * t.xf;
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    Renderer::new().render(&d, 160, 160, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..opts })
}

/// The box of the pixels `like` says yes to.
fn ink(img: &Rendered, like: impl Fn([u8; 4]) -> bool) -> Option<(u32, u32, u32, u32)> {
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for y in 0..img.height {
        for x in 0..img.width {
            if like(img.pixel(x, y)) {
                b = Some(b.map_or((x, y, x, y), |(x0, y0, x1, y1)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y))));
            }
        }
    }
    b
}

fn magenta(p: [u8; 4]) -> bool {
    p[0] > 200 && p[2] > 200 && p[1] < 60
}

#[test]
fn colr_layers_draw_their_colours_not_the_fill() {
    let img = render(true, COLOUR_CHAR, 0.0, RenderOptions::default());
    // The `O` with its linear gradient red → blue from x 0 to 500 units (20 to 70 pt).
    let reddish = ink(&img, |p| p[0] > 180 && p[2] < 90 && p[1] < 60).expect("red at the gradient's start");
    let bluish = ink(&img, |p| p[2] > 180 && p[0] < 90 && p[1] < 60).expect("blue at its end");
    assert!(reddish.0 < bluish.0, "red left of blue: {reddish:?} {bluish:?}");
    assert!(ink(&img, magenta).is_none(), "the fill doesn't paint the glyph");
}

#[test]
fn the_foreground_layer_takes_the_fill() {
    let img = render(false, COLOUR_CHAR, 0.0, RenderOptions::default());
    assert!(ink(&img, magenta).is_some(), "the `I` layer is in the foreground colour");
    assert!(ink(&img, |p| p == [255, 0, 0, 255]).is_some(), "the `O` layer in red");
}

#[test]
fn turned_type_turns_its_colour_layers() {
    let size = |turn| {
        let (x0, y0, x1, y1) = ink(&render(false, COLOUR_CHAR, turn, RenderOptions::default()), magenta).unwrap();
        (x1 - x0, y1 - y0)
    };
    let (w, h) = size(0.0);
    assert!(h > w * 3, "the `I` stands: {w}×{h}");
    let (w, h) = size(std::f64::consts::FRAC_PI_2);
    assert!(w > h * 3, "turned, it lies: {w}×{h}");
}

#[test]
fn outline_mode_shows_the_layers_outlines() {
    let filled = render(false, COLOUR_CHAR, 0.0, RenderOptions::default());
    let outlined = render(false, COLOUR_CHAR, 0.0, RenderOptions { outline: true, ..Default::default() });
    let (x0, y0, x1, y1) = ink(&filled, |p| p == [255, 0, 0, 255]).unwrap();
    let dark = |p: [u8; 4]| p[0] < 128 && p[1] < 128 && p[2] < 128;
    assert!(ink(&outlined, |p| p == [255, 0, 0, 255]).is_none(), "no colours in outline mode");
    let (ox0, oy0, ox1, oy1) = ink(&outlined, dark).expect("the outlines show");
    assert!(ox0 <= x0 + 2 && oy0 <= y0 + 2 && ox1 + 2 >= x1 && oy1 + 2 >= y1, "around the layers: {:?} {:?}", (ox0, oy0, ox1, oy1), (x0, y0, x1, y1));
}

#[test]
fn a_sweep_glyph_draws_its_image() {
    let img = render(true, COLR_SWEEP_CHAR, 0.0, RenderOptions::default());
    assert!(ink(&img, |p| p[0] > 180 && p[1] < 60 && p[2] < 120).is_some(), "red where the sweep starts");
    assert!(ink(&img, magenta).is_none());
}

/// Type with a geometry effect is drawn reshaped as its outlines: its colour glyphs go with them,
/// as their art (here moved 40 pt by a Transform effect).
#[test]
fn colour_glyphs_go_with_type_that_a_geometry_effect_reshapes() {
    let _adding = crate::tests_fontchange::adding_fonts();
    vectorcraft_text::FontDb::global().add_font(colr_font(false).unwrap());
    let mut d = Document::new(160.0, 160.0);
    let st = CharStyle { font_family: colr_family(false).into(), size: 100.0, ..Default::default() };
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 120.0), &COLOUR_CHAR.to_string(), st))));
    let moved = vectorcraft_doc::Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 40}), visible: true };
    let mut appearance = (*n.appearance).clone();
    appearance.effects.push(moved);
    n.appearance = appearance.into();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let img = Renderer::new().render(&d, 160, 160, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() });
    let red = ink(&img, |p| p == [255, 0, 0, 255]).expect("the `O` layer, in red");
    assert!(red.0 >= 58, "moved right with the type: {red:?}");
}

/// Type's outlines stack as the canvas paints type: a run's colour glyphs under its fill, so a
/// plain character set over a colour one stays on top.
#[test]
fn outlined_colour_glyphs_stay_under_their_runs_fill() {
    let _adding = crate::tests_fontchange::adding_fonts();
    vectorcraft_text::FontDb::global().add_font(colr_font(false).unwrap());
    let mut d = Document::new(160.0, 160.0);
    let st = CharStyle { font_family: colr_family(false).into(), size: 100.0, ..Default::default() };
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 120.0), &format!("{COLOUR_CHAR}F"), st))));
    let outlined = vectorcraft_effects::outline_text(&n).unwrap();
    let kinds: Vec<bool> = outlined.children().unwrap().iter().map(|c| matches!(c.kind, NodeKind::Group { .. })).collect();
    assert_eq!(kinds, [true, false], "the colour glyph's group, then the run's path");
}
