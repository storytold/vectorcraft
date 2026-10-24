//! Drawing placed documents ([`PlacedDocument`]).
//!
//! Exactly: the art read from the file ([`vectorcraft_doc::placed_document::read`]) drawn with its
//! own resources, through a view that maps its artboard into the object's box (nothing is copied
//! into the document).
//!
//! On screen ([`RenderOptions::progressive_placed`]): a placed document can't be edited, so what
//! it shows is cached as a bitmap per file and zoom step (a power of two, in device pixels per
//! point), shared by every object showing that file. A bitmap not made yet is made on a
//! background worker while the frame shows another step's bitmap, or a light placeholder;
//! [`generation`] changes when one is ready and [`busy`] says whether any is coming, so the screen
//! can draw again. Past [`MAX_SIDE`] pixels, and in outline, proofing and ink frames, it is drawn
//! exactly.
//!
//! A document opened with only its placed documents' previews ([`ImageBlob::is_proxy`]) shows them
//! up to their resolution; past it the file is read again in the background
//! ([`vectorcraft_doc::placed_document::full_bytes`]), the preview standing in until then (and for
//! good when the file can't be found). Placed documents inside a placed document are drawn the
//! same way, up to [`MAX_DEPTH`] deep; deeper ones show their previews.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use vectorcraft_doc::placed_document::MAX_DEPTH;
use vectorcraft_doc::{ImageBlob, Node, PlacedDocument};
use vectorcraft_geom::{Affine, Rect};
use vello_cpu::{Pixmap, RenderContext, kurbo, peniko};

use crate::ink::Ink;
use crate::{Frame, Renderer};

/// The longest side of a cached bitmap (pixels); larger steps draw exactly.
pub const MAX_SIDE: f64 = 4096.0;
/// The most bytes of bitmaps kept.
const BUDGET: usize = 512 << 20;
/// The smallest zoom step (device pixels per point).
const MIN_STEP: f64 = 1.0 / 64.0;

/// A bitmap's identity: the file shown (its key) and the zoom step's bits.
type BitmapKey = (String, u64);

struct Bitmap {
    pm: Arc<Pixmap>,
    step: f64,
    used: u64,
}

#[derive(Default)]
struct Bitmaps {
    map: HashMap<BitmapKey, Bitmap>,
    clock: u64,
    bytes: usize,
}

static BITMAPS: LazyLock<Mutex<Bitmaps>> = LazyLock::new(Default::default);
/// Bitmaps being made.
static PENDING: LazyLock<Mutex<HashSet<BitmapKey>>> = LazyLock::new(Default::default);
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // The caches stay whole if a holder panicked (entries are only inserted and removed).
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Changes whenever a bitmap made in the background is ready: a screen showing placed documents
/// draws again when it does.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Are bitmaps being made in the background?
pub fn busy() -> bool {
    !lock(&PENDING).is_empty()
}

/// Is a bitmap of file `key` being made?
#[cfg(test)]
pub(crate) fn pending_for(key: &str) -> bool {
    lock(&PENDING).iter().any(|k| k.0 == key)
}

/// The zoom step for `scale` device pixels per point: the power of two at or above it.
pub(crate) fn step(scale: f64) -> Option<f64> {
    (scale.is_finite() && scale > 0.0).then(|| 2f64.powf(scale.log2().ceil()).max(MIN_STEP))
}

impl Bitmaps {
    fn get(&mut self, k: &BitmapKey) -> Option<Arc<Pixmap>> {
        self.clock += 1;
        let clock = self.clock;
        let b = self.map.get_mut(k)?;
        b.used = clock;
        Some(b.pm.clone())
    }

    /// The bitmap of another step of the same file, the nearest to `step`.
    fn nearest(&self, key: &str, step: f64) -> Option<(Arc<Pixmap>, f64)> {
        self.map
            .iter()
            .filter(|((k, _), _)| k == key)
            .min_by(|a, b| (a.1.step / step).log2().abs().total_cmp(&(b.1.step / step).log2().abs()))
            .map(|(_, b)| (b.pm.clone(), b.step))
    }

    fn insert(&mut self, k: BitmapKey, pm: Pixmap, step: f64) {
        self.clock += 1;
        let size = pm.width() as usize * pm.height() as usize * 4;
        self.bytes += size;
        if let Some(old) = self.map.insert(k, Bitmap { pm: Arc::new(pm), step, used: self.clock }) {
            self.bytes = self.bytes.saturating_sub(old.pm.width() as usize * old.pm.height() as usize * 4);
        }
        while self.bytes > BUDGET && self.map.len() > 1 {
            let Some(k) = self.map.iter().min_by_key(|(_, b)| b.used).map(|(k, _)| k.clone()) else { break };
            if let Some(b) = self.map.remove(&k) {
                self.bytes = self.bytes.saturating_sub(b.pm.width() as usize * b.pm.height() as usize * 4);
            }
        }
    }
}

/// Runs a closure in the file-access scope of whoever scheduled a job (see [`set_scope_capture`]).
pub type Scoped = Box<dyn FnOnce(&mut dyn FnMut()) + Send>;

