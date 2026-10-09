//! Live objects in the renderer.
//!
//! Blends and envelopes are evaluated by `vectorcraft_doc::live` (through
//! `vectorcraft_effects::expand_live`, as the exporters do) and the resulting objects cached by
//! `Arc` identity of the live node and of the symbols and patterns it draws (so the steps keep
//! stable `Arc`s and hit the geometry cache frame after frame, and editing a symbol redraws an
//! envelope around an instance of it). Gradient meshes are tessellated into many small solid-colour quads (vello
//! has no mesh shading), with the density chosen from the on-screen patch size; each quad is
//! grown by half a device pixel along its edges so neighbours overlap and no antialiasing seams
//! show.

use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_doc::live::{GradientMesh, MeshQuad};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::{BezPath, Vec2};
use vello_cpu::RenderContext;
use vello_cpu::peniko;

use crate::{Frame, Renderer};

type Expanded = Arc<Vec<Arc<Node>>>;
type MeshEntry = (Arc<Node>, Arc<Vec<MeshQuad>>, u64);

/// Per-renderer cache of evaluated live objects.
#[derive(Default)]
pub(crate) struct LiveCache {
    expanded: HashMap<usize, (Arc<Node>, Expanded, u64, u64)>,
    meshes: HashMap<(usize, usize), MeshEntry>,
    stamp: u64,
}

impl LiveCache {
    fn tick(&mut self, stamp: u64) {
        self.stamp = stamp;
        if self.expanded.len() > 256 {
            self.expanded.retain(|_, e| stamp - e.2 <= 3);
        }
        if self.meshes.len() > 256 {
            self.meshes.retain(|_, e| stamp - e.2 <= 3);
        }
    }
}

/// One level of evaluation of a live node (blend steps, envelope result with type outlined run by
/// run, mesh → flat pieces) without a document (symbol instances and pattern tiles in envelopes
/// stay as they are). Non-live nodes return themselves.
pub fn expand_live(n: &Node) -> Vec<Node> {
    crate::effects::expand_live(None, n)
}

/// What an evaluated live node depends on besides itself: the art of the symbols and the patterns
/// it draws (their `Arc`s outlive document copies, so this changes only when one is edited).
fn resources_key(doc: &Document, n: &Node) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    n.walk(&mut |c| {
        if let NodeKind::SymbolInstance { symbol, .. } = &c.kind
            && let Some(s) = doc.symbols.iter().find(|s| s.name == *symbol)
        {
            (Arc::as_ptr(&s.art) as usize).hash(&mut h);
        }
        for i in &c.appearance.items {
            if let vectorcraft_color::Paint::Pattern { pattern, .. } = i.paint()
                && let Some(def) = doc.pattern(pattern)
            {
                def.art.iter().for_each(|a| (Arc::as_ptr(a) as usize).hash(&mut h));
                [def.tile.x0, def.tile.y0, def.tile.x1, def.tile.y1].iter().for_each(|v| v.to_bits().hash(&mut h));
            }
        }
    });
    h.finish()
}

/// Subdivisions per patch for a patch about `size_px` device pixels across.
fn mesh_level(m: &GradientMesh, bounds_px: f64) -> usize {
    let per = bounds_px / (m.rows.max(m.cols).max(1) as f64);
    ((per / 5.0).ceil() as usize).clamp(2, 32)
}

impl Renderer {
    fn live_expanded(&mut self, doc: &Document, a: &Arc<Node>, cache: bool) -> Expanded {
        let key = Arc::as_ptr(a) as usize;
        let resources = resources_key(doc, a);
        if cache
            && let Some(e) = self.live.expanded.get_mut(&key)
            && Arc::ptr_eq(&e.0, a)
            && e.3 == resources
        {
            e.2 = self.live.stamp;
            return e.1.clone();
        }
        let v: Expanded = match crate::effects::pathfinder_children(a, crate::effects::text_outliner()) {
            Some(children) => Arc::new(children),
            None => Arc::new(crate::effects::expand_live(Some(doc), a).into_iter().map(Arc::new).collect()),
        };
        if cache {
            self.live.expanded.insert(key, (a.clone(), v.clone(), self.live.stamp, resources));
        }
        v
    }

    fn mesh_quads(&mut self, a: &Arc<Node>, m: &GradientMesh, level: usize, cache: bool) -> Arc<Vec<MeshQuad>> {
        let key = (Arc::as_ptr(a) as usize, level);
        if cache
            && let Some(e) = self.live.meshes.get_mut(&key)
            && Arc::ptr_eq(&e.0, a)
        {
            e.2 = self.live.stamp;
            return e.1.clone();
        }
        let q = Arc::new(m.quads(level));
        if cache {
            self.live.meshes.insert(key, (a.clone(), q.clone(), self.live.stamp));
        }
        q
    }

    /// Draw a live node reached through the tree walk (handles its transparency group).
    pub(crate) fn draw_live(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>) {
        let stamp = self.stamp;
        self.live.tick(stamp);
        let opacity = self.opacity_of(a);
        // A knockout blend's steps knock each other out, as in the exports (opaque ones can't show it).
        let knockout = !f.opts.outline
            && matches!(a.kind, NodeKind::Blend { .. })
            && a.knockout.resolve(self.knockout)
            && vectorcraft_doc::live::steps_knockout_shows(&self.live_expanded(f.doc, a, true));
        let enclosing = std::mem::replace(&mut self.knockout, knockout);
        if !f.opts.outline && (opacity < 1.0 || a.blend != vectorcraft_color::BlendMode::Normal || a.isolate || knockout) {
            let blends = self.blends_through(a);
            let bounds = if blends { self.bounds_of(a) } else { None };
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
            | NodeKind::Group { .. }
            | NodeKind::Layer { .. }
            | NodeKind::CompoundShape { .. } => {
                let items = self.live_expanded(f.doc, a, cache);
                // The flag holds while this blend is drawn (see `draw_live`).
                if self.knockout && matches!(a.kind, NodeKind::Blend { .. }) {
                    self.draw_knockout(ctx, f, &items);
                } else {
                    for c in items.iter() {
                        self.draw_arc(ctx, f, c);
                    }
                }
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
        let quads = self.mesh_quads(a, m, level, cache);
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
