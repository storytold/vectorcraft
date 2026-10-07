//! Artboard tool (Shift+O).
//!
//! Click an artboard to make it active (dashed bounds, 8 handles and its name). Drag inside moves
//! it (with its artwork when the `moveArt` option is on, Shift constrains), drag a handle resizes it
//! (Shift proportional, Alt from centre), drag on the pasteboard draws a new artboard, Delete removes
//! the active one and Escape returns to the Selection tool.

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Rect, Vec2};

use super::{BLUE, polygon, rect_corners};
use crate::bbox::{Handle, hit_handle, move_delta, scale_for_drag};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

#[derive(Clone, Copy, Debug)]
enum Drag {
    Move { index: usize, start: Point, began: bool },
    Resize { index: usize, handle: Handle, rect: Rect, began: bool },
    Create { start: Point, cur: Point },
}

pub struct ArtboardTool {
    /// Index of the active artboard.
    pub active: usize,
    /// Move artwork with the artboard.
    pub move_art: bool,
    drag: Option<Drag>,
    preview: Option<Rect>,
}

impl Default for ArtboardTool {
    fn default() -> Self {
        Self { active: 0, move_art: true, drag: None, preview: None }
    }
}

fn rect_json(index: usize, r: Rect) -> Value {
    json!({ "index": index, "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height() })
}

impl ArtboardTool {
    fn active_rect(&self, cx: &ToolContext) -> Option<Rect> {
        cx.doc.artboards.get(self.active).map(|a| a.rect)
    }
}

impl Tool for ArtboardTool {
    fn id(&self) -> &'static str {
        "artboard"
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        let m = ev.mods;
        match (ev.kind, self.drag) {
            (PointerKind::Down, _) => {
                let locked = |i: usize| cx.doc.artboards.get(i).is_some_and(|a| a.locked);
                if let Some(r) = self.active_rect(cx)
                    && !locked(self.active)
                    && let Some(handle) = hit_handle(r, p, cx.tol(5.0))
                {
                    self.drag = Some(Drag::Resize { index: self.active, handle, rect: r, began: false });
                    return vec![];
                }
                if let Some(i) = (0..cx.doc.artboards.len()).rev().find(|i| cx.doc.artboards.get(*i).is_some_and(|a| a.rect.contains(p))) {
                    self.active = i;
                    // A locked artboard becomes active but stays where it is.
                    if !locked(i) {
                        self.drag = Some(Drag::Move { index: i, start: p, began: false });
                    }
                    return vec![Action::Exec("artboard.setActive".into(), json!({ "index": i }))];
                } else {
                    let (p, _) = crate::guides::snap_draw(cx, p, &[]);
                    self.drag = Some(Drag::Create { start: p, cur: p });
                }
                vec![]
            }
            (PointerKind::Drag, Some(Drag::Move { index, start, began })) => {
                let mut out = vec![];
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
                        return out;
                    }
                    out.push(Action::Begin("Move Artboard".into()));
                }
                self.drag = Some(Drag::Move { index, start, began: true });
                let d = move_delta(start, p, m.shift);
                self.preview = cx.doc.artboards.get(index).map(|a| a.rect + d);
                out.push(Action::Preview("artboard.move".into(), json!({ "index": index, "dx": d.x, "dy": d.y, "moveArt": self.move_art })));
                out
            }
            (PointerKind::Drag, Some(Drag::Resize { index, handle, rect, began })) => {
                let mut out = vec![];
                if !began {
                    out.push(Action::Begin("Resize Artboard".into()));
                }
                self.drag = Some(Drag::Resize { index, handle, rect, began: true });
                let nr = scale_for_drag(rect, handle, p, m.shift, m.alt).transform_rect_bbox(rect);
                self.preview = Some(nr);
                out.push(Action::Preview("artboard.setProps".into(), rect_json(index, nr)));
                out
            }
            (PointerKind::Drag, Some(Drag::Create { start, .. })) => {
                let (mut cur, _) = crate::guides::snap_draw(cx, p, &[]);
                if m.shift {
                    let d = cur - start;
                    let s = d.x.abs().max(d.y.abs());
                    cur = start + Vec2::new(s.copysign(d.x), s.copysign(d.y));
                }
                self.drag = Some(Drag::Create { start, cur });
                vec![]
            }
            (PointerKind::Up, Some(d)) => {
                self.drag = None;
                self.preview = None;
                match d {
                    Drag::Move { began: true, .. } | Drag::Resize { began: true, .. } => vec![Action::Commit],
                    Drag::Create { start, cur } => {
                        let r = Rect::from_points(start, cur);
                        if r.width() < cx.tol(3.0) || r.height() < cx.tol(3.0) {
                            return vec![];
                        }
                        vec![
                            Action::Exec("artboard.new".into(), json!({ "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height() })),
                            Action::Notify("created".into()),
                        ]
                    }
                    _ => vec![],
                }
            }
            (PointerKind::DoubleClick, _) => {
                self.drag = None;
                match cx.doc.artboards.get(self.active) {
                    Some(a) => {
                        let mut v = rect_json(self.active, a.rect);
                        v["name"] = json!(a.name);
                        v["background"] = json!(a.background.map(|c| c.to_hex()).unwrap_or_default());
                        vec![Action::Dialog("artboardOptions".into(), v)]
                    }
                    None => vec![],
                }
            }
            _ => vec![],
        }
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Delete | ToolKey::Backspace if self.drag.is_none() => {
                if cx.doc.artboards.len() <= 1 || self.active >= cx.doc.artboards.len() {
                    return vec![];
                }
                let i = self.active;
                self.active = i.saturating_sub(1);
                vec![Action::Exec("artboard.delete".into(), json!({ "index": i }))]
            }
            ToolKey::Escape => {
                let busy = matches!(self.drag, Some(Drag::Move { began: true, .. } | Drag::Resize { began: true, .. }));
                self.drag = None;
                self.preview = None;
                if busy { vec![Action::Cancel] } else { vec![Action::SwitchTool("selection".into())] }
            }
            _ => vec![],
        }
    }

    fn notify(&mut self, cx: &ToolContext, what: &str) {
        if what == "created" {
            self.active = cx.doc.artboards.len().saturating_sub(1);
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = vec![];
        if let Some(Drag::Create { start, cur }) = self.drag {
            let r = Rect::from_points(start, cur);
            o.push(Overlay::Marquee(r));
            o.push(Overlay::Measure { p: cur + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: cx.size_label(r.width(), r.height()) });
            return o;
        }
        let Some(ab) = cx.doc.artboards.get(self.active) else { return o };
        let r = self.preview.unwrap_or(ab.rect);
        o.push(Overlay::Path { path: polygon(&rect_corners(r), true), color: BLUE, width: 1.0, dashed: true });
        for h in Handle::ALL {
            o.push(Overlay::Anchor { p: h.pos(r), color: BLUE, filled: false, size: 7.0 });
        }
        o.push(Overlay::Label { p: Point::new(r.x0, r.y0 - cx.tol(14.0)), text: format!("{:02} - {}", self.active + 1, ab.name), color: BLUE });
        if let Some(Drag::Resize { began: true, .. }) = self.drag {
            o.push(Overlay::Measure {
                p: Point::new(r.x1, r.y1) + Vec2::new(cx.tol(12.0), cx.tol(12.0)),
                text: cx.size_label(r.width(), r.height()),
            });
        }
        o
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        if let Some(r) = self.active_rect(cx)
            && let Some(h) = hit_handle(r, p, cx.tol(5.0))
        {
            return match h {
                Handle::Top | Handle::Bottom => Cursor::ResizeV,
                Handle::Left | Handle::Right => Cursor::ResizeH,
                Handle::TopLeft | Handle::BottomRight => Cursor::ResizeNwSe,
                Handle::TopRight | Handle::BottomLeft => Cursor::ResizeNeSw,
            };
        }
        if cx.doc.artboards.iter().any(|a| a.rect.contains(p)) { Cursor::Move } else { Cursor::Crosshair }
    }

    fn options(&self) -> Value {
        json!({ "active": self.active, "moveArt": self.move_art })
    }

    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "active" => self.active = value.as_u64().unwrap_or(0) as usize,
            "moveArt" => self.move_art = value.as_bool().unwrap_or(true),
            _ => {}
        }
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        let busy = matches!(self.drag, Some(Drag::Move { began: true, .. } | Drag::Resize { began: true, .. }));
        self.drag = None;
        self.preview = None;
        if busy { vec![Action::Cancel] } else { vec![] }
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
    fn move_resize_create_delete() {
        let (mut d, _) = doc_with_rect();
        let mut ab = d.artboards[0].clone();
        ab.rect = Rect::new(600.0, 0.0, 800.0, 200.0);
        ab.name = "Artboard 2".into();
        d.artboards.push(ab);
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ArtboardTool::default();
        // Click the second artboard → active; drag moves it.
        t.pointer(&cx, &ev(PointerKind::Down, 700.0, 100.0));
        assert_eq!(t.active, 1);
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 710.0, 120.0));
        assert_eq!(a[0], Action::Begin("Move Artboard".into()));
        assert_eq!(a[1], Action::Preview("artboard.move".into(), json!({"index": 1, "dx": 10.0, "dy": 20.0, "moveArt": true})));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 710.0, 120.0)), vec![Action::Commit]);
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "02 - Artboard 2")));
        // Handle drag resizes.
        t.pointer(&cx, &ev(PointerKind::Down, 800.0, 100.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 900.0, 100.0));
        assert_eq!(a[1], Action::Preview("artboard.setProps".into(), json!({"index": 1, "x": 600.0, "y": 0.0, "width": 300.0, "height": 200.0})));
        t.pointer(&cx, &ev(PointerKind::Up, 900.0, 100.0));
        // Drag on the pasteboard creates one.
        t.pointer(&cx, &ev(PointerKind::Down, 1000.0, 1000.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 1100.0, 1050.0));
        assert!(matches!(t.overlays(&cx)[0], Overlay::Marquee(_)));
        let a = t.pointer(&cx, &ev(PointerKind::Up, 1100.0, 1050.0));
        assert_eq!(a[0], Action::Exec("artboard.new".into(), json!({"x": 1000.0, "y": 1000.0, "width": 100.0, "height": 50.0})));
        assert_eq!(a[1], Action::Notify("created".into()));
        // Delete removes the active artboard.
        assert_eq!(t.key(&cx, ToolKey::Delete, Mods::default()), vec![Action::Exec("artboard.delete".into(), json!({"index": 1}))]);
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::SwitchTool("selection".into())]);
    }
}