static SCOPE_CAPTURE: std::sync::OnceLock<fn() -> Scoped> = std::sync::OnceLock::new();

/// Register how the scope the caller reads files in is carried to the background workers, which
/// read placed files too (the first registration stays). The engine registers its automation roots.
pub fn set_scope_capture(f: fn() -> Scoped) {
    let _ = SCOPE_CAPTURE.set(f);
}

/// What a bitmap is made from.
struct Job {
    key: BitmapKey,
    object: PlacedDocument,
    blob: ImageBlob,
    step: f64,
    /// The scheduler's file-access scope, which the worker runs the job in.
    scope: Option<Scoped>,
}

/// Device pixels per point of `object`'s box its preview `bytes` have.
fn preview_density(object: &PlacedDocument, bytes: &[u8]) -> f64 {
    vectorcraft_doc::placed_document::image_size(bytes).map_or(0.0, |(w, _)| w as f64 / object.width.max(1e-9))
}

thread_local! {
    /// How deep inside placed documents this thread is drawing.
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// While alive: drawing inside a placed document, one level deeper.
struct Inside;

impl Inside {
    fn enter() -> Self {
        DEPTH.with(|d| d.set(d.get() + 1));
        Inside
    }
}

impl Drop for Inside {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

fn depth() -> usize {
    DEPTH.with(Cell::get)
}

/// Render `art` of a placed document's file (`doc`, its resources) into `w`×`h` transparent
/// pixels through `view`, the placed documents inside it one level deeper.
pub fn render_inside(r: &mut Renderer, doc: &vectorcraft_doc::Document, art: &Arc<Node>, w: u16, h: u16, view: Affine) -> Option<crate::Rendered> {
    let _inside = Inside::enter();
    r.render_node(doc, art, w, h, view)
}

/// `object`'s file (`blob`) as a bitmap of `step` device pixels per point of its box: from its
/// preview when that is all there is and is fine enough, else from the file (read again when
/// needed; the preview when it can't be).
fn make(r: &mut Renderer, object: &PlacedDocument, blob: &ImageBlob, step: f64) -> Option<Pixmap> {
    let (w, h) = (pixels(object.width * step), pixels(object.height * step));
    let preview = if blob.is_proxy() { Some(blob.bytes.clone()) } else { blob.proxy.clone() };
    let full = match blob.is_proxy() {
        true if step <= preview_density(object, &blob.bytes) * 1.01 => None,
        true => vectorcraft_doc::placed_document::full_bytes(object, blob),
        false => Some(blob.bytes.clone()),
    };
    let read = full.and_then(|bytes| vectorcraft_doc::placed_document::read(object, &bytes));
    let Some(e) = read else { return crate::paint::decode_pixmap_fit(&preview?, w, h) };
    let f = e.frame;
    let (sx, sy) = (w as f64 / f.width(), h as f64 / f.height());
    let view = Affine::scale_non_uniform(sx, sy) * Affine::translate((-f.x0, -f.y0));
    let img = render_inside(r, &e.doc, &e.art, w, h, view)?;
    let data = img.pixels.as_chunks::<4>().0.iter().map(|&[r, g, b, a]| vello_cpu::color::PremulRgba8 { r, g, b, a }).collect();
    Some(Pixmap::from_parts(data, w, h))
}

/// A bitmap side for `v` pixels (at least 1).
fn pixels(v: f64) -> u16 {
    v.round().clamp(1.0, MAX_SIDE) as u16
}

/// Make `job`'s bitmap and keep it.
fn run(r: &mut Renderer, mut job: Job) {
    let scope = job.scope.take();
    let mut body = || {
        if let Some(pm) = make(r, &job.object, &job.blob, job.step) {
            lock(&BITMAPS).insert(job.key.clone(), pm, job.step);
        }
        lock(&PENDING).remove(&job.key);
        GENERATION.fetch_add(1, Ordering::Relaxed);
    };
    match scope {
        Some(scoped) => scoped(&mut body),
        None => body(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod workers {
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Arc, LazyLock, Mutex};

    use super::Job;

    /// The queue the workers take jobs from; `None` when no worker could start.
    static QUEUE: LazyLock<Option<Mutex<Sender<Box<Job>>>>> = LazyLock::new(|| {
        let (tx, rx) = channel::<Box<Job>>();
        let rx = Arc::new(Mutex::new(rx));
        let n = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4));
        let mut started = 0;
        for i in 0..n {
            let rx = rx.clone();
            let spawned = std::thread::Builder::new().name(format!("vectorcraft-placed-{i}")).spawn(move || {
                let mut r = crate::Renderer::new();
                r.threads = 0;
                loop {
                    let job = super::lock(&rx).recv();
                    let Ok(job) = job else { break };
                    // A panic loses this bitmap, not the worker.
                    let key = job.key.clone();
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| super::run(&mut r, *job))).is_err() {
                        super::lock(&super::PENDING).remove(&key);
                    }
                }
            });
            started += usize::from(spawned.is_ok());
        }
        (started > 0).then(|| Mutex::new(tx))
    });

    /// Hand `job` to a worker; back when there is none.
    pub(super) fn submit(job: Box<Job>) -> Result<(), Box<Job>> {
        match QUEUE.as_ref() {
            Some(q) => super::lock(q).send(job).map_err(|e| e.0),
            None => Err(job),
        }
    }
}

