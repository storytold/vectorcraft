use super::cache::RegionCache;
use super::lookup::{is_live_paint, live_paint_at, live_paint_at_where, visible_edge_near, visible_face_at};
use super::map::{HIGHLIGHT_RED, sorted_roots};
use super::{
    Action, Arc, BezPath, CLICK_TOL_PX, Cursor, Document, EDGE_NAME, FACE_NAME, FillRule, Mods, Node, NodeId, Overlay, Point, PointerEvent,
    PointerKind, Rect, Tool, ToolContext, ToolKey, Value, json, json_ids, path_contains,
};

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
/// click paints nothing. With Highlight off, the face or edge under the pointer is not
/// highlighted. The cursor is a filled bucket where a click fills a face, a brush where it paints
/// an edge and an empty bucket where it paints nothing; with Alt it is the Eyedropper.
pub struct LivePaintBucketTool {
    pub(super) cache: RegionCache,
    pub(super) hover: LpHover,
    /// Where the pointer last moved, and whether Shift and Alt were held.
    at: Option<(Point, bool, bool)>,
    /// The fill command and polyline previewed from the press's snapshot.
    fill_drag: Option<(Value, Vec<Vec<Point>>)>,
    fills: bool,
    strokes: bool,
    highlight: bool,
    color: [u8; 3],
    width: f64,
}

/// Maximum points in one Live Paint fill path chunk.
pub const LP_FILL_CHUNK_POINTS: usize = 4096;

impl Default for LivePaintBucketTool {
    fn default() -> Self {
        Self {
            cache: RegionCache::live_paint(),
            hover: LpHover::None,
            at: None,
            fill_drag: None,
            fills: true,
            strokes: false,
            highlight: true,
            color: HIGHLIGHT_RED,
            width: LP_BUCKET_WIDTH,
        }
    }
}

impl LivePaintBucketTool {
    fn begin_fill(&mut self, params: Value, p: Point) -> Vec<Action> {
        self.fill_drag = Some((params.clone(), vec![vec![p]]));
        vec![Action::Begin("Live Paint Bucket".into()), Action::Preview("livePaint.fill".into(), params)]
    }

    fn drag_fill(&mut self, p: Point) -> Vec<Action> {
        let Some((params, chunks)) = &mut self.fill_drag else { return vec![] };
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
        vec![Action::Preview("livePaint.fill".into(), params)]
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

    /// The highlight at `p` for a pointer with Shift and Alt held as given: none with Alt or with
    /// Highlight off, else the face or edge a click there paints. With Highlight off the selection's
    /// map is still made, for the cursor ([`Self::paints_at`]).
    fn hover_at(&mut self, cx: &ToolContext, p: Point, shift: bool, alt: bool) -> LpHover {
        if alt {
            return LpHover::None;
        }
        let target = self.target_at(cx, p, shift);
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
        if ev.kind == PointerKind::Down && self.fill_drag.take().is_some() {
            let mut acts = vec![Action::Commit];
            acts.extend(self.pointer(cx, ev));
            return acts;
        }
        match ev.kind {
            PointerKind::Move => {
                self.at = Some((p, ev.mods.shift, ev.mods.alt));
                self.hover = self.hover_at(cx, p, ev.mods.shift, ev.mods.alt);
                vec![]
            }
            PointerKind::Down => {
                self.hover = LpHover::None;
                if ev.mods.alt {
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
                    && let Some((g, Some(_))) = lp_part_at(cx.doc, p, cx.tol(4.0), true, &|id| lp_visible(cx, id))
                {
                    return vec![Action::Exec("livePaint.strokeEdge".into(), json!({ "group": g.0, "point": point, "tolerance": cx.tol(4.0) }))];
                }
                if !matches!(paint, LpPaint::Fills | LpPaint::Both) {
                    return vec![];
                }
                if let Some(g) = live_paint_at(cx.doc, p) {
                    return self.begin_fill(json!({ "group": g.0, "point": point }), p);
                }
                if self.cache.at(cx, p).is_some() {
                    let roots = sorted_roots(cx.doc, &cx.selection.objects);
                    return self.begin_fill(json!({ "ids": json_ids(&roots), "point": point }), p);
                }
                vec![]
            }
            PointerKind::Drag => self.drag_fill(p),
            PointerKind::Up => {
                let mut acts = self.drag_fill(p);
                if self.fill_drag.take().is_some() {
                    acts.push(Action::Commit);
                }
                acts
            }
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.fill_drag.take().is_some() { vec![Action::Cancel] } else { vec![] }
    }

    fn busy(&self) -> bool {
        self.fill_drag.is_some()
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        key == ToolKey::Escape && self.busy()
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        if self.fill_drag.take().is_some() { vec![Action::Commit] } else { vec![] }
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        lp_overlays(&self.hover, &self.cache, self.color, self.width as f32, (self.width * LP_BUCKET_EDGE_FRACTION) as f32)
    }

    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        if m.alt { Cursor::Eyedropper } else { self.paints_at(cx, p, m.shift) }
    }

    fn options(&self) -> Value {
        let [r, g, b] = self.color;
        json!({
            "paintFills": self.fills,
            "paintStrokes": self.strokes,
            "highlight": self.highlight,
            "color": format!("#{r:02x}{g:02x}{b:02x}"),
            "width": self.width,
        })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "paintFills" => self.fills = v.as_bool().unwrap_or(self.fills),
            "paintStrokes" => self.strokes = v.as_bool().unwrap_or(self.strokes),
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
            if let Some((p, shift, alt)) = self.at {
                self.hover = self.hover_at(cx, p, shift, alt);
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
/// otherwise. A click and a marquee change the selection on release, so Escape between the press
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
                    return match (d.hit, d.shift) {
                        (Some(id), true) => vec![Action::Exec("select.toggle".into(), json!({ "id": id.0 }))],
                        (Some(id), false) => vec![Action::Exec("select.set".into(), json!({ "ids": [id.0] }))],
                        (None, true) => vec![],
                        (None, false) => vec![Action::Exec("select.none".into(), json!({}))],
                    };
                };
                let ids = lp_parts_in(cx, r, self.fills, self.strokes);
                match (ids.is_empty(), d.shift) {
                    (true, true) => vec![],
                    (true, false) => vec![Action::Exec("select.none".into(), json!({}))],
                    (false, true) => vec![Action::Exec("select.toggle".into(), json!({ "ids": json_ids(&ids) }))],
                    (false, false) => vec![Action::Exec("select.set".into(), json!({ "ids": json_ids(&ids) }))],
                }
            }
            _ => vec![],
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
        vec![]
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
