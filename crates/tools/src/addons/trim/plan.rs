//! What Trim at Crossings removes: the stretch at a point, every stretch a rectangle touches, or
//! every stretch a line through some points crosses, on every path that can be edited. The command
//! `addon.pathtools.trim` applies it, and the tool shows it before a click or while a drag stays
//! small, so the two always agree.

use std::collections::{HashMap, HashSet};

use vectorcraft_doc::{Document, NodeId, NodeKind};
use vectorcraft_geom::kurbo::{Line, PathSeg};
use vectorcraft_geom::{PathData, Point, Rect, SubPath, hit};
use vectorcraft_pathops::addons::pathtools as pt;

/// What picks the stretches to remove.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// The stretch of the nearest path within the tolerance (pt) of the point.
    At(Point, f64),
    /// Every stretch the rectangle touches.
    Rect(Rect),
    /// Every stretch the line through the points crosses, with the points' bounds.
    Line(Vec<Point>, Rect),
}

/// Why a pick could not be worked out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooMuch {
    /// Finding where the paths cross took more than its budget.
    Crossings,
    /// The line has too many pieces, or passes too many segments, to check.
    Line,
    /// The rectangle or line reaches more paths than the limit.
    Paths,
}

impl TooMuch {
    /// The error message the command reports.
    pub fn message(self) -> &'static str {
        match self {
            TooMuch::Crossings => "the paths are too many or too tangled to find where they cross",
            TooMuch::Line => "the line is too long, or the paths it passes too many, to check",
            TooMuch::Paths => "the rectangle or line reaches too many paths to check",
        }
    }
}

/// How much work one pick may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most paths a rectangle or a line may reach.
    pub paths: usize,
    /// Most box tests between the paths to trim and the other paths.
    pub box_tests: usize,
    /// Most box tests between a line's pieces and the paths' segments.
    pub line_tests: usize,
    /// The crossing work of every path the pick trims together.
    pub crossings: pt::Work,
}

impl Limits {
    /// The command's limits.
    pub const COMMAND: Limits = Limits { paths: usize::MAX, box_tests: 50_000_000, line_tests: 60_000_000, crossings: pt::Work::share(1) };
}

/// One path the pick trims.
#[derive(Clone, Debug, PartialEq)]
pub struct Trimmed {
    pub id: NodeId,
    /// What takes its place: a path per piece for a path of one subpath, one path holding the
    /// pieces for a path of more, nothing when no piece is left.
    pub pieces: Vec<PathData>,
    /// The stretches removed, each an open subpath.
    pub removed: Vec<SubPath>,
    /// How many stretches were removed.
    pub count: usize,
}

/// The stretches of each subpath of a path between the places it is crossed, for the document as
/// it was when they were found, with the crossing work finding them took. A pick fills it with the
/// paths it looks at; one kept between picks on an unchanged document saves finding the same
/// crossings again, and a pick still draws their work from its budget, so it fails where a pick
/// without them would. It also keeps the paths whose crossings could not be found within the whole
/// crossing work of the picks' limits (`whole`), which no pick with those limits can find either.
#[derive(Default)]
pub struct Stretches {
    by_path: HashMap<NodeId, (Vec<Vec<pt::Span>>, pt::Work)>,
    too_much: HashSet<NodeId>,
    whole: Option<pt::Work>,
}

impl Stretches {
    pub fn clear(&mut self) {
        self.by_path.clear();
        self.too_much.clear();
        self.whole = None;
    }
}

/// A path that takes part: its id, geometry and control-point box.
type Part<'a> = (NodeId, &'a PathData, Rect);

/// The paths that take part, bottom to top: the ones that can be edited, without guides and
/// clipping paths. The clipping path of a clip group or a clipped layer is left out whole, the
/// members of a compound clipping path included.
fn parts(d: &Document) -> Vec<Part<'_>> {
    let editable = d.editable();
    let mut masks = HashSet::new();
    d.walk(|n| {
        if n.clips()
            && let Some(first) = n.children().and_then(|c| c.first())
        {
            first.walk(&mut |m| {
                masks.insert(m.id);
            });
        }
    });
    let mut out = vec![];
    d.walk(|n| {
        if let NodeKind::Path { path, guide: false, clipping: false, .. } = &n.kind
            && !masks.contains(&n.id)
            && let Some(b) = path.control_bounds()
            && editable(n.id)
        {
            out.push((n.id, path, b));
        }
    });
    out
}

/// Does the line through `pts` cross `sp`? Each piece of the line compared costs one of `budget`;
/// `None` once it is spent.
fn crossed_by_line(sp: &SubPath, pts: &[Point], budget: &mut usize) -> Option<bool> {
    let segs: Vec<_> = (0..sp.segment_count())
        .map(|i| {
            let c = sp.segment(i);
            (c, Rect::from_points(c.p0, c.p1).union_pt(c.p2).union_pt(c.p3).inflate(1e-9, 1e-9))
        })
        .collect();
    let Some(reach) = segs.iter().map(|s| s.1).reduce(|a, b| a.union(b)) else { return Some(false) };
    for w in pts.windows(2) {
        let [a, b] = w else { continue };
        let piece = Rect::from_points(*a, *b);
        *budget = budget.checked_sub(1)?;
        if !reach.overlaps(piece) {
            continue;
        }
        for (c, r) in &segs {
            *budget = budget.checked_sub(1)?;
            if r.overlaps(piece) && !PathSeg::Cubic(*c).intersect_line(Line::new(*a, *b)).is_empty() {
                return Some(true);
            }
        }
    }
    Some(false)
}

