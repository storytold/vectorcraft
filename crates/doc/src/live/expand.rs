use super::envelope::{EnvelopeOptions, expand_envelope};
use super::mesh::GradientMesh;
use super::warp::map_nonlinear;
use super::*;

/// Optional hook that converts nodes the pure evaluation can't map (text) to outlines.
pub type Outliner<'a> = Option<&'a dyn Fn(&Node) -> Option<Node>>;

/// Distort Pattern Fills: the tiles pattern swatch `name` painted with placement `xf` lays over
/// `region` (see [`crate::PatternDef::instances_in`]); `None` without such a swatch.
pub type PatternTiles<'a> = &'a dyn Fn(&str, Affine, Rect) -> Option<Vec<Node>>;

/// What live evaluation asks of the crates above this one (`vectorcraft-effects` supplies them).
#[derive(Clone, Copy, Default)]
pub struct Hooks<'a> {
    /// Type as its glyph outlines, a symbol instance as its art (see [`Outliner`]).
    pub outline: Outliner<'a>,
    /// Distort Appearance: a path's strokes and geometry effects baked into filled art (`None`
    /// when it has none to bake).
    pub appearance: Outliner<'a>,
    /// Distort Pattern Fills: the tiles a pattern fill lays (see [`PatternTiles`]).
    pub pattern: Option<PatternTiles<'a>>,
    /// The document whose symbols' art counts toward the instance budget of a Repeat of their
    /// instances ([`crate::RepeatSpec::expand`]).
    pub symbols: Option<&'a crate::Document>,
}

impl<'a> From<Outliner<'a>> for Hooks<'a> {
    fn from(outline: Outliner<'a>) -> Self {
        Self { outline, ..Default::default() }
    }
}

/// The affine map that best matches `f` near `p` (finite differences with step `eps`).
pub fn affine_near(f: &dyn Fn(Point) -> Point, p: Point, eps: f64) -> Affine {
    let o = f(p);
    let ex = (f(p + Vec2::new(eps, 0.0)) - f(p - Vec2::new(eps, 0.0))) / (2.0 * eps);
    let ey = (f(p + Vec2::new(0.0, eps)) - f(p - Vec2::new(0.0, eps))) / (2.0 * eps);
    let lin = Affine::new([ex.x, ex.y, ey.x, ey.y, 0.0, 0.0]);
    Affine::translate(o.to_vec2()) * lin * Affine::translate(-p.to_vec2())
}

/// How deep hooks may nest art inside art (symbols in symbols, envelopes in envelopes) before
/// the rest is left as it is: a file can make a symbol contain itself.
const MAX_HOOK_DEPTH: u32 = 16;

/// How an envelope pushes art through its point map.
#[derive(Clone, Copy)]
pub(super) struct Warper<'a> {
    pub(super) f: &'a dyn Fn(Point) -> Point,
    /// Longest piece a segment is cut into before mapping.
    pub(super) piece: f64,
    /// Finite-difference step for the local affine maps gradients follow.
    pub(super) eps: f64,
    /// Rows and columns of the pieces an image is cut into.
    pub(super) cells: usize,
    pub(super) hooks: Hooks<'a>,
    pub(super) options: EnvelopeOptions,
    /// How many hooks the art being mapped came through.
    pub(super) depth: u32,
}

impl Warper<'_> {
    /// This warper one hook deeper (`None` past [`MAX_HOOK_DEPTH`]).
    fn deeper(&self) -> Option<Self> {
        (self.depth < MAX_HOOK_DEPTH).then_some(Self { depth: self.depth + 1, ..*self })
    }
}

/// `to` (art standing in for `from`: its outlines, its art) with `from`'s name, transparency,
/// opacity mask and effects.
fn carry_look(from: &Node, mut to: Node) -> Node {
    to.name = from.name.clone();
    to.opacity = from.opacity;
    to.blend = from.blend;
    to.isolate = from.isolate;
    to.knockout = from.knockout;
    to.knockout_shape = from.knockout_shape;
    to.extras_mut().mask = from.extras().mask.clone();
    to.appearance.effects = from.appearance.effects.clone();
    to
}

