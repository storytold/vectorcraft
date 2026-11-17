use super::cache::RegionCache;
use super::lookup::{
    connected_edges, connected_edges_through, edges, face_at, faces, flood_faces, is_live_paint, leaf_shapes, live_paint_at, live_paint_at_where,
    lp_stroke_paint, node_outline, visible_edge_near, visible_edges_along, visible_face_at,
};
use super::map::{BuilderMap, HIGHLIGHT_RED, sorted_roots};
use super::{
    Action, Arc, BezPath, CLICK_TOL_PX, Cursor, Document, EDGE_NAME, FACE_NAME, FillRule, Mods, Node, NodeId, Overlay, Point, PointerEvent,
    PointerKind, Rect, Tool, ToolContext, ToolKey, Value, color_swatches, json, json_ids, path_contains, pick_swatch, plain, swatch_chips,
};
use vectorcraft_color::Paint;
use vectorcraft_pathops as po;

// ---------- Live Paint Bucket ----------

/// What the Live Paint tools highlight under the cursor.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum LpHover {
    #[default]
    None,
    /// A face of an existing Live Paint group.
    Part(BezPath),
    /// An edge of an existing Live Paint group.
    Edge(BezPath),
    /// A region of the (not yet live) selection.
    Region(usize),
}

/// The Live Paint group and part the Live Paint tools hover and click at `p`. Inside a visible face
/// of a visible Live Paint group ([`live_paint_at`]) it is that group with its visible face under
/// `p`, or with `edge` its visible edge nearest `p` within `tol` (None when there is none). Outside
/// every visible face, with `edge`, it is the visible edge nearest `p` within `tol` among the
/// visible Live Paint groups (the front-most group's on a tie). None when no group is found. Only
/// the groups for which `takes` is true are looked at: [`lp_visible`] for the Live Paint Bucket and
/// [`lp_selectable`] for Live Paint Selection.
fn lp_part_at<'a>(doc: &'a Document, p: Point, tol: f64, edge: bool, takes: &dyn Fn(NodeId) -> bool) -> Option<(NodeId, Option<&'a Arc<Node>>)> {
    if let Some(g) = live_paint_at_where(doc, p, takes).and_then(|id| doc.node(id)) {
        return Some((g.id, if edge { visible_edge_near(g, p, tol) } else { visible_face_at(g, p) }));
    }
    if !edge {
        return None;
    }
    let mut best: Option<(f64, NodeId, &Arc<Node>)> = None;
    doc.walk(|n| {
        if is_live_paint(n)
            && let Some(e) = visible_edge_near(n, p, tol)
            && let Some((.., d)) = e.path_data().and_then(|pd| pd.nearest(p))
            && best.is_none_or(|b| d <= b.0)
            && takes(n.id)
        {
            best = Some((d, n.id, e));
        }
    });
    best.map(|(_, g, e)| (g, Some(e)))
}

/// The face and edge of a Live Paint group at `p`, among the groups `takes` takes, by the rules of
/// [`lp_part_at`], in one walk of the document. Inside a visible face of a group, the face is the
/// front-most such group's visible face under `p`, and with `edge` the edge is that group's visible
/// edge nearest `p` within `tol`. Outside every visible face there is no face, and with `edge` the
/// edge is the visible edge nearest `p` within `tol` among the groups (the front-most group's on a
/// tie).
fn lp_face_and_edge<'a>(
    doc: &'a Document,
    p: Point,
    tol: f64,
    edge: bool,
    takes: &dyn Fn(NodeId) -> bool,
) -> (Option<&'a Arc<Node>>, Option<&'a Arc<Node>>) {
    let mut in_face: Option<(&Arc<Node>, Option<&Arc<Node>>)> = None;
    let mut best: Option<(f64, &Arc<Node>)> = None;
    doc.walk(|n| {
        if !is_live_paint(n) || !takes(n.id) {
            return;
        }
        let near = if edge { visible_edge_near(n, p, tol) } else { None };
        if let Some(f) = visible_face_at(n, p) {
            in_face = Some((f, near));
        }
        if let Some(e) = near
            && let Some((.., d)) = e.path_data().and_then(|pd| pd.nearest(p))
            && best.is_none_or(|b| d <= b.0)
        {
            best = Some((d, e));
        }
    });
    match in_face {
        Some((f, e)) => (Some(f), e),
        None => (None, best.map(|b| b.1)),
    }
}

/// Whether the Live Paint Bucket takes Live Paint group `id`: it is visible, with every layer and
/// group around it.
fn lp_visible(cx: &ToolContext, id: NodeId) -> bool {
    cx.doc.is_visible(id)
}

/// Whether the Live Paint Selection tool takes Live Paint group `id`: it is editable
/// ([`Document::is_editable`]: visible, and not locked, on a locked layer or on a locked artboard),
/// and in isolation mode it is the isolated object or inside it ([`crate::direct::in_scope`]).
fn lp_selectable(cx: &ToolContext, id: NodeId) -> bool {
    cx.doc.is_editable(id) && crate::direct::in_scope(cx.doc, cx.isolation, id)
}

/// Whether every selected object is a Live Paint group (true with nothing selected). A selection
/// that also holds other paths has regions outside its groups' faces that a click fills, making
/// one Live Paint group of the selection.
fn lp_sel_all_live(cx: &ToolContext) -> bool {
    cx.selection.objects.iter().all(|id| cx.doc.node(*id).is_some_and(is_live_paint))
}

/// What the Live Paint tools highlight at `p`: the part of a Live Paint group ([`lp_part_at`], among
/// the groups `takes` takes), else with `cache` the region of the selection under `p` when the
/// selection is not only Live Paint groups ([`lp_sel_all_live`]). A large selection's regions are
/// found on another thread ([`RegionCache::ready`]), and until they are, no region is highlighted.
fn lp_hover(cx: &ToolContext, cache: Option<&mut RegionCache>, p: Point, edge: bool, takes: &dyn Fn(NodeId) -> bool) -> LpHover {
    if let Some((_, part)) = lp_part_at(cx.doc, p, cx.tol(4.0), edge, takes) {
        let path = part.and_then(|n| n.path_data()).map(|pd| pd.to_bezpath());
        return path.map(|bp| if edge { LpHover::Edge(bp) } else { LpHover::Part(bp) }).unwrap_or_default();
    }
    if !lp_sel_all_live(cx)
        && let Some(i) = cache.and_then(|c| c.ready(cx, true)).and_then(|m| m.region_at(p))
    {
        return LpHover::Region(i);
    }
    LpHover::None
}

/// The highlight of `h` in `color`: a face or region `width` screen pixels wide, an edge
/// `edge_width` screen pixels wide.
fn lp_overlays(h: &LpHover, cache: &RegionCache, color: [u8; 3], width: f32, edge_width: f32) -> Vec<Overlay> {
    match h {
        LpHover::None => vec![],
        LpHover::Part(bp) => vec![Overlay::Path { path: bp.clone(), color, width, dashed: false }],
        LpHover::Edge(bp) => vec![Overlay::Path { path: bp.clone(), color, width: edge_width, dashed: false }],
        LpHover::Region(i) => {
            cache.map.regions.get(*i).map(|(_, bp, _)| vec![Overlay::Path { path: bp.clone(), color, width, dashed: false }]).unwrap_or_default()
        }
    }
}

/// The Live Paint Bucket's highlight width (screen pixels) by default.
pub const LP_BUCKET_WIDTH: f64 = 4.0;

/// The least and greatest highlight width (screen pixels) the Live Paint Bucket takes.
pub const LP_BUCKET_WIDTH_RANGE: (f64, f64) = (1.0, 20.0);

