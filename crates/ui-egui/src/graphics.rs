//! Losing the graphics device, and the textures uploaded to it (#369).
//!
//! A window can lose its graphics device: a driver reset or a GPU hang on the desktop, the browser
//! dropping the page's WebGPU device or WebGL context. Everything uploaded to it is gone and
//! nothing it draws reaches the screen any more, while the app itself goes on.
//!
//! - The host reports the loss through a [`GraphicsLoss`] (from wgpu's device-lost callback, the
//!   canvas's `webglcontextlost` event), and the app keeps recovery copies of the modified
//!   documents at once ([`VectorcraftApp::graphics_lost`]).
//! - The web host then starts again with new graphics and a new egui context, handing it the same
//!   app. The app notices the new context and makes everything it uploaded to the old one again:
//!   its fonts and theme, the canvas, and every texture cache ([`TexCache`]).

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::VectorcraftApp;

/// Raised by the host when the window's graphics device is lost (from any thread), read on the UI
/// thread. Cheap to clone; the clones share the report.
#[derive(Clone, Default)]
pub struct GraphicsLoss(Arc<Mutex<Option<String>>>);

impl GraphicsLoss {
    /// The graphics device was lost (`why`, for the log): the next frame of `ctx` handles it. A
    /// second report before then adds nothing.
    pub fn report(&self, ctx: &egui::Context, why: impl Into<String>) {
        let why = why.into();
        log::error!("vectorcraft: the graphics device was lost: {why}");
        self.0.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(why);
        ctx.request_repaint();
    }

    /// The loss reported since the last call, if any.
    pub fn take(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// A number naming egui context `ctx`, the same every call and different from every other
/// context's.
pub(crate) fn context_tag(ctx: &egui::Context) -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = egui::Id::unique("vectorcraft-context-tag");
    if let Some(tag) = ctx.data(|d| d.get_temp::<u64>(id)) {
        return tag;
    }
    ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || NEXT.fetch_add(1, Ordering::Relaxed)))
}

thread_local! {
    /// Bumped when the UI moves to a new egui context: [`TexCache`]s made before are emptied.
    static EPOCH: Cell<u64> = const { Cell::new(0) };
}

/// The UI moved to a new egui context: every [`TexCache`] of this thread empties at its next use.
pub(crate) fn forget_textures() {
    EPOCH.with(|e| e.set(e.get().wrapping_add(1)));
}

/// A per-thread cache holding textures (thumbnails, previews, samples). Textures belong to the
/// egui context they were uploaded to, so the cache goes back to `T::default()` when the UI moves
/// to another one ([`forget_textures`]). Used like the `RefCell` it wraps.
#[derive(Default)]
pub(crate) struct TexCache<T> {
    epoch: Cell<u64>,
    value: RefCell<T>,
}

impl<T: Default> TexCache<T> {
    /// Empty the cache if it was filled for another context.
    fn refresh(&self) {
        let now = EPOCH.with(Cell::get);
        if self.epoch.replace(now) != now {
            *self.value.borrow_mut() = T::default();
        }
    }

    pub(crate) fn borrow(&self) -> Ref<'_, T> {
        self.refresh();
        self.value.borrow()
    }

    pub(crate) fn borrow_mut(&self) -> RefMut<'_, T> {
        self.refresh();
        self.value.borrow_mut()
    }
}

impl VectorcraftApp {
    /// The window's graphics device was lost (`why`, from [`GraphicsLoss::take`]): recovery copies
    /// of the modified documents are written now, since the window may never draw again (even
    /// with Data Recovery turned off: they go when the documents are saved or closed) → whether
    /// every modified document has an up-to-date copy (none modified counts).
    pub fn graphics_lost(&mut self, why: &str) -> bool {
        if let Err(e) = self.session.execute("file.recovery.save", &serde_json::json!({})) {
            log::warn!("vectorcraft: no recovery copies after losing the graphics ({why}): {e}");
        }
        self.session.documents().iter().filter(|st| st.is_dirty()).all(|st| st.recovery.as_ref().is_some_and(|c| st.same_content(&c.doc)))
    }

    /// Textures live in one egui context. When the host hands the app a new one (it started again
    /// after losing its graphics), everything uploaded to the old one is made again: the fonts and
    /// theme, the canvas, the place cursor's thumbnails and every [`TexCache`]. The host hands it
    /// the window's context when it creates the app, so the first frame's input has the app's
    /// commands.
    pub fn adopt_context(&mut self, ctx: &egui::Context) {
        let tag = context_tag(ctx);
        self.ui_ctx = Some(ctx.clone());
        if self.context == tag {
            return;
        }
        if self.context != 0 {
            log::info!("vectorcraft: new graphics context: uploading the UI again");
            forget_textures();
            // The IME tracker's pass numbers are per context. A new context starts at pass 0; only
            // an explicit teardown guarantees it cannot look continuous with the old context's
            // last observed pass and reuse its owner/baseline generation.
            self.ime_context.end_viewport(ctx.viewport_id());
            self.canvas.texture = None;
            self.canvas.key = None;
        }
        self.context = tag;
    }
}

