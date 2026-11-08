//! Knockout groups, the knockout shape and the page group (Transparency panel).

use crate::*;
use vectorcraft_color::{BlendMode as Blend, Color, Paint};
use vectorcraft_doc::{Appearance, Knockout, Node};
use vectorcraft_geom::shapes;

/// A 50% opaque rectangle: red at 10..60, blue at 40..90 (they overlap at 40..60).
fn half(d: &mut Document, x: f64, rgb: (f32, f32, f32)) -> Node {
    let id = d.alloc_id();
    let mut n = Node::path(
        id,
        shapes::rectangle(Rect::new(x, 10.0, x + 50.0, 90.0)),
        Appearance::basic(Paint::solid(Color::rgb(rgb.0, rgb.1, rgb.2)), Paint::None, 0.0),
    );
    n.opacity = 0.5;
    n
}

/// A document whose layer holds a group (`knockout`) of a 50% red and a 50% blue rectangle,
/// after `tweak` adjusts the group.
fn doc(knockout: Knockout, tweak: impl FnOnce(&mut Document, &mut Node)) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let red = half(&mut d, 10.0, (1.0, 0.0, 0.0));
    let blue = half(&mut d, 40.0, (0.0, 0.0, 1.0));
    let id = d.alloc_id();
    let mut g = Node::group(id, vec![Arc::new(red), Arc::new(blue)]);
    g.knockout = knockout;
    tweak(&mut d, &mut g);
    d.insert(Some(d.layers[0].id), 0, g).unwrap();
    d
}

fn render(d: &Document) -> Rendered {
    Renderer::new().render(d, 100, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= 3)
}

const BLUE_OVER_WHITE: [u8; 4] = [128, 128, 255, 255];
/// 50% blue over 50% red over white.
const BLUE_OVER_RED: [u8; 4] = [128, 64, 191, 255];
/// The overlap with half of the red knocked out by an opaque blue: 0.5 · (50% red over white) +
/// 0.5 · blue (for Normal blending, the same as 50% blue over the red).
const HALF_KNOCKED: [u8; 4] = [128, 64, 191, 255];

#[test]
fn knockout_group_shows_the_top_object_over_the_backdrop() {
    let on = render(&doc(Knockout::On, |_, _| {}));
    assert!(close(on.pixel(50, 50), BLUE_OVER_WHITE), "overlap: {:?}", on.pixel(50, 50));
    assert!(close(on.pixel(20, 50), [255, 128, 128, 255]), "red alone: {:?}", on.pixel(20, 50));
    for k in [Knockout::Off, Knockout::Neutral] {
        let r = render(&doc(k, |_, _| {}));
        assert!(close(r.pixel(50, 50), BLUE_OVER_RED), "{k:?}: {:?}", r.pixel(50, 50));
    }
}

#[test]
fn opacity_defines_the_knockout_shape() {
    // The blue object's 50% opacity knocks out only half of the red below it.
    let r = render(&doc(Knockout::On, |_, g| {
        let blue = Arc::make_mut(&mut g.children_mut().unwrap()[1]);
        blue.knockout_shape = true;
    }));
    assert!(close(r.pixel(50, 50), HALF_KNOCKED), "{:?}", r.pixel(50, 50));
    // Its opacity mask counts too when it defines the shape, and is ignored otherwise.
    for shape in [false, true] {
        let r = render(&doc(Knockout::On, |d, g| {
            let id = d.alloc_id();
            let white = Node::path(
                id,
                shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)),
                Appearance::basic(Paint::solid(Color::rgb(0.5, 0.5, 0.5)), Paint::None, 0.0),
            );
            let blue = Arc::make_mut(&mut g.children_mut().unwrap()[1]);
            blue.opacity = 1.0;
            blue.knockout_shape = shape;
            blue.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(white, true)));
        }));
        let want = if shape { HALF_KNOCKED } else { BLUE_OVER_WHITE };
        // A mid-grey mask is ~50% (luminance 0.5, not gamma-corrected): allow a little more slack.
        let px = r.pixel(50, 50);
        assert!(px.iter().zip(want).all(|(a, b)| (*a as i32 - b as i32).abs() <= 6), "shape {shape}: {px:?}");
    }
}

#[test]
fn neutral_groups_pass_knockout_through() {
    // Red and blue in a neutral group inside a knockout group knock each other out…
    let nested = |inner: Knockout| {
        doc(Knockout::On, |d, g| {
            let children = std::mem::take(g.children_mut().unwrap());
            let mut inner_group = Node::group(d.alloc_id(), children);
            inner_group.knockout = inner;
            *g.children_mut().unwrap() = vec![Arc::new(inner_group)];
        })
    };
    assert!(close(render(&nested(Knockout::Neutral)).pixel(50, 50), BLUE_OVER_WHITE));
    // …an Off group's do not…
    assert!(close(render(&nested(Knockout::Off)).pixel(50, 50), BLUE_OVER_RED));
    // …and with no knockout group around it, a neutral group knocks out nothing.
    let mut plain = nested(Knockout::Neutral);
    Arc::make_mut(&mut Arc::make_mut(&mut plain.layers[0]).children_mut().unwrap()[0]).knockout = Knockout::Neutral;
    assert!(close(render(&plain).pixel(50, 50), BLUE_OVER_RED));
}

