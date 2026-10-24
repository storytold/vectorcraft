//! Strokes on type: character strokes take their join, cap and dashes, and object strokes on type
//! take every stroke option, as path strokes do.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{AppearanceItem, Dash, LineJoin, Node, NodeKind};
use vectorcraft_geom::Point;

use super::*;
use crate::tests_fontchange::fonts_steady;

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

/// A document with 60 pt `text` without a fill and with a 1 pt character stroke, and the type's id.
fn stroked_text_doc(text: &str) -> (Document, NodeId) {
    let mut d = Document::new(SIZE as f64, SIZE as f64);
    let style = CharStyle { size: 60.0, fill: Paint::None, stroke: Paint::solid(Color::BLACK), stroke_width: 1.0, ..Default::default() };
    let mut n = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 120.0), text, style))));
    n.id = d.alloc_id();
    let id = n.id;
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    (d, id)
}

/// The character stroke outlines that `r` keeps and that its last frame made or used.
fn character_outlines(r: &Renderer) -> Vec<Arc<BezPath>> {
    r.char_strokes.used_in(r.stamp).map(|(_, outline)| outline.clone()).collect()
}

/// The outlines of type that changes in every frame (an animation) go within a few frames: the
/// renderer keeps no more than the last few frames' strokes, however long it runs.
#[test]
fn the_character_strokes_of_type_that_changes_in_every_frame_go() {
    let _fonts = fonts_steady();
    let (mut d, id) = stroked_text_doc("Type");
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    for i in 0..40 {
        if let Some(Node { kind: NodeKind::Text(t), .. }) = d.node_mut(id) {
            t.xf = Affine::translate((f64::from(i), 0.0));
        }
        r.render(&d, SIZE, SIZE, Affine::IDENTITY, &opts);
    }
    assert!(r.char_strokes.len() <= 4, "{} outlines kept", r.char_strokes.len());
}

/// A character stroke is expanded once: the next frame of the same view draws the outline the
/// first made. A change of the zoom level or of the text expands it again.
#[test]
fn character_stroke_outlines_are_kept_between_frames() {
    let _fonts = fonts_steady();
    let (mut d, id) = stroked_text_doc("Type");
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    r.render(&d, SIZE, SIZE, Affine::IDENTITY, &opts);
    let first = character_outlines(&r);
    assert_eq!(first.len(), 1, "the outline of the one run's stroke");
    r.render(&d, SIZE, SIZE, Affine::IDENTITY, &opts);
    let again = character_outlines(&r);
    assert!(again.len() == 1 && Arc::ptr_eq(&again[0], &first[0]), "the second frame draws the outline the first made");
    r.render(&d, SIZE, SIZE, Affine::translate((-40.0, -300.0)) * Affine::scale(4.0), &opts);
    let zoomed = character_outlines(&r);
    assert!(zoomed.len() == 1 && !Arc::ptr_eq(&zoomed[0], &first[0]), "zoomed in 4×, it is expanded again");
    if let Some(Node { kind: NodeKind::Text(t), .. }) = d.node_mut(id) {
        t.runs[0].text = "Typo".into();
    }
    r.render(&d, SIZE, SIZE, Affine::IDENTITY, &opts);
    let changed = character_outlines(&r);
    assert!(changed.len() == 1 && !Arc::ptr_eq(&changed[0], &first[0]), "the changed text is expanded again");
}

/// Frames that draw character strokes from the cache draw the same pixels, and what a renderer
/// draws whose glyph cache has no room, so that it expands them in every frame: exactly at 100%,
/// where both take a tolerance of 1/4 device pixel, and within antialiasing at 105% and 150%,
/// where the cache takes 1/8 device pixel and the frames without it about 1/4 and 1/6.
#[test]
fn cached_character_strokes_draw_what_uncached_ones_draw() {
    let _fonts = fonts_steady();
    let (d, _) = stroked_text_doc("Type");
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    for (zoom, most) in [(1.0, 0), (1.05, 24), (1.5, 24)] {
        let view = Affine::scale(zoom);
        let mut r = Renderer::new();
        let first = r.render(&d, SIZE, SIZE, view, &opts).pixels;
        assert!(r.render(&d, SIZE, SIZE, view, &opts).pixels == first, "{zoom}×: the cached frames match");
        let mut uncached = Renderer::new();
        uncached.texts = FrameCache::new(0, 0);
        let plain = uncached.render(&d, SIZE, SIZE, view, &opts).pixels;
        assert!(character_outlines(&uncached).is_empty());
        let diff = first.iter().zip(&plain).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert!(diff <= most, "{zoom}×: cached and uncached frames differ by {diff}");
        assert!(plain.iter().any(|&c| c < 128), "{zoom}×: the strokes show");
    }
}
