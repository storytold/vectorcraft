//! A blank folded into its 3D shape ([`fold`]).
//!
//! The flat blank lies at z = 0 with document x to the right and document y flipped so y points up;
//! the printed side faces +z. The base panel stays in place. The other panels are reached from it
//! through a breadth-first tree of hinges, each panel's hinges taken in the order of their ends (y,
//! then x, the first end first), so the tree does not depend on the order the lines were drawn in. A child
//! panel turns about its hinge by the hinge's angle times the fold amount; a positive angle moves it
//! toward -z, away from the printed side, whichever way the hinge was drawn. Hinges outside the tree
//! close a loop of panels: [`FoldedHinge::gap`] says how far apart their two placements end up when
//! the blank is folded all the way.

use std::collections::{HashSet, VecDeque};

use vectorcraft_geom::{PathData, PathEl, Point, Shape as _};

use super::{Blank, base_panel, point_in};

/// Most points a panel's outline is flattened to; a panel with more is left out of the collision
/// test and the preview.
pub const MAX_POLYGON_POINTS: usize = 1 << 16;

/// How far apart two placements of a panel may turn a unit direction and still be one placement:
/// above the rounding of up to [`super::MAX_PANELS`] turns, below the turn of a hinge a millionth
/// of a degree from another ([`super::kept_angle`], 1.7e-8).
const TURNED: f64 = 1e-10;

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// A document point on the flat blank (y flipped so it points up, z = 0).
pub(super) fn flat(p: Point) -> V3 {
    [p.x, -p.y, 0.0]
}

/// Where a panel is: a rotation `m` (rows) and a translation `t`, taking a point of the flat
/// blank (see [`flat`]) to its place in 3D.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub m: [[f64; 3]; 3],
    pub t: [f64; 3],
}

impl Pose {
    /// The flat blank's own place.
    pub const IDENTITY: Pose = Pose { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t: [0.0; 3] };

    /// Where the flat point `v` goes.
    pub fn apply(&self, v: [f64; 3]) -> [f64; 3] {
        add(self.turn(v), self.t)
    }

    /// The direction `v` turned by the rotation alone.
    pub fn turn(&self, v: [f64; 3]) -> [f64; 3] {
        let [a, b, c] = self.m;
        [dot(a, v), dot(b, v), dot(c, v)]
    }

    /// Where the document point `p` goes.
    pub fn place(&self, p: Point) -> [f64; 3] {
        self.apply(flat(p))
    }

    /// The direction the printed side faces.
    pub fn normal(&self) -> [f64; 3] {
        self.turn([0.0, 0.0, 1.0])
    }

    /// This pose followed, in the flat blank's terms, by a right-handed turn of `theta` radians
    /// about the line through `at` along the unit direction `axis`: the pose of a panel hinged to
    /// this one there.
    fn turned_about(&self, at: V3, axis: V3, theta: f64) -> Pose {
        let (s, c) = theta.sin_cos();
        let [x, y, z] = axis;
        // Rodrigues' rotation matrix.
        let r = [
            [c + x * x * (1.0 - c), x * y * (1.0 - c) - z * s, x * z * (1.0 - c) + y * s],
            [y * x * (1.0 - c) + z * s, c + y * y * (1.0 - c), y * z * (1.0 - c) - x * s],
            [z * x * (1.0 - c) - y * s, z * y * (1.0 - c) + x * s, c + z * z * (1.0 - c)],
        ];
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.m[i][k] * r[k][j]).sum();
            }
        }
        // p ↦ self(r (p - at) + at) = (self.m r) p + self.m (at - r at) + self.t
        let r_at = [dot(r[0], at), dot(r[1], at), dot(r[2], at)];
        Pose { m, t: add(self.turn(sub(at, r_at)), self.t) }
    }

    /// This pose, taken about `origin` (a point less `origin` to its place less `origin`), in the
    /// flat blank's own terms.
    fn about(&self, origin: V3) -> Pose {
        Pose { m: self.m, t: add(sub(self.t, self.turn(origin)), origin) }
    }
}

