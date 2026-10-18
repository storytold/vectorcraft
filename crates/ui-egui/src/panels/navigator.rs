//! Navigator panel: a thumbnail of the document (rendered with `Renderer::render_region_with`,
//! cached by document revision, off the UI thread for heavy documents and not during drags: see
//! [`crate::thumb_worker`]) with the red view box you drag to pan, zoom out/in buttons, a zoom slider
//! and a zoom field.

use std::sync::Arc;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;
use vectorcraft_geom::{Point, Rect as DRect};

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::thumb_worker::{Settle, ThumbWorker};
use crate::widgets::{self, menu_item};

/// What a navigator image shows: document `uid` at `revision`, `w`×`h` pixels, the artboards only.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ImageKey {
    uid: u64,
    revision: u64,
    w: u32,
    h: u32,
    artboard_only: bool,
}

#[derive(Default)]
struct Cache {
    /// Renders heavy documents' images (None: on this thread).
    worker: Option<ThumbWorker<(), (ImageKey, DRect)>>,
    worker_started: bool,
    /// The region shown for (uid, revision, artboard only): the art bounds walk the whole document.
    region: Option<((u64, u64, bool), DRect)>,
    /// The image shown: what it shows and the document rect it covers.
    shown: Option<(ImageKey, DRect)>,
    tex: Option<egui::TextureHandle>,
    /// The image the worker is making.
    pending: Option<ImageKey>,
    /// How long the last image took: past [`HEAVY_MS`] the next goes to the worker.
    last_ms: f64,
    settle: Settle,
    /// Images made (tests count them).
    renders: u64,
}

/// An image that took longer than this (ms) makes the next one render off the UI thread.
const HEAVY_MS: f64 = 8.0;

thread_local! {
    static CACHE: crate::graphics::TexCache<Cache> = crate::graphics::TexCache::default();
}

impl Cache {
    /// Upload image `r` showing `key` over `region`.
    fn upload(&mut self, ctx: &egui::Context, key: ImageKey, region: DRect, r: &vectorcraft_render::Rendered, ms: f64) {
        let img = egui::ColorImage::from_rgba_premultiplied([r.width as usize, r.height as usize], &r.pixels);
        match &mut self.tex {
            Some(tx) => tx.set(img, egui::TextureOptions::LINEAR),
            None => self.tex = Some(ctx.load_texture("navigator", img, egui::TextureOptions::LINEAR)),
        }
        self.shown = Some((key, region));
        self.last_ms = ms;
        self.renders += 1;
    }

    /// The region (artboards, and the art unless `artboard_only`, with a margin) to show
    /// `doc` (`uid` at `revision`) in. While `busy` (a drag) the last one of the document stays,
    /// rather than walking the whole document every frame.
    fn region(&mut self, doc: &vectorcraft_doc::Document, uid: u64, revision: u64, artboard_only: bool, busy: bool) -> DRect {
        let key = (uid, revision, artboard_only);
        if let Some((k, r)) = self.region
            && (k == key || (busy && k.0 == uid && k.2 == artboard_only))
        {
            return r;
        }
        let mut region = doc.artboards.iter().map(|a| a.rect).reduce(|a, b| a.union(b)).unwrap_or(DRect::new(0.0, 0.0, 612.0, 792.0));
        if !artboard_only && let Some(b) = doc.art_bounds() {
            region = region.union(b);
        }
        let region = region.inflate(region.width() * 0.04 + 4.0, region.height() * 0.04 + 4.0);
        self.region = Some((key, region));
        region
    }