thread_local! {
    /// The one renderer of the small renders below.
    static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
}

/// A small document rendered at one pixel to the point (stroke previews: brushes, arrowheads),
/// transparent where nothing paints. `build` makes the document of `size_px` pixels, as points.
pub(crate) fn doc_image(size_px: [usize; 2], build: impl FnOnce(f64, f64) -> Option<vectorcraft_doc::Document>) -> Option<egui::ColorImage> {
    let (w, h) = (size_px[0] as f64, size_px[1] as f64);
    let doc = build(w, h)?;
    let img = RENDERER.with(|r| r.borrow_mut().render_region(&doc, vectorcraft_geom::Rect::new(0.0, 0.0, w, h), 1.0, false));
    Some(crate::canvas::color_image(&img))
}

/// `region` of `doc` rendered on white, its longest side `px` pixels (artboard and page
/// thumbnails).
pub(crate) fn region_image(doc: &vectorcraft_doc::Document, region: vectorcraft_geom::Rect, px: u32) -> egui::ColorImage {
    let scale = f64::from(px) / region.width().max(region.height()).max(1.0);
    let img = RENDERER.with(|r| r.borrow_mut().render_region(doc, region, scale, true));
    crate::canvas::color_image(&img)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::cmd::recovery::{MemoryStore, RecoveryStore};

    use super::*;
    use crate::Services;

    /// One headless frame of the whole window (800×600) in `ctx`.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
        crate::testing::app_frame(app, ctx, raw);
    }

    /// The name of texture `id` in `ctx`, if `ctx` has it.
    fn texture_name(ctx: &egui::Context, id: egui::TextureId) -> Option<String> {
        ctx.tex_manager().read().meta(id).map(|m| m.name.clone())
    }

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
        app
    }

    #[test]
    fn a_loss_is_reported_once_from_any_thread() {
        let (loss, ctx) = (GraphicsLoss::default(), crate::testing::ui_context());
        let (l, c) = (loss.clone(), ctx.clone());
        std::thread::spawn(move || {
            l.report(&c, "Unknown: driver reset");
            l.report(&c, "Destroyed");
        })
        .join()
        .unwrap();
        assert_eq!(loss.take().as_deref(), Some("Unknown: driver reset"), "the first report wins");
        assert_eq!(loss.take(), None, "handled once");
    }

    #[test]
    fn a_tex_cache_empties_when_the_ui_moves_to_a_new_context() {
        thread_local! {
            static CACHE: TexCache<Vec<u32>> = TexCache::default();
        }
        CACHE.with(|c| c.borrow_mut().push(7));
        assert_eq!(CACHE.with(|c| c.borrow().clone()), [7]);
        forget_textures();
        assert!(CACHE.with(|c| c.borrow().is_empty()), "emptied for the new context");
        CACHE.with(|c| c.borrow_mut().push(8));
        assert_eq!(CACHE.with(|c| c.borrow().clone()), [8], "and filled again");
    }

    /// After the graphics are lost, the app uploads its canvas to the new context.
    #[test]
    fn the_app_uploads_everything_again_in_a_new_context() {
        let mut app = app();
        app.canvas.worker_started = true; // render on this thread
        let old = crate::testing::ui_context();
        for _ in 0..3 {
            frame(&mut app, &old);
        }
        let canvas = app.canvas.texture.as_ref().map(egui::TextureHandle::id).expect("the canvas is drawn");
        assert_eq!(texture_name(&old, canvas).as_deref(), Some("canvas"));

        // The host lost its graphics and started again: same app, new context.
        let new = crate::testing::ui_context();
        for _ in 0..3 {
            frame(&mut app, &new);
        }
        let canvas = app.canvas.texture.as_ref().map(egui::TextureHandle::id).expect("the canvas is drawn again");
        assert_eq!(texture_name(&new, canvas).as_deref(), Some("canvas"), "uploaded to the new context");
    }

    #[test]
    fn losing_the_graphics_keeps_recovery_copies_at_once() {
        let store = Arc::new(MemoryStore::default());
        let mut app = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store.clone()), ..Default::default() });
        assert!(app.graphics_lost("test"), "nothing modified: nothing to lose");
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
        app.session.prefs.autosave_recovery = false;
        assert!(app.graphics_lost("test"), "copied even with Data Recovery off");
        assert_eq!(store.list().unwrap().iter().filter(|n| n.ends_with(".vectorcraft")).count(), 1);
        // Without a store the work isn't safe.
        assert!(!self::app().graphics_lost("test"));
    }
}
