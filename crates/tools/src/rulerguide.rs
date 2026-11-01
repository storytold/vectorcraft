//! Ruler guides on the canvas. Dragged out of a ruler with any tool ([`NewGuide`]), a guide is
//! made where it is released over the canvas: with the Artboard tool, an artboard guide of the
//! active artboard. For the selection tools ([`GuideEdit`]), a press on a guide selects it (Shift
//! adds it to the selected guides or, released without a drag, takes it out; a click on a guide
//! selected with objects makes it the key object Align lines them up on, `select.key`, and a
//! click on the key guide again cancels it); dragging moves the
//! selected guides (Alt copies them), and a marquee selects the guides it crosses along with the
//! art ([`marquee_select`]). A dragged guide snaps ([`GuideSnap`]): with Shift to the
//! ruler's ticks, else to whole pixels, the grid, with Smart Guides the art's and the artboards'
//! edges, centres and anchors, or with Snap to Point the anchors. Dropped off the canvas onto its
//! ruler, a moved guide is deleted. Hidden or locked guides (View › Guides), and guides on a hidden
//! or locked layer, can't be picked.

use serde_json::json;
use vectorcraft_geom::{Point, Rect};

use crate::guides::Targets;
use crate::{Action, Cursor, Overlay, PointerEvent, PointerKind, ToolContext};

