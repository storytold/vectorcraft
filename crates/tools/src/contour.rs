//! The Contour tool: drag away from a selected shape's edge to grow a live copy of it, or toward the
//! inside to shrink it (`object.contour`). The copy is the original's outline offset by the distance
//! the pointer has moved from the edge, previewed while the button is down and kept as one undo step
//! on release. Nothing is baked: the copy carries a live Offset Path effect, so it keeps being
//! editable (a second drag on a selected Contour changes its distance instead of nesting another),
//! and Expand Appearance bakes it. While a Contour is selected the tool draws the outline the
//! shape had before as a dashed guide.
//!
//! The distance is how far the pointer has moved from the shape's edge since the press, so a press
//! anywhere changes nothing until the pointer moves, and a Contour that already stands at some
//! distance continues from there. Shift rounds the distance to whole points.

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{BezPath, FillRule, Point, hit};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The live effect a Contour is made of (the one `object.contour` applies).
const EFFECT: &str = "path.offsetPath";
/// The largest distance, in points, `object.contour` accepts.
const MAX_OFFSET: f64 = 1e5;
/// The guide showing the shape a Contour was made from.
const GUIDE: [u8; 3] = [0x1e, 0x88, 0xe5];

/// A selected shape's outline, in the document.
struct Base {
    path: BezPath,
    rule: FillRule,
}

fn base_of(n: &Node) -> Option<Base> {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => Some(Base { path: path.to_bezpath(), rule: *rule }),
        NodeKind::Compound { children, rule } => {
            let mut path = BezPath::new();
            for c in children {
                if let Some(p) = c.path_data() {
                    path.extend(p.to_bezpath().elements().iter().copied());
                }
            }
            Some(Base { path, rule: *rule })
        }
        _ => None,
    }
}

/// The Offset Path distance of a Contour (a shape that carries the effect).
fn contour_offset(n: &Node) -> Option<f64> {
    let e = n.appearance.effects.iter().rev().find(|e| e.id == EFFECT)?;
    Some(e.params.get("offset").and_then(Value::as_f64).unwrap_or(0.0))
}

/// How far `p` is from the nearest of `bases`' edges: positive outside, negative inside.
fn signed_distance(bases: &[Base], p: Point) -> Option<f64> {
    let mut nearest = f64::INFINITY;
    let mut inside = false;
    for b in bases {
        nearest = nearest.min(hit::distance_to_outline(&b.path, p));
        inside |= hit::fill_contains(&b.path, b.rule, p);
    }
    nearest.is_finite().then_some(if inside { -nearest } else { nearest })
}

/// A press in progress.
struct Drag {
    bases: Vec<Base>,
    /// Where the pointer stood against the edge at the press.
    pressed_at: f64,
    /// The Contour's distance at the press (0 for a shape that is not one yet).
    start: f64,
    /// The distance last previewed.
    last: Option<f64>,
    /// Where the pointer is, for the distance label.
    pos: Point,
}

#[derive(Default)]
pub struct ContourTool {
    drag: Option<Drag>,
    corners: Corners,
    miter_limit: Option<f64>,
}

/// How a Contour treats the corners of the shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Corners {
    /// Pointed (mitered) corners.
    #[default]
    Sharp,
    Round,
    /// Corners cut off flat.
    Square,
}

impl Corners {
    fn id(self) -> &'static str {
        match self {
            Corners::Sharp => "sharp",
            Corners::Round => "round",
            Corners::Square => "square",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Corners::Sharp, Corners::Round, Corners::Square].into_iter().find(|c| c.id().eq_ignore_ascii_case(s))
    }
}

impl ContourTool {
    fn params(&self, offset: f64) -> Value {
        let mut p = json!({"offset": offset, "corners": self.corners.id()});
        if let Some(m) = self.miter_limit {
            p["miterLimit"] = json!(m);
        }
        p
    }
}

/// The selected shapes a Contour can be made of.
fn selected_shapes<'a>(cx: &'a ToolContext) -> Vec<&'a Node> {
    cx.selection
        .objects
        .iter()
        .filter_map(|id| cx.doc.node(*id))
        .filter(|n| matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }))
        .collect()
}

fn round_distance(v: f64, mods: Mods) -> f64 {
    let v = if mods.shift { v.round() } else { (v * 100.0).round() / 100.0 };
    v.clamp(-MAX_OFFSET, MAX_OFFSET)
}

impl Tool for ContourTool {
    fn id(&self) -> &'static str {
        "contour"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                let shapes = selected_shapes(cx);
                let start = shapes.first().and_then(|n| contour_offset(n)).unwrap_or(0.0);
                let bases: Vec<Base> = shapes.into_iter().filter_map(base_of).collect();
                let Some(pressed_at) = signed_distance(&bases, ev.pos) else { return vec![] };
                self.drag = Some(Drag { bases, pressed_at, start, last: None, pos: ev.pos });
                vec![Action::Begin("Contour".into())]
            }
            PointerKind::Drag => {
                let mut params = self.params(0.0);
                let Some(drag) = self.drag.as_mut() else { return vec![] };
                drag.pos = ev.pos;
                let Some(now) = signed_distance(&drag.bases, ev.pos) else { return vec![] };
                let offset = round_distance(drag.start + now - drag.pressed_at, ev.mods);
                if !offset.is_finite() || drag.last == Some(offset) {
                    return vec![];
                }
                drag.last = Some(offset);
                params["offset"] = json!(offset);
                vec![Action::Preview("object.contour".into(), params)]
            }
            PointerKind::Up => match self.drag.take() {
                Some(d) if d.last.is_some() => vec![Action::Commit],
                Some(_) => vec![Action::Cancel],
                None => vec![],
            },
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape if self.drag.take().is_some() => vec![Action::Cancel],
            _ => vec![],
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        // The outline each selected Contour was made from.
        for n in selected_shapes(cx) {
            if contour_offset(n).is_none() {
                continue;
            }
            if let Some(b) = base_of(n) {
                // A white halo under the dashes, so the guide shows over any fill.
                out.push(Overlay::Path { path: b.path.clone(), color: [255, 255, 255], width: 3.0, dashed: false });
                out.push(Overlay::Path { path: b.path, color: GUIDE, width: 1.0, dashed: true });
            }
        }
        if let Some(d) = &self.drag
            && let Some(offset) = d.last
        {
            out.push(Overlay::Measure { p: d.pos, text: format!("{offset:+.2} pt") });
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }

    fn options(&self) -> Value {
        json!({"corners": self.corners.id(), "miterLimit": self.miter_limit.unwrap_or(4.0)})
    }

    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "corners" => {
                if let Some(c) = value.as_str().and_then(Corners::parse) {
                    self.corners = c;
                }
            }
            "miterLimit" => {
                if let Some(m) = value.as_f64().filter(|m| m.is_finite()) {
                    self.miter_limit = Some(m.clamp(1.0, 500.0));
                }
            }
            _ => {}
        }
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }
}

/// The Contour tool, for [`crate::create`].
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    (id == "contour").then(|| Box::new(ContourTool::default()) as Box<dyn Tool>)
}
