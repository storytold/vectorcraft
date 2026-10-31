//! Slide Anchor Along Path (Path Precision Tools): drag an anchor, selected or not, and it runs
//! along the curve it sits on, the two segments beside it refitted so the outline stays
//! (`addon.pathtools.slideAnchor`, previewed while dragging, one undo step). Shift snaps to where
//! the path's tangent is at a multiple of 45° from the constrain angle. Ends of open paths don't
//! slide.

use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{Point, SubPath};
use vectorcraft_pathops::addons::pathtools::{slide_stops, slide_t_at};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "slideAnchor";
/// The undo step's name.
const LABEL: &str = "Slide Anchor Along Path";
const BLUE: [u8; 3] = [0x4f, 0x9d, 0xff];
const SIZE: f32 = 7.0;

/// An anchor that can slide.
#[derive(Clone, Debug, PartialEq)]
struct Grip {
    id: NodeId,
    subpath: usize,
    anchor: usize,
    p: Point,
    /// The subpath as it was.
    sp: SubPath,
}

#[derive(Default)]
pub struct SlideAnchorTool {
    hover: Option<Grip>,
    /// The anchor being dragged and how far along.
    drag: Option<(Grip, f64)>,
}

/// The anchor of an editable path nearest `p` within the point tolerance that can slide.
fn grip_at(cx: &ToolContext, p: Point) -> Option<Grip> {
    let (tol, editable) = (cx.point_tol(), cx.doc.editable());
    let mut best: Option<(f64, NodeId, usize, usize, Point)> = None;
    cx.doc.walk(|n| {
        let NodeKind::Path { path, guide: false, .. } = &n.kind else { return };
        for (si, ai, a) in path.anchors() {
            let d = a.p.distance(p);
            let open_end = path.subpaths.get(si).is_some_and(|sp| !sp.closed && (ai == 0 || ai + 1 == sp.anchors.len()));
            if d <= tol && !open_end && best.is_none_or(|b| d < b.0) && editable(n.id) {
                best = Some((d, n.id, si, ai, a.p));
            }
        }
    });
    let (_, id, subpath, anchor, at) = best?;
    let sp = cx.doc.node(id)?.path_data()?.subpaths.get(subpath)?.clone();
    Some(Grip { id, subpath, anchor, p: at, sp })
}

/// How far along a drag to `p` takes `g`: with Shift, to the nearest place where the tangent is at
/// a multiple of 45° from the constrain angle.
fn slide_to(cx: &ToolContext, g: &Grip, p: Point, mods: Mods) -> f64 {
    let t = slide_t_at(&g.sp, g.anchor, p).unwrap_or(0.0);
    let stops = if mods.shift { slide_stops(&g.sp, g.anchor, cx.constrain_angle) } else { vec![] };
    stops.into_iter().min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs())).unwrap_or(t)
}