/// The width of the Live Paint Bucket's edge highlight, as a fraction of its Width setting: a
/// face is outlined at the Width and an edge at this fraction of it.
pub const LP_BUCKET_EDGE_FRACTION: f64 = 0.5;

/// What a Live Paint Bucket click paints.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LpPaint {
    Nothing,
    /// The face under the pointer.
    Fills,
    /// The edge nearest the pointer.
    Strokes,
    /// The edge nearest the pointer when one is in reach, else the face under the pointer.
    Both,
}

/// Live Paint Bucket: by default, a click fills the face under the cursor with the current fill,
/// and dragging fills the faces crossed in that group as one undo step (Escape cancels).
/// Shift-click paints the nearest edge with the current stroke. Alt-click samples a fill or
/// stroke into the paint defaults; Alt+Shift-click samples only the color under the pointer.
/// Clicking selected ordinary paths without Alt makes them a Live Paint group first. Its Tool
/// Options (double-click the tool) are `paintFills` (on) and `paintStrokes` (off), `highlight`
/// (on), the highlight `color` (`#rrggbb`, red by default) and its `width` ([`LP_BUCKET_WIDTH`]
/// pixels): a face is outlined at the width and an edge at [`LP_BUCKET_EDGE_FRACTION`] of it.
/// With Paint Strokes alone on, a click paints the edge nearest the pointer and
/// Shift-click fills the face. With both on, a click paints the edge nearest the pointer when one
/// is within 4 pixels, else it fills the face, and Shift-click fills the face. With both off, a
/// click paints nothing. A drag from where a click paints an edge paints that edge and every edge
/// of the same group that the drag crosses or touches with the current stroke, as one undo step
/// (Escape cancels); a drag from where a click fills a face fills only faces. With Highlight off,
/// the face or edge under the pointer is not highlighted. The cursor is a filled bucket where a
/// click fills a face, a brush where it paints an edge and an empty bucket where it paints nothing;
/// with Alt it is the Eyedropper.
///
/// A double-click on a face fills it and the faces reached from it across edges that draw no
/// stroke (`scope: "connected"`); on an edge it paints the edges joined to it that have its stroke.
/// A triple-click on a face fills every face of the group that has the face's fill, and on an edge
/// it paints every edge of the group that draws with the edge's stroke paint (`scope:
/// "samePaint"`); each of them takes the bucket's paint. Each click of a double-click or
/// triple-click paints as it arrives, as its own undo step when it changes a paint, and the paint
/// compared is the one the face or edge had before the first of its clicks that painted it,
/// including a click whose drag or release crossed it. A first click
/// that made the Live Paint group counts when the face it filled is the clicked face. Shift and the
/// options choose a face or an edge as for a click; with Alt or Cmd/Ctrl held a double-click or
/// triple-click paints nothing. With Space held for the view tools, a click, double-click or triple-click paints
/// nothing and no face or edge is highlighted.
///
/// The Left and Right arrow keys without Shift, Alt or Cmd/Ctrl set the active paint (the fill,
/// or the stroke when the Stroke proxy is in front) to the color swatch before or after it in the
/// Swatches panel, wrapping around; from a color that is no swatch, Right picks the first swatch
/// and Left the last. The selected art keeps its paint. With `cursorSwatchPreview` (on), the
/// active paint shows as a chip above the pointer between the swatches before and after it,
/// except during a fill or stroke drag or with Alt held; a solid color that is no swatch shows alone, and a
/// paint that is not a solid color shows no chip.
pub struct LivePaintBucketTool {
    pub(super) cache: RegionCache,
    pub(super) hover: LpHover,
    /// Where the pointer last moved, and the modifiers held there.
    at: Option<(Point, Mods)>,
    /// The command (`livePaint.fill` or `livePaint.strokeEdge`), its params and the polyline of a
    /// drag, previewed from the press's snapshot.
    fill_drag: Option<(&'static str, Value, Vec<Vec<Point>>)>,
    /// What a drag paints, as it was at its press.
    drag_from: LpDragFrom,
    /// The last [`LP_PRESSES`] presses, oldest first: the face or edge each one painted, with its
    /// paint before that press, or None for a press that painted no face or edge.
    presses: Vec<Option<LpPress>>,
    fills: bool,
    strokes: bool,
    highlight: bool,
    color: [u8; 3],
    width: f64,
    /// Cursor Swatch Preview.
    preview: bool,
    /// The index in [`color_swatches`] of the swatch the arrow keys picked last for the fill and for
    /// the stroke.
    picked: [Option<usize>; 2],
}

/// The face or edge a Live Paint Bucket press paints.
#[derive(Clone, Debug, PartialEq)]
struct LpPress {
    /// The face or edge; None for the face of the Live Paint group the press makes.
    part: Option<NodeId>,
    /// Where the press was.
    at: Point,
    /// Its fill (a face), or the stroke paint it draws with ([`lp_stroke_paint`], an edge), before
    /// the press.
    before: Paint,
    /// The other faces or edges the double-click that followed the press reached, each with its
    /// fill or drawn stroke paint before the double-click.
    also: Vec<(NodeId, Paint)>,
    /// The other faces or edges the press's drag crossed, its release included, each with its fill
    /// or drawn stroke paint before the press.
    dragged: Vec<(NodeId, Paint)>,
    /// For a press that makes the Live Paint group: what the group is made from, with the path
    /// chunks of the press's drag, its release included.
    dragged_made: Option<Arc<(LpMade, Vec<Vec<Point>>)>>,
}

/// What a Live Paint Bucket fill or stroke drag paints, as it was at the press.
#[derive(Default)]
enum LpDragFrom {
    #[default]
    None,
    /// The faces of an existing Live Paint group.
    Faces(Vec<Arc<Node>>),
    /// What the group is made from, for a drag that makes the group.
    Made(LpMade),
    /// The Live Paint group whose edges a stroke drag paints.
    Edges(Arc<Node>),
}

/// The faces among `faces` (a Live Paint group's faces, back to front) that a drag along the path
/// chunks `chunks` crosses, as `livePaint.fill` finds them, each with its fill.
fn crossed_faces(faces: &[Arc<Node>], chunks: &[Vec<Point>]) -> Vec<(NodeId, Paint)> {
    // Only a face whose bounds meet the drag's bounds can be crossed.
    let Some(drag) = chunks.iter().flatten().fold(None, |r: Option<Rect>, p| Some(r.map_or(Rect::from_points(*p, *p), |r| r.union_pt(*p)))) else {
        return vec![];
    };
    let mut hit = vec![];
    let mut regions = vec![];
    // Front first, as visible_face_at looks.
    for f in faces.iter().rev().filter(|f| f.visible) {
        if let Some(path) = f.path_data()
            && let Some(bounds) = path.bounds()
            && bounds.x0 <= drag.x1
            && drag.x0 <= bounds.x1
            && bounds.y0 <= drag.y1
            && drag.y0 <= bounds.y1
        {
            hit.push(f);
            regions.push((po::Region { path: path.clone(), sources: vec![] }, path.to_bezpath(), bounds));
        }
    }
    let map = BuilderMap { regions, ..Default::default() };
    let mut crossed: Vec<(NodeId, Paint)> = vec![];
    for chunk in chunks {
        for f in map.hits_along(chunk, None).regions.iter().filter_map(|i| hit.get(*i)) {
            if !crossed.iter().any(|(id, _)| *id == f.id) {
                crossed.push((f.id, f.appearance.fill_paint()));
            }
        }
    }
    crossed
}

/// What the Live Paint group a fill press makes from the selection is made from, as it was at the
/// press, following Live Paint Make and Merge.
#[derive(Debug, PartialEq)]
struct LpMade {
    /// The paint Merge keeps from the faces of the first selected Live Paint group that draw: a
    /// point inside each face with its fill, given to the made face that holds the point.
    kept: Vec<(Point, Paint)>,
    /// The paths the faces are made from, back to front: Merge's group's source paths first, then
    /// the other objects', a Live Paint group giving its source paths.
    sources: Vec<Node>,
}

/// What the Live Paint group the selected `roots` make is made from ([`LpMade`]).
fn made_leaves(doc: &Document, roots: &[NodeId]) -> LpMade {
    let nodes: Vec<&Node> = roots.iter().filter_map(|id| doc.node(*id)).collect();
    let target = nodes.iter().position(|n| is_live_paint(n));
    let mut kept = vec![];
    let mut sources = vec![];
    for i in target.into_iter().chain((0..nodes.len()).filter(|i| Some(*i) != target)) {
        let Some(n) = nodes.get(i) else { continue };
        if !is_live_paint(n) {
            leaf_shapes(n, &mut sources);
            continue;
        }
        for s in super::lookup::sources(n).and_then(|g| g.children()).into_iter().flatten() {
            leaf_shapes(s, &mut sources);
        }
        if Some(i) == target {
            let draws = |f: &&Arc<Node>| f.visible && f.opacity > 0.0 && (f.appearance.has_drawn_fill() || f.appearance.has_drawn_stroke());
            // Merge fixes a kept unplaced gradient to its fit on the face it comes from.
            let kept_fill = |f: &Arc<Node>| {
                let mut n = (**f).clone();
                n.pin_gradients();
                n.appearance.fill_paint()
            };
            kept.extend(faces(n).into_iter().filter(draws).filter_map(|f| Some((po::interior_point(f.path_data()?)?, kept_fill(f)))));
        }
    }
    LpMade { kept, sources: sources.into_iter().cloned().collect() }
}

/// The fill a face made at `p` from the source paths of `made` gets: the topmost fill that draws
/// of the front-most source path whose closed subpaths hold `p` and that has such a fill, else
/// none. Open subpaths bound no face.
fn made_face_fill(made: &LpMade, p: Point) -> Paint {
    let holds = |l: &Node| {
        node_outline(l).is_some_and(|(path, rule)| {
            let closed = vectorcraft_geom::PathData::new(path.subpaths.into_iter().filter(|sp| sp.closed && sp.anchors.len() >= 2).collect());
            !closed.is_empty() && path_contains(&closed, rule, p)
        })
    };
    // As Make: the front-most covering source with a fill that draws (shown, with paint and an
    // opacity above 0, on a source with an opacity above 0) gives the face its topmost such fill,
    // an unplaced gradient fixed to its fit on the source.
    made.sources
        .iter()
        .rev()
        .filter(|l| l.opacity > 0.0 && l.appearance.has_drawn_fill() && holds(l))
        .find_map(|l| {
            let mut n = l.clone();
            n.pin_gradients();
            n.appearance.items.iter().rev().find_map(|i| match i {
                vectorcraft_doc::AppearanceItem::Fill(f) if f.visible && !f.paint.is_none() && f.opacity > 0.0 => Some(f.paint.clone()),
                _ => None,
            })
        })
        .unwrap_or(Paint::None)
}

/// The fill face `face` of Live Paint group `group` was made with from `made`: the kept paint
/// whose point the face holds (the last one, as Merge applies them in order), else the fill of the
/// source path at a point inside the face.
fn made_part_fill(made: &LpMade, group: &Node, face: &Node) -> Paint {
    if let Some((_, paint)) = made.kept.iter().rev().find(|(p, _)| face_at(group, *p).is_some_and(|f| f.id == face.id)) {
        return paint.clone();
    }
    face.path_data().and_then(po::interior_point).map_or(Paint::None, |p| made_face_fill(made, p))
}

/// How many presses the Live Paint Bucket keeps: the three of a triple-click.
const LP_PRESSES: usize = 3;

/// Maximum points in one Live Paint fill path chunk.
pub const LP_FILL_CHUNK_POINTS: usize = 4096;

impl Default for LivePaintBucketTool {
    fn default() -> Self {
        Self {
            cache: RegionCache::live_paint(),
            hover: LpHover::None,
            at: None,
            fill_drag: None,
            drag_from: LpDragFrom::None,
            presses: Vec::new(),
            fills: true,
            strokes: false,
            highlight: true,
            color: HIGHLIGHT_RED,
            width: LP_BUCKET_WIDTH,
            preview: true,
            picked: [None; 2],
        }
    }
}

impl LivePaintBucketTool {
    /// The active paint the arrow keys step and the preview shows: the fill, or the stroke when the
    /// Stroke proxy is in front.
    fn active_paint<'a>(cx: &'a ToolContext) -> &'a vectorcraft_color::Paint {
        if cx.fill_active { &cx.paint.fill } else { &cx.paint.stroke }
    }

    /// The slot of [`Self::picked`] for the active paint.
    fn picked_slot(cx: &ToolContext) -> usize {
        usize::from(!cx.fill_active)
    }

    /// Start a drag pressed at `p` that previews `cmd` with `params` and the drag's path.
    fn begin_fill(&mut self, cmd: &'static str, params: Value, p: Point) -> Vec<Action> {
        self.fill_drag = Some((cmd, params.clone(), vec![vec![p]]));
        vec![Action::Begin("Live Paint Bucket".into()), Action::Preview(cmd.into(), params)]
    }

    fn drag_fill(&mut self, p: Point) -> Vec<Action> {
        let Some((cmd, params, chunks)) = &mut self.fill_drag else { return vec![] };
        let cmd = *cmd;
        let Some(points) = chunks.last_mut() else { return vec![] };
        let Some(&last) = points.last() else { return vec![] };
        if last == p {
            return vec![];
        }
        if points.len() == LP_FILL_CHUNK_POINTS {
            // Share the boundary point so no segment is lost between chunks.
            chunks.push(vec![last, p]);
        } else {
            points.push(p);
        }
        let mut params = params.clone();
        let mut chunks = chunks.iter().map(|chunk| json!(chunk.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>()));
        if let Some(first) = chunks.next() {
            params["points"] = first;
        }
        let continuation: Vec<_> = chunks.collect();
        if !continuation.is_empty() {
            params["continuation"] = Value::Array(continuation);
        }
        vec![Action::Preview(cmd.into(), params)]
    }

    /// What a click paints, Shift held or not. Shift switches Paint Fills alone to Paint Strokes
    /// alone and back, and switches both to Paint Fills alone.
    fn paints(&self, shift: bool) -> LpPaint {
        match (self.fills, self.strokes, shift) {
            (false, false, _) => LpPaint::Nothing,
            (true, true, false) => LpPaint::Both,
            (true, false, false) | (_, true, true) => LpPaint::Fills,
            (true, false, true) | (false, true, false) => LpPaint::Strokes,
        }
    }

    /// Note a press: what it paints, None when it paints no face or edge.
    fn note_press(&mut self, press: Option<LpPress>) {
        if self.presses.len() == LP_PRESSES {
            self.presses.remove(0);
        }
        self.presses.push(press);
    }

    /// Note a press at `at` that paints face or edge (`edge`) `n`.
    fn note_part(&mut self, n: &Node, edge: bool, at: Point) {
        let before = if edge { lp_stroke_paint(n) } else { n.appearance.fill_paint() };
        self.note_press(Some(LpPress { part: Some(n.id), at, before, also: vec![], dragged: vec![], dragged_made: None }));
    }

    /// End the drag, noting on the last press the other faces or edges its drag crossed, with their
    /// fills or drawn stroke paints before the press (for a press that makes the group, what the
    /// group is made from and the drag, for [`Self::before`] to look the faces up in the group
    /// made). → whether a drag was on.
    fn end_fill(&mut self) -> bool {
        let from = std::mem::take(&mut self.drag_from);
        let Some((_, _, chunks)) = self.fill_drag.take() else { return false };
        let Some(Some(last)) = self.presses.last_mut() else { return true };
        match from {
            LpDragFrom::None => {}
            LpDragFrom::Faces(faces) => {
                let part = last.part;
                last.dragged = crossed_faces(&faces, &chunks).into_iter().filter(|(id, _)| part != Some(*id)).collect();
            }
            LpDragFrom::Made(made) => last.dragged_made = Some(Arc::new((made, chunks))),
            LpDragFrom::Edges(group) => {
                let part = last.part;
                last.dragged =
                    visible_edges_along(&group, &chunks).into_iter().filter(|e| part != Some(e.id)).map(|e| (e.id, lp_stroke_paint(e))).collect();
            }
        }
        true
    }

    /// Note that the double-click after the last press also reaches `parts` of group `group`, with
    /// the paint each has now (a face's fill, or an edge's drawn stroke paint with `edge`).
    fn note_reached(&mut self, group: &Node, parts: Vec<NodeId>, edge: bool) {
        let Some(Some(last)) = self.presses.last_mut() else { return };
        let reached: std::collections::HashSet<NodeId> = parts.into_iter().filter(|id| last.part != Some(*id)).collect();
        last.also = group
            .children()
            .into_iter()
            .flatten()
            .filter(|c| reached.contains(&c.id))
            .map(|n| (n.id, if edge { lp_stroke_paint(n) } else { n.appearance.fill_paint() }))
            .collect();
    }

    /// The paint `part` had before the first press of a click sequence of `clicks` presses that
    /// painted `part`, or before the double-click after a press when that double-click reached it
    /// (the sequence's first press counts when it made the group `part` is in and the face it
    /// filled there is `part`), a press's drag or release crossing `part` included; `now` when no
    /// press of the sequence painted `part` and the last press's drag did not cross it.
    fn before(&self, cx: &ToolContext, part: NodeId, clicks: usize, now: Paint) -> Paint {
        let Some(run) = self.presses.len().checked_sub(clicks).and_then(|from| self.presses.get(from..)) else { return now };
        let made_part = |at: Point| live_paint_at(cx.doc, at).and_then(|g| visible_face_at(cx.doc.node(g)?, at)).is_some_and(|f| f.id == part);
        let on_part = |i: usize, q: &LpPress| q.part == Some(part) || (i == 0 && q.part.is_none() && made_part(q.at));
        let dragged = |q: &LpPress| q.dragged.iter().any(|(id, _)| *id == part);
        // An earlier press of the sequence that painted `part` counts too: a press on the next edge
        // released back on `part` paints that edge, and the double-click targets `part`.
        let painted = run.iter().enumerate().any(|(i, q)| q.as_ref().is_some_and(|q| on_part(i, q)));
        if !painted && !run.last().and_then(Option::as_ref).is_some_and(dragged) {
            return now;
        }
        for (i, q) in run.iter().enumerate() {
            let Some(q) = q else { continue };
            if on_part(i, q) {
                return q.before.clone();
            }
            // A face of the group the press made (the one under the press) that its drag crossed has
            // the fill it was made with.
            if i == 0
                && let Some(drag) = &q.dragged_made
                && let Some(group) = live_paint_at(cx.doc, q.at).and_then(|g| cx.doc.node(g))
                && let Some(face) = faces(group).into_iter().find(|f| f.id == part)
                && !crossed_faces(std::slice::from_ref(face), &drag.1).is_empty()
            {
                return made_part_fill(&drag.0, group, face);
            }
            if let Some((_, paint)) = q.dragged.iter().chain(&q.also).find(|(id, _)| *id == part) {
                return paint.clone();
            }
        }
        now
    }

    /// What a double-click (`scope` "connected", 2 `clicks`) or a triple-click ("samePaint", 3) at
    /// `p` runs: the command a click there runs, with `scope`, and the paint it compares as `match`.
    /// A double-click notes the other faces or edges it reaches, for the triple-click's `match`.
    fn multi_click(&mut self, cx: &ToolContext, p: Point, mods: Mods, scope: &str, clicks: usize) -> Vec<Action> {
        // With Alt held, with Space held for the view tools, or with Cmd/Ctrl held (its presses went
        // to the selection tool Cmd lends), a double-click or triple-click paints nothing.
        if mods.alt || mods.space || mods.cmd {
            return vec![];
        }
        let point = json!([p.x, p.y]);
        let paint = self.paints(mods.shift);
        if matches!(paint, LpPaint::Strokes | LpPaint::Both)
            && let Some((g, Some(e))) = lp_part_at(cx.doc, p, cx.tol(4.0), true, &|id| lp_visible(cx, id))
        {
            let mut v = json!({ "group": g.0, "point": point, "tolerance": cx.tol(4.0), "scope": scope });
            let like = self.before(cx, e.id, clicks, lp_stroke_paint(e));
            if clicks == 2
                && let Some(group) = cx.doc.node(g)
            {
                // The other edges of this group either click painted, by its press or its drag, join
                // as the stroke paint they had before. A face either click filled is not an edge and
                // does not join. `livePaint.strokeEdge` takes at most 4096 of them.
                let group_edges: std::collections::HashSet<NodeId> = edges(group).iter().map(|x| x.id).collect();
                let mut seen = std::collections::HashSet::from([e.id]);
                let mut through: Vec<NodeId> = vec![];
                for id in self.presses.len().checked_sub(2).and_then(|i| self.presses.get(i..)).into_iter().flatten().flatten().flat_map(|q| {
                    q.part.filter(|_| q.before == like).into_iter().chain(q.dragged.iter().filter(|(_, before)| *before == like).map(|(id, _)| *id))
                }) {
                    if through.len() < 4096 && group_edges.contains(&id) && seen.insert(id) {
                        through.push(id);
                    }
                }
                self.note_reached(group, connected_edges_through(group, e.id, &like, &through), true);
                if !through.is_empty() {
                    v["through"] = json_ids(&through);
                }
            }
            if let Ok(like) = serde_json::to_value(like) {
                v["match"] = like;
            }
            return vec![Action::Exec("livePaint.strokeEdge".into(), v)];
        }
        if !matches!(paint, LpPaint::Fills | LpPaint::Both) {
            return vec![];
        }
        let Some((g, f)) = live_paint_at(cx.doc, p).and_then(|g| Some((g, visible_face_at(cx.doc.node(g)?, p)?))) else { return vec![] };
        let mut v = json!({ "group": g.0, "point": point, "scope": scope });
        if clicks == 2
            && let Some(group) = cx.doc.node(g)
        {
            self.note_reached(group, flood_faces(group, f.id), false);
        }
        if scope == "samePaint"
            && let Ok(like) = serde_json::to_value(self.before(cx, f.id, clicks, f.appearance.fill_paint()))
        {
            v["match"] = like;
        }
        vec![Action::Exec("livePaint.fill".into(), v)]
    }

    /// The highlight at `p` for a pointer with the modifiers `mods`: none with Alt, with Space held
    /// for the view tools or with Highlight off, else the face or edge a click there paints. With
    /// Highlight off the selection's map is still made, for the cursor ([`Self::paints_at`]).
    fn hover_at(&mut self, cx: &ToolContext, p: Point, mods: Mods) -> LpHover {
        if mods.alt || mods.space {
            return LpHover::None;
        }
        let target = self.target_at(cx, p, mods.shift);
        if self.highlight { target } else { LpHover::None }
    }

    /// What a click at `p` reaches, Shift held or not: whether it fills a face and paints an edge,
    /// with the face and the edge of a Live Paint group there ([`lp_face_and_edge`]). The document
    /// is walked once: the highlight and the cursor are asked for on every pointer update.
    fn parts_at<'a>(&self, cx: &ToolContext<'a>, p: Point, shift: bool) -> (bool, Option<&'a Arc<Node>>, Option<&'a Arc<Node>>) {
        let (wants_face, wants_edge) = match self.paints(shift) {
            LpPaint::Nothing => return (false, None, None),
            LpPaint::Fills => (true, false),
            LpPaint::Strokes => (false, true),
            LpPaint::Both => (true, true),
        };
        let (face, edge) = lp_face_and_edge(cx.doc, p, cx.tol(4.0), wants_edge, &|id| lp_visible(cx, id));
        (wants_face, face, edge)
    }

    /// The face or edge a click at `p` paints, Shift held or not. With Paint Strokes alone, a click
    /// paints no region of the selection, so none is highlighted.
    fn target_at(&mut self, cx: &ToolContext, p: Point, shift: bool) -> LpHover {
        let (wants_face, face, edge) = self.parts_at(cx, p, shift);
        let path = |n: &Arc<Node>| n.path_data().map(|pd| pd.to_bezpath());
        if let Some(e) = edge {
            return path(e).map(LpHover::Edge).unwrap_or_default();
        }
        if !wants_face {
            return LpHover::None;
        }
        if let Some(f) = face {
            return path(f).map(LpHover::Part).unwrap_or_default();
        }
        if !lp_sel_all_live(cx)
            && let Some(i) = self.cache.ready(cx, true).and_then(|m| m.region_at(p))
        {
            return LpHover::Region(i);
        }
        LpHover::None
    }

    /// The cursor for a click at `p`, Shift held or not: [`Cursor::LivePaintBucketEdge`] where it
    /// paints an edge of a Live Paint group, [`Cursor::LivePaintBucketFace`] where it fills a face
    /// of one or a region of the selected paths, else [`Cursor::LivePaintBucket`]. A region counts
    /// once the selection's map is made (the pointer moving over it makes it), as it is highlighted.
    fn paints_at(&self, cx: &ToolContext, p: Point, shift: bool) -> Cursor {
        let (wants_face, face, edge) = self.parts_at(cx, p, shift);
        let edge = edge.is_some();
        let region =
            || !lp_sel_all_live(cx) && self.cache.key.as_ref() == Some(&RegionCache::key_for(cx, true)) && self.cache.map.region_at(p).is_some();
        let face = wants_face && !edge && (face.is_some() || region());
        if edge {
            Cursor::LivePaintBucketEdge
        } else if face {
            Cursor::LivePaintBucketFace
        } else {
            Cursor::LivePaintBucket
        }
    }
}

