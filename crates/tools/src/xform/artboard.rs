//! Artboard tool (Shift+O).
//!
//! Click an object to create an artboard fitted to its visible bounds; drag still moves the
//! containing artboard or draws one on the pasteboard. The second click of a double-click on an
//! object does not fit another artboard, and that double-click does not open Artboard Options.
//! Double-click an empty part of an artboard to open Artboard Options. Click an empty
//! part of an artboard to make it active (dashed bounds, 8 handles and its name); Shift-click adds
//! artboards to the selection or takes them out (dashed bounds), and a drag or Delete then acts on
//! all of them (#1139). Shift-drag inside an artboard draws a new artboard there, and Shift-drag on
//! the pasteboard is a marquee: the artboards it touches leave the selection or join it. Drag inside
//! moves it (with its artwork when the `moveArt` option is on; Alt moves a copy and
//! leaves the artboard where it was), drag a handle resizes it
//! (Shift proportional, Alt from centre; with the `scaleArt` option on, Scale Artwork with Artboard,
//! always proportional, and the art fully inside scales with it), drag on the pasteboard draws a new artboard, Delete removes
//! the active one (Copy, Cut and Paste take it with its art: `artboard.copy`), the arrow keys move the
//! selected artboards by the keyboard increment (Shift ten times as far; with their artwork when
//! `moveArt` is on) and Escape returns to
//! the Selection tool (during a drag it only cancels the drag). Moving and resizing snap like drawing
//! does: to whole pixels, to the grid, or with Smart Guides to other artboards, their bleed and
//! objects (never to the dragged artboard or the art moving or scaling with it); the artboard's own bleed edges
//! snap too. A new artboard's corners snap as drawn points do ([`DrawSnap`]). A locked artboard (the
//! Artboards panel's lock column) is selected by a click, but a drag or an arrow key doesn't move it
//! or copy it, it shows no handles, and Delete leaves it.

use serde_json::{Value, json};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use super::{BLUE, polygon, rect_corners};
use crate::bbox::{Handle, hit_handle, scale_for_drag};
use crate::guides::{DrawSnap, Leave, Targets};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

#[derive(Clone, Copy, Debug)]
enum Drag {
    /// `rect` is the artboard as it was when the drag started: during the drag the document
    /// already shows the preview, so it can't be read back from there. `copy`: Alt was held at the
    /// last drag event (the step under way is a duplicate).
    Move {
        index: usize,
        /// Several artboards move together (the selected ones, `index` among them).
        many: bool,
        start: Point,
        rect: Rect,
        began: bool,
        copy: bool,
    },
    /// `grab` is the handle's offset from the pointer, so the handle itself follows and snaps.
    Resize {
        index: usize,
        handle: Handle,
        rect: Rect,
        grab: Vec2,
        began: bool,
    },
    Create {
        start: Point,
        cur: Point,
        /// A Shift-press inside an artboard (its index and the pressed point): released without
        /// being dragged, it toggles that artboard.
        toggle: Option<(usize, Point)>,
    },
    Marquee {
        start: Point,
        cur: Point,
    },
}

pub struct ArtboardTool {
    /// Index of the active artboard.
    pub active: usize,
    /// The other selected artboards (Shift-click), in order.
    pub others: Vec<usize>,
    /// Move artwork with the artboard.
    pub move_art: bool,
    /// Scale artwork with the artboard: a resize takes the art fully inside it along, and keeps the
    /// artboard's proportions.
    pub scale_art: bool,
    drag: Option<Drag>,
    /// The pressed object can be fitted on release; a drag, modifier or cancellation takes over.
    click_art: Option<(Point, Rect)>,
    preview: Option<Rect>,
    /// Smart Guide targets, gathered when a move or resize starts.
    targets: Option<Targets>,
    /// The targets for an Alt-drag copy: the original artboard and its art stay where they are.
    copy_targets: Option<Targets>,
    /// Smart Guides shown while dragging.
    guides: Vec<Overlay>,
    /// Smart Guides for the corners of a new artboard.
    draw: DrawSnap,
    /// The canvas sets this on the second click of a double-click, so that release does not fit
    /// a second artboard before [`PointerKind::DoubleClick`] arrives.
    suppress_fit: bool,
}