/// A folded panel: where it is, the same about [`Folded::origin`] (`local`), its parent in the fold
/// tree (`None` for the base and for a panel no chain of hinges joins to it, which stays flat where
/// it is) and how many hinges lie between it and the base.
#[derive(Clone, Debug)]
pub struct FoldedPanel {
    pub pose: Pose,
    /// Where the panel is with the flat blank moved so `origin` is at 0: it takes a flat point less
    /// `origin` to its place less `origin`. Worked out in these terms, it keeps the rounding of a
    /// blank far from the document's origin to the size of the blank.
    pub local: Pose,
    pub parent: Option<usize>,
    pub depth: usize,
}

/// A folded hinge: its panels, its angle (degrees, before the fold amount), whether it is in the
/// fold tree and, for one that is not, how far apart its two placements end up with every hinge at
/// its full angle, whatever the fold amount (`gap`, points), and whether that is close enough to call
/// it closed.
#[derive(Clone, Debug)]
pub struct FoldedHinge {
    pub a: usize,
    pub b: usize,
    pub angle: f64,
    pub tree: bool,
    pub closes: bool,
    pub gap: f64,
}

/// A blank folded at one amount.
#[derive(Clone, Debug)]
pub struct Folded {
    pub base: usize,
    pub panels: Vec<FoldedPanel>,
    /// The panels in breadth-first order from the base, the panels not joined to it last.
    pub order: Vec<usize>,
    pub hinges: Vec<FoldedHinge>,
    /// Pairs of panels (`a` < `b`) that pass through each other.
    pub collisions: Vec<(usize, usize)>,
    /// The fold tree that panels folded flat onto each other stack by.
    pub stack: Stack,
    /// The middle of the blank's bounds (flat, see [`flat`]), which [`FoldedPanel::local`] is about.
    pub origin: [f64; 3],
}

/// The fold tree that panels in one plane stack by: the tree from the blank's largest panel (its
/// `root`), whichever panel is the base, so choosing another base moves the folded blank without
/// changing which panel shows where they lie flat on each other. The base's own tree when the two
/// join different panels.
#[derive(Clone, Debug, Default)]
pub struct Stack {
    pub root: usize,
    /// Each panel's parent in this tree.
    pub parent: Vec<Option<usize>>,
    /// The panels in breadth-first order from `root`, the panels not joined to it last.
    pub order: Vec<usize>,
    /// For each panel, the way from the hinge it is folded on into its parent, as folded: a vector
    /// in the parent's plane, across the hinge to the side the fold took the parent to be on, as
    /// long as the corners of the parent's bounds reach that way. `None` for `root` and the
    /// panels not joined to it.
    pub toward_parent: Vec<Option<[f64; 3]>>,
}

/// The panel of `blank` that `p` (document coordinates) lies in.
pub fn panel_at(blank: &Blank, p: Point) -> Option<usize> {
    blank.panels.iter().position(|panel| panel.outline.to_bezpath().winding(p) != 0)
}

/// Which of `blanks` to fold, and about which of its panels: the blank and panel `at` (document
/// coordinates) lies in, or, without a point, the blank with the largest panel (the first of equals)
/// and its base panel. `None` when `at` lies in no panel, or no blank has a panel.
pub fn pick(blanks: &[Blank], at: Option<Point>) -> Option<(usize, usize)> {
    match at {
        Some(p) => blanks.iter().enumerate().find_map(|(i, b)| Some((i, panel_at(b, p)?))),
        None => {
            let largest = |b: &Blank| b.panels.iter().map(|p| p.area).fold(f64::NEG_INFINITY, f64::max);
            let (i, b) =
                blanks.iter().enumerate().filter(|(_, b)| !b.panels.is_empty()).fold(None, |best: Option<(usize, &Blank)>, (i, b)| match best {
                    Some((_, kept)) if largest(kept) >= largest(b) => best,
                    _ => Some((i, b)),
                })?;
            Some((i, base_panel(&b.panels)?))
        }
    }
}

