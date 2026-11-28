//! The panels and hinges of a packaging blank, from its fold lines.
//!
//! Cut lines bound the material. Each connected piece of Cut lines with material is one blank: its
//! outline is the largest closed Cut subpath of the piece when every other face of the piece lies
//! inside it or inside another closed Cut subpath, and the piece's outer boundary otherwise. A face
//! is material when it lies inside that outline and inside no other closed Cut subpath of the piece,
//! so a slot drawn as a rectangle across the outline leaves the same material as a notch drawn in
//! it. Crease, Partial Cut and Perforation lines divide the material into panels; the pieces of them
//! between two panels are that pair's hinge. [`blanks`] also lists what is wrong with the lines
//! ([`IssueCode`]).

use std::collections::{HashMap, HashSet};

use vectorcraft_geom::kurbo::{Line, ParamCurveArclen, ParamCurveNearest, PathSeg};
use vectorcraft_geom::{Affine, BezPath, FillRule, ParamCurve, PathData, Point, Rect, Shape as _, SubPath};
use vectorcraft_pathops::{Adjacency, DEFAULT_PRECISION, SHAPE_BUILDER_MAX_SEGMENTS, Shape, adjacency, find_gaps, interior_point, unite_all};

/// Most panels one blank may have. Its hinges are the edges of a planar graph on its panels, so there
/// are at most 3 × 256 - 6 = 762 of them.
pub const MAX_PANELS: usize = 256;
/// Most segments the fold lines may have together.
pub const MAX_SEGMENTS: usize = SHAPE_BUILDER_MAX_SEGMENTS;

/// What a fold line does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Separates the material.
    Cut,
    /// A fold.
    Crease,
    /// A fold cut part way through the board.
    PartialCut,
    /// A fold along a line of cuts.
    Perforation,
}

impl LineKind {
    /// The angle (degrees) a line of this kind folds at: 90 for a Crease or a Perforation, -90 for
    /// a Partial Cut. A positive angle folds away from the printed side; none for a Cut.
    pub fn fold_angle(self) -> Option<f64> {
        match self {
            LineKind::Cut => None,
            LineKind::Crease | LineKind::Perforation => Some(90.0),
            LineKind::PartialCut => Some(-90.0),
        }
    }
}

/// A fold line: the path (document coordinates), its kind, a key the issues name it by, and its
/// own fold angle (degrees), which a line that folds takes instead of its kind's.
#[derive(Clone, Debug)]
pub struct FoldLine {
    pub path: PathData,
    pub kind: LineKind,
    pub key: u64,
    pub angle: Option<f64>,
}

/// `angle` (degrees) to the whole millionth of a degree, the finest fold angle a line keeps. A fold
/// that close to flat is flat: the preview tells panels in one plane from panels a millionth of a
/// degree apart, but not from panels closer than that.
pub fn kept_angle(angle: f64) -> f64 {
    (angle * 1e6).round() / 1e6 + 0.0
}

impl FoldLine {
    /// The angle (degrees) the line folds at: its own angle when it has one from -180 to 180, to the
    /// millionth of a degree ([`kept_angle`]), else its kind's ([`LineKind::fold_angle`]); none for
    /// a Cut, whatever its own angle.
    pub fn fold_angle(&self) -> Option<f64> {
        let own = self.angle.filter(|a| (-180.0..=180.0).contains(a)).map(kept_angle);
        self.kind.fold_angle().map(|default| own.unwrap_or(default))
    }
}

/// One blank: its panels, the hinges between them, and its holes.
#[derive(Clone, Debug, Default)]
pub struct Blank {
    pub panels: Vec<Panel>,
    pub hinges: Vec<Hinge>,
    pub holes: Vec<PathData>,
}

/// A panel: an area of material between fold lines.
#[derive(Clone, Debug)]
pub struct Panel {
    pub outline: PathData,
    pub area: f64,
}

impl Panel {
    /// A point inside the panel, or the middle of its bounds for one too thin to find one in.
    pub fn point_inside(&self) -> Option<Point> {
        point_in(&self.outline)
    }
}

/// The hinge between panels `a` and `b` (indices into the blank's panels, `a` < `b`): the straight
/// line from `from` to `to`, made of the lines `keys`. `straight` is false when its pieces are not on
/// one straight line within the tolerance; `from` and `to` are then on the line through its longest
/// piece. `angle` (degrees) is the fold angle ([`FoldLine::fold_angle`]) of the line with the most
/// length in the hinge, the one with the lowest key among lines of equal length. `on` is the middle
/// of its longest piece, a point where the two panels meet, which a hinge with gaps between its
/// pieces may not have at the middle of `from` to `to`.
#[derive(Clone, Debug)]
pub struct Hinge {
    pub a: usize,
    pub b: usize,
    pub from: Point,
    pub to: Point,
    pub on: Point,
    pub keys: Vec<u64>,
    pub straight: bool,
    pub angle: f64,
}

/// What can be wrong with fold lines, in the order [`blanks`] lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IssueCode {
    /// No Cut line.
    NoCut,
    /// The Cut lines enclose no area.
    Open,
    /// An end of a Cut line meets no other fold line within the tolerance (a slit that ends on a
    /// crease is not loose).
    LooseEnd,
    /// An end of a Cut line comes within the tolerance of a Cut line without meeting it; the panels
    /// are built with the gap closed.
    Gap,
    /// Over [`MAX_SEGMENTS`] segments or [`MAX_PANELS`] panels in a blank, or the lines can't be
    /// mapped.
    TooComplex,
    /// A Cut line and a crease lie on top of each other.
    CutOverCrease,
    /// Two Cut lines, or two of the lines that fold (of the same kind or not), lie on top of each
    /// other.
    Duplicate,
    /// A crease ends inside a panel.
    DanglingCrease,
    /// A crease divides no two panels.
    NoSplit,
    /// A crease lies outside every blank, wholly, or in part for the tolerance or more.
    Outside,
    /// A hinge's pieces are not on one straight line.
    NotStraight,
    /// A gap between lines is narrower than the planar map can tell apart.
    Sliver,
    /// A hole holds crease lines, or a blank lies inside a hole.
    Ambiguous,
    /// A panel is joined to the largest panel of its blank by no chain of hinges.
    LoosePanel,
    /// A fold line is on a layer that does not print (the caller reports it).
    NonPrinting,
}

impl IssueCode {
    /// Every code, in order.
    pub const ALL: [IssueCode; 15] = [
        IssueCode::NoCut,
        IssueCode::Open,
        IssueCode::LooseEnd,
        IssueCode::Gap,
        IssueCode::TooComplex,
        IssueCode::CutOverCrease,
        IssueCode::Duplicate,
        IssueCode::DanglingCrease,
        IssueCode::NoSplit,
        IssueCode::Outside,
        IssueCode::NotStraight,
        IssueCode::Sliver,
        IssueCode::Ambiguous,
        IssueCode::LoosePanel,
        IssueCode::NonPrinting,
    ];

    /// The code's stable id.
    pub fn id(self) -> &'static str {
        match self {
            IssueCode::NoCut => "noCut",
            IssueCode::Open => "open",
            IssueCode::LooseEnd => "looseEnd",
            IssueCode::Gap => "gap",
            IssueCode::TooComplex => "tooComplex",
            IssueCode::CutOverCrease => "cutOverCrease",
            IssueCode::Duplicate => "duplicate",
            IssueCode::DanglingCrease => "danglingCrease",
            IssueCode::NoSplit => "noSplit",
            IssueCode::Outside => "outside",
            IssueCode::NotStraight => "notStraight",
            IssueCode::Sliver => "sliver",
            IssueCode::Ambiguous => "ambiguous",
            IssueCode::LoosePanel => "loosePanel",
            IssueCode::NonPrinting => "nonPrinting",
        }
    }

    /// Whether it is an error (the blank can't be made as drawn) rather than a warning.
    pub fn is_error(self) -> bool {
        matches!(self, IssueCode::NoCut | IssueCode::Open | IssueCode::LooseEnd | IssueCode::TooComplex | IssueCode::CutOverCrease)
    }
}

/// One issue: its code, the lines it is about, and the place it is at, when it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub code: IssueCode,
    pub keys: Vec<u64>,
    pub at: Option<Point>,
}

impl Issue {
    fn new(code: IssueCode, mut keys: Vec<u64>, at: Option<Point>) -> Self {
        keys.sort_unstable();
        keys.dedup();
        Issue { code, keys, at }
    }
}

/// One subpath of a fold line.
struct Piece {
    sp: SubPath,
    line: usize,
}

