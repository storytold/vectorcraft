//! Drawing placed documents ([`PlacedDocument`]).
//!
//! Exactly: the art read from the file ([`vectorcraft_doc::placed_document::read`]) drawn with its
//! own resources, through a view that maps its artboard into the object's box (nothing is copied
//! into the document).
//!
//! On screen ([`RenderOptions::progressive_placed`]): a placed document can't be edited, so what
//! it shows is cached as a bitmap per file and zoom step (a power of two, in device pixels per
//! point), shared by every object showing that file. Images inside it whose pixels can't be
//! decoded are drawn as crossed boxes in the bitmap when the frame marks them
//! ([`RenderOptions::mark_undecoded_images`]), and the bitmaps with and without marks are kept
//! apart. A bitmap not made yet is made on a
//! background worker while the frame shows another step's bitmap, or a light placeholder;
//! [`generation`] changes when one is ready, [`made_between`] says for which files, and [`busy`]
//! says whether any is coming, so the screen can draw again where those files show
//! ([`crate::placed_area`]). Bitmaps drawn since a frame began are not dropped to make room for
//! the ones it asks for, up to [`MAX_BYTES`], and a bitmap that can't be made is not asked for
//! again. Past [`MAX_SIDE`] pixels, past [`MAX_BYTES`], and in outline, proofing and ink frames,
//! it is drawn exactly. An outline frame draws only its box, crossed from corner to corner, unless
//! Document Setup → Show Images in Outline Mode is on.
//!
//! A document opened with only its placed documents' previews ([`ImageBlob::is_proxy`]) shows them
//! up to their resolution; past it the file is read again in the background
//! ([`vectorcraft_doc::placed_document::full_bytes`]), the preview standing in until then (and for
//! good when the file can't be found). Placed documents inside a placed document are drawn the
//! same way, up to [`MAX_DEPTH`] deep; deeper ones show their previews.

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, PoisonError};

use vectorcraft_doc::placed_document::MAX_DEPTH;
use vectorcraft_doc::{ImageBlob, Node, PlacedDocument};
use vectorcraft_geom::{Affine, Rect};
use vello_cpu::{Pixmap, RenderContext, kurbo, peniko};

use crate::ink::Ink;
use crate::{Frame, Renderer};

/// The longest side of a cached bitmap (pixels); larger steps draw exactly.
pub const MAX_SIDE: f64 = 4096.0;
/// The bytes of bitmaps kept, past which those not drawn since the frame that asks for another
/// one began are dropped.
const BUDGET: usize = 512 << 20;
/// The most bytes of bitmaps kept, those drawn in the frame included: a bitmap that would take
/// them past it is not kept, and its file draws exactly at that zoom step.
const MAX_BYTES: usize = 4 * BUDGET;
/// The smallest zoom step (device pixels per point).
const MIN_STEP: f64 = 1.0 / 64.0;

/// A bitmap's identity: the file shown (its key), the zoom step's bits, and whether images whose
/// pixels can't be decoded are marked ([`crate::RenderOptions::mark_undecoded_images`]).
type BitmapKey = (String, u64, bool);

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
/// Signaled, under [`PENDING`]'s lock, when the last bitmap being made stops being pending.
static IDLE: Condvar = Condvar::new();
/// Bitmaps that could not be made (neither the file nor a preview could be read): not asked for
/// again.
static FAILED: LazyLock<Mutex<HashSet<BitmapKey>>> = LazyLock::new(Default::default);
/// Bitmaps made but not kept, past [`MAX_BYTES`]: their files draw exactly at those zoom steps.
static UNKEPT: LazyLock<Mutex<HashSet<BitmapKey>>> = LazyLock::new(Default::default);
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// The files of the last [`MADE_KEPT`] bitmaps made, each with the [`generation`] it gave.
static MADE: LazyLock<Mutex<VecDeque<(u64, String)>>> = LazyLock::new(Default::default);
/// How many bitmaps [`MADE`] keeps.
const MADE_KEPT: usize = 256;
/// How many bitmaps of each file were asked for (tests).
#[cfg(test)]
static SCHEDULED: LazyLock<Mutex<HashMap<String, usize>>> = LazyLock::new(Default::default);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // The caches stay whole if a holder panicked (entries are only inserted and removed).
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Changes whenever a bitmap made in the background is ready: a screen showing placed documents
/// draws again when it does, where [`made_between`] says.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// The files whose bitmaps were made after [`generation`] `from`, up to `to`: a screen drawn at
/// `from` draws again where they show ([`crate::placed_area`]). None when they are no longer
/// known (more than [`MADE_KEPT`] bitmaps were made since).
pub fn made_between(from: u64, to: u64) -> Option<HashSet<String>> {
    let made = lock(&MADE);
    let oldest = made.front().map_or(u64::MAX, |(g, _)| *g);
    if to > from && oldest > from.saturating_add(1) {
        return None;
    }
    Some(made.iter().filter(|(g, _)| *g > from && *g <= to).map(|(_, key)| key.clone()).collect())
}

