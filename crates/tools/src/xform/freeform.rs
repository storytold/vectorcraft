//! The Gradient tool on a freeform gradient.
//!
//! The annotator shows each point as a colour chip, the lines through them and, around the
//! selected point, its spread: a dashed ring with a handle. Click a point to select it
//! (`paint.freeform.selectPoint`; the Gradient and Color panels edit the selected point), drag it
//! to move it, drag it out of the object to remove it, and double-click it for its popover. Drag
//! the ring or its handle to change the spread. Click a line to add a point on it.
//!
//! Points mode (the Gradient panel's Draw toggle): a click inside the selected art adds a point
//! there, coloured as the gradient is there. Lines mode: successive clicks draw a smooth line
//! through the points they add, until Escape (or a click off the art) ends it; clicking an
//! existing point first starts the line from it. Delete or Backspace removes the selected point.

use serde_json::{Value, json};
use vectorcraft_color::freeform::LineHit;
use vectorcraft_color::{Color, Freeform, FreeformMode, FreeformPoint};
use vectorcraft_doc::NodeId;
use vectorcraft_doc::hit::hit_test;
use vectorcraft_geom::{Point, Shape, Vec2};

use super::paint_owner;
use crate::{Action, Cursor, Overlay, ToolContext, ToolKey};

const LINE: [u8; 3] = [0x20, 0x20, 0x20];
/// Hit radius of a point's chip and of a line, screen pixels.
const POINT_HIT: f64 = 7.0;
const LINE_HIT: f64 = 4.0;
/// Hit distance of the spread ring and its handle, screen pixels.
const RING_HIT: f64 = 4.0;
/// The spread handle sits at least this far from its point (so a spread of 0 can be dragged
/// out), screen pixels.
const RING_MIN: f64 = 16.0;
/// A press becomes a drag after this much movement, screen pixels.
const DRAG_START: f64 = 3.0;

/// The freeform gradient behind the active proxy of the first selected object that has one: its
/// points in document coordinates and the length spreads are fractions of there.
pub(super) struct Annotator {
    /// The object it paints.
    pub id: NodeId,
    pub freeform: Freeform,
    pub scale: f64,
}

impl Annotator {
    pub fn of(cx: &ToolContext) -> Option<Self> {
        cx.selection.objects.iter().find_map(|id| {
            let (freeform, scale) = cx.doc.node(*id)?.proxy_freeform(!cx.fill_active, cx.appearance_item)?;
            Some(Self { id: *id, freeform, scale })
        })
    }

    /// The point under `p` (the topmost chip where chips overlap).
    fn point_at(&self, cx: &ToolContext, p: Point) -> Option<usize> {
        (0..self.freeform.points.len()).rev().find(|i| self.freeform.points[*i].at.distance(p) <= cx.tol(POINT_HIT))
    }

    /// The spot on a line under `p`.
    fn line_at(&self, cx: &ToolContext, p: Point) -> Option<LineHit> {
        self.freeform.nearest_on_lines(p).filter(|h| h.distance <= cx.tol(LINE_HIT))
    }

    /// The selected point, if it exists.
    fn selected(&self, cx: &ToolContext) -> Option<usize> {
        cx.freeform_point.filter(|i| *i < self.freeform.points.len())
    }

    /// Point `i`'s spread radius (document units) and where its spread handle sits: on the ring,
    /// to the right of the point (at least [`RING_MIN`] pixels out).
    fn ring(&self, cx: &ToolContext, i: usize) -> (f64, Point) {
        let pt = &self.freeform.points[i];
        let r = pt.spread as f64 * self.scale;
        (r, pt.at + Vec2::new(r.max(cx.tol(RING_MIN)), 0.0))
    }

    /// The selected point when `p` is on its spread ring or handle.
    fn ring_at(&self, cx: &ToolContext, p: Point) -> Option<usize> {
        let i = self.selected(cx)?;
        let (r, handle) = self.ring(cx, i);
        let on_ring = r > 0.0 && (self.freeform.points[i].at.distance(p) - r).abs() <= cx.tol(RING_HIT);
        (on_ring || handle.distance(p) <= cx.tol(RING_HIT)).then_some(i)
    }

    /// Is a line being drawn from the selected point (Lines mode)?
    fn line_open(&self, cx: &ToolContext, lines: &Lines) -> Option<usize> {
        self.selected(cx).filter(|_| lines.open && self.freeform.mode == FreeformMode::Lines)
    }

