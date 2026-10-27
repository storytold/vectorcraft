use super::cache::RegionCache;
use super::lookup::{is_live_paint, live_paint_at, visible_edge_near, visible_face_at};
use super::map::{HIGHLIGHT_RED, sorted_roots};
use super::*;

// ---------- Live Paint Bucket ----------

/// What the Live Paint tools highlight under the cursor.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum LpHover {
    #[default]
    None,
    /// A face (or edge) of an existing Live Paint group.
    Part(BezPath),
    /// A region of the (not yet live) selection.
    Region(usize),
}

/// The Live Paint group and part the Live Paint tools hover and click at `p`. Inside a visible face
/// of a visible Live Paint group ([`live_paint_at`]) it is that group with its visible face under
/// `p`, or with `edge` its visible edge nearest `p` within `tol` (None when there is none). Outside
/// every visible face, with `edge`, it is the visible edge nearest `p` within `tol` among the
/// visible Live Paint groups (the front-most group's on a tie). None when no group is found.
fn lp_part_at(doc: &Document, p: Point, tol: f64, edge: bool) -> Option<(NodeId, Option<&Arc<Node>>)> {
    if let Some(g) = live_paint_at(doc, p).and_then(|id| doc.node(id)) {
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
            && doc.is_visible(n.id)
        {
            best = Some((d, n.id, e));
        }
    });
    best.map(|(_, g, e)| (g, Some(e)))
}

/// What the Live Paint tools highlight at `p`: the part of a Live Paint group ([`lp_part_at`]), else
/// with `cache` the region of the selection under `p`. A large selection's regions are found on
/// another thread ([`RegionCache::ready`]), and until they are, no region is highlighted.
fn lp_hover(cx: &ToolContext, cache: Option<&mut RegionCache>, p: Point, edge: bool) -> LpHover {
    if let Some((_, part)) = lp_part_at(cx.doc, p, cx.tol(4.0), edge) {
        return part.and_then(|n| n.path_data()).map(|pd| LpHover::Part(pd.to_bezpath())).unwrap_or_default();
    }
    let sel_is_live = cx.selection.objects.iter().any(|id| cx.doc.node(*id).is_some_and(is_live_paint));
    if !sel_is_live && let Some(i) = cache.and_then(|c| c.ready(cx, true)).and_then(|m| m.region_at(p)) {
        return LpHover::Region(i);
    }
    LpHover::None
}

/// The highlight of `h`, `width` screen pixels wide in `color`.
fn lp_overlays(h: &LpHover, cache: &RegionCache, color: [u8; 3], width: f32) -> Vec<Overlay> {
    match h {
        LpHover::None => vec![],
        LpHover::Part(bp) => vec![Overlay::Path { path: bp.clone(), color, width, dashed: false }],
        LpHover::Region(i) => {
            cache.map.regions.get(*i).map(|(_, bp, _)| vec![Overlay::Path { path: bp.clone(), color, width, dashed: false }]).unwrap_or_default()
        }
    }
}

/// Live Paint Bucket: click fills the face under the cursor with the current fill; Shift-click
/// paints the nearest edge with the current stroke. Clicking selected ordinary paths makes them a
/// Live Paint group first.
pub struct LivePaintBucketTool {
    pub(super) cache: RegionCache,
    pub(super) hover: LpHover,
    /// Where the pointer last moved, and whether Shift was held.
    at: Option<(Point, bool)>,
}

impl Default for LivePaintBucketTool {
    fn default() -> Self {
        Self { cache: RegionCache::live_paint(), hover: LpHover::None, at: None }
    }
}