impl Tool for LivePaintBucketTool {
    fn id(&self) -> &'static str {
        "livePaintBucket"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        // A second press finishes the preceding click, including callers without a release.
        if ev.kind == PointerKind::Down && self.end_fill() {
            let mut acts = vec![Action::Commit];
            acts.extend(self.pointer(cx, ev));
            return acts;
        }
        match ev.kind {
            PointerKind::Move => {
                self.at = Some((p, ev.mods));
                self.hover = self.hover_at(cx, p, ev.mods);
                vec![]
            }
            PointerKind::Down => {
                self.hover = LpHover::None;
                // With Space held for the view tools, a press paints nothing.
                if ev.mods.space {
                    self.note_press(None);
                    return vec![];
                }
                if ev.mods.alt {
                    self.note_press(None);
                    if ev.mods.shift {
                        return crate::xform::eyedropper::sample_color_at(cx, p).map_or_else(Vec::new, |color| {
                            vec![Action::Exec("paint.sampleColor".into(), json!({ "color": crate::params::color_json(&color), "ids": [] }))]
                        });
                    }
                    return crate::xform::eyedropper::sample_source(cx, p).map_or_else(Vec::new, |(source, _)| {
                        if cx.doc.node(source).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_))) {
                            return crate::xform::eyedropper::sample_color_at(cx, p).map_or_else(Vec::new, |color| {
                                vec![Action::Exec(
                                    "paint.sampleColor".into(),
                                    json!({ "color": crate::params::color_json(&color), "stroke": false, "ids": [], "clearStroke": true }),
                                )]
                            });
                        }
                        if crate::xform::eyedropper::has_sampleable_paint(cx, source) {
                            vec![Action::Exec("appearance.copyFrom".into(), json!({ "source": source.0, "defaultsOnly": true }))]
                        } else {
                            vec![]
                        }
                    });
                }
                let point = json!([p.x, p.y]);
                let paint = self.paints(ev.mods.shift);
                if matches!(paint, LpPaint::Strokes | LpPaint::Both)
                    && let Some((g, Some(e))) = lp_part_at(cx.doc, p, cx.tol(4.0), true, &|id| lp_visible(cx, id))
                {
                    self.note_part(e, true, p);
                    self.drag_from = cx.doc.node(g).map_or(LpDragFrom::None, |n| LpDragFrom::Edges(Arc::new(n.clone())));
                    return self.begin_fill("livePaint.strokeEdge", json!({ "group": g.0, "point": point, "tolerance": cx.tol(4.0) }), p);
                }
                if !matches!(paint, LpPaint::Fills | LpPaint::Both) {
                    self.note_press(None);
                    return vec![];
                }
                if let Some(g) = live_paint_at(cx.doc, p) {
                    match cx.doc.node(g).and_then(|n| visible_face_at(n, p)) {
                        Some(f) => self.note_part(f, false, p),
                        None => self.note_press(None),
                    }
                    self.drag_from = LpDragFrom::Faces(cx.doc.node(g).map(|n| faces(n).into_iter().cloned().collect()).unwrap_or_default());
                    return self.begin_fill("livePaint.fill", json!({ "group": g.0, "point": point }), p);
                }
                if self.cache.at(cx, p).is_some() {
                    let roots = sorted_roots(cx.doc, &cx.selection.objects);
                    let made = made_leaves(cx.doc, &roots);
                    let before = made_face_fill(&made, p);
                    self.note_press(Some(LpPress { part: None, at: p, before, also: vec![], dragged: vec![], dragged_made: None }));
                    self.drag_from = LpDragFrom::Made(made);
                    return self.begin_fill("livePaint.fill", json!({ "ids": json_ids(&roots), "point": point }), p);
                }
                self.note_press(None);
                vec![]
            }
            PointerKind::Drag => self.drag_fill(p),
            PointerKind::Up => {
                // The swatch preview shows again at the release point.
                self.at = Some((p, ev.mods));
                let mut acts = self.drag_fill(p);
                if self.end_fill() {
                    acts.push(Action::Commit);
                }
                acts
            }
            PointerKind::DoubleClick => self.multi_click(cx, p, ev.mods, "connected", 2),
            PointerKind::TripleClick => self.multi_click(cx, p, ev.mods, "samePaint", 3),
        }
    }

    fn history_moved(&mut self) {
        self.presses.clear();
    }

    /// Escape during a fill or stroke drag cancels it. Left and Right pick the color swatch before or after
    /// the active paint, while no drag is on.
    fn key(&mut self, cx: &ToolContext, key: ToolKey, mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.drag_from = LpDragFrom::None;
            return if self.fill_drag.take().is_some() { vec![Action::Cancel] } else { vec![] };
        }
        if !self.claims_key_with(cx, key, mods) {
            return vec![];
        }
        let slot = Self::picked_slot(cx);
        let Some((i, name)) = pick_swatch(cx.doc, Self::active_paint(cx), self.picked[slot], key == ToolKey::Right) else {
            return vec![];
        };
        self.picked[slot] = Some(i);
        // The active paint only (no `ids`): the selected art keeps its own.
        let cmd = if cx.fill_active { "paint.setFill" } else { "paint.setStroke" };
        vec![Action::Exec(cmd.into(), json!({ "swatch": name, "ids": [] }))]
    }

    fn busy(&self) -> bool {
        self.fill_drag.is_some()
    }

    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        match key {
            ToolKey::Escape => self.busy(),
            ToolKey::Left | ToolKey::Right => !self.busy() && !color_swatches(cx.doc).is_empty(),
            _ => false,
        }
    }

    /// The arrows pick swatches only without modifiers; Shift, Alt and Cmd/Ctrl arrows keep their
    /// shortcuts.
    fn claims_key_with(&self, cx: &ToolContext, key: ToolKey, mods: Mods) -> bool {
        self.claims_key(cx, key) && (key == ToolKey::Escape || plain(mods))
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.presses.clear();
        if self.end_fill() { vec![Action::Commit] } else { vec![] }
    }

    /// The pointer hovers nothing for the tool: its highlight and its swatch preview go.
    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = LpHover::None;
        self.at = None;
    }

    /// A press that did not reach the tool counts as a press that paints no face or edge, for the clicks
    /// a double-click or triple-click compares.
    fn press_elsewhere(&mut self) {
        self.note_press(None);
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = lp_overlays(&self.hover, &self.cache, self.color, self.width as f32, (self.width * LP_BUCKET_EDGE_FRACTION) as f32);
        // No chips during a fill or stroke drag, or while Alt makes the tool sample with the eyedropper.
        if self.preview
            && !self.busy()
            && let Some((at, mods)) = self.at
            && !mods.alt
        {
            out.extend(swatch_chips(cx, at, Self::active_paint(cx), self.picked[Self::picked_slot(cx)], true));
        }
        out
    }

    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        if m.alt { Cursor::Eyedropper } else { self.paints_at(cx, p, m.shift) }
    }

    fn options(&self) -> Value {
        let [r, g, b] = self.color;
        json!({
            "paintFills": self.fills,
            "paintStrokes": self.strokes,
            "cursorSwatchPreview": self.preview,
            "highlight": self.highlight,
            "color": format!("#{r:02x}{g:02x}{b:02x}"),
            "width": self.width,
        })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "paintFills" => self.fills = v.as_bool().unwrap_or(self.fills),
            "paintStrokes" => self.strokes = v.as_bool().unwrap_or(self.strokes),
            "cursorSwatchPreview" => self.preview = v.as_bool().unwrap_or(self.preview),
            "highlight" => self.highlight = v.as_bool().unwrap_or(self.highlight),
            "color" => self.color = lp_color(v).unwrap_or(self.color),
            "width" => {
                let (min, max) = LP_BUCKET_WIDTH_RANGE;
                self.width = v.as_f64().filter(|w| w.is_finite()).map_or(self.width, |w| w.clamp(min, max));
            }
            _ => return,
        }
        self.hover = LpHover::None;
    }

    fn waiting(&self) -> bool {
        self.cache.waiting()
    }

    fn poll(&mut self, cx: &ToolContext, wait: bool) -> Vec<Action> {
        if !self.cache.waiting() {
            return vec![];
        }
        // As the Shape Builder's: the map that arrives may be for an earlier revision. Over a part
        // of a Live Paint group the hover takes no map.
        loop {
            self.cache.settle(wait);
            if let Some((p, mods)) = self.at {
                self.hover = self.hover_at(cx, p, mods);
            }
            if !wait || !self.cache.waiting() {
                break;
            }
        }
        vec![]
    }
}

