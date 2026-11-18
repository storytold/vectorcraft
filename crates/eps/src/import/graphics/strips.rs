//! Gradients a program paints as a row of abutting strips, each in one colour (the way many
//! PostScript writers draw a smooth shading at Level 2, which has no `shfill`), read as one
//! linear gradient.
//!
//! Drawn one by one, abutting strips leave seams: a renderer that anti-aliases each fill lets the
//! art below show through along every shared edge (PDF 32000-1:2008 §10.6.4 notes the effect,
//! "conflation", for adjacent areas painted separately), which reads as thin stripes across the
//! gradient, and as a tint of the colour below over the whole of it where the strips are only a
//! pixel or two wide. A run of them under the same clips becomes one object filled with a linear
//! gradient through the strips' colours, which has no seams and stays editable.
//!
//! Only strips that clearly are a gradient merge: at least [`MIN_STRIPS`] parallelograms of one
//! length, side by side in one direction with no gap or overlap, each a small step in colour from
//! the one before ([`MAX_STEP`]). Abutting blocks of distinct colours (a bar chart, a row of
//! swatches) stay as they are.

use super::{
    Affine, Appearance, AppearanceItem, BezPath, Color, Drawn, FillLayer, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, HashSet,
    MAX_STITCH_STOPS, Node, NodeId, NodeKind, Paint, PathData, PathEl, Point, Vec2,
};

/// Fewest strips (that paint something) a gradient is read from.
const MIN_STRIPS: usize = 6;
/// Largest change of any colour component (0 to 1) from one strip to the next.
const MAX_STEP: f32 = 0.08;
/// How far apart corners may be and still meet (points).
const EPS: f64 = 0.01;
/// Largest change of any colour component a stop left out may make to the gradient.
const TOLERANCE: f32 = 0.5 / 255.0;

/// A filled parallelogram in one process colour.
struct Strip {
    corners: [Point; 4],
    model: u8,
    comps: [f32; 4],
    color: Color,
    overprint: bool,
}

/// The parallelogram `n` fills, in one process colour: `None` for anything else.
fn strip_of(n: &Node) -> Option<Strip> {
    let NodeKind::Path { path, live: None, clipping: false, guide: false, .. } = &n.kind else { return None };
    if n.appearance.items.len() != 1 {
        return None;
    }
    let Some(AppearanceItem::Fill(f)) = n.appearance.items.first() else { return None };
    if f.opacity != 1.0 || !f.visible || !f.effects.is_empty() || f.blend != vectorcraft_color::BlendMode::Normal {
        return None;
    }
    let Paint::Solid { color, swatch: None, .. } = &f.paint else { return None };
    let (model, comps) = match *color {
        Color::Rgb { r, g, b } => (0, [r, g, b, 0.0]),
        Color::Cmyk { c, m, y, k } => (1, [c, m, y, k]),
        Color::Gray { k } => (2, [k, 0.0, 0.0, 0.0]),
        Color::Lab { .. } => return None,
    };
    let [sp] = path.subpaths.as_slice() else { return None };
    let mut pts: Vec<Point> = Vec::with_capacity(5);
    for a in &sp.anchors {
        if a.h_in != a.p || a.h_out != a.p {
            return None;
        }
        pts.push(a.p);
    }
    if pts.len() == 5 && pts.first().zip(pts.last()).is_some_and(|(a, b)| a.distance(*b) <= EPS) {
        pts.pop();
    }
    let [a, b, c, d] = pts.as_slice() else { return None };
    // Opposite corners share a midpoint.
    if (a.to_vec2() + c.to_vec2() - b.to_vec2() - d.to_vec2()).hypot() > EPS {
        return None;
    }
    Some(Strip { corners: [*a, *b, *c, *d], model, comps, color: *color, overprint: f.overprint })
}

/// The edge two strips share, as it runs along the first: `(k, from, to)`, from its corner `k`.
fn shared_edge(a: &Strip, b: &Strip) -> Option<(usize, Point, Point)> {
    let near = |p: Point, q: Point| p.distance(q) <= EPS;
    for k in 0..4 {
        let (p, q) = (a.corners[k], a.corners[(k + 1) % 4]);
        if p.distance(q) <= EPS {
            continue;
        }
        for j in 0..4 {
            let (r, s) = (b.corners[j], b.corners[(j + 1) % 4]);
            if (near(p, r) && near(q, s)) || (near(p, s) && near(q, r)) {
                return Some((k, p, q));
            }
        }
    }
    None
}

/// Coordinates along a run of strips: a point is `o + s·w + t·v`, `v` along the strips (their
/// shared edges run from `t` = 0 to 1), `w` across them.
struct Axes {
    o: Point,
    w: Vec2,
    v: Vec2,
    inv: Affine,
}

impl Axes {
    fn of(a: &Strip, b: &Strip) -> Option<Self> {
        let (k, from, to) = shared_edge(a, b)?;
        let w = a.corners[(k + 2) % 4] - a.corners[(k + 1) % 4];
        let v = to - from;
        let m = Affine::new([w.x, w.y, v.x, v.y, from.x, from.y]);
        if w.hypot() <= EPS || m.determinant().abs() < 1e-9 * w.hypot() * v.hypot() {
            return None;
        }
        Some(Self { o: from, w, v, inv: m.inverse() })
    }

