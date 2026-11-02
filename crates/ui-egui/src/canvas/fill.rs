use super::cursor::only_view_differs;
use super::{Affine, CacheKey, Color32, Point, Pos2, Shape, VectorcraftApp, Xf, color_image, pos2, vec2};

/// The fraction of the canvas's resolution, each way, that [`Fill`] is rendered at.
const FILL_SCALE: f64 = 0.25;
/// How many times the canvas's width and height [`Fill`] covers, around the view it is rendered
/// for. A pan or zoom-out that stays within it draws it again without rendering it.
const FILL_REACH: f64 = 2.0;

/// [`crate::CanvasCache::fill`]: a frame of the art from [`crate::CanvasCache::fill_worker`],
/// rendered for the canvas key `key` as [`fill_view`] gives, and its texture.
pub struct Fill {
    /// The document, revision and view it was rendered for.
    pub(super) key: CacheKey,
    /// Document → its pixels.
    pub(super) view: Affine,
    /// Its size in pixels.
    pub(super) size: (u32, u32),
    pub(super) tex: egui::TextureHandle,
}

/// The size in pixels and the view (document → its pixels) of the fill for the canvas key `key`:
/// [`FILL_SCALE`] of the canvas's resolution, covering [`FILL_REACH`] times the canvas's width and
/// height around the view of `key`.
pub(super) fn fill_view(key: &CacheKey) -> (u32, u32, Affine) {
    let fw = (f64::from(key.w) * FILL_REACH * FILL_SCALE).ceil().clamp(1.0, f64::from(u16::MAX)) as u32;
    let fh = (f64::from(key.h) * FILL_REACH * FILL_SCALE).ceil().clamp(1.0, f64::from(u16::MAX)) as u32;
    let ppp = if key.ppp > 0.0 { f64::from(key.ppp) } else { 1.0 };
    let view = Affine::translate((f64::from(fw) / 2.0, f64::from(fh) / 2.0))
        * Affine::rotate(key.rot.to_radians())
        * Affine::scale(key.zoom * ppp * FILL_SCALE)
        * Affine::translate((-key.cx, -key.cy));
    (fw, fh, view)
}