/// A bitmap of file `key` is ready: the generation changes, and [`MADE`] records which file
/// changed it. Both happen under [`MADE`]'s lock, so a generation read is in [`MADE`] by the time
/// [`made_between`] reads it.
fn made(key: &str) {
    let mut made = lock(&MADE);
    let g = GENERATION.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    made.push_back((g, key.to_owned()));
    while made.len() > MADE_KEPT {
        made.pop_front();
    }
}

/// Are bitmaps being made in the background?
pub fn busy() -> bool {
    !lock(&PENDING).is_empty()
}

/// Bitmap `key` stops being pending; when none is left, [`IDLE`] is signaled.
fn finish(key: &BitmapKey) {
    let mut pending = lock(&PENDING);
    pending.remove(key);
    if pending.is_empty() {
        IDLE.notify_all();
    }
}

/// Wait until no bitmap is being made ([`busy`] is false).
#[cfg(test)]
pub(crate) fn wait_idle() {
    let mut pending = lock(&PENDING);
    while !pending.is_empty() {
        pending = IDLE.wait(pending).unwrap_or_else(PoisonError::into_inner);
    }
}

/// Is a bitmap of file `key` being made?
#[cfg(test)]
pub(crate) fn pending_for(key: &str) -> bool {
    lock(&PENDING).iter().any(|k| k.0 == key)
}

/// How many bitmaps of file `key` were asked for.
#[cfg(test)]
pub(crate) fn scheduled(key: &str) -> usize {
    lock(&SCHEDULED).get(key).copied().unwrap_or(0)
}

/// Mark the bitmaps of file `key` at zoom `step`, with and without marks on undecoded images, as
/// not kept past [`MAX_BYTES`].
#[cfg(test)]
pub(crate) fn mark_unkept(key: &str, step: f64) {
    for mark in [false, true] {
        lock(&UNKEPT).insert((key.to_string(), step.to_bits(), mark));
    }
}

