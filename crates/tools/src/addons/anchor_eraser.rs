//! Anchor Eraser Brush (Path Precision Tools): paint over selected paths and the anchors the round
//! brush passes over go, their neighbours' handles refitted, wherever the outline can do without
//! them within the tolerance (`addon.pathtools.erase`, previewed while painting, one undo step per
//! stroke). Alt removes every anchor under the brush that can go whatever the tolerance; Cmd/Ctrl
//! deletes them without refitting. `[` and `]` make the brush smaller and larger. With Pressure on,
//! a pen's pressure scales the brush.

use serde_json::{Value, json};
use vectorcraft_geom::Point;

use crate::{Action, Cursor, MAX_STROKE_SAMPLES, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey, previews_at};

/// The tool's id.
pub const ID: &str = "anchorEraserBrush";
/// The brush's diameter on screen (px) by default, and its range.
pub const SIZE: f64 = 20.0;
pub const SIZE_RANGE: (f64, f64) = (1.0, 2500.0);
/// The tolerance by default and its range.
pub const TOLERANCE: f64 = 10.0;
pub const TOLERANCE_RANGE: (f64, f64) = (1.0, 100.0);
/// The undo step's name.
const LABEL: &str = "Anchor Eraser Brush";
/// The brush's outline colour.
const RED: [u8; 3] = [0xe0, 0x30, 0x30];

pub struct AnchorEraserTool {
    /// Diameter on screen (px).
    size: f64,
    tolerance: f64,
    pressure: bool,
    /// The stroke under way: its points and their pen pressure.
    points: Vec<(Point, f32)>,
    /// The modifiers the stroke started with: Alt ignores the tolerance, Cmd/Ctrl skips refitting.
    mods: Mods,
    painting: bool,
    /// Where the brush is.
    at: Option<Point>,
}

impl Default for AnchorEraserTool {
    fn default() -> Self {
        Self { size: SIZE, tolerance: TOLERANCE, pressure: false, points: vec![], mods: Mods::default(), painting: false, at: None }
    }
}

impl AnchorEraserTool {
    /// `addon.pathtools.erase`'s params for the stroke so far.
    fn params(&self, cx: &ToolContext) -> Value {
        let points: Vec<Value> = self
            .points
            .iter()
            .map(|(p, f)| if self.pressure && *f < 1.0 { json!([p.x, p.y, (f64::from(*f) * 1000.0).round() / 1000.0]) } else { json!([p.x, p.y]) })
            .collect();
        json!({
            "points": points,
            "diameter": cx.tol(self.size).clamp(0.1, SIZE_RANGE.1),
            "tolerance": self.tolerance,
            "ignoreTolerance": self.mods.alt,
            "plain": self.mods.cmd,
        })
    }
}