impl Tool for SlideAnchorTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Move => {
                self.hover = grip_at(cx, ev.pos);
                vec![]
            }
            PointerKind::Down => {
                let Some(g) = grip_at(cx, ev.pos) else { return vec![] };
                self.hover = None;
                self.drag = Some((g, 0.0));
                vec![Action::Begin(LABEL.into())]
            }
            PointerKind::Drag | PointerKind::Up => {
                let Some((g, _)) = self.drag.take() else { return vec![] };
                let t = slide_to(cx, &g, ev.pos, ev.mods);
                let params = json!({ "id": g.id.0, "subpath": g.subpath, "anchor": g.anchor, "t": t });
                let preview = Action::Preview("addon.pathtools.slideAnchor".into(), params);
                if ev.kind == PointerKind::Drag {
                    self.drag = Some((g, t));
                    return vec![preview];
                }
                // Back where it started: nothing to keep.
                if t == 0.0 { vec![Action::Cancel] } else { vec![preview, Action::Commit] }
            }
            PointerKind::DoubleClick => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.drag.take().is_some() { vec![Action::Cancel] } else { vec![] }
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        self.drag.is_some() && key == ToolKey::Escape
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        let anchor = |p, filled| Overlay::Anchor { p, color: BLUE, filled, size: SIZE };
        match (&self.drag, &self.hover) {
            (Some((g, _)), _) => vec![anchor(g.p, false)],
            (None, Some(g)) => vec![anchor(g.p, true)],
            (None, None) => vec![],
        }
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::SlideAnchor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Document, Node, Selection};
    use vectorcraft_geom::PathData;

    /// A document with an open zigzag (100, 100), (200, 100), (300, 100) and a circle-ish curve.
    fn doc_with_line() -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let pts = [Point::new(100.0, 100.0), Point::new(200.0, 100.0), Point::new(300.0, 100.0)];
        d.insert(Some(l), 0, Node::path(id, PathData::single(SubPath::polyline(&pts, false)), Appearance::default_art())).unwrap();
        (d, id)
    }

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    #[test]
    fn dragging_an_anchor_previews_the_slide_and_commits() {
        let (d, id) = doc_with_line();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = SlideAnchorTool::default();
        assert!(t.pointer(&cx, &ev(PointerKind::Move, 201.0, 101.0)).is_empty());
        assert!(matches!(t.overlays(&cx).as_slice(), [Overlay::Anchor { filled: true, .. }]));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 201.0, 101.0)), vec![Action::Begin(LABEL.into())]);
        assert!(t.busy());
        let drag = t.pointer(&cx, &ev(PointerKind::Drag, 250.0, 130.0));
        let Some(Action::Preview(cmd, params)) = drag.first() else { panic!("{drag:?}") };
        assert_eq!(cmd, "addon.pathtools.slideAnchor");
        assert_eq!((params["id"].clone(), params["subpath"].clone(), params["anchor"].clone()), (json!(id.0), json!(0), json!(1)));
        assert!((params["t"].as_f64().unwrap() - 0.5).abs() < 1e-6);
        // The other way along the path.
        let up = t.pointer(&cx, &ev(PointerKind::Up, 175.0, 90.0));
        assert!(matches!(up.as_slice(), [Action::Preview(_, p), Action::Commit] if p["t"].as_f64().unwrap() < 0.0), "{up:?}");
        assert!(!t.busy());
    }

    #[test]
    fn back_where_it_started_nothing_is_kept_and_escape_cancels() {
        let (d, _) = doc_with_line();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = SlideAnchorTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 100.0));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 200.0, 100.0)), vec![Action::Cancel]);
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 100.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 250.0, 100.0));
        assert!(t.claims_key(&cx, ToolKey::Escape));
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert!(!t.busy() && !t.claims_key(&cx, ToolKey::Escape));
    }

    #[test]
    fn ends_and_empty_canvas_do_nothing() {
        let (d, _) = doc_with_line();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = SlideAnchorTool::default();
        for (x, y) in [(100.0, 100.0), (300.0, 100.0), (150.0, 300.0)] {
            assert!(t.pointer(&cx, &ev(PointerKind::Down, x, y)).is_empty(), "({x}, {y})");
            assert!(t.pointer(&cx, &ev(PointerKind::Up, x, y)).is_empty());
        }
        assert_eq!(t.cursor(&cx, Point::ZERO, Mods::default()), Cursor::SlideAnchor);
    }

    #[test]
    fn shift_snaps_to_where_the_tangent_is_at_45_degrees() {
        // A smooth anchor on a curve turning from east to south through south-east.
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let a = |x: f64, y: f64, h: (f64, f64), o: (f64, f64)| vectorcraft_geom::Anchor {
            p: Point::new(x, y),
            h_in: Point::new(h.0, h.1),
            h_out: Point::new(o.0, o.1),
            kind: vectorcraft_geom::AnchorKind::Smooth,
        };
        let sp = SubPath::new(
            vec![
                a(0.0, 100.0, (0.0, 100.0), (0.0, 100.0)),
                a(100.0, 100.0, (100.0, 100.0), (155.0, 100.0)),
                a(200.0, 200.0, (200.0, 145.0), (200.0, 200.0)),
            ],
            false,
        );
        d.insert(Some(l), 0, Node::path(id, PathData::single(sp.clone()), Appearance::default_art())).unwrap();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = SlideAnchorTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 100.0, 100.0));
        let mut shifted = ev(PointerKind::Drag, 140.0, 105.0);
        shifted.mods.shift = true;
        let drag = t.pointer(&cx, &shifted);
        let Some(Action::Preview(_, params)) = drag.first() else { panic!("{drag:?}") };
        let at = params["t"].as_f64().unwrap();
        assert!((at - 0.5).abs() < 1e-6, "half way round a symmetric quarter turn: {at}");
    }
}
