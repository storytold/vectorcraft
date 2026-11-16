use std::collections::HashSet;
use vectorcraft_doc::{NodeKind, TextKind};
use vectorcraft_geom::PathData;

use super::{Arc, Color32, Document, HashMap, Node, NodeId, Settle, ThumbWorker, Ui, VectorcraftApp, Weak, vec2};
use crate::thumb_worker::Done;

/// Where row thumbnails come from: document `uid` at `revision`, and whether a drag is in
/// progress (the thumbnails shown then stay until it ends).
pub(crate) struct ThumbSource<'a> {
    pub uid: u64,
    pub revision: u64,
    pub doc: &'a Arc<Document>,
    pub busy: bool,
}

impl<'a> ThumbSource<'a> {
    /// The thumbnails of the document `st` holds (`doc`, its document).
    pub(crate) fn of(app: &VectorcraftApp, st: &vectorcraft_engine::DocState, doc: &'a Arc<Document>) -> Self {
        Self { uid: st.uid, revision: st.revision, doc, busy: st.interaction.is_some() || app.session.tool_busy() }
    }
}

/// Node `id` of `doc`, as the `Arc` its parent holds.
pub(crate) fn node_arc(doc: &Document, id: NodeId) -> Option<Arc<Node>> {
    let path = doc.index_path(id)?;
    let (first, rest) = path.split_first()?;
    let mut cur = doc.layers.get(*first)?;
    for &i in rest {
        cur = cur.children()?.get(i)?;
    }
    Some(cur.clone())
}

/// A row thumbnail's image.
struct ThumbEntry {
    /// The node it shows. Weak, so the cache doesn't keep old art alive, but its allocation stays
    /// and no other node can come to have its address.
    node: Weak<Node>,
    px: u32,
    /// None: the node draws nothing.
    tex: Option<egui::TextureHandle>,
    /// How long it took to render (ms).
    ms: f64,
    /// The pass it was last drawn in.
    used: u64,
}

/// A row thumbnail job: the row (document uid, node), the node and the size rendered.
type ThumbJob = ((u64, NodeId), Weak<Node>, u32);

#[derive(Default)]
pub(super) struct Thumbs {
    entries: HashMap<(u64, NodeId), ThumbEntry>,
    /// Documents still open. None until the app first synchronizes its documents.
    open_docs: Option<HashSet<u64>>,
    /// What the worker is rendering, by row.
    pending: HashMap<(u64, NodeId), (Weak<Node>, u32)>,
    worker: Option<ThumbWorker<(u64, NodeId), ThumbJob>>,
    worker_started: bool,
    /// The uid of the document whose rows were last drawn: the worker holds jobs for it alone.
    pub(super) showing: Option<u64>,
    settle: Settle,
    /// (pass, ms, count) rendered on the UI thread in that pass.
    spent: (u64, f64, u32),
    /// Thumbnails rendered on the UI thread (tests count them).
    pub(super) renders: u64,
    /// A test's clock (ms) for the render budget, replacing [`crate::now_ms`] so a test decides how
    /// long each render takes (production always leaves this `None`).
    #[cfg(test)]
    pub(super) clock: Option<Box<dyn FnMut() -> f64>>,
}

thread_local! {
    pub(super) static THUMBS: crate::graphics::TexCache<Thumbs> = crate::graphics::TexCache::default();
}

/// The most row thumbnails kept: past it the ones drawn longest ago go.
const MAX_THUMBS: usize = 512;

/// Rendering a thumbnail that took longer than this (ms) again goes to the worker.
const LIGHT_THUMB_MS: f64 = 4.0;

/// A node with more objects than this inside has its first thumbnail made on the worker.
const LIGHT_THUMB_NODES: usize = 200;

/// A node whose paths have more anchors than this in all has its first thumbnail made on the
/// worker. Type's glyphs, mesh points and image pixels count against it too, each as the anchors
/// that take about as long to render: [`GLYPH_ANCHORS`], [`MESH_POINT_ANCHORS`] and
/// [`IMAGE_PIXELS_PER_ANCHOR`].
const LIGHT_THUMB_ANCHORS: usize = 2_000;

/// Anchors one glyph of type counts as.
const GLYPH_ANCHORS: usize = 4;

/// Anchors one point of a gradient mesh counts as.
const MESH_POINT_ANCHORS: usize = 1;

/// Pixels of an image that count as one anchor.
const IMAGE_PIXELS_PER_ANCHOR: usize = 100;

/// The most symbols and placed documents counted inside one another: a node holding them deeper
/// (a symbol whose art holds an instance of itself, for one) has its first thumbnail made on the
/// worker.
const MAX_THUMB_NESTING: usize = 8;

/// The most steps counting a node takes: each object, subpath, text run and symbol looked at is
/// one step, and so is every 64 bytes of a symbol or placed file name looked up. A node that
/// needs more has its thumbnail made on the worker, and the painted thumbnail draws nothing for
/// it. A row drawn before counts at most this many objects too.
const THUMB_COUNT_STEPS: usize = 20_000;

/// At most this long (ms) and this many thumbnails are rendered on the UI thread in a pass; the
/// rest go to the worker, or wait for the next pass where there is none.
const THUMB_BUDGET_MS: f64 = 8.0;

const THUMB_BUDGET_COUNT: u32 = 16;

/// How row thumbnails render: images far smaller than they are sample their mip levels.
pub(crate) fn thumb_options() -> vectorcraft_render::RenderOptions {
    vectorcraft_render::RenderOptions { image_mips: true, ..Default::default() }
}

/// Has `n` at most `nodes` objects inside it, and at most `anchors` anchors in its paths and the
/// paths inside it, the frames of area type and the paths of type on a path included (counting
/// stops at either limit)? With `masks`, the art of the opacity masks that are not disabled, and
/// the objects and anchors inside it, count as well. With `doc` (the document `n` is in), type's
/// glyphs, mesh points and image pixels count as anchors too ([`LIGHT_THUMB_ANCHORS`]), and a
/// symbol instance and a placed document count the objects and anchors of the art they show: the
/// symbol's art in `doc`, and the placed file's art when it has been read (a placed document whose
/// file has not been read, or art nested deeper than [`MAX_THUMB_NESTING`], is not small). A node
/// that takes more than [`THUMB_COUNT_STEPS`] steps to count is not small.
fn small_node(n: &Node, nodes: usize, anchors: usize, masks: bool, doc: Option<&Document>) -> bool {
    weigh(n, nodes, anchors, masks, doc).0
}