    pub fn overlays(&self, cx: &ToolContext, lines: &Lines) -> Vec<Overlay> {
        let f = &self.freeform;
        let mut o: Vec<Overlay> =
            (0..f.lines.len()).map(|l| Overlay::Path { path: f.line_path(l), color: LINE, width: 1.0, dashed: false }).collect();
        // The line being drawn, on to the pointer.
        if let (Some(from), Some(hover)) = (self.line_open(cx, lines), lines.hover) {
            o.push(Overlay::Path { path: rubber_band(f, from, hover), color: LINE, width: 1.0, dashed: true });
        }
        if let Some(i) = self.selected(cx) {
            let (r, handle) = self.ring(cx, i);
            if r > 0.0 {
                let path = vectorcraft_geom::kurbo::Circle::new(f.points[i].at, r).to_path(cx.tol(0.25));
                o.push(Overlay::Path { path, color: LINE, width: 1.0, dashed: true });
            }
            o.push(Overlay::Handle { p: handle, color: LINE });
        }
        o.extend(f.points.iter().enumerate().map(|(i, pt)| Overlay::Swatch {
            p: pt.at,
            color: pt.color.to_rgba8(pt.opacity),
            selected: cx.freeform_point == Some(i),
        }));
        o
    }
}

/// The segment a click at `to` would add to the line drawn from point `from`.
fn rubber_band(f: &Freeform, from: usize, to: Point) -> vectorcraft_geom::BezPath {
    let mut f = f.clone();
    let i = f.add_point(FreeformPoint::new(to, Color::BLACK));
    let mut bp = vectorcraft_geom::BezPath::new();
    if let Some(c) = f.connect(from, i).and_then(|l| f.segments(l).into_iter().find(|c| c.p0 == to || c.p3 == to)) {
        bp.move_to(c.p0);
        bp.curve_to(c.p1, c.p2, c.p3);
    }
    bp
}

/// The Gradient tool's freeform state between gestures: the line Lines mode is drawing.
#[derive(Debug, Default)]
pub(super) struct Lines {
    /// A line is being drawn from the selected point: the next click on the art extends it.
    open: bool,
    /// Where the pointer hovers over the art (the line's rubber band runs there).
    hover: Option<Point>,
}

