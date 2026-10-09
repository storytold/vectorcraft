//! Object → Expand: live shapes, type and effects (Object), strokes (Stroke) and gradient fills
//! (Fill) into plain art in one undo step. Placed documents (Object) become editable copies of the
//! art they show, their resources joining the document. A linear or radial gradient fill becomes a
//! gradient mesh, or solid strips / concentric ellipses, inside a clip group shaped like the object; a freeform
//! gradient becomes a mesh shaped like the object. Create Gradient Mesh shares the sampling.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::cms::Model;
use vectorcraft_color::freeform::spread_scale;
use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::appearance::{AppearanceItem, FillLayer};
use vectorcraft_doc::live::{GradientMesh, lerp_color};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, PathData, Point, Rect, SubPath, Vec2, shapes};

use super::edit::selected_roots;
use super::menucmds::{run_raw, squash};
use super::object::as_clipping_path;
use super::pathops::{assemble, node_path, replace_leaves, styled_copy, with_transparency};
use super::*;

const C: &str = "object.expand";
/// Expand Gradient To → Specify … Objects: the default and the most.
pub const DEFAULT_STEPS: u32 = 255;
pub const MAX_STEPS: u32 = 1000;
/// Rows and columns of the mesh a freeform gradient expands to.
const FREEFORM_MESH: u32 = 8;
/// Columns (around the centre) of the mesh a radial gradient expands to.
const RADIAL_MESH_COLS: usize = 8;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.expand",
            "Expand…",
            ["Object"],
            None,
            "{object?: true (type → outlines, live shapes → paths, effects baked, blends → groups of their steps, envelopes → their distorted content, placed documents → editable copies of their art, as placing the file without link gives, its symbols, patterns, swatches and images joining the document), fill?: true (gradient fills → `gradient` art), stroke?: true (strokes → filled outlines), gradient?: \"objects\" (default: `steps` solid strips, concentric ellipses for radial gradients) | \"mesh\" (a gradient mesh), both in a clip group shaped like the object (freeform gradients always become a mesh shaped like it), steps?: 1..1000 (255)} one undo step → {ids}",
            has_selection,
            expand
        ),
        cmd!(
            query "object.expand.info",
            "Expand Options",
            [],
            None,
            "{} what each Expand option would change in the selection → {object, fill, stroke} (true: something to expand)",
            has_selection,
            |s, _| {
                let e = Expandable::of(&s.doc()?.doc, &selected_roots(s)?);
                Ok(json!({"object": e.object(), "fill": e.fill, "stroke": e.stroke}))
            }
        ),
    ]
}

/// Every placed document in `roots` of `d` (at any depth) replaced by an editable copy of the art it
/// shows, keeping its id and place; the resources the art uses join `d` (a name `d` uses for
/// something else gets a free variant).
fn expand_placed_under(d: &mut Document, roots: &[NodeId]) -> Result<()> {
    let mut ids = vec![];
    for n in roots.iter().filter_map(|r| d.node(*r)) {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::PlacedDocument(_)) {
                ids.push(c.id);
            }
        });
    }
    ids.into_iter().try_for_each(|id| super::place::document::expand(d, id, C))
}

/// Expand Gradient To.
#[derive(Clone, Copy, Debug, PartialEq)]
enum GradientTo {
    Mesh,
    Objects(u32),
}

impl GradientTo {
    fn parse(p: &Value) -> Result<Self> {
        let steps = f64_or(p, "steps", DEFAULT_STEPS as f64).round();
        if !(1.0..=MAX_STEPS as f64).contains(&steps) {
            return Err(bad(C, format!("steps must be 1..{MAX_STEPS}")));
        }
        match str_param(p, "gradient").unwrap_or("objects") {
            "objects" => Ok(Self::Objects(steps as u32)),
            "mesh" => Ok(Self::Mesh),
            _ => Err(bad(C, "gradient must be \"objects\" or \"mesh\"")),
        }
    }
}

