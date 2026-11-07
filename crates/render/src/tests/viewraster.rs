//! Effect rasters cropped to the view, or made at a lower resolution, are kept while the view
//! stays as it is: dragging other art over an object whose effects reach far past the view (a wide
//! blur, or any blur zoomed in) draws its raster from the last frame. A cropped raster made again
//! after a pan is kept while later pans stay within the margin it is made with.

use serde_json::json;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, ImageBlob, ImageObject, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Rect, shapes};

use crate::{RenderOptions, Renderer};

/// An A4 page filled by a blue rectangle under `effects`, and a small orange square over it
/// (returned with the document): the square is the art a drag moves.
fn backdrop(effects: &[(&str, serde_json::Value)]) -> (Document, NodeId) {
    let mut d = Document::new(595.0, 842.0);
    let layer = d.layers[0].id;
    let mut back = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 595.0, 842.0)),
        Appearance::basic(Paint::solid(Color::rgb(0.1, 0.3, 0.9)), Paint::None, 0.0),
    );
    back.appearance.effects = effects.iter().map(|(id, p)| Effect { id: (*id).into(), params: p.clone(), visible: true }).collect();
    d.insert(Some(layer), 0, back).unwrap();
    let id = d.alloc_id();
    let square = Node::path(
        id,
        shapes::rectangle(Rect::new(280.0, 400.0, 300.0, 420.0)),
        Appearance::basic(Paint::solid(Color::rgb(0.9, 0.4, 0.1)), Paint::None, 0.0),
    );
    d.insert(Some(layer), 1, square).unwrap();
    (d, id)
}

/// A `w` × `h` view at `zoom`, centered on the page.
fn view(zoom: f64, (w, h): (u32, u32)) -> Affine {
    Affine::translate((f64::from(w) / 2.0, f64::from(h) / 2.0)) * Affine::scale(zoom) * Affine::translate((-297.5, -421.0))
}

/// `base` with the square moved by `step` steps of a drag.
fn moved(base: &Document, id: NodeId, step: u32, zoom: f64) -> Document {
    let mut d = base.clone();
    let by = (f64::from(step) * 3.0 / zoom, f64::from(step) * 2.0 / zoom);
    d.node_mut(id).unwrap().transform(Affine::translate(by), false);
    d
}

/// A case's name, its effects (id and params) and its zoom.
type Case = (&'static str, Vec<(&'static str, serde_json::Value)>, f64);

/// Every effect whose raster can be cropped to the view or made coarser, with a zoom at which it
/// is: wide blurs at the fit zoom, narrow ones zoomed in, a Photoshop-style filter, a shadow and a
/// glow.
fn cases() -> Vec<Case> {
    vec![
        ("Gaussian Blur 500 pt", vec![("blur.gaussian", json!({"radius": 500}))], 1.6),
        ("Gaussian Blur 100 pt", vec![("blur.gaussian", json!({"radius": 100}))], 1.6),
        ("Gaussian Blur 20 pt", vec![("blur.gaussian", json!({"radius": 20}))], 4.0),
        ("Gaussian Blur 8 pt", vec![("blur.gaussian", json!({"radius": 8}))], 16.0),
        ("Feather 20 pt", vec![("stylize.feather", json!({"radius": 20}))], 4.0),
        ("Craquelure", vec![("texture.craquelure", json!({}))], 4.0),
        ("Drop Shadow blur 100 pt", vec![("stylize.dropShadow", json!({"blur": 100, "x": 20, "y": 20}))], 1.6),
        ("Drop Shadow blur 10 pt", vec![("stylize.dropShadow", json!({"blur": 10, "x": 5, "y": 5}))], 8.0),
        ("Outer Glow blur 100 pt", vec![("stylize.outerGlow", json!({"blur": 100}))], 1.6),
    ]
}