/// [`small_node`], and the steps counting took.
fn weigh(n: &Node, nodes: usize, anchors: usize, masks: bool, doc: Option<&Document>) -> (bool, usize) {
    /// What may still be counted.
    struct Left {
        nodes: usize,
        anchors: usize,
        steps: usize,
        masks: bool,
    }
    impl Left {
        /// Count `anchors` more: false past the limit.
        fn take(&mut self, anchors: usize) -> bool {
            match self.anchors.checked_sub(anchors) {
                Some(left) => {
                    self.anchors = left;
                    true
                }
                None => false,
            }
        }
        /// Take `steps` more: false past the limit.
        fn step(&mut self, steps: usize) -> bool {
            match self.steps.checked_sub(steps) {
                Some(left) => {
                    self.steps = left;
                    true
                }
                None => false,
            }
        }
        /// Look up `name`: false past the step limit.
        fn look_up(&mut self, name: &str) -> bool {
            self.step(1 + name.len() / 64)
        }
        /// Count one more object: false past the limit.
        fn object(&mut self) -> bool {
            match self.nodes.checked_sub(1) {
                Some(left) => {
                    self.nodes = left;
                    true
                }
                None => false,
            }
        }
        /// Count the anchors of `p`, a step for each subpath.
        fn path(&mut self, p: &PathData) -> bool {
            p.subpaths.iter().all(|s| self.step(1) && self.take(s.anchors.len()))
        }
    }
    /// Count `art`, drawn in place of an object (`doc` its document), one level deeper.
    fn inside(art: &Node, doc: &Document, left: &mut Left, depth: usize) -> bool {
        depth < MAX_THUMB_NESTING && left.object() && count(art, Some(doc), left, depth + 1)
    }
    /// Count symbol `name` of `doc` (one that doesn't exist draws nothing).
    fn symbol(name: &str, doc: &Document, left: &mut Left, depth: usize) -> bool {
        for s in &doc.symbols {
            if !left.look_up(name) {
                return false;
            }
            if s.name == name {
                return inside(&s.art, doc, left, depth);
            }
        }
        true
    }
    /// Count what `n` draws besides its children.
    fn own(n: &Node, doc: Option<&Document>, left: &mut Left, depth: usize) -> bool {
        let path = match &n.kind {
            NodeKind::Text(t) => match &t.kind {
                TextKind::Area { frame: p } | TextKind::OnPath { path: p, .. } => Some(p),
                TextKind::Point => None,
            },
            _ => n.path_data(),
        };
        if !path.is_none_or(|p| left.path(p)) {
            return false;
        }
        let Some(doc) = doc else { return true };
        match &n.kind {
            // Glyphs are counted only up to one past what the anchors left allow.
            NodeKind::Text(t) => t.runs.iter().all(|r| {
                let glyphs = r.text.chars().take(left.anchors / GLYPH_ANCHORS + 1).count();
                left.step(1)
                    && left.take(glyphs.saturating_mul(GLYPH_ANCHORS))
                    && r.inline.as_ref().is_none_or(|a| symbol(&a.symbol, doc, left, depth))
            }),
            NodeKind::SymbolInstance { symbol: name, .. } => symbol(name, doc, left, depth),
            NodeKind::Mesh(m) => left.take(m.points.len().saturating_mul(MESH_POINT_ANCHORS)),
            NodeKind::Image(im) => left.take((im.width as usize).saturating_mul(im.height as usize) / IMAGE_PIXELS_PER_ANCHOR),
            NodeKind::PlacedDocument(p) => {
                left.look_up(&p.key) && vectorcraft_doc::placed_document::read_cached(p).is_some_and(|e| inside(&e.art, &e.doc, left, depth))
            }
            _ => true,
        }
    }
    fn count(n: &Node, doc: Option<&Document>, left: &mut Left, depth: usize) -> bool {
        if !(left.step(1) && own(n, doc, left, depth)) {
            return false;
        }
        let mask = n.extras().mask.as_deref().filter(|m| left.masks && !m.disabled).map(|m| &m.art);
        // A layer's ruler guides draw nothing in its thumbnail and use no budget.
        n.children().into_iter().flatten().filter(|c| !c.is_ruler_guide()).chain(mask).all(|c| left.object() && count(c, doc, left, depth))
    }
    let mut left = Left { nodes, anchors, steps: THUMB_COUNT_STEPS, masks };
    let small = count(n, doc, &mut left, 0);
    (small, THUMB_COUNT_STEPS - left.steps)
}

/// Is row `n` (an object of `doc`) light enough to make its thumbnail on the UI thread? `quick`:
/// whether its last thumbnail was quick to render, if it has one.
fn light_row(n: &Node, quick: Option<bool>, doc: &Document) -> bool {
    match quick {
        Some(quick) => quick && small_node(n, THUMB_COUNT_STEPS, LIGHT_THUMB_ANCHORS, true, Some(doc)),
        None => small_node(n, LIGHT_THUMB_NODES, LIGHT_THUMB_ANCHORS, true, Some(doc)),
    }
}

/// The thumbnail of `n` (`px` pixels square) as `r` renders it, on the UI thread or the worker.
fn render_thumb(r: &mut vectorcraft_render::Renderer, doc: &Document, n: &Node, px: u32) -> Option<vectorcraft_render::Rendered> {
    r.render_node_thumbnail_with(doc, n, px, None, &thumb_options())
}

impl Thumbs {
    #[cfg(test)]
    pub(super) fn cached(&self, uid: u64) -> bool {
        self.entries.keys().any(|key| key.0 == uid)
    }

    /// The time (ms) a render on the UI thread is measured by: [`crate::now_ms`], or a test's clock.
    fn now_ms(&mut self) -> f64 {
        #[cfg(test)]
        if let Some(clock) = self.clock.as_mut() {
            return clock();
        }
        crate::now_ms()
    }

    /// Release thumbnails and queued jobs for documents that have closed, keeping the cache for
    /// other open documents even when the panels are hidden or another document is active. When a
    /// document closes, the worker drops its jobs, the one running included, and the images it
    /// finished that the panels have not collected yet.
    fn retain_open(&mut self, uids: &[u64]) {
        if self.open_docs.as_ref().is_some_and(|open| open.len() == uids.len() && uids.iter().all(|uid| open.contains(uid))) {
            return;
        }
        let open: HashSet<u64> = uids.iter().copied().collect();
        let closed: HashSet<u64> = self.open_docs.as_ref().map(|before| before.difference(&open).copied().collect()).unwrap_or_default();
        let stale =
            self.entries.keys().chain(self.pending.keys()).any(|key| !open.contains(&key.0)) || self.showing.is_some_and(|uid| !open.contains(&uid));
        self.entries.retain(|key, _| open.contains(&key.0));
        self.pending.retain(|key, _| open.contains(&key.0));
        self.showing = self.showing.filter(|uid| open.contains(uid));
        if !closed.is_empty() {
            // Images finished before now are dropped with the rest of the queue: the open
            // documents' rows are asked for again. Documents opened later keep their jobs.
            if let Some(w) = &self.worker {
                w.forget(move |key| closed.contains(&key.0));
                if w.poll().is_none() {
                    self.worker = None;
                }
            }
            self.pending.clear();
        } else if stale
            && let Some(w) = &self.worker
            && !w.discard({
                let open = open.clone();
                move |key| !open.contains(&key.0)
            })
        {
            self.worker = None;
            self.pending.clear();
        }
        self.open_docs = Some(open);
    }

