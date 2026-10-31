//! Corner Tool (Path Precision Tools): editable corners on any path, selected or not, through the
//! app's Live Corners (`addon.pathtools.cornerSet` / `cornerRemove`). Over an anchor that can take
//! a corner, the anchor shows; a click cuts one with the tool's radius and kind, a drag sizes it
//! (Shift: whole values of the ruler's unit). A cut corner shows a marker at the centre of its cut:
//! a click selects the corner and loads its radius and kind into the tool, a drag resizes it (to 0
//! it goes), a double-click removes it. A corner can't grow past what fits. Alt-marquee cuts
//! corners on every anchor inside; a plain marquee selects them, and a click away deselects.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, LiveCorners, NodeId, NodeKind};
use vectorcraft_geom::corners::Corner;
use vectorcraft_geom::shapes::CornerKind;
use vectorcraft_geom::{Affine, Point, Rect};

use crate::builder::HIGHLIGHT_RED;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "cornerTool";
/// The radius a click cuts by default (pt).
pub const RADIUS: f64 = 10.0;
/// The largest radius asked for (pt).
pub const MAX_RADIUS: f64 = 1e6;
/// The undo step of a drag.
const LABEL: &str = "Corner Radius";
const SET: &str = "addon.pathtools.cornerSet";
const REMOVE: &str = "addon.pathtools.cornerRemove";
/// How far (px) the pointer travels before a press is a drag.
const DRAG_PX: f64 = 3.0;
/// Most markers drawn at once.
const MAX_MARKERS: usize = 2000;
const MARKER_PX: f32 = 7.0;
const BLUE: [u8; 3] = [0x4f, 0x9d, 0xff];

/// A corner the tool can act on.
#[derive(Clone, Debug, PartialEq)]
struct Spot {
    id: NodeId,
    corner: Corner,
    /// Maps the corner's space into the document.
    xf: Affine,
    /// The radius it is drawn with: 0 for a sharp corner.
    drawn: f64,
    kind: CornerKind,
}

impl Spot {
    fn of(lc: &LiveCorners, id: NodeId, corner: &Corner) -> Self {
        Self { id, corner: *corner, xf: lc.xf, drawn: lc.drawn_radius(corner.index), kind: lc.kind(corner.index) }
    }

    /// Where it shows: a cut corner's marker, the centre of a round cut, or a sharp corner's anchor.
    fn at(&self) -> Point {
        self.xf * self.corner.on_bisector(self.drawn / self.corner.sin())
    }

    /// The params naming it.
    fn items(&self) -> Value {
        json!([{ "id": self.id.0, "anchors": [self.corner.index] }])
    }
}

/// Every cut corner of the editable paths.
fn cut_corners(doc: &Document) -> Vec<Spot> {
    let editable = doc.editable();
    let mut out = vec![];
    doc.walk(|n| {
        if matches!(n.kind, NodeKind::Path { live: Some(_), guide: false, .. })
            && editable(n.id)
            && let Some(lc) = LiveCorners::of(n)
        {
            out.extend(lc.corners.iter().filter(|c| lc.drawn_radius(c.index) > 0.0).map(|c| Spot::of(&lc, n.id, c)));
        }
    });
    out
}

/// The sharp corner of an editable path nearest `p` within the point tolerance.
fn sharp_corner_at(cx: &ToolContext, p: Point) -> Option<Spot> {
    let (tol, editable) = (cx.point_tol(), cx.doc.editable());
    let mut best: Option<(f64, Spot)> = None;
    cx.doc.walk(|n| {
        let NodeKind::Path { path, guide: false, .. } = &n.kind else { return };
        if !path.anchors().any(|(_, _, a)| a.p.distance(p) <= tol) || !editable(n.id) {
            return;
        }
        let Some(lc) = LiveCorners::of(n) else { return };
        for c in lc.corners.iter().filter(|c| lc.drawn_radius(c.index) <= 0.0) {
            let s = Spot::of(&lc, n.id, c);
            let d = s.at().distance(p);
            if d <= tol && best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, s));
            }
        }
    });
    best.map(|(_, s)| s)
}

/// The corners of the editable paths inside `r`, by path: anchor indices of each uncut outline.
fn corners_inside(cx: &ToolContext, r: Rect) -> BTreeMap<NodeId, Vec<usize>> {
    let editable = cx.doc.editable();
    let mut out = BTreeMap::new();
    cx.doc.walk(|n| {
        if !matches!(n.kind, NodeKind::Path { guide: false, .. }) || !editable(n.id) {
            return;
        }
        let Some(lc) = LiveCorners::of(n) else { return };
        let inside: Vec<usize> = lc.corners.iter().filter(|c| r.contains(lc.xf * c.at)).map(|c| c.index).collect();
        if !inside.is_empty() {
            out.insert(n.id, inside);
        }
    });
    out
}