/// What a press on a freeform annotator does.
#[derive(Clone, Debug, PartialEq)]
enum Grab {
    /// A point: a drag moves it, out of the object removes it.
    Point(usize),
    /// The selected point's spread ring: a drag resizes it from radius `r` (the press `d` from
    /// the point at `at`).
    Spread { index: usize, at: Point, r: f64, d: f64, scale: f64 },
    /// A line: a click adds a point there.
    Line(LineHit),
    /// Inside selected art: a click adds a point there.
    Add,
    /// Elsewhere: a click leaves no point selected.
    Clear,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Gesture {
    grab: Grab,
    /// The object whose gradient it edits and its point count at the press.
    id: NodeId,
    points: usize,
    at: Point,
    began: bool,
    /// A dragged point is out of the object (releasing removes it); `left`: it has been.
    out: bool,
    left: bool,
}

impl Gesture {
    /// Has the press become a drag (an interaction is open)?
    pub fn began(&self) -> bool {
        self.began
    }
}

/// `params` aimed at the paint behind the active proxy.
fn on_proxy(cx: &ToolContext, mut params: Value) -> Value {
    params["stroke"] = json!(!cx.fill_active);
    params
}

fn select(i: Option<usize>) -> Action {
    Action::Exec("paint.freeform.selectPoint".into(), json!({ "index": i }))
}

/// Is `p` on selected art (where a click adds a point)?
fn on_art(cx: &ToolContext, p: Point) -> bool {
    hit_test(cx.doc, p, cx.hit_options()).is_some_and(|h| cx.selection.objects.contains(&paint_owner(cx.doc, h.leaf)))
}

pub(super) fn press(cx: &ToolContext, a: &Annotator, p: Point) -> (Vec<Action>, Gesture) {
    let mut out = vec![];
    let grab = if let Some(i) = a.point_at(cx, p) {
        if cx.freeform_point != Some(i) {
            out.push(select(Some(i)));
        }
        Grab::Point(i)
    } else if let Some(index) = a.ring_at(cx, p) {
        let at = a.freeform.points[index].at;
        Grab::Spread { index, at, r: a.ring(cx, index).0, d: at.distance(p), scale: a.scale }
    } else if let Some(h) = a.line_at(cx, p) {
        Grab::Line(h)
    } else if on_art(cx, p) {
        Grab::Add
    } else {
        Grab::Clear
    };
    (out, Gesture { grab, id: a.id, points: a.freeform.points.len(), at: p, began: false, out: false, left: false })
}

pub(super) fn drag(cx: &ToolContext, g: &mut Gesture, p: Point) -> Vec<Action> {
    if !matches!(g.grab, Grab::Point(_) | Grab::Spread { .. }) {
        return vec![];
    }
    let mut out = vec![];
    if !g.began {
        if p.distance(g.at) < cx.tol(DRAG_START) {
            return out;
        }
        g.began = true;
        out.push(Action::Begin("Gradient".into()));
    }
    let (cmd, params) = match g.grab {
        Grab::Point(index) => {
            // Out of the object the point goes (never the last one).
            g.out = g.points > 1 && cx.doc.node(g.id).is_some_and(|n| !n.contains_fn()(p));
            g.left |= g.out;
            if g.out {
                ("paint.freeform.deletePoint", json!({ "index": index }))
            } else {
                ("paint.freeform.setPoint", json!({ "index": index, "at": [p.x, p.y] }))
            }
        }
        Grab::Spread { index, at, r, d, scale } => {
            let spread = if scale > 0.0 { ((r + at.distance(p) - d) / scale).clamp(0.0, 1.0) } else { 0.0 };
            ("paint.freeform.setPoint", json!({ "index": index, "spread": spread }))
        }
        // Checked above.
        _ => return out,
    };
    out.push(Action::Preview(cmd.into(), on_proxy(cx, params)));
    out
}

pub(super) fn release(cx: &ToolContext, a: Option<&Annotator>, g: Gesture, lines: &mut Lines) -> Vec<Action> {
    let mode = a.map_or(FreeformMode::Points, |a| a.freeform.mode);
    if g.began {
        let mut out = vec![Action::Commit];
        // A point dragged out and back was deselected by the removal its drag previewed.
        if let Grab::Point(i) = g.grab
            && g.left
            && !g.out
        {
            out.push(select(Some(i)));
        }
        lines.open &= !g.out;
        return out;
    }
    match g.grab {
        // In Lines mode a line can start from an existing point.
        Grab::Point(_) => {
            lines.open = mode == FreeformMode::Lines;
            vec![]
        }
        Grab::Spread { .. } => vec![],
        Grab::Line(h) => {
            lines.open = false;
            vec![Action::Exec("paint.freeform.splitLine".into(), on_proxy(cx, json!({ "line": h.line, "segment": h.segment, "t": h.t })))]
        }
        Grab::Add => {
            let join = a.is_some_and(|a| a.line_open(cx, lines).is_some());
            lines.open = mode == FreeformMode::Lines;
            vec![Action::Exec("paint.freeform.addPoint".into(), on_proxy(cx, json!({ "at": [g.at.x, g.at.y], "line": join })))]
        }
        Grab::Clear => {
            lines.open = false;
            if cx.freeform_point.is_some() { vec![select(None)] } else { vec![] }
        }
    }
}

/// Escape during a drag: the point a drag-out deselected is selected again.
pub(super) fn cancel(g: &Gesture) -> Vec<Action> {
    match g.grab {
        Grab::Point(i) if g.out => vec![Action::Cancel, select(Some(i))],
        _ => vec![Action::Cancel],
    }
}

/// Double-clicking a point opens its popover beside it.
pub(super) fn double_click(cx: &ToolContext, a: &Annotator, p: Point) -> Vec<Action> {
    let Some(i) = a.point_at(cx, p) else { return vec![] };
    let at = a.freeform.points[i].at;
    let mut out = if cx.freeform_point != Some(i) { vec![select(Some(i))] } else { vec![] };
    out.push(Action::Dialog("gradientStop".into(), json!({ "index": i, "x": at.x, "y": at.y })));
    out
}

/// Pointer moves: Lines mode's rubber band follows the pointer over the art (where a click
/// extends the line).
pub(super) fn hover(cx: &ToolContext, lines: &mut Lines, p: Point) {
    lines.hover = (lines.open && on_art(cx, p)).then_some(p);
}

/// Delete / Backspace remove the selected point (never the last one); Escape ends the line being
/// drawn.
pub(super) fn claims_key(cx: &ToolContext, a: &Annotator, lines: &Lines, key: ToolKey) -> bool {
    match key {
        ToolKey::Delete | ToolKey::Backspace => a.selected(cx).is_some() && a.freeform.points.len() > 1,
        ToolKey::Escape => a.line_open(cx, lines).is_some(),
        _ => false,
    }
}

pub(super) fn key(cx: &ToolContext, a: &Annotator, lines: &mut Lines, key: ToolKey) -> Vec<Action> {
    if !claims_key(cx, a, lines, key) {
        return vec![];
    }
    if key == ToolKey::Escape {
        lines.open = false;
        return vec![];
    }
    vec![Action::Exec("paint.freeform.deletePoint".into(), on_proxy(cx, json!({ "index": a.selected(cx) })))]
}

pub(super) fn cursor(cx: &ToolContext, a: &Annotator, g: Option<&Gesture>, p: Point) -> Cursor {
    match g.filter(|g| g.began).map(|g| (&g.grab, g.out)) {
        Some((Grab::Point(_), true)) => return Cursor::RemoveStop,
        Some((Grab::Spread { .. }, _)) => return Cursor::ResizeH.turned(cx.view_angle()),
        Some(_) => return Cursor::Move,
        None => {}
    }
    if a.point_at(cx, p).is_some() {
        Cursor::Move
    } else if a.ring_at(cx, p).is_some() {
        Cursor::ResizeH.turned(cx.view_angle())
    } else if a.line_at(cx, p).is_some() || on_art(cx, p) {
        Cursor::AddStop
    } else {
        Cursor::Crosshair
    }
}

#[cfg(test)]
mod tests {
    use super::super::GradientTool;
    use super::*;
    use crate::testutil::*;
    use crate::{PointerEvent, PointerKind, Tool};
    use vectorcraft_color::{FreeformPoint, Gradient, GradientKind, GradientPaint, Paint};
    use vectorcraft_doc::{Document, Selection};

