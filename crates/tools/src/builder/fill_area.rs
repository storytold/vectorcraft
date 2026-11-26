//! The Fill Area tool: a click inside an area that visible, unlocked paths enclose adds a new
//! closed path of that area with the current fill and stroke (`shapeBuilder.copyRegion`). The
//! paths around the area stay as they are, and nothing needs to be selected.

use vectorcraft_geom::path::segment_bounds;
use vectorcraft_geom::{ParamCurve, PathSeg};

use super::lookup::path_contains;
use super::*;

/// The area [`AreaFinder::find`] finds under a point.
#[derive(Clone, Debug, PartialEq)]
pub struct EnclosedArea {
    /// The area's outline, as a Shape Builder merge of that one region makes it.
    pub path: PathData,
    /// The object directly in a layer that holds the front-most path along the area (its outline
    /// touches the area's outline, or it covers the area), or in isolation mode the object directly
    /// in the isolated object: the new path goes directly above it, so it never goes inside a group,
    /// a Live Paint group, a Shaper group or a clipping group other than the isolated one.
    pub above: NodeId,
}

/// How many times [`AreaFinder::find`] widens the part of the document it maps before it maps
/// every path.
const WINDOW_ROUNDS: usize = 24;

/// The side (pt) of the first window [`AreaFinder::find`] maps around a point that no path's bounds
/// hold.
const FIRST_WINDOW: f64 = 32.0;

/// How many times [`AreaFinder::find`] makes the window four times larger when the paths in it hold
/// no area under the point, before it maps every path: a 32 pt window reaches 8,192 pt.
const NEAR_ROUNDS: usize = 4;

/// Finds the area under a point among the paths of some objects, mapping only the paths near the
/// point first: an area that lies inside a window is bounded by the paths whose bounds reach the
/// window, so the paths outside it cannot change it. When the maps of [`NEAR_ROUNDS`] larger windows
/// have no area under the point, or the area does not fit after [`WINDOW_ROUNDS`] widenings, the
/// finder maps every path. When a window's paths have more segments than one Shape Builder map
/// takes, the paths with the most segments are left out until the rest fit, and an area is taken
/// only when no path left out crosses it or comes within the gap of it; a path left out that
/// covers it still counts for where the new path goes. When every path together has more segments than that,
/// an area no window finds is not found. The map of every path, once a search has needed it, is
/// kept for the next search.
pub struct AreaFinder {
    /// The inputs back → front, as [`shape_builder_shapes`] gives them, with their bounds and the
    /// object directly in a layer that holds each one ([`outermost`]).
    inputs: Vec<(po::Shape, Rect, NodeId)>,
    /// The bounds of every input.
    total: Option<Rect>,
    gap: f64,
    /// The segments of all the inputs.
    segments: usize,
    /// Whether all the inputs fit the segment limit of one Shape Builder map. Past it a map keeps
    /// only the filled areas and loses the lines between them, so no map of every input is made.
    fits: bool,
    /// The map of every input, with the index of each of its shapes in `inputs`.
    full: Option<(BuilderMap, Vec<usize>)>,
}

/// Does `r` hold `p`, edges included?
fn holds(r: &Rect, p: Point) -> bool {
    p.x >= r.x0 && p.x <= r.x1 && p.y >= r.y0 && p.y <= r.y1
}

/// Do `a` and `b` overlap or touch?
fn meet(a: &Rect, b: &Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// The area a path left out of a window's map may cut: its outline, the same as a closed path
/// (`bez`), and the box (`zone`) its outline reaches with the gap.
struct Reach<'a> {
    face: &'a PathData,
    bez: &'a vectorcraft_geom::kurbo::BezPath,
    zone: Rect,
    /// How close (pt) to the outline a path may come and still change the area: the gap and 1 pt.
    reach: f64,
}

/// Does a segment of `path` (closing segments included) lie inside the area or come within its
/// reach of the area's outline? A path in the box of the area may still lie wholly outside the
/// area, in the empty corner of an L or around it, and such a path does not change it. `budget`
/// counts the work (pieces checked times the outline's segments); when it runs out, the path
/// counts as cutting the area.
fn cuts(path: &PathData, area: &Reach, budget: &mut usize) -> bool {
    let bez = path.to_bezpath();
    let cost = area.face.subpaths.iter().map(|sp| sp.segment_count()).sum::<usize>().max(1);
    vectorcraft_geom::kurbo::segments(bez.elements().iter().copied()).any(|seg| segment_cuts(seg, area, cost, budget, 0))
}

