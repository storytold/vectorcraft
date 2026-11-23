//! Trim at Crossings Tool (Path Precision Tools): a click removes the stretch of the path under the
//! pointer between the places where other paths, or the path itself, cross it; a drag draws a
//! rectangle and removes every stretch it touches; an Alt-drag draws a freehand line and removes
//! every stretch it crosses. Each runs `addon.pathtools.trim` on release, as one undo step, on
//! every path that can be edited, selected or not. Escape during a drag removes nothing.
//!
//! Hovering shows in red the stretch a click would remove while finding its crossings takes at
//! most a twentieth of the work a click may do ([`HOVER`]), and a drag shows what it would remove while
//! it reaches few paths ([`PREVIEW`]).

pub mod plan;

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{BezPath, PathData, Point, Rect};
use vectorcraft_pathops::addons::pathtools as pt;

use self::plan::{Limits, Pick, Stretches};
use crate::builder::HIGHLIGHT_RED;
use crate::draw2::FEEDBACK;
use crate::xform::polygon;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "trimAtCrossings";
/// How near the pointer a path counts as under it (screen pixels).
pub const REACH: f64 = 4.0;
/// A press that moves less than this many screen pixels is a click.
const MIN_DRAG_PX: f64 = 3.0;
/// The most points a freehand line keeps (one per screen pixel dragged). A longer one is dropped.
pub(crate) const MAX_POINTS: usize = 65_536;
/// The command the tool runs.
const COMMAND: &str = "addon.pathtools.trim";
/// How much finding the crossings for a preview may take: a twentieth of what the command may.
const PREVIEW_CROSSINGS: pt::Work = pt::Work::share(20);
/// How much hovering may work out on each move: past it the hover shows no highlight, and a
/// click still removes the stretch.
pub const HOVER: Limits = Limits { crossings: PREVIEW_CROSSINGS, ..Limits::COMMAND };
/// How much a drag's preview may work out on each move: past it the drag shows no preview, and
/// the release still removes everything it reaches.
pub const PREVIEW: Limits = Limits { paths: 32, box_tests: 10_000_000, line_tests: 1_000_000, crossings: PREVIEW_CROSSINGS };
/// The most points a freehand line has for the drag to show a preview.
const PREVIEW_POINTS: usize = 4_096;
/// The preview's line width (screen pixels).
const PREVIEW_PX: f32 = 3.0;

#[derive(Default)]
pub struct TrimTool {
    /// Where the press started, while the button is down.
    start: Option<Point>,
    end: Point,
    /// An Alt-drag's freehand line (empty for a rectangle).
    line: Vec<Point>,
    freehand: bool,
    /// The pointer went `MIN_DRAG_PX` or more from the press: the gesture is a drag from then on.
    moved: bool,
    /// The stretches the gesture would remove, and the document state they were found in.
    preview: Vec<BezPath>,
    preview_state: Option<State>,
    /// Finding the drag's crossings took more work than a preview may: it shows none until the
    /// next press.
    gave_up: bool,
    /// The stretches of the paths looked at so far, for the document state `seen`.
    stretches: Stretches,
    seen: Option<State>,
}

/// The document's revision and its isolated object: what a pick is worked out for.
type State = ((u64, u64), Option<NodeId>);

fn state(cx: &ToolContext) -> State {
    (cx.revision, cx.isolation)
}

/// How near the pointer a click finds a path (pt).
fn reach(cx: &ToolContext) -> f64 {
    cx.tol(REACH).min(100.0)
}

impl TrimTool {
    fn reset(&mut self) {
        self.start = None;
        self.line.clear();
        self.freehand = false;
        self.moved = false;
        self.gave_up = false;
        self.preview.clear();
    }

    /// Work out what `pick` would remove, within `limits`, as the preview: nothing when it
    /// removes nothing or needs more work than that (then why).
    fn look(&mut self, cx: &ToolContext, pick: &Pick, limits: Limits) -> Option<plan::TooMuch> {
        if self.seen != Some(state(cx)) {
            self.stretches.clear();
            self.seen = Some(state(cx));
        }
        let root = crate::ordinary_isolation(cx.doc, cx.isolation);
        self.preview_state = Some(state(cx));
        match plan::plan(cx.doc, root, pick, limits, &mut self.stretches) {
            Ok(all) => {
                self.preview = all.into_iter().flat_map(|t| t.removed).map(|sp| PathData::single(sp).to_bezpath()).collect();
                None
            }
            Err(e) => {
                self.preview.clear();
                Some(e)
            }
        }
    }