/// What the selection holds for each Expand option.
#[derive(Default)]
struct Expandable {
    text: bool,
    live: bool,
    effects: bool,
    fill: bool,
    stroke: bool,
    /// An envelope (Object expands it into its distorted content).
    envelope: bool,
    /// Live blends (Object expands them into groups of their steps).
    blends: bool,
    /// Placed documents (Object expands them into the art they show).
    placed: bool,
    /// Compound shapes (Object expands them into their outline).
    compound_shapes: bool,
}

impl Expandable {
    fn of(d: &Document, roots: &[NodeId]) -> Self {
        let mut e = Self::default();
        for n in roots.iter().filter_map(|r| d.node(*r)) {
            n.walk(&mut |c| {
                let shape = matches!(c.kind, NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::CompoundShape { .. });
                e.text |= matches!(c.kind, NodeKind::Text(_));
                e.live |= matches!(c.kind, NodeKind::Path { live: Some(_), .. });
                e.effects |= !c.appearance.effects.is_empty();
                e.envelope |= matches!(c.kind, NodeKind::Envelope { .. });
                e.blends |= matches!(c.kind, NodeKind::Blend { .. });
                e.placed |= matches!(c.kind, NodeKind::PlacedDocument(_));
                e.compound_shapes |= matches!(c.kind, NodeKind::CompoundShape { .. });
                e.stroke |= shape && !c.appearance.stroke_paint().is_none();
                e.fill |= match &c.kind {
                    NodeKind::Text(t) => t.runs.iter().any(|r| matches!(r.style.fill, Paint::Gradient(_))),
                    _ => shape && c.appearance.items.iter().any(|i| gradient_fill(i).is_some()),
                };
            });
        }
        e
    }

    fn object(&self) -> bool {
        self.text || self.live || self.effects || self.envelope || self.blends || self.placed || self.compound_shapes
    }
}

/// The gradient of a visible gradient fill.
fn gradient_fill(i: &AppearanceItem) -> Option<(&FillLayer, &GradientPaint)> {
    match i {
        AppearanceItem::Fill(f) if f.visible => match &f.paint {
            Paint::Gradient(g) => Some((f, g)),
            _ => None,
        },
        _ => None,
    }
}