/// A drag of the square over the backdrop draws the backdrop's effects from the rasters of the
/// frame before the drag, at full resolution: no frame of the drag previews them at a drag's lower
/// resolution, and each draws as a fresh renderer draws its still frame.
#[test]
fn a_drag_over_art_with_effects_past_the_view_draws_their_rasters_kept() {
    let size = (640, 360);
    for (name, effects, zoom) in cases() {
        let (base, id) = backdrop(&effects);
        let xf = view(zoom, size);
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let live = RenderOptions { interacting: true, dragging: true, ..opts.clone() };
        let mut r = Renderer::new();
        r.threads = 2;
        r.render(&base, size.0, size.1, xf, &opts);
        for step in 1..4 {
            let d = moved(&base, id, step, zoom);
            let drag = r.render(&d, size.0, size.1, xf, &live);
            assert_eq!(r.stats.previewed, 0, "{name}: drag frame {step} drew the backdrop's effects from the rasters kept");
            let mut fresh = Renderer::new();
            fresh.threads = 2;
            let still = fresh.render(&d, size.0, size.1, xf, &opts);
            assert!(drag.pixels == still.pixels, "{name}: drag frame {step} draws as a still frame");
        }
    }
}

/// A pan by whole pixels or a zoom makes the rasters cropped to the view again: the frame after it draws as a
/// fresh renderer draws it.
#[test]
fn rasters_cropped_to_the_view_follow_a_pan_and_a_zoom() {
    let size = (640, 360);
    for (name, effects, zoom) in cases() {
        let (d, _) = backdrop(&effects);
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut r = Renderer::new();
        r.threads = 2;
        r.render(&d, size.0, size.1, view(zoom, size), &opts);
        for xf in [Affine::translate((37.0, -21.0)) * view(zoom, size), view(zoom * 1.25, size)] {
            let image = r.render(&d, size.0, size.1, xf, &opts);
            let mut fresh = Renderer::new();
            fresh.threads = 2;
            assert!(image.pixels == fresh.render(&d, size.0, size.1, xf, &opts).pixels, "{name}");
        }
    }
}

