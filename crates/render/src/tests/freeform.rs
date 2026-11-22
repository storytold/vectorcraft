//! Freeform gradients: the rendered colour grid against the colour field.

use crate::*;
use vectorcraft_color::freeform::spread_scale;
use vectorcraft_color::{Color, Freeform, FreeformPoint, Gradient, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::{Point, shapes};

/// Red at the top left, blue at the bottom right, a half-transparent green at the top right.
fn freeform() -> Freeform {
    let p = |x, y, c, opacity| FreeformPoint { opacity, spread: 0.1, ..FreeformPoint::new(Point::new(x, y), c) };
    Freeform {
        points: vec![
            p(20.0, 20.0, Color::rgb(1.0, 0.0, 0.0), 1.0),
            p(80.0, 80.0, Color::rgb(0.0, 0.0, 1.0), 1.0),
            p(80.0, 20.0, Color::rgb(0.0, 1.0, 0.0), 0.5),
        ],
        ..Default::default()
    }
}

fn paint(f: Option<Freeform>) -> Paint {
    let mut g = GradientPaint::new(Gradient { kind: GradientKind::Freeform, ..Default::default() });
    g.freeform = f;
    Paint::Gradient(Box::new(g))
}

/// A 100 × 100 document with `shape` (on 0..100) painted `fill`, at `opacity`.
fn doc(shape: vectorcraft_geom::PathData, fill: Paint, opacity: f32) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let id = d.alloc_id();
    let mut n = Node::path(id, shape, Appearance::basic(fill, Paint::None, 0.0));
    n.opacity = opacity;
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    d
}

