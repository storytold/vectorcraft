use super::shape::scale_across;
use super::*;

/// One axis of an object that [`set_bounds`] sizes, square to its frame: the geometric box runs
/// from `lo` to `hi`, and the visual box reaches past each end (before, after) by a margin that
/// scales with the strokes and one that stays.
#[derive(Clone, Copy, Debug)]
pub(super) struct Span {
    pub(super) lo: f64,
    pub(super) hi: f64,
    scaling: [f64; 2],
    kept: [f64; 2],
}

impl Span {
    /// The ends of the visual box after a scale by `s` about `c`, with the margins that scale
    /// grown by `k`. A negative scale turns the box over, and its margins with it.
    pub(super) fn scaled(self, c: f64, s: f64, k: f64) -> (f64, f64) {
        let ([sb, sa], [kb, ka]) = (self.scaling, self.kept);
        let (before, after) = (sb * k + kb, sa * k + ka);
        let (a, b) = (c + s * (self.lo - c), c + s * (self.hi - c));
        if s < 0.0 { (b - after, a + before) } else { (a - before, b + after) }
    }
}

/// The objects that [`set_bounds`] sizes, measured one by one square to its frame.
pub(super) struct Spans {
    x: Vec<Span>,
    y: Vec<Span>,
    /// Whether strokes scale. The margins that scale grow by the square root of the product of the
    /// two scales; with strokes that do not scale, every margin stays.
    strokes: bool,
}

impl Spans {
    /// The objects of `ids` that have a box, measured one by one as [`Document::oriented_bounds`]
    /// measures them square to `frame`, each plain group member by member and each symbol instance
    /// that [`open`] opens object by object in its art. Past [`MAX_SPANS`] parts, each object of
    /// `ids` is measured as one. The margins of the visual box without the strokes in symbols' art
    /// ([`scaling_box`]) scale with the strokes, and the rest stay. With `strokes` false, every
    /// margin stays.
    pub(super) fn measure(d: &Document, ids: &[NodeId], frame: &OrientedBox, strokes: bool) -> Self {
        let (symbols, without, art) = (d.symbol_boxes(), d.symbol_boxes().without_art_strokes(), d.symbol_boxes().quick());
        let to_local = frame.to_doc().inverse();
        let measure = |n: &Node, s: &vectorcraft_doc::SymbolBoxes, visual: bool| match (frame.angle == 0.0, visual) {
            (true, false) => n.geometric_bounds_with(s),
            (true, true) => n.visual_bounds_with(s),
            (false, _) => n.bounds_in(s, to_local, visual),
        };
        let roots: Vec<&Node> = ids.iter().filter_map(|id| d.node(*id)).collect();
        let mut parts = vec![];
        if !roots.iter().all(|n| open(n, 0.0, d, &symbols, to_local, &mut parts)) {
            parts = roots.iter().map(|n| Part::Object(n, 0.0)).collect();
        }
        let mut out = Self { x: vec![], y: vec![], strokes };
        for part in parts {
            let (g, v, vs) = match part {
                Part::Object(n, grow) => {
                    let Some(g) = measure(n, &symbols, false) else { continue };
                    let v = measure(n, &symbols, true).unwrap_or(g).inflate(grow, grow);
                    let vs = if strokes { measure(n, &without, true).unwrap_or(g).inflate(grow, grow) } else { g };
                    (g, v, vs)
                }
                // Measured as the instance's own box measures its art (`Node::visual_bounds_with`):
                // the strokes at their own weight, and the instances in it by their art boxes.
                Part::Art(n, xf, grow, kept) => {
                    let mut placed = n.clone();
                    placed.transform(xf, false);
                    let Some(g) = placed.geometric_bounds_with(&art) else { continue };
                    let v = placed.visual_bounds_with(&art).map_or(g, |v| v.union(g)).inflate(kept + grow, kept + grow);
                    let vs = if strokes { g.inflate(grow, grow) } else { g };
                    (g, v, vs)
                }
            };
            out.x.push(Span { lo: g.x0, hi: g.x1, scaling: [g.x0 - vs.x0, vs.x1 - g.x1], kept: [vs.x0 - v.x0, v.x1 - vs.x1] });
            out.y.push(Span { lo: g.y0, hi: g.y1, scaling: [g.y0 - vs.y0, vs.y1 - g.y1], kept: [vs.y0 - v.y0, v.y1 - vs.y1] });
        }
        out
    }