/// Does the rectangle or line of `pick` touch the outline of `piece`? Never for `Pick::At`.
fn touches(piece: &SubPath, pick: &Pick, budget: &mut usize) -> Result<bool, TooMuch> {
    match pick {
        Pick::At(..) => Ok(false),
        Pick::Rect(r) => Ok(hit::intersects_rect(&PathData::single(piece.clone()), *r)),
        Pick::Line(pts, _) => crossed_by_line(piece, pts, budget).ok_or(TooMuch::Line),
    }
}

/// What a trim leaves of a path: its subpaths, the stretches removed and how many.
type Cut = (Vec<SubPath>, Vec<SubPath>, usize);

/// What trimming `path` by `pick` leaves (`on`: the place of `Pick::At` on it): its subpaths then,
/// the stretches removed and how many. `None` when none goes. A rectangle or a line that touches
/// no part of `path` leaves it out before its crossings are looked for.
#[allow(clippy::too_many_arguments)]
fn trimmed<'a>(
    id: NodeId,
    path: &PathData,
    others: &dyn Fn() -> Vec<&'a PathData>,
    pick: &Pick,
    on: Option<pt::Place>,
    budget: &mut usize,
    work: &mut pt::Work,
    stretches: &mut Stretches,
) -> Result<Option<Cut>, TooMuch> {
    let reached = match pick {
        Pick::At(..) => true,
        Pick::Rect(r) => hit::intersects_rect(path, *r),
        Pick::Line(..) => {
            let mut any = false;
            for sp in &path.subpaths {
                if touches(sp, pick, budget)? {
                    any = true;
                    break;
                }
            }
            any
        }
    };
    if !reached {
        return Ok(None);
    }
    let all = match stretches.by_path.get(&id) {
        Some((all, cost)) => {
            work.draw(*cost).ok_or(TooMuch::Crossings)?;
            all.clone()
        }
        None if stretches.too_much.contains(&id) => return Err(TooMuch::Crossings),
        None => {
            let before = *work;
            let Some(places) = pt::crossings_on_within(path, &others(), work) else {
                if Some(before) == stretches.whole {
                    stretches.too_much.insert(id);
                }
                return Err(TooMuch::Crossings);
            };
            let all: Vec<Vec<pt::Span>> = path
                .subpaths
                .iter()
                .enumerate()
                .map(|(si, sp)| {
                    // The places come in subpath order: this subpath's are one run of them.
                    let run = places.partition_point(|p| p.0 < si)..places.partition_point(|p| p.0 <= si);
                    let here: Vec<(usize, f64)> = places.get(run).unwrap_or_default().iter().map(|&(_, seg, t)| (seg, t)).collect();
                    pt::spans(sp, &here)
                })
                .collect();
            stretches.by_path.insert(id, (all.clone(), work.spent_since(before)));
            all
        }
    };
    let mut subs = vec![];
    let mut removed = vec![];
    let mut count = 0;
    for ((si, sp), spans) in path.subpaths.iter().enumerate().zip(&all) {
        let marks: Vec<bool> = match pick {
            Pick::At(..) => {
                let at = on.filter(|(s, ..)| *s == si).and_then(|(_, seg, t)| pt::span_at(sp, spans, seg, t));
                (0..spans.len()).map(|i| Some(i) == at).collect()
            }
            _ => spans.iter().map(|s| touches(&pt::slice(sp, *s), pick, budget)).collect::<Result<_, _>>()?,
        };
        match pt::without_spans(sp, spans, &marks) {
            Some(pieces) => {
                count += marks.iter().filter(|m| **m).count();
                removed.extend(spans.iter().zip(&marks).filter(|(_, m)| **m).map(|(s, _)| pt::slice(sp, *s)).filter(|p| p.anchors.len() >= 2));
                subs.extend(pieces);
            }
            None => subs.push(sp.clone()),
        }
    }
    Ok((count > 0).then_some((subs, removed, count)))
}