/// How many times [`segment_cuts`] halves a segment: a piece of 1/2^24 of a segment near the area
/// counts as cutting it.
const REACH_SPLITS: u32 = 24;

/// The side (pt) below which a piece of a segment that may come within reach of the area counts as
/// cutting it.
const REACH_TOLERANCE: f64 = 0.01;

/// How much work [`cuts`] does for one area before it takes a path as cutting it: pieces checked
/// times the segments of the area's outline.
const CUT_BUDGET: usize = 1 << 24;

/// Does `seg` lie inside the area or come within its reach of the area's outline? A piece whose
/// box misses the area's zone does not; a piece whose middle is inside the area or within reach of
/// its outline does; a piece farther from the outline than its box is wide does not, since every
/// point of it is then out of reach and on the same side. The rest is halved.
fn segment_cuts(seg: PathSeg, area: &Reach, cost: usize, budget: &mut usize, depth: u32) -> bool {
    let b = segment_bounds(seg);
    if !meet(&b, &area.zone) {
        return false;
    }
    let Some(left) = budget.checked_sub(cost) else { return true };
    *budget = left;
    let mid = seg.eval(0.5);
    if area.bez.winding(mid) != 0 {
        return true;
    }
    let Some((.., d)) = area.face.nearest(mid) else { return true };
    if d <= area.reach {
        return true;
    }
    let wide = b.width().hypot(b.height());
    if d - area.reach > wide {
        return false;
    }
    if depth >= REACH_SPLITS || wide < REACH_TOLERANCE {
        return true;
    }
    segment_cuts(seg.subsegment(0.0..0.5), area, cost, budget, depth + 1) || segment_cuts(seg.subsegment(0.5..1.0), area, cost, budget, depth + 1)
}

/// How far (pt) an anchor of an area's outline may lie from a path's outline, with Gap Detection
/// off, and still count as on it: the least snap distance [`outline_snap`] gives, five times the
/// 0.01 pt precision the Shape Builder's map works to.
const CONTACT: f64 = 0.05;

/// Does the filled area of the closed subpaths of `s` hold `p`?
fn covers(s: &po::Shape, p: Point) -> bool {
    let closed = PathData::new(s.path.subpaths.iter().filter(|sp| sp.closed).cloned().collect());
    path_contains(&closed, s.rule, p)
}

/// Does `outer` hold all of `inner`?
fn within(inner: &Rect, outer: &Rect) -> bool {
    inner.x0 >= outer.x0 && inner.x1 <= outer.x1 && inner.y0 >= outer.y0 && inner.y1 <= outer.y1
}

/// The outlines of the clipping masks [`unmasked`] has read, by the clipping group or clipped
/// layer: None for one whose mask clips to nothing, which hides all it clips.
type Masks = std::collections::HashMap<NodeId, Option<(PathData, FillRule)>>;

/// `shape`, the outline of `leaf`, cut to where every clipping mask around `leaf` shows it
/// ([`clip_to`]). A path in a mask is not cut by that mask. None when the masks hide all of it.
fn unmasked(doc: &Document, leaf: &Node, mut shape: po::Shape, masks: &mut Masks) -> Option<po::Shape> {
    let Some(line) = doc.ancestry(leaf.id) else { return Some(shape) };
    let filled = leaf.appearance.has_drawn_fill();
    for a in line.iter().filter(|a| **a != leaf.id) {
        let Some(n) = doc.node(*a).filter(|n| n.clips()) else { continue };
        let Some(mask) = n.children().and_then(|c| c.first()) else { continue };
        if line.contains(&mask.id) {
            continue;
        }
        let outline = masks.entry(*a).or_insert_with(|| {
            vectorcraft_effects::clip_outline(mask).map(|(bp, rule)| (PathData::from_bezpath(&bp), rule)).filter(|(p, _)| !p.is_empty())
        });
        let (m, rule) = outline.as_ref()?;
        shape = clip_to(shape, filled, m, *rule)?;
    }
    Some(shape)
}

