//! Mirror & Cut, Line Cut and Rectangle Cut: drag across the selected art.
//!
//! The cut previews live while dragging (`path.mirrorCut`, `path.lineCut` or `path.rectCut`
//! re-applied on the snapshot) and the release keeps it as one undo step; Escape cancels.
//!
//! - Mirror & Cut: the drag draws the axis (Shift: 45° steps from the constrain angle). With the
//!   `axis` option vertical or horizontal the axis follows the pointer instead, and a click places
//!   it. `keep` names the side kept; Alt on release keeps the other one.
//! - Line Cut: the drag draws the cut line (Shift: 45° steps).
//! - Rectangle Cut: the drag draws the rectangle (Shift: a square, Alt: from its centre).

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Vec2, constrain_angle_from};

use crate::draw2::FEEDBACK;
use crate::shape::drag_rect;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// Create a cutting tool by id (None = not one of ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    matches!(id, "mirrorCut" | "lineCut" | "rectCut").then(|| Box::new(CutTool::new(id)) as Box<dyn Tool>)
}

pub struct CutTool {
    id: &'static str,
    start: Option<Point>,
    end: Point,
    mods: Mods,
    began: bool,
    /// Mirror & Cut: "free", "vertical" or "horizontal".
    pub axis: &'static str,
    /// Mirror & Cut: the side kept, "left", "right", "top" or "bottom" (a side across the axis
    /// stands for its counterpart: left for top, right for bottom).
    pub keep: &'static str,
}

/// Drags shorter than this many screen pixels don't cut.
const MIN_DRAG_PX: f64 = 3.0;

impl CutTool {
    pub fn new(id: &str) -> Self {
        let id = match id {
            "mirrorCut" => "mirrorCut",
            "lineCut" => "lineCut",
            _ => "rectCut",
        };
        Self { id, start: None, end: Point::ZERO, mods: Mods::default(), began: false, axis: "free", keep: "left" }
    }

    fn label(&self) -> &'static str {
        match self.id {
            "mirrorCut" => "Mirror & Cut",
            "lineCut" => "Line Cut",
            _ => "Rectangle Cut",
        }
    }

    /// A constrained Mirror & Cut axis (it follows the pointer; a click places it).
    fn constrained(&self) -> bool {
        self.id == "mirrorCut" && self.axis != "free"
    }

    /// The drag's end point: Shift snaps lines to 45° steps.
    fn end_point(&self, cx: &ToolContext, start: Point, p: Point, m: Mods) -> Point {
        if m.shift && self.id != "rectCut" { start + constrain_angle_from(p - start, 45.0, cx.constrain_angle) } else { p }
    }

    /// The side Mirror & Cut keeps for an axis running along `d`: the `keep` option named for the
    /// axis's orientation, the other side with Alt.
    fn keep_side(&self, d: Vec2, alt: bool) -> &'static str {
        let upright = match self.axis {
            "vertical" => true,
            "horizontal" => false,
            _ => d.y.abs() >= d.x.abs(),
        };
        let first = matches!(self.keep, "left" | "top");
        match (upright, first != alt) {
            (true, true) => "left",
            (true, false) => "right",
            (false, true) => "top",
            (false, false) => "bottom",
        }
    }

    /// The command for the current drag, `None` while it is too short to cut.
    pub fn command(&self, cx: &ToolContext) -> Option<(&'static str, Value)> {
        let start = self.start?;
        let (a, b) = (start, self.end);
        let short = a.distance(b) < cx.tol(MIN_DRAG_PX);
        match self.id {
            "mirrorCut" if self.constrained() => {
                Some(("path.mirrorCut", json!({"axis": self.axis, "from": [b.x, b.y], "keep": self.keep_side(Vec2::ZERO, self.mods.alt)})))
            }
            _ if short => None,
            "mirrorCut" => {
                Some(("path.mirrorCut", json!({"axis": "free", "from": [a.x, a.y], "to": [b.x, b.y], "keep": self.keep_side(b - a, self.mods.alt)})))
            }
            "lineCut" => Some(("path.lineCut", json!({"from": [a.x, a.y], "to": [b.x, b.y]}))),
            _ => {
                let r = drag_rect(a, b, self.mods);
                (r.width() > 0.0 && r.height() > 0.0).then(|| ("path.rectCut", json!({"rect": [r.x0, r.y0, r.width(), r.height()]})))
            }
        }
    }

    fn reset(&mut self) {
        self.start = None;
        self.began = false;
    }
}