/// While the frame shown is drawn for a view it was not rendered for, and its key differs from
/// `key` in the view alone (the frame for `key` is on its way from the render worker), the parts
/// of the canvas `rect` it leaves uncovered show `doc` from [`crate::CanvasCache::fill`]. The fill
/// worker ([`crate::CanvasCache::fill_worker`]) renders it with the frame shown's options and these
/// changes: images draw only from pixels already decoded
/// ([`vectorcraft_render::RenderOptions::kept_images_only`]) and draw mips
/// ([`vectorcraft_render::RenderOptions::image_mips`]), raster effects render within a drag's
/// budget ([`vectorcraft_render::RenderOptions::dragging`]), and rasters cached for moved objects
/// are not reused ([`vectorcraft_render::RenderOptions::interacting`] off). This thread only copies a
/// finished fill into its texture and draws it. A fill is asked for again when the one shown or on
/// its way does not cover the canvas, when the view's zoom is more than twice or less than half
/// its own, and when the document, its revision or anything but the view changed; until it
/// arrives, the fill shown is drawn where it reaches. It is dropped once the canvas shows the frame
/// for `key`, and while the frame shown differs from `key` in more than the view.
pub(super) fn fill_uncovered(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    painter: &egui::Painter,
    key: &CacheKey,
    doc: &std::sync::Arc<vectorcraft_doc::Document>,
    rect: egui::Rect,
    xf: &Xf,
) {
    // Taken every frame, so a fill that is no longer needed does not stay queued.
    let done = app.canvas.fill_worker.as_mut().and_then(|w| w.poll());
    let (Some(k), Some((_, opts))) = (&app.canvas.key, &app.canvas.shown) else {
        app.canvas.fill = None;
        return;
    };
    // The fill takes the frame shown's options, which match `key`'s only when the two keys differ
    // in the view alone.
    if k == key || !only_view_differs(k, key) || app.canvas.texture.is_none() {
        app.canvas.fill = None;
        return;
    }
    let screen_to_doc = xf.affine().inverse();
    let corners =
        [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].map(|p| screen_to_doc * Point::new(p.x as f64, p.y as f64));
    let usable = |f: &CacheKey| only_view_differs(f, key) && (0.5..=2.0).contains(&(key.zoom / f.zoom));
    let covers = |f: &CacheKey| {
        let (w, h, view) = fill_view(f);
        usable(f)
            && corners.iter().all(|c| {
                let p = view * *c;
                (-0.5..=f64::from(w) + 0.5).contains(&p.x) && (-0.5..=f64::from(h) + 0.5).contains(&p.y)
            })
    };
    let opts = vectorcraft_render::RenderOptions { kept_images_only: true, image_mips: true, interacting: false, dragging: true, ..opts.clone() };
    let gaps = shown_frame(k, rect, xf).map_or_else(Vec::new, |shown| uncovered(shown, rect));
    if let Some(done) = done.filter(|d| usable(&d.key)) {
        let color = color_image(&done.img);
        let tex = match app.canvas.fill.take() {
            Some(mut f) => {
                f.tex.set(color, egui::TextureOptions::LINEAR);
                f.tex
            }
            None => ctx.load_texture("canvas-fill", color, egui::TextureOptions::LINEAR),
        };
        let (_, _, view) = fill_view(&done.key);
        app.canvas.fill = Some(Fill { key: done.key, view, size: (done.img.width, done.img.height), tex });
    }
    let Some(worker) = app.canvas.fill_worker.as_mut().filter(|_| !gaps.is_empty()) else { return };
    if !app.canvas.fill.as_ref().is_some_and(|f| covers(&f.key)) && !worker.pending.as_ref().is_some_and(covers) {
        let (w, h, view) = fill_view(key);
        worker.submit(crate::render_worker::Job { key: key.clone(), doc: doc.clone(), w, h, view, opts });
    }
    let Some(f) = app.canvas.fill.as_ref().filter(|f| usable(&f.key)) else { return };
    let to_uv = Affine::scale_non_uniform(1.0 / f64::from(f.size.0.max(1)), 1.0 / f64::from(f.size.1.max(1))) * f.view * screen_to_doc;
    let from_uv = to_uv.inverse();
    let mut mesh = egui::Mesh::with_texture(f.tex.id());
    for gap in gaps {
        // Only where the fill reaches: its pixels are 0..1 each way in uv.
        let uv: Vec<Point> = gap.iter().map(|p| to_uv * Point::new(p.x as f64, p.y as f64)).collect();
        let uv = clip_polygon(&clip_polygon(&uv, |p| p.x, 0.0, false), |p| p.x, 1.0, true);
        let uv = clip_polygon(&clip_polygon(&uv, |p| p.y, 0.0, false), |p| p.y, 1.0, true);
        if uv.len() < 3 {
            continue;
        }
        let first = mesh.vertices.len() as u32;
        for p in &uv {
            let pos = from_uv * *p;
            mesh.vertices.push(egui::epaint::Vertex {
                pos: pos2(pos.x as f32, pos.y as f32),
                uv: pos2(p.x as f32, p.y as f32),
                color: Color32::WHITE,
            });
        }
        for i in 1..uv.len() as u32 - 1 {
            mesh.indices.extend([first, first + i, first + i + 1]);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Where the canvas draws the frame shown (its key `k`) in the view `xf` of the canvas `rect`:
/// the unit square (the frame's texture coordinates) → screen points. None when the mapping is
/// degenerate.
fn shown_frame(k: &CacheKey, rect: egui::Rect, xf: &Xf) -> Option<Affine> {
    let to_screen = match k.pixel {
        Some([x0, y0, x1, y1]) => {
            xf.affine() * Affine::translate((x0 as f64, y0 as f64)) * Affine::scale_non_uniform((x1 - x0) as f64, (y1 - y0) as f64)
        }
        None => {
            let ppp = if k.ppp > 0.0 { k.ppp } else { 1.0 };
            let old_rect = egui::Rect::from_center_size(rect.center(), vec2(k.w as f32 / ppp, k.h as f32 / ppp));
            let old = Xf { rect: old_rect, zoom: k.zoom, center: Point::new(k.cx, k.cy), rot: k.rot.to_radians() };
            xf.affine()
                * old.affine().inverse()
                * Affine::translate((old_rect.min.x as f64, old_rect.min.y as f64))
                * Affine::scale_non_uniform(old_rect.width() as f64, old_rect.height() as f64)
        }
    };
    let det = to_screen.determinant();
    (det.is_finite() && det.abs() > 1e-12).then_some(to_screen)
}

/// The parts of `rect` outside the parallelogram `shown` maps the unit square to, as convex
/// polygons of screen points, leaving out those thinner than half a point (a frame whose pixel
/// size was rounded from the canvas's leaves such slivers along its edges).
fn uncovered(shown: Affine, rect: egui::Rect) -> Vec<Vec<Pos2>> {
    // In the unit square's coordinates the canvas is a convex polygon and the frame shown is
    // 0..1 each way: the parts left of it, right of it, and above and below it in between.
    let inv = shown.inverse();
    let canvas: Vec<Point> = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()]
        .iter()
        .map(|p| inv * Point::new(p.x as f64, p.y as f64))
        .collect();
    let left = clip_polygon(&canvas, |p| p.x, 0.0, true);
    let right = clip_polygon(&canvas, |p| p.x, 1.0, false);
    let between = clip_polygon(&clip_polygon(&canvas, |p| p.x, 0.0, false), |p| p.x, 1.0, true);
    let above = clip_polygon(&between, |p| p.y, 0.0, true);
    let below = clip_polygon(&between, |p| p.y, 1.0, false);
    [left, right, above, below]
        .into_iter()
        .map(|part| part.into_iter().map(|p| shown * p).map(|p| pos2(p.x as f32, p.y as f32)).collect::<Vec<_>>())
        .filter(|part| {
            // Twice the area over the perimeter: about the width of a thin strip.
            let perimeter: f32 = part.iter().zip(part.iter().cycle().skip(1)).map(|(a, b)| a.distance(*b)).sum();
            perimeter > 0.0 && 2.0 * polygon_area(part) / perimeter > 0.5
        })
        .collect()
}

/// The part of convex polygon `poly` where `along(p)` is below `at` (`below`) or above it.
fn clip_polygon(poly: &[Point], along: impl Fn(&Point) -> f64, at: f64, below: bool) -> Vec<Point> {
    let inside = |p: &Point| if below { along(p) <= at } else { along(p) >= at };
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (a, b) in poly.iter().zip(poly.iter().cycle().skip(1)) {
        if inside(a) {
            out.push(*a);
        }
        if inside(a) != inside(b) {
            let t = (at - along(a)) / (along(b) - along(a));
            out.push(a.lerp(*b, t));
        }
    }
    out
}

/// The area of polygon `poly` (shoelace formula).
pub(super) fn polygon_area(poly: &[Pos2]) -> f32 {
    poly.iter().zip(poly.iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f32>().abs() / 2.0
}