    /// Bring the image up to `key` (`doc` over `region`): heavy documents render on the worker
    /// while the last image stays, and nothing renders during a drag (`busy`) or before the
    /// document has settled, unless there's no image of it yet.
    fn update(&mut self, ctx: &egui::Context, doc: &Arc<vectorcraft_doc::Document>, key: ImageKey, region: DRect, busy: bool) {
        if !self.worker_started {
            self.worker_started = true;
            self.worker = ThumbWorker::spawn(ctx, "navigator");
        }
        let done = match self.worker.as_ref().map(ThumbWorker::poll) {
            Some(Some(done)) => done,
            Some(None) => {
                // The thread is gone: render here from now on.
                (self.worker, self.pending) = (None, None);
                vec![]
            }
            None => vec![],
        };
        for d in done {
            if self.pending == Some(d.key.0) {
                self.pending = None;
            }
            if let Some(img) = d.img {
                self.upload(ctx, d.key.0, d.key.1, &img, d.ms);
            }
        }
        let shown = self.shown.map(|(k, _)| k);
        if shown == Some(key) || self.pending == Some(key) {
            return;
        }
        let have = self.tex.is_some() && shown.is_some_and(|k| k.uid == key.uid);
        let now = ctx.input(|i| i.time);
        if have && (busy || !self.settle.settled(ctx, key.uid, key.revision, now)) {
            return;
        }
        let s = (key.w as f64 / region.width()).min(key.h as f64 / region.height());
        // As exported (template layers and guides left out), transparent; images far smaller than
        // they are sample their mip levels.
        let opts = vectorcraft_render::RenderOptions { skip_templates: true, image_mips: true, ..Default::default() };
        let heavy = self.last_ms > HEAVY_MS && have;
        if heavy && let Some(w) = &self.worker {
            let (doc, opts) = (doc.clone(), opts.clone());
            if w.submit((), (key, region), Box::new(move |r| Some(r.render_region_with(&doc, region, s, &opts)))) {
                self.pending = Some(key);
                return;
            }
            self.worker = None;
        }
        let t0 = crate::now_ms();
        let img = NAV_RENDERER.with(|r| r.borrow_mut().render_region_with(doc, region, s, &opts));
        self.upload(ctx, key, region, &img, crate::now_ms() - t0);
    }
}

thread_local! {
    /// Renders light documents' images on the UI thread.
    static NAV_RENDERER: std::cell::RefCell<vectorcraft_render::Renderer> = std::cell::RefCell::new(vectorcraft_render::Renderer::new());
}

/// Map a document point into the thumbnail rect (region → rect, aspect preserved).
pub fn doc_to_thumb(region: DRect, thumb: Rect, p: Point) -> egui::Pos2 {
    let s = (thumb.width() as f64 / region.width()).min(thumb.height() as f64 / region.height());
    let ox = thumb.left() as f64 + (thumb.width() as f64 - region.width() * s) / 2.0;
    let oy = thumb.top() as f64 + (thumb.height() as f64 - region.height() * s) / 2.0;
    pos2((ox + (p.x - region.x0) * s) as f32, (oy + (p.y - region.y0) * s) as f32)
}

/// Inverse of [`doc_to_thumb`].
pub fn thumb_to_doc(region: DRect, thumb: Rect, p: egui::Pos2) -> Point {
    let s = (thumb.width() as f64 / region.width()).min(thumb.height() as f64 / region.height());
    let ox = thumb.left() as f64 + (thumb.width() as f64 - region.width() * s) / 2.0;
    let oy = thumb.top() as f64 + (thumb.height() as f64 - region.height() * s) / 2.0;
    Point::new(region.x0 + (p.x as f64 - ox) / s, region.y0 + (p.y as f64 - oy) / s)
}

/// The red view box in the thumbnail: the corners of the canvas `canvas` showing view `v`, mapped
/// into the document (turned with the view) and then into the thumbnail, cut to the thumbnail
/// grown by a point. Fewer than three points when the view is outside the thumbnail.
fn view_box(region: DRect, thumb: Rect, canvas: Rect, v: &crate::state::View) -> Vec<egui::Pos2> {
    let xf = crate::canvas::Xf::new(canvas, v);
    let mut pts: Vec<egui::Pos2> = [canvas.left_top(), canvas.right_top(), canvas.right_bottom(), canvas.left_bottom()]
        .iter()
        .map(|c| doc_to_thumb(region, thumb, xf.to_doc(*c)))
        .collect();
    let b = thumb.expand(1.0);
    // One edge of `b` at a time: how far a point is inside it (negative outside).
    let edges: [fn(egui::Pos2, Rect) -> f32; 4] = [|p, b| p.x - b.left(), |p, b| b.right() - p.x, |p, b| p.y - b.top(), |p, b| b.bottom() - p.y];
    for inside in edges {
        let Some(&last) = pts.last() else { break };
        let mut out = vec![];
        let mut prev = last;
        for &p in &pts {
            let (s0, s1) = (inside(prev, b), inside(p, b));
            if (s0 >= 0.0) != (s1 >= 0.0) {
                // Where the side from `prev` to `p` crosses the edge.
                out.push(prev + (p - prev) * (s0 / (s0 - s1)));
            }
            if s1 >= 0.0 {
                out.push(p);
            }
            prev = p;
        }
        pts = out;
    }
    pts
}