/// The largest radius `spot` draws with the corners around it as they are.
fn max_radius(doc: &Document, spot: &Spot) -> f64 {
    let Some(lc) = doc.node(spot.id).and_then(LiveCorners::of) else { return 0.0 };
    let radii: Vec<f64> = (0..lc.base.anchor_count()).map(|k| lc.drawn_radius(k)).collect();
    spot.corner.max_shared_radius(&lc.corners, &radii, |k| k == spot.corner.index)
}

/// `select.anchors` params selecting `corners` of path `id`.
fn select_params(cx: &ToolContext, id: NodeId, corners: &[usize], mode: &str) -> Option<Value> {
    let lc = cx.doc.node(id).and_then(LiveCorners::of)?;
    let anchors: Vec<_> = lc.anchors_of(&corners.iter().copied().collect()).into_iter().collect();
    Some(json!({ "id": id.0, "anchors": anchors, "mode": mode }))
}

/// A document revision and the cut corners it has.
type Marks = ((u64, u64), Arc<Vec<Spot>>);

#[derive(Clone, Debug, Default)]
enum State {
    #[default]
    Idle,
    /// Pressed on a corner: a drag sizes it.
    Corner {
        spot: Box<Spot>,
        start: Point,
        /// The largest radius that fits.
        max: f64,
        /// The drag has left the click.
        began: bool,
        /// The radius the drag asks for.
        radius: f64,
        /// Where the pointer is.
        at: Point,
    },
    /// `cut`: an Alt-marquee, which cuts corners; else it selects them.
    Marquee { start: Point, cur: Point, cut: bool, toggle: bool },
}

pub struct CornerTool {
    radius: f64,
    kind: CornerKind,
    state: State,
    hover: Option<Spot>,
    /// The cut corners of the document as it was at a revision, for hovering and the markers.
    cache: Cell<Option<Marks>>,
}

impl Default for CornerTool {
    fn default() -> Self {
        Self { radius: RADIUS, kind: CornerKind::Round, state: State::Idle, hover: None, cache: Cell::new(None) }
    }
}

impl CornerTool {
    fn markers(&self, cx: &ToolContext) -> Arc<Vec<Spot>> {
        let kept = self.cache.take().filter(|(rev, _)| *rev == cx.revision);
        let list = kept.map_or_else(|| Arc::new(cut_corners(cx.doc)), |(_, list)| list);
        self.cache.set(Some((cx.revision, list.clone())));
        list
    }

    /// The corner at `p`: a cut corner's marker, else a sharp corner's anchor.
    fn spot_at(&self, cx: &ToolContext, p: Point) -> Option<Spot> {
        let (marks, tol) = (self.markers(cx), cx.point_tol());
        let dist = |s: &&Spot| s.at().distance(p);
        let mark = marks.iter().filter(|s| dist(s) <= tol).min_by(|a, b| dist(a).total_cmp(&dist(b)));
        mark.cloned().or_else(|| sharp_corner_at(cx, p))
    }

    /// The command a drag of `spot` to `radius` stands for.
    fn sized(&self, spot: &Spot, radius: f64) -> Action {
        if radius > 0.0 {
            let kind = if spot.drawn > 0.0 { spot.kind } else { self.kind };
            Action::Preview(SET.into(), json!({ "items": spot.items(), "radius": radius, "kind": kind }))
        } else {
            Action::Preview(REMOVE.into(), json!({ "items": spot.items() }))
        }
    }

    fn drag_corner(&mut self, cx: &ToolContext, p: Point, mods: Mods) -> Vec<Action> {
        let State::Corner { spot, start, max, began, radius, at } = &mut self.state else { return vec![] };
        *at = p;
        let mut out = vec![];
        if !*began {
            if p.distance(*start) < cx.tol(DRAG_PX) {
                return out;
            }
            *began = true;
            out.push(Action::Begin(LABEL.into()));
        }
        let inv = spot.xf.inverse();
        let change = spot.corner.radius_change(inv * p - inv * *start);
        // From a sharp corner a drag either way sizes it; a cut one grows inward and shrinks back.
        let asked = if spot.drawn > 0.0 { spot.drawn + change } else { change.abs() };
        let asked = if mods.shift { cx.unit.to_pt(cx.unit.from_pt(asked).round()) } else { asked };
        *radius = asked.min(*max).max(0.0);
        let (spot, r) = (spot.clone(), *radius);
        out.push(self.sized(&spot, r));
        out
    }

    /// A press on a corner let go without a drag.
    fn click_corner(&self, cx: &ToolContext, spot: &Spot, mods: Mods) -> Vec<Action> {
        if spot.drawn <= 0.0 {
            return vec![Action::Exec(SET.into(), json!({ "items": spot.items(), "radius": self.radius, "kind": self.kind }))];
        }
        let mode = if mods.shift { "toggle" } else { "set" };
        select_params(cx, spot.id, &[spot.corner.index], mode).map_or_else(Vec::new, |p| vec![Action::Exec("select.anchors".into(), p)])
    }