    /// The test rectangle (100..200) with three points; `lines` mode joins new ones.
    fn doc(mode: FreeformMode) -> (Document, Selection) {
        let (mut d, id) = doc_with_rect();
        let mut g = GradientPaint::new(Gradient { kind: GradientKind::Freeform, ..Default::default() });
        let pt = |x, y| FreeformPoint { spread: 0.2, ..FreeformPoint::new(Point::new(x, y), vectorcraft_color::Color::rgb(1.0, 0.0, 0.0)) };
        g.freeform = Some(Freeform { points: vec![pt(120.0, 120.0), pt(180.0, 120.0), pt(150.0, 180.0)], lines: vec![vec![0, 1]], mode });
        d.node_mut(id).unwrap().appearance.set_fill(Paint::Gradient(Box::new(g)));
        let mut s = Selection::default();
        s.add(id);
        (d, s)
    }

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    fn click(t: &mut GradientTool, cx: &ToolContext, x: f64, y: f64) -> Vec<Action> {
        let mut a = t.pointer(cx, &ev(PointerKind::Down, x, y));
        a.extend(t.pointer(cx, &ev(PointerKind::Up, x, y)));
        a
    }

    #[test]
    fn clicks_select_points_add_points_and_split_lines() {
        let (d, s) = doc(FreeformMode::Points);
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        let mut t = GradientTool::default();
        assert_eq!(click(&mut t, &cx, 151.0, 181.0), vec![select(Some(2))]);
        cx.freeform_point = Some(2);
        // Inside the art: a point there (Points mode: not joined).
        let a = click(&mut t, &cx, 140.0, 150.0);
        assert_eq!(a, vec![Action::Exec("paint.freeform.addPoint".into(), json!({"at": [140.0, 150.0], "line": false, "stroke": false}))]);
        // On the line: a point on it.
        let a = click(&mut t, &cx, 150.0, 122.0);
        let Action::Exec(c, v) = &a[0] else { panic!("{a:?}") };
        assert_eq!((c.as_str(), &v["line"], &v["segment"]), ("paint.freeform.splitLine", &json!(0), &json!(0)));
        // Off the art: the point selection clears.
        assert_eq!(click(&mut t, &cx, 400.0, 400.0), vec![select(None)]);
        // The overlays: a chip per point (the selected one ringed), the line and the spread.
        let o = t.overlays(&cx);
        assert_eq!(o.iter().filter(|o| matches!(o, Overlay::Swatch { .. })).count(), 3);
        assert!(o.iter().any(|o| matches!(o, Overlay::Swatch { selected: true, p, .. } if p == &Point::new(150.0, 180.0))));
        assert_eq!(o.iter().filter(|o| matches!(o, Overlay::Path { dashed: true, .. })).count(), 1, "the spread circle");
    }

