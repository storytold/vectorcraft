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

/// A node whose paths have more anchors than this in all has its first thumbnail made on the
/// worker.
const LIGHT_THUMB_ANCHORS: usize = 2_000;

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
/// the objects and anchors inside it, count as well.
fn small_node(n: &Node, nodes: usize, anchors: usize, masks: bool) -> bool {
    fn own_anchors(n: &Node) -> usize {
        match &n.kind {
            NodeKind::Text(t) => match &t.kind {
                TextKind::Area { frame: p } | TextKind::OnPath { path: p, .. } => p.anchor_count(),
                TextKind::Point => 0,
            },
            _ => n.path_data().map_or(0, PathData::anchor_count),
        }
    }
    fn count(n: &Node, nodes: &mut usize, anchors: &mut usize, masks: bool) -> bool {
        let Some(left) = anchors.checked_sub(own_anchors(n)) else { return false };
        *anchors = left;
        let mask = n.extras().mask.as_deref().filter(|m| masks && !m.disabled).map(|m| &m.art);
        for c in n.children().into_iter().flatten().chain(mask) {
            if *nodes == 0 {
                return false;
            }
            *nodes -= 1;
            if !count(c, nodes, anchors, masks) {
                return false;
            }
        }
        true
    }
    let (mut nodes, mut anchors) = (nodes, anchors);
    count(n, &mut nodes, &mut anchors, masks)
}

/// The thumbnail of `n` (`px` pixels square) as `r` renders it, on the UI thread or the worker.
fn render_thumb(r: &mut vectorcraft_render::Renderer, doc: &Document, n: &Node, px: u32) -> Option<vectorcraft_render::Rendered> {
    r.render_node_thumbnail_with(doc, n, px, None, &thumb_options())
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

    /// Keep the thumbnail `d` the worker finished.
    fn receive(&mut self, ctx: &egui::Context, d: Done<ThumbJob>) {
        let (key, node, px) = d.key;
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
        let light = match light {
            Some(quick) => quick && small_node(n, usize::MAX, LIGHT_THUMB_ANCHORS, true),
            None => small_node(n, LIGHT_THUMB_NODES, LIGHT_THUMB_ANCHORS, true),
        } && in_budget;
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
        let t0 = crate::now_ms();
        let img = RENDERER.with(|r| render_thumb(&mut r.borrow_mut(), src.doc, n, px));
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

/// Tiny vector thumbnail painted with egui (fallback when rendering isn't possible). A node with
/// more than [`LIGHT_THUMB_ANCHORS`] anchors paints nothing, since this runs on every frame and
/// measuring its bounds takes time in proportion to its anchor count.
pub(super) fn thumb(ui: &Ui, n: &Node, r: egui::Rect) {
    if !small_node(n, usize::MAX, LIGHT_THUMB_ANCHORS, false) {
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
    use vectorcraft_geom::{Point, SubPath};

    use super::*;

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
        small_node(n, LIGHT_THUMB_NODES, LIGHT_THUMB_ANCHORS, true)
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
        assert!(small_node(&masked(rect(1), long(), false), usize::MAX, LIGHT_THUMB_ANCHORS, false), "masks left out");
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