fn render(d: &Document) -> Rendered {
    Renderer::new().render(d, 100, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

/// The field's colour at `p` composited over white (8-bit).
fn expected(f: &Freeform, p: Point, opacity: f32) -> [u8; 4] {
    let ([r, g, b], a) = f.field(spread_scale(Rect::new(0.0, 0.0, 100.0, 100.0))).sample(p);
    let a = a * opacity;
    let q = |v: f32| ((v * a + (1.0 - a)) * 255.0).round() as u8;
    [q(r), q(g), q(b), 255]
}

fn near(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

/// `a` and `b` on either side of (50.5, 50.5): the point a pixel center and a grid cell share.
fn pair(a: Color, b: Color) -> Freeform {
    let p = |x, c| FreeformPoint::new(Point::new(x, 50.5), c);
    Freeform { points: vec![p(20.5, a), p(80.5, b)], ..Default::default() }
}

const CYAN: Color = Color::Cmyk { c: 1.0, m: 0.0, y: 0.0, k: 0.0 };
const BLACK: Color = Color::Cmyk { c: 0.0, m: 0.0, y: 0.0, k: 1.0 };

/// The middle pixel of a square painted with `pair(a, b)`.
fn middle(a: Color, b: Color) -> [u8; 4] {
    render(&doc(shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), paint(Some(pair(a, b))), 1.0)).pixel(50, 50)
}

#[test]
fn points_that_share_a_model_mix_in_it() {
    let want = Color::cmyk(0.5, 0.0, 0.0, 0.5).to_rgba8(1.0);
    assert!(near(middle(CYAN, BLACK), want, 3), "CMYK: {:?} vs {want:?}", middle(CYAN, BLACK));
    let (lab1, lab2) = (Color::lab(90.0, -60.0, 40.0), Color::lab(30.0, 60.0, -40.0));
    let want = Color::lab(60.0, 0.0, 0.0).to_rgba8(1.0);
    assert!(near(middle(lab1, lab2), want, 3), "Lab: {:?} vs {want:?}", middle(lab1, lab2));
    let want = Color::gray(0.5).to_rgba8(1.0);
    assert!(near(middle(Color::gray(0.2), Color::gray(0.8)), want, 3), "Gray");
}

#[test]
fn rgb_points_and_points_in_different_models_mix_in_display_rgb() {
    let (red, blue) = (Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
    let want = Color::rgb(0.5, 0.0, 0.5).to_rgba8(1.0);
    assert!(near(middle(red, blue), want, 3), "RGB: {:?} vs {want:?}", middle(red, blue));
    let gray = Color::gray(0.5);
    let [r, g, b] = [red.to_rgb(), gray.to_rgb()].into_iter().fold([0.0; 3], |m, c| [m[0] + c[0] / 2.0, m[1] + c[1] / 2.0, m[2] + c[2] / 2.0]);
    let want = Color::rgb(r, g, b).to_rgba8(1.0);
    assert!(near(middle(red, gray), want, 3), "mixed models: {:?} vs {want:?}", middle(red, gray));
}

/// The middle grid cell of `pair(a, b)` rasterized on a 101-point box with `ink`, as RGB.
fn middle_cell(a: Color, b: Color, ink: crate::ink::Ink) -> [u8; 3] {
    let cell = crate::freeform::rasterize(&pair(a, b), Rect::new(0.0, 0.0, 101.0, 101.0), 17, 17, ink).sample(8, 8);
    [cell.r, cell.g, cell.b]
}

#[test]
fn ink_planes_separate_the_mix_of_lab_points() {
    use crate::ink::Ink;
    let (lab1, lab2) = (Color::lab(90.0, -60.0, 40.0), Color::lab(30.0, 60.0, -40.0));
    let mid = Color::lab(60.0, 0.0, 0.0);
    // A color far out of gamut stays as exact as a solid fill of it.
    let far = Color::lab(20.0, -125.0, 125.0);
    for ink in [Ink::Cmy, Ink::K] {
        for (a, b, c) in [(lab1, lab2, mid), (far, far, far)] {
            let want = ink.color(&c, 1.0).to_rgba8();
            let got = middle_cell(a, b, ink);
            assert!(near([got[0], got[1], got[2], 255], [want.r, want.g, want.b, 255], 2), "{ink:?} {c:?}: {got:?} vs {want:?}");
        }
    }
}

#[test]
fn ink_planes_separate_the_mix_of_rgb_points() {
    use crate::ink::Ink;
    let (red, blue) = (Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
    let mid = Color::rgb(0.5, 0.0, 0.5);
    for ink in [Ink::Cmy, Ink::K] {
        let want = ink.color(&mid, 1.0).to_rgba8();
        let got = middle_cell(red, blue, ink);
        assert!(near([got[0], got[1], got[2], 255], [want.r, want.g, want.b, 255], 3), "{ink:?}: {got:?} vs {want:?}");
    }
}

#[test]
fn lab_points_on_an_ink_plane_use_a_coarser_grid() {
    use crate::ink::Ink;
    let b = Rect::new(0.0, 0.0, 1000.0, 1000.0);
    let lab = pair(Color::lab(90.0, -60.0, 40.0), Color::lab(30.0, 60.0, -40.0));
    let (cols, rows) = crate::freeform::cells(&lab, b, 4000.0, Ink::Cmy);
    assert_eq!((cols, rows), (64, 64));
    assert_eq!(crate::freeform::cells(&lab, b, 4000.0, Ink::K), (64, 64));
    let (wide, _) = crate::freeform::cells(&lab, b, 4000.0, Ink::Display);
    assert!(wide > 64, "{wide}");
    let (cmyk, _) = crate::freeform::cells(&pair(CYAN, BLACK), b, 4000.0, Ink::Cmy);
    assert!(cmyk > 64, "{cmyk}");
}

#[test]
fn pure_black_points_fade_from_rgb_black_under_rich_black() {
    use crate::ink::Ink;
    let white = Color::cmyk(0.0, 0.0, 0.0, 0.0);
    let [r, g, b] = white.to_rgb();
    let want = [r, g, b].map(|v| (v * 0.5 * 255.0).round() as u8);
    let got = middle_cell(BLACK, white, Ink::RichBlack);
    assert!(near([got[0], got[1], got[2], 255], [want[0], want[1], want[2], 255], 2), "{got:?} vs {want:?}");
    // Without a pure black point, the points mix in their model.
    let (a, c) = (Color::cmyk(0.0, 0.0, 0.0, 0.9), white);
    let want = Color::cmyk(0.0, 0.0, 0.0, 0.45).to_rgba8(1.0);
    let got = middle_cell(a, c, Ink::RichBlack);
    assert!(near([got[0], got[1], got[2], 255], want, 2), "{got:?} vs {want:?}");
}

#[test]
fn darkening_changes_the_points_before_they_mix() {
    use crate::ink::Ink;
    // Half black added to each point: K100 stays K100 and K0 becomes K50, so they meet at K75.
    let got = middle_cell(BLACK, Color::cmyk(0.0, 0.0, 0.0, 0.0), Ink::Display.darkened(0.5));
    let want = Color::cmyk(0.0, 0.0, 0.0, 0.75).to_rgba8(1.0);
    assert!(near([got[0], got[1], got[2], 255], want, 2), "{got:?} vs {want:?}");
}

#[test]
fn the_rendered_grid_follows_the_colour_field() {
    let f = freeform();
    let r = render(&doc(shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), paint(Some(f.clone())), 1.0));
    assert_eq!(r.pixel(20, 20), [255, 0, 0, 255], "the red point's colour");
    assert_eq!(r.pixel(80, 80), [0, 0, 255, 255], "the blue point's colour");
    for (x, y) in [(50, 50), (35, 60), (65, 30), (10, 90), (95, 5), (50, 20)] {
        let want = expected(&f, Point::new(x as f64 + 0.5, y as f64 + 0.5), 1.0);
        assert!(near(r.pixel(x, y), want, 6), "({x}, {y}): {:?} vs {want:?}", r.pixel(x, y));
    }
}

#[test]
fn the_grid_is_clipped_to_the_path_and_takes_the_object_opacity() {
    let f = freeform();
    let r = render(&doc(shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)), paint(Some(f.clone())), 0.5));
    assert_eq!(r.pixel(2, 2), [255, 255, 255, 255], "outside the ellipse");
    let want = expected(&f, Point::new(50.5, 50.5), 0.5);
    assert!(near(r.pixel(50, 50), want, 6), "{:?} vs {want:?}", r.pixel(50, 50));
}

#[test]
fn unplaced_freeform_gradients_seed_points_on_the_box() {
    // Without points: the automatic ones, coloured white → black along the default stops.
    let r = render(&doc(shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), paint(None), 1.0));
    let auto = Freeform::auto(Rect::new(0.0, 0.0, 100.0, 100.0), &Gradient::default(), &|_| true);
    let at = |i: usize| auto.points[i].at;
    // (Pixel centres sit half a pixel off the points.)
    assert!(near(r.pixel(at(0).x as u32, at(0).y as u32), [255, 255, 255, 255], 3));
    assert!(near(r.pixel(at(3).x as u32, at(3).y as u32), [0, 0, 0, 255], 3));
}

#[test]
fn strokes_take_freeform_paint() {
    let mut d = Document::new(100.0, 100.0);
    let id = d.alloc_id();
    let line = shapes::rectangle(Rect::new(20.0, 20.0, 80.0, 80.0));
    let n = Node::path(id, line, Appearance::basic(Paint::None, paint(Some(freeform())), 10.0));
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    let r = render(&d);
    assert_eq!(r.pixel(20, 20), [255, 0, 0, 255], "the stroke's corner sits on the red point");
    assert_eq!(r.pixel(50, 50), [255, 255, 255, 255], "unfilled inside");
}
