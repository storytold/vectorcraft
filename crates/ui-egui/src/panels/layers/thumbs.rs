use super::{Arc, Color32, Document, HashMap, Node, NodeId, Settle, ThumbWorker, Ui, VectorcraftApp, Weak, vec2};

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
    /// What the worker is rendering, by row.
    pending: HashMap<(u64, NodeId), (Weak<Node>, u32)>,
    worker: Option<ThumbWorker<(u64, NodeId), ThumbJob>>,
    worker_started: bool,
    settle: Settle,
    /// (pass, ms, count) rendered on the UI thread in that pass.
    spent: (u64, f64, u32),
    /// Thumbnails rendered on the UI thread (tests count them).
    pub(super) renders: u64,
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

/// At most this long (ms) and this many thumbnails are rendered on the UI thread in a pass; the
/// rest go to the worker, or wait for the next pass where there is none.
const THUMB_BUDGET_MS: f64 = 8.0;

const THUMB_BUDGET_COUNT: u32 = 16;

/// How row thumbnails render: images far smaller than they are sample their mip levels.
pub(crate) fn thumb_options() -> vectorcraft_render::RenderOptions {
    vectorcraft_render::RenderOptions { image_mips: true, ..Default::default() }
}

/// Has `n` at most `limit` objects inside it (counting stops there)?
fn small_node(n: &Node, limit: usize) -> bool {
    fn count(n: &Node, left: &mut usize) -> bool {
        for c in n.children().into_iter().flatten() {
            if *left == 0 {
                return false;
            }
            *left -= 1;
            if !count(c, left) {
                return false;
            }
        }
        true
    }
    let mut left = limit;
    count(n, &mut left)
}

impl Thumbs {
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
            let (key, node, px) = d.key;
            if self.pending.get(&key).is_some_and(|(p, ppx)| p.ptr_eq(&node) && *ppx == px) {
                self.pending.remove(&key);
            }
            self.upload(ctx, key, node, px, d.img.as_ref(), d.ms);
        }
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
        let light = light.unwrap_or_else(|| small_node(n, LIGHT_THUMB_NODES)) && in_budget;
        if !light && let Some(w) = &self.worker {
            let (doc, node) = (src.doc.clone(), n.clone());
            let job: ThumbJob = (key, Arc::downgrade(n), px);
            if w.submit(key, job, Box::new(move |r| r.render_node_thumbnail_with(&doc, &node, px, None, &thumb_options()))) {
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
        let t0 = crate::now_ms();
        let img = RENDERER.with(|r| r.borrow_mut().render_node_thumbnail_with(src.doc, n, px, None, &thumb_options()));
        let ms = crate::now_ms() - t0;
        self.spent.1 += ms;
        self.spent.2 += 1;
        self.renders += 1;
        self.upload(ctx, key, Arc::downgrade(n), px, img.as_ref(), ms);
        self.entries.get(&key).and_then(|e| e.tex.clone())
    }
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

/// Tiny vector thumbnail painted with egui (fallback when rendering isn't possible).
pub(super) fn thumb(ui: &Ui, n: &Node, r: egui::Rect) {
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