/// `shape` cut to `mask` (filled with `rule`). With `filled` (the path's fill draws), its closed
/// subpaths are cut to their overlap with the mask, whose edge then bounds the fill. Its other
/// subpaths, and every subpath of a path without a fill, are cut to their pieces inside the mask,
/// so a hidden part of an outline does not close an area. A subpath with more segments or
/// crossings than [`po::addons::pathtools::crossings_on`] takes is left out. None when nothing of
/// it is inside.
fn clip_to(shape: po::Shape, filled: bool, mask: &PathData, rule: FillRule) -> Option<po::Shape> {
    let (b, mb) = (shape.path.bounds()?, mask.bounds()?);
    if !meet(&b, &mb) {
        return None;
    }
    let (closed, lines): (Vec<SubPath>, Vec<SubPath>) = shape.path.subpaths.into_iter().partition(|sp| filled && sp.closed);
    let had_closed = !closed.is_empty();
    let mut subs = if had_closed { po::boolean(&PathData::new(closed), shape.rule, mask, rule, po::BoolOp::Intersect).subpaths } else { vec![] };
    for sp in lines {
        let one = PathData::new(vec![sp]);
        // A subpath whose crossings with the mask are past what can be found is left out, so no
        // part of it the mask hides bounds an area.
        let Some(crossed) = po::addons::pathtools::crossings_on(&one, &[mask]) else { continue };
        let places: Vec<(usize, f64)> = crossed.into_iter().map(|(_, s, t)| (s, t)).collect();
        let Some(sp) = one.subpaths.first() else { continue };
        let (cut, at) = po::addons::pathtools::insert_at(sp, &places);
        subs.extend(cut.cut_at(&at).into_iter().filter(|piece| {
            let n = piece.segment_count();
            n > 0 && path_contains(mask, rule, piece.segment(n / 2).eval(0.5))
        }));
    }
    if subs.is_empty() {
        return None;
    }
    let rule = if had_closed { FillRule::NonZero } else { shape.rule };
    Some(po::Shape::new(PathData::new(subs), rule, shape.key))
}

/// The object directly in `scope` (the isolated object in isolation mode), or else directly in a
/// layer (or sublayer), that holds `id`: `id` itself when `scope` or a layer holds it, or when `id`
/// is `scope`.
fn outermost(doc: &Document, id: NodeId, scope: Option<NodeId>) -> NodeId {
    let Some(line) = doc.ancestry(id) else { return id };
    let k = match scope {
        Some(s) => line.iter().rposition(|a| *a == s),
        None => line.iter().rposition(|a| doc.node(*a).is_some_and(|n| n.is_layer())),
    };
    k.and_then(|k| line.get(k + 1)).copied().unwrap_or(id)
}

/// Does `n` make, move or combine the art in it itself, so that a new path in it would not stay as
/// Fill Area adds it: a Live Paint group, a compound shape, a Shaper group, a graph, an Image Trace
/// result, a dimension, a flare or an Objects on Path group.
fn managed(n: &Node) -> bool {
    let e = n.extras();
    super::lookup::is_live_paint(n)
        || n.is_compound_shape()
        || e.shaper.is_some()
        || e.graph.is_some()
        || e.trace.is_some()
        || e.dimension.is_some()
        || e.flare.is_some()
        || e.objects_on_path.is_some()
}

