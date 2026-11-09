//! Extend Path tool (Path Precision Tools): drag an end of an open path (selected or not) to carry
//! the path on the way it runs there, or back along itself to shorten it
//! (`addon.pathtools.extend`, previewed while dragging, one undo step). Over an end, a dashed line
//! shows how it would go on. E cycles the mode (straight, the last curve stretched, an arc of the
//! circle the end curves along); Shift steps the length by the keyboard increment; back over the
//! end, the length snaps to none.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{Point, SubPath};
use vectorcraft_pathops::addons::pathtools::{ExtendMode, extend_length_to, extend_open};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "extendPath";
/// The undo step's name.
const LABEL: &str = "Extend Path";
/// How far ahead (px) the hover preview shows an end going on.
const PREVIEW_PX: f64 = 60.0;
const GREY: [u8; 3] = [0x80, 0x80, 0x80];
const BLUE: [u8; 3] = [0x4f, 0x9d, 0xff];

/// An end of an open subpath.
#[derive(Clone, Debug, PartialEq)]
struct End {
    id: NodeId,
    subpath: usize,
    start: bool,
    p: Point,
    /// The subpath as it was.
    sp: SubPath,
}

#[derive(Default)]
pub struct ExtendPathTool {
    mode: Mode,
    hover: Option<End>,
    /// The end being dragged and the length the drag asks for.
    drag: Option<(End, f64)>,
}

/// [`ExtendMode`], with the default the tool starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mode(ExtendMode);

impl Default for Mode {
    fn default() -> Self {
        Mode(ExtendMode::Straight)
    }
}

/// The end of an editable open path nearest `p` within the point tolerance.
fn end_at(cx: &ToolContext, p: Point) -> Option<End> {
    let tol = cx.point_tol();
    let mut best: Option<(f64, NodeId, usize, bool, Point)> = None;
    cx.doc.walk(|n| {
        let NodeKind::Path { path, guide: false, .. } = &n.kind else { return };
        for (si, sp) in path.subpaths.iter().enumerate().filter(|(_, sp)| !sp.closed && sp.anchors.len() >= 2) {
            for (start, a) in [(true, sp.anchors.first()), (false, sp.anchors.last())] {
                let Some(a) = a else { continue };
                let d = a.p.distance(p);
                if d <= tol && best.is_none_or(|b| d < b.0) {
                    best = Some((d, n.id, si, start, a.p));
                }
            }
        }
    });
    let (_, id, subpath, start, at) = best.filter(|b| cx.doc.is_editable(b.1))?;
    let sp = cx.doc.node(id)?.path_data()?.subpaths.get(subpath)?.clone();
    Some(End { id, subpath, start, p: at, sp })
}

impl ExtendPathTool {
    /// The length a drag to `p` asks for: back over the end none, with Shift in steps of the
    /// keyboard increment.
    fn length_at(&self, cx: &ToolContext, end: &End, p: Point, mods: Mods) -> f64 {
        if p.distance(end.p) <= cx.point_tol() {
            return 0.0;
        }
        let len = extend_length_to(&end.sp, end.start, p, self.mode.0).unwrap_or(0.0);
        let step = cx.keyboard_increment;
        if mods.shift && step > 0.0 && step.is_finite() { (len / step).round() * step } else { len }
    }

    fn params(&self, end: &End, length: f64) -> Value {
        json!({
            "id": end.id.0,
            "subpath": end.subpath,
            "end": if end.start { "start" } else { "end" },
            "length": length,
            "mode": self.mode.0.id(),
        })
    }
}

/// The length of `sp` (pt).
fn length_of(sp: &SubPath) -> f64 {
    (0..sp.segment_count()).map(|i| kurbo::ParamCurveArclen::arclen(&sp.segment(i), 1e-6)).sum()
}