// ---------- Live Paint Selection ----------

/// The Live Paint Selection tool's highlight width (screen pixels) by default.
pub const LP_SELECT_WIDTH: f64 = 4.0;

/// The least and greatest highlight width (screen pixels) the Live Paint Selection tool takes.
pub const LP_SELECT_WIDTH_RANGE: (f64, f64) = (1.0, 20.0);

/// Live Paint Selection: click selects a face (Shift adds); Alt-click selects an edge. A drag draws
/// a marquee, and on release the faces and edges it touches ([`lp_parts_in`]) become the selection;
/// with Shift held at the press, each of them leaves the selection if it is selected and joins it
/// otherwise. A double-click on a face selects it and the faces of its group that are not parted
/// from it by a stroked edge; on an edge, it selects the edge and the edges joined to it end to end
/// that have its stroke paint ([`LivePaintSelectionTool::reach_at`]). A triple-click on a face
/// selects every face of its group with that face's fill; on an edge, every edge of its group with
/// that edge's stroke paint ([`LivePaintSelectionTool::same_paint_at`]). With Shift held, each part a
/// double-click reaches leaves the selection if it is selected and joins it otherwise, and after a
/// Shift-triple-click each part with the clicked part's paint is toggled from the selection there
/// was before the three clicks. Clicks of the three made without Shift select as they do alone, so
/// when Shift is first held at a later click, the parts are toggled from the selection there was
/// before that click.
/// A click and a marquee change the selection on release, so Escape between the press
/// and the release (which removes the marquee) leaves the selection as it was. The tool does not
/// highlight or select the faces and edges of a Live Paint group that is locked, on a locked layer
/// or on a locked artboard, or in isolation mode outside the isolated art. Its Tool Options
/// (double-click the tool) are `selectFills` and `selectStrokes` (both on), the highlight `color`
/// (`#rrggbb`, red by default) and its `width` ([`LP_SELECT_WIDTH`] pixels). With
/// Select Strokes off, Alt-click selects the face under the pointer as a click does and a marquee
/// takes no edges; with Select Fills off, a click selects the edge nearest the pointer as Alt-click
/// does and a marquee takes no faces; with both off, a click or a marquee selects no face or edge.
/// The pointer shows what a click at that point selects ([`LivePaintSelectionTool::pick_at`]): a
/// face, an edge, or nothing. From the press to the release it keeps the look it had at the press.
pub struct LivePaintSelectionTool {
    pub(super) cache: RegionCache,
    pub(super) hover: LpHover,
    fills: bool,
    strokes: bool,
    color: [u8; 3],
    width: f64,
    /// The drag in progress, from the press to the release.
    drag: Option<LpDrag>,
    /// The part each of the last two clicks toggled with Shift held, the latest last (None for a
    /// click that toggled nothing). A click or marquee without Shift clears both.
    recent: [Option<NodeId>; 2],
    /// The parts the last double-click, the two clicks before it and the Shift-clicks after it
    /// toggled, each listed once when toggled an odd number of times. A Shift-triple-click toggles
    /// them back as it toggles the parts with the clicked part's paint, so the selection ends as it
    /// was before the clicks with those parts toggled.
    toggled: Vec<NodeId>,
}