/// The ruler guide under `p`: the nearest within the selection tolerance (the newest of equals).
pub fn guide_at(cx: &ToolContext, p: Point) -> Option<usize> {
    if !cx.guides {
        return None;
    }
    let tol = cx.pick_tol();
    let off = |pos: f64, vertical: bool| (if vertical { p.x } else { p.y } - pos).abs();
    cx.doc
        .guides
        .iter()
        .enumerate()
        .filter(|(_, g)| cx.doc.guide_editable(g) && cx.doc.guide_passes(g, p, tol))
        .map(|(i, g)| (i, off(g.pos, g.vertical)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(i, _)| i)
}

/// What a selection tool's marquee `r` does to the ruler guides, after its action selecting the
/// art: the guides it crosses join the selection (with a Shift marquee, the selected ones leave
/// it, as with a Shift-click on a guide). With `enclosed` (E pressed during the drag), only the
/// artboard guides that lie entirely inside the marquee are taken; a canvas guide is never taken.
pub fn marquee_select(cx: &ToolContext, r: Rect, enclosed: bool) -> Option<Action> {
    let mut guides = guides_in_rect(cx, r);
    if enclosed {
        guides.retain(|i| cx.doc.guides.get(*i).is_some_and(|g| guide_inside(cx, r, g)));
    }
    (!guides.is_empty()).then(|| Action::Exec("guide.select".into(), json!({ "indexes": guides, "toggle": true })))
}

/// The editable ruler guides intersecting marquee rectangle `r`.
fn guides_in_rect(cx: &ToolContext, r: Rect) -> Vec<usize> {
    if !cx.guides {
        return vec![];
    }
    cx.doc
        .guides
        .iter()
        .enumerate()
        .filter(|(_, g)| cx.doc.guide_editable(g))
        .filter(|(_, g)| {
            if g.vertical {
                r.min_x() <= g.pos && g.pos <= r.max_x() && cx.doc.guide_span(g).is_none_or(|(y0, y1)| r.max_y() >= y0 && r.min_y() <= y1)
            } else {
                r.min_y() <= g.pos && g.pos <= r.max_y() && cx.doc.guide_span(g).is_none_or(|(x0, x1)| r.max_x() >= x0 && r.min_x() <= x1)
            }
        })
        .map(|(i, _)| i)
        .collect()
}

/// Does ruler guide `g`, crossed by marquee `r`, run along its line within the marquee? An artboard
/// guide does when the marquee spans its artboard; a canvas guide runs across the whole canvas and
/// never does.
fn guide_inside(cx: &ToolContext, r: Rect, g: &vectorcraft_doc::Guide) -> bool {
    let Some((a, b)) = cx.doc.guide_span(g) else { return false };
    let (lo, hi) = if g.vertical { (r.min_y(), r.max_y()) } else { (r.min_x(), r.max_x()) };
    lo <= a.min(b) && a.max(b) <= hi
}

/// The pointer for a guide: it moves sideways if vertical, up and down if horizontal.
fn guide_cursor(vertical: bool) -> Cursor {
    if vertical { Cursor::ResizeH } else { Cursor::ResizeV }
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    index: usize,
    vertical: bool,
    /// Where the guide was, and where it is now (x of a vertical guide, y of a horizontal one).
    from: f64,
    at: f64,
    start: Point,
    pointer: Point,
    began: bool,
    /// Shift-pressed on a selected guide: released without a drag, it leaves the selection.
    deselect: bool,
    /// Pressed (no Shift) on a guide selected with objects: released without a drag, it becomes
    /// the key object Align lines them up on, or stops being it when it was.
    key: bool,
}

/// Snapping for a dragged ruler guide, its targets gathered when the drag begins.
#[derive(Default)]
struct GuideSnap {
    /// With Smart Guides the art's and the artboards' edges, centres and anchors, else with Snap
    /// to Point the anchors, and how near (in screen pixels) they pull; none with pixels or the
    /// grid snapping instead, or nothing on.
    targets: Option<(Targets, f64)>,
    /// The target the guide snapped to.
    snapped: Vec<Overlay>,
}

impl GuideSnap {
    fn new(cx: &ToolContext) -> Self {
        let targets = || Targets::collect(cx.doc, &[], None);
        let targets = if cx.snap_to_pixel || cx.snap_to_grid {
            None
        } else if cx.smart_guides {
            Some((targets().styled(cx), cx.snapping_tolerance))
        } else if cx.snap_to_point {
            Some((targets().anchors_only(), cx.snap_tolerance))
        } else {
            None
        };
        Self { targets, snapped: vec![] }
    }

    /// Where a guide dragged to `v` (the x of a vertical one, the y of a horizontal one) goes:
    /// with Shift on the nearest ruler tick, else on whole pixels, the grid or a target.
    fn snap(&mut self, cx: &ToolContext, vertical: bool, v: f64, shift: bool) -> f64 {
        self.snapped.clear();
        if shift {
            let origin = cx.ruler_origin;
            let origin = if vertical { origin.x } else { origin.y };
            return origin + cx.unit.snap_to_ruler_tick(v - origin, cx.zoom);
        }
        if cx.snap_to_pixel {
            return v.round();
        }
        if cx.snap_to_grid {
            return vectorcraft_geom::snap::snap_to_grid(v, cx.grid_step(), 0.0);
        }
        let Some((t, tol)) = &self.targets else { return v };
        let (at, snapped) = t.snap_guide(vertical, v, cx.tol(*tol));
        self.snapped = snapped;
        at
    }

    /// The target the guide at `at` snapped to, and its position by the `pointer`.
    fn overlays(&self, cx: &ToolContext, vertical: bool, at: f64, pointer: Point) -> Vec<Overlay> {
        let mut o = self.snapped.clone();
        if cx.measurement_labels {
            let origin = cx.ruler_origin;
            let (axis, zero) = if vertical { ("X", origin.x) } else { ("Y", origin.y) };
            o.push(Overlay::Measure { p: pointer, text: format!("{axis}: {}", cx.len(at - zero)) });
        }
        o
    }
}

/// A guide dragged out of a ruler (a vertical one out of the left ruler), with any tool: it shows
/// where the pointer is, snapped as a moved guide is, and is made where the button is released
/// over the canvas (`guide.add`, previewed as it goes); taken off the canvas, it goes again.
/// Alt swaps its orientation while the guide is dragged out of the ruler.
pub struct NewGuide {
    /// The source ruler's orientation, before Alt swaps it.
    vertical: bool,
    /// The artboard (index) it belongs to: an artboard guide.
    artboard: Option<usize>,
    snap: GuideSnap,
    /// The preview's orientation and position (none off the canvas), and the pointer.
    at: Option<(bool, f64)>,
    pointer: Point,
}

impl NewGuide {
    /// A vertical guide out of the left ruler, else a horizontal one out of the top ruler; an
    /// artboard guide of `artboard`, if any.
    pub fn new(cx: &ToolContext, vertical: bool, artboard: Option<usize>) -> Self {
        Self { vertical, artboard, snap: GuideSnap::new(cx), at: None, pointer: Point::ZERO }
    }

    /// The pointer at `ev`, over the canvas or not (`on_canvas`): the actions that show the guide
    /// there or take it away, and on release make it.
    pub fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent, on_canvas: bool) -> Vec<Action> {
        let mut out = vec![];
        self.pointer = ev.pos;
        if on_canvas {
            let vertical = self.vertical != ev.mods.alt;
            let at = self.snap.snap(cx, vertical, if vertical { ev.pos.x } else { ev.pos.y }, ev.mods.shift);
            if self.at.is_none() {
                out.push(Action::Begin("New Guide".into()));
            }
            if self.at != Some((vertical, at)) {
                let mut p = json!({ "vertical": vertical, "pos": at });
                if let Some(ab) = self.artboard {
                    p["artboard"] = json!(ab);
                }
                out.push(Action::Preview("guide.add".into(), p));
            }
            self.at = Some((vertical, at));
        } else if self.at.take().is_some() {
            self.snap.snapped.clear();
            out.push(Action::Cancel);
        }
        if ev.kind == PointerKind::Up && self.at.take().is_some() {
            out.push(Action::Commit);
        }
        out
    }

    /// While the guide shows: the target it snapped to and its position by the pointer.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        self.at.map(|(vertical, at)| self.snap.overlays(cx, vertical, at, self.pointer)).unwrap_or_default()
    }
}

/// A selection tool's ruler-guide handling: [`Self::press`] on a press, [`Self::pointer`] for
/// the rest of the gesture.
#[derive(Default)]
pub struct GuideEdit {
    drag: Option<Drag>,
    snap: GuideSnap,
}