/// `object`'s file (`blob`) as a bitmap of `step` device pixels per point, made here and now.
#[cfg(test)]
pub(crate) fn bitmap(object: &PlacedDocument, blob: &ImageBlob, step: f64, mark_undecoded_images: bool) -> Option<Pixmap> {
    make(&mut Renderer::new(), object, blob, step, mark_undecoded_images)
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

    /// The bitmap of another step of the same file, with the same marks on undecoded images, the
    /// nearest to `step`.
    fn nearest(&self, key: &str, step: f64, mark: bool) -> Option<(Arc<Pixmap>, f64)> {
        self.map
            .iter()
            .filter(|((k, _, m), _)| k == key && *m == mark)
            .min_by(|a, b| (a.1.step / step).log2().abs().total_cmp(&(b.1.step / step).log2().abs()))
            .map(|(_, b)| (b.pm.clone(), b.step))
    }

    /// Keep `pm` as bitmap `k`, and past `budget` bytes drop the bitmaps drawn least recently,
    /// but not those drawn or made since clock `keep` (when the frame that asked for `k` began),
    /// which stay even past it. When the bitmaps would still be past `max` bytes, `k` is not kept:
    /// false.
    fn insert(&mut self, k: BitmapKey, pm: Pixmap, step: f64, keep: u64, (budget, max): (usize, usize)) -> bool {
        self.clock += 1;
        let size = pm.width() as usize * pm.height() as usize * 4;
        self.bytes += size;
        if let Some(old) = self.map.insert(k.clone(), Bitmap { pm: Arc::new(pm), step, used: self.clock }) {
            self.bytes = self.bytes.saturating_sub(old.pm.width() as usize * old.pm.height() as usize * 4);
        }
        while self.bytes > budget {
            let Some(k) = self.map.iter().filter(|(key, b)| b.used <= keep && **key != k).min_by_key(|(_, b)| b.used).map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(b) = self.map.remove(&k) {
                self.bytes = self.bytes.saturating_sub(b.pm.width() as usize * b.pm.height() as usize * 4);
            }
        }
        if self.bytes <= max {
            return true;
        }
        if self.map.remove(&k).is_some() {
            self.bytes = self.bytes.saturating_sub(size);
        }
        false
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
    /// The bitmap clock when the frame that asked for it began ([`Bitmaps::insert`]).
    keep: u64,
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

#[cfg(test)]
thread_local! {
    /// The deepest this thread has drawn inside placed documents since [`reset_deepest`].
    static DEEPEST: Cell<usize> = const { Cell::new(0) };
}

/// Start counting how deep this thread draws inside placed documents ([`deepest`]).
#[cfg(test)]
pub(crate) fn reset_deepest() {
    DEEPEST.with(|d| d.set(0));
}

/// The deepest this thread has drawn inside placed documents since [`reset_deepest`].
#[cfg(test)]
pub(crate) fn deepest() -> usize {
    DEEPEST.with(Cell::get)
}

/// While alive: drawing inside a placed document, one level deeper.
struct Inside;

impl Inside {
    fn enter() -> Self {
        DEPTH.with(|d| d.set(d.get() + 1));
        #[cfg(test)]
        DEEPEST.with(|d| d.set(d.get().max(depth())));
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

/// How a frame draws art ([`Renderer::layer_mode`]): whether Layer Options apply (on screen;
/// exports and thumbnails ignore them), whether the art draws in Outline mode, and whether Show
/// Images in Outline Mode is on for it ([`Renderer::shows_outline_images`]). Layers are measured
/// in the mode they draw in, and placed documents are their crossed box in Outline mode without
/// Show Images in Outline Mode ([`vectorcraft_doc::placed_document::Crossed`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LayerMode {
    pub(crate) options: bool,
    pub(crate) outline: bool,
    pub(crate) images: bool,
}

thread_local! {
    /// The mode of the frame being drawn or measured on this thread ([`LayerMode::enter`]).
    static MODE: Cell<Option<LayerMode>> = const { Cell::new(None) };
}

impl LayerMode {
    /// The mode layer `n` draws its art in when its Layer Options change it: a template layer stays
    /// in Preview in Outline mode (#1486), and a layer whose Preview is off draws in outline.
    /// `None` for other objects and for a layer that draws in this mode.
    pub(crate) fn inside(self, n: &Node) -> Option<Self> {
        let vectorcraft_doc::NodeKind::Layer { preview, template, .. } = &n.kind else { return None };
        if !self.options {
            return None;
        }
        let outline = if *template && self.outline {
            false
        } else if !preview && !template && !self.outline {
            true
        } else {
            return None;
        };
        Some(Self { outline, ..self })
    }

    /// The mode of the frame being drawn or measured on this thread, if any.
    pub(crate) fn now() -> Option<Self> {
        MODE.with(Cell::get)
    }

    /// Draw and measure in this mode on this thread until the guard returned is dropped.
    pub(crate) fn enter(self) -> InMode {
        let crossed = vectorcraft_doc::placed_document::Crossed::set(self.outline && !self.images);
        InMode { was: MODE.with(|m| m.replace(Some(self))), _crossed: crossed }
    }
}

/// While alive: art on this thread is drawn and measured in a [`LayerMode`]; once dropped, in the
/// mode before it.
#[must_use]
pub(crate) struct InMode {
    was: Option<LayerMode>,
    _crossed: vectorcraft_doc::placed_document::Crossed,
}

impl Drop for InMode {
    fn drop(&mut self) {
        MODE.with(|m| m.set(self.was));
    }
}

/// Measure layer `n` in the mode its Layer Options draw it in, inside the mode on this thread
/// ([`LayerMode::inside`]), until the guard returned is dropped; `None` when that is the same mode.
pub(crate) fn enter_layer(n: &Node) -> Option<InMode> {
    LayerMode::now()?.inside(n).map(LayerMode::enter)
}

/// Render `art` of a placed document's file (`doc`, its resources) into `w`×`h` transparent
/// pixels through `view`, the placed documents inside it one level deeper.
pub fn render_inside(r: &mut Renderer, doc: &vectorcraft_doc::Document, art: &Arc<Node>, w: u16, h: u16, view: Affine) -> Option<crate::Rendered> {
    render_inside_marked(r, doc, art, w, h, view, false)
}

/// [`render_inside`], drawing images whose pixels can't be decoded as crossed boxes when
/// `mark_undecoded_images` is set ([`crate::RenderOptions::mark_undecoded_images`]).
fn render_inside_marked(
    r: &mut Renderer,
    doc: &vectorcraft_doc::Document,
    art: &Arc<Node>,
    w: u16,
    h: u16,
    view: Affine,
    mark_undecoded_images: bool,
) -> Option<crate::Rendered> {
    let _inside = Inside::enter();
    let opts = crate::RenderOptions { mark_undecoded_images, ..Default::default() };
    r.render_node_with(doc, art, w, h, view, &opts)
}

/// `object`'s file (`blob`) as a bitmap of `step` device pixels per point of its box: from its
/// preview when that is all there is and is fine enough, else from the file (read again when
/// needed; the preview when it can't be). Images whose pixels can't be decoded are drawn as
/// crossed boxes when `mark_undecoded_images` is set.
fn make(r: &mut Renderer, object: &PlacedDocument, blob: &ImageBlob, step: f64, mark_undecoded_images: bool) -> Option<Pixmap> {
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
    let img = render_inside_marked(r, &e.doc, &e.art, w, h, view, mark_undecoded_images)?;
    let data = img.pixels.as_chunks::<4>().0.iter().map(|&[r, g, b, a]| vello_cpu::color::PremulRgba8 { r, g, b, a }).collect();
    Some(Pixmap::from_parts(data, w, h))
}

/// A bitmap side for `v` pixels (at least 1).
fn pixels(v: f64) -> u16 {
    v.round().clamp(1.0, MAX_SIDE) as u16
}

/// Make `job`'s bitmap and keep it (or remember that it was not kept), or remember that it can't
/// be made. The generation changes before the job stops being pending, so a screen that sees
/// nothing pending sees the change.
fn run(r: &mut Renderer, mut job: Job) {
    let scope = job.scope.take();
    let mut body = || {
        match make(r, &job.object, &job.blob, job.step, job.key.2) {
            Some(pm) => {
                if !lock(&BITMAPS).insert(job.key.clone(), pm, job.step, job.keep, (BUDGET, MAX_BYTES)) {
                    lock(&UNKEPT).insert(job.key.clone());
                }
                made(&job.key.0);
            }
            None => {
                lock(&FAILED).insert(job.key.clone());
            }
        }
        finish(&job.key);
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
                    // A panic loses this bitmap, not the worker; it is not asked for again.
                    let key = job.key.clone();
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| super::run(&mut r, *job))).is_err() {
                        super::lock(&super::FAILED).insert(key.clone());
                        super::finish(&key);
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
    #[cfg(test)]
    {
        *lock(&SCHEDULED).entry(job.key.0.clone()).or_default() += 1;
    }
    #[cfg(not(target_arch = "wasm32"))]
    let job = match workers::submit(Box::new(job)) {
        Ok(()) => return,
        Err(job) => *job,
    };
    run_here(job);
}

/// [`run`] `job` on this thread. When it panics, its bitmap failed (it is not asked for again)
/// and stops being pending, and the panic goes on to the caller.
fn run_here(job: Job) {
    let key = job.key.clone();
    if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut Renderer::new(), job))) {
        lock(&FAILED).insert(key.clone());
        finish(&key);
        std::panic::resume_unwind(panic);
    }
}

