//! The Dimension tool. Hover a segment (linear), a corner (angular) or a circle or perfect arc
//! (radial) and drag outwards: releasing places the linked dimension. Plain clicks pick what to
//! measure instead: two points (linear) or two straight lines (angular), then a click places it;
//! a click on a circle or arc gives its radius. Escape cancels without leaving any artwork.
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};
use serde_json::{Value, json};
use std::collections::HashMap;
use vectorcraft_doc::{
    Node, NodeId, NodeKind,
    dimension::{self, Dimension, DimensionMode, DimensionOptions, DimensionSource, FreePoint, Orientation, PointRef, SegmentRef},
};
use vectorcraft_geom::{BezPath, Point, Rect, Vec2};

/// The three kinds of dimension the tool draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Linear,
    Angular,
    Radial,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Linear => "linear",
            Kind::Angular => "angular",
            Kind::Radial => "radial",
        }
    }
    /// `radius` and `diameter` are the radial kind.
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "linear" => Some(Kind::Linear),
            "angular" => Some(Kind::Angular),
            "radial" | "radius" | "diameter" => Some(Kind::Radial),
            _ => None,
        }
    }
}

/// What the pointer is over.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Segment(SegmentRef),
    Corner { first: SegmentRef, second: SegmentRef, at: Point },
    Radial { source: DimensionSource, center: Point, radius: f64 },
}

#[derive(Default)]
pub struct DimensionTool {
    kind: Kind,
    style: DimensionOptions,
    /// Where the button went down, and what was under it.
    press: Option<(Point, Option<Target>)>,
    /// Plain-click picks: points (linear) or straight lines (angular).
    points: Vec<(Point, Option<PointRef>)>,
    lines: Vec<SegmentRef>,
    /// A dimension being placed: previews follow the pointer.
    pending: Option<Dimension>,
    dragging: bool,
    hover: Option<Target>,
    at: Point,
    /// Whether Shift was down at the last pointer event: the picked line shows constrained.
    shift: bool,
    began: bool,
    /// Smart Guides for the Linear picks, hovering too: the guides show before each click.
    snap: crate::guides::DrawSnap,
    /// Where the next Linear pick lands, as last worked out for the pointer (for the overlays).
    snapped: Option<(Point, Option<PointRef>)>,
    /// Whether the last Linear pick worked out landed on the end of a line of a dimension already
    /// drawn, or on a corner of a path's preview bounds (shown as an anchor).
    on_end: bool,
    /// The path that holds the last Linear pick worked out, when it landed on a corner of that
    /// path's preview bounds.
    corner: Option<NodeId>,
    /// The path that holds the first Linear point, when it was picked on a corner of that path's
    /// preview bounds.
    first_corner: Option<NodeId>,
    /// The link the first Linear point took at its click when it was not on an anchor: held
    /// against the path whose preview-bounds corner it was picked on, or on the segment under it.
    /// When the art moves before the second click, the first point moves with it.
    first_link: Option<FreePoint>,
    /// Where the first Linear point was picked. The point goes back there when its anchor or
    /// [`Self::first_link`] no longer resolves.
    first_pick: Option<Point>,
    /// Whether the press under way began on an anchor a Linear click picks.
    press_on_anchor: bool,
    /// The second point clicked for the Linear dimension being placed, and where the pointer was
    /// for that click (a snap or Shift can move the point away from it): placed with the pointer
    /// still at either, it is aligned.
    second: Option<(Point, Point)>,
    /// The preview bounds the Linear picks measured, for the document's revision.
    preview_boxes: PreviewBoxes,
    /// While a Linear dimension is placed with its line on the level of another dimension's line:
    /// the ends of that other line. A Smart Guide runs along the level across both lines.
    level_guide: Option<(Point, Point)>,
    /// While a linear dimension is placed: the lines of the visible, unlocked linear dimensions in
    /// the document ([`dimension_lines`]), which its line can line up with while Smart Guides are
    /// on. With Smart Guides on, they are read again when the document they come from has changed.
    levels: Vec<DimensionLine>,
    /// Which document `levels` were read from: the open document's id and revision, or, while an
    /// interaction is in progress, the open document's id and the address of the document the
    /// interaction began with.
    levels_at: (u64, u64, usize),
    /// The document's next object id at the dimension's first preview: the id its preview takes
    /// (`dimension.create` gives the dimension the next id). The preview is not a level.
    preview_id: u64,
}

/// What a linear dimension between `a` and `b` measures with the pointer at `p` while it is
/// placed (VectorCraft's own rule). Beside the points, where the nearest point of the line
/// through them lies between them, the dimension is aligned: it measures the distance between
/// them. Past either point it measures along an axis. The line through the points and the
/// perpendicular at that point bound a quarter on each side; the pointer's quarter holds either
/// the straight up or down direction, which gives the horizontal distance, or the straight left
/// or right direction, which gives the vertical distance. Points on one horizontal or vertical
/// line only give an aligned dimension.
fn orientation_at(a: Point, b: Point, p: Point) -> Orientation {
    let u = b - a;
    let len = u.hypot();
    if len < 1e-9 {
        return Orientation::Aligned;
    }
    let along = (p - a).dot(u) / (len * len);
    if (0.0..=1.0).contains(&along) {
        return Orientation::Aligned;
    }
    // Out past the nearer point, and the side of the line the pointer is on.
    let (out, side) = (if along > 1.0 { u } else { -u }, u.cross(p - a));
    let axes = [
        (Vec2::new(0.0, 1.0), Orientation::Horizontal),
        (Vec2::new(0.0, -1.0), Orientation::Horizontal),
        (Vec2::new(1.0, 0.0), Orientation::Vertical),
        (Vec2::new(-1.0, 0.0), Orientation::Vertical),
    ];
    let within = len * 1e-9;
    axes.into_iter()
        .find(|(axis, _)| axis.dot(out) > within && u.cross(*axis).abs() > within && u.cross(*axis) * side > 0.0)
        .map_or(Orientation::Aligned, |(_, orientation)| orientation)
}

/// How near an anchor (screen pixels) a pick snaps to it.
const ANCHOR_REACH: f64 = 8.0;

/// `at` as a Shift click picks it: the direction from `last` snapped to 45° steps (from the
/// constrain angle), the distance kept.
fn constrained(cx: &ToolContext, last: Point, at: Point) -> Point {
    last + vectorcraft_geom::constrain_angle_from(at - last, 45.0, cx.constrain_angle)
}

/// Nearest editable segment, with bounded work and no snapping to dimension artwork itself.
pub fn pick(cx: &ToolContext, p: Point) -> Option<SegmentRef> {
    let mut best = None;
    let mut dist = cx.tol(8.0);
    let mut budget = 20_000usize;
    fn visit(n: &Node, p: Point, budget: &mut usize, dist: &mut f64, best: &mut Option<SegmentRef>) {
        if *budget == 0 || !n.visible || n.locked || n.extras().dimension.is_some() {
            return;
        }
        if let Some(path) = n.path_data() {
            for (si, sp) in path.subpaths.iter().enumerate() {
                for i in 0..sp.segment_count() {
                    if *budget == 0 {
                        return;
                    }
                    *budget -= 1;
                    use vectorcraft_geom::kurbo::ParamCurveNearest;
                    let near = sp.segment(i).nearest(p, 1e-4);
                    if near.distance_sq < *dist * *dist {
                        *dist = near.distance_sq.sqrt();
                        *best = Some(SegmentRef { node: n.id, subpath: si, segment: i });
                    }
                }
            }
        }
        if let Some(ch) = n.children() {
            for c in ch.iter().rev() {
                visit(c, p, budget, dist, best);
            }
        }
    }
    for n in cx.doc.layers.iter().rev() {
        visit(n, p, &mut budget, &mut dist, &mut best);
    }
    best
}

/// The anchor of `edge` within reach of `p`, if any.
fn anchor_near(cx: &ToolContext, edge: &SegmentRef, p: Point) -> Option<(usize, Point)> {
    let sp = cx.doc.node(edge.node)?.path_data()?.subpaths.get(edge.subpath)?;
    let next = if edge.segment + 1 < sp.anchors.len() { edge.segment + 1 } else { 0 };
    [edge.segment, next]
        .into_iter()
        .filter_map(|i| Some((i, sp.anchors.get(i)?.p)))
        .filter(|(_, a)| a.distance(p) <= cx.tol(ANCHOR_REACH))
        .min_by(|(_, a), (_, b)| a.distance(p).total_cmp(&b.distance(p)))
}

/// The anchor nearest `p` within reach, on any editable path (the topmost on a tie), as a link to
/// its path. Bounded work, and dimension artwork is skipped, as in [`pick`].
fn anchor_under(cx: &ToolContext, p: Point) -> Option<(Point, PointRef)> {
    anchor_under_where(cx, p, |_| true)
}

/// [`anchor_under`] outside the objects `keep` turns down, and outside everything in them.
fn anchor_under_where(cx: &ToolContext, p: Point, keep: impl Fn(NodeId) -> bool) -> Option<(Point, PointRef)> {
    type Best = Option<(f64, Point, PointRef)>;
    fn visit(n: &Node, p: Point, reach: f64, keep: &dyn Fn(NodeId) -> bool, budget: &mut usize, best: &mut Best) {
        if *budget == 0 || !n.visible || n.locked || n.extras().dimension.is_some() || !keep(n.id) {
            return;
        }
        if let Some(path) = n.path_data() {
            for (subpath, sp) in path.subpaths.iter().enumerate() {
                for (anchor, a) in sp.anchors.iter().enumerate() {
                    if *budget == 0 {
                        return;
                    }
                    *budget -= 1;
                    let d = a.p.distance(p);
                    if d <= reach && best.as_ref().is_none_or(|(nearest, ..)| d < *nearest) {
                        *best = Some((d, a.p, PointRef { node: n.id, subpath, anchor }));
                    }
                }
            }
        }
        if let Some(ch) = n.children() {
            for c in ch.iter().rev() {
                visit(c, p, reach, keep, budget, best);
            }
        }
    }
    let (reach, mut budget, mut best) = (cx.tol(ANCHOR_REACH), 20_000usize, None);
    for n in cx.doc.layers.iter().rev() {
        visit(n, p, reach, &keep, &mut budget, &mut best);
    }
    best.map(|(_, at, r)| (at, r))
}

/// The geometric bounds of the paths [`preview_corner`] looked at, and the preview bounds of
/// those it measured, kept while the document is unchanged, so that the pointer moving over the
/// art measures each path once.
#[derive(Default)]
struct PreviewBoxes {
    revision: Option<(u64, u64)>,
    geometric: HashMap<NodeId, Option<Rect>>,
    preview: HashMap<NodeId, Option<Rect>>,
    /// The editable path holding each corner of a measured object's preview bounds, found when
    /// the object is measured, so that a pointer move reads no member.
    holders: HashMap<NodeId, [Option<NodeId>; 4]>,
    /// Whether a point at each corner (by object and corner number) can be held against its path.
    holdable: HashMap<(NodeId, usize), bool>,
}

/// How many member paths one pointer move may read to measure objects not yet measured, apart
/// from the objects it visits: an object with more members than are left gives no corner on
/// that move.
const PREVIEW_MEMBER_WORK: usize = 20_000;

/// A corner of an object's preview bounds that a Linear pick can land on.
#[derive(Clone, Copy, Debug)]
struct PreviewCorner {
    at: Point,
    /// The path the point is held against.
    holder: NodeId,
    /// The same corner of the object's geometric bounds: an anchor of `holder` there is the
    /// anchor the corner stands in for.
    stands_for: Point,
}

/// With Use Preview Bounds on, the corner of a path's or compound path's preview bounds (its
/// strokes and effects included) nearest `p` within reach, on any editable one whose preview
/// bounds differ from its geometric bounds. In a compound path, the member whose own box reaches
/// nearest to the corner holds it. The work is bounded, and dimension artwork is skipped, as in
/// [`pick`]: the objects visited count against one budget, and the member paths read to measure
/// them against [`PREVIEW_MEMBER_WORK`]. `cache` keeps the bounds and holders measured for the
/// document's revision.
fn preview_corner(cx: &ToolContext, p: Point, cache: &mut PreviewBoxes) -> Option<PreviewCorner> {
    struct Walk<'a, 'c> {
        symbols: Option<vectorcraft_doc::SymbolBoxes<'a>>,
        doc: &'a vectorcraft_doc::Document,
        cache: &'c mut PreviewBoxes,
        p: Point,
        reach: f64,
        budget: usize,
        /// Member paths left to read for objects not yet measured.
        work: usize,
        best: Option<(f64, PreviewCorner)>,
    }
    /// Takes `count` from the member work left, or nothing when less is left.
    fn charge(work: &mut usize, count: usize) -> bool {
        let fits = count <= *work;
        if fits {
            *work -= count;
        }
        fits
    }
    fn corners(r: Rect) -> [Point; 4] {
        [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
    }
    /// For each corner of `preview`, the path whose own box reaches nearest to it: in a compound
    /// path, the member that puts that corner where it is. A hidden or locked member, or one made
    /// into a guide, gives no corner. One pass reads each member once for all four corners.
    fn holders<'n>(members: impl Iterator<Item = &'n Node>, preview: Rect) -> [Option<NodeId>; 4] {
        let mut best: [Option<(f64, NodeId, bool)>; 4] = [None; 4];
        for m in members {
            let Some(b) = m.geometric_bounds() else { continue };
            let editable = m.visible && !m.locked && !m.is_guide();
            for ((holder, c), q) in best.iter_mut().zip(corners(b)).zip(corners(preview)) {
                let gap = c.distance(q);
                if holder.is_none_or(|(near, _, was)| gap < near || (gap == near && editable && !was)) {
                    *holder = Some((gap, m.id, editable));
                }
            }
        }
        best.map(|h| h.filter(|(.., editable)| *editable).map(|(_, id, _)| id))
    }
    fn visit(n: &Node, w: &mut Walk) {
        if w.budget == 0 || !n.visible || n.locked || n.extras().dimension.is_some() {
            return;
        }
        w.budget -= 1;
        // Only effects, on the object or on one of its fills or strokes, paint further out than
        // its strokes reach.
        let effects = !n.appearance.effects.is_empty() || n.appearance.items.iter().any(|i| !i.effects().is_empty());
        let reach = n.appearance.outset() + w.reach;
        // A path is its own member; a compound path's members are its paths.
        let compound = match &n.kind {
            NodeKind::Compound { children, .. } => Some(children.as_slice()),
            _ => None,
        };
        let members =
            || compound.into_iter().flatten().map(|c| c.as_ref()).chain(compound.is_none().then_some(n)).filter(|m| m.path_data().is_some());
        // Measuring the object, its bounds or its holders, reads every member once.
        let count = compound.map_or(1, <[_]>::len);
        // A path made into a guide draws as a guide, without its strokes or effects. The pointer
        // test includes the right and bottom edges, as the distance test below does.
        let within = |r: Rect| (r.x0..=r.x1).contains(&w.p.x) && (r.y0..=r.y1).contains(&w.p.y);
        // Set once this move has paid for reading the members: measuring the preview bounds and
        // holders on the same move reads them again without a second charge.
        let mut paid = false;
        let geometric = match w.cache.geometric.get(&n.id) {
            _ if n.is_guide() || members().next().is_none() => None,
            Some(g) => *g,
            None if charge(&mut w.work, count) => {
                paid = true;
                let g = n.geometric_bounds();
                w.cache.geometric.insert(n.id, g);
                g
            }
            None => None,
        };
        if let Some(geometric) = geometric
            && (effects || within(geometric.inflate(reach, reach)))
        {
            let doc = w.doc;
            let measured = match w.cache.preview.get(&n.id) {
                Some(r) => Some(*r),
                None if paid || charge(&mut w.work, count) => {
                    let symbols = w.symbols.get_or_insert_with(|| doc.symbol_boxes());
                    let r = vectorcraft_effects::preview::visual_bounds(symbols, n);
                    w.cache.preview.insert(n.id, r);
                    if let Some(r) = r
                        && r != geometric
                    {
                        w.cache.holders.insert(n.id, holders(members(), r));
                    }
                    Some(r)
                }
                None => None,
            };
            if let Some(Some(r)) = measured
                && r != geometric
            {
                for (k, q) in corners(r).into_iter().enumerate() {
                    let d = q.distance(w.p);
                    if d > w.reach || w.best.is_some_and(|(nearest, ..)| d >= nearest) {
                        continue;
                    }
                    let holder = w.cache.holders.get(&n.id).and_then(|h| h.get(k).copied().flatten());
                    // A path that cannot hold a point at the corner (a stroked dot, whose anchors
                    // all coincide) gives no corner.
                    let holdable = |id: NodeId, cache: &mut PreviewBoxes| {
                        *cache.holdable.entry((n.id, k)).or_insert_with(|| FreePoint::on(doc, id, q).is_some())
                    };
                    if let Some((holder, stands_for)) = holder.filter(|id| holdable(*id, w.cache)).zip(corners(geometric).get(k).copied()) {
                        w.best = Some((d, PreviewCorner { at: q, holder, stands_for }));
                    }
                }
            }
        }
        // A compound path draws its members with its own appearance: they give no corners of
        // their own.
        if compound.is_none()
            && let Some(ch) = n.children()
        {
            for c in ch.iter().rev() {
                visit(c, w);
            }
        }
    }
    // Anchors take a pick whatever the snapping options, and so do the corners standing in for them.
    if !cx.preview_bounds {
        return None;
    }
    if cache.revision != Some(cx.revision) {
        *cache = PreviewBoxes { revision: Some(cx.revision), ..Default::default() };
    }
    let mut w = Walk { symbols: None, doc: cx.doc, cache, p, reach: cx.tol(ANCHOR_REACH), budget: 20_000, work: PREVIEW_MEMBER_WORK, best: None };
    for n in cx.doc.layers.iter().rev() {
        visit(n, &mut w);
    }
    w.best.map(|(_, corner)| corner)
}

