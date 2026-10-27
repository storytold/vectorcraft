//! Dithered gradient fills: fine noise instead of bands, the same colours on average, the same
//! pixels on every render, and nothing changed without dither or on strokes.

use crate::*;
use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::shapes;

const W: u32 = 400;
const H: u32 = 40;

/// A shallow grey ramp, #404040 → #4a4a4a: ten levels over 400 pixels, bands 40 pixels wide.
fn shallow(kind: GradientKind, dither: bool) -> Paint {
    let mut g =
        Gradient::new(kind, vec![GradientStop::new(0.0, Color::rgb8(0x40, 0x40, 0x40)), GradientStop::new(1.0, Color::rgb8(0x4a, 0x4a, 0x4a))]);
    g.dither = dither;
    Paint::Gradient(Box::new(GradientPaint::new(g)))
}

/// A W × H document with a rectangle over it painted `fill`, stroked `stroke` 10 wide.
fn doc(fill: Paint, stroke: Paint) -> Document {
    let mut d = Document::new(W as f64, H as f64);
    let id = d.alloc_id();
    let n = Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, W as f64, H as f64)), Appearance::basic(fill, stroke, 10.0));
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    d
}

fn render(d: &Document, xf: Affine) -> Rendered {
    Renderer::new().render(d, W, H, xf, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

/// The longest run of identical pixels along row `y` (the stroke's 5 pixels at each end left out).
fn longest_run(r: &Rendered, y: u32) -> usize {
    let row: Vec<[u8; 4]> = (5..W - 5).map(|x| r.pixel(x, y)).collect();
    row.chunk_by(|a, b| a == b).map(<[_]>::len).max().unwrap_or(0)
}

#[test]
fn a_dithered_shallow_gradient_has_no_bands_where_the_plain_one_does() {
    let plain = render(&doc(shallow(GradientKind::Linear, false), Paint::None), Affine::IDENTITY);
    let dithered = render(&doc(shallow(GradientKind::Linear, true), Paint::None), Affine::IDENTITY);
    for y in [3, 20, 36] {
        assert!(longest_run(&plain, y) >= 30, "the plain ramp bands: row {y}, longest run {}", longest_run(&plain, y));
        assert!(longest_run(&dithered, y) <= 12, "dithered: row {y}, longest run {}", longest_run(&dithered, y));
    }
    // The noise moves no colour: each 20-pixel stretch averages what the plain ramp does.
    for x0 in (0..W).step_by(20) {
        let mean = |r: &Rendered| {
            (x0..x0 + 20).flat_map(|x| (0..H).map(move |y| (x, y))).map(|(x, y)| f64::from(r.pixel(x, y)[0])).sum::<f64>() / f64::from(20 * H)
        };
        assert!((mean(&plain) - mean(&dithered)).abs() < 0.35, "x {x0}: {} vs {}", mean(&plain), mean(&dithered));
    }
    // Noise of at most a level or so either way.
    for (x, y) in [(10, 10), (200, 20), (390, 30)] {
        let (a, b) = (i32::from(plain.pixel(x, y)[0]), i32::from(dithered.pixel(x, y)[0]));
        assert!((a - b).abs() <= 2, "({x}, {y}): {a} vs {b}");
    }
}

#[test]
fn dithered_renders_are_the_same_every_time() {
    let d = doc(shallow(GradientKind::Radial, true), Paint::None);
    let (a, b) = (render(&d, Affine::IDENTITY), render(&d, Affine::IDENTITY));
    assert!((0..H).all(|y| (0..W).all(|x| a.pixel(x, y) == b.pixel(x, y))));
    // And with the grid drawn afresh on another renderer, panned by a whole pixel.
    let moved = Renderer::new().render(
        &d,
        W,
        H,
        Affine::translate((1.0, 0.0)),
        &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() },
    );
    assert!((0..H).all(|y| (1..W).all(|x| moved.pixel(x, y) == a.pixel(x - 1, y))), "the noise moves with the art");
}

#[test]
fn dither_leaves_strokes_and_undithered_fills_as_they_were() {
    // Dither applies to fills only: a stroke draws its gradient as before.
    let fill = Paint::solid(Color::WHITE);
    let a = render(&doc(fill.clone(), shallow(GradientKind::Linear, false)), Affine::IDENTITY);
    let b = render(&doc(fill, shallow(GradientKind::Linear, true)), Affine::IDENTITY);
    assert!((0..H).all(|y| (0..W).all(|x| a.pixel(x, y) == b.pixel(x, y))), "a dithered gradient on a stroke draws as an undithered one");
    // Off: the vector gradient, its bands where they always were.
    let plain = render(&doc(shallow(GradientKind::Linear, false), Paint::None), Affine::IDENTITY);
    assert_eq!(plain.pixel(0, 20)[0], 0x40);
    assert_eq!(plain.pixel(W - 1, 20)[0], 0x4a);
}

#[test]
fn a_zoomed_or_rotated_dithered_gradient_keeps_its_colours() {
    for xf in [Affine::scale(3.0), Affine::rotate_about(0.3, vectorcraft_geom::Point::new(200.0, 20.0)), Affine::scale(40.0)] {
        let plain = render(&doc(shallow(GradientKind::Linear, false), Paint::None), xf);
        let dithered = render(&doc(shallow(GradientKind::Linear, true), Paint::None), xf);
        for (x, y) in [(50, 10), (150, 20), (250, 30)] {
            let (a, b) = (plain.pixel(x, y), dithered.pixel(x, y));
            assert!(a.iter().zip(b).all(|(p, q)| (i32::from(*p) - i32::from(q)).abs() <= 2), "{xf:?} ({x}, {y}): {a:?} vs {b:?}");
        }
    }
}

#[test]
fn a_large_dithered_box_mostly_off_screen_samples_only_what_shows() {
    // A 2000-unit square, the shallow ramp across it: a 2048-cell grid a side, 16 MiB.
    const S: u32 = 2000;
    let mut d = Document::new(f64::from(S), f64::from(S));
    let id = d.alloc_id();
    let n = Node::path(
        id,
        shapes::rectangle(Rect::new(0.0, 0.0, f64::from(S), f64::from(S))),
        Appearance::basic(shallow(GradientKind::Linear, true), Paint::None, 0.0),
    );
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let whole = Renderer::new().render(&d, S, S, Affine::IDENTITY, &opts);
    // A 64-pixel view of its middle draws the pixels the whole render has there...
    let mut part = None;
    let bytes = dither::grid_bytes_after(|| part = Some(Renderer::new().render(&d, 64, 64, Affine::translate((-1000.0, -1000.0)), &opts)));
    let part = part.unwrap();
    assert!((0..64).all(|y| (0..64).all(|x| part.pixel(x, y) == whole.pixel(1000 + x, 1000 + y))), "the view draws what the whole render does");
    // ...from the cells around it only, not the whole grid.
    assert!(bytes > 0 && bytes <= 512 * 512 * 4, "{bytes} bytes of grid for a 64-pixel view");
}

#[test]
fn a_dithered_gradient_without_stops_draws_nothing_and_samples_nothing() {
    let mut g = Gradient::new(GradientKind::Linear, Vec::new());
    g.dither = true;
    let d = doc(Paint::Gradient(Box::new(GradientPaint::new(g))), Paint::None);
    let mut r = None;
    let bytes = dither::grid_bytes_after(|| r = Some(render(&d, Affine::IDENTITY)));
    assert_eq!(bytes, 0, "no grid is sampled");
    let r = r.unwrap();
    assert!((0..H).all(|y| (0..W).all(|x| r.pixel(x, y) == [255, 255, 255, 255])), "nothing is drawn");
}

/// Blue to yellow across the document, mixed `method`'s way, dithered or not.
fn blue_yellow(method: vectorcraft_color::GradientMethod, dither: bool) -> Paint {
    let mut g = Gradient::new(
        GradientKind::Linear,
        vec![GradientStop::new(0.0, Color::rgb(0.0, 0.0, 1.0)), GradientStop::new(1.0, Color::rgb(1.0, 1.0, 0.0))],
    );
    (g.method, g.dither) = (method, dither);
    Paint::Gradient(Box::new(GradientPaint::new(g)))
}

#[test]
fn a_dithered_perceptual_gradient_is_the_perceptual_mix_with_noise() {
    use vectorcraft_color::GradientMethod::{Classic, Perceptual};
    // Classic first: the two share stops and box, and must not share a dithered grid.
    let classic = render(&doc(blue_yellow(Classic, true), Paint::None), Affine::IDENTITY);
    let dithered = render(&doc(blue_yellow(Perceptual, true), Paint::None), Affine::IDENTITY);
    // Each pixel within a level or so of the perceptual mix across it (its grid cell's centre lies
    // in it; the noise, rounding, and the mix's stops within half a level), and dither's noise
    // moving many off the rounded mix at its centre.
    let Paint::Gradient(want) = blue_yellow(Perceptual, false) else { panic!("a gradient") };
    let mix = |x: f32| want.gradient.sample(x / W as f32).0.to_rgb().map(|v| v.clamp(0.0, 1.0) * 255.0);
    let mut noisy = 0;
    for x in 5..W - 5 {
        let p = dithered.pixel(x, 20);
        let (a, m, b) = (mix(x as f32), mix(x as f32 + 0.5), mix(x as f32 + 1.0));
        let near = |k: usize| (a[k].min(b[k]) - 2.0..=a[k].max(b[k]) + 2.0).contains(&f32::from(p[k]));
        assert!((0..3).all(near), "({x}, 20): {p:?} vs {a:?} to {b:?}");
        noisy += usize::from((0..3).any(|k| p[k] != m[k].round() as u8));
    }
    assert!(noisy > 100, "dither's noise: {noisy} pixels off the rounded mix");
    // The middle: a blue-grey (OKLab), where Classic+Dither gives the display RGB mid grey.
    let [r, g, b, _] = dithered.pixel(W / 2, 20).map(i32::from);
    assert!(b > r + 20 && b > g + 10, "perceptual: {:?}", [r, g, b]);
    let [r, g, b, _] = classic.pixel(W / 2, 20).map(i32::from);
    assert!((r - b).abs() <= 4 && (g - b).abs() <= 4, "classic: {:?}", [r, g, b]);
}