impl GuideEdit {
    /// Is a guide pressed or being dragged?
    pub fn busy(&self) -> bool {
        self.drag.is_some()
    }

    /// Escape drops a pressed guide; a begun preview is rolled back without a history step.
    pub fn cancel(&mut self) -> Vec<Action> {
        let drag = self.drag.take();
        self.snap = GuideSnap::default();
        if drag.is_some_and(|d| d.began) { vec![Action::Cancel] } else { vec![] }
    }

    /// A press `ev` on a ruler guide: it gets selected and the drag starts. None (and nothing
    /// done) when no guide is there.
    pub fn press(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Option<Vec<Action>> {
        let index = guide_at(cx, ev.pos)?;
        let g = cx.doc.guides.get(index)?;
        let (shift, selected) = (ev.mods.shift, cx.selection.guides.contains(&index));
        self.drag = Some(Drag {
            index,
            vertical: g.vertical,
            from: g.pos,
            at: g.pos,
            start: ev.pos,
            pointer: ev.pos,
            began: false,
            deselect: shift && selected,
            key: !shift && selected && !cx.selection.is_empty(),
        });
        self.snap = GuideSnap::default();
        Some(match (selected, shift) {
            (true, _) => vec![],
            (false, true) => vec![Action::Exec("guide.select".into(), json!({ "indexes": [index], "toggle": true }))],
            (false, false) => vec![Action::Exec("guide.select".into(), json!({ "indexes": [index] }))],
        })
    }

    /// The drag and release of a guide [`Self::press`] picked; None when no guide is pressed.
    pub fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Option<Vec<Action>> {
        let mut d = self.drag?;
        Some(match ev.kind {
            PointerKind::Drag => {
                let out = self.drag_to(cx, &mut d, ev);
                self.drag = Some(d);
                out
            }
            PointerKind::Up => {
                self.drag = None;
                self.snap = GuideSnap::default();
                if d.began && on_ruler(cx, d.vertical, ev.pos) {
                    // Dropped on its ruler: the guide goes (a copy is just not made).
                    let mut out = vec![Action::Cancel];
                    if !ev.mods.alt {
                        out.push(Action::Exec("guide.remove".into(), json!({ "index": d.index })));
                    }
                    out
                } else if d.began {
                    vec![Action::Commit]
                } else if d.deselect {
                    vec![Action::Exec("guide.select".into(), json!({ "indexes": [d.index], "toggle": true }))]
                } else if d.key && cx.selection.key_guide == Some(d.index) {
                    // A click on the key guide again: no key.
                    vec![Action::Exec("select.key".into(), json!({}))]
                } else if d.key {
                    vec![Action::Exec("select.key".into(), json!({ "guide": d.index }))]
                } else {
                    vec![]
                }
            }
            // Another press ends the gesture.
            _ => {
                self.drag = None;
                return None;
            }
        })
    }

    fn drag_to(&mut self, cx: &ToolContext, d: &mut Drag, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        let mut out = vec![];
        if !d.began {
            if p.distance(d.start) < cx.tol(3.0) {
                return out;
            }
            d.began = true;
            out.push(Action::Begin(if ev.mods.alt { "Copy Guide" } else { "Move Guide" }.into()));
            self.snap = GuideSnap::new(cx);
        }
        let moved = p - d.start;
        let at = self.snap.snap(cx, d.vertical, d.from + if d.vertical { moved.x } else { moved.y }, ev.mods.shift);
        (d.at, d.pointer) = (at, p);
        // The other selected guides follow the pointer: vertical ones across, horizontal ones down.
        let (dx, dy) = if d.vertical { (at - d.from, moved.y) } else { (moved.x, at - d.from) };
        out.push(Action::Preview("guide.move".into(), json!({ "dx": dx, "dy": dy, "copy": ev.mods.alt })));
        out
    }

    /// The pointer over a guide, or while one is dragged, turned with the view.
    pub fn cursor(&self, cx: &ToolContext, p: Point) -> Option<Cursor> {
        if let Some(d) = &self.drag {
            return Some(guide_cursor(d.vertical).turned(cx.view_angle()));
        }
        guide_at(cx, p).and_then(|i| cx.doc.guides.get(i)).map(|g| guide_cursor(g.vertical).turned(cx.view_angle()))
    }

    /// While a guide is dragged: the smart guide it snapped to and its position by the pointer.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some(d) = self.drag.filter(|d| d.began) else { return vec![] };
        self.snap.overlays(cx, d.vertical, d.at, d.pointer)
    }
}

/// Is `p` off the canvas on the side of the ruler a guide comes from (the top one for a
/// horizontal guide, the left one for a vertical guide)? Never without a window.
fn on_ruler(cx: &ToolContext, vertical: bool, p: Point) -> bool {
    cx.screen.and_then(|s| s.to_screen(p)).is_some_and(|(x, y)| if vertical { x < 0.0 } else { y < 0.0 })
}

#[cfg(test)]
mod tests;