    /// Keep `img` (None: nothing drawn) as the thumbnail of row `key` showing `node`.
    fn upload(&mut self, ctx: &egui::Context, key: (u64, NodeId), node: Weak<Node>, px: u32, img: Option<&vectorcraft_render::Rendered>, ms: f64) {
        let color = img.map(crate::canvas::color_image);
        let used = ctx.cumulative_pass_nr();
        let name = format!("layer-thumb-{}", key.1.0);
        match self.entries.get_mut(&key) {
            Some(e) => {
                match (color, &mut e.tex) {
                    (Some(color), Some(tex)) => tex.set(color, egui::TextureOptions::LINEAR),
                    (Some(color), tex) => *tex = Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR)),
                    (None, tex) => *tex = None,
                }
                (e.node, e.px, e.ms, e.used) = (node, px, ms, used);
            }
            None => {
                let tex = color.map(|color| ctx.load_texture(name, color, egui::TextureOptions::LINEAR));
                self.entries.insert(key, ThumbEntry { node, px, tex, ms, used });
                if self.entries.len() > MAX_THUMBS {
                    // Keep the three quarters drawn last.
                    let mut by_use: Vec<u64> = self.entries.values().map(|e| e.used).collect();
                    by_use.sort_unstable();
                    let cut = by_use.get(by_use.len() - MAX_THUMBS * 3 / 4).copied().unwrap_or(0);
                    self.entries.retain(|_, e| e.used >= cut);
                }
            }
        }
    }

    /// The panels draw the rows of document `uid` (None: no document). When that is another
    /// document than before, the worker drops the jobs it has not started for every other
    /// document, so they don't hold up this one's thumbnails, and those rows are asked for again
    /// when their document shows again. The thumbnails of still-open documents stay.
    pub(super) fn show(&mut self, uid: Option<u64>) {
        if self.showing == uid {
            return;
        }
        self.showing = uid;
        self.pending.retain(|key, _| Some(key.0) == uid);
        if let Some(w) = &self.worker
            && !w.discard(move |key| Some(key.0) != uid)
        {
            self.worker = None;
            self.pending.clear();
        }
    }

    /// Keep the thumbnail `d` the worker finished.
    fn receive(&mut self, ctx: &egui::Context, d: Done<ThumbJob>) {
        let (key, node, px) = d.key;
        if self.open_docs.as_ref().is_some_and(|open| !open.contains(&key.0)) {
            return;
        }
        if self.pending.get(&key).is_some_and(|(p, ppx)| p.ptr_eq(&node) && *ppx == px) {
            self.pending.remove(&key);
        }
        self.upload(ctx, key, node, px, d.img.as_ref(), d.ms);
    }

    /// The texture of row `n` (`px` pixels square) to draw: made or updated now, or on the
    /// worker (the old one is drawn meanwhile). Thumbnails shown stay during a drag and until
    /// the document settles.
    fn texture(&mut self, ctx: &egui::Context, src: &ThumbSource, n: &Arc<Node>, px: u32) -> Option<egui::TextureHandle> {
        if !self.worker_started {
            self.worker_started = true;
            self.worker = ThumbWorker::spawn(ctx, "thumbnails");
        }
        let done = match self.worker.as_ref().map(ThumbWorker::poll) {
            Some(Some(done)) => done,
            Some(None) => {
                // The thread is gone: render here from now on.
                self.worker = None;
                self.pending.clear();
                vec![]
            }
            None => vec![],
        };
        for d in done {
            self.receive(ctx, d);
        }
        self.show(Some(src.uid));
        let key = (src.uid, n.id);
        let pass = ctx.cumulative_pass_nr();
        let (fresh, light) = match self.entries.get_mut(&key) {
            Some(e) => {
                e.used = pass;
                (std::ptr::eq(e.node.as_ptr(), Arc::as_ptr(n)) && e.px == px, Some(e.ms <= LIGHT_THUMB_MS))
            }
            None => (false, None),
        };
        let shown = self.entries.get(&key).and_then(|e| e.tex.clone());
        if fresh || self.pending.get(&key).is_some_and(|(p, ppx)| std::ptr::eq(p.as_ptr(), Arc::as_ptr(n)) && *ppx == px) {
            return shown;
        }
        if light.is_some() && (src.busy || !self.settle.settled(ctx, src.uid, src.revision, ctx.input(|i| i.time))) {
            return shown;
        }
        if self.spent.0 != pass {
            self.spent = (pass, 0.0, 0);
        }
        let in_budget = self.spent.1 < THUMB_BUDGET_MS && self.spent.2 < THUMB_BUDGET_COUNT;
        // A row drawn before is light when its last thumbnail was quick to render, unless its
        // object now has more anchors than the limit.
        let light = light_row(n, light, src.doc) && in_budget;
        if !light && let Some(w) = &self.worker {
            let (doc, node) = (src.doc.clone(), n.clone());
            let job: ThumbJob = (key, Arc::downgrade(n), px);
            if w.submit(key, job, Box::new(move |r| render_thumb(r, &doc, &node, px))) {
                self.pending.insert(key, (Arc::downgrade(n), px));
                return shown;
            }
            self.worker = None;
            self.pending.clear();
        }
        if !in_budget {
            ctx.request_repaint();
            return shown;
        }
        thread_local! {
            static RENDERER: std::cell::RefCell<vectorcraft_render::Renderer> = std::cell::RefCell::new(vectorcraft_render::Renderer::new());
        }
        let t0 = self.now_ms();
        let img = RENDERER.with(|r| render_thumb(&mut r.borrow_mut(), src.doc, n, px));
        let ms = self.now_ms() - t0;
        self.spent.1 += ms;
        self.spent.2 += 1;
        self.renders += 1;
        self.upload(ctx, key, Arc::downgrade(n), px, img.as_ref(), ms);
        self.entries.get(&key).and_then(|e| e.tex.clone())
    }
}

/// The Layers or Links panel shows the rows of document `uid` in this frame (None: no document
/// is open), whether or not a thumbnail is drawn; see [`Thumbs::show`].
pub(crate) fn show_thumbs_of(uid: Option<u64>) {
    THUMBS.with(|c| c.borrow_mut().show(uid));
}

/// Forget thumbnails of documents no longer open, even if neither thumbnail panel is shown.
pub(crate) fn retain_open_thumbs(uids: &[u64]) {
    THUMBS.with(|c| c.borrow_mut().retain_open(uids));
}

/// A real rendered thumbnail of row `n`, cached by node identity (unchanged nodes keep their `Arc`
/// allocation, so the address is a free change detector). Only rendered for visible rows; see
/// [`Thumbs::texture`] for when.
pub(crate) fn real_thumb(ui: &Ui, src: &ThumbSource, n: &Arc<Node>, r: egui::Rect) -> bool {
    if !ui.is_rect_visible(r) {
        return true;
    }
    let ppp = ui.ctx().pixels_per_point();
    let px = ((r.width() * ppp).round() as u32).max(8);
    match THUMBS.with(|c| c.borrow_mut().texture(ui.ctx(), src, n, px)) {
        Some(t) => {
            ui.painter().image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            true
        }
        None => false,
    }
}