/// `blank` folded about its panel `base` (the largest when `None`) by `amount` (0, flat, to 1, every
/// hinge at its angle). `tolerance` (points) is how far apart a closing hinge's placements may end
/// up and still close (or 0.5% of its length, when that is more), and how far apart two panels in
/// one plane may lie and still be stacked rather than crossing. `None` when the blank has no
/// panels, `base` is not one of them, or `amount` is not from 0 to 1.
pub fn fold(blank: &Blank, base: Option<usize>, amount: f64, tolerance: f64) -> Option<Folded> {
    let base = match base {
        Some(b) => b,
        None => base_panel(&blank.panels)?,
    };
    if base >= blank.panels.len() || !(0.0..=1.0).contains(&amount) {
        return None;
    }
    let tolerance = if tolerance.is_finite() && tolerance > 0.0 { tolerance } else { 0.01 };
    let n = blank.panels.len();
    // Each panel's hinges, by the place they start (y, then x), then by the other panel.
    let mut by_panel: Vec<Vec<usize>> = vec![vec![]; n];
    for (i, h) in blank.hinges.iter().enumerate() {
        for p in [h.a, h.b] {
            if let Some(list) = by_panel.get_mut(p) {
                list.push(i);
            }
        }
    }
    // A hinge's ends, the one first by (y, x) first.
    let ends = |i: &usize| {
        blank.hinges.get(*i).map_or([f64::INFINITY; 4], |h| {
            let (s, e) = if (h.from.y, h.from.x) <= (h.to.y, h.to.x) { (h.from, h.to) } else { (h.to, h.from) };
            [s.y, s.x, e.y, e.x]
        })
    };
    for list in &mut by_panel {
        list.sort_by(|a, b| {
            let (ka, kb) = (ends(a), ends(b));
            ka.iter().zip(&kb).fold(std::cmp::Ordering::Equal, |o, (x, y)| o.then(x.total_cmp(y))).then(a.cmp(b))
        });
    }
    let centers: Vec<Option<V3>> = blank.panels.iter().map(|p| point_in(&p.outline).map(flat)).collect();
    // The corners of each panel's bounds (flat).
    let corners: Vec<Option<[V3; 4]>> = blank
        .panels
        .iter()
        .map(|p| p.outline.bounds().map(|b| [(b.x0, b.y0), (b.x1, b.y0), (b.x0, b.y1), (b.x1, b.y1)].map(|(x, y)| flat(Point::new(x, y)))))
        .collect();
    let origin = corners
        .iter()
        .flatten()
        .flatten()
        .fold(None, |b: Option<(V3, V3)>, c| {
            Some(b.map_or((*c, *c), |(lo, hi)| ([lo[0].min(c[0]), lo[1].min(c[1]), 0.0], [hi[0].max(c[0]), hi[1].max(c[1]), 0.0])))
        })
        .map_or([0.0; 3], |(lo, hi)| scale(add(lo, hi), 0.5));
    let placing = Placing { blank, by_panel: &by_panel, centers: &centers, corners: &corners, origin };
    let (panels, tree, order, toward_parent) = place(&placing, base, amount);
    // Closing hinges are measured with every hinge at its full angle, so a blank that closes when
    // folded is not reported open part of the way there.
    let full = if amount == 1.0 { None } else { Some(place(&placing, base, 1.0).0) };
    let measured = full.as_deref().unwrap_or(&panels);
    let hinges = blank
        .hinges
        .iter()
        .zip(&tree)
        .map(|(h, &tree)| {
            let (pa, pb) = (measured.get(h.a).map(|p| p.pose), measured.get(h.b).map(|p| p.pose));
            let gap = match (pa, pb) {
                (Some(pa), Some(pb)) if !tree => [h.from, h.to].iter().map(|&e| length(sub(pa.place(e), pb.place(e)))).fold(0.0, f64::max),
                _ => 0.0,
            };
            let closes = tree || gap <= tolerance.max(0.005 * h.from.distance(h.to));
            FoldedHinge { a: h.a, b: h.b, angle: h.angle, tree, closes, gap }
        })
        .collect();
    let collisions = collisions(blank, &panels, tolerance);
    let own = Stack { root: base, parent: panels.iter().map(|p| p.parent).collect(), order: order.clone(), toward_parent };
    let stack = match base_panel(&blank.panels) {
        Some(root) if root != base => {
            let (placed, root_tree, root_order, toward) = place(&placing, root, amount);
            // Whether the root's tree puts every panel where the base's does, moved with the root
            // (which lies at the identity in its own): the same hinges do, and around a loop of
            // panels another tree can leave out a fold that placed them. Each panel's turn is
            // compared to within `TURNED` whatever its size or place, and its inside point by the
            // poses about `origin` (`local`), to rounding at that distance from it.
            let agrees = |r: &FoldedPanel| {
                let near = |a: V3, b: V3| length(sub(a, b)) <= 1e-9 * (1.0 + length(a).max(length(b)));
                placed.iter().zip(&panels).enumerate().all(|(i, (p, q))| {
                    let joined = (i == root || p.parent.is_some(), i == base || q.parent.is_some());
                    let at = centers.get(i).copied().flatten().map_or([0.0; 3], |c| sub(c, origin));
                    let turned = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
                        .into_iter()
                        .all(|e| length(sub(r.local.turn(p.local.turn(e)), q.local.turn(e))) <= TURNED);
                    joined.0 == joined.1 && (!joined.0 || (turned && near(r.local.apply(p.local.apply(at)), q.local.apply(at))))
                })
            };
            match panels.get(root) {
                // The root's tree in the base's terms.
                Some(r) if root_tree == tree || agrees(r) => Stack {
                    root,
                    parent: placed.iter().map(|p| p.parent).collect(),
                    order: root_order,
                    toward_parent: toward.into_iter().map(|v| v.map(|v| r.pose.turn(v))).collect(),
                },
                _ => own,
            }
        }
        _ => own,
    };
    Some(Folded { base, panels, order, hinges, collisions, stack, origin })
}