/// The roots the Fill Area tool and `shapeBuilder.copyRegion` find areas among, with the isolated
/// object the new path goes into ([`AreaFinder::new`]'s `scope`), for `isolation` (the document's
/// isolated object) and `named` (the objects or layers named; None: every layer). In isolation mode
/// only the isolated layer or group's paths count: of `named`, the ones inside it, with the
/// isolated object in place of one that holds it. An isolated object that no longer exists counts
/// as no isolation. None while an object that is not a layer or a group is isolated (a compound
/// path or a single path), a layer or group with a live effect, a group that is [`managed`], a
/// clipping group or clipped layer, or one inside such an object or inside another object that is
/// not a layer or a group (an envelope): no area is found then.
pub fn fill_area_roots(doc: &Document, isolation: Option<NodeId>, named: Option<&[NodeId]>) -> Option<(Vec<NodeId>, Option<NodeId>)> {
    let Some(iso) = isolation.filter(|i| doc.node(*i).is_some()) else {
        return Some((named.map_or_else(|| every_layer(doc), <[NodeId]>::to_vec), None));
    };
    let line = doc.ancestry(iso)?;
    // Every object from the layer down to the isolated one is a layer or a plain group: an
    // envelope, a blend or another object that draws its contents its own way does not hold
    // the new path as drawn.
    // A layer or group with a live effect (a Transform effect moves what is in it) draws the new
    // path elsewhere than the area.
    let plain =
        |a: &Node| (a.is_layer() || (matches!(a.kind, NodeKind::Group { .. }) && !managed(a))) && !a.appearance.effects.iter().any(|e| e.visible);
    if !line.iter().all(|a| doc.node(*a).is_some_and(plain)) {
        return None;
    }
    // A clipping group or clipped layer's mask clips and is not painted, and its mask hides or cuts
    // a new path among the art it clips.
    if line.iter().any(|a| doc.node(*a).is_some_and(Node::clips)) {
        return None;
    }
    let Some(named) = named else { return Some((vec![iso], Some(iso))) };
    let mut out = vec![];
    for r in named {
        if line.contains(r) {
            out.push(iso);
        } else if doc.ancestry(*r).is_some_and(|a| a.contains(&iso)) {
            out.push(*r);
        }
    }
    Some((out, Some(iso)))
}

impl AreaFinder {
    /// The finder over the paths of `roots` (layers or objects) that the Shape Builder takes:
    /// visible, unlocked paths, compound paths and Live Paint faces and edges that draw, not on a
    /// locked artboard. A path a clipping mask clips counts only where the mask shows it
    /// ([`unmasked`]). Gaps up to `gap` close, and with `open_filled_as_closed` a filled open path
    /// bounds an area as if an invisible edge closed it. With `scope` (the isolated object in
    /// isolation mode), an area's new path goes directly above the object in `scope` that holds the
    /// front-most path along it, so it stays inside the isolated object.
    pub fn new(doc: &Document, roots: &[NodeId], scope: Option<NodeId>, gap: f64, open_filled_as_closed: bool) -> Self {
        let roots = sorted_roots(doc, roots);
        let (shapes, leaves) = shape_builder_shapes(doc, &roots, open_filled_as_closed);
        let editable = doc.editable();
        let mut masks = Masks::new();
        let inputs: Vec<(po::Shape, Rect, NodeId)> = shapes
            .into_iter()
            .zip(leaves)
            .filter(|(_, l)| editable(l.id))
            .filter_map(|(s, l)| {
                let s = unmasked(doc, l, s, &mut masks)?;
                let b = s.path.bounds()?;
                Some((s, b, outermost(doc, l.id, scope)))
            })
            .collect();
        let total = inputs.iter().map(|(_, b, _)| *b).reduce(|a, b| a.union(b));
        let gap = if gap.is_finite() { gap.clamp(0.0, SHAPE_BUILDER_MAX_GAP) } else { 0.0 };
        let segments: usize = inputs.iter().flat_map(|(s, ..)| &s.path.subpaths).map(|sp| sp.segment_count()).sum();
        Self { inputs, total, gap, segments, fits: segments <= po::SHAPE_BUILDER_MAX_SEGMENTS, full: None }
    }