    fn finish_marquee(&self, cx: &ToolContext, r: Rect, cut: bool, toggle: bool) -> Vec<Action> {
        let inside = corners_inside(cx, r);
        if cut {
            if inside.is_empty() {
                return vec![];
            }
            let items: Vec<Value> = inside.iter().map(|(id, corners)| json!({ "id": id.0, "anchors": corners })).collect();
            return vec![Action::Exec(SET.into(), json!({ "items": items, "radius": self.radius, "kind": self.kind }))];
        }
        if inside.is_empty() && !toggle {
            return vec![Action::Exec("select.none".into(), json!({}))];
        }
        let modes = std::iter::once(if toggle { "toggle" } else { "set" }).chain(std::iter::repeat(if toggle { "toggle" } else { "add" }));
        inside
            .iter()
            .zip(modes)
            .filter_map(|((id, corners), mode)| Some(Action::Exec("select.anchors".into(), select_params(cx, *id, corners, mode)?)))
            .collect()
    }
}

impl Tool for CornerTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                self.hover = self.spot_at(cx, p);
                vec![]
            }
            PointerKind::Down => {
                self.hover = None;
                let spot = if ev.mods.alt { None } else { self.spot_at(cx, p) };
                self.state = match spot {
                    Some(spot) => {
                        // A cut corner's values are the tool's from now on, as the panel shows.
                        if spot.drawn > 0.0 {
                            (self.radius, self.kind) = (spot.drawn, spot.kind);
                        }
                        let max = max_radius(cx.doc, &spot);
                        State::Corner { radius: spot.drawn, spot: Box::new(spot), start: p, max, began: false, at: p }
                    }
                    None => State::Marquee { start: p, cur: p, cut: ev.mods.alt, toggle: ev.mods.shift },
                };
                vec![]
            }
            PointerKind::Drag => {
                if let State::Marquee { cur, .. } = &mut self.state {
                    *cur = p;
                    return vec![];
                }
                self.drag_corner(cx, p, ev.mods)
            }
            PointerKind::Up => match std::mem::take(&mut self.state) {
                State::Idle => vec![],
                State::Marquee { start, cut, toggle, .. } => self.finish_marquee(cx, Rect::from_points(start, p), cut, toggle),
                state @ State::Corner { began: true, .. } => {
                    self.state = state;
                    let mut out = self.drag_corner(cx, p, ev.mods);
                    if let State::Corner { spot, radius, .. } = std::mem::take(&mut self.state) {
                        // Back where it started: nothing to keep.
                        if spot.drawn <= 0.0 && radius <= 0.0 {
                            return vec![Action::Cancel];
                        }
                    }
                    out.push(Action::Commit);
                    out
                }
                State::Corner { spot, .. } => self.click_corner(cx, &spot, ev.mods),
            },
            PointerKind::DoubleClick => {
                self.state = State::Idle;
                match self.spot_at(cx, p).filter(|s| s.drawn > 0.0) {
                    Some(s) => vec![Action::Exec(REMOVE.into(), json!({ "items": s.items() }))],
                    None => vec![],
                }
            }
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key != ToolKey::Escape {
            return vec![];
        }
        match std::mem::take(&mut self.state) {
            State::Corner { began: true, .. } => vec![Action::Cancel],
            _ => vec![],
        }
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        self.busy() && key == ToolKey::Escape
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let ring = |p, color, filled| Overlay::CurvePoint { p, color, filled, size: MARKER_PX };
        match &self.state {
            State::Marquee { start, cur, .. } => return vec![Overlay::Marquee(Rect::from_points(*start, *cur))],
            State::Corner { began: true, radius, max, at, .. } => {
                let mut out = vec![];
                if cx.measurement_labels {
                    out.push(Overlay::Measure { p: *at, text: format!("Radius: {}", cx.len(*radius)) });
                }
                // At the limit of what fits.
                if *max > 0.0 && *radius >= *max {
                    out.push(ring(*at, HIGHLIGHT_RED, false));
                }
                return out;
            }
            _ => {}
        }
        let mut out: Vec<Overlay> = self.markers(cx).iter().take(MAX_MARKERS).map(|s| ring(s.at(), BLUE, false)).collect();
        match &self.hover {
            Some(s) if s.drawn > 0.0 => out.push(ring(s.at(), BLUE, true)),
            Some(s) => out.push(Overlay::Anchor { p: s.at(), color: BLUE, filled: true, size: MARKER_PX }),
            None => {}
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::CornerTool
    }

    fn options(&self) -> Value {
        json!({ "radius": self.radius, "kind": self.kind })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "radius" => {
                if let Some(r) = v.as_f64().filter(|r| (0.0..=MAX_RADIUS).contains(r)) {
                    self.radius = r;
                }
            }
            "kind" => {
                if let Ok(k) = serde_json::from_value(v.clone()) {
                    self.kind = k;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
