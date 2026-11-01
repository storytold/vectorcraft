//! Vector Halftone's live art: drawn as dots, following its settings, kept while the art doesn't
//! change, and refused past the caps.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node};
use vectorcraft_effects::addons::halftone::HALFTONE;
use vectorcraft_geom::{Affine, Rect, shapes};

use super::{SAMPLED, art};
use crate::{RenderOptions, Renderer};

/// A document with a mid-grey square of side `side` at (`x`, 10) carrying Vector Halftone with
/// `params`, and the square.
fn doc(x: f64, side: f64, params: Value) -> (Document, Node) {
    let mut d = Document::new(200.0, 200.0);
    let grey = Paint::solid(Color::rgb(0.5, 0.5, 0.5));
    let mut n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 10.0, x + side, 10.0 + side)), Appearance::basic(grey, Paint::None, 0.0));
    n.appearance.effects.push(Effect { id: HALFTONE.into(), params, visible: true });
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n).unwrap();
    let n = (*d.layers[0].children().unwrap()[0]).clone();
    (d, n)
}

fn sampled() -> usize {
    SAMPLED.with(std::cell::Cell::get)
}

/// The square's pixels (from (x, 10), `side` wide), drawn on white.
fn square_pixels(d: &Document, x: f64, side: f64) -> Vec<[u8; 4]> {
    let r = Renderer::new().render(d, 200, 200, Affine::IDENTITY, &RenderOptions { background: Some([255; 4]), ..Default::default() });
    let (x0, y0, s) = (x as u32, 10, side as u32);
    (y0..y0 + s).flat_map(|y| (x0..x0 + s).map(move |x| (x, y))).map(|(x, y)| r.pixel(x, y)).collect()
}

#[test]
fn the_effect_draws_the_grey_as_black_dots_on_white() {
    // 6 lpi: 12 pt cells, each half black for the mid-grey.
    let (d, _) = doc(10.0, 60.0, json!({"frequency": 6}));
    let px = square_pixels(&d, 10.0, 60.0);
    let share = |f: &dyn Fn(&[u8; 4]) -> bool| px.iter().filter(|p| f(p)).count() as f64 / px.len() as f64;
    let (black, white) = (share(&|p| p[0] < 40), share(&|p| p[0] > 215));
    assert!(black > 0.25 && white > 0.25, "black {black}, white {white}");
    // Without the effect, the square is grey all over.
    let mut plain = d.clone();
    let id = d.layers[0].children().unwrap()[0].id;
    plain.node_mut(id).unwrap().appearance.effects.clear();
    assert!(square_pixels(&plain, 10.0, 60.0).iter().all(|p| (100..=160).contains(&p[0])));
}

#[test]
fn the_settings_change_the_dots() {
    let look = |params: Value| square_pixels(&doc(20.0, 60.0, params).0, 20.0, 60.0);
    let base = look(json!({"frequency": 18}));
    for other in [
        json!({"frequency": 9}),
        json!({"frequency": 18, "angle": 0}),
        json!({"frequency": 18, "shape": "square"}),
        json!({"frequency": 18, "invert": true}),
        json!({"frequency": 18, "mode": "cmyk"}),
    ] {
        assert_ne!(look(other.clone()), base, "{other}");
    }
    // A red dot colour paints red dots.
    let red = look(json!({"frequency": 18, "color": "#ff0000"}));
    assert!(red.iter().any(|p| p[0] > 215 && p[1] < 40 && p[2] < 40));
    // The art's own colour under each dot.
    let (d, _) = doc(20.0, 60.0, json!({"frequency": 18, "fromArt": true}));
    assert!(square_pixels(&d, 20.0, 60.0).iter().any(|p| (100..=160).contains(&p[0]) && p[0] == p[1]));
}

#[test]
fn unchanged_art_is_not_sampled_again() {
    let (d, n) = doc(30.0, 50.0, json!({"frequency": 12}));
    let before = sampled();
    let first = art(&d, &n).unwrap();
    assert_eq!(sampled(), before + 1);
    // Drawn again, in a copy of the object or of the document, by another renderer: the same dots.
    assert_eq!(art(&d.clone(), &n.clone()), Some(first.clone()));
    square_pixels(&d, 30.0, 50.0);
    square_pixels(&d, 30.0, 50.0);
    assert_eq!(sampled(), before + 1);
    // Changed settings, or a moved object, are sampled again.
    let mut changed = n.clone();
    changed.appearance.effects[0].params = json!({"frequency": 13});
    assert_ne!(art(&d, &changed), Some(first));
    assert_eq!(sampled(), before + 2);
    let (moved, m) = doc(31.0, 50.0, json!({"frequency": 12}));
    art(&moved, &m).unwrap();
    assert_eq!(sampled(), before + 3);
}

#[test]
fn too_many_cells_draw_the_object_as_it_is() {
    let (d, n) = doc(40.0, 10_000.0, json!({"frequency": 300}));
    assert!(art(&d, &n).is_none());
    // Drawn without dots, and without a panic.
    assert!(square_pixels(&d, 40.0, 100.0).iter().all(|p| (100..=160).contains(&p[0])));
    let (d, n) = doc(40.0, 0.0, json!({}));
    assert!(art(&d, &n).is_none(), "no area");
}

#[test]
fn unclipped_dots_reach_past_the_object() {
    let (d, n) = doc(50.0, 40.0, json!({"frequency": 9, "invert": true, "clip": false}));
    let b = crate::effect_bounds(&d.symbol_boxes(), &n).unwrap();
    let dots = art(&d, &n).unwrap().visual_bounds().unwrap();
    assert!(b.x0 <= dots.x0 && b.y0 <= dots.y0 && b.x1 >= dots.x1 && b.y1 >= dots.y1, "{b:?} holds {dots:?}");
    assert!(dots.x0 < 50.0, "the inverted white around the object gets dots: {dots:?}");
}