    #[test]
    fn lines_mode_joins_new_points_and_drags_move_points() {
        let (d, s) = doc(FreeformMode::Lines);
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.freeform_point = Some(1);
        let mut t = GradientTool::default();
        // Clicking the selected point starts a line from it; a click on the art extends it.
        assert!(click(&mut t, &cx, 180.0, 120.0).is_empty());
        let a = click(&mut t, &cx, 190.0, 190.0);
        assert_eq!(a, vec![Action::Exec("paint.freeform.addPoint".into(), json!({"at": [190.0, 190.0], "line": true, "stroke": false}))]);
        t.pointer(&cx, &ev(PointerKind::Down, 120.0, 121.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 130.0, 140.0));
        assert_eq!(a[0], Action::Begin("Gradient".into()));
        assert_eq!(a[1], Action::Preview("paint.freeform.setPoint".into(), json!({"index": 0, "at": [130.0, 140.0], "stroke": false})));
        assert_eq!(t.cursor(&cx, Point::new(130.0, 140.0), Default::default()), Cursor::Move);
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 130.0, 140.0)), vec![Action::Commit]);
        // Delete removes the selected point.
        assert!(t.claims_key(&cx, ToolKey::Delete));
        assert_eq!(
            t.key(&cx, ToolKey::Delete, Default::default()),
            vec![Action::Exec("paint.freeform.deletePoint".into(), json!({"index": 1, "stroke": false}))]
        );
    }

    #[test]
    fn successive_clicks_draw_a_line_until_escape() {
        let (d, s) = doc(FreeformMode::Lines);
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        let mut t = GradientTool::default();
        let add = |x: f64, y: f64, line: bool| Action::Exec("paint.freeform.addPoint".into(), json!({"at": [x, y], "line": line, "stroke": false}));
        // The first click starts a line; the engine selects each new point, which the next joins.
        assert_eq!(click(&mut t, &cx, 140.0, 150.0), vec![add(140.0, 150.0, false)]);
        // (The engine selects the new point; any existing one stands in for it here.)
        cx.freeform_point = Some(2);
        assert!(t.claims_key(&cx, ToolKey::Escape));
        // The rubber band follows the pointer from the line's end.
        t.pointer(&cx, &ev(PointerKind::Move, 170.0, 160.0));
        let dashed = |t: &GradientTool, cx: &ToolContext| t.overlays(cx).iter().filter(|o| matches!(o, Overlay::Path { dashed: true, .. })).count();
        assert_eq!(dashed(&t, &cx), 2, "the rubber band and the selected point's spread ring");
        // Off the art (where a click ends the line) it goes.
        t.pointer(&cx, &ev(PointerKind::Move, 400.0, 400.0));
        assert_eq!(dashed(&t, &cx), 1);
        assert_eq!(click(&mut t, &cx, 170.0, 160.0), vec![add(170.0, 160.0, true)]);
        // Escape ends the line: the next click starts another.
        assert!(t.key(&cx, ToolKey::Escape, Default::default()).is_empty());
        assert!(!t.claims_key(&cx, ToolKey::Escape));
        assert_eq!(dashed(&t, &cx), 1);
        assert_eq!(click(&mut t, &cx, 130.0, 170.0), vec![add(130.0, 170.0, false)]);
        // Points mode never joins.
        let (d, s) = doc(FreeformMode::Points);
        let mut cx = crate::testutil::cx(&d, &s, &p);
        cx.freeform_point = Some(0);
        let mut t = GradientTool::default();
        click(&mut t, &cx, 120.0, 120.0);
        assert_eq!(click(&mut t, &cx, 140.0, 150.0), vec![add(140.0, 150.0, false)]);
        assert!(!t.claims_key(&cx, ToolKey::Escape));
    }

    #[test]
    fn a_point_dragged_out_of_the_object_is_removed() {
        let (d, s) = doc(FreeformMode::Points);
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.freeform_point = Some(0);
        let mut t = GradientTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 120.0, 120.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 60.0, 60.0));
        assert_eq!(a[1], Action::Preview("paint.freeform.deletePoint".into(), json!({"index": 0, "stroke": false})));
        assert_eq!(t.cursor(&cx, Point::new(60.0, 60.0), Default::default()), Cursor::RemoveStop);
        // Back inside it moves again.
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 110.0, 110.0));
        assert_eq!(a[0], Action::Preview("paint.freeform.setPoint".into(), json!({"index": 0, "at": [110.0, 110.0], "stroke": false})));
        // Released there it is selected again (the removal previewed on the way deselected it).
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 110.0, 110.0)), vec![Action::Commit, select(Some(0))]);
        t.pointer(&cx, &ev(PointerKind::Down, 120.0, 120.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 60.0, 60.0));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 60.0, 60.0)), vec![Action::Commit]);
        // Escape mid-drag restores the point and its selection.
        t.pointer(&cx, &ev(PointerKind::Down, 120.0, 120.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 60.0, 60.0));
        assert_eq!(t.key(&cx, ToolKey::Escape, Default::default()), vec![Action::Cancel, select(Some(0))]);
    }

    #[test]
    fn the_spread_ring_drags_the_spread() {
        let (d, s) = doc(FreeformMode::Points);
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.freeform_point = Some(2);
        let mut t = GradientTool::default();
        // Spread 0.2 of half the 100 pt box: a 10 pt ring around (150, 180), its handle on it.
        assert_eq!(t.cursor(&cx, Point::new(150.0, 170.0), Default::default()), Cursor::ResizeH);
        assert!(t.overlays(&cx).contains(&Overlay::Handle { p: Point::new(166.0, 180.0), color: LINE }), "at least 16 px out");
        // In a view turned 90° the handle shows below the point and drags up and down on screen.
        let mut turned = crate::testutil::cx(&d, &s, &p);
        turned.freeform_point = Some(2);
        turned.screen = turned_view(90.0);
        assert_eq!(t.cursor(&turned, Point::new(166.0, 180.0), Default::default()), Cursor::ResizeV);
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 160.0, 180.0)).is_empty());
        let spread = |a: &[Action]| match a.last() {
            Some(Action::Preview(c, v)) if c == "paint.freeform.setPoint" && v["index"] == 2 => (v["spread"].as_f64().unwrap() * 1e4).round() / 1e4,
            a => panic!("{a:?}"),
        };
        assert_eq!(spread(&t.pointer(&cx, &ev(PointerKind::Drag, 170.0, 180.0))), 0.4);
        // Dragged in, it shrinks with the pointer's distance.
        assert_eq!(spread(&t.pointer(&cx, &ev(PointerKind::Drag, 145.0, 180.0))), 0.1);
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 145.0, 180.0)), vec![Action::Commit]);
        // Grabbed by the handle (16 px out), it reaches no spread before the pointer reaches the point.
        t.pointer(&cx, &ev(PointerKind::Down, 166.0, 180.0));
        assert_eq!(spread(&t.pointer(&cx, &ev(PointerKind::Drag, 152.0, 180.0))), 0.0);
        t.pointer(&cx, &ev(PointerKind::Up, 152.0, 180.0));
        // The handle of a point without spread drags one out.
        let mut d0 = d.clone();
        let n = d0.node_mut(s.objects[0]).unwrap();
        let Paint::Gradient(mut g) = n.appearance.fill_paint() else { panic!("a gradient fill") };
        g.freeform.as_mut().unwrap().points[2].spread = 0.0;
        n.appearance.set_fill(Paint::Gradient(g));
        let mut cx = crate::testutil::cx(&d0, &s, &p);
        cx.freeform_point = Some(2);
        t.pointer(&cx, &ev(PointerKind::Down, 166.0, 180.0));
        assert_eq!(spread(&t.pointer(&cx, &ev(PointerKind::Drag, 176.0, 180.0))), 0.2);
    }

    #[test]
    fn double_clicking_a_point_opens_its_popover() {
        let (d, s) = doc(FreeformMode::Points);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let a = GradientTool::default().pointer(&cx, &ev(PointerKind::DoubleClick, 181.0, 119.0));
        assert_eq!(a, vec![select(Some(1)), Action::Dialog("gradientStop".into(), json!({"index": 1, "x": 180.0, "y": 120.0}))]);
        assert!(GradientTool::default().pointer(&cx, &ev(PointerKind::DoubleClick, 140.0, 150.0)).is_empty());
    }
}
