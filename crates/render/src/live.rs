//! Live objects in the renderer.
//!
//! Blends and envelopes are evaluated by `vectorcraft_doc::live` (through
//! `vectorcraft_effects::expand_live`, as the exporters do) and the resulting objects cached by
//! `Arc` identity of the live node and by the inputs from outside it that [`resources_key`] hashes:
//! the symbols, patterns and placed files it shows, the fonts for type, the plug-ins for plug-in
//! effects, and the color settings (so the steps keep
//! stable `Arc`s and hit the geometry cache frame after frame, and editing a symbol redraws an
//! envelope around an instance of it). What the walk behind that key found is kept with the node
//! and with the document's symbols and patterns as the walk read them ([`Walk`]). A later frame
//! makes the key from what was kept while the node and the document's symbols and patterns stay
//! the same. A walk that found a placed file is not kept.
//! Gradient meshes are tessellated into many small solid-colour quads (vello
//! has no mesh shading), with the density chosen from the on-screen patch size; each quad is
//! grown by half a device pixel along its edges so neighbours overlap and no antialiasing seams
//! show. The quads are made again when the color settings change.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Weak};

use vectorcraft_doc::live::{GradientMesh, MeshQuad};
use vectorcraft_doc::placed_document::ReadDocument;
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::{Affine, BezPath, Point, Vec2};
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
/// A node, held weakly (the reference keeps its allocation, so no other node gets its address),
/// what the walk of [`resources_key`] found in it, and the frame's [`DocInputs`] the walk read
/// the symbols and patterns of.
type Walk = (Weak<Node>, Walked, Arc<DocInputs>);

/// Per-renderer cache of evaluated live objects: (live node, its evaluation, [`Found`]), a
/// [`MeshEntry`] per mesh node and subdivision level, and a [`Walk`] per node whose key
/// [`Renderer::live_found`] found without finding a placed file.
pub(crate) struct LiveCache {
    expanded: FrameCache<usize, (Arc<Node>, Expanded, Found)>,
    meshes: FrameCache<(usize, usize), MeshEntry>,
    walks: FrameCache<usize, Walk>,
    /// How many live objects [`Renderer::live_expanded`] evaluated and how many meshes
    /// [`Renderer::mesh_quads`] tessellated.
    #[cfg(test)]
    pub(crate) evaluated: usize,
    /// How many walks [`Renderer::live_found`] made.
    #[cfg(test)]
    pub(crate) walked: usize,
}

impl Default for LiveCache {
    fn default() -> Self {
        Self {
            expanded: FrameCache::new(lru::LIVE_BUDGET, 256),
            meshes: FrameCache::new(lru::MESH_BUDGET, 256),
            walks: FrameCache::new(lru::WALK_BUDGET, 256),
            #[cfg(test)]
            evaluated: 0,
            #[cfg(test)]
            walked: 0,
        }
    }
}