/// `a` (which lists each id once) with each id of `b` toggled in turn: an id `b` lists an odd number
/// of times is removed when `a` holds it and added at the end otherwise, in `b`'s order.
fn toggle_ids(a: &mut Vec<NodeId>, b: &[NodeId]) {
    use std::collections::HashSet;
    let mut odd = HashSet::new();
    for id in b {
        if !odd.insert(*id) {
            odd.remove(id);
        }
    }
    let held: HashSet<NodeId> = a.iter().copied().collect();
    a.retain(|x| !odd.contains(x));
    for id in b {
        if !held.contains(id) && odd.remove(id) {
            a.push(*id);
        }
    }
}

/// A press and drag of the Live Paint Selection tool.
#[derive(Clone, Copy, Debug)]
struct LpDrag {
    /// Where the press was.
    start: Point,
    /// Where the pointer is.
    cur: Point,
    /// Shift was held at the press.
    shift: bool,
    /// The face or edge under the press, which a click selects (or with Shift toggles).
    hit: Option<NodeId>,
    /// The cursor at the press, which the drag keeps.
    cursor: Cursor,
}

impl LpDrag {
    /// The marquee from the press to the pointer, None while the drag is shorter than
    /// [`CLICK_TOL_PX`] screen pixels both across and down (a click).
    fn marquee(&self, cx: &ToolContext) -> Option<Rect> {
        let r = Rect::from_points(self.start, self.cur);
        let tol = cx.tol(CLICK_TOL_PX);
        (r.width() >= tol || r.height() >= tol).then_some(r)
    }
}

