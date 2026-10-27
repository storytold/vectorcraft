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