/// A 6000 × 4000 pt image (300 × 200 pixels, 20 pt a pixel) under `effects`.
fn large_image(effects: &[(&str, serde_json::Value)]) -> Document {
    let img = image::RgbaImage::from_fn(300, 200, |x, y| image::Rgba([(x * 7) as u8, (y * 13) as u8, ((x + y) * 3) as u8, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut d = Document::new(6000.0, 4000.0);
    d.images.insert("viewraster-pan".into(), ImageBlob::new("image/png", png));
    let im = ImageObject {
        key: "viewraster-pan".into(),
        width: 300,
        height: 200,
        xf: Affine::scale(20.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.appearance.effects = effects.iter().map(|(id, p)| Effect { id: (*id).into(), params: p.clone(), visible: true }).collect();
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n).unwrap();
    d
}

/// At 100 %, the Drop Shadow, Gaussian Blur and Feather rasters of a large image are cropped to the view. The pan
/// that makes one again at the same zoom makes it with a margin around the view, and later pans within the margin
/// (by up to 259 pixels from the frame that made them, and back) draw it without making any offscreen raster: each
/// such frame makes as many offscreen pixels as the same view drawn again, and draws as a fresh renderer draws it,
/// within a level a channel: an image sampled into a raster that starts at another pixel rounds by up to a level, as it
/// does in the frames of two fresh renderers whose views differ in size.
#[test]
fn pans_within_a_cropped_rasters_margin_draw_it_kept() {
    let size = (640, 360);
    let cases: Vec<Case> = vec![
        ("Drop Shadow", vec![("stylize.dropShadow", json!({"blur": 20, "x": 7, "y": 7}))], 1.0),
        ("Gaussian Blur", vec![("blur.gaussian", json!({"radius": 20}))], 1.0),
        ("Feather", vec![("stylize.feather", json!({"radius": 20}))], 1.0),
        (
            "Drop Shadow over a Drop Shadow",
            vec![("stylize.dropShadow", json!({"blur": 20, "x": 7, "y": 7})), ("stylize.dropShadow", json!({"blur": 10, "x": -5, "y": 9}))],
            1.0,
        ),
    ];
    let counted = |r: &mut Renderer, d: &Document, xf: Affine, opts: &RenderOptions| {
        crate::OFFSCREEN_PIXELS.with(|n| n.set(0));
        let image = r.render(d, size.0, size.1, xf, opts);
        (image, crate::OFFSCREEN_PIXELS.with(|n| n.get()))
    };
    for (name, effects, zoom) in cases {
        let d = large_image(&effects);
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let at = |dx: f64, dy: f64| Affine::translate((320.0 + dx, 180.0 + dy)) * Affine::scale(zoom) * Affine::translate((-3000.0, -2000.0));
        let mut r = Renderer::new();
        r.threads = 2;
        r.render(&d, size.0, size.1, at(0.0, 0.0), &opts);
        // The pan that makes the rasters again, and the same view drawn again.
        r.render(&d, size.0, size.1, at(37.0, 0.0), &opts);
        let (_, kept) = counted(&mut r, &d, at(37.0, 0.0), &opts);
        for (dx, dy) in [(74.0, 0.0), (74.0, -37.0), (296.0, -37.0), (0.0, 0.0), (-222.0, 37.0)] {
            let (image, made) = counted(&mut r, &d, at(dx, dy), &opts);
            assert_eq!(made, kept, "{name}: the pan to ({dx}, {dy}) drew the rasters kept");
            let mut fresh = Renderer::new();
            fresh.threads = 2;
            let still = fresh.render(&d, size.0, size.1, at(dx, dy), &opts);
            let worst = image.pixels.iter().zip(&still.pixels).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            assert!(worst <= 1, "{name}: ({dx}, {dy}): a channel differs by {worst}");
        }
    }
}

/// Scrolls by amounts that are not whole pixels, as the wheel and a trackpad give them, moved in whole pixels with the part left
/// over carried to the next, as the canvas moves them: after the scroll that makes the Drop Shadow's cropped raster
/// again with its margin, every later scroll draws it kept. The same scrolls moved as given make it again in every
/// frame.
#[test]
fn fractional_scrolls_moved_in_whole_pixels_draw_a_cropped_raster_kept() {
    let size = (640, 360);
    let d = large_image(&[("stylize.dropShadow", json!({"blur": 20, "x": 7, "y": 7}))]);
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let at = |dx: f64, dy: f64| Affine::translate((320.0 + dx, 180.0 + dy)) * Affine::translate((-3000.0, -2000.0));
    let counted = |r: &mut Renderer, xf: Affine| {
        crate::OFFSCREEN_PIXELS.with(|n| n.set(0));
        r.render(&d, size.0, size.1, xf, &opts);
        crate::OFFSCREEN_PIXELS.with(|n| n.get())
    };
    let scrolls = [(0.37, 1.13), (-2.71, 0.0), (0.0, 0.45), (1.9, -0.83), (0.21, 0.21), (13.6, -4.4), (-0.5, 0.5), (7.77, 2.22)];
    for whole in [true, false] {
        let mut r = Renderer::new();
        r.threads = 2;
        r.render(&d, size.0, size.1, at(0.0, 0.0), &opts);
        // The scroll that makes the raster again with its margin, and the same view drawn again.
        r.render(&d, size.0, size.1, at(37.0, 0.0), &opts);
        let kept = counted(&mut r, at(37.0, 0.0));
        let (mut given, mut moved): ((f64, f64), (f64, f64)) = ((37.0, 0.0), (37.0, 0.0));
        let mut made_again = 0;
        for (k, (x, y)) in scrolls.into_iter().enumerate() {
            given = (given.0 + x, given.1 + y);
            moved = if whole { (given.0.round(), given.1.round()) } else { given };
            let made = counted(&mut r, at(moved.0, moved.1));
            if whole {
                assert_eq!(made, kept, "scroll {k} to {moved:?} drew the raster kept");
            } else if made > kept {
                made_again += 1;
            }
        }
        assert!((moved.0 - given.0).abs() <= 0.5 && (moved.1 - given.1).abs() <= 0.5, "moved {moved:?} for {given:?}");
        if !whole {
            assert_eq!(made_again, scrolls.len(), "scrolls by amounts that are not whole pixels make the raster again");
        }
    }
}

/// A group's shadow raster holds the shadows of the members outside the view: a pan that brings a member into view
/// draws its shadow from the raster kept as a fresh renderer draws it, for a raster of the group's whole reach (two
/// 100 pt squares) and for one cropped to the view and the margin a pan adds (two 3000 × 2000 pt rectangles).
#[test]
fn a_groups_shadow_raster_holds_the_members_outside_the_view() {
    let size = (640, 360);
    let blue = || Appearance::basic(Paint::solid(Color::rgb(0.1, 0.3, 0.9)), Paint::None, 0.0);
    // The two members, the point the view is centered on, and the pans after the first frame.
    let cases = [
        ("whole", [Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(600.0, 0.0, 700.0, 100.0)], (220.0, 50.0), vec![-300.0]),
        ("cropped", [Rect::new(0.0, 0.0, 3000.0, 2000.0), Rect::new(3400.0, 0.0, 6400.0, 2000.0)], (3000.0, 1000.0), vec![-37.0, -237.0]),
    ];
    let mut wrong = vec![];
    for (name, members, (cx, cy), pans) in cases {
        let mut d = Document::new(6400.0, 2000.0);
        let members = members.iter().map(|r| std::sync::Arc::new(Node::path(d.alloc_id(), shapes::rectangle(*r), blue()))).collect();
        let mut group = Node::group(d.alloc_id(), members);
        group.appearance.effects.push(Effect { id: "stylize.dropShadow".into(), params: json!({"blur": 20, "x": 7, "y": 7}), visible: true });
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, group).unwrap();
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let at = |dx: f64| Affine::translate((320.0 + dx, 180.0)) * Affine::translate((-cx, -cy));
        let mut r = Renderer::new();
        r.threads = 2;
        r.render(&d, size.0, size.1, at(0.0), &opts);
        for dx in pans {
            let image = r.render(&d, size.0, size.1, at(dx), &opts);
            let mut fresh = Renderer::new();
            fresh.threads = 2;
            if image.pixels != fresh.render(&d, size.0, size.1, at(dx), &opts).pixels {
                wrong.push(format!("{name}: panned by {dx}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// While interacting, a group with a member outside the view that only moved draws its shadow from the raster of the
/// frame before, where a fresh renderer draws it: the moved silhouette holds that member too.
#[test]
fn a_moved_group_with_a_member_outside_the_view_draws_its_shadow_kept() {
    let size = (640, 360);
    let blue = || Appearance::basic(Paint::solid(Color::rgb(0.1, 0.3, 0.9)), Paint::None, 0.0);
    let mut base = Document::new(700.0, 100.0);
    let members = [Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(600.0, 0.0, 700.0, 100.0)]
        .iter()
        .map(|r| std::sync::Arc::new(Node::path(base.alloc_id(), shapes::rectangle(*r), blue())))
        .collect();
    let mut group = Node::group(base.alloc_id(), members);
    group.appearance.effects.push(Effect { id: "stylize.dropShadow".into(), params: json!({"blur": 20, "x": 7, "y": 7}), visible: true });
    let (id, layer) = (group.id, base.layers[0].id);
    base.insert(Some(layer), 0, group).unwrap();
    let view = Affine::translate((100.0, 130.0));
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let live = RenderOptions { interacting: true, ..opts.clone() };
    let mut r = Renderer::new();
    r.threads = 2;
    r.render(&base, size.0, size.1, view, &opts);
    let mut moved = base.clone();
    moved.node_mut(id).unwrap().transform(Affine::translate((3.0, 2.0)), false);
    let frame = r.render(&moved, size.0, size.1, view, &live);
    assert!(r.stats.moved > 0, "drawn from the raster of the frame before");
    let mut fresh = Renderer::new();
    fresh.threads = 2;
    assert!(frame.pixels == fresh.render(&moved, size.0, size.1, view, &opts).pixels);
}

/// A shadow made over an earlier cropped shadow covers only the rect the earlier one was made for. In a 2000 × 1200
/// view of the image's right edge, the pixel budget gives the earlier shadow, which also covers what the later one
/// reads 60 pixels below, a margin of 548 pixels and the later one 643: a pan of 600 pixels past the first makes both
/// again, and the frame draws as a fresh renderer draws it.
#[test]
fn a_shadow_over_a_cropped_shadow_keeps_to_the_earlier_ones_margin() {
    let size = (2000, 1200);
    let effects = [("stylize.dropShadow", json!({"blur": 20, "x": 7, "y": 7})), ("stylize.dropShadow", json!({"blur": 10, "x": -5, "y": -60}))];
    let d = large_image(&effects);
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let at = |dy: f64| Affine::translate((1000.0, 600.0 + dy)) * Affine::translate((-6000.0, -2000.0));
    let mut r = Renderer::new();
    r.threads = 2;
    r.render(&d, size.0, size.1, at(0.0), &opts);
    r.render(&d, size.0, size.1, at(37.0), &opts);
    for dy in [37.0 - 600.0, 37.0 + 600.0] {
        let image = r.render(&d, size.0, size.1, at(dy), &opts);
        let mut fresh = Renderer::new();
        fresh.threads = 2;
        let still = fresh.render(&d, size.0, size.1, at(dy), &opts);
        let worst = image.pixels.iter().zip(&still.pixels).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert_eq!(worst, 0, "panned to {dy}");
    }
}

/// Time a drag of the square over each backdrop in a 2560 × 1440 view: the first frame renders as
/// before a drag, the next eight as a drag's frames (`interacting` and `dragging`, as the canvas
/// renders them), then the frame after the drag. The hashes of that frame and of a fresh renderer's
/// frame compare builds.
/// Run with `cargo test --release -p vectorcraft-render backdrop_drag_perf -- --ignored --nocapture`.
#[test]
#[ignore = "manual timing probe; wall-clock performance depends on machine load"]
fn backdrop_drag_perf() {
    let hash = |px: &[u8]| {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        px.hash(&mut h);
        h.finish()
    };
    let size = (2560, 1440);
    for (name, effects, zoom) in cases() {
        let (base, id) = backdrop(&effects);
        let xf = view(zoom, size);
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let live = RenderOptions { interacting: true, dragging: true, ..opts.clone() };
        let mut r = Renderer::new();
        r.threads = crate::default_threads();
        let start = std::time::Instant::now();
        std::hint::black_box(r.render(&base, size.0, size.1, xf, &opts));
        let first = start.elapsed().as_secs_f64() * 1000.0;
        let (mut ms, mut previewed, mut doc) = (vec![], 0, base.clone());
        for step in 1..9 {
            doc = moved(&base, id, step, zoom);
            let start = std::time::Instant::now();
            std::hint::black_box(r.render(&doc, size.0, size.1, xf, &live));
            ms.push(start.elapsed().as_secs_f64() * 1000.0);
            previewed += r.stats.previewed;
        }
        let start = std::time::Instant::now();
        let still = r.render(&doc, size.0, size.1, xf, &opts);
        let after = start.elapsed().as_secs_f64() * 1000.0;
        let fresh = Renderer::new().render(&doc, size.0, size.1, xf, &opts);
        ms.sort_by(f64::total_cmp);
        println!(
            "{name} at {:.0}%: first frame {first:.1} ms; drag frames median {:.1} ms (min {:.1}, max {:.1}), {previewed} previewed; the frame after the drag {after:.1} ms; hashes {:016x} {:016x}",
            zoom * 100.0,
            (ms[3] + ms[4]) * 0.5,
            ms[0],
            ms[7],
            hash(&still.pixels),
            hash(&fresh.pixels)
        );
    }
}