impl LiveCache {
    /// End of frame `stamp` (see [`FrameCache::sweep`]).
    pub(crate) fn sweep(&mut self, stamp: u64) {
        self.expanded.sweep(stamp);
        self.meshes.sweep(stamp);
        self.walks.sweep(stamp);
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

/// What the evaluation of `n` (a live node, or an object whose effect art the renderer keeps with
/// its inputs, see [`Renderer::live_found`]) reads from the document, placed files,
/// the fonts, the plug-ins and the color settings, hashed: the art of the symbols, the tilings of
/// the patterns and the reads of the placed files it shows ([`hash_resources`]), the font
/// generation ([`vectorcraft_text::FontDb::generation`]) when it shows type or a placed file, the
/// plug-in revision ([`crate::effects::plugin_revision`]) when it shows an object with a plug-in
/// effect, and the color settings (`cms`, their address). Art is hashed by the addresses of its
/// `Arc`s, which copies of the document share: a copy of the document has the same key. It also
/// returns weak references to the reads found, and what the walk found ([`Walked`]) when it found
/// no placed file.
fn resources_key(doc: &Document, cms: usize, n: &Node) -> (u64, Vec<Weak<ReadDocument>>, Option<Walked>) {
    let mut h = DefaultHasher::new();
    let mut shown = Shown::default();
    hash_resources(doc, n, &mut shown, &mut h);
    let walked = Walked { hash: h.finish(), text: shown.text, plugins: shown.plugins };
    (walked.key(cms), shown.reads, (!shown.placed).then_some(walked))
}

/// What [`hash_resources`] found in a node: the hash of what it walked, whether it found type or
/// a placed file, and whether it found an object with a visible plug-in effect.
#[derive(Clone, Copy)]
struct Walked {
    hash: u64,
    text: bool,
    plugins: bool,
}

impl Walked {
    /// The [`resources_key`] of the node walked: the walk's hash, the font generation when it found
    /// type or a placed file, the plug-in revision when it found a plug-in effect, and the color
    /// settings `cms`.
    fn key(self, cms: usize) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash.hash(&mut h);
        if self.text {
            vectorcraft_text::FontDb::global().generation().hash(&mut h);
        }
        if self.plugins {
            crate::effects::plugin_revision().hash(&mut h);
        }
        cms.hash(&mut h);
        h.finish()
    }
}

/// What [`hash_resources`] has found so far.
#[derive(Default)]
struct Shown {
    /// The addresses of the pattern definitions and the symbol art walked.
    seen: Vec<usize>,
    /// Whether type or a placed file was found.
    text: bool,
    /// Whether a placed file was found.
    placed: bool,
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
                    shown.placed = true;
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

/// The most pixels a gradient mesh is rasterized into at once ([`draw_mesh_raster`]); a larger
/// one is drawn quad by quad. The buffer is clipped to the frame; at 4 MP it holds at most 16 MB
/// of pixels per mesh and frame.
const MESH_RASTER_MAX_PIXELS: usize = 4 << 20;

/// Draw a tessellated gradient mesh as one image: its quads are filled into a pixel buffer of the
/// mesh's on-screen box by scanline (each pixel takes the quad its centre is in, so neighbouring
/// quads meet without seams or overlap, and is shaded between the quad's corner colours, so the
/// colour changes smoothly instead of in steps), and the buffer is drawn through the mesh's outline, which
/// antialiases the outer edge. The outline is grown by half a device pixel ([`grown_outline`]), as
/// the quads drawn one by one are: a page of meshes laid edge to edge would otherwise show the
/// background through each shared, antialiased edge. Drawing every quad as a path of its own costs a
/// path's setup per quad: a page of thousands of meshes made millions of them at print size (minutes), where this
/// costs about one write per covered pixel. → false (nothing drawn) when the box is too large for
/// one buffer, so the caller draws the quads.
fn draw_mesh_raster(ctx: &mut RenderContext, f: &Frame, m: &GradientMesh, quads: &[MeshQuad], alpha: f32) -> bool {
    let outline = m.outline().to_bezpath();
    let dev = f.view.transform_rect_bbox(vectorcraft_geom::Shape::bounding_box(&outline));
    if ![dev.x0, dev.y0, dev.x1, dev.y1].iter().all(|v| v.is_finite()) {
        return false;
    }
    let (cw, ch) = (f64::from(ctx.width()), f64::from(ctx.height()));
    let (x0, y0) = ((dev.x0.floor() - 2.0).max(0.0), (dev.y0.floor() - 2.0).max(0.0));
    let (x1, y1) = ((dev.x1.ceil() + 2.0).min(cw), (dev.y1.ceil() + 2.0).min(ch));
    if x1 <= x0 || y1 <= y0 {
        // Off the context: nothing to draw.
        return true;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if w * h > MESH_RASTER_MAX_PIXELS || w > usize::from(u16::MAX) || h > usize::from(u16::MAX) {
        return false;
    }
    let clear = vello_cpu::color::PremulRgba8 { r: 0, g: 0, b: 0, a: 0 };
    let mut px = vec![clear; w * h];
    let mut filled = vec![false; w * h];
    let to_buffer = Affine::translate((-x0, -y0)) * f.view;
    for q in quads {
        // A fully transparent corner still shades toward clear, so the edge fill below leaves it clear.
        let corners = q.corners.map(|(color, opacity)| {
            let c = if opacity > 0.0 { f.ink.color(&color, opacity * alpha).premultiply().to_rgba8() } else { clear };
            [c.r, c.g, c.b, c.a].map(f32::from)
        });
        fill_quad(&mut px, &mut filled, w, h, q.pts.map(|p| to_buffer * p), corners);
    }
    // Twice: the grown edge samples pixels up to about a pixel and a half out from the quads.
    let filled = spread_to_edges(&mut px, filled, w, h);
    spread_to_edges(&mut px, filled, w, h);
    let pm = Arc::new(vello_cpu::Pixmap::from_parts(px, w as u16, h as u16));
    ctx.set_transform(Affine::translate((x0, y0)));
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler: peniko::ImageSampler::default() });
    ctx.fill_path(&grown_outline(&(Affine::translate((-x0, -y0)) * f.view * outline), 0.5));
    ctx.set_transform(Affine::IDENTITY);
    true
}

/// `path` (device pixels) flattened and each closed subpath moved out by `grow` along its edges'
/// normals (mitred, the mitre capped at twice `grow`), so one fill covers the outline and a band
/// around it once: filling the outline and stroking it as well would paint a translucent mesh's
/// edge twice, a darker line.
fn grown_outline(path: &BezPath, grow: f64) -> BezPath {
    let mut rings: Vec<Vec<Point>> = Vec::new();
    vectorcraft_geom::kurbo::flatten(path, 0.1, |el| match el {
        vectorcraft_geom::PathEl::MoveTo(p) => rings.push(vec![p]),
        vectorcraft_geom::PathEl::LineTo(p) => {
            if let Some(r) = rings.last_mut()
                && r.last().is_none_or(|q| (*q - p).hypot() > 1e-9)
            {
                r.push(p);
            }
        }
        _ => {}
    });
    let mut out = BezPath::new();
    for mut r in rings {
        if r.len() > 1 && r.first().zip(r.last()).is_some_and(|(a, b)| (*a - *b).hypot() <= 1e-9) {
            r.pop();
        }
        let n = r.len();
        if n < 3 {
            continue;
        }
        // (d.y, -d.x) points out of a ring with a positive shoelace sum (clockwise on a y-down screen).
        let area: f64 = (0..n).map(|i| r[i].to_vec2().cross(r[(i + 1) % n].to_vec2())).sum();
        let side = if area > 0.0 { 1.0 } else { -1.0 };
        let normal = |a: Point, b: Point| {
            let d = b - a;
            let l = d.hypot();
            if l > 1e-12 { Vec2::new(d.y, -d.x) * (side / l) } else { Vec2::ZERO }
        };
        for i in 0..n {
            let (prev, p, next) = (r[(i + n - 1) % n], r[i], r[(i + 1) % n]);
            let (a, b) = (normal(prev, p), normal(p, next));
            let sum = a + b;
            let k = 1.0 + a.dot(b);
            let mut d = if k > 1e-6 { sum * (grow / k) } else { a * grow };
            if d.hypot() > 2.0 * grow {
                d = d * (2.0 * grow / d.hypot());
            }
            let q = p + d;
            if i == 0 {
                out.move_to(q);
            } else {
                out.line_to(q);
            }
        }
        out.close_path();
    }
    out
}

/// Fill the pixels of the `w`×`h` buffer whose centres are inside quad `p` (buffer pixels, even-odd
/// so a twisted quad is filled as drawn) with `c`, and mark them `filled`.
fn fill_quad(px: &mut [vello_cpu::color::PremulRgba8], filled: &mut [bool], w: usize, h: usize, p: [Point; 4], c: [[f32; 4]; 4]) {
    if !p.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
        return;
    }
    let (min_y, max_y) = p.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), q| (a.min(q.y), b.max(q.y)));
    let first = (min_y - 0.5).ceil().max(0.0) as usize;
    let last = ((max_y - 0.5).ceil().max(0.0) as usize).min(h);
    for y in first..last {
        let yc = y as f64 + 0.5;
        let mut xs = [0.0f64; 4];
        let mut n = 0;
        for i in 0..4 {
            let (a, b) = (p[i], p[(i + 1) % 4]);
            if (a.y <= yc) != (b.y <= yc) {
                xs[n] = a.x + (yc - a.y) * (b.x - a.x) / (b.y - a.y);
                n += 1;
            }
        }
        let xs = &mut xs[..n];
        xs.sort_by(f64::total_cmp);
        for [a, b] in xs.as_chunks::<2>().0 {
            let from = (a - 0.5).ceil().clamp(0.0, w as f64) as usize;
            let to = (b - 0.5).ceil().clamp(0.0, w as f64) as usize;
            let row = y * w;
            for ((x, d), k) in (from..)
                .zip(px.get_mut(row + from..row + to).unwrap_or_default().iter_mut())
                .zip(filled.get_mut(row + from..row + to).unwrap_or_default())
            {
                *d = shade(p, &c, Point::new(x as f64 + 0.5, yc));
                *k = true;
            }
        }
    }
}