fn expand(s: &mut Session, p: &Value) -> Result<Value> {
    let object = bool_or(p, "object", true);
    let fill = bool_or(p, "fill", true);
    let stroke = bool_or(p, "stroke", true);
    let to = GradientTo::parse(p)?;
    let from = s.doc()?.history.undo.len();
    let rev0 = s.doc()?.revision;
    let e = Expandable::of(&s.doc()?.doc, &selected_roots(s)?);
    // Placed documents first: their art (type, strokes, gradients) expands below.
    if object && e.placed {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| expand_placed_under(d, &roots))?;
    }
    // Blends first: what their steps hold (type, effects, strokes, gradients) expands below.
    if object && e.blends {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| super::live::expand_blends(d, &roots).map(|_| ()))?;
    }
    // Compound shapes: their outline (its effects, strokes and gradients expand below).
    if object && e.compound_shapes {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| super::compoundshape::expand_under(d, &roots))?;
    }
    if object && e.effects {
        let _ = run_raw(s, "effect.expandAppearance", &json!({}));
    }
    if object && e.envelope {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| {
            for r in &roots {
                super::live::expand_envelopes_under(d, *r)?;
            }
            Ok(())
        })?;
    }
    if object && e.live {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| {
            for r in &roots {
                if let Some(n) = d.node_mut(*r) {
                    drop_live(n);
                }
            }
            Ok(())
        })?;
    }
    if object && e.text {
        let _ = run_raw(s, "type.createOutlines", &json!({}));
    }
    if stroke && e.stroke {
        let _ = run_raw(s, "object.path.outlineStroke", &json!({}));
    }
    if fill {
        let _ = expand_gradient_fills(s, to);
    }
    if s.doc()?.revision == rev0 {
        return Err(EngineError::Other("Expand: nothing to expand".into()));
    }
    squash(s, from, "Expand");
    Ok(json!({ "ids": s.doc()?.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn drop_live(n: &mut Node) {
    if let NodeKind::Path { live, .. } = &mut n.kind {
        *live = None;
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            drop_live(Arc::make_mut(c));
        }
    }
}

/// Replace every gradient fill of the selected paths by art (see [`expand_fills`]).
fn expand_gradient_fills(s: &mut Session, to: GradientTo) -> Result<()> {
    let roots = selected_roots(s)?;
    s.edit("Expand", |d, sel| {
        let (mut ids, mut changed) = (vec![], 0);
        for r in &roots {
            let (id, n) = replace_leaves(d, *r, |d, l| expand_fills(d, l, to))?;
            ids.push(id);
            changed += n;
        }
        if changed == 0 {
            return Err(EngineError::Other("Expand: no gradient fills".into()));
        }
        sel.set(ids);
        Ok(())
    })
}

/// Path `l` with each gradient fill turned into art, in appearance order: the other items stay
/// on plain copies of the path. `None` when it has no gradient fill.
fn expand_fills(d: &mut Document, l: &Node, to: GradientTo) -> Option<Node> {
    if !matches!(l.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) || !l.appearance.items.iter().any(|i| gradient_fill(i).is_some()) {
        return None;
    }
    let (path, _) = node_path(l)?;
    let bounds = path.bounds()?;
    let mut parts = vec![];
    let mut rest = vec![];
    for item in &l.appearance.items {
        // Items that paint nothing go; a gradient that can't be expanded stays.
        let Some(art) = gradient_fill(item).and_then(|(f, g)| gradient_art(d, l, &path, bounds, f, g, to)) else {
            if item.visible() && !item.paint().is_none() {
                rest.push(item.clone());
            }
            continue;
        };
        if !rest.is_empty() {
            parts.push(styled_copy(d, l, std::mem::take(&mut rest)));
        }
        parts.push(art);
    }
    if !rest.is_empty() {
        parts.push(styled_copy(d, l, rest));
    }
    assemble(d, l, parts)
}

/// Gradient fill `f` (gradient `g`) of path `l` (outline `path`, box `bounds`) as art, with the
/// fill's opacity, blend mode and effects.
fn gradient_art(d: &mut Document, l: &Node, path: &PathData, bounds: Rect, f: &FillLayer, g: &GradientPaint, to: GradientTo) -> Option<Node> {
    let content = if g.gradient.kind == GradientKind::Freeform {
        let m = GradientMesh::for_path_with(path, FREEFORM_MESH, FREEFORM_MESH, &*gradient_sampler(g, bounds))?;
        vec![Node::new(d.alloc_id(), NodeKind::Mesh(m))]
    } else {
        let frame = Frame::new(g, bounds);
        match to {
            GradientTo::Mesh => vec![Node::new(d.alloc_id(), NodeKind::Mesh(frame.mesh(&g.gradient)))],
            GradientTo::Objects(n) => frame
                .pieces(&g.gradient, n as usize)
                .into_iter()
                .map(|(shape, (color, opacity))| {
                    let fill = FillLayer { opacity, overprint: f.overprint, ..FillLayer::new(Paint::solid(color)) };
                    Node::path(d.alloc_id(), shape, Appearance { items: vec![AppearanceItem::Fill(fill)], ..Default::default() })
                })
                .collect(),
        }
    };
    // A freeform mesh takes the object's outline: only holes need the object to clip it.
    let art = if g.gradient.kind == GradientKind::Freeform && path.subpaths.len() == 1 {
        content.into_iter().next()?
    } else {
        let mut clip = styled_copy(d, l, vec![]);
        as_clipping_path(&mut clip).ok()?;
        let children = std::iter::once(clip).chain(content).map(Arc::new).collect();
        Node::new(d.alloc_id(), NodeKind::Group { children, clip: true })
    };
    let mut art = with_transparency(d, art, f.opacity, f.blend);
    art.appearance.effects = f.effects.clone();
    Some(art)
}

/// The colour and opacity gradient fill `g` paints at a document point of an object whose box is
/// `bounds`, as the canvas draws it (the end colours continue past the ends). Colours stay in the
/// colour model their stops (or freeform points) share; display RGB where models differ.
pub(crate) fn gradient_sampler(g: &GradientPaint, bounds: Rect) -> Box<dyn Fn(Point) -> (Color, f32) + '_> {
    if g.gradient.kind == GradientKind::Freeform {
        let b = bounds.abs();
        let f = g.freeform_on(b);
        let model = shared_model(f.points.iter().map(|p| p.color));
        let field = f.field(spread_scale(b));
        return Box::new(move |p| {
            let ([r, g, b], a) = field.sample(p);
            let c = Color::rgb(r, g, b);
            (model.map_or(c, |m| c.in_model(m)), a)
        });
    }
    let geom = g.resolve(bounds);
    Box::new(move |p| g.gradient.sample_with(geom.param_at(g.gradient.kind, p) as f32, lerp_color))
}

/// The colour model all of `colors` share.
fn shared_model(mut colors: impl Iterator<Item = Color>) -> Option<Model> {
    let m = colors.next()?.model();
    colors.all(|c| c.model() == m).then_some(m)
}

/// Where the colour changes along a gradient between parameters `t0` and `t1`: both ends, every
/// stop between them (in its own colour, so coincident stops keep a sharp edge) and every midpoint
/// off the centre, as (parameter, colour, opacity) in order, with more between them where they
/// are over `max_gap` apart. The colour is linear in display RGB between neighbours, as the
/// canvas draws it.
fn breaks(g: &Gradient, t0: f64, t1: f64, max_gap: f64) -> Vec<(f64, (Color, f32))> {
    let at = |t: f64| (t, g.sample_with(t as f32, lerp_color));
    let mut v = vec![at(t0)];
    for (i, s) in g.stops.iter().enumerate() {
        let o = s.offset as f64;
        if o > t0 && o < t1 {
            v.push((o, (s.color, s.opacity)));
        }
        if let Some(n) = g.stops.get(i + 1)
            && (s.midpoint - 0.5).abs() > 1e-3
        {
            let m = (s.offset + (n.offset - s.offset) * s.midpoint) as f64;
            if m > t0 && m < t1 {
                v.push(at(m));
            }
        }
    }
    v.push(at(t1));
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::with_capacity(v.len());
    for w in v.windows(2) {
        out.push(w[0]);
        let (a, b) = (w[0].0, w[1].0);
        let n = ((b - a) / max_gap).ceil() as usize;
        out.extend((1..n).map(|k| at(a + (b - a) * k as f64 / n as f64)));
    }
    out.extend(v.last().copied());
    out
}

/// A linear or radial gradient's geometry over an object's box: its start, the vector to its
/// end, the vector across it (radial: the ellipse's other axis), and the parameter ranges that
/// cover the box (linear: along the vector `t0..t1` and across it `s0..s1` in points; radial:
/// `0..t1`, each level's ellipse centred on the way from the focal point to the start).
struct Frame {
    radial: bool,
    start: Point,
    focal: Point,
    along: Vec2,
    across: Vec2,
    t0: f64,
    t1: f64,
    s0: f64,
    s1: f64,
}

impl Frame {
    fn new(g: &GradientPaint, bounds: Rect) -> Self {
        let geom = g.resolve(bounds);
        let radial = g.gradient.kind == GradientKind::Radial;
        // A vector of no length paints as one pointing right (as the canvas does).
        let along = if geom.start.distance(geom.end) < 1e-9 { Vec2::new(1.0, 0.0) } else { geom.end - geom.start };
        let normal = Vec2::new(-along.y, along.x) / along.hypot();
        let across = if radial { normal * along.hypot() * geom.aspect } else { normal };
        let corners = [bounds.origin(), Point::new(bounds.x1, bounds.y0), Point::new(bounds.x1, bounds.y1), Point::new(bounds.x0, bounds.y1)];
        let range = |f: &dyn Fn(Point) -> f64| corners.iter().map(|c| f(*c)).fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
        let (mut t0, mut t1) = range(&|c| (c - geom.start).dot(along) / along.hypot2());
        let (s0, s1) = range(&|c| (c - geom.start).dot(normal));
        if radial {
            let geom = vectorcraft_color::GradientGeom { end: geom.start + along, ..geom };
            (t0, t1) = (0.0, range(&|c| geom.param_at(GradientKind::Radial, c)).1);
        }
        // Keep a sliver of span so the art maps back invertibly.
        t1 = t1.max(t0 + 1e-6);
        Self { radial, start: geom.start, focal: geom.focal_point(), along, across, t0, t1, s0, s1 }
    }

    /// Linear: the point at parameter `t` along and `s` across. Radial: the point at parameter `t`
    /// and angle `s` (radians).
    fn at(&self, t: f64, s: f64) -> Point {
        if self.radial {
            self.focal + (self.start - self.focal + self.along * s.cos() + self.across * s.sin()) * t
        } else {
            self.start + self.along * t + self.across * s
        }
    }

    /// A gradient mesh covering the box: columns (linear) or rings (radial) where the colour
    /// changes, so it paints the gradient exactly.
    fn mesh(&self, g: &Gradient) -> GradientMesh {
        // Rings as close as the columns around them, so the mesh draws smoothly.
        let gap = if self.radial { (self.t1 - self.t0) / RADIAL_MESH_COLS as f64 } else { f64::INFINITY };
        let br = breaks(g, self.t0, self.t1, gap);
        let ts: Vec<f64> = br.iter().map(|(t, _)| (t - self.t0) / (self.t1 - self.t0)).collect();
        let t = |x: f64| self.t0 + x * (self.t1 - self.t0);
        if self.radial {
            let angles: Vec<f64> = (0..=RADIAL_MESH_COLS).map(|i| i as f64 / RADIAL_MESH_COLS as f64).collect();
            GradientMesh::from_grid(&angles, &ts, &|u, v| self.at(t(v), u * std::f64::consts::TAU), &|r, _, _| br[r].1)
        } else {
            GradientMesh::from_grid(&ts, &[0.0, 1.0], &|u, v| self.at(t(u), self.s0 + v * (self.s1 - self.s0)), &|_, c, _| br[c].1)
        }
    }

    /// `n` solid shapes covering the box in paint order, each coloured as the gradient in the
    /// middle of its band: for a linear gradient, rectangles from each band's start to the far
    /// end, for a radial one, nested ellipses (largest first). Each paints over the ones
    /// below except its band, so no seam shows between bands. With translucent stops each shape
    /// is just its band, so nothing doubles up.
    fn pieces(&self, g: &Gradient, n: usize) -> Vec<(PathData, (Color, f32))> {
        let step = (self.t1 - self.t0) / n as f64;
        let color = |k: usize| g.sample_with((self.t0 + (k as f64 + 0.5) * step) as f32, lerp_color);
        let opaque = g.stops.iter().all(|s| s.opacity >= 1.0);
        if self.radial {
            let ellipse = |t: f64| {
                let c = self.focal + (self.start - self.focal) * t;
                let m = Affine::new([self.along.x * t, self.along.y * t, self.across.x * t, self.across.y * t, c.x, c.y]);
                shapes::ellipse(Rect::new(-1.0, -1.0, 1.0, 1.0)).transformed(m)
            };
            return (0..n)
                .rev()
                .map(|k| {
                    let mut shape = ellipse((k + 1) as f64 * step);
                    if !opaque && k > 0 {
                        let mut hole = ellipse(k as f64 * step).subpaths.remove(0);
                        hole.reverse();
                        shape.subpaths.push(hole);
                    }
                    (shape, color(k))
                })
                .collect();
        }
        (0..n)
            .map(|k| {
                let a = self.t0 + k as f64 * step;
                let b = if opaque { self.t1 } else { a + step };
                let pts = [self.at(a, self.s0), self.at(b, self.s0), self.at(b, self.s1), self.at(a, self.s1)];
                (PathData::single(SubPath::polyline(&pts, true)), color(k))
            })
            .collect()
    }
}
