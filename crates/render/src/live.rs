//! Live objects in the renderer.
//!
//! Blends and envelopes are evaluated by `vectorcraft_doc::live` (through
//! `vectorcraft_effects::expand_live`, as the exporters do) and the resulting objects cached by
//! `Arc` identity of the live node and by the inputs from outside it that [`resources_key`] hashes:
//! the symbols, patterns and placed files it shows, the fonts for type, the plug-ins for plug-in
//! effects, and the color settings (so the steps keep
//! stable `Arc`s and hit the geometry cache frame after frame, and editing a symbol redraws an
//! envelope around an instance of it). Gradient meshes are tessellated into many small solid-colour quads (vello
//! has no mesh shading), with the density chosen from the on-screen patch size; each quad is
//! grown by half a device pixel along its edges so neighbours overlap and no antialiasing seams
//! show. The quads are made again when the color settings change.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Weak};

use vectorcraft_doc::live::{GradientMesh, MeshQuad};
use vectorcraft_doc::placed_document::ReadDocument;
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::{BezPath, Vec2};
use vello_cpu::RenderContext;
use vello_cpu::peniko;

use crate::fx::DocInputs;
use crate::lru::{self, FrameCache};
use crate::{Frame, Renderer};

type Expanded = Arc<Vec<Arc<Node>>>;
type Quads = Arc<Vec<MeshQuad>>;
/// An evaluation's [`resources_key`], with the frame's [`DocInputs`] and weak references to the
/// reads of the placed files shown. While they are kept, no other value takes an address the key
/// hashed: the [`DocInputs`] hold the symbol and pattern art and the color settings weakly.
pub(crate) type Found = (u64, Arc<DocInputs>, Vec<Weak<ReadDocument>>);
/// A mesh node, its quads, and the [`DocInputs`] whose color settings colored them.
type MeshEntry = (Arc<Node>, Quads, Arc<DocInputs>);

/// Per-renderer cache of evaluated live objects: (live node, its evaluation, [`Found`]), and a
/// [`MeshEntry`] per mesh node and subdivision level.
pub(crate) struct LiveCache {
    expanded: FrameCache<usize, (Arc<Node>, Expanded, Found)>,
    meshes: FrameCache<(usize, usize), MeshEntry>,
}

impl Default for LiveCache {
    fn default() -> Self {
        Self { expanded: FrameCache::new(lru::LIVE_BUDGET, 256), meshes: FrameCache::new(lru::MESH_BUDGET, 256) }
    }
}

impl LiveCache {
    /// End of frame `stamp` (see [`FrameCache::sweep`]).
    pub(crate) fn sweep(&mut self, stamp: u64) {
        self.expanded.sweep(stamp);
        self.meshes.sweep(stamp);
    }

    /// A cache whose evaluations take at most `budget` bytes.
    #[cfg(test)]
    pub(crate) fn with_budget(budget: usize) -> Self {
        Self { expanded: FrameCache::new(budget, 256), ..Default::default() }
    }
}

/// One level of evaluation of a live node (blend steps, envelope result with type outlined run by
/// run, mesh → flat pieces) without a document (symbol instances and pattern tiles in envelopes
/// stay as they are). Non-live nodes return themselves.
pub fn expand_live(n: &Node) -> Vec<Node> {
    crate::effects::expand_live(None, n)
}

/// What the evaluation of `n` (a live node, or an object whose effect art can be made from
/// evaluated live objects, see [`Renderer::live_found`]) reads from the document, placed files,
/// the fonts, the plug-ins and the color settings, hashed: the art of the symbols, the tilings of
/// the patterns and the reads of the placed files it shows ([`hash_resources`]), the font
/// generation ([`vectorcraft_text::FontDb::generation`]) when it shows type or a placed file, the
/// plug-in revision ([`crate::effects::plugin_revision`]) when it shows an object with a plug-in
/// effect, and the color settings (`cms`, their address). Art is hashed by the addresses of its
/// `Arc`s, which copies of the document share: a copy of the document has the same key. It also
/// returns weak references to the reads found.
fn resources_key(doc: &Document, cms: usize, n: &Node) -> (u64, Vec<Weak<ReadDocument>>) {
    let mut h = DefaultHasher::new();
    let mut shown = Shown::default();
    hash_resources(doc, n, &mut shown, &mut h);
    if shown.text {
        vectorcraft_text::FontDb::global().generation().hash(&mut h);
    }
    if shown.plugins {
        crate::effects::plugin_revision().hash(&mut h);
    }
    cms.hash(&mut h);
    (h.finish(), shown.reads)
}