/// The blanks `lines` make, and their issues in [`IssueCode`] order. `tolerance` (points) is how
/// far apart a Cut line's end and another Cut line may be and still be joined, by a line [`blanks`]
/// adds, and how far a hinge's pieces may stray from one straight line. Nothing is built when there
/// is no Cut line, the lines are too complex, or the Cut lines enclose no area.
pub fn blanks(lines: &[FoldLine], tolerance: f64) -> (Vec<Blank>, Vec<Issue>) {
    let tolerance = if tolerance.is_finite() && tolerance > 0.0 { tolerance } else { DEFAULT_PRECISION };
    // Counted before anything is copied.
    let segments = lines.iter().flat_map(|l| &l.path.subpaths).filter(|sp| usable(sp)).try_fold(0usize, |n, sp| {
        let n = n.saturating_add(sp.segment_count());
        (n <= MAX_SEGMENTS).then_some(n)
    });
    if segments.is_none() {
        return (vec![], vec![Issue::new(IssueCode::TooComplex, vec![], None)]);
    }
    let pieces: Vec<Piece> = lines
        .iter()
        .enumerate()
        .flat_map(|(line, l)| l.path.subpaths.iter().filter(|sp| usable(sp)).map(move |sp| Piece { sp: sp.clone(), line }))
        .collect();
    let mut issues = vec![];
    let kind = |p: &Piece| lines.get(p.line).map_or(LineKind::Cut, |l| l.kind);
    let key = |p: &Piece| lines.get(p.line).map_or(0, |l| l.key);
    let cuts: Vec<&Piece> = pieces.iter().filter(|p| kind(p) == LineKind::Cut).collect();
    if cuts.is_empty() {
        return (vec![], vec![Issue::new(IssueCode::NoCut, vec![], None)]);
    }
    let folds: Vec<&Piece> = pieces.iter().filter(|p| kind(p) != LineKind::Cut).collect();
    if !gap_work_fits(&cuts, tolerance) {
        return (vec![], vec![Issue::new(IssueCode::TooComplex, vec![], None)]);
    }
    let cut_shapes: Vec<Shape> = cuts.iter().map(|p| shape(p.sp.clone())).collect();
    let gaps = find_gaps(&cut_shapes, tolerance);
    // `find_gaps` takes ends closer than 1/10,000 of the tolerance as joined, which the map may not
    // join: those are bridged too, without an issue.
    let joined = find_gaps(&cut_shapes, (tolerance * 1e-4).max(1e-6));
    // An end a gap starts or ends at is not loose.
    let gap_ends =
        Cells::new(gaps.iter().chain(&joined).flatten().map(|&e| (Rect::from_points(e, e), (e.x.to_bits(), e.y.to_bits()))).collect(), 0.0, 4096.0);
    let bridged = |at: Point| gap_ends.near(at, 1e-6).into_iter().any(|(x, y)| Point::new(f64::from_bits(x), f64::from_bits(y)).distance(at) <= 1e-6);
    issues.extend(
        loose_ends(&cuts, &folds, tolerance)
            .into_iter()
            .filter(|&(_, at)| !bridged(at))
            .map(|(i, at)| Issue::new(IssueCode::LooseEnd, vec![cuts.get(i).map_or(0, |p| key(p))], Some(at))),
    );
    if !gaps.is_empty() {
        // The cut lines by their ends, to name the line each gap starts from.
        let at_ends = cuts.iter().enumerate().flat_map(|(i, p)| ends(&p.sp).into_iter().map(move |e| (Rect::from_points(e, e), (i, e))));
        let by_end = Cells::new(at_ends.map(|(r, (i, e))| (r, (i, e.x.to_bits(), e.y.to_bits()))).collect(), 0.0, 4096.0);
        for g in &gaps {
            let of = by_end
                .near(g[0], 1e-6)
                .into_iter()
                .find(|&(_, x, y)| Point::new(f64::from_bits(x), f64::from_bits(y)).distance(g[0]) <= 1e-6)
                .and_then(|(i, _, _)| cuts.get(i))
                .map(|p| key(p));
            issues.push(Issue::new(IssueCode::Gap, of.into_iter().collect(), Some(g[0])));
        }
    }
    let gaps: Vec<[Point; 2]> = gaps.into_iter().chain(joined).collect();
    let gap_shape = shape_of_gaps(&gaps);
    let Some(material) = Material::new(&cuts, &gaps, cut_shapes.into_iter().chain(gap_shape.clone()).collect()) else {
        issues.push(Issue::new(IssueCode::TooComplex, vec![], None));
        return (vec![], sorted(issues));
    };
    if material.blanks.is_empty() {
        issues.push(Issue::new(IssueCode::Open, cuts.iter().map(|p| key(p)).collect(), None));
        return (vec![], sorted(issues));
    }
    let all: Vec<Shape> = pieces.iter().map(|p| shape(p.sp.clone())).chain(gap_shape).collect();
    let Some(full) = adjacency(&all, DEFAULT_PRECISION) else {
        issues.push(Issue::new(IssueCode::TooComplex, vec![], None));
        return (vec![], sorted(issues));
    };
    let cut_lines: Vec<usize> = cuts.iter().map(|p| p.line).collect();
    let ctx = Context { lines, pieces: &pieces, cut_lines: &cut_lines, material: &material, full: &full, tolerance };
    let built = ctx.build(&mut issues);
    (built, sorted(issues))
}

/// Most (open end, Cut segment) pairs within the tolerance of each other that the gap search may
/// look at.
const MAX_GAP_CANDIDATES: usize = 1 << 24;

/// Whether the gap search over `cuts` stays within [`MAX_GAP_CANDIDATES`]: it looks at the Cut
/// segments within `tolerance` of each open end, and lines drawn on top of each other many times
/// over make that work grow with the square of their number. The count stops at the limit.
fn gap_work_fits(cuts: &[&Piece], tolerance: f64) -> bool {
    let boxes: Vec<(Rect, usize)> = cuts
        .iter()
        .flat_map(|p| {
            PathData::single(p.sp.clone()).to_bezpath().segments().map(|s| s.bounding_box().inflate(tolerance, tolerance)).collect::<Vec<_>>()
        })
        .enumerate()
        .map(|(i, b)| (b, i))
        .collect();
    let grid = Cells::new(boxes, 2.0 * tolerance, 4096.0);
    let mut work = 0usize;
    for e in cuts.iter().flat_map(|p| ends(&p.sp)) {
        work = work.saturating_add(grid.near(e, 0.0).len());
        if work > MAX_GAP_CANDIDATES {
            return false;
        }
    }
    true
}

fn sorted(mut issues: Vec<Issue>) -> Vec<Issue> {
    issues.sort_by_key(|i| i.code);
    issues
}

/// A subpath with two anchors or more, every coordinate finite.
fn usable(sp: &SubPath) -> bool {
    sp.anchors.len() >= 2 && sp.anchors.iter().all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite())
}

fn shape(sp: SubPath) -> Shape {
    Shape::new(PathData::single(sp), FillRule::NonZero, 0)
}

/// The lines across `gaps`, as one input.
fn shape_of_gaps(gaps: &[[Point; 2]]) -> Option<Shape> {
    (!gaps.is_empty()).then(|| Shape::new(PathData::new(gaps.iter().map(|g| SubPath::polyline(g, false)).collect()), FillRule::NonZero, 0))
}

/// The two ends of an open subpath; none for a closed one or one that ends where it starts.
fn ends(sp: &SubPath) -> Vec<Point> {
    match (sp.anchors.first(), sp.anchors.last()) {
        (Some(a), Some(b)) if !sp.closed && a.p.distance(b.p) > 1e-6 => vec![a.p, b.p],
        _ => vec![],
    }
}

/// Whether `sp` encloses an area: closed, or open with its ends together.
fn is_loop(sp: &SubPath) -> bool {
    ends(sp).is_empty() && sp.anchors.len() >= 2
}

/// Most grid cells one item is filed under by [`Cells`]; a larger one is tried for every point.
const SPAN: usize = 1024;

/// The farthest cell from the grid's origin, on each axis, that [`Cells`] numbers a point in.
const CELL_LIMIT: f64 = 1_048_576.0;

/// Most (cell, item) entries [`Cells`] holds in all; the items past it are tried for every point, as
/// large ones are.
const CELL_ENTRIES: usize = 1 << 21;

/// Items (segments, faces, ends) by the cells of a square grid that their bounds cover, so that
/// finding the items near a point tests only those in its cells.
struct Cells<T> {
    origin: Point,
    cell: f64,
    cells: HashMap<(i64, i64), Vec<T>>,
    /// Items whose bounds cover more than [`SPAN`] cells, near every point.
    wide: Vec<T>,
}