    /// The preview of the drag as it is: its rectangle, or its freehand line to `to`.
    fn look_at_drag(&mut self, cx: &ToolContext, to: Point) {
        let Some(start) = self.start else { return };
        if self.gave_up {
            self.preview.clear();
            return;
        }
        if !self.freehand {
            self.look_or_give_up(cx, &Pick::Rect(Rect::from_points(start, to)));
            return;
        }
        if self.line.len() > PREVIEW_POINTS {
            self.preview.clear();
            return;
        }
        let mut pts = self.line.clone();
        if pts.last().is_some_and(|p| *p != to) {
            pts.push(to);
        }
        if pts.len() < 2 {
            self.preview.clear();
            return;
        }
        let bounds = pts.iter().fold(Rect::from_points(to, to), |r, q| r.union_pt(*q));
        self.look_or_give_up(cx, &Pick::Line(pts, bounds));
    }

    /// [`Self::look`] for a drag. A drag that reaches too many paths may come back to fewer; one
    /// whose crossings or line take too much work shows nothing more.
    fn look_or_give_up(&mut self, cx: &ToolContext, pick: &Pick) {
        if let Some(plan::TooMuch::Crossings | plan::TooMuch::Line) = self.look(cx, pick, PREVIEW) {
            self.gave_up = true;
        }
    }
}