#[test]
fn page_knockout_and_isolated_blending() {
    // Page Knockout Group: the neutral layer passes its objects through to the page.
    let mut d = doc(Knockout::Neutral, |_, _| {});
    d.page_knockout = true;
    assert!(close(render(&d).pixel(50, 50), BLUE_OVER_WHITE), "{:?}", render(&d).pixel(50, 50));
    // A layer set to Off keeps its objects from knocking each other out.
    Arc::make_mut(&mut d.layers[0]).knockout = Knockout::Off;
    assert!(close(render(&d).pixel(50, 50), BLUE_OVER_RED));

    // Page Isolated Blending: a Difference object no longer blends with the white under the page.
    let mut d = Document::new(100.0, 100.0);
    let mut red = half(&mut d, 10.0, (1.0, 0.0, 0.0));
    red.opacity = 1.0;
    red.blend = Blend::Difference;
    d.insert(Some(d.layers[0].id), 0, red).unwrap();
    assert!(close(render(&d).pixel(30, 50), [0, 255, 255, 255]), "inverts the white: {:?}", render(&d).pixel(30, 50));
    d.page_isolate = true;
    assert!(close(render(&d).pixel(30, 50), [255, 0, 0, 255]), "isolated: {:?}", render(&d).pixel(30, 50));
}

#[test]
fn page_knockout_looks_the_same_when_another_object_blends() {
    // With Page Knockout Group on, a Multiply square on the page makes the renderer draw the page
    // over a copy of what is below it (white, or nothing). Four objects knock out part of the
    // orange page: a blue square whose left edge covers half of pixel column 10, a 50% blue square
    // whose opacity defines its knockout shape, a 50% blue square, and an isolated group of a green
    // square and a 50% group holding a Multiply square, which draws its middle half transparent.
    // Each looks the same with and without the Multiply square.
    let rect = |d: &mut Document, r: Rect, (red, green, blue): (f32, f32, f32)| {
        Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::solid(Color::rgb(red, green, blue)), Paint::None, 0.0))
    };
    let page = |blending: bool| {
        let mut d = Document::new(100.0, 100.0);
        d.page_knockout = true;
        let orange = rect(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), (0.8, 0.6, 0.2));
        let edge = rect(&mut d, Rect::new(10.5, 10.0, 30.5, 30.0), (0.1, 0.3, 0.9));
        let mut shaped = rect(&mut d, Rect::new(40.0, 10.0, 60.0, 30.0), (0.1, 0.3, 0.9));
        (shaped.opacity, shaped.knockout_shape) = (0.5, true);
        let mut translucent = rect(&mut d, Rect::new(70.0, 10.0, 90.0, 30.0), (0.1, 0.3, 0.9));
        translucent.opacity = 0.5;
        let green = rect(&mut d, Rect::new(10.0, 40.0, 60.0, 90.0), (0.2, 0.9, 0.4));
        let mut multiply = rect(&mut d, Rect::new(20.0, 50.0, 50.0, 80.0), (0.5, 0.5, 1.0));
        multiply.blend = Blend::Multiply;
        let mut inner = Node::group(d.alloc_id(), vec![Arc::new(multiply)]);
        inner.opacity = 0.5;
        let mut isolated = Node::group(d.alloc_id(), vec![Arc::new(green), Arc::new(inner)]);
        isolated.isolate = true;
        let mut objects = vec![orange, edge, shaped, translucent, isolated];
        if blending {
            let mut other = rect(&mut d, Rect::new(70.0, 70.0, 90.0, 90.0), (0.5, 0.5, 1.0));
            other.blend = Blend::Multiply;
            objects.push(other);
        }
        let l = d.layers[0].id;
        for n in objects {
            d.insert(Some(l), usize::MAX, n).unwrap();
        }
        d
    };
    // Half of the orange stays and blue takes the other half: 0.5 · orange + 0.5 · blue. The 50%
    // square knocks out all of the orange and shows over what is below the page: 0.5 · white +
    // 0.5 · blue, or blue at 50% over nothing.
    let half_blue = [115, 115, 140, 255];
    for (below, half_square) in [(Some([255, 255, 255, 255]), [140, 166, 242, 255]), (None, [26, 77, 230, 128])] {
        let opts = RenderOptions { background: below, ..Default::default() };
        let draw = |d: &Document| Renderer::new().render(d, 100, 100, Affine::IDENTITY, &opts);
        let (plain, blending) = (draw(&page(false)), draw(&page(true)));
        let cases = [("half-covered edge", 10, 20, half_blue), ("knockout shape", 50, 20, half_blue), ("50%", 80, 20, half_square)];
        for (what, x, y, want) in cases {
            for r in [&plain, &blending] {
                assert!(close(r.pixel(x, y), want), "{what} over {below:?}: {:?}", r.pixel(x, y));
            }
        }
        let (a, b) = (plain.pixel(35, 65), blending.pixel(35, 65));
        assert!(close(a, b), "the group's middle over {below:?}: {a:?} alone, {b:?} with the Multiply square");
    }
}