/// The colour at `q` in quad `p` with corner colours `c` (premultiplied, 0–255): the corners'
/// colours mixed by `q`'s barycentric weights in the quad's triangle 0-1-2 or 0-2-3, whichever
/// holds it. A tessellated quad is a few pixels across, so the split along its diagonal doesn't show.
fn shade(p: [Point; 4], c: &[[f32; 4]; 4], q: Point) -> vello_cpu::color::PremulRgba8 {
    let weights = |a: Point, b: Point, d: Point| -> Option<[f64; 3]> {
        let det = (b.x - a.x) * (d.y - a.y) - (d.x - a.x) * (b.y - a.y);
        if det.abs() < 1e-12 {
            return None;
        }
        let wb = ((q.x - a.x) * (d.y - a.y) - (d.x - a.x) * (q.y - a.y)) / det;
        let wd = ((b.x - a.x) * (q.y - a.y) - (q.x - a.x) * (b.y - a.y)) / det;
        Some([1.0 - wb - wd, wb, wd])
    };
    let inside = |w: &[f64; 3]| w.iter().all(|w| *w >= -1e-6);
    let first = weights(p[0], p[1], p[2]).map(|w| (w, [0, 1, 2]));
    let second = weights(p[0], p[2], p[3]).map(|w| (w, [0, 2, 3]));
    // The triangle holding `q`, else (at the quad's edge, or a folded quad) the nearer one.
    let pick = match (first, second) {
        (Some(a), _) if inside(&a.0) => Some(a),
        (_, Some(b)) if inside(&b.0) => Some(b),
        (Some(a), Some(b)) => Some(if a.0.iter().fold(0.0f64, |m, w| m.min(*w)) >= b.0.iter().fold(0.0f64, |m, w| m.min(*w)) { a } else { b }),
        (a, b) => a.or(b),
    };
    let mix = match pick {
        Some((w, idx)) => {
            let w = w.map(|w| w.clamp(0.0, 1.0));
            let sum = w.iter().sum::<f64>().max(1e-12);
            std::array::from_fn(|ch| (0..3).map(|k| c[idx[k]][ch] * (w[k] / sum) as f32).sum::<f32>())
        }
        None => std::array::from_fn(|ch| c.iter().map(|corner| corner[ch]).sum::<f32>() / 4.0),
    };
    let [r, g, b, a] = mix.map(|v| v.round().clamp(0.0, 255.0) as u8);
    vello_cpu::color::PremulRgba8 { r: r.min(a), g: g.min(a), b: b.min(a), a }
}