impl Tool for AnchorEraserTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.painting
    }

    fn wants_samples(&self) -> bool {
        true
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        self.at = Some(ev.pos);
        match ev.kind {
            // Nothing selected: nothing to erase from.
            PointerKind::Down if !cx.selection.objects.is_empty() => {
                self.points = vec![(ev.pos, ev.pressure)];
                self.mods = ev.mods;
                self.painting = true;
                vec![Action::Begin(LABEL.into()), Action::Preview("addon.pathtools.erase".into(), self.params(cx))]
            }
            PointerKind::Drag if self.painting => {
                if self.points.len() >= MAX_STROKE_SAMPLES {
                    return vec![];
                }
                self.points.push((ev.pos, ev.pressure));
                if previews_at(self.points.len()) { vec![Action::Preview("addon.pathtools.erase".into(), self.params(cx))] } else { vec![] }
            }
            PointerKind::Up if self.painting => {
                if self.points.len() < MAX_STROKE_SAMPLES && self.points.last().is_none_or(|(p, _)| *p != ev.pos) {
                    self.points.push((ev.pos, ev.pressure));
                }
                self.painting = false;
                let out = vec![Action::Preview("addon.pathtools.erase".into(), self.params(cx)), Action::Commit];
                self.points.clear();
                out
            }
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape if self.painting => {
                self.painting = false;
                self.points.clear();
                vec![Action::Cancel]
            }
            ToolKey::BracketLeft | ToolKey::BracketRight => {
                let step = if key == ToolKey::BracketLeft { 1.0 / 1.25 } else { 1.25 };
                self.size = (self.size * step).round().clamp(SIZE_RANGE.0, SIZE_RANGE.1);
                vec![]
            }
            _ => vec![],
        }
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::BracketLeft | ToolKey::BracketRight) || (self.painting && key == ToolKey::Escape)
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.at = None;
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some(at) = self.at else { return vec![] };
        let r = cx.tol(self.size / 2.0);
        let circle = kurbo::Shape::to_path(&kurbo::Circle::new(at, r), 0.1 * r.max(1e-6));
        vec![Overlay::Path { path: circle, color: RED, width: 1.0, dashed: false }]
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::AnchorEraser
    }

    fn options(&self) -> Value {
        json!({ "size": self.size, "tolerance": self.tolerance, "pressure": self.pressure })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        let num = |v: &Value, (lo, hi): (f64, f64)| v.as_f64().filter(|x| x.is_finite()).map(|x| x.clamp(lo, hi));
        match key {
            "size" => self.size = num(v, SIZE_RANGE).unwrap_or(self.size),
            "tolerance" => self.tolerance = num(v, TOLERANCE_RANGE).unwrap_or(self.tolerance),
            "pressure" => self.pressure = v.as_bool().unwrap_or(self.pressure),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    #[test]
    fn a_stroke_previews_the_erase_and_commits_once() {
        let (d, id) = doc_with_rect();
        let mut sel = Selection::default();
        sel.set([id]);
        let p = paint();
        let cx = cx(&d, &sel, &p);
        let mut t = AnchorEraserTool::default();
        let down = t.pointer(&cx, &ev(PointerKind::Down, 100.0, 100.0));
        assert_eq!(down.first(), Some(&Action::Begin(LABEL.into())));
        assert!(t.busy());
        let drag = t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 100.0));
        let Some(Action::Preview(cmd, params)) = drag.first() else { panic!("{drag:?}") };
        assert_eq!(cmd, "addon.pathtools.erase");
        assert_eq!(params["points"], json!([[100.0, 100.0], [150.0, 100.0]]));
        assert_eq!((params["diameter"].clone(), params["tolerance"].clone()), (json!(20.0), json!(10.0)));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 200.0, 100.0));
        assert!(matches!(up.as_slice(), [Action::Preview(..), Action::Commit]), "{up:?}");
        assert!(!t.busy());
        // Alt ignores the tolerance; Cmd deletes without refitting; Escape cancels.
        let alt = Mods { alt: true, cmd: true, ..Mods::default() };
        let down = t.pointer(&cx, &ev(PointerKind::Down, 100.0, 100.0).with_mods(alt));
        let Some(Action::Preview(_, params)) = down.get(1) else { panic!("{down:?}") };
        assert_eq!((params["ignoreTolerance"].clone(), params["plain"].clone()), (json!(true), json!(true)));
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 100.0, 100.0)).is_empty());
    }

    #[test]
    fn nothing_selected_paints_nothing() {
        let (d, _) = doc_with_rect();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = AnchorEraserTool::default();
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 100.0, 100.0)).is_empty());
        assert!(t.pointer(&cx, &ev(PointerKind::Drag, 120.0, 100.0)).is_empty());
        assert!(!t.busy());
    }

    #[test]
    fn options_size_the_brush_and_pressure_scales_it() {
        let (d, id) = doc_with_rect();
        let mut sel = Selection::default();
        sel.set([id]);
        let p = paint();
        let mut cx = cx(&d, &sel, &p);
        cx.zoom = 2.0;
        let mut t = AnchorEraserTool::default();
        t.set_option("size", &json!(40));
        t.set_option("tolerance", &json!(500));
        t.set_option("pressure", &json!(true));
        assert_eq!(t.options(), json!({"size": 40.0, "tolerance": 100.0, "pressure": true}));
        let mut down = ev(PointerKind::Down, 10.0, 10.0);
        down.pressure = 0.5;
        let a = t.pointer(&cx, &down);
        let Some(Action::Preview(_, params)) = a.get(1) else { panic!("{a:?}") };
        assert_eq!(params["diameter"], json!(20.0), "40 px at 200%");
        assert_eq!(params["points"], json!([[10.0, 10.0, 0.5]]));
        // The brush shows round the pointer, and `]` makes it larger.
        assert!(matches!(t.overlays(&cx).as_slice(), [Overlay::Path { .. }]));
        t.key(&cx, ToolKey::BracketRight, Mods::default());
        assert_eq!(t.options()["size"], json!(50.0));
        assert_eq!(t.cursor(&cx, Point::ZERO, Mods::default()), Cursor::AnchorEraser);
    }
}