    /// The visual box after a scale by `sx`, `sy` about `c`.
    fn visual(&self, c: Point, sx: f64, sy: f64) -> Option<Rect> {
        let k = if self.strokes { (sx * sy).abs().sqrt() } else { 1.0 };
        let boxes = self.x.iter().zip(&self.y).map(|(x, y)| {
            let ((x0, x1), (y0, y1)) = (x.scaled(c.x, sx, k), y.scaled(c.y, sy, k));
            Rect::new(x0, y0, x1, y1)
        });
        boxes.reduce(|a, b| a.union(b))
    }

    /// The scales along x and y that give the visual box the size `size` with a scale about `c`,
    /// or come as near to it as the margins that stay allow ([`Self::solve`]), and the point that
    /// goes to the reference point's place: point `refi` of the visual box they give. Returns
    /// `None`, which keeps `closed` (the scales and the point of [`set_bounds`]'s closed form), when
    /// the scales of `closed` put point `refi` at the point of `closed` and either give the size
    /// `size` or are the scales [`Self::solve`] finds, to within rounding (1e-12 of the largest
    /// value compared, or of 1).
    pub(super) fn refit(&self, c: Point, refi: usize, size: [f64; 2], closed: (f64, f64, Point)) -> Option<(f64, f64, Point)> {
        let ([w, h], (sx, sy, at)) = (size, closed);
        let b = self.visual(c, sx, sy)?;
        let p = vectorcraft_geom::reference_point(b, refi);
        let tolerance = 1e-12 * [b.x0, b.y0, b.x1, b.y1, w, h, at.x, at.y].iter().fold(1.0, |m: f64, v| m.max(v.abs()));
        let near = |a: f64, b: f64| (a - b).abs() <= tolerance;
        let placed = near(p.x, at.x) && near(p.y, at.y);
        if placed && near(b.width(), w) && near(b.height(), h) {
            return None;
        }
        let (tx, ty) = self.solve(w, h);
        let same = |a: f64, b: f64| (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1.0);
        if placed && same(tx, sx) && same(ty, sy) {
            return None;
        }
        let b = self.visual(c, tx, ty)?;
        Some((tx, ty, vectorcraft_geom::reference_point(b, refi)))
    }