/// What [`hash_resources`] has found so far.
#[derive(Default)]
struct Shown {
    /// The addresses of the pattern definitions and the symbol art walked.
    seen: Vec<usize>,
    /// Whether type or a placed file was found.
    text: bool,
    /// Whether an object with a visible plug-in effect was found.
    plugins: bool,
    /// Weak references to the reads of the placed files found.
    reads: Vec<Weak<ReadDocument>>,
}

/// Hash into `h` what the evaluation of `n` reads from the document and from placed files: the
/// tilings ([`crate::pattern::hash_tiling`]) of the patterns in the fills and strokes of its
/// objects and of type's characters, the address of each symbol instance's art, and the address of
/// the read of each placed file that [`vectorcraft_doc::PlacedDocument::art`] shows (none before
/// the file is read). It walks `n` and then, one at a time from a list, the opacity-mask, pattern
/// and symbol art it finds, each pattern and each symbol's art once ([`first_seen`]). `shown`
/// records the type, placed files, reads and plug-in effects found.
fn hash_resources<'a>(doc: &'a Document, n: &'a Node, shown: &mut Shown, h: &mut DefaultHasher) {
    let mut todo = vec![];
    let mut next = Some(n);
    while let Some(art) = next {
        art.walk(&mut |c| {
            shown.plugins |= crate::effects::has_plugin_effect(c);
            let runs = match &c.kind {
                NodeKind::Text(t) => t.runs.as_slice(),
                _ => &[],
            };
            let paints = c.appearance.items.iter().map(|i| i.paint()).chain(runs.iter().flat_map(|r| [&r.style.fill, &r.style.stroke]));
            for paint in paints {
                if let vectorcraft_color::Paint::Pattern { pattern, .. } = paint
                    && let Some(def) = doc.pattern(pattern)
                    && first_seen(&mut shown.seen, std::ptr::from_ref(def) as usize)
                {
                    crate::pattern::hash_tiling(def, h);
                    todo.extend(def.art.iter().map(|a| &**a));
                }
            }
            match &c.kind {
                NodeKind::SymbolInstance { symbol, .. } => {
                    if let Some(s) = doc.symbols.iter().find(|s| s.name == *symbol) {
                        (Arc::as_ptr(&s.art) as usize).hash(h);
                        if first_seen(&mut shown.seen, Arc::as_ptr(&s.art) as usize) {
                            todo.push(&s.art);
                        }
                    }
                }
                NodeKind::PlacedDocument(p) => {
                    let read = vectorcraft_doc::placed_document::read_cached(p);
                    read.as_ref().map(|r| Arc::as_ptr(r) as usize).hash(h);
                    shown.reads.extend(read.as_ref().map(Arc::downgrade));
                    shown.text = true;
                }
                NodeKind::Text(_) => shown.text = true,
                _ => {}
            }
            if let Some(m) = c.extras().mask.as_deref() {
                todo.push(&m.art);
            }
        });
        next = todo.pop();
    }
}

/// Is `address` not in `seen` yet? It is added when it isn't.
fn first_seen(seen: &mut Vec<usize>, address: usize) -> bool {
    let first = !seen.contains(&address);
    if first {
        seen.push(address);
    }
    first
}

/// Subdivisions per patch for a patch about `size_px` device pixels across.
fn mesh_level(m: &GradientMesh, bounds_px: f64) -> usize {
    let per = bounds_px / (m.rows.max(m.cols).max(1) as f64);
    ((per / 5.0).ceil() as usize).clamp(2, 32)
}

impl Renderer {
    /// The [`resources_key`] of `n` in `doc`, with the frame's [`DocInputs`] ([`Found`]): `n` is a
    /// live node, or an object with effects whose art ([`Renderer::fx_art`]) can be made from
    /// evaluated live objects.
    pub(crate) fn live_found(&mut self, doc: &Document, n: &Node) -> Found {
        let inputs = self.doc_inputs(doc);
        let (key, reads) = resources_key(doc, inputs.cms, n);
        (key, inputs, reads)
    }