    fn at(&self, s: f64, t: f64) -> Point {
        self.o + self.w * s + self.v * t
    }

    /// Does `p` lie within the run's length, and across it from `lo` to `hi`?
    fn holds(&self, p: Point, lo: f64, hi: f64) -> bool {
        let (ts, ss) = (EPS / self.v.hypot(), EPS / self.w.hypot());
        let l = self.inv * p;
        l.x >= lo - ss && l.x <= hi + ss && l.y >= -ts && l.y <= 1.0 + ts
    }

    /// The strip's extent across the run (`s` from, to), when it spans the run's length exactly.
    fn band(&self, st: &Strip) -> Option<(f64, f64)> {
        let (ts, ss) = (EPS / self.v.hypot(), EPS / self.w.hypot());
        let local: Vec<Point> = st.corners.iter().map(|p| self.inv * *p).collect();
        let lo = local.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let hi = local.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let fits = local
            .iter()
            .all(|p| ((p.y).abs() <= ts || (p.y - 1.0).abs() <= ts) && ((p.x - lo).abs() <= ss || (p.x - hi).abs() <= ss) && p.x.is_finite());
        // Each end of the strip touches both edges of the run.
        let ends = [0.0, 1.0].iter().all(|t| local.iter().filter(|p| (p.y - t).abs() <= ts).count() == 2);
        (fits && ends).then_some((lo, hi))
    }
}

/// A run of strips found at one place in what was drawn: where it ends (exclusive), the object
/// it becomes, and whether that object covers the clip around it (so the clip path filled with
/// the gradient paints no more than the strips did).
struct Run {
    start: usize,
    end: usize,
    node: Node,
    covers_clip: bool,
}

/// Is `s` (from, to) too thin to paint anything?
fn thin(b: (f64, f64), axes: &Axes) -> bool {
    (b.1 - b.0) * axes.w.hypot() <= EPS
}

/// The run of strips starting at `i`, if one does.
fn run_at(drawn: &Drawn, strips: &[Option<Strip>], i: usize) -> Option<Run> {
    let chain = |k: usize| drawn.get(k).map(|(c, _)| c.iter().map(|c| c.id).collect::<Vec<_>>());
    let here = chain(i)?;
    let strip = |k: usize| strips.get(k).and_then(Option::as_ref).filter(|_| chain(k).as_ref() == Some(&here));
    let first = strip(i)?;
    // The run's axes come from its first two strips that paint something; a line before them
    // (a strip of no width, as some writers draw first) joins the run where it lies on its edge.
    let (lead, a, b) = match (strip(i + 1), strip(i + 2)) {
        (Some(b), _) if Axes::of(first, b).is_some_and(|ax| ax.band(first).is_some_and(|f| !thin(f, &ax))) => (None, first, b),
        (Some(a), Some(b)) => (Some(first), a, b),
        _ => return None,
    };
    let axes = Axes::of(a, b)?;
    let (ba, bb) = (axes.band(a)?, axes.band(b)?);
    if thin(ba, &axes) || thin(bb, &axes) {
        return None;
    }
    let ss = EPS / axes.w.hypot();
    let forward = if (bb.0 - ba.1).abs() <= ss {
        true
    } else if (bb.1 - ba.0).abs() <= ss {
        false
    } else {
        return None;
    };
    let mut edge = if forward { ba.0 } else { ba.1 };
    if let Some(l) = lead {
        let lb = axes.band(l)?;
        if !thin(lb, &axes) || (lb.0 - edge).abs() > ss {
            return None;
        }
    }
    let at = i + usize::from(lead.is_some());
    let mut bands: Vec<(f64, f64, &Strip)> = vec![];
    let mut end = at;
    while bands.len() < MAX_STITCH_STOPS {
        let Some(st) = strip(end) else { break };
        let Some(band) = axes.band(st) else { break };
        let meets = if forward { (band.0 - edge).abs() <= ss } else { (band.1 - edge).abs() <= ss };
        if !meets || st.overprint != a.overprint {
            break;
        }
        if !thin(band, &axes) {
            if let Some((_, _, prev)) = bands.last()
                && (prev.model != st.model || prev.comps.iter().zip(&st.comps).any(|(x, y)| (x - y).abs() > MAX_STEP))
            {
                break;
            }
            bands.push((band.0, band.1, st));
        }
        edge = if forward { band.1 } else { band.0 };
        end += 1;
    }
    if bands.len() < MIN_STRIPS {
        return None;
    }
    let lo = bands.iter().map(|b| b.0).fold(f64::INFINITY, f64::min);
    let hi = bands.iter().map(|b| b.1).fold(f64::NEG_INFINITY, f64::max);
    if !(hi - lo).is_finite() || hi - lo <= 0.0 {
        return None;
    }
    bands.sort_by(|x, y| x.0.total_cmp(&y.0));
    let stops: Vec<(f32, &Strip)> = bands.iter().map(|(s0, s1, st)| ((((s0 + s1) / 2.0 - lo) / (hi - lo)) as f32, *st)).collect();
    let stops: Vec<GradientStop> = simplify(&stops).into_iter().map(|(t, st)| GradientStop::new(t, st.color)).collect();
    // The gradient runs across the strips, square to them, from the run's edge at `lo`.
    let len = axes.v.hypot();
    let mut n = Vec2::new(-axes.v.y / len, axes.v.x / len);
    let across = axes.w.dot(n) * (hi - lo);
    if across < 0.0 {
        n = -n;
    }
    let start = axes.at(lo, 0.0);
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    g.geom = Some(GradientGeom { start, end: start + n * across.abs(), aspect: 1.0, focal: None });
    let mut bp = BezPath::new();
    bp.move_to(axes.at(lo, 0.0));
    bp.line_to(axes.at(hi, 0.0));
    bp.line_to(axes.at(hi, 1.0));
    bp.line_to(axes.at(lo, 1.0));
    bp.close_path();
    let item = AppearanceItem::Fill(FillLayer { overprint: a.overprint, ..FillLayer::new(Paint::Gradient(Box::new(g))) });
    let (clips, first_node) = drawn.get(i)?;
    // Every point of a clip's path, control points included, inside the run: its curves are too
    // (each lies in the hull of its points), and the run's area is convex.
    let covers_clip = clips.last().and_then(|c| c.region.as_ref()).is_some_and(|(clip, _)| {
        clip.elements().iter().all(|el| match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => axes.holds(p, lo, hi),
            PathEl::QuadTo(a, p) => axes.holds(a, lo, hi) && axes.holds(p, lo, hi),
            PathEl::CurveTo(a, b, p) => axes.holds(a, lo, hi) && axes.holds(b, lo, hi) && axes.holds(p, lo, hi),
            PathEl::ClosePath => true,
        })
    });
    let node = Node::path(first_node.id, PathData::from_bezpath(&bp), Appearance { items: vec![item], ..Appearance::default() });
    Some(Run { start: i, end, node, covers_clip })
}

