//! Raster effects stay where they are, and look the same, at every zoom and pan (#755).

use serde_json::json;
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{Appearance, Effect, Node, NodeKind};
use vectorcraft_geom::{Point, shapes};

use super::*;

/// Type blurred by `radius` points (Gaussian Blur, or Feather when `feather`) over a rectangle
/// that overlays at half opacity; without the type when `radius` is `None`.
fn scene(radius: Option<f64>, feather: bool) -> Document {
    let mut d = Document::new(200.0, 100.0);
    let l = d.layers[0].id;
    let mut back = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 100.0)),
        Appearance::basic(Paint::solid(Color::rgb(0.2, 0.4, 0.9)), Paint::None, 0.0),
    );
    back.blend = BlendMode::Overlay;
    back.opacity = 0.5;
    d.insert(Some(l), 0, back).unwrap();
    if let Some(radius) = radius {
        let style = CharStyle { size: 36.0, ..CharStyle::default() };
        let mut t = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(30.0, 60.0), "Hello", style))));
        let id = if feather { "stylize.feather" } else { "blur.gaussian" };
        t.appearance.effects.push(Effect { id: id.into(), params: json!({"radius": radius}), visible: true });
        d.insert(Some(l), 1, t).unwrap();
    }
    d
}

/// `d` with its layer's art in one group: the overlay and the type grouped, as in #755's report.
fn grouped(mut d: Document) -> Document {
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let members = d.children_mut(Some(l)).map(std::mem::take).unwrap();
    d.insert(Some(l), 0, Node::new(id, NodeKind::Group { children: members, clip: false })).unwrap();
    d
}

/// The canvas's options: art on nothing, no artboards.
fn screen() -> RenderOptions {
    RenderOptions { background: None, artboards: false, ..Default::default() }
}

fn render(d: &Document, threads: u16, view: Affine, (w, h): (u32, u32)) -> Rendered {
    let mut r = Renderer::new();
    r.threads = threads;
    r.render(d, w, h, view, &screen())
}

/// Where the type's ink is on the document (its centroid), rendered `threads`-threaded through
/// `view` on a `w` × `h` canvas.
fn ink_centre(radius: f64, threads: u16, view: Affine, size: (u32, u32)) -> Point {
    let with = render(&scene(Some(radius), false), threads, view, size);
    let without = render(&scene(None, false), threads, view, size);
    let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
    for (i, (p, q)) in with.pixels.chunks(4).zip(without.pixels.chunks(4)).enumerate() {
        let d: f64 = (0..4).map(|c| f64::from(p[c].abs_diff(q[c]))).sum();
        let (x, y) = ((i % size.0 as usize) as f64 + 0.5, (i / size.0 as usize) as f64 + 0.5);
        sx += d * x;
        sy += d * y;
        sw += d;
    }
    assert!(sw > 0.0, "the type shows");
    view.inverse() * Point::new(sx / sw, sy / sw)
}

#[test]
fn blurred_type_stays_in_place_at_every_zoom_and_pan() {
    for (radius, threads) in [(3.0, 0), (3.0, 4), (12.0, 0), (12.0, 4)] {
        let base = ink_centre(radius, threads, Affine::IDENTITY, (200, 100));
        // The whole document in view, at other zooms and moved by parts of a pixel.
        for (zoom, pan) in
            [(1.0, (13.3, 7.6)), (2.0, (41.7, 20.2)), (3.3, (15.25, 9.5)), (0.5, (10.4, 5.1)), (6.0, (30.6, 25.3)), (1.37, (0.5, 0.25))]
        {
            let view = Affine::translate(pan) * Affine::scale(zoom);
            let size = ((200.0 * zoom + 2.0 * pan.0).ceil() as u32, (100.0 * zoom + 2.0 * pan.1).ceil() as u32);
            let at = ink_centre(radius, threads, view, size);
            assert!(
                (at.x - base.x).abs() < 1.0 && (at.y - base.y).abs() < 1.0,
                "radius {radius}, threads {threads}, zoom {zoom}, pan {pan:?}: {at:?} vs {base:?}"
            );
        }
    }
}

/// Scrolling moves the blurred type with the page and nothing else: two views a whole number of
/// pixels apart draw the same pixels where they overlap, whether the type fits on the canvas or
/// runs off it.
#[test]
fn scrolling_moves_blurred_type_without_changing_it() {
    for (feather, group) in [(false, false), (true, false), (false, true)] {
        let d = scene(Some(12.0), feather);
        let d = if group { grouped(d) } else { d };
        for threads in [0, 4] {
            for (zoom, (w, h)) in [(1.0, (240, 140)), (4.0, (500, 300)), (8.0, (400, 300))] {
                let at = |dx: f64, dy: f64| render(&d, threads, Affine::translate((-dx, -dy)) * Affine::scale(zoom), (w, h));
                let a = at(20.0, 10.0);
                for (sx, sy) in [(1u32, 0u32), (3, 2), (37, 23), (101, 57)] {
                    let b = at(20.0 + f64::from(sx), 10.0 + f64::from(sy));
                    let mut worst = 0;
                    for y in sy..h {
                        for x in sx..w {
                            let i = ((y * w + x) * 4) as usize;
                            let j = (((y - sy) * w + x - sx) * 4) as usize;
                            for c in 0..4 {
                                worst = worst.max(a.pixels[i + c].abs_diff(b.pixels[j + c]));
                            }
                        }
                    }
                    assert!(
                        worst <= 3,
                        "feather {feather}, grouped {group}, threads {threads}, zoom {zoom}, scrolled by ({sx}, {sy}): a pixel changed by {worst}"
                    );
                }
            }
        }
    }
}

/// Scrolling by parts of a pixel (a trackpad, a zoom about the pointer) moves heavily blurred type
/// by just that: its ink stays put on the page within a tenth of a pixel, at any zoom.
#[test]
fn blurred_type_doesnt_wobble_as_the_view_moves_by_parts_of_a_pixel() {
    for threads in [0, 4] {
        for zoom in [1.0, 4.0] {
            let at = |p: f64| {
                ink_centre(30.0, threads, Affine::translate((400.0 - 75.0 * zoom + p, 330.0 - 48.0 * zoom + p)) * Affine::scale(zoom), (800, 660))
            };
            let base = at(0.0);
            for k in 1..10 {
                let c = at(f64::from(k) * 0.1);
                // How far it moved on the screen, besides the view's own move.
                let (dx, dy) = ((c.x - base.x) * zoom, (c.y - base.y) * zoom);
                assert!(dx.abs() < 0.1 && dy.abs() < 0.1, "threads {threads}, zoom {zoom}, moved by {}: off by ({dx}, {dy}) px", f64::from(k) * 0.1);
            }
        }
    }
}