    fn live_expanded(&mut self, doc: &Document, a: &Arc<Node>, cache: bool) -> Expanded {
        let key = Arc::as_ptr(a) as usize;
        let resources = self.live_found(doc, a);
        if cache
            && let Some(e) = self.live.expanded.get(&key, self.stamp)
            && Arc::ptr_eq(&e.0, a)
            && e.2.0 == resources.0
        {
            return e.1.clone();
        }
        let v: Expanded = match crate::effects::pathfinder_children(a, crate::effects::text_outliner()) {
            Some(children) => Arc::new(children),
            None => Arc::new(crate::effects::expand_live(Some(doc), a).into_iter().map(Arc::new).collect()),
        };
        if cache {
            let bytes = size_of::<Expanded>() + v.iter().map(|n| lru::node_bytes(n)).sum::<usize>();
            self.live.expanded.insert(key, (a.clone(), v.clone(), resources), bytes, self.stamp);
        }
        v
    }

    /// The quads of mesh `a` (`m`) at subdivision `level`, kept while the node and the color
    /// settings that colored them ([`GradientMesh::color_at`]) stay the same.
    fn mesh_quads(&mut self, doc: &Document, a: &Arc<Node>, m: &GradientMesh, level: usize, cache: bool) -> Arc<Vec<MeshQuad>> {
        let key = (Arc::as_ptr(a) as usize, level);
        let inputs = self.doc_inputs(doc);
        if cache
            && let Some(e) = self.live.meshes.get(&key, self.stamp)
            && Arc::ptr_eq(&e.0, a)
            && e.2.cms == inputs.cms
        {
            return e.1.clone();
        }
        let q = Arc::new(m.quads(level));
        if cache {
            let bytes = q.len() * size_of::<MeshQuad>();
            self.live.meshes.insert(key, (a.clone(), q.clone(), inputs), bytes, self.stamp);
        }
        q
    }

    /// Draw a live node reached through the tree walk (handles its transparency group).
    pub(crate) fn draw_live(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>) {
        let opacity = self.opacity_of(a);
        // A knockout blend's steps knock each other out, as in the exports (opaque ones can't show it).
        let knockout = !f.opts.outline
            && matches!(a.kind, NodeKind::Blend { .. })
            && a.knockout.resolve(self.knockout)
            && vectorcraft_doc::live::steps_knockout_shows(&self.live_expanded(f.doc, a, true));
        let enclosing = std::mem::replace(&mut self.knockout, knockout);
        if !f.opts.outline && (opacity < 1.0 || a.blend != vectorcraft_color::BlendMode::Normal || a.isolate || knockout) {
            let blends = self.blends_through(a);
            let bounds = if blends { self.bounds_of(f.symbols, a) } else { None };
            let comp = crate::group::Composite { blend: a.blend, opacity, isolated: a.isolate, blends, bounds, ..Default::default() };
            self.group(ctx, f, comp, &mut |r, c, fr| r.draw_live_body(c, fr, a, true));
        } else {
            self.draw_live_body(ctx, f, a, true);
        }
        self.knockout = enclosing;
        self.stats.drawn += 1;
    }

    /// Draw the evaluated content of a live node (no transparency group).
    pub(crate) fn draw_live_body(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>, cache: bool) {
        match &a.kind {
            NodeKind::PlacedDocument(p) => self.draw_placed(ctx, f, p),
            NodeKind::Mesh(m) => self.draw_mesh(ctx, f, a, m, cache),
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Repeat(_)
            | NodeKind::Intertwine { .. }
            | NodeKind::Group { .. }
            | NodeKind::Layer { .. } => {
                let items = self.live_expanded(f.doc, a, cache);
                // An evaluation the cache did not keep is made again in the next frame.
                let kept = cache && self.live.expanded.get(&(Arc::as_ptr(a) as usize), self.stamp).is_some_and(|e| Arc::ptr_eq(&e.1, &items));
                let outer = self.made_again;
                self.made_again = outer || !kept;
                // The flag holds while this blend is drawn (see `draw_live`).
                if self.knockout && matches!(a.kind, NodeKind::Blend { .. }) {
                    self.draw_knockout(ctx, f, &items);
                } else {
                    for c in items.iter() {
                        self.draw_arc(ctx, f, c);
                    }
                }
                self.made_again = outer;
            }
            _ => {}
        }
    }