    /// The area under `p`: the smallest region of the map of the paths that holds `p`. None when
    /// `p` is in no enclosed area.
    pub fn find(&mut self, p: Point) -> Option<EnclosedArea> {
        if let Some((map, picked)) = &self.full {
            return self.area_in(map, picked, &[], p);
        }
        let total = self.total?;
        // A gap bridge reaches at most `gap` past a path, so a window takes in every path within
        // that of it.
        let reach = self.gap + 1.0;
        if !holds(&total.inflate(reach, reach), p) {
            return None;
        }
        let mut window = self
            .inputs
            .iter()
            .filter(|(_, b, _)| holds(&b.inflate(reach, reach), p))
            .map(|(_, b, _)| *b)
            .reduce(|a, b| a.union(b))
            .unwrap_or_else(|| Rect::from_center_size(p, (FIRST_WINDOW, FIRST_WINDOW)));
        let mut near = 0;
        let mut every_path = false;
        // The paths left out of the windows' maps to keep them within the segment limit, by index.
        let mut left_out = vec![false; self.inputs.len()];
        let mut any_left_out = false;
        for round in 0..=WINDOW_ROUNDS {
            let last = every_path || round == WINDOW_ROUNDS;
            if last && !self.fits {
                return None;
            }
            let zone = window.inflate(reach, reach);
            let mut picked: Vec<usize> = (0..self.inputs.len())
                .filter(|&i| last || (!left_out.get(i).is_some_and(|o| *o) && self.inputs.get(i).is_some_and(|(_, b, _)| meet(b, &zone))))
                .collect();
            // Past the segment limit a map keeps only the filled areas, so the paths with the most
            // segments stay out of the window's map.
            if !last {
                for i in self.over_limit(&picked) {
                    if let Some(o) = left_out.get_mut(i) {
                        *o = true;
                        any_left_out = true;
                    }
                }
                picked.retain(|&i| !left_out.get(i).is_some_and(|o| *o));
            }
            let every = picked.len() == self.inputs.len();
            let shapes: Vec<po::Shape> = picked.iter().filter_map(|&i| self.inputs.get(i).map(|(s, ..)| s.clone())).collect();
            let map = BuilderMap::of_shapes(shapes, false, self.gap);
            let face = map.region_at(p).and_then(|k| map.regions.get(k));
            let bounds = face.map(|(.., rb)| *rb);
            if every {
                let area = self.area_in(&map, &picked, &[], p);
                self.full = Some((map, picked));
                return area;
            }
            let before = window;
            match bounds {
                // A path left out that crosses the area or comes within its reach could cut it, and
                // only the map of every path holds both.
                Some(rb) if within(&rb, &window) => {
                    let reached = rb.inflate(reach, reach);
                    let mut budget = CUT_BUDGET;
                    let cut = any_left_out
                        && face.is_none_or(|(region, bez, _)| {
                            let area = Reach { face: &region.path, bez, zone: reached, reach };
                            self.inputs.iter().zip(&left_out).any(|((s, b, _), out)| *out && meet(b, &reached) && cuts(&s.path, &area, &mut budget))
                        });
                    if !cut {
                        return self.area_in(&map, &picked, &left_out, p);
                    }
                    every_path = true;
                }
                Some(rb) => window = window.union(rb),
                // No area of the paths near the point holds it: an area that does is bounded by
                // paths farther away. The window grows four times over a few rounds, and then the
                // next round maps every path.
                None if near < NEAR_ROUNDS => {
                    near += 1;
                    let (w, h) = (window.width().max(FIRST_WINDOW) * 4.0, window.height().max(FIRST_WINDOW) * 4.0);
                    window = window.union(Rect::from_center_size(window.center(), (w, h)));
                }
                None => window = total,
            }
            // A window that did not grow would give the same map again.
            if window == before {
                every_path = true;
            }
        }
        None
    }

    /// The inputs among `picked` to leave out of a map so that the rest fit the segment limit of
    /// one Shape Builder map: the ones with the most segments first. Empty when all of `picked` fit.
    fn over_limit(&self, picked: &[usize]) -> Vec<usize> {
        let segments = |i: usize| self.inputs.get(i).map_or(0, |(s, ..)| s.path.subpaths.iter().map(|sp| sp.segment_count()).sum::<usize>());
        let mut sum: usize = picked.iter().map(|&i| segments(i)).sum();
        let mut by_size: Vec<(usize, usize)> = picked.iter().map(|&i| (segments(i), i)).collect();
        by_size.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
        let mut out = vec![];
        for (n, i) in by_size {
            if sum <= po::SHAPE_BUILDER_MAX_SEGMENTS {
                break;
            }
            sum -= n;
            out.push(i);
        }
        out
    }