/// What [`place`] gives: the placed panels, which hinges are in the tree, the panels' order and
/// [`Stack::toward_parent`].
type Tree = (Vec<FoldedPanel>, Vec<bool>, Vec<usize>, Vec<Option<V3>>);

/// What [`place`] places: the blank, each panel's hinges in order, each panel's inside point and
/// bounds corners (flat), and the point the panels' local poses are about.
struct Placing<'a> {
    blank: &'a Blank,
    by_panel: &'a [Vec<usize>],
    centers: &'a [Option<V3>],
    corners: &'a [Option<[V3; 4]>],
    origin: V3,
}

/// The panels of the blank placed by the fold tree from `base` with each hinge at `amount` of its
/// angle, which hinges are in the tree, the panels in breadth-first order (the panels not joined
/// to the base last), and [`Stack::toward_parent`].
fn place(placing: &Placing, base: usize, amount: f64) -> Tree {
    let Placing { blank, by_panel, centers, corners, origin } = *placing;
    let n = blank.panels.len();
    let mut panels: Vec<FoldedPanel> = vec![FoldedPanel { pose: Pose::IDENTITY, local: Pose::IDENTITY, parent: None, depth: 0 }; n];
    let mut toward_parent: Vec<Option<V3>> = vec![None; n];
    let mut reached = vec![false; n];
    let mut tree = vec![false; blank.hinges.len()];
    let mut order = Vec::with_capacity(n);
    let mut queue = VecDeque::from([base]);
    if let Some(r) = reached.get_mut(base) {
        *r = true;
    }
    while let Some(p) = queue.pop_front() {
        order.push(p);
        let Some(parent) = panels.get(p).cloned() else { continue };
        for &hi in by_panel.get(p).map(Vec::as_slice).unwrap_or_default() {
            let Some(h) = blank.hinges.get(hi) else { continue };
            let child = if h.a == p { h.b } else { h.a };
            if reached.get(child) != Some(&false) {
                continue;
            }
            let (mut from, mut to) = (flat(h.from), flat(h.to));
            let len = length(sub(to, from));
            if !(len.is_finite() && len > 0.0) {
                continue;
            }
            // The parent on the left of the hinge's direction, the child on its right. The side is
            // read next to the hinge, so a parent with a notch or an arm past the hinge's line still
            // turns the child the right way; the panel's inside point is the fallback.
            let left = match blank.panels.get(p).and_then(|panel| side_of(&panel.outline, from, to, len, flat(h.on))) {
                Some(left) => left,
                None => centers.get(p).copied().flatten().is_none_or(|c| cross(sub(to, from), sub(c, from))[2] >= 0.0),
            };
            if !left {
                std::mem::swap(&mut from, &mut to);
            }
            let axis = scale(sub(to, from), 1.0 / len);
            let theta = (h.angle * amount).to_radians();
            if !theta.is_finite() {
                continue;
            }
            if let Some(slot) = panels.get_mut(child) {
                let local = parent.local.turned_about(sub(from, origin), axis, theta);
                *slot = FoldedPanel { pose: local.about(origin), local, parent: Some(p), depth: parent.depth + 1 };
            }
            // Across the hinge, to its left: into the parent, as far as the corners of its bounds
            // reach that way.
            let into = [-axis[1], axis[0], 0.0];
            let reach = corners.get(p).copied().flatten().map_or(0.0, |c| c.into_iter().map(|c| dot(sub(c, from), into)).fold(0.0, f64::max));
            if let Some(slot) = toward_parent.get_mut(child) {
                *slot = Some(parent.pose.turn(scale(into, reach)));
            }
            if let Some(r) = reached.get_mut(child) {
                *r = true;
            }
            if let Some(t) = tree.get_mut(hi) {
                *t = true;
            }
            queue.push_back(child);
        }
    }
    order.extend((0..n).filter(|&i| reached.get(i) == Some(&false)));
    (panels, tree, order, toward_parent)
}