impl<T: Copy + Ord> Cells<T> {
    /// `items` by their bounds, in cells `cell` wide or `1/divisions` of all the bounds where that is
    /// wider.
    fn new(items: Vec<(Rect, T)>, cell: f64, divisions: f64) -> Self {
        let bounds = items.iter().map(|(b, _)| *b).filter(Rect::is_finite).reduce(|a, b| a.union(b)).unwrap_or_default();
        let cell = cell.max(bounds.width().max(bounds.height()) / divisions);
        let cell = if cell.is_finite() && cell > 0.0 { cell } else { 1.0 };
        let mut grid = Cells { origin: bounds.origin(), cell, cells: HashMap::new(), wide: vec![] };
        let mut entries = 0usize;
        for (b, item) in items {
            let ((x0, y0), (x1, y1)) = (grid.cell_of(Point::new(b.x0, b.y0)), grid.cell_of(Point::new(b.x1, b.y1)));
            let span = usize::try_from((x1 - x0 + 1).saturating_mul(y1 - y0 + 1)).unwrap_or(usize::MAX);
            if span > SPAN || entries.saturating_add(span) > CELL_ENTRIES {
                grid.wide.push(item);
                continue;
            }
            entries += span;
            for x in x0..=x1 {
                for y in y0..=y1 {
                    grid.cells.entry((x, y)).or_default().push(item);
                }
            }
        }
        grid
    }

    fn cell_of(&self, p: Point) -> (i64, i64) {
        let at = |v: f64, o: f64| ((v - o) / self.cell).floor().clamp(-CELL_LIMIT, CELL_LIMIT) as i64;
        (at(p.x, self.origin.x), at(p.y, self.origin.y))
    }