    /// The area under `p` in `map`, made of the inputs `picked` (in its order). `left_out` marks
    /// the inputs left out of the map, by index, none of which comes near the area: the front-most
    /// of them that covers the area decides where the new path goes when it is in front of the
    /// shape the map gives.
    fn area_in(&self, map: &BuilderMap, picked: &[usize], left_out: &[bool], p: Point) -> Option<EnclosedArea> {
        let (region, ..) = map.region_at(p).and_then(|k| map.regions.get(k))?;
        let path = po::FaceMerger::with_gap(&map.shapes, self.gap).merge(&[region]);
        if path.is_empty() {
            return None;
        }
        let anchors: Vec<Point> = region.path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)).collect();
        // The front-most shape that covers the area or that an anchor of its outline lies on. A
        // shape within the gap of an anchor counts only when no shape is along the area itself,
        // so a path near the area that bounds none of it does not decide where the new path goes.
        // Without a gap, an anchor is on a shape's outline within [`CONTACT`].
        let along = |gap: f64| {
            map.shapes.iter().enumerate().rev().find(|(i, s)| {
                let tol = if gap > 0.0 { outline_snap(&s.path) + gap } else { CONTACT };
                region.sources.contains(i)
                    || s.path.bounds().is_some_and(|b| {
                        let b = b.inflate(tol, tol);
                        anchors.iter().any(|q| holds(&b, *q) && s.path.nearest(*q).is_some_and(|(.., d)| d <= tol))
                    })
            })
        };
        let k = along(0.0).or_else(|| along(self.gap)).map_or(map.shapes.len().checked_sub(1)?, |(i, _)| i);
        let mapped = *picked.get(k)?;
        // A path left out comes near no part of the area, so it covers all of it or none of it.
        let covering =
            left_out.iter().enumerate().rev().find(|(i, out)| **out && *i > mapped && self.inputs.get(*i).is_some_and(|(s, ..)| covers(s, p)));
        let front = covering.map_or(mapped, |(i, _)| i);
        let above = self.inputs.get(front).map(|(.., id)| *id)?;
        Some(EnclosedArea { path, above })
    }
}

/// The paths the Fill Area tool finds areas among: every layer's.
pub fn every_layer(doc: &Document) -> Vec<NodeId> {
    doc.layers.iter().map(|l| l.id).collect()
}

/// Fill Area (in the Live Paint Bucket's group): a click inside an area that visible, unlocked
/// paths on any layer enclose adds a new closed path of that area with the current fill and stroke
/// (`shapeBuilder.copyRegion`), directly above the front-most object along it. The paths stay as
/// they are and nothing needs to be selected. The area under the pointer is outlined while it
/// hovers. Gap Detection and Consider Open Filled Paths as Closed are the Shape Builder's options.
#[derive(Default)]
pub struct FillAreaTool {
    gap_detection: bool,
    gap_length: f64,
    /// Consider Open Filled Paths as Closed turned off: a filled open path cuts areas as a line
    /// does.
    open_filled_cuts: bool,
    /// The finder for the document as it is, with the key it was made for. Away while a search
    /// runs on another thread.
    finder: Option<(FinderKey, AreaFinder)>,
    /// The area under the pointer, with the key of the finder that found it.
    hover: Option<(FinderKey, EnclosedArea)>,
    /// Where the pointer last moved over the canvas.
    at: Option<Point>,
    /// A hover search running on another thread.
    job: Option<HoverJob>,
    /// The key whose search on another thread failed: hovering does not search it again.
    failed: Option<FinderKey>,
    /// A press added the path as a preview, which the release keeps.
    pressing: bool,
}

/// The document revision, the gap, the open-filled rule and the isolated object an [`AreaFinder`]
/// was made with.
type FinderKey = ((u64, u64), f64, bool, Option<NodeId>);

/// A search for the area under `p` on another thread, which hands back the finder it took.
struct HoverJob {
    key: FinderKey,
    p: Point,
    done: std::sync::mpsc::Receiver<(AreaFinder, Option<EnclosedArea>)>,
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// The hover searches this thread has started on other threads ([`hover_searches_started`]).
    static HOVER_SEARCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many hover searches this thread has started on other threads, for tests that count them.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(super) fn hover_searches_started() -> usize {
    HOVER_SEARCHES.with(std::cell::Cell::get)
}

/// Runs `finder.find(p)` on another thread; gives the finder back when no thread starts.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_find(finder: AreaFinder, p: Point) -> Result<std::sync::mpsc::Receiver<(AreaFinder, Option<EnclosedArea>)>, Box<AreaFinder>> {
    HOVER_SEARCHES.with(|n| n.set(n.get() + 1));
    let (tx, done) = std::sync::mpsc::channel();
    let (back_tx, back) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("vectorcraft-fill-area".into()).spawn(move || {
        // The finder comes in through a channel, so a thread that does not start leaves it here.
        if let Ok(mut f) = back.recv() {
            let area = AreaFinder::find(&mut f, p);
            // The tool may have dropped the receiver meanwhile (another tool, a new revision).
            let _ = tx.send((f, area));
        }
    });
    match spawned {
        Ok(_) => match back_tx.send(finder) {
            Ok(()) => Ok(done),
            Err(e) => Err(Box::new(e.0)),
        },
        Err(_) => Err(Box::new(finder)),
    }
}

