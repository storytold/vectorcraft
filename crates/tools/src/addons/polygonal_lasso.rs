//! Polygonal Lasso selects anchor points inside a straight-edged loop, as the Lasso does inside a
//! freehand one. Each click sets a vertex, and a segment from the last vertex follows the pointer.
//! A double-click, Enter or a click on the first vertex closes the loop; Backspace or Delete removes
//! the last vertex and Escape drops the loop. Shift keeps the next segment to 45° steps (from the Constrain
//! Angle). Shift held at the first click adds to the selection and Alt subtracts from it.

use vectorcraft_geom::{Point, constrain_angle_from};

use crate::meshedit::MeshEdit;
use crate::xform::lasso_select;

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub const ID: &str = "polygonalLasso";
/// The most vertices a loop keeps; clicks past it are ignored.
pub(crate) const MAX_VERTICES: usize = 65_536;

#[derive(Default)]
pub struct PolygonalLassoTool {
    vertices: Vec<Point>,
    /// Where the segment from the last vertex ends: the pointer, constrained while Shift is held.
    hover: Option<Point>,
    /// Shift held at the first click: the loop adds to the selection.
    add: bool,
    /// Alt held at the first click: the loop subtracts from the selection.
    sub: bool,
    /// Whether the press in progress set a vertex (a dropped press takes it back).
    pressed_vertex: bool,
    /// The picked gradient mesh points (the tool's `meshPoints` option), as with the Lasso.
    mesh: MeshEdit,
}

impl PolygonalLassoTool {
    /// The next vertex for a pointer at `p`: with Shift and a vertex already set, at a 45° step
    /// from the last vertex.
    fn target(&self, cx: &ToolContext, p: Point, mods: Mods) -> Point {
        match self.vertices.last() {
            Some(&last) if mods.shift => last + constrain_angle_from(p - last, 45.0, cx.constrain_angle),
            _ => p,
        }
    }

    /// How near a vertex a click counts as a click on it.
    fn reach(cx: &ToolContext) -> f64 {
        cx.tol(5.0).max(cx.point_tol())
    }

    /// Removes the last vertex and hides the segment that followed the pointer until the pointer
    /// moves again. A press in progress no longer owns a vertex, so dropping it removes nothing more.
    fn take_back_vertex(&mut self) {
        self.vertices.pop();
        self.hover = None;
        self.pressed_vertex = false;
    }

    fn clear(&mut self) {
        self.vertices.clear();
        self.hover = None;
        self.pressed_vertex = false;
    }

    /// Closes the loop and selects inside it. A loop of fewer than three vertices selects nothing.
    fn close(&mut self, cx: &ToolContext) -> Vec<Action> {
        let vertices = std::mem::take(&mut self.vertices);
        self.clear();
        if vertices.len() < 3 {
            return vec![];
        }
        vec![lasso_select(cx, &mut self.mesh, &vertices, self.add, self.sub)]
    }
}

impl Tool for PolygonalLassoTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if !ev.pos.is_finite() {
            return vec![];
        }
        match ev.kind {
            PointerKind::Move | PointerKind::Drag => {
                self.hover = (!self.vertices.is_empty()).then(|| self.target(cx, ev.pos, ev.mods));
                vec![]
            }
            PointerKind::Down => {
                self.pressed_vertex = false;
                let reach = Self::reach(cx);
                let Some(&first) = self.vertices.first() else {
                    self.add = ev.mods.shift;
                    self.sub = ev.mods.alt;
                    self.vertices.push(ev.pos);
                    self.pressed_vertex = true;
                    return vec![];
                };
                if first.distance(ev.pos) <= reach {
                    // With fewer than three vertices, a click on the first one adds nothing.
                    return if self.vertices.len() >= 3 { self.close(cx) } else { vec![] };
                }
                let p = self.target(cx, ev.pos, ev.mods);
                // The second click of a double-click lands on the vertex the first one set.
                if self.vertices.last().is_some_and(|last| last.distance(p) > reach) && self.vertices.len() < MAX_VERTICES {
                    self.vertices.push(p);
                    self.pressed_vertex = true;
                }
                self.hover = Some(p);
                vec![]
            }
            PointerKind::Up => {
                self.pressed_vertex = false;
                vec![]
            }
            PointerKind::DoubleClick => self.close(cx),
            // The third click of a triple-click starts no loop of its own.
            PointerKind::TripleClick => {
                if self.vertices.len() == 1 {
                    self.clear();
                }
                vec![]
            }
        }
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Enter => self.close(cx),
            ToolKey::Escape => {
                self.clear();
                vec![]
            }
            ToolKey::Backspace | ToolKey::Delete => {
                self.take_back_vertex();
                vec![]
            }
            _ => vec![],
        }
    }

    /// A dropped press (a second finger, a canceled touch) takes back only the vertex it set.
    fn drop_press(&mut self, _cx: &ToolContext) -> Vec<Action> {
        if std::mem::take(&mut self.pressed_vertex) {
            self.take_back_vertex();
        }
        vec![]
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        !self.vertices.is_empty() && matches!(key, ToolKey::Enter | ToolKey::Escape | ToolKey::Backspace | ToolKey::Delete)
    }

    /// A loop being drawn: Escape drops it. Keys the tool does not claim run their commands.
    fn ends_on_escape(&self, _cx: &ToolContext) -> bool {
        !self.vertices.is_empty()
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.clear();
        vec![]
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
    }

    fn options(&self) -> serde_json::Value {
        self.mesh.options()
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = self.mesh.overlays(cx, &|id| cx.selection.contains(id));
        let mut line = self.vertices.clone();
        line.extend(self.hover.filter(|_| !self.vertices.is_empty()));
        if line.len() >= 2 {
            out.push(Overlay::Path { path: crate::xform::polygon(&line, false), color: [0x40, 0x40, 0x40], width: 1.0, dashed: true });
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Lasso
    }
}

#[cfg(test)]
mod tests;