/// Make the bitmap of `object`'s file (`blob`) at zoom `step` on this thread, as [`schedule`]
/// does where no worker starts, inside `scope`.
#[cfg(test)]
pub(crate) fn make_here(object: &PlacedDocument, blob: ImageBlob, step: f64, scope: Scoped) {
    let key = (object.key.clone(), step.to_bits(), false);
    lock(&PENDING).insert(key.clone());
    run_here(Job { key, object: object.clone(), blob, step, keep: 0, scope: Some(scope) });
}

/// Could a bitmap of file `key` not be made?
#[cfg(test)]
pub(crate) fn failed_for(key: &str) -> bool {
    lock(&FAILED).iter().any(|k| k.0 == key)
}

/// The file `p` shows (its bytes `blob`) read as art, and kept for
/// [`vectorcraft_doc::placed_document::read_cached`]: from its bytes stored, read again already,
/// or (inside another placed document, while not too deep) read again now.
fn read_placed(p: &PlacedDocument, blob: &ImageBlob) -> Option<Arc<vectorcraft_doc::placed_document::ReadDocument>> {
    let full = match depth() {
        0 => vectorcraft_doc::placed_document::full_bytes_cached(p, blob),
        d if d < MAX_DEPTH => vectorcraft_doc::placed_document::full_bytes(p, blob),
        _ => None,
    };
    full.and_then(|bytes| vectorcraft_doc::placed_document::read(p, &bytes))
}