    /// The items whose bounds come within `r` of `p` (and some farther ones), each once. When `r`
    /// spans more cells than hold items, the cells that hold items are read instead.
    fn near(&self, p: Point, r: f64) -> Vec<T> {
        let mut out = self.wide.clone();
        let ((x0, y0), (x1, y1)) = (self.cell_of(Point::new(p.x - r, p.y - r)), self.cell_of(Point::new(p.x + r, p.y + r)));
        let span = (x1 - x0 + 1).saturating_mul(y1 - y0 + 1);
        if usize::try_from(span).map_or(true, |n| n > self.cells.len()) {
            for (&(x, y), items) in &self.cells {
                if (x0..=x1).contains(&x) && (y0..=y1).contains(&y) {
                    out.extend(items);
                }
            }
        } else {
            for x in x0..=x1 {
                for y in y0..=y1 {
                    out.extend(self.cells.get(&(x, y)).into_iter().flatten());
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The ends of `cuts` that lie more than `tolerance` from every other fold line among `cuts` and
/// `folds`, as (index into `cuts`, end). A line's own points count when they are more than
/// `tolerance` from the end along the line, so a cut line that comes back to its own end meets it.
fn loose_ends(cuts: &[&Piece], folds: &[&Piece], tolerance: f64) -> Vec<(usize, Point)> {
    let segs: Vec<Vec<PathSeg>> = cuts.iter().chain(folds).map(|p| PathData::single(p.sp.clone()).to_bezpath().segments().collect()).collect();
    let boxes = segs.iter().enumerate().flat_map(|(j, ss)| ss.iter().enumerate().map(move |(k, s)| (s.bounding_box(), (j, k))));
    // The cells are as wide as the tolerance, or 1/4096 of the lines where that is wider.
    let grid = Cells::new(boxes.collect(), tolerance, 4096.0);
    let mut out = vec![];
    for (i, p) in cuts.iter().enumerate() {
        let own = segs.get(i).map(Vec::as_slice).unwrap_or_default();
        // How far along the line each of its segments starts, and the line's length.
        let starts: Vec<f64> = own.iter().scan(0.0, |at, s| Some(std::mem::replace(at, *at + s.arclen(1e-3)))).collect();
        let total = own.last().zip(starts.last()).map_or(0.0, |(s, at)| at + s.arclen(1e-3));
        for (k, e) in ends(&p.sp).into_iter().enumerate() {
            let near = grid.near(e, tolerance).into_iter().any(|(j, s)| {
                let Some(seg) = segs.get(j).and_then(|ss| ss.get(s)) else { return false };
                let b = seg.bounding_box();
                if e.x < b.x0 - tolerance || e.x > b.x1 + tolerance || e.y < b.y0 - tolerance || e.y > b.y1 + tolerance {
                    return false;
                }
                let hit = seg.nearest(e, 1e-9);
                if hit.distance_sq.sqrt() > tolerance {
                    return false;
                }
                if j != i {
                    return true;
                }
                let along = starts.get(s).copied().unwrap_or(0.0) + seg.subsegment(0.0..hit.t).arclen(1e-3);
                (if k == 0 { along } else { total - along }) > tolerance
            });
            if !near {
                out.push((i, e));
            }
        }
    }
    out
}

/// A point inside `path`, or the middle of its bounds for one too thin to find one in.
fn point_in(path: &PathData) -> Option<Point> {
    interior_point(path).or_else(|| path.bounds().map(|b| b.center()))
}

fn contains(bp: &BezPath, p: Point) -> bool {
    bp.winding(p) != 0
}

/// The loops that open Cut pieces and the lines across `gaps` make end to end: each chain of them
/// whose every end meets exactly one end of another piece of the chain (within 1e-6), as (index
/// into `cuts` of its first Cut piece, closed path). Pieces that lie on no cycle are left out first.
/// A connected set of the remaining pieces with an end where three or more meet gives the outlines
/// of the faces it encloses instead.
fn chains(cuts: &[&Piece], gaps: &[[Point; 2]]) -> Vec<(usize, BezPath)> {
    const NEAR: f64 = 1e-6;
    // Cell numbers stay far from the ends of `i64`, so a neighbor's number does not overflow.
    let cell = |v: f64| (v / NEAR).round().clamp(-1e15, 1e15) as i64;
    let key = |p: Point| (cell(p.x), cell(p.y));
    // The ends by node: ends within `NEAR` of a node's first end share it.
    let mut nodes: HashMap<(i64, i64), usize> = HashMap::new();
    let mut firsts: Vec<Point> = vec![];
    let mut node_of = |p: Point| {
        let (kx, ky) = key(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(&n) = nodes.get(&(kx.saturating_add(dx), ky.saturating_add(dy)))
                    && firsts.get(n).is_some_and(|q| q.distance(p) <= NEAR)
                {
                    return n;
                }
            }
        }
        firsts.push(p);
        nodes.insert((kx, ky), firsts.len() - 1);
        firsts.len() - 1
    };
    // The open Cut pieces (with their index into `cuts`) and the gap lines, each with its segments.
    let pieces = cuts
        .iter()
        .enumerate()
        .filter(|(_, p)| ends(&p.sp).len() == 2)
        .map(|(i, p)| (Some(i), PathData::single(p.sp.clone()).to_bezpath().segments().collect()));
    let lines = gaps.iter().map(|g| (None, vec![PathSeg::Line(Line::new(g[0], g[1]))]));
    let items: Vec<(Option<usize>, Vec<PathSeg>)> = pieces.chain(lines).collect();
    // (index into `items`, start node, end node) of each, and the pieces at each node.
    let open: Vec<(usize, usize, usize)> =
        items.iter().enumerate().filter_map(|(k, (_, segs))| Some((k, node_of(segs.first()?.start()), node_of(segs.last()?.end())))).collect();
    let mut at: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, &(_, a, b)) in open.iter().enumerate() {
        at.entry(a).or_default().push(k);
        at.entry(b).or_default().push(k);
    }
    // A piece that lies on no cycle of pieces (a slit, a line from another one's side, a cut that
    // joins two loops) is no part of a loop: such pieces are dropped, so that a slit at a loop's
    // corner does not stop the loop.
    let live: Vec<bool> = bridges(firsts.len(), &open).into_iter().map(|b| !b).collect();
    for ks in at.values_mut() {
        ks.retain(|&k| live.get(k) == Some(&true));
    }
    // The connected pieces of what is left, by node. In a piece where some node joins more than two
    // pieces (a window with a cut across it), the loop is the outline of the faces the piece
    // encloses; the other pieces are walked end to end below.
    let mut group: Vec<usize> = (0..firsts.len()).collect();
    for (k, &(_, a, b)) in open.iter().enumerate() {
        if live.get(k) == Some(&true) {
            let (ra, rb) = (root_of(&mut group, a), root_of(&mut group, b));
            if let Some(slot) = group.get_mut(ra) {
                *slot = rb;
            }
        }
    }
    let mut branched: HashSet<usize> = HashSet::new();
    for (&n, ks) in &at {
        if !ks.is_empty() && ks.len() != 2 {
            branched.insert(root_of(&mut group, n));
        }
    }
    let mut used: Vec<bool> = live.iter().map(|l| !l).collect();
    let mut out = vec![];
    let mut joined: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, &(_, a, _)) in open.iter().enumerate() {
        let r = root_of(&mut group, a);
        if used.get(k) == Some(&false) && branched.contains(&r) {
            joined.entry(r).or_default().push(k);
            if let Some(u) = used.get_mut(k) {
                *u = true;
            }
        }
    }
    let mut joined: Vec<Vec<usize>> = joined.into_values().collect();
    joined.sort_unstable();
    for ks in joined {
        let Some(first_cut) = ks.iter().filter_map(|&k| open.get(k).and_then(|&(j, _, _)| items.get(j)).and_then(|(i, _)| *i)).min() else {
            continue;
        };
        let shapes: Vec<Shape> = ks
            .iter()
            .filter_map(|&k| open.get(k).and_then(|&(j, _, _)| items.get(j)))
            .map(|(_, segs)| Shape::new(PathData::from_bezpath(&BezPath::from_path_segments(segs.iter().copied())), FillRule::NonZero, 0))
            .collect();
        out.extend(enclosed_loops(&shapes).into_iter().map(|bp| (first_cut, bp)));
    }
    for start in 0..open.len() {
        if used.get(start) != Some(&false) {
            continue;
        }
        // Walk from the start piece's first end until the chain comes back to it, or stops.
        let Some(&(_, first_node, _)) = open.get(start) else { continue };
        let (mut k, mut from, mut walk) = (start, first_node, vec![]);
        let closed = loop {
            let Some(&(_, a, b)) = open.get(k) else { break false };
            if used.get(k) != Some(&false) {
                break false;
            }
            if let Some(u) = used.get_mut(k) {
                *u = true;
            }
            let forward = a == from;
            walk.push((k, forward));
            let to = if forward { b } else { a };
            if to == first_node {
                break true;
            }
            let next = at.get(&to).filter(|ks| ks.len() == 2).and_then(|ks| ks.iter().copied().find(|&j| j != k));
            match next {
                Some(j) => (k, from) = (j, to),
                None => break false,
            }
        };
        if !closed || at.get(&first_node).is_none_or(|ks| ks.len() != 2) {
            continue;
        }
        let mut bp = BezPath::new();
        let mut first_cut = None;
        for (n, &(k, forward)) in walk.iter().enumerate() {
            let Some((i, segs)) = open.get(k).and_then(|&(j, _, _)| items.get(j)) else { continue };
            first_cut = first_cut.or(*i);
            let segs: Vec<PathSeg> = if forward { segs.clone() } else { segs.iter().rev().map(|s| s.reverse()).collect() };
            for (m, s) in segs.iter().enumerate() {
                if n == 0 && m == 0 {
                    bp.move_to(s.start());
                }
                match *s {
                    PathSeg::Line(l) => bp.line_to(l.p1),
                    PathSeg::Quad(q) => bp.quad_to(q.p1, q.p2),
                    PathSeg::Cubic(c) => bp.curve_to(c.p1, c.p2, c.p3),
                }
            }
        }
        bp.close_path();
        if let Some(i) = first_cut {
            out.push((i, bp));
        }
    }
    out
}

/// The loops of a connected set of open Cut pieces `shapes` where some end joins three pieces or
/// more: the outline of all the faces they enclose, and for each group of faces that meet only other
/// faces and are bounded by pieces that meet end to end (a window joined to an outline by two slits,
/// not the middle cell of cut lines that cross), the group's outline, as the pieces of a chain that
/// closes count as one closed Cut line.
fn enclosed_loops(shapes: &[Shape]) -> Vec<BezPath> {
    let Some(map) = adjacency(shapes, DEFAULT_PRECISION) else { return vec![] };
    let mut inner = vec![true; map.faces.len()];
    for e in &map.edges {
        for (a, b) in [(e.left, e.right), (e.right, e.left)] {
            if b.is_none()
                && let Some(slot) = a.and_then(|f| inner.get_mut(f))
            {
                *slot = false;
            }
        }
    }
    let mut group: Vec<usize> = (0..map.faces.len()).collect();
    for e in &map.edges {
        if let (Some(a), Some(b)) = (e.left, e.right)
            && a != b
            && inner.get(a) == Some(&true)
            && inner.get(b) == Some(&true)
        {
            let (ra, rb) = (root_of(&mut group, a), root_of(&mut group, b));
            if let Some(slot) = group.get_mut(ra) {
                *slot = rb;
            }
        }
    }
    let mut sets: HashMap<usize, Vec<usize>> = HashMap::new();
    for f in (0..map.faces.len()).filter(|&f| inner.get(f) == Some(&true)) {
        sets.entry(root_of(&mut group, f)).or_default().push(f);
    }
    // A group is a loop when its boundary edges come from pieces that lie on that boundary alone,
    // that is, pieces that meet end to end round it; pieces of lines that cross it go on past it.
    let mut edges_of: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, e) in map.edges.iter().enumerate() {
        for &s in &e.sources {
            edges_of.entry(s).or_default().push(i);
        }
    }
    let side = |g: &mut Vec<usize>, f: Option<usize>| f.filter(|&f| inner.get(f) == Some(&true)).map(|f| root_of(g, f));
    let mut boundary: HashMap<usize, HashSet<usize>> = HashMap::new();
    for (i, e) in map.edges.iter().enumerate() {
        let (l, r) = (side(&mut group, e.left), side(&mut group, e.right));
        if l != r {
            for root in [l, r].into_iter().flatten() {
                boundary.entry(root).or_default().insert(i);
            }
        }
    }
    let whole = |root: &usize| {
        boundary.get(root).is_some_and(|edges| {
            edges
                .iter()
                .filter_map(|&i| map.edges.get(i))
                .flat_map(|e| &e.sources)
                .all(|s| edges_of.get(s).is_some_and(|all| all.iter().all(|i| edges.contains(i))))
        })
    };
    let mut sets: Vec<(usize, Vec<usize>)> = sets.into_iter().filter(|(root, _)| whole(root)).collect();
    sets.push((usize::MAX, (0..map.faces.len()).collect()));
    sets.sort_unstable();
    let mut out = vec![];
    for (_, set) in sets {
        let faces: Vec<(&PathData, FillRule)> = set.iter().filter_map(|&f| map.faces.get(f)).map(|f| (&f.path, FillRule::NonZero)).collect();
        if !faces.is_empty() {
            out.extend(unite_all(&faces).subpaths.into_iter().map(|sp| PathData::single(sp).to_bezpath()));
        }
    }
    out
}

/// The node that stands for `v`'s set in `group` (a disjoint-set forest), halving the path on the way.
fn root_of(group: &mut [usize], mut v: usize) -> usize {
    while let Some(&up) = group.get(v) {
        if up == v {
            break;
        }
        let next = group.get(up).copied().unwrap_or(up);
        if let Some(slot) = group.get_mut(v) {
            *slot = next;
        }
        v = next;
    }
    v
}

/// Which of `edges` ((id, end node, end node), nodes below `nodes`) lie on no cycle: the bridges of
/// the graph, found depth first without recursion.
fn bridges(nodes: usize, edges: &[(usize, usize, usize)]) -> Vec<bool> {
    let mut adj: Vec<Vec<(usize, usize)>> = vec![vec![]; nodes];
    for (k, &(_, a, b)) in edges.iter().enumerate() {
        adj.get_mut(a).into_iter().for_each(|l| l.push((b, k)));
        adj.get_mut(b).into_iter().for_each(|l| l.push((a, k)));
    }
    let mut bridge = vec![false; edges.len()];
    let (mut disc, mut low) = (vec![usize::MAX; nodes], vec![0usize; nodes]);
    let mut timer = 0usize;
    for s in 0..nodes {
        if disc.get(s) != Some(&usize::MAX) {
            continue;
        }
        let mut stamp = |v: usize, disc: &mut Vec<usize>, low: &mut Vec<usize>| {
            disc.get_mut(v).into_iter().for_each(|d| *d = timer);
            low.get_mut(v).into_iter().for_each(|l| *l = timer);
            timer += 1;
        };
        stamp(s, &mut disc, &mut low);
        // (node, the edge it was reached by, the next of its edges to follow)
        let mut stack: Vec<(usize, Option<usize>, usize)> = vec![(s, None, 0)];
        while let Some(&(v, by, i)) = stack.last() {
            if let Some(&(w, e)) = adj.get(v).and_then(|l| l.get(i)) {
                if let Some(top) = stack.last_mut() {
                    top.2 += 1;
                }
                if Some(e) == by {
                    continue;
                }
                match disc.get(w).copied() {
                    Some(usize::MAX) => {
                        stamp(w, &mut disc, &mut low);
                        stack.push((w, Some(e), 0));
                    }
                    Some(d) => {
                        if let Some(l) = low.get_mut(v) {
                            *l = (*l).min(d);
                        }
                    }
                    None => {}
                }
            } else {
                stack.pop();
                let low_v = low.get(v).copied().unwrap_or(0);
                if let Some(&(u, _, _)) = stack.last() {
                    if let Some(l) = low.get_mut(u) {
                        *l = (*l).min(low_v);
                    }
                    if disc.get(u).is_some_and(|&d| low_v > d)
                        && let Some(b) = by.and_then(|e| bridge.get_mut(e))
                    {
                        *b = true;
                    }
                }
            }
        }
    }
    bridge
}

/// The material the Cut lines leave: the faces of their planar map, which are material, which are
/// holes, and the blanks (connected pieces with material).
struct Material {
    map: Adjacency,
    /// For each face of `map`: a point inside it, whether it is material, whether it is a hole.
    points: Vec<Option<Point>>,
    material: Vec<bool>,
    hole: Vec<bool>,
    /// The pieces of `map` that are blanks, in order, and each one's outline.
    blanks: Vec<usize>,
    bounds: Vec<BezPath>,
    /// For each piece of `map`, the blank (index into `blanks`) it is part of: its own, or for a
    /// piece that lies in a blank's material (a window), that blank.
    owner: Vec<Option<usize>>,
    /// The faces of each piece of `map`, and its edges.
    faces_by_piece: Vec<Vec<usize>>,
    edges_by_piece: Vec<Vec<usize>>,
    /// Each face's outer boundary and its whole path (with its holes), and the faces by where they
    /// are.
    outer: Vec<BezPath>,
    whole: Vec<BezPath>,
    face_cells: Cells<usize>,
    /// The blanks by where their outlines are (built once every piece is classified).
    bound_cells: Cells<usize>,
}

/// A loop among the Cut lines: its path and its area.
type Loop = (BezPath, f64);

impl Material {
    /// The material of `cuts` with the lines across `gaps`, mapped as `shapes` (the cuts, then the
    /// gap lines).
    fn new(cuts: &[&Piece], gaps: &[[Point; 2]], shapes: Vec<Shape>) -> Option<Self> {
        let map = adjacency(&shapes, DEFAULT_PRECISION)?;
        let points: Vec<Option<Point>> = map.faces.iter().map(|f| point_in(&f.path)).collect();
        let pieces = map.faces.iter().map(|f| f.piece + 1).chain(map.edges.iter().map(|e| e.piece + 1)).max().unwrap_or(0);
        let mut faces_by_piece = vec![vec![]; pieces];
        for (f, face) in map.faces.iter().enumerate() {
            if let Some(list) = faces_by_piece.get_mut(face.piece) {
                list.push(f);
            }
        }
        // The piece each input is part of, from the edges it lies on.
        let mut piece_of_input: Vec<Option<usize>> = vec![None; shapes.len()];
        let mut edges_by_piece = vec![vec![]; pieces];
        for (e, edge) in map.edges.iter().enumerate() {
            if let Some(list) = edges_by_piece.get_mut(edge.piece) {
                list.push(e);
            }
            for &s in &edge.sources {
                if let Some(slot) = piece_of_input.get_mut(s).filter(|p| p.is_none()) {
                    *slot = Some(edge.piece);
                }
            }
        }
        let mut loops: Vec<Vec<Loop>> = vec![vec![]; pieces];
        for (i, p) in cuts.iter().enumerate().filter(|(_, p)| is_loop(&p.sp)) {
            let Some(list) = piece_of_input.get(i).copied().flatten().and_then(|piece| loops.get_mut(piece)) else { continue };
            let bp = PathData::single(SubPath::of(p.sp.anchors.clone(), true)).to_bezpath();
            let area = bp.area().abs();
            list.push((bp, area));
        }
        // Loops drawn as open pieces end to end, such as a slot of four lines.
        for (i, bp) in chains(cuts, gaps) {
            let Some(list) = piece_of_input.get(i).copied().flatten().and_then(|piece| loops.get_mut(piece)) else { continue };
            let area = bp.area().abs();
            list.push((bp, area));
        }
        // Each piece's outer boundary and a point of it.
        let mut outlines: Vec<Option<(BezPath, Option<Point>)>> = vec![None; pieces];
        for o in &map.outlines {
            if let Some(slot) = outlines.get_mut(o.piece).filter(|s| s.is_none()) {
                *slot = Some((o.path.to_bezpath(), o.path.subpaths.first().and_then(|sp| sp.anchors.first()).map(|a| a.p)));
            }
        }
        // Outer pieces first: the face a piece lies in belongs to a piece with a larger outline.
        let mut order: Vec<(usize, f64, Option<Point>)> =
            outlines.iter().enumerate().map(|(p, o)| (p, o.as_ref().map_or(0.0, |o| o.0.area().abs()), o.as_ref().and_then(|o| o.1))).collect();
        order.sort_by(|a, b| b.1.total_cmp(&a.1));
        let outer: Vec<BezPath> =
            map.faces.iter().map(|f| f.path.subpaths.first().map(|sp| PathData::single(sp.clone()).to_bezpath()).unwrap_or_default()).collect();
        let whole: Vec<BezPath> = map.faces.iter().map(|f| f.path.to_bezpath()).collect();
        let face_cells = Cells::new(outer.iter().enumerate().map(|(f, bp)| (bp.bounding_box(), f)).collect(), 0.0, 256.0);
        let faces = map.faces.len();
        let mut m = Material {
            map,
            points,
            material: vec![false; faces],
            hole: vec![false; faces],
            blanks: vec![],
            bounds: vec![],
            owner: vec![None; pieces],
            faces_by_piece,
            edges_by_piece,
            outer,
            whole,
            face_cells,
            bound_cells: Cells::new(vec![], 0.0, 256.0),
        };
        for (piece, _, first) in order {
            match first.and_then(|q| m.around(piece, q)) {
                // A piece in a blank's material is a hole in it, a window.
                Some(f) if m.material.get(f) == Some(&true) => {
                    let owner = m.map.faces.get(f).and_then(|x| m.owner.get(x.piece)).copied().flatten();
                    for g in m.faces_by_piece.get(piece).cloned().unwrap_or_default() {
                        let usable = m.points.get(g).copied().flatten().is_some();
                        if let Some(h) = m.hole.get_mut(g) {
                            *h = usable;
                        }
                    }
                    if let Some(o) = m.owner.get_mut(piece) {
                        *o = owner;
                    }
                }
                _ => {
                    let own = loops.get(piece).map(Vec::as_slice).unwrap_or_default();
                    let fallback = outlines.get(piece).cloned().flatten().map(|o| o.0);
                    m.classify(piece, own, fallback);
                }
            }
        }
        m.bound_cells = Cells::new(m.bounds.iter().enumerate().map(|(b, bp)| (bp.bounding_box(), b)).collect(), 0.0, 256.0);
        Some(m)
    }

