//! Magic Wand (Y) and Lasso (Q).
//!
//! Magic Wand: click an object to select every object with similar attributes. The matching and
//! its settings (Magic Wand panel) live in the engine's `select.magicWand` command; Shift adds,
//! Alt subtracts. Lasso: freehand loop selecting anchor points inside it (direct-selection
//! style); Shift adds, Alt subtracts.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::hit::hit_test;
use vectorcraft_doc::{AnchorRef, NodeId, NodeKind};
use vectorcraft_geom::Point;

use super::paint_owner;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

#[derive(Default)]
pub struct MagicWandTool;

impl Tool for MagicWandTool {
    fn id(&self) -> &'static str {
        "magicWand"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if ev.kind != PointerKind::Down {
            return vec![];
        }
        let Some(h) = hit_test(cx.doc, ev.pos, cx.hit_options()) else {
            return if ev.mods.shift || ev.mods.alt { vec![] } else { vec![Action::Exec("select.none".into(), json!({}))] };
        };
        let mode = if ev.mods.alt {
            "subtract"
        } else if ev.mods.shift {
            "add"
        } else {
            "set"
        };
        vec![Action::Exec("select.magicWand".into(), json!({ "id": paint_owner(cx.doc, h.leaf).0, "mode": mode }))]
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::MagicWand
    }
}

/// Even-odd point-in-polygon.
pub fn point_in_polygon(poly: &[Point], p: Point) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[derive(Default)]
pub struct LassoTool {
    points: Vec<Point>,
    active: bool,
}

fn anchors_json(v: &BTreeSet<AnchorRef>) -> Value {
    Value::Array(v.iter().map(|(s, a)| json!([s, a])).collect())
}

impl Tool for LassoTool {
    fn id(&self) -> &'static str {
        "lasso"
    }

    fn busy(&self) -> bool {
        self.active
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Down => {
                self.points = vec![p];
                self.active = true;
                vec![]
            }
            PointerKind::Drag if self.active => {
                if self.points.last().is_none_or(|q| q.distance(p) >= cx.tol(1.0)) {
                    self.points.push(p);
                }
                vec![]
            }
            PointerKind::Up if self.active => {
                self.active = false;
                let poly = std::mem::take(&mut self.points);
                let (add, sub) = (ev.mods.shift, ev.mods.alt);
                if poly.len() < 3 {
                    return if add || sub { vec![] } else { vec![Action::Exec("select.none".into(), json!({}))] };
                }
                // Anchors inside the loop for every editable path.
                let mut hits: Vec<(NodeId, BTreeSet<AnchorRef>)> = vec![];
                cx.doc.walk(|n| {
                    if let NodeKind::Path { path, .. } = &n.kind {
                        let v: BTreeSet<AnchorRef> =
                            path.anchors().filter(|(_, _, a)| point_in_polygon(&poly, a.p)).map(|(s, i, _)| (s, i)).collect();
                        if !v.is_empty() {
                            hits.push((n.id, v));
                        }
                    }
                });
                let editable = cx.doc.editable();
                hits.retain(|(id, _)| editable(*id) && cx.doc.is_visible(*id));
                let items: Vec<Value> = hits.iter().map(|(id, v)| json!({ "id": id.0, "anchors": anchors_json(v) })).collect();
                let mode = if sub {
                    "subtract"
                } else if add {
                    "add"
                } else {
                    "set"
                };
                vec![Action::Exec("select.anchorsMany".into(), json!({ "items": items, "mode": mode }))]
            }
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.active = false;
            self.points.clear();
        }
        vec![]
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        if !self.active || self.points.len() < 2 {
            return vec![];
        }
        vec![Overlay::Path { path: super::polygon(&self.points, false), color: [0x40, 0x40, 0x40], width: 1.0, dashed: true }]
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Lasso
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    #[test]
    fn magic_wand_emits_the_engine_command() {
        let (d, a) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = MagicWandTool;
        let r = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert_eq!(r, vec![Action::Exec("select.magicWand".into(), json!({"id": a.0, "mode": "set"}))]);
        let shift = Mods { shift: true, ..Default::default() };
        let r = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0).with_mods(shift));
        assert_eq!(r, vec![Action::Exec("select.magicWand".into(), json!({"id": a.0, "mode": "add"}))]);
        let miss = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 900.0, 900.0));
        assert_eq!(miss, vec![Action::Exec("select.none".into(), json!({}))]);
    }

    #[test]
    fn lasso_selects_anchors_inside() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = LassoTool::default();
        // A triangle around the top-left corner (100,100) only.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 80.0, 80.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 80.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 80.0, 140.0));
        assert_eq!(t.overlays(&cx).len(), 1);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 80.0, 140.0));
        let Action::Exec(c, v) = &a[0] else { panic!() };
        assert_eq!(c, "select.anchorsMany");
        assert_eq!(v["items"][0]["id"], id.0);
        assert_eq!(v["items"][0]["anchors"].as_array().unwrap().len(), 1);
        assert_eq!(v["mode"], "set");
        assert!(point_in_polygon(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0)], Point::new(2.0, 2.0)));
    }
}