impl Default for LivePaintSelectionTool {
    fn default() -> Self {
        Self {
            cache: RegionCache::live_paint(),
            hover: LpHover::None,
            fills: true,
            strokes: true,
            color: HIGHLIGHT_RED,
            width: LP_SELECT_WIDTH,
            drag: None,
            recent: [None; 2],
            toggled: vec![],
        }
    }
}

impl LivePaintSelectionTool {
    /// What a click picks, Alt held or not: Some(true) an edge, Some(false) a face, None nothing.
    fn picks_edge(&self, alt: bool) -> Option<bool> {
        match (self.fills, self.strokes) {
            (true, true) => Some(alt),
            (true, false) => Some(false),
            (false, true) => Some(true),
            (false, false) => None,
        }
    }

    /// The face or edge a click at `p` selects, Alt held or not, with its cursor: the visible part of
    /// the kind [`LivePaintSelectionTool::picks_edge`] asks for, of a Live Paint group the tool takes
    /// ([`lp_part_at`], [`lp_selectable`]), and [`Cursor::LivePaintSelectFace`] or
    /// [`Cursor::LivePaintSelectEdge`]; None and [`Cursor::LivePaintSelectOutside`] where a click
    /// selects no face or edge.
    fn pick_at(&self, cx: &ToolContext, p: Point, alt: bool) -> (Option<NodeId>, Cursor) {
        let Some(edge) = self.picks_edge(alt) else { return (None, Cursor::LivePaintSelectOutside) };
        let hit = lp_part_at(cx.doc, p, cx.tol(4.0), edge, &|id| lp_selectable(cx, id)).and_then(|(_, part)| part).map(|n| n.id);
        let cursor = match (hit, edge) {
            (None, _) => Cursor::LivePaintSelectOutside,
            (Some(_), true) => Cursor::LivePaintSelectEdge,
            (Some(_), false) => Cursor::LivePaintSelectFace,
        };
        (hit, cursor)
    }