    /// The face of another piece than `piece` whose outer boundary holds `q`, a point of its
    /// outline, the one with the smallest outer boundary when several do. The boundaries are
    /// compared, not the faces' own areas, which leave out the holes in them.
    fn around(&self, piece: usize, q: Point) -> Option<usize> {
        let outer_area = |f: usize| self.outer.get(f).map_or(f64::INFINITY, |bp| bp.area().abs());
        self.face_cells
            .near(q, 0.0)
            .into_iter()
            .filter(|&f| self.map.faces.get(f).is_some_and(|x| x.piece != piece))
            .filter(|&f| self.outer.get(f).is_some_and(|bp| contains(bp, q)))
            .min_by(|&a, &b| outer_area(a).total_cmp(&outer_area(b)))
    }

    fn area(&self, f: usize) -> f64 {
        self.map.faces.get(f).map_or(f64::INFINITY, |x| x.area)
    }

    /// Which faces of `piece` are material and which are holes, by its own loops `loops`; with
    /// material, the piece is a blank. A loop that holds the same faces as a larger one (a loop
    /// drawn twice) counts once. Its outlines are its largest loop and each smaller loop that shares
    /// no face with a larger outline (two outlines joined by a cut line); its other loops cut
    /// material out. When the outlines and the loops that cut out do not cover every face (an
    /// outline drawn as open lines), every loop cuts out and the blank is bounded by `fallback`, its
    /// outer boundary.
    fn classify(&mut self, piece: usize, loops: &[Loop], fallback: Option<BezPath>) {
        let faces = self.faces_by_piece.get(piece).cloned().unwrap_or_default();
        let mut by_area: Vec<&Loop> = loops.iter().collect();
        by_area.sort_by(|a, b| b.1.total_cmp(&a.1));
        let points = &self.points;
        let unusable = |f: usize| points.get(f).copied().flatten().is_none();
        // The piece's faces by their points, so that each loop tests only the faces near it.
        let at: Vec<(Rect, usize)> =
            faces.iter().enumerate().filter_map(|(k, &f)| points.get(f).copied().flatten().map(|p| (Rect::from_points(p, p), k))).collect();
        let extent = at.iter().map(|(r, _)| *r).reduce(|a, b| a.union(b));
        let near = Cells::new(at, 0.0, 256.0);
        // Each loop with the faces of the piece it holds (positions in `faces`, ascending); loops
        // that hold none, and loops that hold the same faces as a larger one, are left out.
        let mut seen: HashSet<Vec<usize>> = HashSet::new();
        let mut own: Vec<(&BezPath, Vec<usize>)> = vec![];
        for l in by_area {
            let b = l.0.bounding_box().inflate(1e-6, 1e-6);
            // Only the part of the loop's bounds where faces are is looked up, a few hundred cells
            // across at most.
            let Some(q) = extent.map(|e| b.intersect(e)).filter(|q| q.width() >= 0.0 && q.height() >= 0.0) else { continue };
            let holds: Vec<usize> = near
                .near(q.center(), q.width().max(q.height()) / 2.0)
                .into_iter()
                .filter(|&k| faces.get(k).and_then(|&f| points.get(f).copied().flatten()).is_some_and(|p| b.contains(p) && contains(&l.0, p)))
                .collect();
            if !holds.is_empty() && seen.insert(holds.clone()) {
                own.push((&l.0, holds));
            }
        }
        // Outlines go largest first; a loop holding a face an outline holds cuts out instead.
        let (mut outlines, mut others): (Vec<usize>, Vec<usize>) = (vec![], vec![]);
        let mut in_an_outline = vec![false; faces.len()];
        let mut held = vec![false; faces.len()];
        for (i, (_, holds)) in own.iter().enumerate() {
            holds.iter().for_each(|&k| held.get_mut(k).into_iter().for_each(|h| *h = true));
            if holds.iter().any(|&k| in_an_outline.get(k) == Some(&true)) {
                others.push(i);
            } else {
                holds.iter().for_each(|&k| in_an_outline.get_mut(k).into_iter().for_each(|h| *h = true));
                outlines.push(i);
            }
        }
        let covered = faces.iter().enumerate().all(|(k, &f)| held.get(k) == Some(&true) || unusable(f));
        if !covered {
            outlines.clear();
            others = (0..own.len()).collect();
        }
        let mut cut = vec![false; faces.len()];
        for (_, holds) in others.iter().filter_map(|&i| own.get(i)) {
            holds.iter().for_each(|&k| cut.get_mut(k).into_iter().for_each(|c| *c = true));
        }
        let mut any = false;
        for (k, &f) in faces.iter().enumerate() {
            let in_outline = outlines.is_empty() || in_an_outline.get(k) == Some(&true);
            let cut_out = cut.get(k) == Some(&true);
            let material = !unusable(f) && in_outline && !cut_out;
            any |= material;
            if let Some(m) = self.material.get_mut(f) {
                *m = material;
            }
            if let Some(h) = self.hole.get_mut(f) {
                *h = !unusable(f) && in_outline && cut_out;
            }
        }
        self.open_notches(piece, fallback.as_ref());
        if any {
            let bound = if outlines.is_empty() {
                fallback
            } else {
                let mut all = BezPath::new();
                outlines.iter().filter_map(|&o| own.get(o)).for_each(|o| all.extend(o.0.elements().iter().copied()));
                Some(all)
            };
            if let Some(o) = self.owner.get_mut(piece) {
                *o = Some(self.blanks.len());
            }
            self.blanks.push(piece);
            self.bounds.push(bound.unwrap_or_default());
        }
    }