    fn draw_mesh(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>, m: &GradientMesh, cache: bool) {
        if !m.is_valid() {
            return;
        }
        if f.opts.outline {
            let bp = m.lines().to_bezpath();
            self.hairline(ctx, f, &bp, [0, 0, 0, 255]);
            return;
        }
        let Some(b) = m.bounds() else { return };
        let level = mesh_level(m, b.width().max(b.height()) / f.px.max(1e-12));
        let quads = self.mesh_quads(f.doc, a, m, level, cache);
        ctx.set_transform(f.view);
        ctx.set_fill_rule(peniko::Fill::NonZero);
        let grow = f.px * 0.5;
        let alpha = self.alpha;
        for q in quads.iter() {
            if q.opacity <= 0.0 {
                continue;
            }
            let mut bp = BezPath::new();
            for (i, p) in q.pts.iter().enumerate() {
                // Out along both edges at the corner, so thin quads grow across too.
                let d: Vec2 = [q.pts[(i + 3) % 4], q.pts[(i + 1) % 4]]
                    .iter()
                    .map(|n| *p - *n)
                    .filter(|e| e.hypot() > 1e-12)
                    .map(|e| e / e.hypot())
                    .fold(Vec2::ZERO, |a, e| a + e);
                let p2 = *p + d * grow;
                if i == 0 {
                    bp.move_to(p2);
                } else {
                    bp.line_to(p2);
                }
            }
            bp.close_path();
            ctx.set_paint(f.ink.color(&q.color, q.opacity * alpha));
            ctx.fill_path(&bp);
        }
    }

    /// Draw a live node reached without its `Arc` (thumbnails, clip fallbacks): uncached.
    pub(crate) fn draw_live_node(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node) {
        let a = Arc::new(n.clone());
        self.draw_live_body(ctx, f, &a, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::{Appearance, EnvelopeKind, Symbol};
    use vectorcraft_geom::{Affine, Rect, shapes};

    /// The key of an envelope around the first of 5,000 symbols, each showing the next, is found on
    /// a 256 KiB stack, and it changes when the last symbol's art changes.
    #[test]
    fn a_long_chain_of_symbols_is_walked_on_a_small_stack() {
        const N: usize = 5_000;
        let mut d = Document::new(100.0, 100.0);
        let square = |d: &mut Document| Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default());
        for k in 0..N {
            let art = match k + 1 < N {
                true => Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: format!("S{}", k + 1), xf: Affine::IDENTITY }),
                false => square(&mut d),
            };
            d.symbols.push(Symbol { name: format!("S{k}"), art: Arc::new(art) });
        }
        let first = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S0".into(), xf: Affine::IDENTITY });
        let kind = EnvelopeKind::Warp { style: "arc".into(), bend: 30.0, h: 0.0, v: 0.0, horizontal: true };
        let envelope = NodeKind::Envelope {
            content: vec![Arc::new(first)],
            kind: Box::new(kind),
            fidelity: 50.0,
            editing: false,
            options: Default::default(),
            frame: Box::new(Affine::IDENTITY),
        };
        let envelope = Node::new(d.alloc_id(), envelope);
        let key = |d: &Document| {
            std::thread::scope(|s| {
                std::thread::Builder::new().stack_size(256 << 10).spawn_scoped(s, || resources_key(d, 0, &envelope).0).unwrap().join().unwrap()
            })
        };
        let before = key(&d);
        let last = square(&mut d);
        d.symbols[N - 1].art = Arc::new(last);
        assert_ne!(key(&d), before, "the last symbol's art is hashed");
    }

    /// The key of an envelope around instances of symbols `A`, `B` and `C`, where `A` and `C` show
    /// the same art, changes when `C` is given the art of `B`.
    #[test]
    fn every_symbol_instance_hashes_its_art() {
        let mut d = Document::new(100.0, 100.0);
        let square = |d: &mut Document, x: f64| {
            Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::default()))
        };
        let (x, y) = (square(&mut d, 0.0), square(&mut d, 20.0));
        for (name, art) in [("A", &x), ("B", &y), ("C", &x)] {
            d.symbols.push(Symbol { name: name.into(), art: art.clone() });
        }
        let content = ["A", "B", "C"].map(|s| Arc::new(Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: s.into(), xf: Affine::IDENTITY })));
        let kind = EnvelopeKind::Warp { style: "arc".into(), bend: 30.0, h: 0.0, v: 0.0, horizontal: true };
        let envelope = NodeKind::Envelope {
            content: content.to_vec(),
            kind: Box::new(kind),
            fidelity: 50.0,
            editing: false,
            options: Default::default(),
            frame: Box::new(Affine::IDENTITY),
        };
        let envelope = Node::new(d.alloc_id(), envelope);
        let before = resources_key(&d, 0, &envelope).0;
        d.symbols[2].art = y.clone();
        assert_ne!(resources_key(&d, 0, &envelope).0, before, "the art of every instance is hashed");
    }
}