/// Tiny vector thumbnail painted with egui (fallback when rendering isn't possible). A node with
/// more than [`LIGHT_THUMB_ANCHORS`] anchors, or that takes more than [`THUMB_COUNT_STEPS`] steps
/// to count, paints nothing, since this runs on every frame and
/// measuring its bounds takes time in proportion to its anchor count.
pub(super) fn thumb(ui: &Ui, n: &Node, r: egui::Rect) {
    if !small_node(n, usize::MAX, LIGHT_THUMB_ANCHORS, false, None) {
        return;
    }
    let Some(b) = n.visual_bounds() else { return };
    let s = ((r.width() - 3.0) as f64 / b.width().max(b.height()).max(1e-6)) as f32;
    let off = r.center() - vec2(b.center().x as f32 * s, b.center().y as f32 * s);
    let p = ui.painter().with_clip_rect(r);
    let mut count = 0;
    n.walk(&mut |c| {
        if count > 40 {
            return;
        }
        if let Some(pd) = c.path_data() {
            count += 1;
            let col = c.appearance.fill_paint().color().or(c.appearance.stroke_paint().color()).map(|cc| {
                let [a, b2, d, _] = cc.to_rgba8(1.0);
                Color32::from_rgb(a, b2, d)
            });
            if let Some(bb) = pd.bounds() {
                let rr = egui::Rect::from_min_max(off + vec2(bb.x0 as f32 * s, bb.y0 as f32 * s), off + vec2(bb.x1 as f32 * s, bb.y1 as f32 * s));
                p.rect_filled(rr, 0.0, col.unwrap_or(Color32::from_gray(120)));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Appearance, CharStyle, LayerColor, NodeKind, OpacityMask, TextKind, TextObject};
    use vectorcraft_geom::{Affine, Point, SubPath};

    use super::*;

    #[test]
    fn a_finished_job_cannot_restore_a_closed_documents_thumbnail() {
        let mut thumbs = Thumbs::default();
        let ctx = egui::Context::default();
        thumbs.retain_open(&[2]);
        thumbs.receive(&ctx, Done { key: ((1, NodeId(1)), Weak::new(), 16), img: None, ms: 0.0 });
        assert!(thumbs.entries.is_empty(), "a closed document's late result is discarded");
        thumbs.receive(&ctx, Done { key: ((2, NodeId(2)), Weak::new(), 16), img: None, ms: 0.0 });
        assert!(thumbs.entries.contains_key(&(2, NodeId(2))), "an open document's result is kept");
    }

    #[test]
    fn closing_a_document_drops_the_image_its_running_job_finishes_while_the_panels_are_hidden() {
        let ctx = egui::Context::default();
        let Some(worker) = ThumbWorker::start(&ctx, "test") else { return };
        let mut thumbs = Thumbs { worker: Some(worker), worker_started: true, ..Default::default() };
        thumbs.retain_open(&[1, 2]);
        // A job of document 1 is running, so discarding the queue cannot drop it.
        let (started, running) = std::sync::mpsc::channel::<()>();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let key = (1, NodeId(1));
        let job: crate::thumb_worker::JobFn = Box::new(move |_| {
            started.send(()).ok();
            held.recv().ok();
            None
        });
        assert!(thumbs.worker.as_ref().is_some_and(|w| w.submit(key, (key, Weak::new(), 1), job)));
        running.recv().expect("the job starts");
        // Another row of document 1 waits behind it.
        let queued = (1, NodeId(3));
        assert!(thumbs.worker.as_ref().is_some_and(|w| w.submit(queued, (queued, Weak::new(), 1), Box::new(|_| None))));
        // Document 1 closes while no panel collects results, then its running job finishes.
        thumbs.retain_open(&[2]);
        release.send(()).ok();
        // A job of document 2 queued after them comes back first: document 1's images were dropped.
        let next = (2, NodeId(2));
        assert!(thumbs.worker.as_ref().is_some_and(|w| w.submit(next, (next, Weak::new(), 1), Box::new(|_| None))));
        let done = thumbs.worker.as_ref().and_then(ThumbWorker::wait).expect("the same worker finishes document 2's job");
        assert_eq!(done.key.0, next, "no worker keeps the closed document's image");
    }

    #[test]
    fn a_document_opened_while_a_job_runs_after_a_close_gets_its_thumbnails() {
        let ctx = egui::Context::default();
        let Some(worker) = ThumbWorker::start(&ctx, "test") else { return };
        let mut thumbs = Thumbs { worker: Some(worker), worker_started: true, ..Default::default() };
        thumbs.retain_open(&[1, 2]);
        // A job of document 2 is running.
        let (started, running) = std::sync::mpsc::channel::<()>();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let blocker = (2, NodeId(1));
        let job: crate::thumb_worker::JobFn = Box::new(move |_| {
            started.send(()).ok();
            held.recv().ok();
            None
        });
        assert!(thumbs.worker.as_ref().is_some_and(|w| w.submit(blocker, (blocker, Weak::new(), 1), job)));
        running.recv().expect("the job starts");
        // Document 1 closes, then document 3 opens and one of its rows is queued.
        thumbs.retain_open(&[2]);
        thumbs.retain_open(&[2, 3]);
        let opened = (3, NodeId(2));
        assert!(thumbs.worker.as_ref().is_some_and(|w| w.submit(opened, (opened, Weak::new(), 1), Box::new(|_| None))));
        // A job of document 2 queued after it marks the end of the queue.
        let last = (2, NodeId(3));
        assert!(thumbs.worker.as_ref().is_some_and(|w| w.submit(last, (last, Weak::new(), 1), Box::new(|_| None))));
        release.send(()).ok();
        let worker = thumbs.worker.as_ref().expect("the worker is kept");
        let order: Vec<(u64, NodeId)> = (0..3).map_while(|_| worker.wait().map(|d| d.key.0)).take_while(|key| *key != last).collect();
        assert_eq!(order, [blocker, opened], "document 3's job comes back before the last one");
    }

    /// An open zigzag of `anchors` anchors.
    fn line(anchors: usize) -> PathData {
        let pts: Vec<Point> = (0..anchors).map(|i| Point::new(i as f64 * 0.01, (i % 2) as f64 * 50.0)).collect();
        PathData::single(SubPath::polyline(&pts, false))
    }

    /// An open stroked path `id` of `anchors` anchors.
    fn path(id: u64, anchors: usize) -> Node {
        Node::path(NodeId(id), line(anchors), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0))
    }

    /// Area type `id` whose frame has `anchors` anchors.
    fn area_type(id: u64, anchors: usize) -> Node {
        typed(id, TextKind::Area { frame: line(anchors) })
    }

    /// Type on a path `id` whose path has `anchors` anchors.
    fn path_type(id: u64, anchors: usize) -> Node {
        typed(id, TextKind::OnPath { path: line(anchors), start: 0.0, end: None })
    }

    fn typed(id: u64, kind: TextKind) -> Node {
        let mut t = TextObject::point(Point::ZERO, "Type", CharStyle::default());
        t.kind = kind;
        Node::new(NodeId(id), NodeKind::Text(Box::new(t)))
    }

    fn rect(id: u64) -> Node {
        let r = vectorcraft_geom::rect_from_points(Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        Node::path(NodeId(id), vectorcraft_geom::shapes::rectangle(r), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 1.0))
    }

    fn small(n: &Node) -> bool {
        small_node(n, LIGHT_THUMB_NODES, LIGHT_THUMB_ANCHORS, true, None)
    }

    /// Is `n`, an object of `doc`, small?
    fn small_in(doc: &Document, n: &Node) -> bool {
        small_node(n, LIGHT_THUMB_NODES, LIGHT_THUMB_ANCHORS, true, Some(doc))
    }

    /// `n` with an opacity mask of `art`, disabled or not.
    fn masked(mut n: Node, art: Node, disabled: bool) -> Node {
        let mut m = OpacityMask::new(art, true);
        m.disabled = disabled;
        n.extras_mut().mask = Some(Box::new(m));
        n
    }

    #[test]
    fn the_anchors_of_an_opacity_mask_count_unless_it_is_disabled() {
        let long = || path(2, LIGHT_THUMB_ANCHORS + 1);
        assert!(!small(&masked(rect(1), long(), false)), "a small object with a long mask");
        assert!(small(&masked(rect(1), long(), true)), "a small object with a long mask that is disabled");
        let inner = Node::group(NodeId(3), vec![Arc::new(long())]);
        assert!(!small(&masked(rect(1), inner, false)), "a mask holding a long path");
        let child = Node::group(NodeId(1), vec![Arc::new(masked(rect(4), long(), false))]);
        assert!(!small(&child), "a group holding an object with a long mask");
        assert!(small_node(&masked(rect(1), long(), false), usize::MAX, LIGHT_THUMB_ANCHORS, false, None), "masks left out");
    }

    #[test]
    fn the_first_thumbnail_goes_to_the_worker_by_anchors_as_well_as_objects() {
        assert!(!small(&path(1, LIGHT_THUMB_ANCHORS + 1)), "one long path");
        assert!(small(&path(1, LIGHT_THUMB_ANCHORS)), "a path of as many anchors as the limit");
        let group = |n: usize| Node::group(NodeId(1), (0..n).map(|i| Arc::new(rect(i as u64 + 2))).collect());
        assert!(small(&group(LIGHT_THUMB_NODES)), "many small objects");
        assert!(!small(&group(LIGHT_THUMB_NODES + 1)), "more objects than the limit");
        assert!(small(&Node::layer(NodeId(1), "Layer 1", LayerColor::Preset(0))), "an empty layer");
        let inner = Node::group(NodeId(2), vec![Arc::new(path(3, LIGHT_THUMB_ANCHORS + 1))]);
        assert!(!small(&Node::group(NodeId(1), vec![Arc::new(rect(4)), Arc::new(inner)])), "a group holding a long path");
        let halves = Node::group(NodeId(1), vec![Arc::new(path(2, LIGHT_THUMB_ANCHORS / 2)), Arc::new(path(3, LIGHT_THUMB_ANCHORS / 2 + 1))]);
        assert!(!small(&halves), "paths whose anchors together pass the limit");
        assert!(small(&area_type(1, LIGHT_THUMB_ANCHORS)), "area type whose frame has as many anchors as the limit");
        assert!(!small(&area_type(1, LIGHT_THUMB_ANCHORS + 1)), "area type with a long frame");
        assert!(small(&path_type(1, LIGHT_THUMB_ANCHORS)), "type on a path of as many anchors as the limit");
        assert!(!small(&path_type(1, LIGHT_THUMB_ANCHORS + 1)), "type on a long path");
        let both = Node::group(NodeId(1), vec![Arc::new(path(2, LIGHT_THUMB_ANCHORS / 2)), Arc::new(area_type(3, LIGHT_THUMB_ANCHORS / 2 + 1))]);
        assert!(!small(&both), "a path and an area frame whose anchors together pass the limit");
    }

    /// Point type `id` of `glyphs` glyphs.
    fn glyphs(id: u64, glyphs: usize) -> Node {
        let t = TextObject::point(Point::ZERO, &"a".repeat(glyphs), CharStyle::default());
        Node::new(NodeId(id), NodeKind::Text(Box::new(t)))
    }

    /// A gradient mesh `id` of `rows` x `rows` patches.
    fn mesh(id: u64, rows: u32) -> Node {
        let m = vectorcraft_doc::GradientMesh::from_surface(rows, rows, &|u, v| Point::new(u * 100.0, v * 100.0), &|_, _| Color::BLACK);
        Node::new(NodeId(id), NodeKind::Mesh(m))
    }

    /// An image `id` of `w` x `h` pixels.
    fn image(id: u64, w: u32, h: u32) -> Node {
        let im = vectorcraft_doc::ImageObject {
            key: "im".into(),
            width: w,
            height: h,
            xf: Affine::IDENTITY,
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        Node::new(NodeId(id), NodeKind::Image(Box::new(im)))
    }

    /// An instance `id` of symbol `name`.
    fn instance(id: u64, name: &str) -> Node {
        Node::new(NodeId(id), NodeKind::SymbolInstance { symbol: name.into(), xf: Affine::IDENTITY })
    }

    /// A document whose symbol `name` is `art`.
    fn with_symbol(name: &str, art: Node) -> Document {
        let mut doc = Document::new(200.0, 200.0);
        doc.symbols.push(vectorcraft_doc::Symbol { name: name.into(), art: Arc::new(art), dynamic: false });
        doc
    }

    /// A placed document `id` showing file `key`, read already when `art` is given.
    fn placed(id: u64, key: &str, art: Option<Node>) -> Node {
        let p = vectorcraft_doc::PlacedDocument {
            link: vectorcraft_doc::LinkInfo::new("/art/placed.vectorcraft"),
            key: key.into(),
            bounding: false,
            page_box: Default::default(),
            width: 100.0,
            height: 100.0,
            xf: Affine::IDENTITY,
            placement: Default::default(),
        };
        if let Some(art) = art {
            let frame = vectorcraft_geom::Rect::new(0.0, 0.0, 100.0, 100.0);
            vectorcraft_doc::placed_document::prime(&p, Arc::new(key.as_bytes().to_vec()), (Document::new(100.0, 100.0), art, frame));
        }
        Node::new(NodeId(id), NodeKind::PlacedDocument(Box::new(p)))
    }

    #[test]
    fn type_meshes_and_images_count_their_work_as_anchors() {
        let doc = Document::new(200.0, 200.0);
        let limit = LIGHT_THUMB_ANCHORS / GLYPH_ANCHORS;
        assert!(small_in(&doc, &glyphs(1, limit)), "type of as many glyphs as the limit");
        assert!(!small_in(&doc, &glyphs(1, limit + 1)), "type of more glyphs");
        assert!(!small_in(&doc, &glyphs(1, 5_000)), "long type");
        let beside = Node::group(NodeId(1), vec![Arc::new(glyphs(2, limit / 2)), Arc::new(path(3, LIGHT_THUMB_ANCHORS / 2 + 1))]);
        assert!(!small_in(&doc, &beside), "type and a path that together pass the limit");
        // (rows + 1)² points: 1,936 and 2,025.
        assert!(small_in(&doc, &mesh(1, 43)), "a small mesh");
        assert!(!small_in(&doc, &mesh(1, 44)), "a mesh of more points");
        let px = (LIGHT_THUMB_ANCHORS * IMAGE_PIXELS_PER_ANCHOR) as u32;
        assert!(small_in(&doc, &image(1, px / 100, 100)), "an image of as many pixels as the limit");
        assert!(!small_in(&doc, &image(1, px / 100, 101)), "a larger image");
        assert!(!small_in(&doc, &image(1, 4_000, 3_000)), "a photo");
        // Without the document only paths and frames count, as the painted thumbnail needs.
        assert!(small(&glyphs(1, 5_000)) && small(&mesh(1, 100)) && small(&image(1, 4_000, 3_000)));
    }

    #[test]
    fn a_symbol_instance_and_a_placed_document_count_the_art_they_show() {
        let doc = with_symbol("Long", path(10, LIGHT_THUMB_ANCHORS + 1));
        assert!(!small_in(&doc, &instance(1, "Long")), "an instance of a symbol of a long path");
        assert!(small(&instance(1, "Long")), "without the document");
        assert!(small_in(&doc, &instance(1, "Missing")), "an instance of a symbol that doesn't exist");
        let doc = with_symbol("Short", path(10, LIGHT_THUMB_ANCHORS));
        assert!(small_in(&doc, &instance(1, "Short")), "an instance of a symbol of a path at the limit");
        let two = Node::group(NodeId(1), vec![Arc::new(instance(2, "Short")), Arc::new(instance(3, "Short"))]);
        assert!(!small_in(&doc, &two), "two instances whose art together passes the limit");
        let many = with_symbol("Group", Node::group(NodeId(10), (0..LIGHT_THUMB_NODES).map(|i| Arc::new(rect(i as u64 + 11))).collect()));
        assert!(!small_in(&many, &instance(1, "Group")), "a symbol of more objects than the limit");
        // Inline graphics in type draw their symbol too.
        let mut t = TextObject::point(Point::ZERO, "a", CharStyle::default());
        t.runs[0].inline = Some(vectorcraft_doc::InlineArt { symbol: "Long".into(), scale: 1.0, baseline_shift: 0.0, bounds: None });
        let long = with_symbol("Long", path(10, LIGHT_THUMB_ANCHORS + 1));
        assert!(!small_in(&long, &Node::new(NodeId(1), NodeKind::Text(Box::new(t)))), "type showing a long symbol inline");

        let doc = Document::new(200.0, 200.0);
        assert!(small_in(&doc, &placed(1, "thumbs-small-placed", Some(path(10, LIGHT_THUMB_ANCHORS)))), "a placed file of a short path");
        assert!(!small_in(&doc, &placed(1, "thumbs-long-placed", Some(path(10, LIGHT_THUMB_ANCHORS + 1)))), "a placed file of a long path");
        assert!(!small_in(&doc, &placed(1, "thumbs-unread-placed", None)), "a placed file not read yet");
    }

    #[test]
    fn a_symbol_that_holds_itself_is_not_small_and_is_counted_to_an_end() {
        let doc = with_symbol("Loop", Node::group(NodeId(10), vec![Arc::new(rect(11)), Arc::new(instance(12, "Loop"))]));
        assert!(!small_in(&doc, &instance(1, "Loop")));
        assert!(!small_node(&instance(1, "Loop"), usize::MAX, usize::MAX, true, Some(&doc)), "without limits on objects and anchors");
        // Nested as deep as counted, the art is small.
        let mut doc = with_symbol("S0", rect(10));
        for i in 1..MAX_THUMB_NESTING {
            let art = Node::group(NodeId(10 + i as u64), vec![Arc::new(instance(100 + i as u64, &format!("S{}", i - 1)))]);
            doc.symbols.push(vectorcraft_doc::Symbol { name: format!("S{i}"), art: Arc::new(art), dynamic: false });
        }
        assert!(small_in(&doc, &instance(1, &format!("S{}", MAX_THUMB_NESTING - 1))), "symbols as deep as counted");
        let art = Node::group(NodeId(9), vec![Arc::new(instance(99, &format!("S{}", MAX_THUMB_NESTING - 1)))]);
        doc.symbols.push(vectorcraft_doc::Symbol { name: "Deeper".into(), art: Arc::new(art), dynamic: false });
        assert!(!small_in(&doc, &instance(1, "Deeper")), "one level deeper");
    }

    /// A document of symbols `L0` to `L{levels}`: `L0` is an empty group, and each one above it a
    /// group of `fan` instances of the one below.
    fn fan_out(levels: usize, fan: usize) -> Document {
        let mut doc = with_symbol("L0", Node::group(NodeId(10), vec![]));
        let mut id = 100;
        for level in 1..=levels {
            let below = format!("L{}", level - 1);
            let kids = (0..fan)
                .map(|_| {
                    id += 1;
                    Arc::new(instance(id, &below))
                })
                .collect();
            id += 1;
            doc.symbols.push(vectorcraft_doc::Symbol { name: format!("L{level}"), art: Arc::new(Node::group(NodeId(id), kids)), dynamic: false });
        }
        doc
    }

    #[test]
    fn symbols_that_each_hold_many_instances_are_counted_within_the_step_limit() {
        // 40 instances on each of 4 levels: 40⁴ (2,560,000) empty groups, and no anchors.
        let doc = fan_out(4, 40);
        let top = instance(1, "L4");
        assert!(!light_row(&top, Some(true), &doc), "a row drawn before, quick to render");
        assert!(!light_row(&top, None, &doc), "a row not drawn yet");
        let (small, steps) = weigh(&top, THUMB_COUNT_STEPS, usize::MAX, true, Some(&doc));
        assert!(!small && steps <= THUMB_COUNT_STEPS, "{steps} steps");
        let (small, steps) = weigh(&top, usize::MAX, usize::MAX, true, Some(&doc));
        assert!(!small && steps <= THUMB_COUNT_STEPS, "{steps} steps without a limit on objects");
        // 5 instances on each of 3 levels stay light once drawn quickly.
        let doc = fan_out(3, 5);
        assert!(light_row(&instance(1, "L3"), Some(true), &doc));
        assert!(!light_row(&instance(1, "L3"), Some(false), &doc), "drawn slowly");
    }

    #[test]
    fn empty_subpaths_text_runs_and_symbol_lookups_take_steps() {
        let doc = Document::new(200.0, 200.0);
        let style = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
        let empty = Node::path(NodeId(1), PathData::new(vec![SubPath::default(); THUMB_COUNT_STEPS]), style.clone());
        assert!(!small_in(&doc, &empty), "a path of many empty subpaths");
        let fewer = Node::path(NodeId(1), PathData::new(vec![SubPath::default(); 100]), style);
        assert!(small_in(&doc, &fewer), "a path of a few");
        let mut t = TextObject::point(Point::ZERO, "", CharStyle::default());
        t.runs = vec![vectorcraft_doc::TextRun::new("", CharStyle::default()); THUMB_COUNT_STEPS];
        assert!(!small_in(&doc, &Node::new(NodeId(2), NodeKind::Text(Box::new(t)))), "type of many empty runs");
        let mut doc = with_symbol("S", rect(10));
        for i in 0..THUMB_COUNT_STEPS {
            doc.symbols.push(vectorcraft_doc::Symbol { name: format!("Other {i}"), art: Arc::new(rect(11)), dynamic: false });
        }
        assert!(small_in(&doc, &instance(1, "S")), "the first of many symbols");
        assert!(!small_in(&doc, &instance(1, &format!("Other {}", THUMB_COUNT_STEPS - 1))), "the last of many symbols");
    }

    #[test]
    fn the_painted_thumbnail_leaves_out_a_long_path() {
        let ctx = egui::Context::default();
        let painted = |n: &Node| {
            let r = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(24.0, 24.0));
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| thumb(ui, n, r));
            out.textures_delta.clear();
            out.shapes.len()
        };
        let none = painted(&Node::layer(NodeId(1), "Layer 1", LayerColor::Preset(0)));
        assert_eq!(painted(&rect(1)), none + 1, "a small path is painted");
        assert_eq!(painted(&path(1, LIGHT_THUMB_ANCHORS)), none + 1, "a path of as many anchors as the limit");
        assert_eq!(painted(&path(1, LIGHT_THUMB_ANCHORS + 1)), none, "a long path is not");
        // Type is not painted, so each holds a small path beside it: the group is painted only
        // while the type's frame or path keeps the anchors in all within the limit.
        let left = LIGHT_THUMB_ANCHORS - rect(2).path_data().map_or(0, PathData::anchor_count);
        let beside = |typ: Node| Node::group(NodeId(1), vec![Arc::new(rect(2)), Arc::new(typ)]);
        assert_eq!(painted(&beside(area_type(3, left))), none + 1, "area type whose frame keeps the group at the limit");
        assert_eq!(painted(&beside(area_type(3, left + 1))), none, "area type with a long frame");
        assert_eq!(painted(&beside(path_type(3, left))), none + 1, "type on a path that keeps the group at the limit");
        assert_eq!(painted(&beside(path_type(3, left + 1))), none, "type on a long path");
    }

    /// A document with `n` on its layer, and `n` as the layer holds it.
    fn doc_with(n: impl FnOnce(NodeId) -> Node) -> (Arc<Document>, Arc<Node>) {
        let mut doc = Document::new(200.0, 200.0);
        let layer = doc.layers.first().map(|l| l.id);
        let id = doc.alloc_id();
        doc.insert(layer, usize::MAX, n(id)).unwrap();
        let doc = Arc::new(doc);
        let node = node_arc(&doc, id).unwrap();
        (doc, node)
    }

    #[test]
    fn a_long_paths_thumbnail_is_made_on_the_worker_and_matches_one_made_here() {
        let ctx = egui::Context::default();
        let Some(worker) = ThumbWorker::start(&ctx, "test") else { return };
        let mut thumbs = Thumbs { worker: Some(worker), worker_started: true, ..Default::default() };
        let (doc, n) = doc_with(|id| path(id.0, 50_000));
        let src = ThumbSource { uid: 1, revision: 1, doc: &doc, busy: false };
        assert!(thumbs.texture(&ctx, &src, &n, 24).is_none(), "nothing is shown until it is made");
        assert_eq!(thumbs.renders, 0, "not made on this thread");
        assert!(thumbs.pending.contains_key(&(1, n.id)), "sent to the worker");
        let done = thumbs.worker.as_ref().and_then(ThumbWorker::wait).expect("the worker finishes it");
        let there = done.img.as_ref().expect("the worker draws the path");
        let here = render_thumb(&mut vectorcraft_render::Renderer::new(), &doc, &n, 24).expect("this thread draws the path");
        assert!(here.pixels.chunks(4).any(|p| p[3] > 0), "the thumbnail shows the path");
        assert_eq!((there.width, there.height, &there.pixels), (here.width, here.height, &here.pixels), "the same pixels");
        thumbs.receive(&ctx, done);
        assert!(thumbs.pending.is_empty());
        assert!(thumbs.texture(&ctx, &src, &n, 24).is_some(), "the worker's thumbnail is shown");
        assert_eq!(thumbs.renders, 0, "and not made again here");

        // A small object's thumbnail is still made on this thread, at once.
        let (doc, n) = doc_with(|id| rect(id.0));
        let src = ThumbSource { uid: 2, revision: 1, doc: &doc, busy: false };
        assert!(thumbs.texture(&ctx, &src, &n, 24).is_some());
        assert_eq!(thumbs.renders, 1);
        assert!(thumbs.pending.is_empty());
    }

    /// A document of `rows` paths of 5,000 anchors on its layer, and the paths as the layer holds
    /// them.
    fn long_paths(rows: usize) -> (Arc<Document>, Vec<Arc<Node>>) {
        let mut doc = Document::new(200.0, 200.0);
        let layer = doc.layers.first().map(|l| l.id);
        let ids: Vec<NodeId> = (0..rows)
            .map(|_| {
                let id = doc.alloc_id();
                doc.insert(layer, usize::MAX, path(id.0, 5_000)).unwrap();
                id
            })
            .collect();
        let doc = Arc::new(doc);
        let nodes = ids.iter().filter_map(|id| node_arc(&doc, *id)).collect();
        (doc, nodes)
    }

    /// Keep the worker of `thumbs` busy with a job of row (0, 0) until the sender returned sends,
    /// so the jobs submitted meanwhile wait in its queue.
    fn hold(thumbs: &Thumbs) -> std::sync::mpsc::Sender<()> {
        let (started, running) = std::sync::mpsc::channel::<()>();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let key = (0, NodeId(0));
        let job: crate::thumb_worker::JobFn = Box::new(move |_| {
            started.send(()).ok();
            held.recv().ok();
            None
        });
        assert!(thumbs.worker.as_ref().is_some_and(|w| w.submit(key, (key, Weak::new(), 1), job)));
        running.recv().expect("the job starts");
        release
    }

    /// The next job `thumbs`'s worker finishes, checked against the thumbnail of the same node in
    /// `rows` of `doc` made on this thread, then kept.
    fn receive_checked(ctx: &egui::Context, thumbs: &mut Thumbs, uid: u64, doc: &Document, rows: &[Arc<Node>]) {
        let done = thumbs.worker.as_ref().and_then(ThumbWorker::wait).expect("the worker finishes a job");
        assert_eq!(done.key.0.0, uid, "a job of document {uid} is done next");
        let n = rows.iter().find(|n| n.id == done.key.0.1).expect("one of its rows");
        let there = done.img.as_ref().expect("the worker draws the path");
        let here = render_thumb(&mut vectorcraft_render::Renderer::new(), doc, n, 24).expect("this thread draws the path");
        assert!(here.pixels.chunks(4).any(|p| p[3] > 0), "the thumbnail shows the path");
        assert_eq!((there.width, there.height, &there.pixels), (here.width, here.height, &here.pixels), "the same pixels");
        thumbs.receive(ctx, done);
    }

    /// Prioritize the shown document’s thumbnails, requeue them when revisited, and retain cached rows.
    #[test]
    fn the_shown_documents_thumbnails_are_made_before_the_queued_ones_of_another() {
        let ctx = egui::Context::default();
        let Some(worker) = ThumbWorker::start(&ctx, "test") else { return };
        let mut thumbs = Thumbs { worker: Some(worker), worker_started: true, ..Default::default() };
        let (a, a_rows) = long_paths(3);
        let (b, b_rows) = long_paths(2);
        let release = hold(&thumbs);
        let src_a = ThumbSource { uid: 1, revision: 1, doc: &a, busy: false };
        for n in &a_rows {
            assert!(thumbs.texture(&ctx, &src_a, n, 24).is_none());
        }
        assert_eq!(thumbs.pending.len(), 3, "document 1's rows wait on the worker");
        // Document 2 shows before the worker gets to them.
        let src_b = ThumbSource { uid: 2, revision: 1, doc: &b, busy: false };
        for n in &b_rows {
            assert!(thumbs.texture(&ctx, &src_b, n, 24).is_none());
        }
        assert_eq!(thumbs.pending.keys().map(|k| k.0).collect::<Vec<_>>(), [2, 2], "only the shown document's rows are pending");
        release.send(()).expect("the held job is waiting");
        let held = thumbs.worker.as_ref().and_then(ThumbWorker::wait).expect("the held job finishes");
        assert_eq!(held.key.0, (0, NodeId(0)));
        for _ in &b_rows {
            receive_checked(&ctx, &mut thumbs, 2, &b, &b_rows);
        }
        for n in &b_rows {
            assert!(thumbs.texture(&ctx, &src_b, n, 24).is_some(), "the shown document's thumbnails are shown");
        }
        // Shown again, document 1's rows are asked for again and come back.
        // Hold the worker so texture() cannot collect a completed row before the count.
        let release = hold(&thumbs);
        for n in &a_rows {
            assert!(thumbs.texture(&ctx, &src_a, n, 24).is_none());
        }
        assert_eq!(thumbs.pending.keys().filter(|k| k.0 == 1).count(), 3);
        release.send(()).expect("the held job is waiting");
        let held = thumbs.worker.as_ref().and_then(ThumbWorker::wait).expect("the held job finishes");
        assert_eq!(held.key.0, (0, NodeId(0)));
        for _ in &a_rows {
            receive_checked(&ctx, &mut thumbs, 1, &a, &a_rows);
        }
        for n in &a_rows {
            assert!(thumbs.texture(&ctx, &src_a, n, 24).is_some());
        }
        // Document 2's thumbnails were kept: showing it again asks for nothing.
        for n in &b_rows {
            assert!(thumbs.texture(&ctx, &src_b, n, 24).is_some());
        }
        assert!(thumbs.pending.is_empty());
        assert_eq!(thumbs.renders, 0, "none was made on this thread");
    }

    #[test]
    fn the_queued_thumbnails_of_a_closed_document_are_dropped() {
        let ctx = egui::Context::default();
        let Some(worker) = ThumbWorker::start(&ctx, "test") else { return };
        let mut thumbs = Thumbs { worker: Some(worker), worker_started: true, ..Default::default() };
        let (a, a_rows) = long_paths(3);
        let release = hold(&thumbs);
        let src_a = ThumbSource { uid: 1, revision: 1, doc: &a, busy: false };
        for n in &a_rows {
            thumbs.texture(&ctx, &src_a, n, 24);
        }
        // The document closes and no other shows.
        thumbs.show(None);
        assert!(thumbs.pending.is_empty());
        // A job queued after it, straight on the worker, comes back next.
        let (b, b_rows) = long_paths(1);
        let (doc, node) = (b.clone(), b_rows[0].clone());
        let key = (2, node.id);
        let job: crate::thumb_worker::JobFn = Box::new(move |r| render_thumb(r, &doc, &node, 24));
        assert!(thumbs.worker.as_ref().is_some_and(|w| w.submit(key, (key, Arc::downgrade(&b_rows[0]), 24), job)));
        release.send(()).expect("the held job is waiting");
        let held = thumbs.worker.as_ref().and_then(ThumbWorker::wait).expect("the held job finishes");
        assert_eq!(held.key.0, (0, NodeId(0)));
        receive_checked(&ctx, &mut thumbs, 2, &b, &b_rows);
    }

    #[test]
    fn an_instance_of_a_long_symbols_thumbnail_is_made_on_the_worker_and_matches_one_made_here() {
        let ctx = egui::Context::default();
        let Some(worker) = ThumbWorker::start(&ctx, "test") else { return };
        let mut thumbs = Thumbs { worker: Some(worker), worker_started: true, ..Default::default() };
        let mut doc = with_symbol("Long", path(900, 50_000));
        let layer = doc.layers.first().map(|l| l.id);
        let id = doc.alloc_id();
        doc.insert(layer, usize::MAX, instance(id.0, "Long")).unwrap();
        let doc = Arc::new(doc);
        let n = node_arc(&doc, id).unwrap();
        let src = ThumbSource { uid: 1, revision: 1, doc: &doc, busy: false };
        assert!(thumbs.texture(&ctx, &src, &n, 24).is_none(), "nothing is shown until it is made");
        assert_eq!(thumbs.renders, 0, "not made on this thread");
        assert!(thumbs.pending.contains_key(&(1, n.id)), "sent to the worker");
        let done = thumbs.worker.as_ref().and_then(ThumbWorker::wait).expect("the worker finishes it");
        let there = done.img.as_ref().expect("the worker draws the symbol");
        let here = render_thumb(&mut vectorcraft_render::Renderer::new(), &doc, &n, 24).expect("this thread draws the symbol");
        assert!(here.pixels.chunks(4).any(|p| p[3] > 0), "the thumbnail shows the symbol's art");
        assert_eq!((there.width, there.height, &there.pixels), (here.width, here.height, &here.pixels), "the same pixels");
    }

    #[test]
    fn a_row_whose_object_becomes_a_long_path_is_drawn_again_on_the_worker() {
        let ctx = egui::Context::default();
        let Some(worker) = ThumbWorker::start(&ctx, "test") else { return };
        let mut thumbs = Thumbs { worker: Some(worker), worker_started: true, ..Default::default() };
        let (doc, n) = doc_with(|id| path(id.0, 50_000));
        let key = (1, n.id);
        // The row's last thumbnail showed a small object and took no time to render.
        let before = Arc::new(rect(n.id.0));
        thumbs.upload(&ctx, key, Arc::downgrade(&before), 24, None, 0.0);
        let src = ThumbSource { uid: 1, revision: 2, doc: &doc, busy: false };
        // The document reached this revision long before now, so it has settled.
        thumbs.settle.settled(&ctx, 1, 2, f64::NEG_INFINITY);
        thumbs.texture(&ctx, &src, &n, 24);
        assert_eq!(thumbs.renders, 0, "not made on this thread");
        assert!(thumbs.pending.contains_key(&key), "sent to the worker");
    }
}