/// Give each pixel no quad covered the colour of a covered neighbour: the outline's antialiased
/// edge samples pixels whose centres fall just outside the quads, which would otherwise fade it.
/// → the pixels covered now (the given ones and those just given a colour).
fn spread_to_edges(px: &mut [vello_cpu::color::PremulRgba8], filled: Vec<bool>, w: usize, h: usize) -> Vec<bool> {
    let mut now = filled.clone();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if filled.get(i).copied().unwrap_or(true) {
                continue;
            }
            let near = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            if let Some(j) = near.into_iter().flatten().find(|j| filled.get(*j).copied().unwrap_or(false))
                && let Some(c) = px.get(j).copied()
                && let Some(d) = px.get_mut(i)
            {
                *d = c;
                if let Some(k) = now.get_mut(i) {
                    *k = true;
                }
            }
        }
    }
    now
}

impl Renderer {
    /// The [`resources_key`] of `n` in `doc`, with the frame's [`DocInputs`] ([`Found`]): `n` is a
    /// live node, or an object with effects whose art ([`Renderer::fx_art`]) can be made from
    /// evaluated live objects, type, symbol art or colors shaded through the color settings. The
    /// key is made from the [`Walk`] kept for `n` while `doc`'s symbols and patterns are those it
    /// read; `n` is walked again when they changed, and in every frame when the walk found a
    /// placed file.
    pub(crate) fn live_found(&mut self, doc: &Document, n: &Arc<Node>) -> Found {
        let inputs = self.doc_inputs(doc);
        let at = Arc::as_ptr(n) as usize;
        if let Some((node, walked, read)) = self.live.walks.get(&at, self.stamp)
            && std::ptr::eq(node.as_ptr(), Arc::as_ptr(n))
            && read.same_art(&inputs)
        {
            return (walked.key(inputs.cms), inputs, vec![]);
        }
        #[cfg(test)]
        {
            self.live.walked += 1;
        }
        let (key, reads, walked) = resources_key(doc, inputs.cms, n);
        if let Some(walked) = walked {
            // The bytes count the key and the entry, and the node's allocation that the weak
            // reference keeps.
            let bytes = size_of::<(usize, Walk, Node)>();
            self.live.walks.insert(at, (Arc::downgrade(n), walked, inputs.clone()), bytes, self.stamp);
        }
        (key, inputs, reads)
    }

