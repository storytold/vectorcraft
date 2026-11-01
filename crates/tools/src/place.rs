//! The place cursor (`file.place.queue` loads it with files): a click places the current file at
//! 100% with its top-left corner at the click, a drag places it at the dragged size (its aspect
//! kept), ←/→ and ↑/↓ cycle the files and Esc discards the current one. Each placement is one
//! `file.place`; after the last file the previous tool comes back.

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Rect};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// One loaded file.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    /// `file.place` params (the file and its options).
    params: Value,
    name: String,
    /// Its size at 100% (pt).
    width: f64,
    height: f64,
}

#[derive(Default)]
pub struct PlaceTool {
    entries: Vec<Entry>,
    current: usize,
    /// The tool to return to when the last file is placed or discarded.
    prev: Option<String>,
    /// Pressed at, and the current drag point.
    drag: Option<(Point, Point)>,
}

impl PlaceTool {
    fn entry(&self) -> Option<&Entry> {
        self.entries.get(self.current)
    }

    /// The box the current file goes into for a press at `a` released at `b`: its 100% size with
    /// its top-left corner at `a` for a click (under `tol`), else grown from `a` towards `b` with
    /// its aspect kept.
    fn target(&self, a: Point, b: Point, tol: f64) -> Option<Rect> {
        let e = self.entry()?;
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        if dx.abs().max(dy.abs()) < tol || e.width <= 0.0 || e.height <= 0.0 {
            return Some(Rect::new(a.x, a.y, a.x + e.width, a.y + e.height));
        }
        let k = (dx.abs() / e.width).max(dy.abs() / e.height);
        let (w, h) = (e.width * k, e.height * k);
        let x = if dx < 0.0 { a.x - w } else { a.x };
        let y = if dy < 0.0 { a.y - h } else { a.y };
        Some(Rect::new(x, y, x + w, y + h))
    }

    /// Drop the current file; after the last one, back to the previous tool.
    fn discard(&mut self) -> Vec<Action> {
        if self.current < self.entries.len() {
            self.entries.remove(self.current);
        }
        if self.entries.is_empty() {
            self.current = 0;
            return vec![Action::SwitchTool(self.prev.clone().unwrap_or_else(|| "selection".into()))];
        }
        self.current %= self.entries.len();
        vec![]
    }

    fn cycle(&mut self, forward: bool) {
        let n = self.entries.len();
        if n > 0 {
            self.current = if forward { (self.current + 1) % n } else { (self.current + n - 1) % n };
        }
    }
}