/// Whether `outline` lies on the left of the line from `from` to `to` (flat coordinates, `len` apart)
/// next to it: `true` when a point just off the line on the left is inside it and the matching
/// point on the right is not, `false` the other way round. `on` (a point of the line where the
/// panels meet: [`super::Hinge::on`]) is tried first, then the middle and points a quarter and an
/// eighth of the way along, each with the nearest offset first; `None` when no point tells the
/// sides apart.
fn side_of(outline: &PathData, from: V3, to: V3, len: f64, on: V3) -> Option<bool> {
    let bp = outline.to_bezpath();
    let left = [-(to[1] - from[1]) / len, (to[0] - from[0]) / len, 0.0];
    let doc = |v: V3| Point::new(v[0], -v[1]);
    let mut offsets = [1e-3, 0.25, len * 0.01];
    offsets.sort_by(f64::total_cmp);
    let along = [0.5, 0.25, 0.75, 0.125, 0.375, 0.625, 0.875].map(|t| add(from, scale(sub(to, from), t)));
    std::iter::once(on).chain(along).find_map(|at| {
        offsets.into_iter().find_map(|off| {
            let (l, r) = (bp.winding(doc(add(at, scale(left, off)))) != 0, bp.winding(doc(sub(at, scale(left, off)))) != 0);
            (l != r).then_some(l)
        })
    })
}

/// The rings of `path` flattened to `tolerance`, in document coordinates; `None` past
/// [`MAX_POLYGON_POINTS`] points.
pub fn polygon(path: &PathData, tolerance: f64) -> Option<Vec<Vec<Point>>> {
    let mut rings: Vec<Vec<Point>> = vec![];
    let mut count = 0usize;
    let mut fits = true;
    vectorcraft_geom::flatten(path.to_bezpath(), tolerance, |el| {
        if !fits {
            return;
        }
        match el {
            PathEl::MoveTo(p) => rings.push(vec![p]),
            PathEl::LineTo(p) => match rings.last_mut() {
                Some(r) => r.push(p),
                None => rings.push(vec![p]),
            },
            _ => return,
        }
        count += 1;
        fits = count <= MAX_POLYGON_POINTS;
    });
    rings.retain(|r| r.len() >= 3);
    (fits && rings.iter().flatten().all(|p| p.is_finite())).then_some(rings)
}