impl Tool for TrimTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.start.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if !ev.pos.is_finite() {
            if ev.kind == PointerKind::Up {
                self.reset();
            }
            self.preview.clear();
            return vec![];
        }
        match ev.kind {
            PointerKind::Move => {
                self.look(cx, &Pick::At(ev.pos, reach(cx)), HOVER);
                vec![]
            }
            PointerKind::Down => {
                self.start = Some(ev.pos);
                self.end = ev.pos;
                self.freehand = ev.mods.alt;
                self.moved = false;
                self.line = if self.freehand { vec![ev.pos] } else { vec![] };
                // Until the pointer moves away, the press is a click at this point.
                self.look(cx, &Pick::At(ev.pos, reach(cx)), HOVER);
                vec![]
            }
            PointerKind::Drag if self.start.is_some() => {
                self.end = ev.pos;
                self.moved |= self.start.is_some_and(|start| start.distance(ev.pos) >= cx.tol(MIN_DRAG_PX));
                if self.freehand && self.line.last().is_none_or(|p| p.distance(ev.pos) >= cx.tol(1.0)) {
                    // A line cut short would cross other stretches than the one drawn: drop it.
                    if self.line.len() >= MAX_POINTS {
                        self.reset();
                        return vec![];
                    }
                    self.line.push(ev.pos);
                }
                if self.moved {
                    self.look_at_drag(cx, ev.pos);
                }
                vec![]
            }
            PointerKind::Up => {
                let Some(start) = self.start else { return vec![] };
                let mut line = std::mem::take(&mut self.line);
                let (freehand, moved) = (self.freehand, self.moved);
                self.reset();
                // A press that stayed near where it started is a click, even when a freehand line
                // took a few points on the way. One that went farther stays a drag when it comes back.
                if !moved && start.distance(ev.pos) < cx.tol(MIN_DRAG_PX) {
                    return vec![Action::Exec(COMMAND.into(), json!({"at": [start.x, start.y], "tolerance": reach(cx)}))];
                }
                if !freehand {
                    let r = Rect::from_points(start, ev.pos);
                    return vec![Action::Exec(COMMAND.into(), json!({"rect": [r.x0, r.y0, r.width(), r.height()]}))];
                }
                if line.last().is_some_and(|p| *p != ev.pos) {
                    line.push(ev.pos);
                }
                let points: Vec<_> = line.iter().map(|p| json!([p.x, p.y])).collect();
                if points.len() < 2 { vec![] } else { vec![Action::Exec(COMMAND.into(), json!({ "points": points }))] }
            }
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.reset();
        }
        vec![]
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        self.start.is_some() && key == ToolKey::Escape
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        if self.start.is_none() {
            self.preview.clear();
        }
    }

    /// The highlight shows what lies under the pointer, which panning or an edit changes.
    fn wants_rehover(&self) -> bool {
        true
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        // A preview found in another revision of the document, or for another isolated object, is
        // out of date.
        let mut out: Vec<Overlay> = if self.preview_state == Some(state(cx)) {
            self.preview.iter().map(|p| Overlay::Path { path: p.clone(), color: HIGHLIGHT_RED, width: PREVIEW_PX, dashed: false }).collect()
        } else {
            vec![]
        };
        let Some(start) = self.start else { return out };
        if self.freehand {
            if self.line.len() >= 2 {
                out.push(Overlay::Path { path: polygon(&self.line, false), color: FEEDBACK, width: 1.0, dashed: false });
            }
        } else if self.moved {
            out.push(Overlay::Marquee(Rect::from_points(start, self.end)));
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Knife
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Document, Node, Selection};
    use vectorcraft_geom::kurbo::{ParamCurve, PathSeg};
    use vectorcraft_geom::shapes;

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    #[test]
    fn a_click_trims_at_the_point_with_nothing_selected() {
        let (d, _) = doc_with_rect();
        let (sel, p) = (Selection::default(), paint());
        let mut cx = cx(&d, &sel, &p);
        cx.zoom = 2.0;
        let mut t = TrimTool::default();
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 100.0, 100.0)).is_empty());
        // No marquee for a press; the rectangle, which nothing crosses, would go whole.
        assert!(t.busy() && matches!(t.overlays(&cx).as_slice(), [Overlay::Path { color: HIGHLIGHT_RED, .. }]));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 100.5, 100.0));
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"at": [100.0, 100.0], "tolerance": 2.0}))], "4 px at 200%");
        assert!(!t.busy());
        assert_eq!(t.cursor(&cx, Point::ZERO, Mods::default()), Cursor::Knife, "the Knife's cursor, the blade's point on the pointer");
        assert_eq!(Cursor::Knife.precise(), Cursor::Crosshair, "Use Precise Cursors shows the crosshair");
    }

    #[test]
    fn a_drag_trims_what_its_rectangle_touches_and_escape_drops_it() {
        let (d, _) = doc_with_rect();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TrimTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 160.0, 130.0));
        assert!(t.pointer(&cx, &ev(PointerKind::Drag, 140.0, 90.0)).is_empty(), "nothing runs while dragging");
        assert!(matches!(
            t.overlays(&cx).as_slice(),
            [Overlay::Path { color: HIGHLIGHT_RED, .. }, Overlay::Marquee(r)] if *r == Rect::new(140.0, 90.0, 160.0, 130.0)
        ));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 140.0, 90.0));
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"rect": [140.0, 90.0, 20.0, 40.0]}))]);
        // Escape during a drag: nothing runs on release, and the key is the tool's only then.
        assert!(!t.claims_key(&cx, ToolKey::Escape));
        t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 60.0, 60.0));
        assert!(t.claims_key(&cx, ToolKey::Escape));
        assert!(t.key(&cx, ToolKey::Escape, Mods::default()).is_empty());
        assert!(!t.busy() && t.overlays(&cx).is_empty());
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 60.0, 60.0)).is_empty());
    }

    #[test]
    fn a_drag_that_comes_back_near_the_press_stays_a_rectangle() {
        let (d, _) = doc_with_rect();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TrimTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 20.0, 20.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 1.0, 1.0));
        assert!(matches!(t.overlays(&cx).as_slice(), [Overlay::Marquee(r)] if *r == Rect::new(0.0, 0.0, 1.0, 1.0)));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 1.0, 1.0));
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"rect": [0.0, 0.0, 1.0, 1.0]}))]);
        // A drag that stays within a pixel or two is still a click.
        t.pointer(&cx, &ev(PointerKind::Down, 0.0, 0.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 2.0, 0.0));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 1.0, 1.0));
        assert!(matches!(up.as_slice(), [Action::Exec(_, v)] if v.get("at").is_some()), "{up:?}");
    }

    #[test]
    fn an_alt_drag_trims_what_its_line_crosses() {
        let (d, _) = doc_with_rect();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let alt = Mods { alt: true, ..Mods::default() };
        let mut t = TrimTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0).with_mods(alt));
        t.pointer(&cx, &ev(PointerKind::Drag, 10.2, 10.0).with_mods(alt));
        t.pointer(&cx, &ev(PointerKind::Drag, 40.0, 30.0));
        assert!(matches!(t.overlays(&cx).as_slice(), [Overlay::Path { .. }]));
        // Alt at the press decides; points less than a pixel apart are one.
        let up = t.pointer(&cx, &ev(PointerKind::Up, 80.0, 30.0));
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"points": [[10.0, 10.0], [40.0, 30.0], [80.0, 30.0]]}))]);
        // An Alt-click is a click.
        t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0).with_mods(alt));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 10.0, 10.0).with_mods(alt));
        assert!(matches!(up.as_slice(), [Action::Exec(_, v)] if v.get("at").is_some()), "{up:?}");
        // So is one that moves a pixel or two, and one that goes away and comes back is a line.
        t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0).with_mods(alt));
        t.pointer(&cx, &ev(PointerKind::Drag, 12.0, 10.0).with_mods(alt));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 12.0, 11.0).with_mods(alt));
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"at": [10.0, 10.0], "tolerance": 4.0}))]);
        t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0).with_mods(alt));
        t.pointer(&cx, &ev(PointerKind::Drag, 40.0, 10.0).with_mods(alt));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 10.0, 11.0).with_mods(alt));
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"points": [[10.0, 10.0], [40.0, 10.0], [10.0, 11.0]]}))]);
        // A line past the cap is dropped rather than cut short.
        t.pointer(&cx, &ev(PointerKind::Down, 0.0, 0.0).with_mods(alt));
        for i in 1..=MAX_POINTS {
            t.pointer(&cx, &ev(PointerKind::Drag, i as f64 * 2.0, 0.0));
        }
        assert!(!t.busy());
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 0.0, 0.0)).is_empty());
    }

    /// A document holding `paths`, bottom to top.
    fn doc_of(paths: Vec<PathData>) -> (Document, Vec<NodeId>) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let mut ids = vec![];
        for (i, p) in paths.into_iter().enumerate() {
            let id = d.alloc_id();
            d.insert(Some(l), i, Node::path(id, p, Appearance::default_art())).unwrap();
            ids.push(id);
        }
        (d, ids)
    }

    fn open(pts: &[(f64, f64)]) -> PathData {
        let mut b = BezPath::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 { b.move_to((*x, *y)) } else { b.line_to((*x, *y)) }
        }
        PathData::from_bezpath(&b)
    }

    /// The red stretches shown, each as its two ends.
    fn shown(t: &TrimTool, cx: &ToolContext) -> Vec<(Point, Point)> {
        t.overlays(cx)
            .iter()
            .filter_map(|o| match o {
                Overlay::Path { path, color, .. } if *color == HIGHLIGHT_RED => {
                    let segs: Vec<PathSeg> = path.segments().collect();
                    Some((segs.first()?.start(), segs.last()?.end()))
                }
                _ => None,
            })
            .collect()
    }

    fn hover(t: &mut TrimTool, cx: &ToolContext, x: f64, y: f64) -> Vec<(Point, Point)> {
        assert!(t.pointer(cx, &ev(PointerKind::Move, x, y)).is_empty(), "hovering runs nothing");
        shown(t, cx)
    }

    /// The two ends, either way round, are near (ax, ay) and (bx, by).
    fn ends_are(e: (Point, Point), (ax, ay): (f64, f64), (bx, by): (f64, f64)) -> bool {
        let near = |p: Point, x: f64, y: f64| p.distance(Point::new(x, y)) < 1e-3;
        (near(e.0, ax, ay) && near(e.1, bx, by)) || (near(e.0, bx, by) && near(e.1, ax, ay))
    }

    /// What a click at (x, y) with the tool's reach removes, worked out as the command does.
    fn click_removes(d: &Document, cx: &ToolContext, x: f64, y: f64) -> Vec<BezPath> {
        let all = plan::plan(d, None, &Pick::At(Point::new(x, y), reach(cx)), Limits::COMMAND, &mut Stretches::default()).unwrap();
        all.into_iter().flat_map(|t| t.removed).map(|sp| PathData::single(sp).to_bezpath()).collect()
    }

    fn red_paths(t: &TrimTool, cx: &ToolContext) -> Vec<BezPath> {
        t.overlays(cx)
            .into_iter()
            .filter_map(|o| match o {
                Overlay::Path { path, color, .. } if color == HIGHLIGHT_RED => Some(path),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn hovering_shows_the_stretch_a_click_removes_on_a_line_across_an_ellipse() {
        let ellipse = shapes::ellipse(Rect::new(100.0, 100.0, 300.0, 200.0));
        let (d, _) = doc_of(vec![ellipse, open(&[(50.0, 150.0), (350.0, 150.0)])]);
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TrimTool::default();
        // The ellipse's upper half, between the two places the line crosses it.
        let top = hover(&mut t, &cx, 200.0, 101.0);
        assert!(matches!(top.as_slice(), [e] if ends_are(*e, (100.0, 150.0), (300.0, 150.0))), "{top:?}");
        assert_eq!(red_paths(&t, &cx), click_removes(&d, &cx, 200.0, 101.0));
        // The line's end outside the ellipse, and its middle inside it.
        let end = hover(&mut t, &cx, 70.0, 150.0);
        assert!(matches!(end.as_slice(), [e] if ends_are(*e, (50.0, 150.0), (100.0, 150.0))), "{end:?}");
        let middle = hover(&mut t, &cx, 200.0, 151.0);
        assert!(matches!(middle.as_slice(), [e] if ends_are(*e, (100.0, 150.0), (300.0, 150.0))), "{middle:?}");
        assert_eq!(red_paths(&t, &cx), click_removes(&d, &cx, 200.0, 151.0));
        // Off every path: no highlight.
        assert!(hover(&mut t, &cx, 400.0, 400.0).is_empty());
        assert!(t.overlays(&cx).is_empty());
    }

    #[test]
    fn hovering_an_open_square_and_a_path_that_crosses_itself() {
        // Three sides of a square, crossed by an upright through the top and bottom sides.
        let square = open(&[(100.0, 100.0), (200.0, 100.0), (200.0, 200.0), (100.0, 200.0)]);
        let (d, _) = doc_of(vec![square, open(&[(150.0, 50.0), (150.0, 250.0)])]);
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TrimTool::default();
        let side = hover(&mut t, &cx, 201.0, 150.0);
        assert!(matches!(side.as_slice(), [e] if ends_are(*e, (150.0, 100.0), (150.0, 200.0))), "{side:?}");
        assert_eq!(red_paths(&t, &cx), click_removes(&d, &cx, 201.0, 150.0));
        let corner = hover(&mut t, &cx, 110.0, 99.0);
        assert!(matches!(corner.as_slice(), [e] if ends_are(*e, (100.0, 100.0), (150.0, 100.0))), "{corner:?}");

        // A Z drawn back over itself: its first and last sides cross at (50, 50).
        let (d, _) = doc_of(vec![open(&[(0.0, 0.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)])]);
        let cx = crate::testutil::cx(&d, &sel, &p);
        let mut t = TrimTool::default();
        let lobe = hover(&mut t, &cx, 101.0, 50.0);
        assert!(matches!(lobe.as_slice(), [e] if ends_are(*e, (50.0, 50.0), (50.0, 50.0))), "{lobe:?}");
        assert_eq!(red_paths(&t, &cx), click_removes(&d, &cx, 101.0, 50.0));
        let tail = hover(&mut t, &cx, 10.0, 11.0);
        assert!(matches!(tail.as_slice(), [e] if ends_are(*e, (0.0, 0.0), (50.0, 50.0))), "{tail:?}");
    }

    #[test]
    fn hidden_and_locked_paths_show_nothing_and_a_changed_document_drops_the_highlight() {
        let (mut d, ids) = doc_of(vec![open(&[(0.0, 100.0), (300.0, 100.0)]), open(&[(0.0, 200.0), (300.0, 200.0)])]);
        d.node_mut(ids[0]).unwrap().visible = false;
        d.node_mut(ids[1]).unwrap().locked = true;
        let (sel, p) = (Selection::default(), paint());
        let mut cx = cx(&d, &sel, &p);
        let mut t = TrimTool::default();
        assert!(hover(&mut t, &cx, 150.0, 100.0).is_empty(), "hidden");
        assert!(hover(&mut t, &cx, 150.0, 200.0).is_empty(), "locked");

        let (d, _) = doc_of(vec![open(&[(0.0, 100.0), (300.0, 100.0)])]);
        cx.doc = &d;
        assert_eq!(hover(&mut t, &cx, 150.0, 100.0).len(), 1);
        // Once the document changes, the highlight found before is not shown.
        cx.revision = (0, 1);
        assert!(shown(&t, &cx).is_empty());
        // Leaving the canvas clears it.
        cx.revision = (0, 0);
        t.hover_ended(&cx);
        assert!(shown(&t, &cx).is_empty());
    }

    #[test]
    fn in_isolation_mode_only_paths_inside_the_isolated_group_are_highlighted() {
        // A group holding a line across, and an upright outside it that crosses the line.
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let across = Node::path(d.alloc_id(), open(&[(0.0, 100.0), (300.0, 100.0)]), Appearance::default_art());
        let group = Node::group(d.alloc_id(), vec![std::sync::Arc::new(across)]);
        let g = group.id;
        d.insert(Some(l), 0, group).unwrap();
        let upright = d.alloc_id();
        d.insert(Some(l), 1, Node::path(upright, open(&[(150.0, 0.0), (150.0, 300.0)]), Appearance::default_art())).unwrap();
        let (sel, p) = (Selection::default(), paint());
        let mut cx = cx(&d, &sel, &p);
        let mut t = TrimTool::default();
        // Outside isolation mode the upright is highlighted; entering it drops that highlight.
        assert_eq!(hover(&mut t, &cx, 151.0, 50.0).len(), 1);
        cx.isolation = Some(g);
        assert!(shown(&t, &cx).is_empty(), "found before the group was isolated");
        // The upright is not trimmed, and still crosses the line inside the group.
        assert!(hover(&mut t, &cx, 151.0, 50.0).is_empty());
        let left = hover(&mut t, &cx, 50.0, 101.0);
        assert!(matches!(left.as_slice(), [e] if ends_are(*e, (0.0, 100.0), (150.0, 100.0))), "{left:?}");
    }

    #[test]
    fn a_path_whose_crossings_take_more_than_a_hover_may_shows_nothing_and_a_click_still_trims_it() {
        // 2,500 segments crossing each other's boxes: about 3,125,000 pairs, more than a twentieth
        // of the 60,000,000 a click may look at.
        let row = open(&(0..=2500).map(|i| (i as f64 * 0.1, 100.0)).collect::<Vec<_>>());
        let (d, _) = doc_of(vec![row]);
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TrimTool::default();
        assert!(hover(&mut t, &cx, 125.0, 100.0).is_empty());
        assert_eq!(click_removes(&d, &cx, 125.0, 100.0).len(), 1);
        let up = {
            t.pointer(&cx, &ev(PointerKind::Down, 125.0, 100.0));
            t.pointer(&cx, &ev(PointerKind::Up, 125.0, 100.0))
        };
        assert_eq!(up, vec![Action::Exec(COMMAND.into(), json!({"at": [125.0, 100.0], "tolerance": 4.0}))]);
    }

    #[test]
    fn a_small_drag_shows_what_it_would_remove_and_a_large_one_shows_nothing() {
        let ellipse = shapes::ellipse(Rect::new(100.0, 100.0, 300.0, 200.0));
        let (d, _) = doc_of(vec![ellipse, open(&[(50.0, 150.0), (350.0, 150.0)])]);
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TrimTool::default();
        // A rectangle across the ellipse's top: its upper half.
        t.pointer(&cx, &ev(PointerKind::Down, 190.0, 90.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 210.0, 110.0));
        assert!(matches!(shown(&t, &cx).as_slice(), [e] if ends_are(*e, (100.0, 150.0), (300.0, 150.0))));
        t.pointer(&cx, &ev(PointerKind::Up, 210.0, 110.0));
        assert!(shown(&t, &cx).is_empty(), "the release drops the preview");
        // An Alt line through the ellipse's left side and the line's left end.
        let alt = Mods { alt: true, ..Mods::default() };
        t.pointer(&cx, &ev(PointerKind::Down, 60.0, 140.0).with_mods(alt));
        t.pointer(&cx, &ev(PointerKind::Drag, 110.0, 160.0));
        assert_eq!(shown(&t, &cx).len(), 2, "the line's end and the ellipse's lower half");
        t.key(&cx, ToolKey::Escape, Mods::default());
        assert!(shown(&t, &cx).is_empty());

        // A rectangle over more paths than the preview's limit shows nothing.
        let many: Vec<PathData> = (0..=PREVIEW.paths).map(|i| open(&[(0.0, i as f64 * 2.0), (100.0, i as f64 * 2.0)])).collect();
        let (d, _) = doc_of(many);
        let cx = crate::testutil::cx(&d, &sel, &p);
        t.pointer(&cx, &ev(PointerKind::Down, 50.0, -10.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 60.0, 500.0));
        assert!(shown(&t, &cx).is_empty());
        assert!(matches!(t.overlays(&cx).as_slice(), [Overlay::Marquee(_)]));
        // What lies under a still pointer changes when the view pans: the canvas hovers again.
        assert!(t.wants_rehover());
    }

    #[test]
    fn crossings_kept_from_earlier_picks_still_count_against_the_budget() {
        // Three rows of 7,000 segments, apart: each costs 24,500,000 pairs of the 60,000,000 a
        // pick may look at, so a rectangle over all three is refused.
        let row = |y: f64| open(&(0..=7000).map(|i| (i as f64 * 0.01, y)).collect::<Vec<_>>());
        let (d, _) = doc_of(vec![row(0.0), row(10.0), row(20.0)]);
        let rect = Pick::Rect(Rect::new(-1.0, -1.0, 80.0, 21.0));
        assert_eq!(plan::plan(&d, None, &rect, Limits::COMMAND, &mut Stretches::default()), Err(plan::TooMuch::Crossings));
        // Hovering two of them keeps their crossings; the rectangle is refused all the same.
        let mut kept = Stretches::default();
        for y in [0.0, 10.0] {
            assert_eq!(plan::plan(&d, None, &Pick::At(Point::new(35.0, y), 1.0), Limits::COMMAND, &mut kept).map(|t| t.len()), Ok(1));
        }
        assert_eq!(plan::plan(&d, None, &rect, Limits::COMMAND, &mut kept), Err(plan::TooMuch::Crossings));
        // A rectangle over two is not.
        let two = Pick::Rect(Rect::new(-1.0, -1.0, 80.0, 11.0));
        assert_eq!(plan::plan(&d, None, &two, Limits::COMMAND, &mut kept).map(|t| t.len()), Ok(2));

        // A drag whose crossings take too much shows nothing until the next press, even over less.
        // Rows of 1,500 segments cost 1,125,000 pairs each, so a preview has room for two.
        let row = |y: f64| open(&(0..=1500).map(|i| (i as f64 * 0.05, y)).collect::<Vec<_>>());
        let (d, _) = doc_of(vec![row(0.0), row(10.0), row(20.0)]);
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TrimTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, -1.0, -1.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 80.0, 21.0));
        assert!(shown(&t, &cx).is_empty());
        t.pointer(&cx, &ev(PointerKind::Drag, 80.0, 5.0));
        assert!(shown(&t, &cx).is_empty());
        t.pointer(&cx, &ev(PointerKind::Up, 80.0, 5.0));
        t.pointer(&cx, &ev(PointerKind::Down, -1.0, -1.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 80.0, 5.0));
        assert_eq!(shown(&t, &cx).len(), 1);
    }
}