/// Slider position (0..1) ↔ zoom percent on Illustrator's log range 3.13 %–64 000 %.
pub fn zoom_to_slider(z: f64) -> f64 {
    ((z.clamp(3.13, 64000.0)).ln() - 3.13f64.ln()) / (64000f64.ln() - 3.13f64.ln())
}
pub fn slider_to_zoom(t: f64) -> f64 {
    (3.13f64.ln() + t.clamp(0.0, 1.0) * (64000f64.ln() - 3.13f64.ln())).exp()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        super::empty_state(ui, "map", tl!("No document"), tl!("Open a document to navigate it."));
        return;
    };
    let artboard_only: bool = pstate(ui.ctx(), "nav-ab-only");
    let doc = st.doc.clone();
    let (uid, revision) = (st.uid, st.revision);
    // A drag in progress: every pointer move makes a new revision. The image waits for its end.
    let busy = st.interaction.is_some() || app.session.tool_busy();
    let region = CACHE.with(|c| c.borrow_mut().region(&doc, uid, revision, artboard_only, busy));
    let w = ui.available_width();
    let h = (w * 0.72).clamp(100.0, 200.0);
    let (thumb, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click_and_drag());
    ui.painter().rect_filled(thumb, 0.0, t.pasteboard);
    let ppp = ui.ctx().pixels_per_point();
    let (pw, ph) = ((w * ppp) as u32, (h * ppp) as u32);
    let key = ImageKey { uid, revision, w: pw, h: ph, artboard_only };
    let (tex, tex_region) = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.update(ui.ctx(), &doc, key, region, busy);
        // Never another document's image.
        match c.shown {
            Some((k, r)) if k.uid == uid => (c.tex.clone(), r),
            _ => (None, region),
        }
    });
    // Artboards (white) under the art thumbnail, like the canvas.
    for a in &doc.artboards {
        let r = Rect::from_min_max(doc_to_thumb(region, thumb, a.rect.origin()), doc_to_thumb(region, thumb, Point::new(a.rect.x1, a.rect.y1)));
        ui.painter().rect_filled(r, 0.0, Color32::WHITE);
    }
    if let Some(tex) = tex {
        let tl = doc_to_thumb(region, thumb, tex_region.origin());
        let br = doc_to_thumb(region, thumb, Point::new(tex_region.x1, tex_region.y1));
        ui.painter().image(tex.id(), Rect::from_min_max(tl, br), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    // The view box: the canvas's corners in the document, turned with the view, cut to the thumbnail.
    let view = app.view().copied();
    if let (Some(v), Some(cr)) = (view, app.canvas_rect) {
        let box_points = view_box(region, thumb, cr, &v);
        if box_points.len() > 2 {
            ui.painter().with_clip_rect(thumb).add(egui::Shape::closed_line(box_points, Stroke::new(1.5, Color32::from_rgb(0xff, 0x30, 0x30))));
        }
        if (resp.dragged() || resp.clicked())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let c = thumb_to_doc(region, thumb, p);
            app.run("view.setZoom", json!({"zoom": v.zoom * 100.0, "center": [c.x, c.y]})).ok();
        }
    }
    ui.painter().rect_stroke(thumb, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
    // Zoom controls.
    let zoom = view.map(|v| v.zoom * 100.0).unwrap_or(100.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut z = None;
        if let Some(v) = widgets::plain_field(ui, "nav-zoom", zoom, "%", 2, 64.0) {
            z = Some(v);
        }
        if widgets::icon_button(ui, "dc-zoom-small", tl!("Zoom Out"), false, 22.0).clicked() {
            app.run("view.zoomOut", json!({})).ok();
        }
        let sw = (ui.available_width() - 30.0).max(40.0);
        let track = |_: f32| t.input_border;
        let shown = widgets::SliderValue::field(zoom, 3.13..=64000.0, "%", 2);
        if let (Some(v), phase) = widgets::color_slider_valued(ui, "nav-slider", tl!("Zoom"), Some(shown), zoom_to_slider(zoom) as f32, sw, &track)
            && phase != widgets::Live::Idle
        {
            z = Some(slider_to_zoom(v as f64));
        }
        if widgets::icon_button(ui, "dc-zoom-large", tl!("Zoom In"), false, 22.0).clicked() {
            app.run("view.zoomIn", json!({})).ok();
        }
        if let Some(z) = z {
            app.run("view.setZoom", json!({"zoom": z.clamp(3.13, 64000.0)})).ok();
        }
    });
}

