//! Blend-mode accuracy: every mode matches the reference formulas ([`blend_rgb`]) within 1/255
//! (see [`tolerance`]), whether it is an object's blend mode, a fill's or a group's.

use crate::*;
use vectorcraft_color::blend::blend_rgb;
use vectorcraft_color::{BlendMode as Blend, Color, Paint};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::shapes;

/// Backdrop / source pairs (8-bit, so the expected values carry no input rounding) that take every
/// branch of the formulas: dark and light backdrops, sources on both sides of ½, greys, extremes.
const PAIRS: [([u8; 3], [u8; 3]); 6] = [
    ([51, 128, 230], [153, 64, 128]),
    ([230, 26, 102], [77, 204, 179]),
    ([0, 255, 128], [255, 0, 127]),
    ([200, 180, 40], [20, 60, 250]),
    ([128, 128, 128], [240, 30, 90]),
    ([10, 90, 160], [128, 128, 128]),
];

fn rect(d: &mut Document, r: Rect, rgb: [u8; 3]) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::solid(Color::rgb8(rgb[0], rgb[1], rgb[2])), Paint::None, 0.0))
}

/// Where the blend is set: on the object, on its fill, or on a group around it.
#[derive(Clone, Copy, Debug)]
enum Level {
    Object,
    Fill,
    Group,
}

/// An opaque backdrop with the source on top of it, blended with `mode` at `level`.
fn doc(mode: Blend, level: Level, backdrop: [u8; 3], source: [u8; 3]) -> Document {
    let mut d = Document::new(20.0, 20.0);
    let below = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), backdrop);
    let mut top = rect(&mut d, Rect::new(5.0, 5.0, 15.0, 15.0), source);
    match level {
        Level::Object => top.blend = mode,
        Level::Fill => {
            if let Some(vectorcraft_doc::AppearanceItem::Fill(f)) = top.appearance.items.first_mut() {
                f.blend = mode;
            }
        }
        Level::Group => {}
    }
    let top = match level {
        Level::Group => {
            let mut g = Node::group(d.alloc_id(), vec![Arc::new(top)]);
            g.blend = mode;
            g
        }
        _ => top,
    };
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, below).unwrap();
    d.insert(Some(l), usize::MAX, top).unwrap();
    d
}

pub(crate) fn expected(mode: Blend, backdrop: [u8; 3], source: [u8; 3]) -> [u8; 3] {
    let f = |c: [u8; 3]| c.map(|v| v as f32 / 255.0);
    blend_rgb(mode, f(backdrop), f(source)).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Whether `a` is within `tol`/255 of `b` in every channel.
pub(crate) fn within(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

/// Tolerance of `mode` on screen: 1/255, except Exclusion, which the rasteriser's 8-bit pipeline
/// rounds twice (b + s − 2bs).
fn tolerance(mode: Blend) -> i32 {
    if mode == Blend::Exclusion { 2 } else { 1 }
}

#[test]
fn every_mode_matches_the_reference_formulas() {
    let mut r = Renderer::new();
    for mode in Blend::ALL {
        for level in [Level::Object, Level::Fill, Level::Group] {
            for (b, s) in PAIRS {
                let px = r.render(&doc(mode, level, b, s), 20, 20, Affine::IDENTITY, &RenderOptions::default()).pixel(10, 10);
                let want = expected(mode, b, s);
                assert!(
                    within([px[0], px[1], px[2]], want, tolerance(mode)) && px[3] == 255,
                    "{mode:?} at {level:?}, {b:?} under {s:?}: {px:?}, want {want:?}"
                );
            }
        }
    }
}

#[test]
fn a_translucent_source_mixes_the_blend_by_its_opacity() {
    let mut r = Renderer::new();
    for mode in Blend::ALL {
        let (b, s) = PAIRS[0];
        let mut d = doc(mode, Level::Object, b, s);
        let top = d.layers[0].children().unwrap()[1].id;
        d.node_mut(top).unwrap().opacity = 0.6;
        let px = r.render(&d, 20, 20, Affine::IDENTITY, &RenderOptions::default()).pixel(10, 10);
        let f = |c: [u8; 3], a: f32| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, a];
        let o = vectorcraft_color::blend::composite(mode, f(b, 1.0), f(s, 0.6));
        let want = [o[0], o[1], o[2]].map(|v| (v * 255.0).round() as u8);
        // The opacity is one more 8-bit step.
        assert!(within([px[0], px[1], px[2]], want, tolerance(mode) + 1), "{mode:?}: {px:?}, want {want:?}");
    }
}