impl Default for ArtboardTool {
    fn default() -> Self {
        Self {
            active: 0,
            others: vec![],
            move_art: true,
            scale_art: false,
            drag: None,
            click_art: None,
            preview: None,
            targets: None,
            copy_targets: None,
            guides: vec![],
            draw: DrawSnap::default(),
            suppress_fit: false,
        }
    }
}

fn rect_json(index: usize, r: Rect) -> Value {
    json!({ "index": index, "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height() })
}

impl ArtboardTool {
    fn active_rect(&self, cx: &ToolContext) -> Option<Rect> {
        cx.doc.artboards.get(self.active).map(|a| a.rect)
    }

    /// The active artboard's rectangle when it can be resized: it isn't locked.
    fn resizable_rect(&self, cx: &ToolContext) -> Option<Rect> {
        cx.doc.artboards.get(self.active).filter(|a| !a.locked).map(|a| a.rect)
    }

    /// The artboards a drag from artboard `index` moves: with `many`, the selected ones that
    /// aren't locked, else `index`; none when `index` is locked.
    fn movable(&self, cx: &ToolContext, index: usize, many: bool) -> Vec<usize> {
        let unlocked = |i: &usize| cx.doc.artboards.get(*i).is_some_and(|a| !a.locked);
        if !unlocked(&index) {
            return vec![];
        }
        if many { self.selected(cx).into_iter().filter(unlocked).collect() } else { vec![index] }
    }

    /// The selected artboards that are still in the document: the active one and the others,
    /// in order.
    fn selected(&self, cx: &ToolContext) -> Vec<usize> {
        let mut all: Vec<usize> = std::iter::once(self.active).chain(self.others.iter().copied()).filter(|i| *i < cx.doc.artboards.len()).collect();
        all.sort_unstable();
        all.dedup();
        all
    }

    /// Shift-click on artboard `i`: into the selection, active, or out of it (another becomes
    /// active).
    fn toggle(&mut self, cx: &ToolContext, i: usize) {
        let mut sel = self.selected(cx);
        if let Some(at) = sel.iter().position(|j| *j == i) {
            if sel.len() > 1 {
                sel.remove(at);
                self.active = if i == self.active { sel.first().copied().unwrap_or(0) } else { self.active };
            }
        } else {
            sel.push(i);
            self.active = i;
        }
        self.others = sel.into_iter().filter(|j| *j != self.active).collect();
    }

    /// A Shift-drag marquee `r`: the artboards it touches leave the selection when they are in
    /// it and join it otherwise, the last one joining becoming active. One artboard always stays
    /// selected.
    fn toggle_touched(&mut self, cx: &ToolContext, r: Rect) {
        let touched: Vec<usize> = cx
            .doc
            .artboards
            .iter()
            .enumerate()
            .filter(|(_, a)| r.x0 <= a.rect.x1 && a.rect.x0 <= r.x1 && r.y0 <= a.rect.y1 && a.rect.y0 <= r.y1)
            .map(|(i, _)| i)
            .collect();
        let sel = self.selected(cx);
        let added: Vec<usize> = touched.iter().copied().filter(|i| !sel.contains(i)).collect();
        let kept: Vec<usize> = sel.into_iter().filter(|i| !touched.contains(i)).chain(added.iter().copied()).collect();
        let Some(&first) = kept.first() else { return };
        self.active = match added.last() {
            Some(i) => *i,
            None if kept.contains(&self.active) => self.active,
            None => first,
        };
        self.others = kept.into_iter().filter(|j| *j != self.active).collect();
    }

    /// Gather the Smart Guide targets for dragging artboard `index` (call before the first preview,
    /// while the document still shows where everything started).
    fn begin_snapping(&mut self, cx: &ToolContext, index: usize, art_moves: bool) {
        self.guides.clear();
        self.targets = cx.smart_guides.then(|| {
            let art = if art_moves { cx.doc.art_moving_with(index, cx.move_locked_with_artboard) } else { vec![] };
            Targets::for_artboard(cx.doc, index, &art).styled(cx)
        });
        // Nothing to skip: no artboard has this index.
        self.copy_targets = cx.smart_guides.then(|| Targets::for_artboard(cx.doc, usize::MAX, &[]).styled(cx));
    }

    /// Snap a dragged handle: to whole pixels, the grid, or Smart Guides (in that order, as when
    /// drawing). With Smart Guides the bleed edge `bleed` beyond the handle may snap instead.
    fn snap_handle(&mut self, cx: &ToolContext, p: Point, bleed: Vec2) -> Point {
        self.guides.clear();
        if cx.snap_to_pixel {
            return Point::new(p.x.round(), p.y.round());
        }
        if cx.snap_to_grid {
            return vectorcraft_geom::snap::snap_point_to_grid(p, cx.grid_step());
        }
        let Some(t) = &self.targets else { return p };
        let offsets: &[Vec2] = if bleed == Vec2::ZERO { &[] } else { &[bleed] };
        let (q, ov) = t.snap_point_with(p, offsets, cx.snap_tol());
        self.guides = ov;
        q
    }

    /// Snap a move (or an Alt-drag `copy`) of `rect` by `d`: its top-left to whole pixels or the
    /// grid, or any of its edges and centre to Smart Guides. Returns the snapped delta.
    fn snap_move(&mut self, cx: &ToolContext, rect: Rect, d: Vec2, copy: bool) -> Vec2 {
        self.guides.clear();
        let tl = Point::new(rect.x0, rect.y0);
        if cx.snap_to_pixel {
            return Vec2::new((tl.x + d.x).round() - tl.x, (tl.y + d.y).round() - tl.y);
        }
        if cx.snap_to_grid {
            return vectorcraft_geom::snap::snap_point_to_grid(tl + d, cx.grid_step()) - tl;
        }
        let Some(t) = (if copy { &self.copy_targets } else { &self.targets }) else { return d };
        let moved = rect + d;
        let (adj, ov) = if cx.doc.setup.has_bleed() {
            t.snap_rects(&[moved, cx.doc.setup.bleed_rect(moved)], cx.snap_tol())
        } else {
            t.snap_rect(moved, cx.snap_tol())
        };
        self.guides = ov;
        d + adj
    }

    fn end_drag(&mut self) {
        self.drag = None;
        self.click_art = None;
        self.preview = None;
        self.targets = None;
        self.copy_targets = None;
        self.guides.clear();
        self.draw.clear();
    }
}

/// An artboard's shortest side (as `artboard.setProps` keeps it).
const MIN_SIDE: f64 = 1.0;

fn rect_params(r: Rect) -> Value {
    json!({ "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height() })
}

fn create_artboard(r: Rect) -> Vec<Action> {
    vec![Action::Exec("artboard.new".into(), rect_params(r)), Action::Notify("created".into())]
}

/// The visible bounds of the topmost object under `p`, when they are large enough to be an artboard.
fn fittable_bounds(cx: &ToolContext, p: Point) -> Option<Rect> {
    vectorcraft_doc::hit::hit_test(cx.doc, p, cx.hit_options())
        .and_then(|h| cx.doc.bounds_of(&[h.top_object(cx.isolation)], true))
        .filter(|r| r.width() >= MIN_SIDE && r.height() >= MIN_SIDE && [r.x0, r.y0, r.x1, r.y1].iter().all(|n| n.is_finite()))
}

/// `r` grown about `anchor` until both sides are at least [`MIN_SIDE`], by one factor, so a
/// proportional resize keeps its proportions where the command would lengthen only the short side.
fn keep_min_side(r: Rect, anchor: Point) -> Rect {
    let short = r.width().min(r.height());
    if short >= MIN_SIDE || short <= 0.0 || !short.is_finite() {
        return r;
    }
    let k = MIN_SIDE / short;
    (Affine::translate(anchor.to_vec2()) * Affine::scale(k) * Affine::translate(-anchor.to_vec2())).transform_rect_bbox(r)
}

/// Where the bleed edge lies beyond a handle, along the axes the handle moves.
fn bleed_offset(cx: &ToolContext, rect: Rect, handle: Handle) -> Vec2 {
    let d = handle.pos(cx.doc.setup.bleed_rect(rect)) - handle.pos(rect);
    let (ax, ay) = handle.axes();
    Vec2::new(if ax { d.x } else { 0.0 }, if ay { d.y } else { 0.0 })
}

/// The grid's snapping step (gridline spacing over subdivisions), as `guides::snap_draw` uses.
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
        // Set by the canvas on the second click of a double-click, and only for the release that
        // follows. Any other event drops it, so a later single click still fits.
        let skip_fit = ev.kind == PointerKind::Up && self.suppress_fit;
        self.suppress_fit = false;
        if ev.kind == PointerKind::Drag && self.click_art.is_some_and(|(start, _)| m != Mods::default() || p.distance(start) >= cx.tol(3.0)) {
            self.click_art = None;
        }
        match (ev.kind, self.drag) {
            (PointerKind::Down, _) => {
                self.click_art = None;
                if let Some(r) = self.resizable_rect(cx)
                    && let Some(handle) = hit_handle(r, p, cx.tol(5.0))
                {
                    self.drag = Some(Drag::Resize { index: self.active, handle, rect: r, grab: handle.pos(r) - p, began: false });
                    return vec![];
                }
                if m == Mods::default() {
                    // Match Fit to Selected Art, and do not stretch zero-sized art to the
                    // artboard command's minimum side or pass non-finite bounds to it.
                    self.click_art = fittable_bounds(cx, p).map(|r| (p, r));
                }
                if m.shift {
                    // Shift-drag inside an artboard draws a new one there, and a Shift-click
                    // toggles the artboard on release. Shift-drag outside every artboard is a
                    // marquee: the artboards it touches toggle on release.
                    match cx.doc.artboard_at(p) {
                        Some(i) => {
                            let start = self.draw.press(cx, p, &[], None);
                            self.drag = Some(Drag::Create { start, cur: start, toggle: Some((i, p)) });
                        }
                        None => self.drag = Some(Drag::Marquee { start: p, cur: p }),
                    }
                    return vec![];
                }
                if let Some(i) = cx.doc.artboard_at(p)
                    && let Some(a) = cx.doc.artboards.get(i)
                {
                    if !self.selected(cx).contains(&i) {
                        // A plain click on an artboard that isn't selected selects just it; on a
                        // selected one, the selection stays, so the drag moves them all.
                        self.others.clear();
                        self.active = i;
                    }
                    let many = self.selected(cx).len() > 1 && self.selected(cx).contains(&i);
                    self.drag = Some(Drag::Move { index: i, many, start: p, rect: a.rect, began: false, copy: false });
                } else {
                    self.others.clear();
                    let p = self.draw.press(cx, p, &[], None);
                    self.drag = Some(Drag::Create { start: p, cur: p, toggle: None });
                }
                vec![]
            }
            (PointerKind::Drag, Some(Drag::Move { index, many, start, rect, began, copy })) => {
                let mut out = vec![];
                let label = |copy: bool| if copy { "Duplicate Artboard" } else { "Move Artboard" };
                let moving = self.movable(cx, index, many);
                if !began {
                    // A locked artboard doesn't move, and nor is it copied by an Alt-drag.
                    if p.distance(start) < cx.tol(3.0) || moving.is_empty() {
                        return out;
                    }
                    out.push(Action::Begin(label(m.alt).into()));
                    self.begin_snapping(cx, index, self.move_art);
                } else if m.alt != copy {
                    // Alt pressed or released mid-drag: start over as a copy or a move.
                    out.push(Action::Cancel);
                    out.push(Action::Begin(label(m.alt).into()));
                }
                self.drag = Some(Drag::Move { index, many, start, rect, began: true, copy: m.alt });
                let d = self.snap_move(cx, rect, p - start, m.alt);
                self.preview = Some(rect + d);
                let mut v = json!({ "index": index, "dx": d.x, "dy": d.y, "moveArt": self.move_art, "copy": m.alt });
                if many {
                    v["indices"] = json!(moving);
                }
                out.push(Action::Preview("artboard.move".into(), v));
                out
            }
            (PointerKind::Drag, Some(Drag::Resize { index, handle, rect, grab, began })) => {
                let mut out = vec![];
                if !began {
                    out.push(Action::Begin("Resize Artboard".into()));
                    self.begin_snapping(cx, index, self.scale_art);
                }
                self.drag = Some(Drag::Resize { index, handle, rect, grab, began: true });
                let h = self.snap_handle(cx, p + grab, bleed_offset(cx, rect, handle));
                let proportional = m.shift || self.scale_art;
                let mut nr = scale_for_drag(rect, handle, h, proportional, m.alt).transform_rect_bbox(rect);
                if proportional {
                    nr = keep_min_side(nr, if m.alt { rect.center() } else { handle.opposite().pos(rect) });
                }
                self.preview = Some(nr);
                let mut v = rect_json(index, nr);
                if self.scale_art {
                    v["scaleArt"] = json!(true);
                }
                out.push(Action::Preview("artboard.setProps".into(), v));
                out
            }
            (PointerKind::Drag, Some(Drag::Create { start, toggle, .. })) => {
                let cur = self.draw.drag(cx, p, Some(&Leave::diagonal(start, false)));
                self.drag = Some(Drag::Create { start, cur, toggle });
                vec![]
            }
            (PointerKind::Drag, Some(Drag::Marquee { start, .. })) => {
                self.drag = Some(Drag::Marquee { start, cur: p });
                vec![]
            }
            (PointerKind::Up, Some(d)) => {
                let click_art = self.click_art;
                self.end_drag();
                if m == Mods::default()
                    && !skip_fit
                    && let Some((start, r)) = click_art
                    && p.distance(start) < cx.tol(3.0)
                {
                    return create_artboard(r);
                }
                match d {
                    Drag::Move { index, began: true, copy, many, .. } => {
                        // The copies, last in the previewed document, become the selection (the
                        // last one active).
                        if copy {
                            let n = cx.doc.artboards.len();
                            let k = if many { self.movable(cx, index, many).len().min(n) } else { 1 };
                            self.active = n.saturating_sub(1);
                            self.others = (n.saturating_sub(k)..n.saturating_sub(1)).collect();
                        }
                        vec![Action::Commit]
                    }
                    Drag::Resize { began: true, .. } => vec![Action::Commit],
                    Drag::Create { toggle: Some((i, at)), .. } if p.distance(at) < cx.tol(3.0) => {
                        self.toggle(cx, i);
                        vec![]
                    }
                    Drag::Marquee { start, .. } => {
                        let r = Rect::from_points(start, p);
                        if r.width() >= cx.tol(3.0) || r.height() >= cx.tol(3.0) {
                            self.toggle_touched(cx, r);
                        }
                        vec![]
                    }
                    Drag::Create { start, cur, .. } => {
                        let r = Rect::from_points(start, cur);
                        if r.width() < cx.tol(3.0) || r.height() < cx.tol(3.0) {
                            return vec![];
                        }
                        create_artboard(r)
                    }
                    _ => vec![],
                }
            }
            (PointerKind::DoubleClick, _) => {
                self.end_drag();
                // Fitting already happened on the first click. On an object this gesture does not
                // open Artboard Options; a handle, a modifier, or empty artboard space still does.
                let on_handle = self.active_rect(cx).and_then(|r| hit_handle(r, p, cx.tol(5.0))).is_some();
                if m == Mods::default() && !on_handle && fittable_bounds(cx, p).is_some() {
                    return vec![];
                }
                match cx.doc.artboards.get(self.active) {
                    Some(a) => {
                        let mut v = rect_json(self.active, a.rect);
                        v["name"] = json!(a.name);
                        v["showCenterMark"] = json!(a.show_center_mark);
                        v["showCrossHairs"] = json!(a.show_cross_hairs);
                        v["showVideoSafeAreas"] = json!(a.show_video_safe_areas);
                        v["videoRulerPixelAspectRatio"] = json!(a.video_ruler_pixel_aspect_ratio);
                        vec![Action::Dialog("artboardOptions".into(), v)]
                    }
                    None => vec![],
                }
            }
            _ => vec![],
        }
    }

    /// Escape owns the request to return to Selection (or cancel an active drag), so the UI
    /// applies the tool's request instead of falling through to its general Escape behaviour.
    /// Delete/Backspace belong to the artboards even when deletion is protected or a drag is
    /// active: falling through to Clear would delete selected artwork instead. So do the arrows:
    /// they nudge the artboards, never the selected art on its own (#2103).
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::Escape | ToolKey::Delete | ToolKey::Backspace | ToolKey::Left | ToolKey::Right | ToolKey::Up | ToolKey::Down)
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Delete | ToolKey::Backspace => {
                // A locked artboard isn't deleted.
                let mut sel: Vec<usize> = self.selected(cx).into_iter().filter(|i| cx.doc.artboards.get(*i).is_some_and(|a| !a.locked)).collect();
                if self.drag.is_some() || cx.doc.artboards.len() <= 1 || self.active >= cx.doc.artboards.len() || sel.is_empty() {
                    return vec![];
                }
                // With every artboard selected, keep the first one in the current order.
                if sel.len() == cx.doc.artboards.len() {
                    sel.retain(|i| *i != 0);
                }
                self.active = sel.first().copied().unwrap_or(0).saturating_sub(1);
                self.others.clear();
                let params = match sel.as_slice() {
                    [i] => json!({ "index": i }),
                    _ => json!({ "indices": sel }),
                };
                vec![Action::Exec("artboard.delete".into(), params)]
            }
            ToolKey::Escape => {
                // During any drag (a move, a resize, a new artboard or a marquee) Escape only
                // cancels the gesture; the next one leaves the tool.
                let dragging = self.drag.is_some();
                let previewed = matches!(self.drag, Some(Drag::Move { began: true, .. } | Drag::Resize { began: true, .. }));
                self.end_drag();
                if previewed {
                    vec![Action::Cancel]
                } else if dragging {
                    vec![]
                } else {
                    vec![Action::SwitchTool("selection".into())]
                }
            }
            ToolKey::Left | ToolKey::Right | ToolKey::Up | ToolKey::Down => {
                // The selected artboards that aren't locked move by the keyboard increment (Shift:
                // ten times as far, as object.nudge moves art), with their art when Move Artwork
                // with Artboard is on, as a drag moves them: one step per press.
                let moving: Vec<usize> = self.selected(cx).into_iter().filter(|i| cx.doc.artboards.get(*i).is_some_and(|a| !a.locked)).collect();
                if self.drag.is_some() || moving.is_empty() {
                    return vec![];
                }
                let step = cx.keyboard_increment * if mods.shift { 10.0 } else { 1.0 };
                let (dx, dy) = match key {
                    ToolKey::Left => (-step, 0.0),
                    ToolKey::Right => (step, 0.0),
                    ToolKey::Up => (0.0, -step),
                    _ => (0.0, step),
                };
                let mut v = json!({ "dx": dx, "dy": dy, "moveArt": self.move_art });
                match moving.as_slice() {
                    [i] => v["index"] = json!(i),
                    _ => v["indices"] = json!(moving),
                }
                vec![Action::Exec("artboard.move".into(), v)]
            }
            _ => vec![],
        }
    }

    fn notify(&mut self, cx: &ToolContext, what: &str) {
        if what == "created" {
            self.active = cx.doc.artboards.len().saturating_sub(1);
            self.others.clear();
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = vec![];
        if let Some(Drag::Create { start, cur, .. }) = self.drag {
            o.extend_from_slice(self.draw.guides());
            let r = Rect::from_points(start, cur);
            o.push(Overlay::Marquee(r));
            if cx.measurement_labels {
                o.push(Overlay::Measure { p: cur + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: cx.size_label(r.width(), r.height()) });
            }
            return o;
        }
        if let Some(Drag::Marquee { start, cur }) = self.drag {
            o.push(Overlay::Marquee(Rect::from_points(start, cur)));
        }
        // The other selected artboards: dashed bounds (in a drag the document shows them moved).
        for r in self.others.iter().filter_map(|i| cx.doc.artboards.get(*i)).map(|a| a.rect) {
            o.push(Overlay::Path { path: polygon(&rect_corners(r), true), color: BLUE, width: 1.0, dashed: true });
        }
        let Some(ab) = cx.doc.artboards.get(self.active) else { return o };
        let r = self.preview.unwrap_or(ab.rect);
        o.push(Overlay::Path { path: polygon(&rect_corners(r), true), color: BLUE, width: 1.0, dashed: true });
        // A locked artboard has no handles: it can't be resized.
        if !ab.locked {
            for h in Handle::ALL {
                o.push(Overlay::Anchor { p: h.pos(r), color: BLUE, filled: false, size: 7.0 });
            }
        }
        // Where the canvas names the other artboards: on the top edge, flush with the left one
        // (labels are drawn 8 px right of and 14 px above their point).
        let at = Point::new(r.x0 - cx.tol(8.0), r.y0 - cx.tol(4.0));
        o.push(Overlay::Label { p: at, text: format!("{:02} - {}", self.active + 1, ab.name), color: BLUE });
        if let (Some(Drag::Resize { began: true, .. }), true) = (self.drag, cx.transform_tools_guides) {
            o.push(Overlay::Measure {
                p: Point::new(r.x1, r.y1) + Vec2::new(cx.tol(12.0), cx.tol(12.0)),
                text: cx.size_label(r.width(), r.height()),
            });
        }
        if self.drag.is_some() {
            o.extend(self.guides.iter().cloned());
        }
        o
    }

    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        if let Some(r) = self.resizable_rect(cx)
            && let Some(h) = hit_handle(r, p, cx.tol(5.0))
        {
            return match h {
                Handle::Top | Handle::Bottom => Cursor::ResizeV,
                Handle::Left | Handle::Right => Cursor::ResizeH,
                Handle::TopLeft | Handle::BottomRight => Cursor::ResizeNwSe,
                Handle::TopRight | Handle::BottomLeft => Cursor::ResizeNeSw,
            }
            .turned(cx.view_angle());
        }
        // With Shift a drag draws or selects, never moves.
        if m.shift {
            return Cursor::Crosshair;
        }
        // The artboard a press would move (the topmost under the pointer, as `artboard_at` finds
        // it): a locked one shows the arrow.
        match cx.doc.artboard_at(p).and_then(|i| cx.doc.artboards.get(i)) {
            Some(a) if a.locked => Cursor::Arrow,
            Some(_) => Cursor::Move,
            None => Cursor::Crosshair,
        }
    }

    fn options(&self) -> Value {
        json!({ "active": self.active, "selected": std::iter::once(self.active).chain(self.others.iter().copied()).collect::<Vec<_>>(), "moveArt": self.move_art, "scaleArt": self.scale_art })
    }

    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "active" => {
                self.active = value.as_u64().unwrap_or(0) as usize;
                self.others.clear();
            }
            // The selected artboards, the first active.
            "selected" => {
                let all: Vec<usize> =
                    value.as_array().into_iter().flatten().filter_map(Value::as_u64).filter_map(|i| usize::try_from(i).ok()).collect();
                if let Some((first, rest)) = all.split_first() {
                    self.active = *first;
                    self.others = rest.iter().copied().filter(|i| i != first).collect();
                }
            }
            "moveArt" => self.move_art = value.as_bool().unwrap_or(true),
            "scaleArt" => self.scale_art = value.as_bool().unwrap_or(false),
            // Not one of the saved options: the canvas sets it for the release of a double-click.
            "suppressFitClick" => self.suppress_fit = value.as_bool().unwrap_or(false),
            _ => {}
        }
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        let busy = matches!(self.drag, Some(Drag::Move { began: true, .. } | Drag::Resize { began: true, .. }));
        self.end_drag();
        if busy { vec![Action::Cancel] } else { vec![] }
    }
}

#[cfg(test)]
mod tests;