/// A panel placed in 3D: its rings, the plane it lies in (unit normal and offset) and its bounds.
struct Placed {
    rings: Vec<Vec<V3>>,
    normal: V3,
    offset: f64,
    lo: V3,
    hi: V3,
}

/// The pairs of `panels` that pass through each other. Two panels hinged to each other are not
/// tested, nor two in parallel planes (stacked in one plane, or apart).
fn collisions(blank: &Blank, panels: &[FoldedPanel], tolerance: f64) -> Vec<(usize, usize)> {
    let placed: Vec<Option<Placed>> = blank
        .panels
        .iter()
        .zip(panels)
        .map(|(panel, f)| {
            let rings: Vec<Vec<V3>> =
                polygon(&panel.outline, tolerance)?.into_iter().map(|r| r.into_iter().map(|p| f.pose.place(p)).collect()).collect();
            let all = rings.iter().flatten();
            let lo = all.clone().fold([f64::INFINITY; 3], |a, p| [a[0].min(p[0]), a[1].min(p[1]), a[2].min(p[2])]);
            let hi = all.fold([f64::NEG_INFINITY; 3], |a, p| [a[0].max(p[0]), a[1].max(p[1]), a[2].max(p[2])]);
            let normal = f.pose.normal();
            let offset = dot(normal, f.pose.t);
            Some(Placed { rings, normal, offset, lo, hi })
        })
        .collect();
    let hinged: HashSet<(usize, usize)> = blank.hinges.iter().map(|h| (h.a.min(h.b), h.a.max(h.b))).collect();
    let mut out = vec![];
    for (i, a) in placed.iter().enumerate() {
        let Some(a) = a else { continue };
        for (j, b) in placed.iter().enumerate().skip(i + 1) {
            let Some(b) = b else { continue };
            if hinged.contains(&(i, j)) || (0..3).any(|k| a.hi[k] + tolerance < b.lo[k] || b.hi[k] + tolerance < a.lo[k]) {
                continue;
            }
            let line = cross(a.normal, b.normal);
            let along = length(line);
            if along.is_nan() || along <= 1e-9 {
                continue;
            }
            let u = scale(line, 1.0 / along);
            let on_a = crossings(&a.rings, b.normal, b.offset, u, tolerance);
            let on_b = crossings(&b.rings, a.normal, a.offset, u, tolerance);
            if on_a.iter().any(|x| on_b.iter().any(|y| x.1.min(y.1) - x.0.max(y.0) > tolerance)) {
                out.push((i, j));
            }
        }
    }
    out
}

/// Where `rings` cross the plane `normal · p = offset`: the spans of the line along `u` that lie
/// inside them, by their place along `u`. A panel with no point more than `tolerance` to one side
/// of the plane only touches it, and crosses it nowhere; otherwise points within `tolerance` of it
/// count as on its positive side.
fn crossings(rings: &[Vec<V3>], normal: V3, offset: f64, u: V3, tolerance: f64) -> Vec<(f64, f64)> {
    let side = |p: V3| dot(normal, p) - offset;
    let sides = rings.iter().flatten().map(|&p| side(p));
    let (above, below) = sides.fold((false, false), |(a, b), s| (a || s > tolerance, b || s < -tolerance));
    if !(above && below) {
        return vec![];
    }
    let mut at = vec![];
    for ring in rings {
        for (k, &p) in ring.iter().enumerate() {
            let Some(&q) = ring.get((k + 1) % ring.len()) else { continue };
            let (sp, sq) = (side(p), side(q));
            if (sp >= -tolerance) != (sq >= -tolerance) {
                let t = (sp / (sp - sq)).clamp(0.0, 1.0);
                at.push(dot(u, add(p, scale(sub(q, p), t))));
            }
        }
    }
    at.retain(|x| x.is_finite());
    at.sort_by(f64::total_cmp);
    at.as_chunks::<2>().0.iter().map(|&[a, b]| (a, b)).collect()
}