    /// What a double-click at `p` selects, Alt held or not: on the face a click there picks
    /// ([`LivePaintSelectionTool::picks_edge`]), that face and the faces of its group a flood from it
    /// reaches across edges that are hidden or draw no stroke ([`flood_faces`]); on the edge a click
    /// picks, that edge and the edges of its group joined to it end to end through edges that draw
    /// with the stroke paint it draws with ([`connected_edges`], [`lp_stroke_paint`]). Empty where a click selects no face or edge.
    fn reach_at(&self, cx: &ToolContext, p: Point, alt: bool) -> Vec<NodeId> {
        let Some(edge) = self.picks_edge(alt) else { return vec![] };
        let Some((gid, Some(part))) = lp_part_at(cx.doc, p, cx.tol(4.0), edge, &|id| lp_selectable(cx, id)) else { return vec![] };
        let Some(group) = cx.doc.node(gid) else { return vec![] };
        if edge { connected_edges(group, part.id, &lp_stroke_paint(part)) } else { flood_faces(group, part.id) }
    }

    /// What a triple-click at `p` selects, Alt held or not: on the face a click there picks
    /// ([`LivePaintSelectionTool::picks_edge`]), that face and every other face of its group that is
    /// not hidden and has its fill; on the edge a click picks, that edge and every other edge of its
    /// group that is not hidden and draws with the stroke paint it draws with ([`lp_stroke_paint`]).
    /// The part under the pointer comes first. Empty where a click selects no face or edge.
    fn same_paint_at(&self, cx: &ToolContext, p: Point, alt: bool) -> Vec<NodeId> {
        let Some(edge) = self.picks_edge(alt) else { return vec![] };
        let Some((gid, Some(part))) = lp_part_at(cx.doc, p, cx.tol(4.0), edge, &|id| lp_selectable(cx, id)) else { return vec![] };
        let Some(group) = cx.doc.node(gid) else { return vec![] };
        let others: Vec<NodeId> = if edge {
            let like = lp_stroke_paint(part);
            edges(group).into_iter().filter(|e| e.visible && e.id != part.id && lp_stroke_paint(e) == like).map(|e| e.id).collect()
        } else {
            let like = part.appearance.fill_paint();
            faces(group).into_iter().filter(|f| f.visible && f.id != part.id && f.appearance.fill_paint() == like).map(|f| f.id).collect()
        };
        std::iter::once(part.id).chain(others).collect()
    }
}

/// The faces (with `faces`) and edges (with `edges`) that marquee `r` touches, among the faces and
/// edges that are not hidden of the Live Paint groups the tool takes ([`lp_selectable`]), in paint
/// order. A face is touched when its outline has an anchor inside `r` or crosses `r`'s sides, or
/// when it holds `r`'s center. An edge is touched when it has an anchor inside `r` or crosses
/// `r`'s sides. A face or edge partly inside `r` is touched.
fn lp_parts_in(cx: &ToolContext, r: Rect, faces: bool, edges: bool) -> Vec<NodeId> {
    let mut out = vec![];
    cx.doc.walk(|n| {
        if !is_live_paint(n) || !lp_selectable(cx, n.id) {
            return;
        }
        for c in n.children().into_iter().flatten().filter(|c| c.visible) {
            let Some(pd) = c.path_data() else { continue };
            let face = faces && c.name.as_deref() == Some(FACE_NAME);
            let edge = edges && c.name.as_deref() == Some(EDGE_NAME);
            let touched = vectorcraft_geom::hit::intersects_rect(pd, r) || (face && path_contains(pd, FillRule::NonZero, r.center()));
            if (face || edge) && touched {
                out.push(c.id);
            }
        }
    });
    out
}

/// A highlight color option: `#rrggbb` or a layer color's name (any case).
pub(super) fn lp_color(v: &Value) -> Option<[u8; 3]> {
    let s = v.as_str()?.trim();
    if let Some((_, c)) = vectorcraft_doc::LAYER_COLORS.iter().find(|(name, _)| name.eq_ignore_ascii_case(s)) {
        return Some(*c);
    }
    let [r, g, b, _] = vectorcraft_color::Color::from_hex(s)?.to_rgba8(1.0);
    Some([r, g, b])
}