impl Tool for LivePaintBucketTool {
    fn id(&self) -> &'static str {
        "livePaintBucket"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                self.at = Some((p, ev.mods.shift));
                self.hover = lp_hover(cx, Some(&mut self.cache), p, ev.mods.shift);
                vec![]
            }
            PointerKind::Down => {
                self.hover = LpHover::None;
                let point = json!([p.x, p.y]);
                if ev.mods.shift {
                    return match lp_part_at(cx.doc, p, cx.tol(4.0), true) {
                        Some((g, Some(_))) => {
                            vec![Action::Exec("livePaint.strokeEdge".into(), json!({ "group": g.0, "point": point, "tolerance": cx.tol(4.0) }))]
                        }
                        _ => vec![],
                    };
                }
                if let Some(g) = live_paint_at(cx.doc, p) {
                    return vec![Action::Exec("livePaint.fill".into(), json!({ "group": g.0, "point": point }))];
                }
                if self.cache.at(cx, p).is_some() {
                    let roots = sorted_roots(cx.doc, &cx.selection.objects);
                    return vec![Action::Exec("livePaint.fill".into(), json!({ "ids": json_ids(&roots), "point": point }))];
                }
                vec![]
            }
            _ => vec![],
        }
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        lp_overlays(&self.hover, &self.cache, HIGHLIGHT_RED, 4.0)
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::LivePaintBucket
    }

    fn waiting(&self) -> bool {
        self.cache.waiting()
    }

    fn poll(&mut self, cx: &ToolContext, wait: bool) {
        if !self.cache.waiting() {
            return;
        }
        // As the Shape Builder's: the map that arrives may be for an earlier revision. Over a part
        // of a Live Paint group the hover takes no map.
        loop {
            self.cache.settle(wait);
            if let Some((p, shift)) = self.at {
                self.hover = lp_hover(cx, Some(&mut self.cache), p, shift);
            }
            if !wait || !self.cache.waiting() {
                break;
            }
        }
    }
}

// ---------- Live Paint Selection ----------

/// The Live Paint Selection tool's highlight width (screen pixels) by default.
pub const LP_SELECT_WIDTH: f64 = 4.0;

/// The least and greatest highlight width (screen pixels) the Live Paint Selection tool takes.
pub const LP_SELECT_WIDTH_RANGE: (f64, f64) = (1.0, 20.0);

/// Live Paint Selection: click selects a face (Shift adds); Alt-click selects an edge. Its Tool
/// Options (double-click the tool) are `selectFills` and `selectStrokes` (both on), the highlight
/// `color` (`#rrggbb`, red by default) and its `width` ([`LP_SELECT_WIDTH`] pixels). With Select
/// Strokes off, Alt-click selects the face under the pointer as a click does; with Select Fills
/// off, a click selects the edge nearest the pointer as Alt-click does; with both off, a click
/// selects no face or edge.
pub struct LivePaintSelectionTool {
    pub(super) cache: RegionCache,
    pub(super) hover: LpHover,
    fills: bool,
    strokes: bool,
    color: [u8; 3],
    width: f64,
}

impl Default for LivePaintSelectionTool {
    fn default() -> Self {
        Self { cache: RegionCache::live_paint(), hover: LpHover::None, fills: true, strokes: true, color: HIGHLIGHT_RED, width: LP_SELECT_WIDTH }
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
                self.hover = match edge.map(|e| lp_hover(cx, None, p, e)) {
                    Some(h @ LpHover::Part(_)) => h,
                    _ => LpHover::None,
                };
                vec![]
            }
            PointerKind::Down => {
                let hit = edge.and_then(|e| lp_part_at(cx.doc, p, cx.tol(4.0), e)).and_then(|(_, part)| part).map(|n| n.id);
                match (hit, ev.mods.shift) {
                    (Some(id), true) => vec![Action::Exec("select.toggle".into(), json!({ "id": id.0 }))],
                    (Some(id), false) => vec![Action::Exec("select.set".into(), json!({ "ids": [id.0] }))],
                    (None, true) => vec![],
                    (None, false) => vec![Action::Exec("select.none".into(), json!({}))],
                }
            }
            _ => vec![],
        }
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        lp_overlays(&self.hover, &self.cache, self.color, self.width as f32)
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Arrow
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