/// The anchor a pick at `p` lands on instead of `corner`, given `anchor`, the nearest anchor
/// within reach; `None` when the corner takes it (VectorCraft's own rule). The corner takes the
/// pick from the anchors of the path holding it at the same corner of the object's geometric
/// bounds, which sit on the stroke's centerline. An anchor right on the corner takes it, and so
/// does the nearest other anchor, of this object or of other art, when it is nearer the pointer
/// than the corner. The holder's anchors and the other art are searched with a budget each, and
/// `anchor` wins a tie, as the topmost.
fn anchor_over_corner(cx: &ToolContext, p: Point, corner: &PreviewCorner, anchor: Option<(Point, PointRef)>) -> Option<(Point, PointRef)> {
    if anchor.as_ref().is_some_and(|(at, _)| at.distance(corner.at) <= 1e-9) {
        return anchor;
    }
    let stands_in = |(at, nearest): &(Point, PointRef)| nearest.node == corner.holder && at.distance(corner.stands_for) <= 1e-9;
    let first = anchor.filter(|a| !stands_in(a));
    let reach = cx.tol(ANCHOR_REACH);
    let own = cx.doc.node(corner.holder).and_then(|n| n.path_data()).and_then(|path| {
        let anchors = path.subpaths.iter().enumerate().flat_map(|(subpath, sp)| sp.anchors.iter().enumerate().map(move |(i, a)| (subpath, i, a.p)));
        anchors
            .take(20_000)
            .filter(|(.., q)| q.distance(p) <= reach && q.distance(corner.stands_for) > 1e-9)
            .min_by(|(.., a), (.., b)| a.distance(p).total_cmp(&b.distance(p)))
            .map(|(subpath, anchor, q)| (q, PointRef { node: corner.holder, subpath, anchor }))
    });
    let other = anchor_under_where(cx, p, |id| id != corner.holder);
    first
        .into_iter()
        .chain(other)
        .chain(own)
        .filter(|(q, _)| q.distance(p) < corner.at.distance(p) || q.distance(corner.at) <= 1e-9)
        .min_by(|(a, _), (b, _)| a.distance(p).total_cmp(&b.distance(p)))
}

/// Where a pick at `p` lands on the artwork of the visible, unlocked dimensions already drawn,
/// with no link to it: the nearest end of one of their lines (dimension and extension lines)
/// within the anchor reach, as an anchor (`true`); else the nearest point of one of those lines
/// within 8 pixels (`false`). The end of an extension line next to the point it measures is not
/// an end here: it lies a few points off that point. Only with Smart Guides or Snap to Point on.
/// Bounded work, and the topmost dimension wins a tie, as in [`anchor_under`].
fn on_dimensions(cx: &ToolContext, p: Point) -> Option<(Point, bool)> {
    use vectorcraft_geom::kurbo::{ParamCurve, ParamCurveNearest};
    #[derive(Default)]
    struct Best {
        end: Option<(f64, Point)>,
        line: Option<(f64, Point)>,
    }
    /// `measured`: a linear dimension's measured points and how far its extension lines start
    /// from them.
    fn lines(n: &Node, measured: Option<([Point; 2], f64)>, p: Point, reach: [f64; 2], budget: &mut usize, best: &mut Best) {
        if *budget == 0 || !n.visible || n.locked {
            return;
        }
        if let Some(path) = n.path_data() {
            // Open subpaths only: the lines, not a filled arrowhead.
            for sp in path.subpaths.iter().filter(|sp| !sp.closed) {
                // An extension line starts at the offset from its measured point and runs straight
                // away from it; the dimension line runs across that direction. With no offset its
                // start is the measured point itself, and stays an end.
                let extension_start = |a: Point, b: Point| {
                    measured.is_some_and(|(points, offset)| {
                        points.iter().any(|m| {
                            let (out, along) = (a - *m, b - a);
                            offset > 1e-6 && (out.hypot() - offset).abs() <= 1e-6 && out.dot(along) >= (1.0 - 1e-6) * out.hypot() * along.hypot()
                        })
                    })
                };
                let (Some(first), Some(last)) = (sp.anchors.first(), sp.anchors.last()) else { continue };
                let skip_first = extension_start(first.p, last.p);
                for a in [first, last].into_iter().skip(usize::from(skip_first)) {
                    let d = a.p.distance(p);
                    if d <= reach[0] && best.end.is_none_or(|(near, _)| d < near) {
                        best.end = Some((d, a.p));
                    }
                }
                for i in 0..sp.segment_count() {
                    if *budget == 0 {
                        return;
                    }
                    *budget -= 1;
                    let c = sp.segment(i);
                    let q = c.eval(c.nearest(p, 1e-4).t);
                    let d = q.distance(p);
                    if q.x.is_finite() && q.y.is_finite() && d <= reach[1] && best.line.is_none_or(|(near, _)| d < near) {
                        best.line = Some((d, q));
                    }
                }
            }
        }
        for c in n.children().into_iter().flatten().rev() {
            if *budget == 0 {
                return;
            }
            lines(c, measured, p, reach, budget, best);
        }
    }
    fn visit(n: &Node, p: Point, reach: [f64; 2], budget: &mut usize, best: &mut Best) {
        if *budget == 0 || !n.visible || n.locked {
            return;
        }
        *budget -= 1;
        if let Some(d) = &n.extras().dimension {
            let measured = (d.mode == DimensionMode::Linear).then_some(([d.a, d.b], d.options.extension_offset));
            lines(n, measured, p, reach, budget, best);
            return;
        }
        for c in n.children().into_iter().flatten().rev() {
            if *budget == 0 {
                return;
            }
            visit(c, p, reach, budget, best);
        }
    }
    if !(cx.smart_guides || cx.snap_to_point) {
        return None;
    }
    let (reach, mut budget, mut best) = ([cx.tol(ANCHOR_REACH), cx.tol(8.0)], 20_000usize, Best::default());
    for n in cx.doc.layers.iter().rev() {
        if budget == 0 {
            break;
        }
        visit(n, p, reach, &mut budget, &mut best);
    }
    best.end.map(|(_, q)| (q, true)).or_else(|| best.line.map(|(_, q)| (q, false)))
}

/// The line of a linear dimension already drawn: its ends and its direction.
type DimensionLine = ((Point, Point), Vec2);

/// The lines of the visible, unlocked linear dimensions in `doc`, except the preview of the
/// dimension being placed: a dimension with the id `preview.0` that measures the points
/// `preview.1`. Bounded work, as in [`on_dimensions`].
fn dimension_lines(doc: &vectorcraft_doc::Document, preview: (u64, [Point; 2])) -> Vec<DimensionLine> {
    fn visit(n: &Node, preview: (u64, [Point; 2]), budget: &mut usize, out: &mut Vec<DimensionLine>) {
        let is_preview = n.id.0 == preview.0
            && n.extras().dimension.as_ref().is_some_and(|d| d.a.distance(preview.1[0]) < 1e-9 && d.b.distance(preview.1[1]) < 1e-9);
        if *budget == 0 || !n.visible || n.locked || is_preview {
            return;
        }
        *budget -= 1;
        if let Some(d) = &n.extras().dimension {
            if d.mode == DimensionMode::Linear {
                out.push((d.line_ends(), d.direction()));
            }
            return;
        }
        for c in n.children().into_iter().flatten().rev() {
            if *budget == 0 {
                return;
            }
            visit(c, preview, budget, out);
        }
    }
    let (mut budget, mut out) = (20_000usize, vec![]);
    for n in doc.layers.iter().rev() {
        if budget == 0 {
            break;
        }
        visit(n, preview, &mut budget, &mut out);
    }
    out
}

/// Where the line of the linear dimension `d`, placed through `p`, lands on the level of one of
/// the `levels` that runs parallel to it, within `reach` of `p` across the line: the point to
/// place `d` through, and the ends of that other line. The nearest level wins, then the line
/// nearest `p` along it (none nearer than a line that `p` lies across).
fn line_level(levels: &[DimensionLine], d: &Dimension, p: Point, reach: f64) -> Option<(Point, (Point, Point))> {
    if d.mode != DimensionMode::Linear {
        return None;
    }
    let (u, normal) = (d.direction(), d.normal());
    let mut best: Option<(f64, f64, f64, (Point, Point))> = None;
    for &(ends, direction) in levels {
        if u.cross(direction).abs() > 1e-6 {
            continue;
        }
        let shift = (ends.0 - p).dot(normal);
        let (s0, s1) = ((ends.0 - p).dot(u), (ends.1 - p).dot(u));
        let along = if s0 * s1 <= 0.0 { 0.0 } else { s0.abs().min(s1.abs()) };
        if shift.is_finite() && along.is_finite() && shift.abs() <= reach && best.is_none_or(|(s, a, _, _)| (shift.abs(), along) < (s, a)) {
            best = Some((shift.abs(), along, shift, ends));
        }
    }
    best.map(|(_, _, shift, ends)| (p + normal * shift, ends))
}

/// The point of `edge` closest to `p`.
fn nearest_on(cx: &ToolContext, edge: &SegmentRef, p: Point) -> Option<Point> {
    use vectorcraft_geom::kurbo::ParamCurveNearest;
    let sp = cx.doc.node(edge.node)?.path_data()?.subpaths.get(edge.subpath)?;
    if edge.segment >= sp.segment_count() {
        return None;
    }
    let c = sp.segment(edge.segment);
    let t = c.nearest(p, 1e-4).t;
    use vectorcraft_geom::kurbo::ParamCurve;
    let q = c.eval(t);
    (q.x.is_finite() && q.y.is_finite()).then_some(q)
}

/// What a dimension between two picked points links to. A point on an anchor links to the anchor.
/// A point on a segment keeps its place along the segment. Any other point is held
/// against the anchors of the path under it, or, with no path under it, the path of the other
/// point, so that it moves with that path. None when neither point has a path to link to.
/// `held_a` and `held_b` name the path holding a point picked on a corner of that path's
/// preview bounds: such a point is held against that path, however far its strokes or effects
/// put the corner from the path and whatever other segment lies there. `link_a` is the link the
/// first point took at its own click ([`own_link`]): while it still resolves, the point keeps it,
/// whatever other art lies there now.
fn points_source(
    cx: &ToolContext,
    (a, ra, held_a): (Point, &Option<PointRef>, Option<NodeId>),
    (b, rb, held_b): (Point, &Option<PointRef>, Option<NodeId>),
    link_a: Option<FreePoint>,
) -> Option<DimensionSource> {
    // A path deleted since the pick holds nothing.
    let holder = |held: Option<NodeId>| held.filter(|node| cx.doc.node(*node).is_some());
    let (corner_a, corner_b) = (holder(held_a), holder(held_b));
    let link_a = link_a.filter(|link| ra.is_none() && link.position(cx.doc).is_some());
    let edge = |p: Point, corner: Option<NodeId>| if corner.is_some() { None } else { pick(cx, p) };
    let (edge_a, edge_b) = (if link_a.is_some() { None } else { edge(a, corner_a) }, edge(b, corner_b));
    let node = |r: &Option<PointRef>, corner: Option<NodeId>, e: &Option<SegmentRef>| {
        r.as_ref().map(|r| r.node).or(corner).or_else(|| e.as_ref().map(|e| e.node))
    };
    let path_a = link_a.as_ref().map(FreePoint::node).or_else(|| node(ra, corner_a, &edge_a));
    let path_b = node(rb, corner_b, &edge_b);
    let free = |p: Point, r: &Option<PointRef>, edge: &Option<SegmentRef>, own: Option<NodeId>, other: Option<NodeId>| {
        if r.is_some() {
            return None;
        }
        edge.as_ref().and_then(|e| FreePoint::on_segment(cx.doc, e, p)).or_else(|| own.or(other).and_then(|node| FreePoint::on(cx.doc, node, p)))
    };
    let free_a = link_a.or_else(|| free(a, ra, &edge_a, path_a, path_b));
    let free_b = free(b, rb, &edge_b, path_b, path_a);
    (ra.is_some() || rb.is_some() || free_a.is_some() || free_b.is_some()).then(|| DimensionSource::Points {
        a: ra.clone(),
        b: rb.clone(),
        vertex: None,
        free_a,
        free_b,
    })
}