/// Make `job`'s bitmap in the background (here and now where there are no threads).
fn schedule(job: Job) {
    if !lock(&PENDING).insert(job.key.clone()) {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    let job = match workers::submit(Box::new(job)) {
        Ok(()) => return,
        Err(job) => *job,
    };
    run(&mut Renderer::new(), job);
}

/// The mean scale of `a`.
fn mean_scale(a: Affine) -> f64 {
    a.determinant().abs().sqrt()
}

impl Renderer {
    /// Draw placed document `p` in frame `f`: from a cached bitmap on screen, else exactly.
    pub(crate) fn draw_placed(&mut self, ctx: &mut RenderContext, f: &Frame, p: &PlacedDocument) {
        let plain = f.ink == Ink::Display && !f.opts.outline && f.opts.proof.is_none() && !f.opts.overprint_preview;
        if f.opts.progressive_placed && plain && depth() == 0 && self.draw_placed_bitmap(ctx, f, p) {
            return;
        }
        let Some(blob) = f.doc.images.get(&p.key) else { return };
        // The file's bytes: stored, read again already, or (inside another placed document, while
        // not too deep) read again now.
        let full = match depth() {
            0 => vectorcraft_doc::placed_document::full_bytes_cached(p, blob),
            d if d < MAX_DEPTH => vectorcraft_doc::placed_document::full_bytes(p, blob),
            _ => None,
        };
        let read = full.and_then(|bytes| vectorcraft_doc::placed_document::read(p, &bytes));
        let Some(e) = read else {
            // Only the preview: drawn scaled into the box.
            if blob.is_proxy()
                && let Some(im) = p.preview_image(&blob.bytes)
            {
                self.draw_image(ctx, f, &im);
            }
            return;
        };
        let Some(m) = p.art_xf(e.frame) else { return };
        let k = mean_scale(m);
        if !k.is_finite() || k <= 1e-12 {
            return;
        }
        // The art in its own coordinates, with its own resources.
        let symbols = e.doc.symbol_boxes().quick();
        let visible = m.inverse().transform_rect_bbox(f.visible);
        let inner = Frame { doc: &e.doc, symbols: &symbols, view: f.view * m, visible, px: f.px / k, ..*f };
        let _inside = Inside::enter();
        self.draw_arc(ctx, &inner, &e.art);
    }

    /// Draw `p` from its bitmap at this zoom step, or (while that is made) another step's, or a
    /// placeholder. False when bitmaps don't suit (too large).
    fn draw_placed_bitmap(&mut self, ctx: &mut RenderContext, f: &Frame, p: &PlacedDocument) -> bool {
        let m = f.view * p.xf;
        let Some(step) = step(mean_scale(m)) else { return false };
        if p.width * step > MAX_SIDE || p.height * step > MAX_SIDE {
            return false;
        }
        let Some(blob) = f.doc.images.get(&p.key).cloned() else { return true };
        let key = (p.key.clone(), step.to_bits());
        let (found, nearest) = {
            let mut b = lock(&BITMAPS);
            let found = b.get(&key);
            let nearest = if found.is_none() { b.nearest(&p.key, step) } else { None };
            (found, nearest)
        };
        let shown = match found {
            Some(pm) => Some(pm),
            None => {
                schedule(Job { key, object: p.clone(), blob, step, scope: SCOPE_CAPTURE.get().map(|capture| capture()) });
                // Made here and now (no threads) or meanwhile another step's.
                let mut b = lock(&BITMAPS);
                b.get(&(p.key.clone(), step.to_bits())).or(nearest.map(|n| n.0))
            }
        };
        let rect = Rect::new(0.0, 0.0, p.width, p.height);
        match shown {
            Some(pm) => {
                let (sx, sy) = (p.width / pm.width().max(1) as f64, p.height / pm.height().max(1) as f64);
                ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler: peniko::ImageSampler::default() });
                ctx.set_transform(m);
                ctx.set_paint_transform(Affine::scale_non_uniform(sx, sy));
                ctx.fill_rect(&kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1));
                ctx.reset_paint_transform();
            }
            None => {
                // Coming: a light box where it will be.
                ctx.set_transform(m);
                ctx.set_paint(peniko::Color::from_rgba8(128, 128, 128, 40));
                ctx.fill_rect(&kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1));
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_are_powers_of_two_at_or_above_the_scale() {
        assert_eq!(step(1.0), Some(1.0));
        assert_eq!(step(1.2), Some(2.0));
        assert_eq!(step(0.3), Some(0.5));
        assert_eq!(step(5.0), Some(8.0));
        assert_eq!(step(1e-9), Some(MIN_STEP));
        assert_eq!(step(f64::NAN), None);
        assert_eq!(step(0.0), None);
    }
}