    fn live_expanded(&mut self, doc: &Document, a: &Arc<Node>, cache: bool) -> Expanded {
        let key = Arc::as_ptr(a) as usize;
        // An evaluation the cache does not keep is made without the key, so the copy that
        // `draw_live_node` makes is not walked.
        let resources = cache.then(|| self.live_found(doc, a));
        if let Some(resources) = &resources
            && let Some(e) = self.live.expanded.get(&key, self.stamp)
            && Arc::ptr_eq(&e.0, a)
            && e.2.0 == resources.0
        {
            return e.1.clone();
        }
        #[cfg(test)]
        {
            self.live.evaluated += 1;
        }
        let v: Expanded = match crate::effects::pathfinder_children(a, crate::effects::text_outliner()) {
            Some(children) => Arc::new(children),
            None => Arc::new(crate::effects::expand_live(Some(doc), a).into_iter().map(Arc::new).collect()),
        };
        if let Some(resources) = resources {
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
        #[cfg(test)]
        {
            self.live.evaluated += 1;
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
        ctx.set_fill_rule(peniko::Fill::NonZero);
        let alpha = self.alpha;
        if draw_mesh_raster(ctx, f, m, &quads, alpha) {
            ctx.set_transform(f.view);
            return;
        }
        ctx.set_transform(f.view);
        // Where a patch narrows to slivers, many quads share each pixel, each covering a little of it: their edges
        // composite to part coverage, and hard edges drop each one. An opaque patch is backed by its own outline in
        // its middle colour, one path that covers fully, so the background can't show through.
        for r in 0..m.rows as usize {
            for c in 0..m.cols as usize {
                if [m.idx(r, c), m.idx(r, c + 1), m.idx(r + 1, c), m.idx(r + 1, c + 1)].iter().any(|&i| m.points[i].opacity < 1.0) {
                    continue;
                }
                let (top, right, bottom, left) = (m.edge_u(r, c), m.edge_v(r, c + 1), m.edge_u(r + 1, c), m.edge_v(r, c));
                let mut bp = BezPath::new();
                bp.move_to(top.p0);
                bp.curve_to(top.p1, top.p2, top.p3);
                bp.curve_to(right.p1, right.p2, right.p3);
                bp.curve_to(bottom.p2, bottom.p1, bottom.p0);
                bp.curve_to(left.p2, left.p1, left.p0);
                bp.close_path();
                ctx.set_paint(f.ink.color(&m.color_at(r, c, 0.5, 0.5).0, alpha));
                ctx.fill_path(&bp);
            }
        }
        let grow = f.px * 0.5;
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

    /// Draw a live node that [`Self::draw_content`] reached without its kept `Arc`
    /// ([`Self::live_root`]): the node of a thumbnail ([`Self::render_node_thumbnail_with`]), symbol
    /// art placed again in every frame, color glyph art, and brush art that the brush cache did not
    /// keep. It evaluates a copy of the node and does not use the live cache.
    pub(crate) fn draw_live_node(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node) {
        let a = Arc::new(n.clone());
        self.draw_live_body(ctx, f, &a, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mesh_patch_of_slivers_leaves_no_holes_with_hard_edges() {
        // GUARDS: patches that narrow to slivers (here fanning out from nearly one point) draw as many sub-pixel quads;
        // with hard edges each was dropped below half coverage, leaving holes through the mesh.
        use vectorcraft_doc::live::MeshPoint;
        let point = |x: f64, y: f64, handles: [(f64, f64); 4]| MeshPoint {
            p: vectorcraft_geom::Point::new(x, y),
            color: vectorcraft_color::Color::rgb(0.7, 0.8, 0.9),
            opacity: 1.0,
            handles: handles.map(|(x, y)| Vec2::new(x, y)),
        };
        let rows = [
            ((100.0, 60.0), (84.0, 46.0)),
            ((98.5, 60.0), (60.0, 51.0)),
            ((97.8, 60.0), (42.0, 52.0)),
            ((70.0, 59.0), (28.0, 51.5)),
            ((15.0, 56.0), (12.0, 55.0)),
        ];
        let mut points = vec![];
        for (i, &((x0, y0), (x1, y1))) in rows.iter().enumerate() {
            let along = if i < 3 { 5.0 } else { 20.0 };
            points.push(point(x0, y0, [((x1 - x0) * 0.35, (y1 - y0) * 0.1), (0.0, 0.0), (-along, 0.0), (if i > 0 { 5.0 } else { 0.0 }, 0.0)]));
            points.push(point(x1, y1, [(0.0, 0.0), ((x0 - x1) * 0.35, (y0 - y1) * 0.6), (-8.0, 0.5), (8.0, -0.5)]));
        }
        let mut d = Document::new(110.0, 30.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::new(id, NodeKind::Mesh(GradientMesh { rows: 4, cols: 1, points }))).unwrap();
        let view = Affine::scale(4.0) * Affine::translate((0.0, -40.0));
        let draw = |anti_alias| crate::Renderer::new().render(&d, 440, 120, view, &crate::RenderOptions { anti_alias, ..Default::default() });
        let (smooth, hard) = (draw(crate::AntiAlias::Art), draw(crate::AntiAlias::None));
        let holes =
            (0..120).flat_map(|y| (0..440).map(move |x| (x, y))).filter(|&(x, y)| smooth.pixel(x, y)[3] >= 200 && hard.pixel(x, y)[3] == 0).count();
        assert!(holes < 60, "{holes} pixels the smooth render covers are empty with hard edges");
    }

    /// A quad is shaded between its corner colours: across a quad running from black on the left to
    /// white on the right, each column is lighter than the one before (no flat step), the left and
    /// right edges are near their corners' colours, and a column's pixels are alike.
    #[test]
    fn a_quad_shades_smoothly_between_its_corner_colours() {
        let (w, h) = (40, 10);
        let clear = vello_cpu::color::PremulRgba8 { r: 0, g: 0, b: 0, a: 0 };
        let mut px = vec![clear; w * h];
        let mut filled = vec![false; w * h];
        let (black, white) = ([0.0, 0.0, 0.0, 255.0], [255.0, 255.0, 255.0, 255.0]);
        let pts = [Point::new(0.0, 0.0), Point::new(40.0, 0.0), Point::new(40.0, 10.0), Point::new(0.0, 10.0)];
        fill_quad(&mut px, &mut filled, w, h, pts, [black, white, white, black]);
        let at = |x: usize, y: usize| px[y * w + x].r;
        for y in 0..h {
            for x in 1..w {
                assert!(at(x, y) > at(x - 1, y), "column {x} row {y}: {} after {}", at(x, y), at(x - 1, y));
            }
            assert!(at(0, y) < 8 && at(w - 1, y) > 247, "row {y}: {} to {}", at(0, y), at(w - 1, y));
        }
        for x in 0..w {
            assert!(at(x, 0).abs_diff(at(x, h - 1)) <= 1, "column {x}: {} vs {}", at(x, 0), at(x, h - 1));
        }
    }

    /// Quads that share edges fill every pixel of their union once: no gap, no overlap.
    #[test]
    fn quads_meeting_edge_to_edge_fill_their_union_once() {
        let (w, h) = (20, 20);
        let clear = vello_cpu::color::PremulRgba8 { r: 0, g: 0, b: 0, a: 0 };
        let red = [[255.0, 0.0, 0.0, 255.0]; 4];
        let mut px = vec![clear; w * h];
        let mut count = vec![0u8; w * h];
        // A 2×2 grid over (2.3, 2.7)–(17.6, 16.2) with an inner corner off the pixel grid.
        let xs = [2.3, 9.71, 17.6];
        let ys = [2.7, 8.35, 16.2];
        for j in 0..2 {
            for i in 0..2 {
                let pts = [Point::new(xs[i], ys[j]), Point::new(xs[i + 1], ys[j]), Point::new(xs[i + 1], ys[j + 1]), Point::new(xs[i], ys[j + 1])];
                let mut mine = vec![false; w * h];
                fill_quad(&mut px, &mut mine, w, h, pts, red);
                for (c, m) in count.iter_mut().zip(&mine) {
                    *c += u8::from(*m);
                }
            }
        }
        for y in 0..h {
            for x in 0..w {
                let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                let inside = (2.3..17.6).contains(&cx) && (2.7..16.2).contains(&cy);
                assert_eq!(count[y * w + x], u8::from(inside), "pixel {x},{y}");
            }
        }
        let filled: Vec<bool> = count.iter().map(|c| *c > 0).collect();
        let filled = spread_to_edges(&mut px, filled, w, h);
        assert_eq!(px[2 * w + 2].a, 255, "the pixel outside the corner takes its neighbour's colour");
        assert_eq!(px[2 * w + 1].a, 0, "one pass reaches one pixel out");
        spread_to_edges(&mut px, filled, w, h);
        assert_eq!(px[2 * w + 1].a, 255, "a second pass reaches the next one, which the grown outline samples");
    }

    /// A grown square covers half a pixel more on every side, whichever way it winds.
    #[test]
    fn an_outline_grows_by_half_a_pixel_both_ways_round() {
        for square in [vectorcraft_geom::shapes::rectangle(Rect::new(2.0, 3.0, 10.0, 8.0)).to_bezpath(), {
            let mut p = BezPath::new();
            p.move_to((2.0, 3.0));
            p.line_to((2.0, 8.0));
            p.line_to((10.0, 8.0));
            p.line_to((10.0, 3.0));
            p.close_path();
            p
        }] {
            let b = vectorcraft_geom::Shape::bounding_box(&grown_outline(&square, 0.5));
            assert!((b.x0 - 1.5).abs() < 1e-6 && (b.y0 - 2.5).abs() < 1e-6, "{b:?}");
            assert!((b.x1 - 10.5).abs() < 1e-6 && (b.y1 - 8.5).abs() < 1e-6, "{b:?}");
        }
    }
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