/// The link a point picked at `p` off any anchor takes from its own pick, as [`points_source`]
/// links it: held against `corner`'s path when it was picked on a corner of that path's preview
/// bounds, else on the segment under it, or held against that segment's path. None when nothing
/// under it holds it.
fn own_link(cx: &ToolContext, p: Point, corner: Option<NodeId>) -> Option<FreePoint> {
    match corner.filter(|node| cx.doc.node(*node).is_some()) {
        Some(node) => FreePoint::on(cx.doc, node, p),
        None => pick(cx, p).and_then(|e| FreePoint::on_segment(cx.doc, &e, p).or_else(|| FreePoint::on(cx.doc, e.node, p))),
    }
}

impl DimensionTool {
    /// Where a Linear pick at `p` lands: with Use Preview Bounds on, on the nearest corner of a
    /// path's preview bounds within reach, where they differ from its anchors, unless another
    /// anchor is nearer (held against the path, labeled "anchor"; [`anchor_over_corner`]); else on the
    /// nearest anchor of any path, within reach (kept as
    /// a link to its path), labeled "anchor"; else on the nearest end of a line of a dimension
    /// already drawn, within the same reach (no link), also labeled "anchor"; else where Smart Guides put it (another object's anchor or centre, in
    /// line with one, or for the second point square to a segment or grazing a curve from the
    /// first with Snap to Perpendicular or Snap to Tangent on, or on a construction guide through
    /// a first point picked on an anchor), its guides showing; else on the closest point of the
    /// line under the pointer, of the art or of a dimension, labeled "path"; else `p` itself. A
    /// first point not on an anchor has no construction guides through it.
    /// A point the guides leave where the pointer is (right on a guide line) stays there, and with
    /// Snap to Pixel or Snap to Grid on a point stays where they place it.
    fn snap_linear(&mut self, cx: &ToolContext, p: Point) -> (Point, Option<PointRef>) {
        self.follow_first(cx);
        let leave = self.points.last().map(|(first, anchor)| match anchor {
            Some(_) => crate::guides::Leave::segment(cx, *first, false),
            None => crate::guides::Leave::contact(*first),
        });
        let guided = self.snap.hover(cx, p, &[], leave.as_ref());
        self.on_end = false;
        self.corner = None;
        let mut anchor = anchor_under(cx, p);
        if let Some(corner) = preview_corner(cx, p, &mut self.preview_boxes) {
            anchor = anchor_over_corner(cx, p, &corner, anchor);
            if anchor.is_none() {
                self.on_end = true;
                self.corner = Some(corner.holder);
                self.snap.label(corner.at, "anchor");
                return (corner.at, None);
            }
        }
        if let Some((point, reference)) = anchor {
            self.snap.label(point, "anchor");
            return (point, Some(reference));
        }
        let drawn = on_dimensions(cx, p);
        if let Some((q, true)) = drawn {
            self.on_end = true;
            self.snap.label(q, "anchor");
            return (q, None);
        }
        if guided.distance(p) > 1e-9 || !self.snap.guides().is_empty() || cx.snap_to_pixel || cx.snap_to_grid {
            return (guided, None);
        }
        // Not on an anchor or a guide: snap to the closest point of the line (#2432), of the art
        // or of a dimension already drawn.
        let art = pick(cx, p).and_then(|e| nearest_on(cx, &e, p));
        let line = drawn.map(|(q, _)| q);
        match art.into_iter().chain(line).min_by(|a, b| a.distance(p).total_cmp(&b.distance(p))) {
            Some(q) => {
                self.snap.label(q, "path");
                (q, None)
            }
            None => (p, None),
        }
    }
    /// Moves the first Linear point to where its link puts it in the document as it is now: on
    /// its anchor, or where [`Self::first_link`] holds it. A point whose link no longer resolves,
    /// or that has none, stays where it was picked, and an anchor that no longer resolves is
    /// dropped from it.
    fn follow_first(&mut self, cx: &ToolContext) {
        if let Some(now) = self.first_now(cx)
            && let [(at, reference)] = self.points.as_mut_slice()
        {
            *at = now;
            if reference.as_ref().is_some_and(|r| r.position(cx.doc).is_none()) {
                *reference = None;
            }
        }
    }
    /// Where the first Linear point is in the document as it is now ([`Self::follow_first`]),
    /// while it is the only point picked.
    fn first_now(&self, cx: &ToolContext) -> Option<Point> {
        let [(at, reference)] = self.points.as_slice() else { return None };
        let now = match reference {
            Some(r) => r.position(cx.doc),
            None => self.first_link.as_ref().and_then(|link| link.position(cx.doc)),
        };
        Some(now.or(self.first_pick).unwrap_or(*at))
    }
    fn target(&self, cx: &ToolContext, p: Point) -> Option<Target> {
        let edge = pick(cx, p)?;
        match self.kind {
            Kind::Linear => Some(Target::Segment(edge)),
            Kind::Angular => {
                let (anchor, at) = anchor_near(cx, &edge, p)?;
                let (first, second) = dimension::corner(cx.doc, edge.node, edge.subpath, anchor)?;
                Some(Target::Corner { first, second, at })
            }
            Kind::Radial => {
                if let Some((center, radius)) = dimension::circle(cx.doc, edge.node) {
                    return Some(Target::Radial { source: DimensionSource::Circle { node: edge.node }, center, radius });
                }
                let (center, radius) = dimension::arc(cx.doc, &edge)?;
                Some(Target::Radial { source: DimensionSource::Arc { edge }, center, radius })
            }
        }
    }
    fn recipe(&self, mode: DimensionMode, a: Point, b: Point, vertex: Point, source: Option<DimensionSource>) -> Dimension {
        Dimension {
            mode,
            a,
            b,
            vertex,
            offset: 0.0,
            length: a.distance(b),
            options: self.style.clone(),
            source,
            orientation: Orientation::Aligned,
            moved: None,
        }
    }
    /// The dimension a target gives when dragged from.
    fn dimension_for(&self, target: &Target, p: Point) -> Option<Dimension> {
        let mut d = match target {
            Target::Segment(edge) => self.recipe(
                DimensionMode::Linear,
                Point::ZERO,
                Point::new(1.0, 0.0),
                Point::ZERO,
                Some(DimensionSource::Segment { edge: edge.clone() }),
            ),
            Target::Corner { first, second, .. } => self.recipe(
                DimensionMode::Angular,
                Point::ZERO,
                Point::new(1.0, 0.0),
                Point::ZERO,
                Some(DimensionSource::Angle { first: first.clone(), second: second.clone() }),
            ),
            Target::Radial { source, center, radius } => {
                // Dragged outwards: the radius; inwards: the diameter.
                let mode = if p.distance(*center) >= *radius { DimensionMode::Radius } else { DimensionMode::Diameter };
                self.recipe(mode, *center, p, Point::ZERO, Some(source.clone()))
            }
        };
        d.offset = 30.0;
        Some(d)
    }
    fn params(d: &Dimension) -> Value {
        let mut p = json!({"mode":d.mode,"a":[d.a.x,d.a.y],"b":[d.b.x,d.b.y],"vertex":[d.vertex.x,d.vertex.y],"offset":d.offset,"options":d.options});
        if let Some(source) = &d.source {
            p["source"] = json!(source);
        }
        if d.orientation != Orientation::Aligned {
            p["orientation"] = json!(d.orientation);
        }
        p
    }
    fn preview(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        self.level_guide = None;
        let Some(d) = &mut self.pending else { return vec![] };
        if matches!(d.mode, DimensionMode::Radius | DimensionMode::Diameter)
            && let Some(Target::Radial { center, radius, .. }) = self.press.as_ref().and_then(|(_, t)| t.as_ref())
        {
            d.mode = if p.distance(*center) >= *radius { DimensionMode::Radius } else { DimensionMode::Diameter };
        }
        if d.mode == DimensionMode::Linear {
            // The points the dimension measures: a dragged segment's ends, read from the art.
            let ends = match &d.source {
                Some(DimensionSource::Segment { edge }) => edge.endpoints(cx.doc),
                _ => Some((d.a, d.b)),
            };
            let again = self.second.is_some_and(|(at, pointer)| at.distance(p).min(pointer.distance(p)) <= cx.tol(ANCHOR_REACH));
            d.orientation = match ends {
                Some((a, b)) if !again => orientation_at(a, b, p),
                _ => Orientation::Aligned,
            };
        }
        // A dimension dragged out from art starts with placeholder points. Read the art's points
        // before placing the dimension at the pointer. Clear the offset first: an out-of-range
        // offset from the last pointer position fails the read.
        if !matches!(d.mode, DimensionMode::Radius | DimensionMode::Diameter) {
            d.offset = 0.0;
        }
        if d.resolve(cx.doc).is_err() {
            return vec![];
        }
        // With Smart Guides on, a linear dimension's line lands on the level of a parallel
        // dimension line already drawn within the Smart Guides snapping tolerance. The lines are
        // read at the first preview and, with Smart Guides on, again when the document they come
        // from has changed since. They come from the document the next preview applies to: while
        // the interaction is in progress, the document as it began, which each preview starts
        // again from, so a dimension that a command moves or hides in between, and that the next
        // preview puts back, stays a level where it was, and the previews themselves do not make
        // the tool read the lines again. The preview of this dimension is not one of them.
        if d.mode == DimensionMode::Linear {
            if !self.began {
                self.preview_id = cx.doc.peek_next_id();
            }
            let (from, at) = match cx.interaction_doc {
                Some(began) => (began, (cx.revision.0, 0, std::ptr::from_ref(began).addr())),
                None => (cx.doc, (cx.revision.0, cx.revision.1, 0)),
            };
            if !self.began || (cx.smart_guides && self.levels_at != at) {
                self.levels = dimension_lines(from, (self.preview_id, [d.a, d.b]));
                self.levels_at = at;
            }
        }
        let level = if cx.smart_guides { line_level(&self.levels, d, p, cx.snap_tol()) } else { None };
        d.place(level.map_or(p, |(on_level, _)| on_level));
        if d.validate().is_err() {
            return vec![];
        }
        self.level_guide = level.map(|(_, ends)| ends);
        let mut actions = vec![];
        if !self.began {
            actions.push(Action::Begin("Create Dimension".into()));
            self.began = true;
        }
        actions.push(Action::Preview("dimension.create".into(), Self::params(d)));
        actions
    }
    /// Places the pending dimension at `p` and commits it.
    fn place(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let mut out = self.preview(cx, p);
        if self.began {
            out.push(Action::Commit);
        }
        self.reset();
        out
    }
    fn reset(&mut self) {
        self.pending = None;
        self.press = None;
        self.press_on_anchor = false;
        self.on_end = false;
        self.corner = None;
        self.first_corner = None;
        self.first_link = None;
        self.first_pick = None;
        self.points.clear();
        self.lines.clear();
        self.dragging = false;
        self.began = false;
        self.snap.clear();
        self.snapped = None;
        self.second = None;
        self.level_guide = None;
        self.levels.clear();
    }
    /// A plain click: picks a point or a line, or measures a circle's radius.
    fn click(&mut self, cx: &ToolContext, p: Point, target: Option<Target>, mods: Mods) -> Vec<Action> {
        match self.kind {
            Kind::Linear => {
                let (mut at, mut reference) = self.snap_linear(cx, p);
                let mut corner = self.corner;
                if mods.shift
                    && let Some((last, _)) = self.points.last()
                {
                    let snapped = at;
                    at = constrained(cx, *last, at);
                    reference = reference.filter(|r| r.position(cx.doc) == Some(at));
                    corner = corner.filter(|_| at.distance(snapped) <= 1e-9);
                }
                if self.points.is_empty() {
                    self.first_corner = corner;
                    self.first_link = reference.is_none().then(|| own_link(cx, at, corner)).flatten();
                    self.first_pick = Some(at);
                }
                self.points.push((at, reference));
                if let [(a, ra), (b, rb)] = self.points.as_slice() {
                    let source = points_source(cx, (*a, ra, self.first_corner), (*b, rb, corner), self.first_link.take());
                    self.first_corner = None;
                    self.first_pick = None;
                    let mut d = self.recipe(DimensionMode::Linear, *a, *b, Point::ZERO, source);
                    if d.resolve(cx.doc).is_ok() {
                        self.second = Some((*b, p));
                        self.pending = Some(d);
                    }
                    self.points.clear();
                }
                vec![]
            }
            Kind::Angular => {
                let Some(edge) = pick(cx, p).filter(|e| e.is_straight(cx.doc)) else { return vec![] };
                if self.lines.first().is_some_and(|first| *first == edge) {
                    return vec![];
                }
                self.lines.push(edge);
                if let [first, second] = self.lines.as_slice() {
                    let mut d = self.recipe(
                        DimensionMode::Angular,
                        Point::ZERO,
                        Point::new(1.0, 0.0),
                        Point::ZERO,
                        Some(DimensionSource::Angle { first: first.clone(), second: second.clone() }),
                    );
                    if d.resolve(cx.doc).is_ok() {
                        d.offset = 30.0;
                        self.pending = Some(d);
                    }
                    self.lines.clear();
                }
                vec![]
            }
            Kind::Radial => {
                let Some(target @ Target::Radial { .. }) = target else { return vec![] };
                let Some(mut d) = self.dimension_for(&target, p) else { return vec![] };
                d.mode = DimensionMode::Radius;
                self.pending = Some(d);
                self.press = Some((p, Some(target)));
                self.place(cx, p)
            }
        }
    }
}