impl Tool for CutTool {
    fn id(&self) -> &'static str {
        self.id
    }
    fn busy(&self) -> bool {
        self.start.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                // The tools cut the selection: without one there is nothing to do.
                if cx.selection.is_empty() {
                    return vec![];
                }
                self.start = Some(ev.pos);
                self.end = ev.pos;
                self.mods = ev.mods;
                self.began = false;
                vec![]
            }
            PointerKind::Drag | PointerKind::Up => {
                let Some(start) = self.start else { return vec![] };
                self.end = self.end_point(cx, start, ev.pos, ev.mods);
                self.mods = ev.mods;
                let mut out = vec![];
                let cmd = self.command(cx);
                if let Some((cmd, p)) = cmd {
                    if !self.began {
                        self.began = true;
                        out.push(Action::Begin(self.label().into()));
                    }
                    out.push(Action::Preview(cmd.into(), p));
                }
                if ev.kind == PointerKind::Up {
                    if self.began {
                        out.push(Action::Commit);
                    }
                    self.reset();
                }
                out
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.start.is_some() {
            let began = self.began;
            self.reset();
            return if began { vec![Action::Cancel] } else { vec![] };
        }
        vec![]
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        let began = self.began;
        self.reset();
        if began { vec![Action::Commit] } else { vec![] }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some(a) = self.start else { return vec![] };
        let b = self.end;
        if self.id == "rectCut" {
            return vec![Overlay::Marquee(drag_rect(a, b, self.mods))];
        }
        // The axis and the cut line run across the art: draw them across the view.
        let d = match (self.id, self.axis) {
            ("mirrorCut", "vertical") => Vec2::new(0.0, 1.0),
            ("mirrorCut", "horizontal") => Vec2::new(1.0, 0.0),
            _ if a.distance(b) < cx.tol(MIN_DRAG_PX) => return vec![],
            _ => (b - a).normalize(),
        };
        let far = d * cx.tol(10_000.0);
        let mid = if self.constrained() { b } else { a };
        vec![Overlay::Line { a: mid - far, b: mid + far, color: FEEDBACK, dashed: true }]
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        match self.id {
            "mirrorCut" => json!({"axis": self.axis, "keep": self.keep}),
            _ => Value::Null,
        }
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match (key, v.as_str()) {
            ("axis", Some(a)) => {
                self.axis = match a {
                    "vertical" => "vertical",
                    "horizontal" => "horizontal",
                    "free" => "free",
                    _ => self.axis,
                }
            }
            ("keep", Some(k)) => {
                self.keep = match k {
                    "left" => "left",
                    "right" => "right",
                    "top" => "top",
                    "bottom" => "bottom",
                    _ => self.keep,
                }
            }
            _ => {}
        }
    }
    /// Mirror & Cut's `axis` and `keep`; the other cutting tools have no options.
    fn check_option(&self, key: &str, v: &Value) -> Result<(), String> {
        use crate::option_check as c;
        match (self.id, key) {
            ("mirrorCut", "axis") => c::one_of(key, v, &["vertical", "horizontal", "free"]),
            ("mirrorCut", "keep") => c::one_of(key, v, &["left", "right", "top", "bottom"]),
            _ => Err(c::unknown(key)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    fn drag(t: &mut CutTool, cx: &ToolContext, from: (f64, f64), to: (f64, f64), m: Mods) -> Vec<Action> {
        let mut out = t.pointer(cx, &PointerEvent::new(PointerKind::Down, from.0, from.1));
        out.extend(t.pointer(cx, &PointerEvent::new(PointerKind::Drag, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0).with_mods(m)));
        out.extend(t.pointer(cx, &PointerEvent::new(PointerKind::Up, to.0, to.1).with_mods(m)));
        out
    }

    #[test]
    fn line_cut_previews_and_commits_once() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = CutTool::new("lineCut");
        let a = drag(&mut t, &cx, (150.0, 50.0), (150.0, 250.0), Mods::default());
        assert_eq!(a[0], Action::Begin("Line Cut".into()));
        assert_eq!(a.iter().filter(|x| matches!(x, Action::Begin(_))).count(), 1);
        assert!(matches!(&a[a.len() - 2], Action::Preview(c, v) if c == "path.lineCut" && v["to"] == json!([150.0, 250.0])));
        assert_eq!(a.last(), Some(&Action::Commit));
        assert!(!t.busy());
    }

    #[test]
    fn needs_a_selection_and_a_drag() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut t = CutTool::new("rectCut");
        assert!(drag(&mut t, &cx(&d, &s, &p), (0.0, 0.0), (300.0, 300.0), Mods::default()).is_empty());
        let mut s = Selection::default();
        s.set([id]);
        let cx = cx(&d, &s, &p);
        assert!(drag(&mut t, &cx, (10.0, 10.0), (11.0, 11.0), Mods::default()).is_empty(), "a click doesn't cut");
        let a = drag(&mut t, &cx, (120.0, 120.0), (160.0, 140.0), Mods { shift: true, ..Default::default() });
        assert!(matches!(&a[a.len() - 2], Action::Preview(c, v) if c == "path.rectCut" && v["rect"] == json!([120.0, 120.0, 40.0, 40.0])));
    }

    #[test]
    fn mirror_cut_axis_options_and_alt() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = CutTool::new("mirrorCut");
        // A free, mostly horizontal axis keeps the top; Alt the bottom.
        let a = drag(&mut t, &cx, (50.0, 150.0), (250.0, 160.0), Mods::default());
        assert!(matches!(&a[a.len() - 2], Action::Preview(_, v) if v["axis"] == "free" && v["keep"] == "top"));
        let a = drag(&mut t, &cx, (50.0, 150.0), (250.0, 160.0), Mods { alt: true, ..Default::default() });
        assert!(matches!(&a[a.len() - 2], Action::Preview(_, v) if v["keep"] == "bottom"));
        // A vertical axis follows the pointer, and a click places it.
        t.set_option("axis", &json!("vertical"));
        t.set_option("keep", &json!("right"));
        assert_eq!(t.options(), json!({"axis": "vertical", "keep": "right"}));
        let mut a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 140.0, 150.0));
        a.extend(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 140.0, 150.0)));
        assert_eq!(a.len(), 3);
        assert!(matches!(&a[1], Action::Preview(_, v) if v["axis"] == "vertical" && v["from"] == json!([140.0, 150.0]) && v["keep"] == "right"));
        t.set_option("axis", &json!("diagonal"));
        assert_eq!(t.axis, "vertical");
    }

    #[test]
    fn escape_cancels_the_preview() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.set([id]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = CutTool::new("lineCut");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 50.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 250.0));
        assert_eq!(a.len(), 2);
        assert_eq!(t.overlays(&cx).len(), 1);
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert!(!t.busy() && t.overlays(&cx).is_empty());
    }
}
