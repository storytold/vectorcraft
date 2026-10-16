//! Strokes on type: character strokes take their join, cap and dashes, and object strokes on type
//! take every stroke option, as path strokes do.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{AppearanceItem, Dash, LineJoin, Node, NodeKind};
use vectorcraft_geom::Point;

use super::*;

const SIZE: u32 = 200;

/// 120 pt `text` with its baseline at (40, 150), on a white 200×200 page.
fn render_text(text: &str, style: CharStyle, items: Vec<AppearanceItem>) -> Rendered {
    let mut d = Document::new(SIZE as f64, SIZE as f64);
    let mut n = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(40.0, 150.0), text, CharStyle { size: 120.0, ..style }))));
    n.appearance.items = items;
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    Renderer::new().render(&d, SIZE, SIZE, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

fn dark(img: &Rendered, x: u32, y: u32) -> bool {
    let p = img.pixel(x, y);
    (p[0] as u32 + p[1] as u32 + p[2] as u32) < 384
}

/// The box of the dark pixels: (x0, y0, x1, y1).
fn ink(img: &Rendered) -> (u32, u32, u32, u32) {
    let mut b = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..SIZE {
        for x in 0..SIZE {
            if dark(img, x, y) {
                b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
    }
    b
}

fn stroked(join: LineJoin) -> CharStyle {
    CharStyle { stroke: Paint::solid(Color::BLACK), stroke_width: 16.0, stroke_join: join, ..Default::default() }
}

#[test]
fn round_joined_character_strokes_have_no_miter_spikes() {
    // The glyph alone, then stroked 16 pt: a round join reaches at most half the weight past it,
    // the default miter join spikes out further at the sharp corners of the A.
    let glyph = ink(&render_text("A", CharStyle::default(), vec![]));
    let round = ink(&render_text("A", stroked(LineJoin::Round), vec![]));
    let miter = ink(&render_text("A", stroked(LineJoin::Miter), vec![]));
    let reach = |b: (u32, u32, u32, u32)| (glyph.0 - b.0).max(glyph.1 - b.1).max(b.2 - glyph.2).max(b.3 - glyph.3);
    assert!(reach(round) <= 9, "round joins stay within half the weight: {glyph:?} → {round:?}");
    assert!(reach(miter) > reach(round) + 2, "miter joins spike: {miter:?} vs {round:?}");
}

#[test]
fn character_and_object_strokes_on_type_are_dashed() {
    // An "I" without a fill: its outline drawn with 8/8 dashes breaks up along the left stem edge.
    let none = CharStyle { fill: Paint::None, ..Default::default() };
    let dash = Some(Dash { pattern: vec![8.0, 8.0], ..Default::default() });
    let transitions = |img: &Rendered| {
        let (x0, y0, _, y1) = ink(img);
        let col = x0 + 1;
        (y0..y1).filter(|y| dark(img, col, *y) != dark(img, col, y + 1)).count()
    };
    let solid = CharStyle { stroke: Paint::solid(Color::BLACK), stroke_width: 3.0, ..none.clone() };
    assert!(transitions(&render_text("I", solid.clone(), vec![])) <= 2, "a solid stroke is continuous");
    let chars = CharStyle { stroke_dash: dash.clone(), ..solid };
    assert!(transitions(&render_text("I", chars, vec![])) >= 6, "character dashes");
    let mut st = vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::BLACK), 3.0);
    st.dash = dash;
    let object = render_text("I", none, vec![AppearanceItem::Stroke(st.clone())]);
    assert!(transitions(&object) >= 6, "object dashes");
    // And its opacity.
    st.opacity = 0.5;
    st.dash = None;
    let half = render_text("I", CharStyle { fill: Paint::None, ..Default::default() }, vec![AppearanceItem::Stroke(st)]);
    let (x0, y0, _, y1) = ink(&render_text("I", CharStyle::default(), vec![]));
    let p = half.pixel(x0, (y0 + y1) / 2);
    assert!(p[0] > 90 && p[0] < 200, "a half-opaque stroke is grey: {p:?}");
}

/// Align Stroke on live type: an outside character stroke paints its whole weight around the
/// glyphs and leaves them unpainted, an inside one stays within them, as on paths.
#[test]
fn aligned_character_strokes_keep_to_their_side_of_the_glyphs() {
    use vectorcraft_doc::StrokeAlign;
    let glyph = ink(&render_text("I", CharStyle::default(), vec![]));
    let reach = |b: (u32, u32, u32, u32)| {
        let d = |a: u32, b: u32| a as i64 - b as i64;
        d(glyph.0, b.0).max(d(glyph.1, b.1)).max(d(b.2, glyph.2)).max(d(b.3, glyph.3))
    };
    let style = |align| CharStyle { fill: Paint::None, stroke_align: align, ..stroked(LineJoin::Round) };
    let centre = render_text("I", style(StrokeAlign::Center), vec![]);
    let outside = render_text("I", style(StrokeAlign::Outside), vec![]);
    let inside = render_text("I", style(StrokeAlign::Inside), vec![]);
    assert!((7..=9).contains(&reach(ink(&centre))), "centred: half the weight past the glyph, {:?}", ink(&centre));
    assert!((15..=17).contains(&reach(ink(&outside))), "outside: the whole weight past the glyph, {:?}", ink(&outside));
    assert!(reach(ink(&inside)) <= 0, "inside: within the glyph, {:?}", ink(&inside));
    let mid = ((glyph.0 + glyph.2) / 2, (glyph.1 + glyph.3) / 2);
    assert!(!dark(&outside, mid.0, mid.1), "an outside stroke leaves the glyph itself unpainted");
    assert!(dark(&inside, mid.0, mid.1), "an inside stroke paints into the glyph");
}