impl Tool for DimensionTool {
    fn id(&self) -> &'static str {
        "dimension"
    }
    fn busy(&self) -> bool {
        self.pending.is_some() || !self.points.is_empty() || !self.lines.is_empty()
    }
    fn cursor(&self, _: &ToolContext, _: Point, _: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        json!({"mode":self.kind.id(),"style":self.style})
    }
    fn set_option(&mut self, key: &str, value: &Value) {
        if key == "mode"
            && let Some(kind) = value.as_str().and_then(Kind::parse)
        {
            self.kind = kind;
        }
        if key == "style"
            && let Ok(o) = serde_json::from_value::<DimensionOptions>(value.clone())
            && o.validate().is_ok()
        {
            self.style = o;
        }
    }
    fn check_option(&self, key: &str, value: &Value) -> Result<(), String> {
        match key {
            "mode" => crate::option_check::one_of(key, value, &["linear", "angular", "radial", "radius", "diameter"]),
            "style" => DimensionOptions::from_params(value.clone()).map(|_| ()),
            _ => Err(crate::option_check::unknown(key)),
        }
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        self.at = ev.pos;
        self.shift = ev.mods.shift;
        match ev.kind {
            PointerKind::Move => {
                if self.pending.is_some() {
                    // Placing: the picks are done, their guides go.
                    self.snap.clear();
                    self.snapped = None;
                    return self.preview(cx, ev.pos);
                }
                self.hover = self.target(cx, ev.pos);
                self.snapped = (self.kind == Kind::Linear && self.lines.is_empty()).then(|| self.snap_linear(cx, ev.pos));
                if self.snapped.is_none() {
                    self.snap.clear();
                }
                vec![]
            }
            PointerKind::Down => {
                if self.pending.is_some() && !self.dragging {
                    self.press = Some((ev.pos, None));
                    return self.preview(cx, ev.pos);
                }
                // A Linear press on an anchor, on a corner of preview bounds or on the end of a
                // dimension's line, picks it unless it drags past the anchor's reach: moving a few
                // pixels before the release does not drag out the dimension of a segment under the
                // pointer.
                self.press_on_anchor = self.kind == Kind::Linear
                    && (anchor_under(cx, ev.pos).is_some()
                        || preview_corner(cx, ev.pos, &mut self.preview_boxes).is_some()
                        || on_dimensions(cx, ev.pos).is_some_and(|(_, end)| end));
                self.press = Some((ev.pos, self.target(cx, ev.pos)));
                vec![]
            }
            PointerKind::Drag => {
                if self.pending.is_some() && !self.dragging && self.press.is_some() {
                    return self.preview(cx, ev.pos);
                }
                if self.pending.is_none()
                    && let Some((start, Some(target))) = self.press.clone()
                    && start.distance(ev.pos) > cx.tol(if self.press_on_anchor { ANCHOR_REACH } else { 3.0 })
                    && self.points.is_empty()
                    && self.lines.is_empty()
                {
                    self.pending = self.dimension_for(&target, ev.pos);
                    self.dragging = self.pending.is_some();
                }
                if self.dragging { self.preview(cx, ev.pos) } else { vec![] }
            }
            PointerKind::Up => {
                if self.dragging {
                    return self.place(cx, ev.pos);
                }
                if self.pending.is_some() {
                    return if self.press.take().is_some() { self.place(cx, ev.pos) } else { vec![] };
                }
                match self.press.take() {
                    Some((start, target)) => self.click(cx, start, target, ev.mods),
                    _ => vec![],
                }
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _: &ToolContext, key: ToolKey, _: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            let cancel = self.began;
            self.reset();
            return if cancel { vec![Action::Cancel] } else { vec![] };
        }
        vec![]
    }
    /// A dropped drag is canceled as by Escape. A dropped click picks nothing, and the points or
    /// lines earlier clicks picked stay.
    fn drop_press(&mut self, cx: &ToolContext) -> Vec<Action> {
        if self.dragging {
            return self.key(cx, ToolKey::Escape, Mods::default());
        }
        self.press = None;
        if self.pending.is_some() && self.began {
            self.began = false;
            self.level_guide = None;
            vec![Action::Cancel]
        } else {
            vec![]
        }
    }
    fn deactivate(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.key(cx, ToolKey::Escape, Mods::default())
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let color = [60, 150, 255];
        let mut out = vec![];
        let segment = |edge: &SegmentRef, out: &mut Vec<Overlay>| {
            if let Some(sp) = cx.doc.node(edge.node).and_then(Node::path_data).and_then(|p| p.subpaths.get(edge.subpath))
                && edge.segment < sp.segment_count()
            {
                let c = sp.segment(edge.segment);
                let mut path = BezPath::new();
                path.move_to(c.p0);
                path.curve_to(c.p1, c.p2, c.p3);
                out.push(Overlay::Path { path, color, width: 3.0, dashed: false });
            }
        };
        if self.pending.is_none() {
            match &self.hover {
                Some(Target::Segment(edge)) => {
                    segment(edge, &mut out);
                    if let Some((a, b)) = edge.endpoints(cx.doc) {
                        for p in [a, b] {
                            out.push(Overlay::Anchor { p, color, filled: true, size: 6.0 });
                        }
                        if cx.measurement_labels
                            && let Ok(text) = self.style.label(a.distance(b), false)
                        {
                            out.push(Overlay::Measure { p: self.at + Vec2::new(12.0, 12.0), text });
                        }
                    }
                }
                Some(Target::Corner { first, second, at }) => {
                    segment(first, &mut out);
                    segment(second, &mut out);
                    out.push(Overlay::Anchor { p: *at, color, filled: true, size: 7.0 });
                }
                Some(Target::Radial { center, radius, .. }) => {
                    out.push(Overlay::Anchor { p: *center, color, filled: false, size: 6.0 });
                    let toward = self.at - *center;
                    if toward.hypot() > 1e-9 {
                        out.push(Overlay::Line { a: *center, b: *center + toward / toward.hypot() * *radius, color, dashed: true });
                    }
                }
                None => {}
            }
        }
        // The first point where the art puts it now, also when the art moved without a pointer move.
        let first = self.first_now(cx);
        for (p, _) in &self.points {
            out.push(Overlay::Anchor { p: first.unwrap_or(*p), color, filled: true, size: 6.0 });
        }
        for edge in &self.lines {
            segment(edge, &mut out);
        }
        // Linear: show where the next click lands (anchor ring, or a dot on a line), and draw the
        // rubber band to that point, so the user sees the snap before clicking (#2516).
        let linear_pick = self.kind == Kind::Linear && self.pending.is_none() && self.lines.is_empty();
        let snapped = self.snapped.clone().filter(|_| linear_pick);
        if let Some((q, reference)) = &snapped
            && (reference.is_some() || self.on_end || q.distance(self.at) > 1e-9)
        {
            let on_anchor = reference.is_some() || self.on_end;
            out.push(Overlay::Anchor { p: *q, color, filled: !on_anchor, size: if on_anchor { 10.0 } else { 6.0 } });
        }
        if let Some(p) = first.or_else(|| self.points.last().map(|(p, _)| *p)) {
            let target = snapped.map_or(self.at, |(q, _)| q);
            let b = if self.shift { constrained(cx, p, target) } else { target };
            out.push(Overlay::Line { a: p, b, color, dashed: true });
        }
        // The Smart Guides of the point the next click picks (after the rubber band).
        if linear_pick {
            out.extend(self.snap.guides().iter().cloned());
        }
        // A dimension line placed on the level of another: one alignment guide across both lines,
        // unless Alignment Guides are off.
        if cx.alignment_guides
            && let (Some(d), Some((c, e))) = (&self.pending, self.level_guide)
        {
            let (a, b) = d.line_ends();
            let u = d.direction();
            let along = |q: &Point| q.to_vec2().dot(u);
            let ends = [a, b, c, e];
            if let (Some(lo), Some(hi)) =
                (ends.iter().min_by(|p, q| along(p).total_cmp(&along(q))), ends.iter().max_by(|p, q| along(p).total_cmp(&along(q))))
            {
                out.push(Overlay::Line { a: *lo, b: *hi, color: cx.smart_guide_color, dashed: false });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    /// A dropped click picks nothing and keeps the point an earlier click picked. A dropped drag
    /// out from a segment is canceled as by Escape.
    #[test]
    fn a_dropped_press_keeps_the_earlier_picks() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y);
        let mut t = DimensionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 400.0, 400.0));
        t.pointer(&c, &ev(PointerKind::Up, 400.0, 400.0));
        assert_eq!(t.points.len(), 1);
        // The engine sends the release after the drop.
        t.pointer(&c, &ev(PointerKind::Down, 450.0, 420.0));
        assert!(t.drop_press(&c).is_empty());
        assert!(t.pointer(&c, &ev(PointerKind::Up, 450.0, 420.0)).is_empty());
        assert_eq!(t.points.len(), 1, "the earlier pick stays and the dropped click picks nothing");
        assert!(t.pending.is_none() && t.busy());
        let mut t = DimensionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 100.0));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 60.0));
        assert!(matches!(acts.first(), Some(Action::Begin(_))), "{acts:?}");
        assert_eq!(t.drop_press(&c), vec![Action::Cancel]);
        assert!(!t.busy());
        assert!(t.pointer(&c, &ev(PointerKind::Up, 150.0, 60.0)).is_empty(), "the release places nothing");
    }

    /// Of two dimension lines on one level, a line the pointer lies across wins over a line whose
    /// end is nearer the pointer.
    #[test]
    fn a_line_the_pointer_lies_across_wins_on_one_level() {
        let pt = Point::new;
        let d = DimensionTool::default().recipe(DimensionMode::Linear, pt(200.0, 200.0), pt(300.0, 200.0), pt(250.0, 200.0), None);
        let across = ((pt(0.0, 170.0), pt(400.0, 170.0)), Vec2::new(1.0, 0.0));
        let beside = ((pt(100.0, 170.0), pt(200.0, 170.0)), Vec2::new(1.0, 0.0));
        for levels in [[beside, across], [across, beside]] {
            let (at, ends) = line_level(&levels, &d, pt(250.0, 172.0), 4.0).expect("on the level");
            assert!(at.distance(pt(250.0, 170.0)) < 1e-9, "{at:?}");
            assert_eq!(ends, across.0);
        }
    }

    /// The lines of the dimensions in the document are read only for a linear dimension. They are
    /// read with Smart Guides off too, so a line placed after Smart Guides are turned on lines up.
    #[test]
    fn dimension_lines_are_read_only_for_a_linear_dimension() {
        use vectorcraft_doc::{Node, NodeKind};
        let (mut d, _) = doc_with_rect();
        let id = d.alloc_id();
        let mut drawn = Node::new(id, NodeKind::Group { children: vec![], clip: false });
        let pt = Point::new;
        drawn.extras_mut().dimension =
            Some(Box::new(DimensionTool::default().recipe(DimensionMode::Linear, pt(300.0, 300.0), pt(400.0, 300.0), pt(350.0, 300.0), None)));
        let l = d.layers[0].id;
        d.insert(Some(l), usize::MAX, drawn).unwrap();
        let (s, p) = (Selection::default(), paint());
        let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y);
        let c = cx(&d, &s, &p);
        let off = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
        let mut t = DimensionTool::default();
        for (x, y) in [(400.0, 400.0), (450.0, 400.0)] {
            t.pointer(&off, &ev(PointerKind::Down, x, y));
            t.pointer(&off, &ev(PointerKind::Up, x, y));
        }
        assert!(matches!(t.pointer(&off, &ev(PointerKind::Move, 420.0, 450.0)).first(), Some(Action::Begin(_))));
        assert_eq!(t.levels.len(), 1);
        let acts = t.pointer(&c, &ev(PointerKind::Move, 420.0, 302.0));
        let Some(Action::Preview(_, params)) = acts.last() else { panic!("{acts:?}") };
        assert_eq!(params["offset"], json!(-100.0), "on the level of the line drawn at y = 300");
        let mut t = DimensionTool::default();
        t.set_option("mode", &json!("angular"));
        t.pointer(&c, &ev(PointerKind::Down, 100.0, 100.0));
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Drag, 130.0, 140.0)).first(), Some(Action::Begin(_))));
        assert!(t.levels.is_empty(), "an angular dimension reads no lines");
        // A radius (dragged outwards from a circle) and a diameter (inwards) read none either.
        let circle = d.alloc_id();
        let round = vectorcraft_geom::shapes::ellipse(Rect::new(400.0, 150.0, 500.0, 250.0));
        d.insert(Some(l), usize::MAX, Node::path(circle, round, vectorcraft_doc::Appearance::default_art())).unwrap();
        let c = cx(&d, &s, &p);
        for (to, mode) in [(530.0, "radius"), (470.0, "diameter")] {
            let mut t = DimensionTool::default();
            t.set_option("mode", &json!("radial"));
            t.pointer(&c, &ev(PointerKind::Down, 500.0, 200.0));
            let acts = t.pointer(&c, &ev(PointerKind::Drag, to, 200.0));
            assert!(matches!(acts.first(), Some(Action::Begin(_))), "{mode}: {acts:?}");
            let Some(Action::Preview(_, params)) = acts.last() else { panic!("{mode}: {acts:?}") };
            assert_eq!(params["mode"], json!(mode));
            assert!(t.levels.is_empty(), "a {mode} dimension reads no lines");
        }
    }

    /// A dimension hidden, locked or deleted while another is placed is no longer a level, and one
    /// shown again or grouped is: the lines are read again when the document's revision moves. The
    /// preview of the dimension itself, with the id the document gave next when it began, is never
    /// a level.
    #[test]
    fn dimension_lines_are_read_again_when_the_document_changes() {
        use vectorcraft_doc::{Node, NodeKind};
        let (mut d, _) = doc_with_rect();
        let pt = Point::new;
        let line = |y: f64| Some(Box::new(DimensionTool::default().recipe(DimensionMode::Linear, pt(300.0, y), pt(400.0, y), pt(350.0, y), None)));
        let id = d.alloc_id();
        let mut drawn = Node::new(id, NodeKind::Group { children: vec![], clip: false });
        drawn.extras_mut().dimension = line(300.0);
        let l = d.layers[0].id;
        d.insert(Some(l), usize::MAX, drawn).unwrap();
        let (s, p) = (Selection::default(), paint());
        let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y);
        let offset = |t: &mut DimensionTool, c: &ToolContext| {
            let acts = t.pointer(c, &ev(PointerKind::Move, 420.0, 302.0));
            let Some(Action::Preview(_, params)) = acts.last() else { panic!("{acts:?}") };
            params["offset"].clone()
        };
        let mut t = DimensionTool::default();
        let c = cx(&d, &s, &p);
        for (x, y) in [(400.0, 400.0), (450.0, 400.0)] {
            t.pointer(&c, &ev(PointerKind::Down, x, y));
            t.pointer(&c, &ev(PointerKind::Up, x, y));
        }
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Move, 420.0, 450.0)).first(), Some(Action::Begin(_))));
        assert_eq!(offset(&mut t, &c), json!(-100.0), "on the level of the line drawn at y = 300");
        // With Smart Guides off the lines are not read; turned on, they are read as they are now.
        let mut hidden = d.clone();
        hidden.node_mut(id).unwrap().visible = false;
        let off = ToolContext { revision: (0, 1), smart_guides: false, ..cx(&hidden, &s, &p) };
        offset(&mut t, &off);
        assert_eq!(t.levels.len(), 1, "not read with Smart Guides off");
        let on = ToolContext { revision: (0, 1), ..cx(&hidden, &s, &p) };
        assert_eq!(offset(&mut t, &on), json!(-98.0), "read when Smart Guides are turned on");
        for (rev, edit) in [(1, "hidden"), (2, "locked"), (3, "deleted")] {
            let mut doc = d.clone();
            match edit {
                "hidden" => doc.node_mut(id).unwrap().visible = false,
                "locked" => doc.node_mut(id).unwrap().locked = true,
                _ => {
                    doc.remove(id).unwrap();
                }
            }
            let c = ToolContext { revision: (0, rev), ..cx(&doc, &s, &p) };
            assert_eq!(offset(&mut t, &c), json!(-98.0), "{edit}: off every level");
        }
        let c = ToolContext { revision: (0, 4), ..cx(&d, &s, &p) };
        assert_eq!(offset(&mut t, &c), json!(-100.0), "shown again: on the level");
        // Grouped while it is placed, in a group with the id the preview takes: still a level.
        let mut doc = d.clone();
        let moved = doc.remove(id).unwrap();
        let group = doc.alloc_id();
        doc.insert(Some(l), usize::MAX, Node::new(group, NodeKind::Group { children: vec![moved], clip: false })).unwrap();
        let c = ToolContext { revision: (0, 5), ..cx(&doc, &s, &p) };
        assert_eq!(offset(&mut t, &c), json!(-100.0), "grouped: on the level");
        // The dimension's own preview, which takes the next id and measures its points, is not a
        // level. Another dimension that takes that id is.
        let mut own = DimensionTool::default().recipe(DimensionMode::Linear, pt(400.0, 400.0), pt(450.0, 400.0), pt(425.0, 400.0), None);
        own.place(pt(425.0, 304.0));
        assert_eq!(own.line_ends(), (pt(400.0, 304.0), pt(450.0, 304.0)));
        for (rev, made, level) in [(6, Some(Box::new(own)), json!(-98.0)), (7, line(304.0), json!(-96.0))] {
            let mut doc = d.clone();
            let mut preview = Node::new(doc.alloc_id(), NodeKind::Group { children: vec![], clip: false });
            preview.extras_mut().dimension = made;
            doc.node_mut(id).unwrap().visible = false;
            doc.insert(Some(l), usize::MAX, preview).unwrap();
            let c = ToolContext { revision: (0, rev), ..cx(&doc, &s, &p) };
            assert_eq!(offset(&mut t, &c), level, "revision {rev}");
        }
    }

    /// While the interaction is in progress, the lines are read from the document it began with,
    /// which the next preview starts from, not from the document a command changed in between: a
    /// dimension moved or hidden there stays a level where it was.
    #[test]
    fn dimension_lines_are_read_from_the_document_the_preview_applies_to() {
        use vectorcraft_doc::{Node, NodeKind};
        let (mut d, _) = doc_with_rect();
        let pt = Point::new;
        let line = |y: f64| Some(Box::new(DimensionTool::default().recipe(DimensionMode::Linear, pt(300.0, y), pt(400.0, y), pt(350.0, y), None)));
        let id = d.alloc_id();
        let mut drawn = Node::new(id, NodeKind::Group { children: vec![], clip: false });
        drawn.extras_mut().dimension = line(300.0);
        let l = d.layers[0].id;
        d.insert(Some(l), usize::MAX, drawn).unwrap();
        let (s, p) = (Selection::default(), paint());
        let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y);
        let offset = |t: &mut DimensionTool, c: &ToolContext, y: f64| {
            let acts = t.pointer(c, &ev(PointerKind::Move, 420.0, y));
            let Some(Action::Preview(_, params)) = acts.last() else { panic!("{acts:?}") };
            params["offset"].clone()
        };
        let mut t = DimensionTool::default();
        let c = cx(&d, &s, &p);
        for (x, y) in [(400.0, 400.0), (450.0, 400.0)] {
            t.pointer(&c, &ev(PointerKind::Down, x, y));
            t.pointer(&c, &ev(PointerKind::Up, x, y));
        }
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Move, 420.0, 450.0)).first(), Some(Action::Begin(_))));
        for (rev, edit) in [(1, "moved"), (2, "hidden")] {
            let mut doc = d.clone();
            match edit {
                "moved" => doc.node_mut(id).unwrap().extras_mut().dimension = line(320.0),
                _ => doc.node_mut(id).unwrap().visible = false,
            }
            let c = ToolContext { revision: (0, rev), interaction_doc: Some(&d), ..cx(&doc, &s, &p) };
            assert_eq!(offset(&mut t, &c, 318.0), json!(-82.0), "{edit}: not on the level at y = 320");
            assert_eq!(offset(&mut t, &c, 302.0), json!(-100.0), "{edit}: on the level at y = 300");
        }
        // A revision that moves with the same document the interaction began with does not read
        // the lines again; another document it began with does.
        t.levels.clear();
        let c = ToolContext { revision: (0, 3), interaction_doc: Some(&d), ..cx(&d, &s, &p) };
        assert_eq!(offset(&mut t, &c, 302.0), json!(-98.0), "not read again");
        let again = d.clone();
        let c = ToolContext { revision: (0, 4), interaction_doc: Some(&again), ..cx(&d, &s, &p) };
        assert_eq!(offset(&mut t, &c, 302.0), json!(-100.0), "read again");
    }

    /// The preview of the dimension itself does not count toward the 20,000 objects read: a
    /// dimension that was the last one read stays a level once the preview is in front of it.
    #[test]
    fn the_preview_does_not_push_the_last_dimension_read_out_of_reach() {
        use std::sync::Arc;
        use vectorcraft_doc::{Node, NodeKind};
        let (mut d, _) = doc_with_rect();
        let pt = Point::new;
        let line = |y: f64| Some(Box::new(DimensionTool::default().recipe(DimensionMode::Linear, pt(300.0, y), pt(400.0, y), pt(350.0, y), None)));
        let mut drawn = Node::new(d.alloc_id(), NodeKind::Group { children: vec![], clip: false });
        drawn.extras_mut().dimension = line(300.0);
        // The layer, the group and 19,997 empty groups before the dimension: it is the 20,000th.
        let mut children = vec![Arc::new(drawn)];
        children.extend((0..19_997).map(|_| Arc::new(Node::new(d.alloc_id(), NodeKind::Group { children: vec![], clip: false }))));
        let l = d.layers[0].id;
        let group = Node::new(d.alloc_id(), NodeKind::Group { children, clip: false });
        d.insert(Some(l), usize::MAX, group).unwrap();
        let (s, p) = (Selection::default(), paint());
        let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y);
        let offset = |t: &mut DimensionTool, c: &ToolContext| {
            let acts = t.pointer(c, &ev(PointerKind::Move, 420.0, 302.0));
            let Some(Action::Preview(_, params)) = acts.last() else { panic!("{acts:?}") };
            params["offset"].clone()
        };
        let mut t = DimensionTool::default();
        let c = cx(&d, &s, &p);
        for (x, y) in [(400.0, 400.0), (450.0, 400.0)] {
            t.pointer(&c, &ev(PointerKind::Down, x, y));
            t.pointer(&c, &ev(PointerKind::Up, x, y));
        }
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Move, 420.0, 450.0)).first(), Some(Action::Begin(_))));
        assert_eq!(offset(&mut t, &c), json!(-100.0), "the 20,000th object is read");
        let mut previewed = d.clone();
        let mut preview = Node::new(previewed.alloc_id(), NodeKind::Group { children: vec![], clip: false });
        preview.extras_mut().dimension =
            Some(Box::new(DimensionTool::default().recipe(DimensionMode::Linear, pt(400.0, 400.0), pt(450.0, 400.0), pt(425.0, 304.0), None)));
        previewed.insert(Some(l), usize::MAX, preview).unwrap();
        let c = ToolContext { revision: (0, 1), ..cx(&previewed, &s, &p) };
        assert_eq!(offset(&mut t, &c), json!(-100.0), "still read with the preview in front");
    }

    #[test]
    fn a_dropped_placement_press_does_not_commit_the_dimension() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y);
        let mut t = DimensionTool::default();
        for (x, y) in [(400.0, 400.0), (450.0, 420.0)] {
            assert!(t.pointer(&c, &ev(PointerKind::Down, x, y)).is_empty());
            assert!(t.pointer(&c, &ev(PointerKind::Up, x, y)).is_empty());
        }
        assert!(t.pending.is_some());
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Move, 470.0, 450.0)).first(), Some(Action::Begin(_))));
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Down, 470.0, 450.0)).last(), Some(Action::Preview(_, _))));
        assert_eq!(t.drop_press(&c), vec![Action::Cancel]);
        assert!(t.pointer(&c, &ev(PointerKind::Up, 470.0, 450.0)).is_empty());
        assert!(t.pending.is_some(), "the dimension can still be placed");
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Down, 480.0, 460.0)).last(), Some(Action::Preview(_, _))));
        let dragged = t.pointer(&c, &ev(PointerKind::Drag, 490.0, 470.0));
        assert!(matches!(dragged.last(), Some(Action::Preview(_, _))), "{dragged:?}");
        let placed = t.pointer(&c, &ev(PointerKind::Up, 490.0, 470.0));
        assert!(matches!(placed.last(), Some(Action::Commit)), "{placed:?}");
    }

    /// A pointer position too far away to place the dimension shows no preview, and the preview
    /// comes back where the pointer returns.
    #[test]
    fn placement_recovers_after_a_position_out_of_range() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = DimensionTool::default();
        for (x, y) in [(400.0, 400.0), (450.0, 400.0)] {
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y));
            t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y));
        }
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Move, 420.0, 3.0e6)).is_empty());
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Move, 420.0, 450.0));
        let Some(Action::Preview(_, params)) = acts.last() else { panic!("{acts:?}") };
        assert_eq!(params["offset"], json!(50.0));
    }

    /// Dragging out from a segment previews the dimension at the pointer from the first drag on.
    #[test]
    fn the_first_preview_of_a_segment_drag_is_at_the_pointer() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = DimensionTool::default();
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 100.0));
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 150.0, 60.0));
        let Some(Action::Preview(_, params)) = acts.last() else { panic!("{acts:?}") };
        assert_eq!(params["offset"], json!(-40.0), "40 pt above the top edge");
    }

    /// Dragging out from a corner previews the angle at the pointer's distance from the corner
    /// from the first drag on.
    #[test]
    fn the_first_preview_of_a_corner_drag_is_at_the_pointer() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = DimensionTool::default();
        t.set_option("mode", &json!("angular"));
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 130.0, 140.0));
        let Some(Action::Preview(_, params)) = acts.last() else { panic!("{acts:?}") };
        assert_eq!(params["vertex"], json!([100.0, 100.0]));
        assert_eq!(params["offset"], json!(50.0), "50 pt from the corner");
    }

    /// A Linear press on an anchor picks that anchor even when the pointer moves a few pixels
    /// before the release: it does not drag out the dimension of a segment that ends there. Both
    /// picks keep their links to the path, so the dimension measures between the two anchors.
    #[test]
    fn a_press_on_an_anchor_picks_it_when_the_pointer_moves() {
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let ev = |kind, x: f64, y: f64| PointerEvent::new(kind, x, y);
        let mut t = DimensionTool::default();
        for ((x, y), (dx, dy)) in [((100.0, 100.0), (3.5, 1.0)), ((200.0, 200.0), (-4.0, -2.0))] {
            assert!(t.pointer(&c, &ev(PointerKind::Down, x, y)).is_empty());
            assert!(t.pointer(&c, &ev(PointerKind::Drag, x + dx, y + dy)).is_empty(), "no segment dimension is dragged out");
            assert!(t.pointer(&c, &ev(PointerKind::Up, x + dx, y + dy)).is_empty(), "nothing is committed");
        }
        let d = t.pending.as_ref().expect("both anchors were picked");
        assert_eq!((d.a, d.b), (Point::new(100.0, 100.0), Point::new(200.0, 200.0)));
        let anchor = |anchor| Some(PointRef { node: id, subpath: 0, anchor });
        assert_eq!(d.source, Some(DimensionSource::Points { a: anchor(0), b: anchor(2), vertex: None, free_a: None, free_b: None }));
        // Dragged past the anchor's reach, the press drags out the dimension of a segment there.
        let mut t = DimensionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 100.0, 100.0));
        assert!(t.pointer(&c, &ev(PointerKind::Drag, 103.0, 106.0)).is_empty());
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 100.0, 60.0));
        assert!(matches!(acts.first(), Some(Action::Begin(_))), "{acts:?}");
        assert!(matches!(t.pointer(&c, &ev(PointerKind::Up, 100.0, 60.0)).last(), Some(Action::Commit)));
    }

    /// A point picked on a segment, not on an anchor, keeps its place along the segment. A point
    /// off the art is held against the anchors of the other point's path. Both then follow the
    /// path, as the anchor at the other end does.
    #[test]
    fn points_off_anchors_are_held_against_the_path() {
        use vectorcraft_doc::dimension::FreePoint;
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let click = |t: &mut DimensionTool, x: f64, y: f64| {
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y));
            t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y));
        };
        let mut t = DimensionTool::default();
        click(&mut t, 100.0, 100.0);
        click(&mut t, 200.0, 150.0);
        let pending = t.pending.as_ref().expect("both points were picked");
        let anchor = Some(PointRef { node: id, subpath: 0, anchor: 0 });
        let Some(DimensionSource::Points { a, b: None, vertex: None, free_a: None, free_b: Some(FreePoint::OnSegment { edge, t }) }) =
            &pending.source
        else {
            panic!("{:?}", pending.source)
        };
        assert_eq!((a, edge), (&anchor, &SegmentRef { node: id, subpath: 0, segment: 1 }));
        assert!((t - 0.5).abs() < 1e-9, "{t}");
        let mut t = DimensionTool::default();
        click(&mut t, 400.0, 400.0);
        click(&mut t, 200.0, 200.0);
        let pending = t.pending.as_ref().expect("both points were picked");
        let Some(DimensionSource::Points { free_a: Some(free), b: Some(b), .. }) = &pending.source else { panic!("{:?}", pending.source) };
        let FreePoint::Anchors { node, anchors, across } = free else { panic!("{free:?}") };
        assert_eq!((*node, anchors.len(), *across, b.node, b.anchor), (id, 3, 0.0, id, 2));
        let at = free.position(&d).expect("the anchors that hold it exist");
        assert!(at.distance(pending.a) < 1e-9, "{at:?} {:?}", pending.a);
    }

    /// A point that Smart Guides put in line with the path's center is held, where the guide put
    /// it, against the anchors of the path at the other end.
    #[test]
    fn a_point_smart_guides_place_is_held_against_the_path() {
        use vectorcraft_doc::dimension::FreePoint;
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        assert!(c.smart_guides);
        let mut t = DimensionTool::default();
        for (x, y) in [(200.0, 200.0), (151.5, 330.0)] {
            t.pointer(&c, &PointerEvent::new(PointerKind::Move, x, y));
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y));
            t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y));
        }
        let pending = t.pending.as_ref().expect("both points were picked");
        assert!((pending.b.x - 150.0).abs() < 1e-9, "in line with the center: {:?}", pending.b);
        let Some(DimensionSource::Points { a: Some(a), free_b: Some(free), .. }) = &pending.source else { panic!("{:?}", pending.source) };
        assert!(matches!(free, FreePoint::Anchors { node, .. } if *node == id), "{free:?}");
        assert_eq!(a.node, id);
        let at = free.position(&d).expect("the anchors that hold it exist");
        assert!(at.distance(pending.b) < 1e-9, "{at:?} {:?}", pending.b);
    }

    /// With Use Preview Bounds on, clicks by the corners of a stroked path land on the corners of
    /// its preview bounds, held against the path, so the dimension measures the outside of the
    /// stroke as W and H do. Without a stroke, or with the preference off, the anchors are picked.
    #[test]
    fn with_preview_bounds_points_land_on_the_corners_of_the_stroke() {
        use vectorcraft_doc::dimension::FreePoint;
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let pick = |c: &ToolContext| {
            let mut t = DimensionTool::default();
            for (x, y) in [(101.0, 101.0), (199.0, 199.0)] {
                t.pointer(c, &PointerEvent::new(PointerKind::Move, x, y));
                t.pointer(c, &PointerEvent::new(PointerKind::Down, x, y));
                t.pointer(c, &PointerEvent::new(PointerKind::Up, x, y));
            }
            t.pending.expect("both points were picked")
        };
        let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
        let pending = pick(&c);
        let near = |a: Point, b: Point| a.distance(b) < 1e-9;
        assert!(near(pending.a, Point::new(99.5, 99.5)) && near(pending.b, Point::new(200.5, 200.5)), "{:?} {:?}", pending.a, pending.b);
        let Some(DimensionSource::Points { a: None, b: None, free_a: Some(free_a), free_b: Some(free_b), .. }) = &pending.source else {
            panic!("{:?}", pending.source)
        };
        for (free, at) in [(free_a, pending.a), (free_b, pending.b)] {
            assert!(matches!(free, FreePoint::Anchors { node, .. } if *node == id), "{free:?}");
            assert!(free.position(&d).expect("the anchors that hold it exist").distance(at) < 1e-9);
        }
        // A line of another path through the corner leaves the point held against the rectangle.
        let mut crossed = d.clone();
        let (l, line) = (crossed.layers[0].id, crossed.alloc_id());
        let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 99.5), Point::new(300.0, 99.5)], false));
        crossed.insert(Some(l), 1, Node::path(line, path, vectorcraft_doc::Appearance::default_art())).expect("the line");
        let pending = pick(&ToolContext { preview_bounds: true, ..cx(&crossed, &s, &p) });
        assert!(near(pending.a, Point::new(99.5, 99.5)), "{:?}", pending.a);
        let Some(DimensionSource::Points { free_a: Some(free_a), .. }) = &pending.source else { panic!("{:?}", pending.source) };
        assert!(matches!(free_a, FreePoint::Anchors { node, .. } if *node == id), "{free_a:?}");
        let anchors = |pending: &Dimension| (pending.a, pending.b, pending.source.clone());
        let anchor = |anchor| Some(PointRef { node: id, subpath: 0, anchor });
        let linked = DimensionSource::Points { a: anchor(0), b: anchor(2), vertex: None, free_a: None, free_b: None };
        let off = pick(&cx(&d, &s, &p));
        assert_eq!(anchors(&off), (Point::new(100.0, 100.0), Point::new(200.0, 200.0), Some(linked.clone())), "preference off");
        let mut bare = d.clone();
        bare.node_mut(id).expect("the rectangle").appearance = vectorcraft_doc::Appearance::default().into();
        let c = ToolContext { preview_bounds: true, ..cx(&bare, &s, &p) };
        assert_eq!(anchors(&pick(&c)), (Point::new(100.0, 100.0), Point::new(200.0, 200.0), Some(linked)), "no stroke");
        // A compound path's stroke: the points are held against the member that reaches the
        // corner, not the small hole drawn first.
        let mut compound = vectorcraft_doc::Document::new(500.0, 500.0);
        let (l, hole, part, whole) = (compound.layers[0].id, compound.alloc_id(), compound.alloc_id(), compound.alloc_id());
        let square = |id, a: f64, b: f64| {
            std::sync::Arc::new(Node::path(id, vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(a, a, b, b)), Default::default()))
        };
        let children = vec![square(hole, 140.0, 160.0), square(part, 100.0, 200.0)];
        let mut n = Node::new(whole, NodeKind::Compound { children, rule: Default::default() });
        n.appearance = vectorcraft_doc::Appearance::default_art().into();
        compound.insert(Some(l), 0, n).expect("the compound path");
        let pending = pick(&ToolContext { preview_bounds: true, ..cx(&compound, &s, &p) });
        assert!(near(pending.a, Point::new(99.5, 99.5)) && near(pending.b, Point::new(200.5, 200.5)), "{:?} {:?}", pending.a, pending.b);
        let Some(DimensionSource::Points { free_a: Some(free_a), free_b: Some(free_b), .. }) = &pending.source else {
            panic!("{:?}", pending.source)
        };
        for free in [free_a, free_b] {
            assert!(matches!(free, FreePoint::Anchors { node, .. } if *node == part), "{free:?}");
        }
        // A 40 point stroke, and an Offset Path effect on the fill, put the corners further from
        // the path than a click on the path reaches: the points still land there and stay linked.
        let mut wide = vectorcraft_doc::Appearance::default_art();
        wide.stroke_mut().expect("a stroke").width = 40.0;
        let mut offset = vectorcraft_doc::Appearance::default_art();
        let effect =
            vectorcraft_doc::Effect { id: "path.offsetPath".into(), params: serde_json::json!({"offset": 30, "joins": "miter"}), visible: true };
        offset.effects_mut(Some(0)).expect("the fill").push(effect);
        for (appearance, out) in [(wide, 20.0), (offset, 30.0)] {
            let mut d = d.clone();
            d.node_mut(id).expect("the rectangle").appearance = appearance.into();
            let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
            let mut t = DimensionTool::default();
            for (x, y) in [(101.0 - out, 101.0 - out), (199.0 + out, 199.0 + out)] {
                t.pointer(&c, &PointerEvent::new(PointerKind::Move, x, y));
                t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y));
                t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y));
            }
            let pending = t.pending.expect("both points were picked");
            let (a, b) = (Point::new(100.0 - out, 100.0 - out), Point::new(200.0 + out, 200.0 + out));
            assert!(near(pending.a, a) && near(pending.b, b), "{out}: {:?} {:?}", pending.a, pending.b);
            let Some(DimensionSource::Points { free_a: Some(free_a), free_b: Some(free_b), .. }) = &pending.source else {
                panic!("{out}: {:?}", pending.source)
            };
            for free in [free_a, free_b] {
                assert!(matches!(free, FreePoint::Anchors { node, .. } if *node == id), "{out}: {free:?}");
            }
        }
        // A press on a corner over another path's segment picks the corner when the pointer moves
        // a few pixels before the release, as on an anchor.
        let mut d = d.clone();
        let mut wide = vectorcraft_doc::Appearance::default_art();
        wide.stroke_mut().expect("a stroke").width = 40.0;
        d.node_mut(id).expect("the rectangle").appearance = wide.into();
        let (l, line) = (d.layers[0].id, d.alloc_id());
        let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 82.0), Point::new(95.0, 82.0)], false));
        d.insert(Some(l), 1, Node::path(line, path, Default::default())).expect("the line");
        let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
        let mut t = DimensionTool::default();
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 81.0, 81.0)).is_empty());
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 85.0, 81.0)).is_empty(), "no segment dimension is dragged out");
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 85.0, 81.0)).is_empty());
        assert!(t.points.first().is_some_and(|(q, _)| near(*q, Point::new(80.0, 80.0))), "{:?}", t.points);
    }

    /// A stroked compound path of a small hole and `extra` more tiny squares inside a square from
    /// 100 to 200, with the ids of the compound, the hole and the square.
    fn stroked_compound(extra: usize) -> (vectorcraft_doc::Document, [NodeId; 3]) {
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let (l, hole, part, whole) = (d.layers[0].id, d.alloc_id(), d.alloc_id(), d.alloc_id());
        let square =
            |id, a: f64, b: f64| std::sync::Arc::new(Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(a, a, b, b)), Default::default()));
        let mut children = vec![square(hole, 140.0, 160.0), square(part, 100.0, 200.0)];
        for _ in 0..extra {
            children.push(square(d.alloc_id(), 150.0, 151.0));
        }
        let mut n = Node::new(whole, NodeKind::Compound { children, rule: Default::default() });
        n.appearance = vectorcraft_doc::Appearance::default_art().into();
        d.insert(Some(l), 0, n).expect("the compound path");
        (d, [whole, hole, part])
    }

    /// The member holding each corner of a compound path is found when the compound is measured,
    /// and later pointer moves over the same revision take it from the cache without reading the
    /// members.
    #[test]
    fn a_compound_paths_corner_holders_are_found_once_per_revision() {
        let (d, [whole, hole, part]) = stroked_compound(0);
        let (s, p) = (Selection::default(), paint());
        let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
        let mut cache = PreviewBoxes::default();
        let corner = preview_corner(&c, Point::new(99.5, 99.5), &mut cache).expect("the corner of the stroke");
        assert_eq!(corner.holder, part, "the member that reaches the corner holds it");
        assert_eq!(cache.holders.get(&whole), Some(&[Some(part); 4]));
        // A move over the same revision reads the holder from the cache: one changed there is the
        // one it gives.
        cache.holders.insert(whole, [Some(hole); 4]);
        let corner = preview_corner(&c, Point::new(200.5, 200.5), &mut cache).expect("the corner of the stroke");
        assert_eq!(corner.holder, hole, "the cached holder is used");
    }

    /// A compound path with more members than one pointer move may read gives no corner, and is
    /// neither measured nor cached, so its member work stays bounded. A compound within the limit
    /// still gives its corner.
    #[test]
    fn a_compound_path_past_the_member_work_gives_no_corner() {
        let (s, p) = (Selection::default(), paint());
        // Two members plus `extra`: exactly the limit, then one more.
        for (extra, within) in [(PREVIEW_MEMBER_WORK - 2, true), (PREVIEW_MEMBER_WORK - 1, false)] {
            let (d, [whole, ..]) = stroked_compound(extra);
            let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
            let mut cache = PreviewBoxes::default();
            let corner = preview_corner(&c, Point::new(99.5, 99.5), &mut cache);
            assert_eq!(corner.is_some(), within, "{} members", extra + 2);
            assert_eq!(cache.preview.contains_key(&whole), within, "{} members", extra + 2);
            assert_eq!(cache.geometric.contains_key(&whole), within, "{} members", extra + 2);
        }
    }

    /// With Use Preview Bounds on, hovering by a corner of a stroked path's preview bounds, or
    /// right on it, shows the anchor ring there.
    #[test]
    fn hovering_a_corner_of_the_preview_bounds_shows_the_anchor_ring() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
        for (x, y) in [(101.0, 101.0), (99.5, 99.5)] {
            let mut t = DimensionTool::default();
            t.pointer(&c, &PointerEvent::new(PointerKind::Move, x, y));
            let ring = t
                .overlays(&c)
                .into_iter()
                .any(|o| matches!(o, Overlay::Anchor { p, filled: false, size, .. } if p.distance(Point::new(99.5, 99.5)) < 1e-9 && size == 10.0));
            assert!(ring, "({x}, {y})");
            let label = t
                .snap
                .guides()
                .iter()
                .any(|o| matches!(o, Overlay::Label { p, text, .. } if text == "anchor" && p.distance(Point::new(99.5, 99.5)) < 1e-9));
            assert!(label, "({x}, {y}): {:?}", t.snap.guides());
        }
    }

    /// With Use Preview Bounds on, a corner of the preview bounds takes a pick with Smart Guides and
    /// Snap to Point off, as an anchor does, and an anchor of other art nearer the pointer than the
    /// corner takes it from the corner. The point moves with the path, and a later change to the
    /// stroke weight leaves it where it was.
    #[test]
    fn preview_bounds_corners_take_picks_as_anchors_do() {
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let near = |a: Point, b: Point| a.distance(b) < 1e-9;
        let pick = |c: &ToolContext, at: [(f64, f64); 2]| {
            let mut t = DimensionTool::default();
            for (x, y) in at {
                t.pointer(c, &PointerEvent::new(PointerKind::Move, x, y));
                t.pointer(c, &PointerEvent::new(PointerKind::Down, x, y));
                t.pointer(c, &PointerEvent::new(PointerKind::Up, x, y));
            }
            t.pending.expect("both points were picked")
        };
        let unsnapped = ToolContext { preview_bounds: true, smart_guides: false, snap_to_point: false, ..cx(&d, &s, &p) };
        let pending = pick(&unsnapped, [(101.0, 101.0), (199.0, 199.0)]);
        assert!(near(pending.a, Point::new(99.5, 99.5)) && near(pending.b, Point::new(200.5, 200.5)), "{:?} {:?}", pending.a, pending.b);
        // An unstroked line whose end lies 2.5 points outside the corner.
        let mut other = d.clone();
        let (l, line) = (other.layers[0].id, other.alloc_id());
        let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(97.0, 97.0), Point::new(50.0, 50.0)], false));
        other.insert(Some(l), 1, Node::path(line, path, Default::default())).expect("the line");
        let c = ToolContext { preview_bounds: true, ..cx(&other, &s, &p) };
        let pending = pick(&c, [(97.5, 97.5), (199.0, 199.0)]);
        assert!(near(pending.a, Point::new(97.0, 97.0)), "the line's end is nearer: {:?}", pending.a);
        let Some(DimensionSource::Points { a: Some(a), .. }) = &pending.source else { panic!("{:?}", pending.source) };
        assert_eq!(a.node, line);
        let pending = pick(&c, [(99.0, 99.0), (199.0, 199.0)]);
        assert!(near(pending.a, Point::new(99.5, 99.5)), "the corner is nearer: {:?}", pending.a);
        // Right on the rectangle's anchor, with another path's anchor nearer than the corner: that
        // anchor takes the point, not the rectangle's own anchor on the stroke's centerline.
        let mut beside = d.clone();
        let (l, line) = (beside.layers[0].id, beside.alloc_id());
        let path =
            vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(100.5, 100.0), Point::new(150.0, 50.0)], false));
        beside.insert(Some(l), 1, Node::path(line, path, Default::default())).expect("the line");
        let pending = pick(&ToolContext { preview_bounds: true, ..cx(&beside, &s, &p) }, [(100.0, 100.0), (199.0, 199.0)]);
        let Some(DimensionSource::Points { a: Some(a), .. }) = &pending.source else { panic!("{:?}", pending.source) };
        assert!(near(pending.a, Point::new(100.5, 100.0)) && a.node == line, "{:?} {a:?}", pending.a);
        // The point held against the rectangle moves with it and stays put when the stroke changes.
        let pending = pick(&ToolContext { preview_bounds: true, ..cx(&d, &s, &p) }, [(101.0, 101.0), (199.0, 199.0)]);
        let Some(DimensionSource::Points { free_a: Some(free), .. }) = &pending.source else { panic!("{:?}", pending.source) };
        let mut moved = d.clone();
        moved.node_mut(id).expect("the rectangle").transform(vectorcraft_geom::Affine::translate((10.0, 0.0)), false);
        assert!(free.position(&moved).is_some_and(|q| near(q, Point::new(109.5, 99.5))), "{:?}", free.position(&moved));
        let mut wider = d.clone();
        let mut wide = vectorcraft_doc::Appearance::default_art();
        wide.stroke_mut().expect("a stroke").width = 40.0;
        wider.node_mut(id).expect("the rectangle").appearance = wide.into();
        assert!(free.position(&wider).is_some_and(|q| near(q, Point::new(99.5, 99.5))), "{:?}", free.position(&wider));
        // Scaled 200% with Scale Strokes off, the point moves with the anchors and no longer sits
        // on the corner of the preview bounds, which the 1 point stroke puts at (199.5, 199.5).
        let mut scaled = d.clone();
        scaled.node_mut(id).expect("the rectangle").transform(vectorcraft_geom::Affine::scale(2.0), false);
        assert!(free.position(&scaled).is_some_and(|q| near(q, Point::new(199.0, 199.0))), "{:?}", free.position(&scaled));
    }

    /// With Use Preview Bounds on, a corner takes the pick only from the anchors at the same corner
    /// of its object's geometric bounds. The other anchors of a small stroked circle stay
    /// pickable, so does the end of another line lying on the anchor the corner stands in for, and
    /// of two stroked rectangles sharing an edge, each corner of the shared anchors is held against
    /// its own rectangle.
    #[test]
    fn a_corner_takes_only_the_anchor_it_stands_for() {
        let (s, p) = (Selection::default(), paint());
        let near = |a: Point, b: Point| a.distance(b) < 1e-9;
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let (l, circle) = (d.layers[0].id, d.alloc_id());
        let path = vectorcraft_geom::shapes::ellipse(Rect::new(296.0, 296.0, 304.0, 304.0));
        d.insert(Some(l), 0, Node::path(circle, path, vectorcraft_doc::Appearance::default_art())).expect("the circle");
        let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
        let mut t = DimensionTool::default();
        let (q, reference) = t.snap_linear(&c, Point::new(300.0, 296.0));
        assert!(near(q, Point::new(300.0, 296.0)) && reference.is_some_and(|r| r.node == circle), "{q:?}");
        let (q, reference) = t.snap_linear(&c, Point::new(296.5, 296.5));
        assert!(near(q, Point::new(295.5, 295.5)) && reference.is_none() && t.corner == Some(circle), "{q:?}");
        // An unstroked line ending right on the rectangle's anchor: a click there picks the line's
        // end, and a click nearer the corner picks the corner.
        let (mut d, id) = doc_with_rect();
        let (l, line) = (d.layers[0].id, d.alloc_id());
        let path =
            vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 100.0), Point::new(50.0, 20.0)], false));
        d.insert(Some(l), 1, Node::path(line, path, Default::default())).expect("the line");
        let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
        let mut t = DimensionTool::default();
        let (q, reference) = t.snap_linear(&c, Point::new(100.0, 100.0));
        assert!(near(q, Point::new(100.0, 100.0)) && reference.is_some_and(|r| r.node == line), "{q:?}");
        let (q, reference) = t.snap_linear(&c, Point::new(99.6, 99.6));
        assert!(near(q, Point::new(99.5, 99.5)) && reference.is_none() && t.corner == Some(id), "{q:?}");
        // Another path's end right on the corner: a click on the rectangle's anchor picks that end.
        let (mut d, _) = doc_with_rect();
        let (l, line) = (d.layers[0].id, d.alloc_id());
        let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(99.5, 99.5), Point::new(50.0, 20.0)], false));
        d.insert(Some(l), 0, Node::path(line, path, Default::default())).expect("the line");
        let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
        let mut t = DimensionTool::default();
        let (q, reference) = t.snap_linear(&c, Point::new(100.0, 100.0));
        assert!(near(q, Point::new(99.5, 99.5)) && reference.as_ref().is_some_and(|r| r.node == line), "{q:?} {reference:?}");
        // A stroked dot, a path whose two anchors coincide, cannot hold a point at a corner: a
        // click by it picks its anchor.
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let (l, dot) = (d.layers[0].id, d.alloc_id());
        let path =
            vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(300.0, 300.0), Point::new(300.0, 300.0)], false));
        let mut round = vectorcraft_doc::Appearance::default_art();
        let stroke = round.stroke_mut().expect("a stroke");
        (stroke.width, stroke.cap) = (10.0, vectorcraft_doc::LineCap::Round);
        d.insert(Some(l), 0, Node::path(dot, path, round)).expect("the dot");
        let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
        let mut t = DimensionTool::default();
        let (q, reference) = t.snap_linear(&c, Point::new(296.0, 296.0));
        assert!(near(q, Point::new(300.0, 300.0)) && reference.is_some_and(|r| r.node == dot) && t.corner.is_none(), "{q:?}");
        // A stroked rectangle made into a guide, and a stroked member of a compound path without
        // a stroke of its own, give no corners.
        for kind in ["guide", "member"] {
            let (mut d, id) = doc_with_rect();
            let mut wide = vectorcraft_doc::Appearance::default_art();
            wide.stroke_mut().expect("a stroke").width = 40.0;
            if kind == "guide" {
                let n = d.node_mut(id).expect("the rectangle");
                n.appearance = wide.into();
                if let NodeKind::Path { guide, .. } = &mut n.kind {
                    *guide = true;
                }
            } else {
                let mut fresh = vectorcraft_doc::Document::new(500.0, 500.0);
                let (l, part, whole) = (fresh.layers[0].id, fresh.alloc_id(), fresh.alloc_id());
                let mut member = Node::path(part, vectorcraft_geom::shapes::rectangle(Rect::new(100.0, 100.0, 200.0, 200.0)), Default::default());
                member.appearance = wide.into();
                let n = Node::new(whole, NodeKind::Compound { children: vec![std::sync::Arc::new(member)], rule: Default::default() });
                fresh.insert(Some(l), 0, n).expect("the compound path");
                d = fresh;
            }
            let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
            let mut t = DimensionTool::default();
            let (q, _) = t.snap_linear(&c, Point::new(81.0, 81.0));
            assert!(!near(q, Point::new(80.0, 80.0)) && t.corner.is_none(), "{kind}: {q:?}");
        }
        // A visible, unlocked member of a stroked compound path made into a guide does not hold
        // the corner its box puts there.
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let (l, hole, part, whole) = (d.layers[0].id, d.alloc_id(), d.alloc_id(), d.alloc_id());
        let mut guide = Node::path(part, vectorcraft_geom::shapes::rectangle(Rect::new(100.0, 100.0, 200.0, 200.0)), Default::default());
        if let NodeKind::Path { guide, .. } = &mut guide.kind {
            *guide = true;
        }
        let small = Node::path(hole, vectorcraft_geom::shapes::rectangle(Rect::new(140.0, 140.0, 160.0, 160.0)), Default::default());
        let mut n =
            Node::new(whole, NodeKind::Compound { children: vec![std::sync::Arc::new(small), std::sync::Arc::new(guide)], rule: Default::default() });
        n.appearance = vectorcraft_doc::Appearance::default_art().into();
        d.insert(Some(l), 0, n).expect("the compound path");
        let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
        let mut t = DimensionTool::default();
        let (q, _) = t.snap_linear(&c, Point::new(99.6, 99.6));
        assert!(t.corner.is_none() && !near(q, Point::new(99.5, 99.5)), "{q:?} {:?}", t.corner);
        // Two stroked rectangles sharing the edge x = 200; the right one is on top.
        let (mut d, left) = doc_with_rect();
        let (l, right) = (d.layers[0].id, d.alloc_id());
        let path = vectorcraft_geom::shapes::rectangle(Rect::new(200.0, 100.0, 300.0, 200.0));
        d.insert(Some(l), 1, Node::path(right, path, vectorcraft_doc::Appearance::default_art())).expect("the right rectangle");
        let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
        for (pointer, corner, holder) in [((200.4, 99.6), (200.5, 99.5), left), ((199.6, 99.6), (199.5, 99.5), right)] {
            let mut t = DimensionTool::default();
            for kind in [PointerKind::Move, PointerKind::Down, PointerKind::Up] {
                t.pointer(&c, &PointerEvent::new(kind, pointer.0, pointer.1));
            }
            for kind in [PointerKind::Move, PointerKind::Down, PointerKind::Up] {
                t.pointer(&c, &PointerEvent::new(kind, 250.0, 300.0));
            }
            let pending = t.pending.expect("both points were picked");
            assert!(near(pending.a, Point::new(corner.0, corner.1)), "{pointer:?}: {:?}", pending.a);
            let Some(DimensionSource::Points { a: None, free_a: Some(free), .. }) = &pending.source else { panic!("{:?}", pending.source) };
            assert!(matches!(free, FreePoint::Anchors { node, .. } if *node == holder), "{pointer:?}: {free:?}");
        }
    }

    /// A point is held against a path for a corner of its preview bounds only when it was picked
    /// there: the same position picked another way links to the segment under it.
    #[test]
    fn a_point_is_held_for_a_corner_only_when_picked_on_it() {
        let (mut d, id) = doc_with_rect();
        let (l, line) = (d.layers[0].id, d.alloc_id());
        let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 99.5), Point::new(300.0, 99.5)], false));
        d.insert(Some(l), 1, Node::path(line, path, Default::default())).expect("the line");
        let (s, p) = (Selection::default(), paint());
        let c = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
        let (corner, b) = (Point::new(99.5, 99.5), Point::new(250.0, 99.5));
        for (held, node) in [(Some(id), id), (None, line)] {
            let source = points_source(&c, (corner, &None, held), (b, &None, None), None);
            let Some(DimensionSource::Points { free_a: Some(free), .. }) = &source else { panic!("{source:?}") };
            let linked = match free {
                FreePoint::Anchors { node, .. } => *node,
                FreePoint::OnSegment { edge, .. } => edge.node,
            };
            assert_eq!(linked, node, "{held:?}: {free:?}");
        }
    }

    /// A first point keeps its link when the art moves before the second click: a point picked on
    /// a corner of the preview bounds, on a segment or on an anchor is measured, shown and
    /// constrained by Shift from where the art puts it now.
    #[test]
    fn a_first_point_follows_art_moved_before_the_second_click() {
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let near = |a: Point, b: Point| a.distance(b) < 1e-9;
        let moved = |by: (f64, f64)| {
            let mut m = d.clone();
            m.node_mut(id).expect("the rectangle").transform(vectorcraft_geom::Affine::translate(by), false);
            m
        };
        let click = |t: &mut DimensionTool, c: &ToolContext, (x, y): (f64, f64), shift: bool| {
            for kind in [PointerKind::Move, PointerKind::Down, PointerKind::Up] {
                let mut ev = PointerEvent::new(kind, x, y);
                ev.mods.shift = shift;
                t.pointer(c, &ev);
            }
        };
        let base = |doc| ToolContext { preview_bounds: true, smart_guides: false, snap_to_point: false, ..cx(doc, &s, &p) };
        // A corner of the preview bounds, then the rectangle moved 20 points right.
        let right = moved((20.0, 0.0));
        let mut t = DimensionTool::default();
        click(&mut t, &base(&d), (101.0, 101.0), false);
        let c = ToolContext { revision: (0, 1), ..base(&right) };
        let shown = t.overlays(&c).into_iter().any(|o| matches!(o, Overlay::Anchor { p, filled: true, .. } if near(p, Point::new(119.5, 99.5))));
        assert!(shown, "the first point shows at the moved corner before the pointer moves");
        t.pointer(&c, &PointerEvent::new(PointerKind::Move, 219.0, 101.0));
        let band = t.overlays(&c).into_iter().any(|o| matches!(o, Overlay::Line { a, dashed: true, .. } if near(a, Point::new(119.5, 99.5))));
        assert!(band, "the rubber band starts at the moved corner");
        click(&mut t, &c, (219.0, 101.0), false);
        let pending = t.pending.take().expect("both points were picked");
        assert!(near(pending.a, Point::new(119.5, 99.5)) && near(pending.b, Point::new(220.5, 99.5)), "{:?} {:?}", pending.a, pending.b);
        assert!((pending.length - 101.0).abs() < 1e-9, "{}", pending.length);
        let Some(DimensionSource::Points { free_a: Some(free), .. }) = &pending.source else { panic!("{:?}", pending.source) };
        assert!(free.position(&right).is_some_and(|q| near(q, Point::new(119.5, 99.5))), "{free:?}");
        // The same corner, the rectangle moved 20 points right with the pointer moving, then
        // deleted: the first point goes back to where it was picked.
        let mut gone = right.clone();
        gone.remove(id).expect("the rectangle");
        let (l, line) = (gone.layers[0].id, gone.alloc_id());
        let path =
            vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 300.0), Point::new(400.0, 300.0)], false));
        gone.insert(Some(l), 0, Node::path(line, path, Default::default())).expect("the line");
        let mut t = DimensionTool::default();
        click(&mut t, &base(&d), (101.0, 101.0), false);
        let c = ToolContext { revision: (0, 1), ..base(&right) };
        t.pointer(&c, &PointerEvent::new(PointerKind::Move, 219.0, 101.0));
        let c = ToolContext { revision: (0, 2), ..base(&gone) };
        t.pointer(&c, &PointerEvent::new(PointerKind::Move, 300.0, 300.0));
        let shown = t.overlays(&c).into_iter().any(|o| matches!(o, Overlay::Anchor { p, filled: true, .. } if near(p, Point::new(99.5, 99.5))));
        assert!(shown, "the first point shows where it was picked once its rectangle is deleted");
        click(&mut t, &c, (300.0, 300.0), false);
        let pending = t.pending.take().expect("both points were picked");
        assert!(near(pending.a, Point::new(99.5, 99.5)), "{:?}", pending.a);
        // An anchor, then its rectangle deleted: the dimension starts where the anchor was picked.
        let mut t = DimensionTool::default();
        let unbounded = |doc| ToolContext { preview_bounds: false, ..base(doc) };
        click(&mut t, &unbounded(&d), (100.0, 100.0), false);
        click(&mut t, &ToolContext { revision: (0, 2), ..unbounded(&gone) }, (300.0, 300.0), false);
        let pending = t.pending.take().expect("both points were picked");
        assert!(near(pending.a, Point::new(100.0, 100.0)), "{:?}", pending.a);
        let Some(DimensionSource::Points { a: None, .. }) = &pending.source else { panic!("{:?}", pending.source) };
        // A point on the top segment, then the rectangle moved 20 points down.
        let down = moved((0.0, 20.0));
        let mut t = DimensionTool::default();
        click(&mut t, &unbounded(&d), (150.0, 100.0), false);
        click(&mut t, &ToolContext { revision: (0, 1), ..unbounded(&down) }, (300.0, 300.0), false);
        let pending = t.pending.take().expect("both points were picked");
        assert!(near(pending.a, Point::new(150.0, 120.0)), "{:?}", pending.a);
        let Some(DimensionSource::Points { free_a: Some(FreePoint::OnSegment { edge, .. }), .. }) = &pending.source else {
            panic!("{:?}", pending.source)
        };
        assert_eq!(edge.node, id);
        // The same, with a line drawn on top along the moved segment before the second click: the
        // point keeps the segment it was picked on, where a pick there now finds the line.
        let mut covered = down.clone();
        let (l, line) = (covered.layers[0].id, covered.alloc_id());
        let path =
            vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 120.0), Point::new(300.0, 120.0)], false));
        covered.insert(Some(l), 0, Node::path(line, path, Default::default())).expect("the line");
        let c = unbounded(&covered);
        let at = Point::new(150.0, 120.0);
        assert_eq!(pick(&c, at).map(|e| e.node), Some(line));
        let link = own_link(&unbounded(&d), Point::new(150.0, 100.0), None);
        assert!(matches!(&link, Some(FreePoint::OnSegment { edge, .. }) if edge.node == id), "{link:?}");
        let source = points_source(&c, (at, &None, None), (Point::new(300.0, 300.0), &None, None), link);
        let Some(DimensionSource::Points { free_a: Some(FreePoint::OnSegment { edge, .. }), .. }) = &source else { panic!("{source:?}") };
        assert_eq!(edge.node, id, "the line on top does not take the point");
        // An anchor, then the rectangle moved 20 points down: Shift constrains from the anchor's
        // new place.
        let mut t = DimensionTool::default();
        click(&mut t, &unbounded(&d), (100.0, 100.0), false);
        click(&mut t, &ToolContext { revision: (0, 1), ..unbounded(&down) }, (300.0, 124.0), true);
        let pending = t.pending.take().expect("both points were picked");
        assert!(near(pending.a, Point::new(100.0, 120.0)) && (pending.b.y - 120.0).abs() < 1e-9, "{:?} {:?}", pending.a, pending.b);
    }

    /// Hovering right on the end of a dimension's line shows the anchor ring there.
    #[test]
    fn hovering_right_on_a_dimension_lines_end_shows_the_anchor_ring() {
        let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
        let (l, id) = (d.layers[0].id, d.alloc_id());
        let path =
            vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(300.0, 300.0), Point::new(400.0, 300.0)], false));
        let mut n = Node::path(id, path, Default::default());
        let recipe = DimensionTool::default().recipe(DimensionMode::Linear, Point::new(0.0, 0.0), Point::new(50.0, 0.0), Point::ZERO, None);
        n.extras_mut().dimension = Some(Box::new(recipe));
        d.insert(Some(l), 0, n).expect("the dimension's line");
        let (s, p) = (Selection::default(), paint());
        let c = ToolContext { smart_guides: true, snap_to_point: true, ..cx(&d, &s, &p) };
        let mut t = DimensionTool::default();
        t.pointer(&c, &PointerEvent::new(PointerKind::Move, 300.0, 300.0));
        let ring = t
            .overlays(&c)
            .into_iter()
            .any(|o| matches!(o, Overlay::Anchor { p, filled: false, size, .. } if p.distance(Point::new(300.0, 300.0)) < 1e-9 && size == 10.0));
        assert!(ring, "{:?}", t.overlays(&c));
    }

    /// The anchors of a live rectangle are picked the same way.
    #[test]
    fn a_press_on_a_live_shapes_anchor_picks_it() {
        let (d, id) = doc_with_live_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = DimensionTool::default();
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 100.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 197.0, 102.0));
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 197.0, 102.0));
        let [(at, Some(r))] = t.points.as_slice() else { panic!("{:?}", t.points) };
        assert_eq!((*at, r.node), (Point::new(200.0, 100.0), id));
    }

    /// An anchor of a compound path inside nested groups, lying just inside another object's
    /// edge, is picked and linked, though that edge is nearer the pointer than any of the
    /// compound path's own segments. A press there that moves a few pixels picks it too.
    #[test]
    fn an_anchor_under_another_objects_edge_is_picked() {
        use std::sync::Arc;
        use vectorcraft_doc::{Appearance, NodeKind};
        use vectorcraft_geom::Rect;
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let (outer, inner, compound) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let mut path = vectorcraft_geom::shapes::rectangle(Rect::new(20.0, 20.0, 40.0, 40.0));
        path.subpaths.extend(vectorcraft_geom::shapes::rectangle(Rect::new(150.0, 103.0, 180.0, 130.0)).subpaths);
        let art = Node::path(compound, path, Appearance::default_art());
        let inner_group = Node::new(inner, NodeKind::Group { children: vec![Arc::new(art)], clip: false });
        let group = Node::new(outer, NodeKind::Group { children: vec![Arc::new(inner_group)], clip: false });
        d.insert(Some(l), 0, group).unwrap();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let corner = (Point::new(150.0, 103.0), Some(PointRef { node: compound, subpath: 1, anchor: 0 }));
        let mut t = DimensionTool::default();
        assert_eq!(t.snap_linear(&c, Point::new(151.0, 101.0)), corner);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 151.0, 101.0));
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 155.0, 100.0)).is_empty(), "no segment dimension is dragged out");
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 155.0, 100.0));
        assert_eq!(t.points, vec![corner]);
    }

    /// Beside the two points the dimension is aligned; past either point it is horizontal when
    /// the pointer's quarter holds the straight up or down direction and vertical when it holds
    /// the straight left or right direction. Points on one horizontal or vertical line, a pointer
    /// on the line through the points, and two equal points give aligned.
    #[test]
    fn the_pointer_chooses_aligned_horizontal_or_vertical() {
        use Orientation::{Aligned, Horizontal, Vertical};
        let at = |a: (f64, f64), b: (f64, f64), p: (f64, f64)| orientation_at(Point::new(a.0, a.1), Point::new(b.0, b.1), Point::new(p.0, p.1));
        // A line rising to the right (y grows downwards), and the same points in the other order.
        for (a, b) in [((100.0, 300.0), (200.0, 200.0)), ((200.0, 200.0), (100.0, 300.0))] {
            for (p, want) in [
                ((120.0, 160.0), Aligned),
                ((200.0, 300.0), Aligned),
                ((150.0, 250.0), Aligned),
                ((200.0, 200.0), Aligned),
                ((100.0, 300.0), Aligned),
                ((210.0, 120.0), Horizontal),
                ((240.0, 150.0), Horizontal),
                ((280.0, 220.0), Vertical),
                ((240.0, 170.0), Vertical),
                ((40.0, 310.0), Vertical),
                ((90.0, 380.0), Horizontal),
                ((250.0, 150.0), Aligned),
            ] {
                assert_eq!(at(a, b, p), want, "{a:?} to {b:?}, pointer at {p:?}");
            }
        }
        // A line falling to the right.
        let (a, b) = ((100.0, 100.0), (300.0, 200.0));
        for (p, want) in
            [((200.0, 50.0), Aligned), ((320.0, 300.0), Horizontal), ((400.0, 210.0), Vertical), ((60.0, 20.0), Horizontal), ((20.0, 90.0), Vertical)]
        {
            assert_eq!(at(a, b, p), want, "pointer at {p:?}");
        }
        for (a, b) in [((100.0, 100.0), (300.0, 100.0)), ((100.0, 100.0), (100.0, 300.0)), ((100.0, 100.0), (100.0, 100.0))] {
            for p in [(50.0, 50.0), (350.0, 50.0), (350.0, 350.0), (50.0, 350.0), (200.0, 60.0), (60.0, 200.0), (400.0, 100.0), (100.0, 400.0)] {
                assert_eq!(at(a, b, p), Aligned, "{a:?} to {b:?}, pointer at {p:?}");
            }
        }
    }

    /// A point that Smart Guides leave where the pointer already is (the pointer right in line
    /// with a target) stays on the guide the overlays show. It does not move to the nearest point
    /// of a segment within the pick's reach. Off the guides, a point near a segment still lands
    /// on it, also straight below the first point: the second point has no construction guide
    /// through a first point that is not on an anchor.
    #[test]
    fn a_point_on_a_guide_stays_on_the_guide() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let click = |t: &mut DimensionTool, x: f64, y: f64| {
            t.pointer(&c, &PointerEvent::new(PointerKind::Move, x, y));
            t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y));
            t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y));
        };
        let mut t = DimensionTool::default();
        // In line with the rectangle's center, 6 units right of its right edge.
        assert_eq!(t.snap_linear(&c, Point::new(206.0, 150.0)), (Point::new(206.0, 150.0), None));
        assert!(!t.snap.guides().is_empty());
        // Off the guides, beside the edge: the closest point of the edge.
        let (on_edge, reference) = t.snap_linear(&c, Point::new(206.0, 170.5));
        assert!(on_edge.distance(Point::new(200.0, 170.5)) < 1e-4 && reference.is_none(), "{on_edge:?}");
        // The first point, above the rectangle, then straight down past the right edge: no
        // vertical guide through the first point holds the second, so beside the edge it lands
        // on the edge, and in line with the center it stays in line.
        click(&mut t, 206.0, 50.0);
        assert_eq!(t.points, vec![(Point::new(206.0, 50.0), None)]);
        for y in [170.5, 190.0] {
            let (q, reference) = t.snap_linear(&c, Point::new(206.0, y));
            assert!(q.distance(Point::new(200.0, y)) < 1e-4 && reference.is_none(), "beside the edge at y = {y}: {q:?}");
        }
        assert_eq!(t.snap_linear(&c, Point::new(207.0, 300.0)), (Point::new(207.0, 300.0), None), "away from the art");
        assert!(t.snap.guides().is_empty());
        assert_eq!(t.snap_linear(&c, Point::new(206.0, 150.0)), (Point::new(206.0, 150.0), None));
        assert!(!t.snap.guides().is_empty());
        click(&mut t, 206.0, 150.0);
        let dim = t.pending.as_ref().expect("both points were picked");
        assert_eq!((dim.a, dim.b), (Point::new(206.0, 50.0), Point::new(206.0, 150.0)));
        // The second point, off the edge, is held against the rectangle's anchors.
        let Some(DimensionSource::Points { free_b: Some(free), .. }) = &dim.source else { panic!("{:?}", dim.source) };
        assert!(matches!(free, FreePoint::Anchors { .. }), "{free:?}");
        // A pointer already on a gridline crossing or on a whole pixel stays there too, and one
        // off the grid lands on the grid, not on the edge.
        let grid = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
        let step = grid.grid_step();
        let on_grid = Point::new((200.0 / step).ceil() * step, (170.0 / step).round() * step);
        assert!(on_grid.x > 200.0 + 1e-6 && on_grid.x - 200.0 < 8.0, "{on_grid:?} is off the edge, within the pick's reach");
        assert_eq!(DimensionTool::default().snap_linear(&grid, on_grid), (on_grid, None));
        let off_grid = on_grid + Vec2::new(-step * 0.3, 0.2);
        assert_eq!(DimensionTool::default().snap_linear(&grid, off_grid), (on_grid, None));
        let pixel = ToolContext { snap_to_pixel: true, ..cx(&d, &s, &p) };
        assert_eq!(DimensionTool::default().snap_linear(&pixel, Point::new(206.0, 170.0)), (Point::new(206.0, 170.0), None));
    }

    /// After a first point that is not on an anchor, the second point still lands where the line
    /// from the first point meets a slanted segment square or grazes a curve, with the
    /// "perpendicular" or "tangent" label, at places no construction guide through the first
    /// point reaches. With Snap to Perpendicular and Snap to Tangent off it stays at the pointer.
    #[test]
    fn the_second_point_lands_square_to_a_segment_or_grazing_a_curve() {
        use vectorcraft_doc::Appearance;
        use vectorcraft_geom::{Rect, shapes};
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let (line, circle) = (d.alloc_id(), d.alloc_id());
        let slanted = shapes::line(Point::new(300.0, 100.0), Point::new(400.0, 150.0));
        d.insert(Some(l), 0, Node::path(line, slanted, Appearance::default_art())).unwrap();
        let round = shapes::ellipse(Rect::new(420.0, 370.0, 480.0, 430.0));
        d.insert(Some(l), 0, Node::path(circle, round, Appearance::default_art())).unwrap();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let first = Point::new(300.0, 250.0);
        let picked = |c: &ToolContext| {
            let mut t = DimensionTool::default();
            for kind in [PointerKind::Move, PointerKind::Down, PointerKind::Up] {
                t.pointer(c, &PointerEvent::new(kind, first.x, first.y));
            }
            assert_eq!(t.points, vec![(first, None)]);
            t
        };
        let labeled = |t: &DimensionTool, label: &str| t.snap.guides().iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == label));
        // The foot of the perpendicular from (300, 250) to the line is (360, 130), along no
        // construction guide angle from the first point.
        let mut t = picked(&c);
        let (q, reference) = t.snap_linear(&c, Point::new(361.0, 132.0));
        assert!(q.distance(Point::new(360.0, 130.0)) < 1e-4 && reference.is_none(), "{q:?}");
        assert!(labeled(&t, "perpendicular"), "{:?}", t.snap.guides());
        // Where a line from the first point grazes the circle about (450, 400), radius 30.
        let (center, radius) = (Point::new(450.0, 400.0), 30.0);
        let away = first - center;
        let angle = away.y.atan2(away.x) + (radius / away.hypot()).acos();
        let touch = center + Vec2::new(angle.cos(), angle.sin()) * radius;
        let (q, _) = t.snap_linear(&c, touch + Vec2::new(0.5, 0.5));
        assert!((q.distance(center) - radius).abs() < 0.05, "{q:?} is on the circle");
        assert!(((q - first).normalize().dot((q - center).normalize())).abs() < 0.01, "{q:?} grazes the circle");
        assert!(labeled(&t, "tangent"), "{:?}", t.snap.guides());
        // With both options off, the pointer near the foot lands on the closest point of the line
        // instead.
        let off = ToolContext { snap_perpendicular: false, snap_tangent: false, ..cx(&d, &s, &p) };
        let mut t = picked(&off);
        let (q, _) = t.snap_linear(&off, Point::new(361.0, 132.0));
        assert!(q.distance(Point::new(360.0, 130.0)) > 0.5 && !labeled(&t, "perpendicular"), "{q:?}");
    }

    /// After a first point picked on an anchor, the second point still lands on the construction
    /// guides through it.
    #[test]
    fn the_second_point_after_an_anchor_lands_on_construction_guides() {
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = DimensionTool::default();
        for kind in [PointerKind::Move, PointerKind::Down, PointerKind::Up] {
            t.pointer(&c, &PointerEvent::new(kind, 200.0, 200.0));
        }
        assert_eq!(t.points, vec![(Point::new(200.0, 200.0), Some(PointRef { node: id, subpath: 0, anchor: 2 }))]);
        // 1.5 units right and below the 45° guide up and to the right of the anchor.
        let (q, reference) = t.snap_linear(&c, Point::new(331.0, 72.0));
        assert!(q.distance(Point::new(329.5, 70.5)) < 1e-4 && reference.is_none(), "{q:?}");
        assert!(!t.snap.guides().is_empty());
        // The same offset from a first point off the anchors stays at the pointer.
        let mut t = DimensionTool::default();
        for kind in [PointerKind::Move, PointerKind::Down, PointerKind::Up] {
            t.pointer(&c, &PointerEvent::new(kind, 240.0, 240.0));
        }
        assert_eq!(t.points, vec![(Point::new(240.0, 240.0), None)]);
        assert_eq!(t.snap_linear(&c, Point::new(371.0, 112.0)), (Point::new(371.0, 112.0), None));
        assert!(t.snap.guides().is_empty());
    }

    #[test]
    fn placement_press_previews_without_a_prior_move() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = DimensionTool::default();
        for (x, y) in [(400.0, 400.0), (450.0, 420.0)] {
            assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, x, y)).is_empty());
            assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, x, y)).is_empty());
        }
        assert!(t.pending.is_some());
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 470.0, 450.0));
        assert!(matches!(acts.first(), Some(Action::Begin(_))), "{acts:?}");
        assert!(matches!(acts.last(), Some(Action::Preview(_, _))), "{acts:?}");
    }
}