/// The web build has no threads.
#[cfg(target_arch = "wasm32")]
fn spawn_find(finder: AreaFinder, _p: Point) -> Result<std::sync::mpsc::Receiver<(AreaFinder, Option<EnclosedArea>)>, Box<AreaFinder>> {
    Err(Box::new(finder))
}

impl FillAreaTool {
    /// A tool with the Shape Builder's default options.
    pub fn new() -> Self {
        Self { gap_length: GAP_LENGTHS[0], ..Self::default() }
    }

    fn gap(&self) -> f64 {
        if self.gap_detection { self.gap_length } else { 0.0 }
    }

    fn open_filled_as_closed(&self) -> bool {
        !self.open_filled_cuts
    }

    fn finder_key(&self, cx: &ToolContext) -> FinderKey {
        (cx.revision, self.gap(), self.open_filled_as_closed(), cx.isolation)
    }

    /// The finder for the document as `cx` has it, made again when the key changed. In isolation
    /// mode it takes only the isolated object's paths, as the command does.
    fn take_finder(&mut self, cx: &ToolContext) -> AreaFinder {
        let key = self.finder_key(cx);
        match self.finder.take() {
            Some((k, f)) if k == key => f,
            _ => {
                // An isolated object Fill Area does not work in gives a finder of no paths.
                let (roots, scope) = fill_area_roots(cx.doc, key.3, None).unwrap_or_default();
                AreaFinder::new(cx.doc, &roots, scope, key.1, key.2)
            }
        }
    }

    /// The hover area when it is current and holds `p`.
    fn hover_holding(&self, cx: &ToolContext, p: Point) -> Option<&EnclosedArea> {
        self.hover.as_ref().filter(|(k, a)| *k == self.finder_key(cx) && path_contains(&a.path, FillRule::NonZero, p)).map(|(_, a)| a)
    }

    /// The area under `p`, found here and now (a click). A search running on another thread is
    /// waited for first, so its finder is not made again; no search starts for where the pointer
    /// moved meanwhile.
    fn area_at(&mut self, cx: &ToolContext, p: Point) -> Option<EnclosedArea> {
        if let Some(a) = self.hover_holding(cx, p) {
            return Some(a.clone());
        }
        self.settle(cx, true, false);
        let key = self.finder_key(cx);
        let mut finder = self.take_finder(cx);
        let area = finder.find(p);
        self.finder = Some((key, finder));
        area
    }

    /// Looks for the area under `p` for the hover. A search of more than [`BACKGROUND_SEGMENTS`]
    /// segments runs on another thread, and [`Tool::poll`] takes its result; while one runs, a new
    /// position waits for it.
    fn hover_at(&mut self, cx: &ToolContext, p: Point) {
        self.at = Some(p);
        if self.hover_holding(cx, p).is_some() {
            return;
        }
        // The outline of an area the pointer left goes at once, while the next search runs.
        self.hover = None;
        if self.job.is_some() {
            return;
        }
        let key = self.finder_key(cx);
        if self.failed == Some(key) {
            self.hover = None;
            return;
        }
        let mut finder = self.take_finder(cx);
        if finder.segments > BACKGROUND_SEGMENTS {
            match spawn_find(finder, p) {
                Ok(done) => {
                    self.job = Some(HoverJob { key, p, done });
                    return;
                }
                Err(f) => finder = *f,
            }
        }
        self.hover = finder.find(p).map(|a| (key, a));
        self.finder = Some((key, finder));
    }