pub fn menu(_app: &mut VectorcraftApp, ui: &mut Ui) {
    let ab: bool = pstate(ui.ctx(), "nav-ab-only");
    if menu_item(ui, tl!("View Artboard Only"), true, ab) {
        set_pstate(ui.ctx(), "nav-ab-only", !ab);
    }
    menu_item(ui, tl!("Panel Options…"), false, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Screen readers get the Zoom slider's percent as the zoom field beside it shows it, on the
    /// slider's 3.13% to 64000% range.
    #[test]
    fn screen_readers_get_the_zoom_the_field_shows() {
        use crate::widgets::accessibility_tests::{slider_value, tree};
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("view.setZoom", json!({"zoom": 250})).unwrap();
        let shown = tree(|ui| show(&mut app, ui));
        assert_eq!(slider_value(&shown, "Zoom"), (Some(250.0), Some(3.13), Some(64000.0), Some("250%".to_string())));
    }

    #[test]
    fn thumb_mapping_roundtrip() {
        let region = DRect::new(0.0, 0.0, 200.0, 100.0);
        let thumb = Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 100.0));
        let p = doc_to_thumb(region, thumb, Point::new(100.0, 50.0));
        assert!((p.x - 60.0).abs() < 1e-3 && (p.y - 60.0).abs() < 1e-3);
        let q = thumb_to_doc(region, thumb, p);
        assert!((q.x - 100.0).abs() < 1e-6 && (q.y - 50.0).abs() < 1e-6);
    }

    /// In a view turned 90° the red view box outlines what the canvas shows: the canvas's corners
    /// mapped into the document and then into the thumbnail.
    #[test]
    fn the_view_box_turns_with_the_view() {
        use vectorcraft_engine::Session;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0));
        app.canvas_rect = Some(canvas);
        app.run("view.setZoom", json!({"zoom": 400, "center": [200, 150]})).unwrap();
        app.run("view.rotate", json!({"angle": 90})).unwrap();
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui));
        out.textures_delta.clear();
        // The thumbnail is the rectangle filled with the pasteboard color.
        let pasteboard = Tokens::get(&ctx).pasteboard;
        let thumb = out
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == pasteboard => Some(r.rect),
                _ => None,
            })
            .expect("the thumbnail");
        let red = egui::epaint::ColorMode::Solid(Color32::from_rgb(0xff, 0x30, 0x30));
        let drawn: Vec<Vec<egui::Pos2>> = out
            .shapes
            .into_iter()
            .filter_map(|c| match c.shape {
                egui::Shape::Path(p) if p.closed && p.stroke.color == red => Some(p.points),
                _ => None,
            })
            .collect();
        let [points] = &drawn[..] else { panic!("one view box: {drawn:?}") };
        let region = CACHE.with(|c| c.borrow().region.unwrap().1);
        let xf = crate::canvas::Xf::new(canvas, app.view().unwrap());
        let want =
            [canvas.left_top(), canvas.right_top(), canvas.right_bottom(), canvas.left_bottom()].map(|c| doc_to_thumb(region, thumb, xf.to_doc(c)));
        assert_eq!(points.len(), 4, "{points:?}");
        assert!(points.iter().zip(&want).all(|(a, b)| a.distance(*b) < 0.5), "{points:?} is not {want:?}");
    }

    #[test]
    fn zoom_slider_roundtrip() {
        assert!((zoom_to_slider(3.13)).abs() < 1e-9);
        assert!((zoom_to_slider(64000.0) - 1.0).abs() < 1e-9);
        assert!((slider_to_zoom(zoom_to_slider(100.0)) - 100.0).abs() < 1e-6);
    }

    /// The navigator images made on this thread so far.
    fn renders() -> u64 {
        CACHE.with(|c| c.borrow().renders)
    }

    #[test]
    fn no_image_is_rendered_during_a_drag_and_one_is_after_it() {
        use vectorcraft_engine::Session;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        // One context, so its clock runs on (1/60 s a pass) and the document can settle.
        let ctx = egui::Context::default();
        let frame = |app: &mut VectorcraftApp| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
            out.textures_delta.clear();
        };
        frame(&mut app);
        let first = renders();
        assert!(first >= 1, "the first image renders at once");
        // A move drag: every preview is a new revision.
        app.session.begin_interaction("Move").unwrap();
        for i in 1..=20 {
            app.session.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, i, 0]})).unwrap();
            frame(&mut app);
        }
        assert_eq!(renders(), first, "nothing renders while the drag goes on");
        app.session.commit_interaction().unwrap();
        frame(&mut app);
        assert_eq!(renders(), first, "the document settles first");
        for _ in 0..12 {
            frame(&mut app);
        }
        assert_eq!(renders(), first + 1, "one image after the drag");
        let shown = CACHE.with(|c| c.borrow().shown.map(|(k, _)| k.revision));
        assert_eq!(shown, Some(app.session.active().unwrap().revision));
    }
}