/// What `pick` removes from the document's paths, bottom to top. With `isolation` (the isolated
/// object in isolation mode, see [`crate::ordinary_isolation`]) only paths inside it are trimmed;
/// paths outside it still cross them. `stretches` must hold only what it learned from this
/// document as it is.
pub fn plan(d: &Document, isolation: Option<NodeId>, pick: &Pick, limits: Limits, stretches: &mut Stretches) -> Result<Vec<Trimmed>, TooMuch> {
    let parts = parts(d);
    let in_scope = |id: NodeId| isolation.is_none_or(|r| id == r || d.ancestry(id).is_some_and(|a| a.contains(&r)));
    // The paths to trim, each with the place `at` falls on.
    let targets: Vec<(usize, Option<pt::Place>)> = match pick {
        Pick::At(at, tolerance) => {
            let mut best: Option<(usize, pt::Place, f64)> = None;
            for (i, (id, path, b)) in parts.iter().enumerate() {
                if !in_scope(*id) {
                    continue;
                }
                let b = b.inflate(*tolerance, *tolerance);
                if !(b.x0 <= at.x && at.x <= b.x1 && b.y0 <= at.y && at.y <= b.y1) {
                    continue;
                }
                // Of paths equally near, the one on top.
                if let Some((si, seg, t, _, dist)) = path.nearest(*at)
                    && dist <= *tolerance
                    && best.is_none_or(|b| dist <= b.2)
                {
                    best = Some((i, (si, seg, t), dist));
                }
            }
            best.map(|(i, on, _)| (i, Some(on))).into_iter().collect()
        }
        Pick::Rect(r) | Pick::Line(_, r) => {
            parts.iter().enumerate().filter(|(_, (id, _, b))| in_scope(*id) && b.overlaps(*r)).map(|(i, _)| (i, None)).collect()
        }
    };
    // Each path to trim has its box tested against every other path's.
    if targets.len() > limits.paths || targets.len().saturating_mul(parts.len()) > limits.box_tests {
        return Err(TooMuch::Paths);
    }
    let mut out = vec![];
    let mut budget = limits.line_tests;
    // A path that failed within a whole budget of another size may fit in this one.
    if stretches.whole != Some(limits.crossings) {
        stretches.too_much.clear();
        stretches.whole = Some(limits.crossings);
    }
    // One budget of crossing work for every path the pick trims.
    let mut work = limits.crossings;
    for (i, on) in targets {
        let Some((id, path, b)) = parts.get(i) else { continue };
        let reach = b.inflate(1e-3, 1e-3);
        let others = || parts.iter().enumerate().filter(|(j, (_, _, ob))| *j != i && reach.overlaps(*ob)).map(|(_, (_, o, _))| *o).collect();
        let Some((subs, removed, count)) = trimmed(*id, path, &others, pick, on, &mut budget, &mut work, stretches)? else { continue };
        // A path of one subpath becomes a path per piece; the pieces of one with more stay its subpaths.
        let pieces = match (path.subpaths.len(), subs.is_empty()) {
            (_, true) => vec![],
            (1, false) => subs.into_iter().map(PathData::single).collect(),
            _ => vec![PathData::new(subs)],
        };
        out.push(Trimmed { id: *id, pieces, removed, count });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::{Appearance, Node};

    fn row(y: f64, segments: usize) -> PathData {
        PathData::single(SubPath::polyline(&(0..=segments).map(|i| Point::new(i as f64 * 0.01, y)).collect::<Vec<_>>(), false))
    }

    #[test]
    fn a_path_too_large_for_a_whole_budget_is_not_searched_again_and_one_cut_short_by_a_shared_budget_is() {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let mut ids = vec![];
        for (i, p) in [row(0.0, 7000), row(10.0, 7000), row(20.0, 7000), row(30.0, 10_001)].into_iter().enumerate() {
            let id = d.alloc_id();
            d.insert(Some(l), i, Node::path(id, p, Appearance::default_art())).unwrap();
            ids.push(id);
        }
        let mut kept = Stretches::default();
        // More segments than a crossing search takes: no budget finds its crossings.
        assert_eq!(plan(&d, None, &Pick::At(Point::new(50.0, 30.0), 1.0), Limits::COMMAND, &mut kept), Err(TooMuch::Crossings));
        assert!(kept.too_much.contains(&ids[3]));
        // The third of three rows runs out of the budget the rectangle shares; alone it has enough.
        let rect = Pick::Rect(Rect::new(-1.0, -1.0, 80.0, 21.0));
        assert_eq!(plan(&d, None, &rect, Limits::COMMAND, &mut kept), Err(TooMuch::Crossings));
        assert!(!kept.too_much.contains(&ids[2]));
        assert_eq!(plan(&d, None, &Pick::At(Point::new(35.0, 20.0), 1.0), Limits::COMMAND, &mut kept).map(|t| t.len()), Ok(1));
        // A row is too much for a twentieth of the budget, and is remembered only for picks with it.
        let small = Limits { crossings: pt::Work::share(20), ..Limits::COMMAND };
        let mut kept = Stretches::default();
        assert_eq!(plan(&d, None, &Pick::At(Point::new(35.0, 10.0), 1.0), small, &mut kept), Err(TooMuch::Crossings));
        assert!(kept.too_much.contains(&ids[1]));
        assert_eq!(plan(&d, None, &Pick::At(Point::new(35.0, 10.0), 1.0), Limits::COMMAND, &mut kept).map(|t| t.len()), Ok(1));
        assert!(!kept.too_much.contains(&ids[1]));
        kept.clear();
        assert!(kept.too_much.is_empty() && kept.by_path.is_empty());
    }
}