/// The stops `stops` (offset, strip) needs: those the gradient between the stops kept on either
/// side wouldn't come within [`TOLERANCE`] of are kept.
fn simplify<'a>(stops: &[(f32, &'a Strip)]) -> Vec<(f32, &'a Strip)> {
    let mut kept: Vec<(f32, &Strip)> = vec![];
    let Some(&first) = stops.first() else { return kept };
    kept.push(first);
    let mut from = 0;
    while from + 1 < stops.len() {
        // The furthest stop reachable from `from` with every stop between on the line.
        let mut to = from + 1;
        while to + 1 < stops.len() && on_line(stops, from, to + 1) {
            to += 1;
        }
        if let Some(&s) = stops.get(to) {
            kept.push(s);
        }
        from = to;
    }
    kept
}

/// Do the stops between `a` and `b` lie on the blend between those two?
fn on_line(stops: &[(f32, &Strip)], a: usize, b: usize) -> bool {
    let (Some((ta, sa)), Some((tb, sb))) = (stops.get(a), stops.get(b)) else { return false };
    let span = tb - ta;
    stops.get(a + 1..b).unwrap_or_default().iter().all(|(t, s)| {
        let f = if span > 0.0 { (t - ta) / span } else { 0.0 };
        sa.comps.iter().zip(&sb.comps).zip(&s.comps).all(|((x, y), z)| (x + (y - x) * f - z).abs() <= TOLERANCE)
    })
}

/// Runs of strips in `drawn` become one gradient-filled object each; one that covers its clip is
/// marked in `shadings`, so a clipping group of nothing else is its clip path filled with the
/// gradient (a clip larger than the strips keeps clipping the gradient to them).
pub(crate) fn merge_strips(drawn: &mut Drawn, shadings: &mut HashSet<NodeId>) {
    let strips: Vec<Option<Strip>> = drawn.iter().map(|(_, n)| strip_of(n)).collect();
    if strips.iter().filter(|s| s.is_some()).count() < MIN_STRIPS {
        return;
    }
    let mut runs: Vec<Run> = vec![];
    let mut i = 0;
    while i < drawn.len() {
        match run_at(drawn, &strips, i) {
            Some(r) => {
                i = r.end.max(i + 1);
                runs.push(r);
            }
            None => i += 1,
        }
    }
    if runs.is_empty() {
        return;
    }
    let old = std::mem::take(drawn);
    let mut runs = runs.into_iter().peekable();
    let mut skip_to = 0;
    for (k, item) in old.into_iter().enumerate() {
        if k < skip_to {
            continue;
        }
        if let Some(r) = runs.next_if(|r| r.start == k) {
            skip_to = r.end;
            if r.covers_clip {
                shadings.insert(r.node.id);
            }
            drawn.push((item.0, r.node));
            continue;
        }
        drawn.push(item);
    }
}
