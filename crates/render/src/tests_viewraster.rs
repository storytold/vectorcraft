//! Effect rasters cropped to the view, or made at a lower resolution, are kept while the view
//! stays as it is: dragging other art over an object whose effects reach far past the view (a wide
//! blur, or any blur zoomed in) draws its raster from the last frame.

use serde_json::json;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node, NodeId};
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