#[test]
fn opaque_knockout_fills_keep_pixels_without_per_element_backdrop_rasters() {
    // Fractional curved edges and holes exercise coverage, not just opaque interiors. A disabled
    // mask is visually neutral and keeps the general knockout path as an oracle, without adding
    // another compositing layer or rounding step to each fill.
    for precise in [false, true] {
        for (anti_alias, supersample) in [(AntiAlias::Art, false), (AntiAlias::Art, true), (AntiAlias::None, false)] {
            for copy_backdrop in [false, true] {
                let make = |reference: bool| {
                    let mut d = Document::new(80.0, 60.0);
                    let mut members = Vec::new();
                    for i in 0..12 {
                        let outer = shapes::ellipse(Rect::new(2.3 + f64::from(i), 3.7, 65.4, 53.2));
                        let inner = shapes::ellipse(Rect::new(20.2, 20.8, 40.6, 38.1));
                        let child = Node::path(d.alloc_id(), inner, Appearance::default());
                        let first = Node::path(d.alloc_id(), outer, Appearance::default());
                        let mut n = if i % 2 == 0 {
                            Node::new(d.alloc_id(), NodeKind::Compound { children: vec![Arc::new(first), Arc::new(child)], rule: FillRule::EvenOdd })
                        } else {
                            first
                        };
                        n.appearance = Appearance::basic(Paint::solid(Color::rgb(i as f32 / 12.0, 0.3, 0.7)), Paint::None, 0.0).into();
                        if reference {
                            let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 80.0, 60.0)), Appearance::default());
                            let mut mask = vectorcraft_doc::OpacityMask::new(art, false);
                            mask.disabled = true;
                            n.extras_mut().mask = Some(Box::new(mask));
                        }
                        members.push(Arc::new(n));
                    }
                    if copy_backdrop {
                        let mut blend = half(&mut d, 70.0, (0.5, 0.5, 0.5));
                        blend.blend = Blend::Multiply;
                        members.push(Arc::new(blend));
                    }
                    let mut g = Node::group(d.alloc_id(), members);
                    g.knockout = Knockout::On;
                    g.opacity = 0.5;
                    d.insert(Some(d.layers[0].id), 0, g).unwrap();
                    d
                };
                let opts = RenderOptions { background: Some([190, 210, 230, 255]), precise, anti_alias, supersample, ..Default::default() };
                let draw = |reference| {
                    crate::group::PIXELS_DRAWN.with(|n| n.set(0));
                    let mut r = Renderer::new();
                    r.threads = 0;
                    let img = r.render(&make(reference), 80, 60, Affine::IDENTITY, &opts);
                    let pixels = crate::group::PIXELS_DRAWN.with(|n| n.get());
                    (img, pixels, r.stats.drawn)
                };
                let (fast, fast_pixels, fast_draws) = draw(false);
                let (reference, reference_pixels, reference_draws) = draw(true);
                assert!(fast.pixels == reference.pixels, "precise={precise}, aa={anti_alias:?}, supersample={supersample}, backdrop={copy_backdrop}");
                if anti_alias == AntiAlias::None || supersample {
                    assert!(fast_draws < reference_draws, "opaque elements should draw once");
                    if copy_backdrop {
                        assert!(fast_pixels < reference_pixels, "opaque elements should not rasterize a backdrop per element");
                    }
                } else {
                    assert_eq!(fast_draws, reference_draws, "analytical AA must keep its rounding");
                    assert_eq!(fast_pixels, reference_pixels);
                }
            }
        }
    }
}

#[test]
fn translucent_knockout_objects_and_fills_keep_their_compositing() {
    for fill_opacity in [false, true] {
        let d = doc(Knockout::On, |_, g| {
            if fill_opacity {
                for n in g.children_mut().unwrap() {
                    let n = Arc::make_mut(n);
                    n.opacity = 1.0;
                    for item in &mut n.appearance.items {
                        if let vectorcraft_doc::AppearanceItem::Fill(fill) = item {
                            fill.opacity = 0.5;
                        }
                    }
                }
            }
        });
        let opts = RenderOptions { anti_alias: AntiAlias::None, background: Some([255; 4]), ..Default::default() };
        let mut renderer = Renderer::new();
        let r = renderer.render(&d, 100, 100, Affine::IDENTITY, &opts);
        // Object opacity is ignored by its knockout shape; a fill's own opacity is retained.
        let want = if fill_opacity { BLUE_OVER_RED } else { BLUE_OVER_WHITE };
        assert!(close(r.pixel(50, 50), want), "fill opacity={fill_opacity}: {:?}", r.pixel(50, 50));
        let nodes = d.layers.iter().map(|n| n.count()).sum::<usize>();
        assert!(renderer.stats.drawn > nodes, "translucent elements keep the separate knockout passes");
    }
}