pub(super) fn map_node(n: &Node, w: &Warper) -> Node {
    let deeper = w.deeper();
    // Distort Appearance: strokes and geometry effects bend with the art.
    if w.options.distort_appearance
        && let Some(d) = &deeper
        && let Some(o) = w.hooks.appearance.and_then(|h| h(n))
    {
        return map_node(&o, d);
    }
    // Distort Pattern Fills: the tiles bend with the art (not the patterns inside them).
    if w.options.patterns()
        && let Some(d) = &deeper
        && let Some(o) = w.hooks.pattern.and_then(|p| pattern_art(n, p))
    {
        let inner = Warper { options: EnvelopeOptions { distort_pattern_fills: false, ..d.options }, ..*d };
        return map_node(&o, &inner);
    }
    let mut out = n.clone();
    // Distort Linear Gradients: the gradient vectors follow the warp where they sit.
    if w.options.gradients()
        && let Some(b) = n.geometric_bounds()
    {
        out.appearance.warp_linear_gradients(b, &|p| affine_near(w.f, p, w.eps));
    }
    if let Some(m) = out.extras_if_any().and_then(|e| e.mask.as_mut()) {
        m.art = Arc::new(map_node(&m.art, w));
    }
    match &mut out.kind {
        NodeKind::Path { path, live, .. } => {
            *path = map_nonlinear(path, w.piece, w.f);
            *live = None;
        }
        NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } | NodeKind::Compound { children, .. } => {
            for c in children.iter_mut() {
                *c = Arc::new(map_node(c, w));
            }
        }
        NodeKind::Mesh(m) => m.map(w.f),
        NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) | NodeKind::Intertwine { .. } => {
            if let Some(d) = &deeper {
                return map_node(&expanded_group_hooks(n, w.hooks), d);
            }
        }
        NodeKind::Text(_) | NodeKind::SymbolInstance { .. } => {
            if let Some(d) = &deeper
                && let Some(o) = w.hooks.outline.and_then(|h| h(n))
            {
                return map_node(&carry_look(n, o), d);
            }
        }
        NodeKind::Image(im) => {
            let im = im.clone();
            return warp_image(&out, &im, w);
        }
        // Its art is plain, so it warps at this depth.
        NodeKind::PlacedDocument(_) => return map_node(&expanded_group_hooks(n, w.hooks), w),
    }
    out
}

/// The affine map taking triangle `s` to triangle `d` (`None` when `s` is degenerate).
fn affine_between(s: [Point; 3], d: [Point; 3]) -> Option<Affine> {
    let basis = |p: [Point; 3]| Affine::new([p[1].x - p[0].x, p[1].y - p[0].y, p[2].x - p[0].x, p[2].y - p[0].y, p[0].x, p[0].y]);
    let bs = basis(s);
    if bs.determinant().abs() < 1e-12 {
        return None;
    }
    let a = basis(d) * bs.inverse();
    a.is_finite().then_some(a)
}

/// Image `n` (`im`) pushed through the warp as a raster mesh warp: its frame cut into
/// `w.cells`² cells of two triangles, each a clip group showing the image under the affine map
/// that takes the triangle where the warp takes its corners. Each piece reaches a little past its
/// edges (at most half a point) so neighbours overlap and no seams show.
fn warp_image(n: &Node, im: &ImageObject, w: &Warper) -> Node {
    let k = w.cells.max(1);
    // Two triangles a cell, three objects each; an image an inner envelope already cut into
    // pieces is cut again here, so the pieces count against [`MAX_GENERATED`]. Past it, the image
    // is left out.
    if !Generating::charge(k.saturating_mul(k).saturating_mul(6)) {
        let mut g = n.clone();
        g.kind = NodeKind::Group { children: vec![], clip: false };
        return g;
    }
    let (pw, ph) = (im.width as f64, im.height as f64);
    let at = |i: usize, j: usize| Point::new(pw * i as f64 / k as f64, ph * j as f64 / k as f64);
    let mut pieces = Vec::with_capacity(2 * k * k);
    for j in 0..k {
        for i in 0..k {
            let s = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
            let d = s.map(|p| (w.f)(im.xf * p));
            for t in [[0, 1, 2], [0, 2, 3]] {
                let (st, dt) = (t.map(|v| s[v]), t.map(|v| d[v]));
                let Some(xf) = affine_between(st, dt) else { continue };
                let c = Point::new((dt[0].x + dt[1].x + dt[2].x) / 3.0, (dt[0].y + dt[1].y + dt[2].y) / 3.0);
                let grow = (dt[0].distance(dt[1]).max(dt[1].distance(dt[2])) * 0.02).min(0.5);
                let tri: Vec<Point> = dt
                    .iter()
                    .map(|p| {
                        let v = *p - c;
                        let len = v.hypot();
                        if len > 1e-12 { *p + v * (grow / len) } else { *p }
                    })
                    .collect();
                let mut clip = Node::path(n.id, PathData::single(SubPath::polyline(&tri, true)), Appearance::default());
                if let NodeKind::Path { clipping, .. } = &mut clip.kind {
                    *clipping = true;
                }
                let image = Node::new(n.id, NodeKind::Image(Box::new(ImageObject { xf, ..im.clone() })));
                pieces.push(Arc::new(Node::new(n.id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(image)], clip: true })));
            }
        }
    }
    let mut g = n.clone();
    g.kind = NodeKind::Group { children: pieces, clip: false };
    g
}