impl Tool for ExtendPathTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Move => {
                self.hover = end_at(cx, ev.pos);
                vec![]
            }
            PointerKind::Down => {
                let Some(end) = end_at(cx, ev.pos) else { return vec![] };
                self.hover = None;
                self.drag = Some((end, 0.0));
                vec![Action::Begin(LABEL.into())]
            }
            PointerKind::Drag | PointerKind::Up => {
                let Some((end, _)) = self.drag.take() else { return vec![] };
                let length = self.length_at(cx, &end, ev.pos, ev.mods);
                let preview = Action::Preview("addon.pathtools.extend".into(), self.params(&end, length));
                if ev.kind == PointerKind::Drag {
                    self.drag = Some((end, length));
                    return vec![preview];
                }
                // Back where it started: nothing to keep.
                if length == 0.0 { vec![Action::Cancel] } else { vec![preview, Action::Commit] }
            }
            PointerKind::DoubleClick => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape if self.drag.is_some() => {
                self.drag = None;
                vec![Action::Cancel]
            }
            ToolKey::Letter('e') => {
                let all = ExtendMode::ALL;
                let i = all.iter().position(|m| *m == self.mode.0).unwrap_or(0);
                self.mode = Mode(all.get((i + 1) % all.len()).copied().unwrap_or(ExtendMode::Straight));
                vec![]
            }
            _ => vec![],
        }
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        key == ToolKey::Letter('e') || (self.drag.is_some() && key == ToolKey::Escape)
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let size = 7.0;
        if let Some((end, length)) = &self.drag {
            let total = length_of(&end.sp) + length;
            return vec![
                Overlay::Anchor { p: end.p, color: BLUE, filled: false, size },
                Overlay::Measure {
                    p: end.p + kurbo::Vec2::new(cx.tol(12.0), cx.tol(12.0)),
                    text: format!("Length: {}\nΔ: {}", cx.len(total), cx.len(*length)),
                },
            ];
        }
        let Some(end) = &self.hover else { return vec![] };
        let mut out = vec![Overlay::Anchor { p: end.p, color: BLUE, filled: true, size }];
        // How the end would go on.
        if let Some(ext) = extend_open(&end.sp, end.start, cx.tol(PREVIEW_PX), self.mode.0) {
            let (k, n) = (end.sp.anchors.len(), ext.anchors.len());
            // From the old end on; a stretched last curve (no anchor added) shows whole.
            let from = if n > k { k - 1 } else { n.saturating_sub(2) };
            let tail = if end.start { ext.anchors.get(..n - from) } else { ext.anchors.get(from..) };
            if let Some(tail) = tail {
                let mut bp = vectorcraft_geom::BezPath::new();
                SubPath::new(tail.to_vec(), false).to_bezpath_into(&mut bp);
                out.push(Overlay::Path { path: bp, color: GREY, width: 1.0, dashed: true });
            }
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::ExtendPath
    }

    fn options(&self) -> Value {
        json!({ "mode": self.mode.0.id() })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        if key == "mode"
            && let Some(m) = v.as_str().and_then(ExtendMode::parse)
        {
            self.mode = Mode(m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Document, Node, Selection};
    use vectorcraft_geom::PathData;

    /// A document with an open line from (100, 100) to (200, 100).
    fn doc_with_line() -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let sp = SubPath::polyline(&[Point::new(100.0, 100.0), Point::new(200.0, 100.0)], false);
        d.insert(Some(l), 0, Node::path(id, PathData::single(sp), Appearance::default_art())).unwrap();
        (d, id)
    }

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    #[test]
    fn dragging_an_end_previews_the_extension_and_commits() {
        let (d, id) = doc_with_line();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = ExtendPathTool::default();
        // Hovering the end shows it and how it would go on.
        assert!(t.pointer(&cx, &ev(PointerKind::Move, 201.0, 101.0)).is_empty());
        let ov = t.overlays(&cx);
        assert!(matches!(ov.as_slice(), [Overlay::Anchor { filled: true, .. }, Overlay::Path { dashed: true, .. }]), "{ov:?}");
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 201.0, 101.0)), vec![Action::Begin(LABEL.into())]);
        let drag = t.pointer(&cx, &ev(PointerKind::Drag, 240.0, 105.0));
        let Some(Action::Preview(cmd, params)) = drag.first() else { panic!("{drag:?}") };
        assert_eq!(cmd, "addon.pathtools.extend");
        assert_eq!((params["id"].clone(), params["end"].clone(), params["mode"].clone()), (json!(id.0), json!("end"), json!("straight")));
        assert!((params["length"].as_f64().unwrap() - 40.0).abs() < 1e-6);
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text.contains("Length: 140"))));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 160.0, 100.0));
        assert!(matches!(up.as_slice(), [Action::Preview(_, p), Action::Commit] if (p["length"].as_f64().unwrap() + 40.0).abs() < 1e-6), "{up:?}");
        // The start end too, and Shift steps the length.
        t.pointer(&cx, &ev(PointerKind::Down, 100.0, 100.0));
        let mut shifted = ev(PointerKind::Drag, 87.4, 100.0);
        shifted.mods.shift = true;
        let drag = t.pointer(&cx, &shifted);
        let Some(Action::Preview(_, params)) = drag.first() else { panic!("{drag:?}") };
        assert_eq!((params["end"].clone(), params["length"].clone()), (json!("start"), json!(13.0)));
        // Back on the end: nothing to keep.
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 100.0, 100.0)), vec![Action::Cancel]);
    }

    #[test]
    fn away_from_ends_nothing_happens_and_e_cycles_the_mode() {
        let (d, _) = doc_with_line();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = ExtendPathTool::default();
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 150.0, 100.0)).is_empty(), "the middle of the path");
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 150.0, 100.0)).is_empty());
        assert!(t.overlays(&cx).is_empty());
        assert!(t.claims_key(&cx, ToolKey::Letter('e')));
        for want in ["bezier", "arc", "straight"] {
            t.key(&cx, ToolKey::Letter('e'), Mods::default());
            assert_eq!(t.options()["mode"], json!(want));
        }
        t.set_option("mode", &json!("arc"));
        t.set_option("mode", &json!("spiral"));
        assert_eq!(t.options()["mode"], json!("arc"));
        assert_eq!(t.cursor(&cx, Point::ZERO, Mods::default()), Cursor::ExtendPath);
        // A closed path has no ends.
        let (d, _) = doc_with_rect();
        let cx = crate::testutil::cx(&d, &sel, &p);
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 100.0, 100.0)).is_empty());
    }
}