/// Whether `v` is a highlight color [`lp_color`] reads without an alpha part: `#rrggbbaa` (with
/// or without `#`) is not one.
pub(super) fn lp_rgb_color(v: &Value) -> bool {
    let Some(s) = v.as_str() else { return false };
    let digits = s.trim().trim_start_matches('#');
    let alpha = digits.len() == 8 && digits.chars().all(|c| c.is_ascii_hexdigit());
    !alpha && lp_color(v).is_some()
}

/// Why `tool.setOption` cannot give the Live Paint tool `tool` (`livePaintBucket` or
/// `livePaintSelection`) the options `values`, checked before any of them is set: an option the
/// tool does not have, a switch that is not true or false, a `color` that is not an RGB color
/// ([`lp_rgb_color`]) and a `width` that is not a finite number are errors. A finite width out of
/// [`LP_BUCKET_WIDTH_RANGE`] or [`LP_SELECT_WIDTH_RANGE`] is not an error: the tool clamps it.
/// Ok for every other tool. Saved settings go to the tools' `set_option` instead, which skips a
/// value it cannot read.
pub fn check_live_paint_options(tool: &str, values: &serde_json::Map<String, Value>) -> Result<(), String> {
    let switches: &[&str] = match tool {
        "livePaintBucket" => &["paintFills", "paintStrokes", "cursorSwatchPreview", "highlight"],
        "livePaintSelection" => &["selectFills", "selectStrokes"],
        _ => return Ok(()),
    };
    for (key, v) in values {
        let error = match key.as_str() {
            "color" => (!lp_rgb_color(v)).then(|| format!("`{key}` must be a #rrggbb color or a layer color name")),
            "width" => (!v.as_f64().is_some_and(f64::is_finite)).then(|| format!("`{key}` must be a number")),
            k if switches.contains(&k) => (!v.is_boolean()).then(|| format!("`{key}` must be true or false")),
            _ => Some(format!("unknown Live Paint option `{key}`")),
        };
        if let Some(e) = error {
            return Err(e);
        }
    }
    Ok(())
}

impl Tool for LivePaintSelectionTool {
    fn id(&self) -> &'static str {
        "livePaintSelection"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        let edge = self.picks_edge(ev.mods.alt);
        match ev.kind {
            PointerKind::Move => {
                // Only the parts of Live Paint groups highlight, so no map of the selection is built.
                self.hover = match edge.map(|e| lp_hover(cx, None, p, e, &|id| lp_selectable(cx, id))) {
                    Some(h @ (LpHover::Part(_) | LpHover::Edge(_))) => h,
                    _ => LpHover::None,
                };
                vec![]
            }
            PointerKind::Down => {
                let (hit, cursor) = self.pick_at(cx, p, ev.mods.alt);
                self.drag = Some(LpDrag { start: p, cur: p, shift: ev.mods.shift, hit, cursor });
                vec![]
            }
            PointerKind::Drag => {
                if let Some(d) = &mut self.drag {
                    d.cur = p;
                    self.hover = LpHover::None;
                }
                vec![]
            }
            PointerKind::Up => {
                let Some(d) = self.drag.take() else { return vec![] };
                let d = LpDrag { cur: p, ..d };
                let Some(r) = d.marquee(cx) else {
                    // A click selects what the press picked.
                    // A click without Shift replaces the selection, so what earlier clicks toggled
                    // no longer counts.
                    if d.shift {
                        self.recent = [self.recent[1], d.hit];
                        if let Some(id) = d.hit {
                            toggle_ids(&mut self.toggled, &[id]);
                        }
                    } else {
                        self.recent = [None; 2];
                        self.toggled.clear();
                    }
                    return match (d.hit, d.shift) {
                        (Some(id), true) => vec![Action::Exec("select.toggle".into(), json!({ "id": id.0 }))],
                        (Some(id), false) => vec![Action::Exec("select.set".into(), json!({ "ids": [id.0] }))],
                        (None, true) => vec![],
                        (None, false) => vec![Action::Exec("select.none".into(), json!({}))],
                    };
                };
                self.recent = [None; 2];
                self.toggled.clear();
                let ids = lp_parts_in(cx, r, self.fills, self.strokes);
                match (ids.is_empty(), d.shift) {
                    (true, true) => vec![],
                    (true, false) => vec![Action::Exec("select.none".into(), json!({}))],
                    (false, true) => vec![Action::Exec("select.toggle".into(), json!({ "ids": json_ids(&ids) }))],
                    (false, false) => vec![Action::Exec("select.set".into(), json!({ "ids": json_ids(&ids) }))],
                }
            }
            PointerKind::DoubleClick => {
                // The two clicks before it already selected the part under the pointer (or, with
                // Shift, toggled it twice); the double-click selects the part with what it reaches.
                let ids = self.reach_at(cx, p, ev.mods.alt);
                self.toggled.clear();
                if ev.mods.shift {
                    let clicks: Vec<NodeId> = self.recent.iter().flatten().copied().collect();
                    toggle_ids(&mut self.toggled, &clicks);
                    toggle_ids(&mut self.toggled, &ids);
                }
                match (ids.is_empty(), ev.mods.shift) {
                    (true, _) => vec![],
                    (false, true) => vec![Action::Exec("select.toggle".into(), json!({ "ids": json_ids(&ids) }))],
                    (false, false) => vec![Action::Exec("select.set".into(), json!({ "ids": json_ids(&ids) }))],
                }
            }
            PointerKind::TripleClick => {
                // The clicks before it already selected the part under the pointer; the
                // triple-click selects the part with every part of its group that has its paint.
                // With Shift held, the double-click and the third click toggled parts too, and the
                // triple-click toggles those back.
                let ids = self.same_paint_at(cx, p, ev.mods.alt);
                let toggled = std::mem::take(&mut self.toggled);
                self.recent = [None; 2];
                if ids.is_empty() {
                    return vec![];
                }
                if !ev.mods.shift {
                    return vec![Action::Exec("select.set".into(), json!({ "ids": json_ids(&ids) }))];
                }
                let mut flip = ids;
                toggle_ids(&mut flip, &toggled);
                if flip.is_empty() { vec![] } else { vec![Action::Exec("select.toggle".into(), json!({ "ids": json_ids(&flip) }))] }
            }
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        // Escape during a press or drag drops it; the release then selects nothing.
        if key == ToolKey::Escape {
            self.drag = None;
        }
        vec![]
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.drag = None;
        self.recent = [None; 2];
        self.toggled.clear();
        vec![]
    }

    /// The pointer hovers nothing for the tool: its highlight goes.
    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = LpHover::None;
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if let Some(r) = self.drag.and_then(|d| d.marquee(cx)) {
            return vec![Overlay::Marquee(r)];
        }
        lp_overlays(&self.hover, &self.cache, self.color, self.width as f32, self.width as f32)
    }

    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        match &self.drag {
            Some(d) => d.cursor,
            None => self.pick_at(cx, p, m.alt).1,
        }
    }

    fn options(&self) -> Value {
        let [r, g, b] = self.color;
        json!({
            "selectFills": self.fills,
            "selectStrokes": self.strokes,
            "color": format!("#{r:02x}{g:02x}{b:02x}"),
            "width": self.width,
        })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "selectFills" => self.fills = v.as_bool().unwrap_or(self.fills),
            "selectStrokes" => self.strokes = v.as_bool().unwrap_or(self.strokes),
            "color" => self.color = lp_color(v).unwrap_or(self.color),
            "width" => {
                let (min, max) = LP_SELECT_WIDTH_RANGE;
                self.width = v.as_f64().filter(|w| w.is_finite()).map_or(self.width, |w| w.clamp(min, max));
            }
            _ => return,
        }
        self.hover = LpHover::None;
    }
}