/// Distort Pattern Fills: path or compound path `n` with each visible pattern fill turned into the
/// tiles it paints in a clip group shaped like `n` (with the fill's opacity and blend mode), its
/// other fills and strokes on copies of it, in paint order, under `n`'s transparency. `None`
/// without a pattern fill whose definition `pattern` finds.
fn pattern_art(n: &Node, tiles: PatternTiles) -> Option<Node> {
    let (path, rule, _) = crate::blend::node_path(n)?;
    let pattern_fill = |i: &AppearanceItem| matches!(i, AppearanceItem::Fill(f) if f.visible && matches!(f.paint, Paint::Pattern { .. }));
    if !n.appearance.items.iter().any(pattern_fill) {
        return None;
    }
    let bounds = path.bounds()?;
    let def = |i: &AppearanceItem| match i {
        AppearanceItem::Fill(f) if f.visible => match &f.paint {
            Paint::Pattern { pattern: name, xf } => tiles(name, *xf, bounds).map(|t| (t, f.opacity, f.blend)),
            _ => None,
        },
        _ => None,
    };
    let plain = |items: Vec<AppearanceItem>| {
        let mut p = Node {
            appearance: Appearance { items, ..Default::default() }.into(),
            opacity: 1.0,
            blend: Default::default(),
            isolate: false,
            knockout: Default::default(),
            knockout_shape: false,
            name: None,
            ..n.clone()
        };
        p.extras_mut().mask = None;
        Arc::new(p)
    };
    let mut pieces = vec![];
    let mut rest = vec![];
    for i in &n.appearance.items {
        let Some((tiles, opacity, blend)) = def(i) else {
            rest.push(i.clone());
            continue;
        };
        if !rest.is_empty() {
            pieces.push(plain(std::mem::take(&mut rest)));
        }
        let mut clip = Node::path(n.id, path.clone(), Appearance::default());
        if let NodeKind::Path { clipping, rule: r, .. } = &mut clip.kind {
            (*clipping, *r) = (true, rule);
        }
        let children = std::iter::once(clip).chain(tiles).map(Arc::new).collect();
        let mut g = Node::new(n.id, NodeKind::Group { children, clip: true });
        g.opacity = opacity;
        g.blend = blend;
        pieces.push(Arc::new(g));
    }
    if !rest.is_empty() {
        pieces.push(plain(rest));
    }
    Some(Node {
        kind: NodeKind::Group { children: pieces, clip: false },
        appearance: Appearance { effects: n.appearance.effects.clone(), ..Default::default() }.into(),
        ..n.clone()
    })
}

// =====================================================================================
// Expansion
// =====================================================================================

/// Is this one of the live kinds?
pub fn is_live(n: &Node) -> bool {
    matches!(
        n.kind,
        NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. }
    )
}

/// Mesh tessellation as filled quad paths (no stroke), `n`×`n` per patch.
pub fn mesh_quad_nodes(m: &GradientMesh, n: usize) -> Vec<Node> {
    m.quads(n)
        .into_iter()
        .map(|q| {
            let mut ap = Appearance::default();
            let mut fl = FillLayer::new(Paint::solid(q.color));
            fl.opacity = q.opacity;
            ap.items.push(AppearanceItem::Fill(fl));
            Node::path(NodeId(0), PathData::single(SubPath::polyline(&q.pts, true)), ap)
        })
        .collect()
}

/// One level of evaluation of a live object (non-live nodes return themselves). Blend steps and
/// envelope results may still contain live objects (e.g. a blend of two meshes).
pub fn expand_live(n: &Node) -> Vec<Node> {
    expand_live_with(n, None)
}

pub fn expand_live_with(n: &Node, outline: Outliner) -> Vec<Node> {
    expand_live_hooks(n, outline.into())
}

/// [`expand_live_with`] with everything an envelope can ask of the crates above (see [`Hooks`]).
pub fn expand_live_hooks(n: &Node, hooks: Hooks) -> Vec<Node> {
    let _generating = Generating::enter();
    match &n.kind {
        NodeKind::Blend { children, spec } => blend_expand(children, spec),
        NodeKind::Envelope { content, kind, fidelity, options, frame, .. } => expand_envelope(content, kind, *fidelity, *options, **frame, hooks),
        NodeKind::Mesh(m) => mesh_quad_nodes(m, 8),
        NodeKind::Repeat(r) => r.expand(hooks.symbols),
        NodeKind::PlacedDocument(p) => p.art(n),
        NodeKind::Intertwine { children } => crate::intertwine::expand(children),
        _ => vec![n.clone()],
    }
}