    /// Faces of `piece` cut out that meet the outside, directly or through other faces cut out, are
    /// a notch, not a hole: they stop being holes. The outside is a usable face that is neither
    /// material nor a hole (the part of a slot beyond the outline), or the side with no face of an
    /// edge on the piece's outer boundary `outline` (elsewhere a side with no face is a sliver).
    fn open_notches(&mut self, piece: usize, outline: Option<&BezPath>) {
        let edges = self.edges_by_piece.get(piece).cloned().unwrap_or_default();
        let hole = |s: &Self, f: Option<usize>| f.is_some_and(|f| s.hole.get(f) == Some(&true));
        let open = |s: &Self, f: Option<usize>| {
            f.is_some_and(|f| s.hole.get(f) != Some(&true) && s.material.get(f) != Some(&true) && s.points.get(f).copied().flatten().is_some())
        };
        // Whether `edge` lies on the piece's outer boundary `outline` (its middle within 1e-6 of
        // it): its side with no face is then the outside, not a sliver.
        let on_outline = |edge: &vectorcraft_pathops::MapEdge| {
            let Some(outline) = outline else { return false };
            let segs: Vec<PathSeg> = edge.path.to_bezpath().segments().collect();
            let Some(mid) = segs.get(segs.len() / 2).map(|s| s.eval(0.5)) else { return false };
            outline.segments().any(|s| s.nearest(mid, 1e-9).distance_sq <= 1e-12)
        };
        let mut next: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut queue = vec![];
        for edge in edges.iter().filter_map(|&e| self.map.edges.get(e)) {
            for (a, b) in [(edge.left, edge.right), (edge.right, edge.left)] {
                let Some(f) = a.filter(|_| a != b && hole(self, a)) else { continue };
                if open(self, b) || (b.is_none() && on_outline(edge)) {
                    queue.push(f);
                } else if let Some(g) = b.filter(|_| hole(self, b)) {
                    next.entry(f).or_default().push(g);
                }
            }
        }
        while let Some(f) = queue.pop() {
            match self.hole.get_mut(f) {
                Some(h) if *h => *h = false,
                _ => continue,
            }
            queue.extend(next.get(&f).into_iter().flatten().copied());
        }
    }

    /// The face of the Cut lines' map that holds `p`, the smallest when several do.
    fn face_at(&self, p: Point) -> Option<usize> {
        self.face_cells
            .near(p, 0.0)
            .into_iter()
            .filter(|&f| self.whole.get(f).is_some_and(|bp| contains(bp, p)))
            .min_by(|&a, &b| self.area(a).total_cmp(&self.area(b)))
    }

    /// Whether `p` lies inside the outline of a blank.
    fn in_a_blank(&self, p: Point) -> bool {
        self.bound_cells.near(p, 0.0).into_iter().any(|b| self.bounds.get(b).is_some_and(|bp| contains(bp, p)))
    }

    /// The blank (index into `blanks`) face `f` is part of.
    fn blank_of(&self, f: usize) -> Option<usize> {
        let piece = self.map.faces.get(f)?.piece;
        self.owner.get(piece).copied().flatten()
    }
}

/// What a face of the full map is.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    /// Panel `panel` of blank `blank`.
    Panel {
        blank: usize,
        panel: usize,
    },
    /// A hole of a blank.
    Hole,
    Outside,
}

/// A piece of crease between two panels of one blank.
struct HingePiece {
    path: PathData,
    lines: Vec<usize>,
}

/// A hinge piece by the (blank, panel, panel) it lies between.
type PlacedPiece = ((usize, usize, usize), HingePiece);

struct Context<'a> {
    lines: &'a [FoldLine],
    pieces: &'a [Piece],
    /// The fold line of each input of the Cut lines' map, in order; the gap lines come after them.
    cut_lines: &'a [usize],
    material: &'a Material,
    full: &'a Adjacency,
    tolerance: f64,
}