/// The mean scale of `a`.
fn mean_scale(a: Affine) -> f64 {
    a.determinant().abs().sqrt()
}

impl Renderer {
    /// Draw placed document `p` in frame `f`: from a cached bitmap on screen, else exactly; in
    /// Outline mode without Document Setup → Show Images in Outline Mode, its box crossed from
    /// corner to corner ([`Self::draw_crossed_box`]) and none of its art.
    pub(crate) fn draw_placed(&mut self, ctx: &mut RenderContext, f: &Frame, p: &PlacedDocument) {
        if self.crosses_placed(f) {
            return self.draw_crossed_placed(ctx, f, p);
        }
        let plain = f.ink == Ink::Display && !f.opts.outline && f.opts.proof.is_none() && !f.opts.overprint_preview;
        if f.opts.progressive_placed && plain && depth() == 0 && self.draw_placed_bitmap(ctx, f, p) {
            return;
        }
        let Some(blob) = f.doc.images.get(&p.key) else { return };
        let Some(e) = read_placed(p, blob) else {
            // Only the preview: drawn scaled into the box.
            if blob.is_proxy()
                && let Some(im) = p.preview_image(&blob.bytes)
            {
                self.draw_image(ctx, f, &im, None);
            }
            return;
        };
        let Some(m) = p.art_xf(e.frame) else { return };
        let k = mean_scale(m);
        if !k.is_finite() || k <= 1e-12 {
            return;
        }
        // The art in its own coordinates, with its own resources (its composite fonts included, and
        // none of the host document's).
        let _lent = vectorcraft_text::FontDb::global().lend_document_composite_fonts(&e.doc.composite_fonts);
        let symbols = e.doc.symbol_boxes().quick();
        let visible = m.inverse().transform_rect_bbox(f.visible);
        let inner = Frame { doc: &e.doc, symbols: &symbols, view: f.view * m, visible, px: f.px / k, output: f.output * m, ..*f };
        let _inside = Inside::enter();
        // Its art follows this document's Show Images in Outline Mode, not the file's.
        let host = self.host_outline_images.replace(self.shows_outline_images(f));
        self.draw_arc(ctx, &inner, &e.art);
        self.host_outline_images = host;
    }

    /// Draw placed document `p` as its box crossed from corner to corner ([`Self::crosses_placed`]).
    /// Its file is still read, as when its art is drawn: once the frame shows images again, the art
    /// its effects and its group's effects make comes from the read kept.
    pub(crate) fn draw_crossed_placed(&mut self, ctx: &mut RenderContext, f: &Frame, p: &PlacedDocument) {
        if let Some(blob) = f.doc.images.get(&p.key) {
            let _ = read_placed(p, blob);
        }
        self.draw_crossed_box(ctx, f, p.corners());
    }

    /// Whether `f` draws a placed document as its box crossed from corner to corner and none of
    /// its art: in Outline mode without Show Images in Outline Mode ([`Self::shows_outline_images`]).
    pub(crate) fn crosses_placed(&self, f: &Frame) -> bool {
        f.opts.outline && !self.shows_outline_images(f)
    }