/// A live object evaluated into a plain group (keeping the object's id, name and transparency).
pub fn expanded_group(n: &Node, outline: Outliner) -> Node {
    expanded_group_hooks(n, outline.into())
}

/// [`expanded_group`] with [`Hooks`].
pub fn expanded_group_hooks(n: &Node, hooks: Hooks) -> Node {
    let _generating = Generating::enter();
    let children = expand_live_hooks(n, hooks).into_iter().map(Arc::new).collect();
    let mut g = Node::new(n.id, NodeKind::Group { children, clip: false });
    g.name = n.name.clone();
    g.visible = n.visible;
    g.locked = n.locked;
    g.opacity = n.opacity;
    g.blend = n.blend;
    g.isolate = n.isolate;
    g.knockout = n.knockout;
    // Knocking out opaque blend steps changes nothing: don't make the outputs work for it.
    if matches!(n.kind, NodeKind::Blend { .. }) && g.knockout == crate::Knockout::On && !crate::blend::knockout_shows(&g) {
        g.knockout = crate::Knockout::Off;
    }
    g.knockout_shape = n.knockout_shape;
    g.extras_mut().mask = n.extras().mask.clone();
    g
}

/// Recursively replace every live object in `n` by plain geometry (for exporters).
pub fn expand_deep(n: &Node, outline: Outliner) -> Node {
    expand_deep_hooks(n, outline.into())
}

/// [`expand_deep`] with [`Hooks`]. The objects live art makes here count against one
/// [`MAX_GENERATED`] ([`Generating`]): blends of blends kept their inner blends live, and each
/// copy expanded on its own, so step counts capped one by one multiplied. Past it, live art
/// expands to nothing.
pub fn expand_deep_hooks(n: &Node, hooks: Hooks) -> Node {
    let _generating = Generating::enter();
    let mut out = if is_live(n) {
        let g = expanded_group_hooks(n, hooks);
        if !Generating::charge(g.count()) {
            return Node::new(n.id, NodeKind::Group { children: vec![], clip: false });
        }
        g
    } else {
        n.clone()
    };
    if let Some(ch) = out.children_mut() {
        for c in ch.iter_mut() {
            if subtree_has_live(c) {
                *c = Arc::new(expand_deep_hooks(c, hooks));
            }
        }
    }
    out
}

/// Most objects one evaluation of live art (an expansion, an envelope's mapping and everything
/// inside them) may make.
pub const MAX_GENERATED: usize = 4_000_000;

std::thread_local! {
    /// What is left of [`MAX_GENERATED`] for the evaluation running on this thread; `None` outside
    /// one.
    static GENERATED_LEFT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// The outermost live evaluation on this thread: it starts [`GENERATED_LEFT`] and ends it.
struct Generating(bool);

impl Generating {
    fn enter() -> Self {
        let outermost = GENERATED_LEFT.get().is_none();
        if outermost {
            GENERATED_LEFT.set(Some(MAX_GENERATED));
        }
        Self(outermost)
    }

    /// Spend `n` objects; `false` (and nothing left) when there aren't that many.
    fn charge(n: usize) -> bool {
        match GENERATED_LEFT.get() {
            None => true,
            Some(left) => {
                GENERATED_LEFT.set(Some(left.saturating_sub(n)));
                n <= left
            }
        }
    }
}

impl Drop for Generating {
    fn drop(&mut self) {
        if self.0 {
            GENERATED_LEFT.set(None);
        }
    }
}

/// Does `n` or any descendant need live evaluation?
pub fn subtree_has_live(n: &Node) -> bool {
    let mut any = false;
    n.walk(&mut |c| any |= is_live(c));
    any
}

/// Largest stroke outset in a subtree (for visual bounds of envelopes). With `symbols`, the
/// outsets in the art of the symbols its instances show count too
/// ([`crate::symbols::ArtBox::reach`]), grown by [`crate::SymbolBoxes::brush_pad`]. In boxes made
/// by [`crate::SymbolBoxes::without_art_strokes`], the outsets in the art do not count, and the pad
/// does.
pub(crate) fn max_outset(nodes: &[Arc<Node>], symbols: Option<&crate::SymbolBoxes>) -> f64 {
    let mut o = 0.0f64;
    for n in nodes {
        n.walk(&mut |c| {
            o = o.max(c.appearance.outset());
            if let (Some(s), NodeKind::SymbolInstance { symbol, .. }) = (symbols, &c.kind)
                && let Some(art) = s.art(symbol)
            {
                let reach = if s.art_strokes() { art.reach } else { 0.0 };
                o = o.max(reach + s.brush_pad(symbol));
            }
        });
    }
    o
}