impl Context<'_> {
    /// The fold line that input `i` of the full map is part of; `None` for the gap lines.
    fn line_of(&self, i: usize) -> Option<usize> {
        self.pieces.get(i).map(|p| p.line)
    }

    fn kind_of(&self, i: usize) -> LineKind {
        self.line_of(i).and_then(|l| self.lines.get(l)).map_or(LineKind::Cut, |l| l.kind)
    }

    fn key(&self, line: usize) -> u64 {
        self.lines.get(line).map_or(0, |l| l.key)
    }

    fn keys(&self, lines: impl IntoIterator<Item = usize>) -> Vec<u64> {
        lines.into_iter().map(|l| self.key(l)).collect()
    }

    fn build(&self, issues: &mut Vec<Issue>) -> Vec<Blank> {
        let mut blanks: Vec<Blank> = vec![Blank::default(); self.material.blanks.len()];
        // Hole faces that share an edge are one hole (a cut line across a window splits it in two
        // faces): they are grouped, and each group's faces are united.
        let m = self.material;
        let is_hole = |f: usize| m.hole.get(f) == Some(&true);
        let mut group: Vec<usize> = (0..m.map.faces.len()).collect();
        let root = |group: &mut Vec<usize>, mut f: usize| {
            while let Some(&g) = group.get(f) {
                if g == f {
                    break;
                }
                let up = group.get(g).copied().unwrap_or(g);
                if let Some(slot) = group.get_mut(f) {
                    *slot = up;
                }
                f = up;
            }
            f
        };
        for e in &m.map.edges {
            if let (Some(a), Some(b)) = (e.left, e.right)
                && a != b
                && is_hole(a)
                && is_hole(b)
            {
                let (ra, rb) = (root(&mut group, a), root(&mut group, b));
                if let Some(slot) = group.get_mut(ra) {
                    *slot = rb;
                }
            }
        }
        let mut by_group: HashMap<usize, Vec<usize>> = HashMap::new();
        for f in (0..m.map.faces.len()).filter(|&f| is_hole(f)) {
            by_group.entry(root(&mut group, f)).or_default().push(f);
        }
        let mut groups: Vec<Vec<usize>> = by_group.into_values().collect();
        groups.sort_unstable();
        for faces in groups {
            let Some(b) = faces.first().and_then(|&f| m.blank_of(f)).and_then(|b| blanks.get_mut(b)) else { continue };
            let paths: Vec<&PathData> = faces.iter().filter_map(|&f| m.map.faces.get(f).map(|x| &x.path)).collect();
            let hole = match paths.as_slice() {
                [one] => (*one).clone(),
                _ => unite_all(&paths.iter().map(|p| (*p, FillRule::NonZero)).collect::<Vec<_>>()),
            };
            b.holes.push(hole);
        }
        let places = self.places(&mut blanks);
        if blanks.iter().any(|b| b.panels.len() > MAX_PANELS) {
            issues.push(Issue::new(IssueCode::TooComplex, vec![], None));
            return vec![];
        }
        let (pieces, outside) = self.edges(&places, issues);
        let hinged = self.hinges(&mut blanks, pieces, issues);
        self.creases(&places, &hinged, &outside, issues);
        self.slivers(issues);
        self.nested(issues);
        self.loose_panels(&blanks, &places, issues);
        blanks
    }

    /// What each face of the full map is, adding the panels to `blanks`.
    fn places(&self, blanks: &mut [Blank]) -> Vec<Place> {
        self.full
            .faces
            .iter()
            .map(|face| {
                let Some(c) = point_in(&face.path).and_then(|p| self.material.face_at(p)) else { return Place::Outside };
                let Some(blank) = self.material.blank_of(c) else { return Place::Outside };
                if self.material.material.get(c) == Some(&true) {
                    let Some(b) = blanks.get_mut(blank) else { return Place::Outside };
                    b.panels.push(Panel { outline: face.path.clone(), area: face.area });
                    Place::Panel { blank, panel: b.panels.len() - 1 }
                } else if self.material.hole.get(c) == Some(&true) {
                    Place::Hole
                } else {
                    Place::Outside
                }
            })
            .collect()
    }

    fn place(&self, places: &[Place], f: Option<usize>) -> Place {
        f.and_then(|f| places.get(f)).copied().unwrap_or(Place::Outside)
    }

    /// Overlaps, dangling creases and creases in holes, from the full map's edges → the hinge
    /// pieces, by (blank, panel, panel), and how long each line runs outside every blank.
    fn edges(&self, places: &[Place], issues: &mut Vec<Issue>) -> (Vec<PlacedPiece>, Vec<f64>) {
        let mut pieces = vec![];
        let mut outside = vec![0.0; self.lines.len()];
        let mut seen: HashSet<(IssueCode, Vec<u64>)> = HashSet::new();
        let mut report = |issues: &mut Vec<Issue>, code: IssueCode, keys: Vec<u64>| {
            let issue = Issue::new(code, keys, None);
            if seen.insert((code, issue.keys.clone())) {
                issues.push(issue);
            }
        };
        for e in &self.full.edges {
            let cut = e.sources.iter().any(|&s| self.kind_of(s) == LineKind::Cut);
            let creases: Vec<usize> = e.sources.iter().filter(|&&s| self.kind_of(s) != LineKind::Cut).filter_map(|&s| self.line_of(s)).collect();
            let mut by_line: Vec<usize> = e.sources.iter().filter_map(|&s| self.line_of(s)).collect();
            by_line.sort_unstable();
            by_line.dedup();
            if cut && !creases.is_empty() {
                report(issues, IssueCode::CutOverCrease, self.keys(by_line.iter().copied()));
                continue;
            }
            // Two subpaths on the same edge, from two lines or from one line's own subpaths (a
            // compound path); the gap lines are left out. Past the check above they are all Cut lines
            // or all lines that fold.
            let mut drawn: Vec<usize> = e.sources.iter().copied().filter(|&s| self.line_of(s).is_some()).collect();
            drawn.sort_unstable();
            drawn.dedup();
            if drawn.len() >= 2 {
                let mut duplicates: Vec<usize> = drawn.iter().filter_map(|&s| self.line_of(s)).collect();
                duplicates.sort_unstable();
                duplicates.dedup();
                report(issues, IssueCode::Duplicate, self.keys(duplicates));
            }
            if creases.is_empty() {
                continue;
            }
            match (self.place(places, e.left), self.place(places, e.right)) {
                (Place::Panel { blank, panel: a }, Place::Panel { blank: other, panel: b }) if blank == other && a != b => {
                    pieces.push(((blank, a.min(b), a.max(b)), HingePiece { path: e.path.clone(), lines: creases }));
                }
                (Place::Panel { .. }, Place::Panel { .. }) if e.left == e.right => {
                    report(issues, IssueCode::DanglingCrease, self.keys(creases));
                }
                (Place::Hole, _) | (_, Place::Hole) => report(issues, IssueCode::Ambiguous, self.keys(creases)),
                _ => {
                    // A side with no face can also be a sliver inside a blank: the edge counts when
                    // its middle lies in a face of the Cut lines that is neither material nor a hole
                    // (a notch), or in no face and outside every blank.
                    let segs: Vec<PathSeg> = e.path.to_bezpath().segments().collect();
                    let Some(mid) = segs.get(segs.len() / 2).map(|s| s.eval(0.5)) else { continue };
                    let m = self.material;
                    let away = match m.face_at(mid) {
                        Some(f) => m.material.get(f) != Some(&true) && m.hole.get(f) != Some(&true),
                        None => !m.in_a_blank(mid),
                    };
                    if !away {
                        continue;
                    }
                    let length: f64 = segs.iter().map(|s| s.arclen(1e-3)).sum();
                    for &line in &creases {
                        if let Some(o) = outside.get_mut(line) {
                            *o += length;
                        }
                    }
                }
            }
        }
        (pieces, outside)
    }

    /// The hinges `pieces` make, added to `blanks` → the lines with a hinge.
    fn hinges(&self, blanks: &mut [Blank], mut pieces: Vec<PlacedPiece>, issues: &mut Vec<Issue>) -> Vec<usize> {
        pieces.sort_by_key(|p| p.0);
        let mut hinged = vec![];
        let mut start = 0;
        while start < pieces.len() {
            let pair = pieces.get(start).map(|p| p.0);
            let end = pieces.iter().skip(start).position(|p| Some(p.0) != pair).map_or(pieces.len(), |n| start + n);
            let group: Vec<&HingePiece> = pieces.get(start..end).unwrap_or_default().iter().map(|p| &p.1).collect();
            start = end;
            let Some((blank, a, b)) = pair else { break };
            let lines: Vec<usize> = group.iter().flat_map(|p| p.lines.iter().copied()).collect();
            match self.hinge(&group, a, b) {
                Some(h) => {
                    if !h.straight {
                        issues.push(Issue::new(IssueCode::NotStraight, h.keys.clone(), None));
                    }
                    hinged.extend(lines);
                    if let Some(bl) = blanks.get_mut(blank) {
                        bl.hinges.push(h);
                    }
                }
                None => issues.push(Issue::new(IssueCode::NoSplit, self.keys(lines.iter().copied()), None)),
            }
        }
        hinged.sort_unstable();
        hinged.dedup();
        hinged
    }

    /// The hinge between panels `a` and `b` made of `group`, along its longest piece; `None` when
    /// its pieces are shorter than the tolerance in all, or span less than it.
    fn hinge(&self, group: &[&HingePiece], a: usize, b: usize) -> Option<Hinge> {
        let chord = |p: &HingePiece| {
            let sp = p.path.subpaths.first()?;
            Some((sp.anchors.first()?.p, sp.anchors.last()?.p))
        };
        let ((p0, p1), _) =
            group.iter().filter_map(|p| Some((chord(p)?, p))).max_by(|(x, _), (y, _)| x.0.distance(x.1).total_cmp(&y.0.distance(y.1)))?;
        let lengths: Vec<f64> = group.iter().map(|p| p.path.to_bezpath().segments().map(|s| s.arclen(1e-3)).sum::<f64>()).collect();
        // The angle of the line with the most length in the hinge, the one with the lowest key on a
        // tie: a line the planar map split into several pieces counts whole.
        let mut by_line: HashMap<usize, f64> = HashMap::new();
        for (p, len) in group.iter().zip(&lengths) {
            // Subpaths of one line on top of each other count once.
            let mut lines = p.lines.clone();
            lines.sort_unstable();
            lines.dedup();
            for l in lines {
                *by_line.entry(l).or_default() += len;
            }
        }
        let folding: Vec<(f64, f64, u64)> = by_line
            .iter()
            .filter_map(|(&l, &len)| {
                let line = self.lines.get(l)?;
                Some((line.fold_angle()?, len, line.key))
            })
            .collect();
        // Lengths summed from pieces in another order differ in their last bits: a few epsilons per
        // piece, and no more, so a line measurably longer still wins.
        let most = folding.iter().map(|f| f.1).fold(0.0, f64::max);
        let pieces = f64::from(u32::try_from(group.len()).unwrap_or(u32::MAX));
        let rounding = 8.0 * f64::EPSILON * (pieces + 1.0) * most;
        let angle = folding.iter().filter(|f| f.1 >= most - rounding).min_by_key(|f| f.2).map_or(90.0, |f| f.0);
        let len = p0.distance(p1);
        // The longest piece only sets the direction; the length check is on the whole hinge below.
        if len.is_nan() || len <= 1e-9 {
            return None;
        }
        let d = (p1 - p0) / len;
        // Each segment turned so the line through `p0` along `d` is the x axis: the exact bounds
        // give how far it strays from the line (y) and its extent along it (x).
        let turn = Affine::rotate(-d.y.atan2(d.x)) * Affine::translate(-p0.to_vec2());
        let mut straight = true;
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in group {
            for seg in p.path.to_bezpath().segments() {
                let b = (turn * seg).bounding_box();
                straight &= b.y0.abs() <= self.tolerance && b.y1.abs() <= self.tolerance;
                lo = lo.min(b.x0);
                hi = hi.max(b.x1);
            }
        }
        // The pieces' own length, not the span from the first to the last, which takes in the gaps
        // between pieces that lie apart.
        let length: f64 = lengths.iter().sum();
        if !(lo.is_finite() && hi.is_finite()) || hi - lo < self.tolerance || length.is_nan() || length < self.tolerance {
            return None;
        }
        let mut lines: Vec<usize> = group.iter().flat_map(|p| p.lines.iter().copied()).collect();
        lines.sort_unstable();
        lines.dedup();
        let mut keys = self.keys(lines);
        keys.sort_unstable();
        keys.dedup();
        Some(Hinge { a, b, from: p0 + d * lo, to: p0 + d * hi, on: p0.midpoint(p1), keys, straight, angle })
    }

    /// Creases with no hinge: outside every blank, or dividing no two panels; and creases beside a
    /// panel that run `outside` every blank (by line) for the tolerance or more.
    fn creases(&self, places: &[Place], hinged: &[usize], outside: &[f64], issues: &mut Vec<Issue>) {
        // A line drawn twice is still checked for where it lies.
        let named: HashSet<u64> = issues
            .iter()
            .filter(|i| !matches!(i.code, IssueCode::LooseEnd | IssueCode::Gap | IssueCode::Duplicate))
            .flat_map(|i| i.keys.iter().copied())
            .collect();
        // Whether each line is drawn (has a usable subpath), and whether one of its edges touches a
        // panel.
        let mut drawn = vec![false; self.lines.len()];
        for p in self.pieces {
            if let Some(d) = drawn.get_mut(p.line) {
                *d = true;
            }
        }
        let mut on_panel = vec![false; self.lines.len()];
        for e in &self.full.edges {
            if [e.left, e.right].into_iter().any(|f| matches!(self.place(places, f), Place::Panel { .. })) {
                for line in e.sources.iter().filter_map(|&s| self.line_of(s)) {
                    if let Some(on) = on_panel.get_mut(line) {
                        *on = true;
                    }
                }
            }
        }
        for (line, l) in self.lines.iter().enumerate() {
            if l.kind == LineKind::Cut || drawn.get(line) != Some(&true) {
                continue;
            }
            let beside = on_panel.get(line) == Some(&true);
            if hinged.binary_search(&line).is_err() && !named.contains(&l.key) {
                issues.push(Issue::new(if beside { IssueCode::NoSplit } else { IssueCode::Outside }, vec![l.key], None));
            }
            if beside && outside.get(line).is_some_and(|&o| o >= self.tolerance) {
                issues.push(Issue::new(IssueCode::Outside, vec![l.key], None));
            }
        }
    }

    /// Slivers of the full map inside a blank.
    fn slivers(&self, issues: &mut Vec<Issue>) {
        for s in &self.full.slivers {
            let Some(p) = point_in(s) else { continue };
            if self.material.in_a_blank(p) {
                issues.push(Issue::new(IssueCode::Sliver, vec![], Some(p)));
            }
        }
    }

    /// Blanks that lie inside a hole of another blank.
    fn nested(&self, issues: &mut Vec<Issue>) {
        let m = self.material;
        for (bi, &piece) in m.blanks.iter().enumerate() {
            let faces = m.faces_by_piece.get(piece).map(Vec::as_slice).unwrap_or_default();
            let Some(p) = faces.iter().find(|&&f| m.material.get(f) == Some(&true)).and_then(|&f| m.points.get(f).copied().flatten()) else {
                continue;
            };
            let in_hole =
                m.face_cells.near(p, 0.0).into_iter().any(|f| {
                    m.hole.get(f) == Some(&true) && m.blank_of(f).is_some_and(|b| b != bi) && m.outer.get(f).is_some_and(|bp| contains(bp, p))
                });
            if in_hole {
                let edges = m.edges_by_piece.get(piece).map(Vec::as_slice).unwrap_or_default();
                let lines = edges.iter().filter_map(|&e| m.map.edges.get(e)).flat_map(|e| e.sources.iter().filter_map(|&s| self.cut_lines.get(s)));
                issues.push(Issue::new(IssueCode::Ambiguous, self.keys(lines.copied()), Some(p)));
            }
        }
    }

    /// Panels joined to their blank's base panel (the largest; on a tie, the one whose inside point is
    /// highest on the page, then leftmost) by no chain of hinges.
    fn loose_panels(&self, blanks: &[Blank], places: &[Place], issues: &mut Vec<Issue>) {
        // The edges beside each panel, by (blank, panel), gathered once.
        let mut beside: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        for (i, e) in self.full.edges.iter().enumerate() {
            for f in [e.left, e.right] {
                if let Place::Panel { blank, panel } = self.place(places, f) {
                    let list = beside.entry((blank, panel)).or_default();
                    if list.last() != Some(&i) {
                        list.push(i);
                    }
                }
            }
        }
        for (bi, b) in blanks.iter().enumerate() {
            let Some(base) = base_panel(&b.panels) else { continue };
            let mut reached = vec![false; b.panels.len()];
            let mut queue = vec![base];
            if let Some(r) = reached.get_mut(base) {
                *r = true;
            }
            while let Some(p) = queue.pop() {
                for h in b.hinges.iter().filter(|h| h.a == p || h.b == p) {
                    let other = if h.a == p { h.b } else { h.a };
                    if let Some(r) = reached.get_mut(other).filter(|r| !**r) {
                        *r = true;
                        queue.push(other);
                    }
                }
            }
            for (pi, panel) in b.panels.iter().enumerate().filter(|(i, _)| reached.get(*i) == Some(&false)) {
                let edges = beside.get(&(bi, pi)).map(Vec::as_slice).unwrap_or_default();
                let lines = edges.iter().filter_map(|&e| self.full.edges.get(e));
                let keys = lines.flat_map(|e| e.sources.iter().filter_map(|&s| self.line_of(s))).map(|l| self.key(l)).collect();
                issues.push(Issue::new(IssueCode::LoosePanel, keys, point_in(&panel.outline)));
            }
        }
    }
}

/// The base panel of `panels`: the largest by area; panels whose areas differ by less than 1e-9 of
/// the larger tie, and the tie goes to the one whose inside point is highest on the page, then
/// leftmost.
pub fn base_panel(panels: &[Panel]) -> Option<usize> {
    let top = panels.iter().map(|p| p.area).fold(f64::NEG_INFINITY, f64::max);
    if !top.is_finite() {
        return None;
    }
    panels
        .iter()
        .enumerate()
        .filter(|(_, p)| top - p.area <= 1e-9 * top.abs())
        .filter_map(|(i, p)| Some((i, point_in(&p.outline)?)))
        .min_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x)))
        .map(|(i, _)| i)
}

mod pose;
pub mod raster;
pub use pose::{Folded, FoldedHinge, FoldedPanel, MAX_POLYGON_POINTS, Pose, Stack, fold, panel_at, pick, polygon};

#[cfg(test)]
mod tests;