impl Tool for PlaceTool {
    fn id(&self) -> &'static str {
        "place"
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down if self.entry().is_some() => self.drag = Some((ev.pos, ev.pos)),
            PointerKind::Drag => {
                if let Some((a, _)) = self.drag {
                    self.drag = Some((a, ev.pos));
                }
            }
            PointerKind::Up => {
                let Some((a, _)) = self.drag.take() else { return vec![] };
                let (Some(r), Some(e)) = (self.target(a, ev.pos, cx.tol(3.0)), self.entry()) else { return vec![] };
                let mut p = e.params.clone();
                p["rect"] = json!([r.x0, r.y0, r.width(), r.height()]);
                let mut acts = vec![Action::Exec("file.place".into(), p)];
                acts.extend(self.discard());
                return acts;
            }
            _ => {}
        }
        vec![]
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape if self.drag.take().is_some() => vec![],
            ToolKey::Escape => self.discard(),
            ToolKey::Left | ToolKey::Up => {
                self.cycle(false);
                vec![]
            }
            ToolKey::Right | ToolKey::Down => {
                self.cycle(true);
                vec![]
            }
            _ => vec![],
        }
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        !self.entries.is_empty() && matches!(key, ToolKey::Escape | ToolKey::Left | ToolKey::Right | ToolKey::Up | ToolKey::Down)
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some((a, b)) = self.drag else { return vec![] };
        let Some(r) = self.target(a, b, cx.tol(3.0)) else { return vec![] };
        let mut o = vec![Overlay::Marquee(r)];
        if cx.measurement_labels {
            o.push(Overlay::Measure { p: b, text: cx.size_label(r.width(), r.height()) });
        }
        o
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }

    /// `{count, current, name, width, height, names}`: what the cursor holds (no file data).
    fn options(&self) -> Value {
        let e = self.entry();
        json!({
            "count": self.entries.len(),
            "current": self.current,
            "name": e.map(|e| &e.name),
            "width": e.map(|e| e.width),
            "height": e.map(|e| e.height),
            "names": self.entries.iter().map(|e| &e.name).collect::<Vec<_>>(),
        })
    }

    /// `queue`: `{entries: [{params, name, width, height}], prev?}` loads the cursor (replacing what
    /// it held); `current`: index of the file to place next.
    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "queue" => {
                let entries: Vec<Entry> = value["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| {
                        Some(Entry {
                            params: e.get("params").filter(|p| p.is_object())?.clone(),
                            name: e["name"].as_str().unwrap_or_default().to_string(),
                            width: e["width"].as_f64().filter(|v| v.is_finite())?,
                            height: e["height"].as_f64().filter(|v| v.is_finite())?,
                        })
                    })
                    .collect();
                self.entries = entries;
                self.current = 0;
                self.drag = None;
                if let Some(prev) = value["prev"].as_str() {
                    self.prev = Some(prev.to_string());
                }
            }
            "current" => {
                if let Some(i) = value.as_u64().map(|i| i as usize).filter(|i| *i < self.entries.len()) {
                    self.current = i;
                }
            }
            _ => {}
        }
    }

    /// Switching tools discards what the cursor holds.
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.entries.clear();
        self.drag = None;
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{cx, doc_with_rect, paint};

    fn loaded(n: usize) -> PlaceTool {
        let mut t = PlaceTool::default();
        let entries: Vec<Value> =
            (0..n).map(|i| json!({"params": {"path": format!("f{i}.png")}, "name": format!("f{i}.png"), "width": 40.0, "height": 20.0})).collect();
        t.set_option("queue", &json!({"entries": entries, "prev": "pen"}));
        t
    }

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    #[test]
    fn click_places_at_100_percent_and_drag_keeps_the_aspect() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Default::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = loaded(3);
        t.pointer(&c, &ev(PointerKind::Down, 10.0, 10.0));
        let acts = t.pointer(&c, &ev(PointerKind::Up, 11.0, 10.0));
        assert_eq!(acts, vec![Action::Exec("file.place".into(), json!({"path": "f0.png", "rect": [10.0, 10.0, 40.0, 20.0]}))]);
        // A drag up and to the left grows the box from the press point, aspect kept.
        t.pointer(&c, &ev(PointerKind::Down, 100.0, 100.0));
        t.pointer(&c, &ev(PointerKind::Drag, 20.0, 90.0));
        let c_marquee_only = ToolContext { measurement_labels: false, ..c };
        assert_eq!(t.overlays(&c_marquee_only), vec![Overlay::Marquee(Rect::new(20.0, 60.0, 100.0, 100.0))]);
        let acts = t.pointer(&c, &ev(PointerKind::Up, 20.0, 90.0));
        assert_eq!(acts[0], Action::Exec("file.place".into(), json!({"path": "f1.png", "rect": [20.0, 60.0, 80.0, 40.0]})));
        assert_eq!(t.options()["count"], 1);
        // Placing the last file returns to the previous tool.
        t.pointer(&c, &ev(PointerKind::Down, 0.0, 0.0));
        let acts = t.pointer(&c, &ev(PointerKind::Up, 0.0, 0.0));
        assert_eq!(acts.last(), Some(&Action::SwitchTool("pen".into())));
        assert!(!t.claims_key(&c, ToolKey::Escape), "an empty cursor takes no keys");
    }

    #[test]
    fn measurement_labels_gate_the_place_size_readout() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Default::default(), paint());
        let base = cx(&d, &s, &p);
        let mut t = loaded(1);
        t.pointer(&base, &ev(PointerKind::Down, 100.0, 100.0));
        t.pointer(&base, &ev(PointerKind::Drag, 20.0, 90.0));
        let r = Rect::new(20.0, 60.0, 100.0, 100.0);
        for on in [false, true] {
            let c = ToolContext { measurement_labels: on, ..base };
            let o = t.overlays(&c);
            assert!(o.iter().any(|x| matches!(x, Overlay::Marquee(_))));
            let measures = o.iter().filter(|x| matches!(x, Overlay::Measure { .. })).count();
            assert_eq!(measures, usize::from(on), "measurement labels {on}");
            if on {
                assert!(o.iter().any(
                    |x| matches!(x, Overlay::Measure { p, text, .. } if *p == Point::new(20.0, 90.0) && text == &c.size_label(r.width(), r.height()))
                ));
            }
        }
    }

    #[test]
    fn arrows_cycle_and_escape_discards() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Default::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = loaded(3);
        assert!(t.claims_key(&c, ToolKey::Left) && t.claims_key(&c, ToolKey::Escape));
        t.key(&c, ToolKey::Left, Mods::default());
        assert_eq!(t.options()["name"], "f2.png");
        t.key(&c, ToolKey::Down, Mods::default());
        assert_eq!(t.options()["name"], "f0.png");
        assert!(t.key(&c, ToolKey::Escape, Mods::default()).is_empty());
        assert_eq!(t.options()["names"], json!(["f1.png", "f2.png"]));
        t.key(&c, ToolKey::Escape, Mods::default());
        assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::SwitchTool("pen".into())]);
    }
}