    /// The scales of 0 or more along x and y that give the visual box the size `w` × `h`, each
    /// found by [`scale_across`]. With strokes that scale, the margins that scale grow by `k`,
    /// found where it is the square root of the product of the two scales.
    fn solve(&self, w: f64, h: f64) -> (f64, f64) {
        let at = |k: f64| (scale_across(&self.x, k, w), scale_across(&self.y, k, h));
        if !self.strokes {
            return at(1.0);
        }
        // The scales get smaller as `k` grows, so the square root of their product minus `k` is at
        // least 0 at `k` 0 and at most 0 at `k` equal to that root. Bisection finds the `k` between
        // the two where it is 0.
        let (sx, sy) = at(0.0);
        let (mut lo, mut hi) = (0.0, (sx * sy).sqrt());
        if hi.is_nan() || hi <= 0.0 {
            return (sx, sy);
        }
        for _ in 0..200 {
            let mid = lo + (hi - lo) / 2.0;
            if mid <= lo || mid >= hi {
                break;
            }
            let (sx, sy) = at(mid);
            if (sx * sy).sqrt() > mid {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let miss = |k: f64| {
            let (sx, sy) = at(k);
            ((sx * sy).sqrt() - k).abs()
        };
        at(if miss(lo) < miss(hi) { lo } else { hi })
    }
}

/// Most parts [`Spans::measure`] opens the objects it sizes into: past it, each object is one part.
const MAX_SPANS: usize = 4096;

/// What [`Spans::measure`] measures as one span.
enum Part<'a> {
    /// An object of the selection, or a member of a plain group in it, and the outset of the
    /// strokes of the plain groups around it, which scale with its own.
    Object(&'a Node, f64),
    /// An object in the art of a symbol instance and the transform that places it square to the
    /// frame. The outset of the strokes of the instance and of the plain groups around the instance
    /// scales with the strokes; the outset of the strokes of the plain groups around the object in
    /// the art stays, as the strokes in the art do.
    Art(&'a Node, Affine, f64, f64),
}

/// Adds the parts that [`Spans::measure`] measures `n` by to `parts`, `grow` being the outset of
/// the strokes of the plain groups around it ([`Part::Object`]). A plain group ([`opens`]) adds the
/// parts of each member. A symbol instance without effects or an opacity mask of its own adds the
/// objects of its art, with plain groups there opened too ([`Part::Art`]), when its symbol's art
/// measures it, it is not in perspective, and its transform keeps the art square to the frame (a
/// turned or slanted instance is measured by the box around its turned art box). Anything else is
/// one part. Returns false when the parts pass [`MAX_SPANS`].
fn open<'a>(n: &'a Node, grow: f64, d: &'a Document, symbols: &vectorcraft_doc::SymbolBoxes, to_local: Affine, parts: &mut Vec<Part<'a>>) -> bool {
    if opens(n) {
        match &n.kind {
            NodeKind::Group { children, .. } => {
                let grow = grow + n.appearance.outset();
                return children.iter().all(|c| open(c, grow, d, symbols, to_local, parts));
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                let xf = to_local * *xf;
                if n.projection().is_none()
                    && square(xf)
                    && symbols.art_bounds(symbol).is_some()
                    && let Some(art) = d.symbols.iter().find(|s| s.name == *symbol)
                {
                    return open_art(&art.art, xf, grow + n.appearance.outset(), 0.0, parts);
                }
            }
            _ => {}
        }
    }
    parts.push(Part::Object(n, grow));
    parts.len() <= MAX_SPANS
}

/// [`open`] for `n` in symbol art that `xf` places square to the frame: a plain group adds the parts
/// of each member, and anything else, a symbol instance included, is one part. `kept` is the outset
/// of the strokes of the plain groups around `n` in the art, and `grow` that of the instance and the
/// plain groups around it.
fn open_art<'a>(n: &'a Node, xf: Affine, grow: f64, kept: f64, parts: &mut Vec<Part<'a>>) -> bool {
    if let NodeKind::Group { children, .. } = &n.kind
        && opens(n)
    {
        let kept = kept + n.appearance.outset();
        return children.iter().all(|c| open_art(c, xf, grow, kept, parts));
    }
    parts.push(Part::Art(n, xf, grow, kept));
    parts.len() <= MAX_SPANS
}

/// Whether [`open`] opens `n`: a group without a clip, a Shaper construction, effects or an opacity
/// mask of its own, or a symbol instance without effects or an opacity mask of its own.
fn opens(n: &Node) -> bool {
    let visible = |effects: &[vectorcraft_doc::Effect]| effects.iter().any(|e| e.visible);
    let effects = visible(&n.appearance.effects) || n.appearance.items.iter().any(|i| visible(i.effects()));
    let kind = match n.kind {
        NodeKind::Group { clip, .. } => !clip && n.extras().shaper.is_none(),
        NodeKind::SymbolInstance { .. } => true,
        _ => false,
    };
    kind && !effects && n.extras().mask.is_none()
}

/// Whether `a` takes boxes square to the frame to boxes square to it: it scales, mirrors, turns by a
/// quarter turn and moves, and nothing else, to within rounding.
fn square(a: Affine) -> bool {
    let [m0, m1, m2, m3, ..] = a.as_coeffs();
    let tolerance = 1e-9 * [m0, m1, m2, m3].iter().fold(0.0, |m: f64, v| m.max(v.abs()));
    m1.abs() <= tolerance && m2.abs() <= tolerance || m0.abs() <= tolerance && m3.abs() <= tolerance
}