    /// Draw `p` from its bitmap at this zoom step, or (while that is made, or when it can't be)
    /// another step's, or a placeholder. False when bitmaps don't suit (too large, or not kept
    /// past [`MAX_BYTES`]).
    fn draw_placed_bitmap(&mut self, ctx: &mut RenderContext, f: &Frame, p: &PlacedDocument) -> bool {
        let m = f.view * p.xf;
        let Some(step) = step(mean_scale(m)) else { return false };
        if p.width * step > MAX_SIDE || p.height * step > MAX_SIDE {
            return false;
        }
        let Some(blob) = f.doc.images.get(&p.key).cloned() else { return true };
        let key = (p.key.clone(), step.to_bits(), f.opts.mark_undecoded_images);
        // Not kept past [`MAX_BYTES`]: drawn exactly.
        if lock(&UNKEPT).contains(&key) {
            return false;
        }
        let (found, nearest, keep) = {
            let mut b = lock(&BITMAPS);
            // The clock when this frame first drew a placed document's bitmap.
            let keep = *self.placed_frame.get_or_insert(b.clock);
            let found = b.get(&key);
            let nearest = if found.is_none() { b.nearest(&p.key, step, f.opts.mark_undecoded_images) } else { None };
            (found, nearest, keep)
        };
        let shown = match found {
            Some(pm) => Some(pm),
            // It can't be made: another step's, or the placeholder.
            None if lock(&FAILED).contains(&key) => nearest.map(|n| n.0),
            None => {
                schedule(Job { key, object: p.clone(), blob, step, keep, scope: SCOPE_CAPTURE.get().map(|capture| capture()) });
                // Made here and now (no threads) or meanwhile another step's.
                let mut b = lock(&BITMAPS);
                b.get(&(p.key.clone(), step.to_bits(), f.opts.mark_undecoded_images)).or(nearest.map(|n| n.0))
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
                // Coming, or it can't be made: a light box where it shows.
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

    /// Past the budget, the bitmaps drawn least recently are dropped, but not those drawn since
    /// the frame that asked for the new one began; past the most, the new one is not kept.
    #[test]
    fn bitmaps_drawn_in_the_frame_stay_up_to_the_most_kept() {
        let mut b = Bitmaps::default();
        // 400 bytes each: a budget of 1,000 bytes and at most 2,000.
        let limits = (1000, 2000);
        let key = |name: &str| (name.to_string(), 1f64.to_bits(), false);
        for name in ["a", "b", "c"] {
            assert!(b.insert(key(name), Pixmap::new(10, 10), 1.0, 0, limits));
        }
        assert_eq!(b.bytes, 1200, "all made in the frame: kept past the budget");
        // The next frame begins and draws "a", then asks for "d".
        let frame = b.clock;
        assert!(b.get(&key("a")).is_some());
        assert!(b.insert(key("d"), Pixmap::new(10, 10), 1.0, frame, limits));
        let mut kept: Vec<&str> = b.map.keys().map(|(name, _, _)| name.as_str()).collect();
        kept.sort_unstable();
        assert_eq!((kept, b.bytes), (vec!["a", "d"], 800), "b and c, not drawn since, are dropped");
        for name in ["e", "f", "g"] {
            assert!(b.insert(key(name), Pixmap::new(10, 10), 1.0, frame, limits), "{name}");
        }
        assert_eq!(b.bytes, 2000);
        assert!(!b.insert(key("h"), Pixmap::new(10, 10), 1.0, frame, limits), "past the most: not kept");
        assert!(!b.map.contains_key(&key("h")) && b.bytes == 2000);
    }

    /// While a bitmap is made, another step's bitmap stands in only when it has the same marks on
    /// undecoded images.
    #[test]
    fn a_stand_in_bitmap_has_the_same_marks() {
        let mut b = Bitmaps::default();
        let limits = (BUDGET, MAX_BYTES);
        assert!(b.insert(("a".into(), 1f64.to_bits(), false), Pixmap::new(10, 10), 1.0, 0, limits));
        assert!(b.insert(("a".into(), 4f64.to_bits(), true), Pixmap::new(40, 40), 4.0, 0, limits));
        assert_eq!(b.nearest("a", 2.0, false).map(|n| n.1), Some(1.0));
        assert_eq!(b.nearest("a", 2.0, true).map(|n| n.1), Some(4.0));
        assert!(b.nearest("b", 2.0, false).is_none());
    }
}