    /// Takes the result of the hover search on another thread when it is done (`wait`: waits for
    /// it), and with `follow` starts the search for where the pointer moved meanwhile.
    fn settle(&mut self, cx: &ToolContext, wait: bool, follow: bool) {
        let Some(job) = self.job.take() else { return };
        let got = if wait {
            job.done.recv().ok()
        } else {
            match job.done.try_recv() {
                Ok(v) => Some(v),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.job = Some(job);
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
            }
        };
        let Some((finder, area)) = got else {
            // The search failed (a bug in the planar map): hovering does not search it again, and
            // a click searches here, which reports the failure.
            self.failed = Some(job.key);
            self.hover = None;
            return;
        };
        let current = job.key == self.finder_key(cx);
        if current {
            self.finder = Some((job.key, finder));
        }
        // The area found serves wherever the pointer moved meanwhile, as long as it holds it.
        let at = self.at;
        self.hover = area.filter(|a| current && at.is_some_and(|p| path_contains(&a.path, FillRule::NonZero, p))).map(|a| (job.key, a));
        if follow
            && let Some(p) = at
            && self.hover.is_none()
            && (p != job.p || !current)
        {
            self.hover_at(cx, p);
        }
    }
}

impl Tool for FillAreaTool {
    fn id(&self) -> &'static str {
        "fillArea"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Move => {
                self.hover_at(cx, ev.pos);
                vec![]
            }
            PointerKind::Down => {
                // A click in no enclosed area does nothing.
                let found = self.area_at(cx, ev.pos).is_some();
                self.hover = None;
                if !found {
                    return vec![];
                }
                let mut params = json!({ "point": [ev.pos.x, ev.pos.y], "gap": self.gap() });
                if !self.open_filled_as_closed() {
                    params["openFilledAsClosed"] = json!(false);
                }
                // The path shows at once and is kept on the release; a press that is dropped (a
                // long or canceled touch, a second finger) or Escape before the release takes it
                // back.
                self.pressing = true;
                vec![Action::Begin("Fill Area".into()), Action::Preview("shapeBuilder.copyRegion".into(), params)]
            }
            PointerKind::Up if std::mem::take(&mut self.pressing) => vec![Action::Commit],
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && std::mem::take(&mut self.pressing) { vec![Action::Cancel] } else { vec![] }
    }

    fn busy(&self) -> bool {
        self.pressing
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        self.hover
            .iter()
            .filter(|(k, _)| *k == self.finder_key(cx))
            .map(|(_, a)| Overlay::Path { path: a.path.to_bezpath(), color: HIGHLIGHT_RED, width: 3.0, dashed: false })
            .collect()
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        if self.hover_holding(cx, p).is_some() { Cursor::LivePaintBucketFace } else { Cursor::LivePaintBucket }
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
        self.at = None;
    }

    /// The area under a still pointer changes with the document and the view.
    fn wants_rehover(&self) -> bool {
        true
    }

    fn waiting(&self) -> bool {
        self.job.is_some()
    }

    fn poll(&mut self, cx: &ToolContext, wait: bool) -> Vec<Action> {
        self.settle(cx, wait, true);
        while wait && self.job.is_some() {
            self.settle(cx, true, true);
        }
        vec![]
    }

    /// The Shape Builder's Gap Detection, Gap Length and Consider Open Filled Paths as Closed,
    /// which the tool shares.
    fn options(&self) -> Value {
        json!({
            "gapDetection": self.gap_detection,
            "gapLength": self.gap_length,
            "openFilledAsClosed": self.open_filled_as_closed(),
        })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "gapDetection" => self.gap_detection = v.as_bool().unwrap_or(self.gap_detection),
            "gapLength" => {
                if let Some(n) = v.as_f64().filter(|n| n.is_finite()) {
                    self.gap_length = n.clamp(0.0, SHAPE_BUILDER_MAX_GAP);
                }
            }
            "openFilledAsClosed" => {
                if let Some(b) = v.as_bool() {
                    self.open_filled_cuts = !b;
                }
            }
            _ => {}
        }
        self.hover = None;
    }
    fn check_option(&self, key: &str, v: &Value) -> Result<(), String> {
        check_builder_options(self.id(), &serde_json::Map::from_iter([(key.to_string(), v.clone())]))
    }
}
