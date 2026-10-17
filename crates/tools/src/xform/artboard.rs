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
//! the active one (Copy, Cut and Paste take it with its art: `artboard.copy`) and Escape returns to
//! the Selection tool (during a drag it only cancels the drag). Moving and resizing snap like drawing
//! does: to whole pixels, to the grid, or with Smart Guides to other artboards, their bleed and
//! objects (never to the dragged artboard or the art moving or scaling with it); the artboard's own bleed edges
//! snap too. A new artboard's corners snap as drawn points do ([`DrawSnap`]).

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
            let art = match cx.doc.artboards.get(index) {
                Some(a) if art_moves => cx.doc.art_on_artboard(a.rect, cx.move_locked_with_artboard),
                _ => vec![],
            };
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
                if let Some(r) = self.active_rect(cx)
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
                if !began {
                    if p.distance(start) < cx.tol(3.0) {
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
                    v["indices"] = json!(self.selected(cx));
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
                    Drag::Move { began: true, copy, many, .. } => {
                        // The copies, last in the previewed document, become the selection (the
                        // last one active).
                        if copy {
                            let n = cx.doc.artboards.len();
                            let k = if many { self.selected(cx).len().min(n) } else { 1 };
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
    /// active: falling through to Clear would delete selected artwork instead.
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::Escape | ToolKey::Delete | ToolKey::Backspace)
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Delete | ToolKey::Backspace => {
                let mut sel = self.selected(cx);
                if self.drag.is_some() || cx.doc.artboards.len() <= 1 || self.active >= cx.doc.artboards.len() {
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
        for h in Handle::ALL {
            o.push(Overlay::Anchor { p: h.pos(r), color: BLUE, filled: false, size: 7.0 });
        }
        // Where the canvas names the other artboards: on the top edge, flush with the left one
        // (labels are drawn 8 px right of and 14 px above their point).
        let at = Point::new(r.x0 - cx.tol(8.0), r.y0 - cx.tol(4.0));
        o.push(Overlay::Label { p: at, text: format!("{:02} - {}", self.active + 1, ab.name), color: BLUE });
        if let (Some(Drag::Resize { began: true, .. }), true) = (self.drag, cx.measurement_labels) {
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
        if let Some(r) = self.active_rect(cx)
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
        if !m.shift && cx.doc.artboard_at(p).is_some() { Cursor::Move } else { Cursor::Crosshair }
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
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    /// [`doc_with_rect`] (artboard 1 at 0–500, a rectangle at 100–200) plus Artboard 2 at x 600–800,
    /// y 0–200.
    fn two_boards() -> vectorcraft_doc::Document {
        let (mut d, _) = doc_with_rect();
        let mut ab = d.artboards[0].clone();
        ab.rect = Rect::new(600.0, 0.0, 800.0, 200.0);
        ab.name = "Artboard 2".into();
        d.artboards.push(ab);
        d
    }

    fn preview_params(a: &[Action]) -> Value {
        a.iter()
            .find_map(|a| match a {
                Action::Preview(_, p) => Some(p.clone()),
                _ => None,
            })
            .unwrap()
    }

    /// Drag artboard 2 from (700, 100) by (dx, dy) and return the preview's params.
    fn drag_board2(cx: &ToolContext, t: &mut ArtboardTool, dx: f64, dy: f64) -> Value {
        t.pointer(cx, &ev(PointerKind::Down, 700.0, 100.0));
        preview_params(&t.pointer(cx, &ev(PointerKind::Drag, 700.0 + dx, 100.0 + dy)))
    }

    #[test]
    fn bounds_follow_the_artboard_while_the_document_shows_the_preview() {
        let mut d = two_boards();
        let (s, p) = (Selection::default(), paint());
        let mut t = ArtboardTool::default();
        assert_eq!(drag_board2(&cx(&d, &s, &p), &mut t, 10.0, 0.0)["dx"], 10.0);
        // The engine applies the preview to the document, as it does during a drag.
        d.artboards[1].rect = Rect::new(610.0, 0.0, 810.0, 200.0);
        let cx = cx(&d, &s, &p);
        assert_eq!(preview_params(&t.pointer(&cx, &ev(PointerKind::Drag, 720.0, 100.0)))["dx"], 20.0);
        let corners: Vec<Point> = t
            .overlays(&cx)
            .iter()
            .filter_map(|o| match o {
                Overlay::Anchor { p, .. } => Some(*p),
                _ => None,
            })
            .collect();
        assert_eq!(corners[0], Point::new(620.0, 0.0), "the handles sit on the moved artboard, not 10 pt past it");
    }

    #[test]
    fn moving_snaps_to_other_artboards_with_smart_guides() {
        let d = two_boards();
        let (s, p) = (Selection::default(), paint());
        let mut t = ArtboardTool::default();
        let mut c = cx(&d, &s, &p);
        // Left edge 3 pt right of artboard 1's right edge (500) → lands on it.
        assert_eq!(drag_board2(&c, &mut t, -97.0, 0.0)["dx"], -100.0);
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Line { color, .. } if *color == crate::guides::MAGENTA)));
        t.pointer(&c, &ev(PointerKind::Up, 603.0, 100.0));
        assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Line { .. })), "guides go away on release");
        c.smart_guides = false;
        assert_eq!(drag_board2(&c, &mut t, -97.0, 0.0)["dx"], -97.0);
    }

    #[test]
    fn moving_never_snaps_to_itself_or_the_art_it_carries() {
        let mut d = two_boards();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let art = vectorcraft_geom::shapes::rectangle(Rect::new(650.0, 50.0, 660.0, 60.0));
        d.insert(Some(l), 0, vectorcraft_doc::Node::path(id, art, vectorcraft_doc::Appearance::default_art())).unwrap();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = ArtboardTool::default();
        // Its own old left edge (600) isn't a target.
        assert_eq!(drag_board2(&c, &mut t, 4.0, 0.0)["dx"], 4.0);
        t.pointer(&c, &ev(PointerKind::Up, 704.0, 100.0));
        // The rectangle moves along: its left edge (650) isn't a target either…
        assert_eq!(drag_board2(&c, &mut t, 52.0, 0.0)["dx"], 52.0);
        t.pointer(&c, &ev(PointerKind::Up, 752.0, 100.0));
        // …unless it stays behind.
        t.move_art = false;
        assert_eq!(drag_board2(&c, &mut t, 52.0, 0.0)["dx"], 50.0);
    }

    #[test]
    fn moving_snaps_to_grid_and_pixels() {
        let d = two_boards();
        let (s, p) = (Selection::default(), paint());
        let mut t = ArtboardTool::default();
        let mut c = cx(&d, &s, &p);
        c.snap_to_grid = true;
        // Grid step 72 / 8 = 9: x 610 → 612, y 4 → 0.
        let v = drag_board2(&c, &mut t, 10.0, 4.0);
        assert_eq!((v["dx"].as_f64(), v["dy"].as_f64()), (Some(12.0), Some(0.0)));
        t.pointer(&c, &ev(PointerKind::Up, 710.0, 104.0));
        c.snap_to_grid = false;
        c.snap_to_pixel = true;
        let v = drag_board2(&c, &mut t, 10.4, 3.6);
        assert_eq!((v["dx"].as_f64(), v["dy"].as_f64()), (Some(10.0), Some(4.0)));
    }

    #[test]
    fn resizing_snaps_the_dragged_handle() {
        let d = two_boards();
        let (s, p) = (Selection::default(), paint());
        let mut t = ArtboardTool { active: 1, ..Default::default() };
        let c = cx(&d, &s, &p);
        // Left handle to 3 pt off artboard 1's right edge.
        t.pointer(&c, &ev(PointerKind::Down, 600.0, 100.0));
        let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 503.0, 100.0)));
        assert_eq!((v["x"].as_f64(), v["width"].as_f64()), (Some(500.0), Some(300.0)));
        t.pointer(&c, &ev(PointerKind::Up, 503.0, 100.0));
        // The handle, not the pointer, follows: grabbed 2 pt left of it, dragged 10 pt.
        t.pointer(&c, &ev(PointerKind::Down, 798.0, 100.0));
        let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 808.0, 100.0)));
        assert_eq!(v["width"].as_f64(), Some(210.0));
    }

    /// In a view turned 90° the handles' cursors turn with the view.
    #[test]
    fn handle_cursors_turn_with_the_view() {
        let d = two_boards();
        let (s, p) = (Selection::default(), paint());
        let t = ArtboardTool { active: 1, ..Default::default() };
        let mut c = cx(&d, &s, &p);
        let (top, right) = (Point::new(700.0, 0.0), Point::new(800.0, 100.0));
        assert_eq!(t.cursor(&c, top, Mods::default()), Cursor::ResizeV);
        c.screen = turned_view(90.0);
        assert_eq!(t.cursor(&c, top, Mods::default()), Cursor::ResizeH);
        assert_eq!(t.cursor(&c, right, Mods::default()), Cursor::ResizeV);
    }

    /// Shift-click selects several artboards (#1139): a drag moves them all, Shift-click takes one
    /// out again, a plain click on an unselected one selects just it, and Delete removes all the
    /// selected ones (one artboard is always left).
    #[test]
    fn shift_click_selects_several_artboards_that_move_and_delete_together() {
        let mut d = two_boards();
        let mut third = d.artboards[0].clone();
        third.rect = Rect::new(0.0, 600.0, 200.0, 800.0);
        d.artboards.push(third);
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Default::default() };
        let click = |t: &mut ArtboardTool, x: f64, y: f64, m: Mods| {
            t.pointer(&c, &ev(PointerKind::Down, x, y).with_mods(m));
            t.pointer(&c, &ev(PointerKind::Up, x, y).with_mods(m));
        };
        let mut t = ArtboardTool::default();
        click(&mut t, 450.0, 450.0, Mods::default());
        click(&mut t, 700.0, 100.0, shift);
        assert_eq!(t.options()["selected"], json!([1, 0]), "the Shift-clicked one is active");
        // A drag from either moves both.
        t.pointer(&c, &ev(PointerKind::Down, 450.0, 450.0));
        let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 470.0, 450.0)));
        assert_eq!((v["indices"].clone(), v["dx"].as_f64()), (json!([0, 1]), Some(20.0)));
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 470.0, 450.0)), vec![Action::Commit]);
        // Delete removes both; with all three selected the keys retain the original first board.
        assert_eq!(t.key(&c, ToolKey::Delete, Mods::default()), vec![Action::Exec("artboard.delete".into(), json!({"indices": [0, 1]}))]);
        for key in [ToolKey::Delete, ToolKey::Backspace] {
            t.set_option("selected", &json!([2, 0, 1]));
            assert!(t.claims_key(&c, key));
            assert_eq!(t.key(&c, key, Mods::default()), vec![Action::Exec("artboard.delete".into(), json!({"indices": [1, 2]}))]);
            assert_eq!(t.options()["selected"], json!([0]));
        }
        t.set_option("selected", &json!([0, 1, 2]));
        // Shift-click takes one out; a plain click on an unselected one selects just it.
        click(&mut t, 100.0, 700.0, shift);
        assert_eq!(t.options()["selected"], json!([0, 1]));
        click(&mut t, 100.0, 700.0, Mods::default());
        assert_eq!(t.options()["selected"], json!([2]));
    }

    /// A Shift-click toggles its artboard when the button comes up, so the press alone leaves the
    /// selection as it was.
    #[test]
    fn a_shift_click_toggles_its_artboard_on_release() {
        let d = two_boards();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Default::default() };
        let mut t = ArtboardTool::default();
        assert_eq!(t.pointer(&c, &ev(PointerKind::Down, 700.0, 100.0).with_mods(shift)), vec![]);
        assert_eq!(t.options()["selected"], json!([0]), "the press alone selects nothing");
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 701.0, 100.0).with_mods(shift)), vec![]);
        assert_eq!(t.options()["selected"], json!([1, 0]));
        for kind in [PointerKind::Down, PointerKind::Up] {
            t.pointer(&c, &ev(kind, 700.0, 100.0).with_mods(shift));
        }
        assert_eq!(t.options()["selected"], json!([0]), "a second Shift-click takes it out");
    }

    /// Shift-drag inside an artboard draws a new artboard there, neither square nor moving the
    /// artboard it starts in, and the cursor shows a crosshair while Shift is held.
    #[test]
    fn a_shift_drag_inside_an_artboard_draws_a_new_one() {
        let d = two_boards();
        let (s, p) = (Selection::default(), paint());
        let mut c = cx(&d, &s, &p);
        c.smart_guides = false;
        let shift = Mods { shift: true, ..Default::default() };
        let mut t = ArtboardTool::default();
        assert_eq!(t.cursor(&c, Point::new(300.0, 300.0), Mods::default()), Cursor::Move);
        assert_eq!(t.cursor(&c, Point::new(300.0, 300.0), shift), Cursor::Crosshair);
        assert_eq!(t.pointer(&c, &ev(PointerKind::Down, 300.0, 300.0).with_mods(shift)), vec![]);
        assert_eq!(t.pointer(&c, &ev(PointerKind::Drag, 420.0, 380.0).with_mods(shift)), vec![], "no move preview");
        assert!(matches!(t.overlays(&c)[0], Overlay::Marquee(_)));
        let a = t.pointer(&c, &ev(PointerKind::Up, 420.0, 380.0).with_mods(shift));
        assert_eq!(a, create_artboard(Rect::new(300.0, 300.0, 420.0, 380.0)));
        assert_eq!(t.options()["selected"], json!([0]), "the artboard drawn in is not toggled");
    }

    /// Shift-drag on the pasteboard is a marquee: the artboards it touches leave the selection
    /// or join it, the last one joining becoming active, and no artboard is drawn.
    #[test]
    fn a_shift_drag_on_the_pasteboard_selects_the_artboards_it_touches() {
        let mut d = two_boards();
        let mut third = d.artboards[0].clone();
        third.rect = Rect::new(0.0, 600.0, 200.0, 800.0);
        d.artboards.push(third);
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Default::default() };
        let mut t = ArtboardTool::default();
        let marquee = |t: &mut ArtboardTool, from: (f64, f64), to: (f64, f64)| {
            assert_eq!(t.pointer(&c, &ev(PointerKind::Down, from.0, from.1).with_mods(shift)), vec![]);
            assert_eq!(t.pointer(&c, &ev(PointerKind::Drag, to.0, to.1).with_mods(shift)), vec![]);
            assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Marquee(_))));
            t.pointer(&c, &ev(PointerKind::Up, to.0, to.1).with_mods(shift))
        };
        // From between the artboards across Artboard 1 (selected) and Artboard 3: 1 leaves, 3 joins.
        assert_eq!(marquee(&mut t, (550.0, 150.0), (150.0, 650.0)), vec![]);
        assert_eq!(t.options()["selected"], json!([2]));
        // Right of Artboard 2 across its corner: it joins and becomes active.
        assert_eq!(marquee(&mut t, (850.0, -50.0), (700.0, 100.0)), vec![]);
        assert_eq!(t.options()["selected"], json!([1, 2]));
        // A Shift-click on the pasteboard changes nothing.
        assert_eq!(marquee(&mut t, (550.0, 150.0), (551.0, 150.0)), vec![]);
        assert_eq!(t.options()["selected"], json!([1, 2]));
    }

    /// A drag begun without Shift still moves the artboard, and on the pasteboard still draws one;
    /// Shift pressed during either no longer keeps the move at 45° or the new artboard square.
    #[test]
    fn a_plain_drag_still_moves_and_shift_no_longer_constrains_it() {
        let d = two_boards();
        let (s, p) = (Selection::default(), paint());
        let mut c = cx(&d, &s, &p);
        c.smart_guides = false;
        let shift = Mods { shift: true, ..Default::default() };
        let mut t = ArtboardTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 700.0, 100.0));
        let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 730.0, 110.0).with_mods(shift)));
        assert_eq!((v["index"].as_u64(), v["dx"].as_f64(), v["dy"].as_f64()), (Some(1), Some(30.0), Some(10.0)));
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 730.0, 110.0).with_mods(shift)), vec![Action::Commit]);
        t.pointer(&c, &ev(PointerKind::Down, 1000.0, 1000.0));
        t.pointer(&c, &ev(PointerKind::Drag, 1100.0, 1050.0).with_mods(shift));
        let a = t.pointer(&c, &ev(PointerKind::Up, 1100.0, 1050.0).with_mods(shift));
        assert_eq!(a, create_artboard(Rect::new(1000.0, 1000.0, 1100.0, 1050.0)));
    }

    /// Escape during a marquee or while a new artboard is being drawn ends the gesture without
    /// selecting or drawing anything and keeps the tool; the next Escape leaves it.
    #[test]
    fn escape_cancels_a_marquee_or_a_new_artboard_and_keeps_the_tool() {
        let d = two_boards();
        let (s, p) = (Selection::default(), paint());
        let mut c = cx(&d, &s, &p);
        c.smart_guides = false;
        let shift = Mods { shift: true, ..Default::default() };
        let mut t = ArtboardTool::default();
        // A Shift-drag marquee on the pasteboard, a Shift-drag inside Artboard 1 and a plain drag on
        // the pasteboard.
        let drags =
            [((850.0, -50.0), (700.0, 100.0), shift), ((300.0, 300.0), (420.0, 380.0), shift), ((1000.0, 1000.0), (1100.0, 1050.0), Mods::default())];
        for (from, to, mods) in drags {
            t.pointer(&c, &ev(PointerKind::Down, from.0, from.1).with_mods(mods));
            t.pointer(&c, &ev(PointerKind::Drag, to.0, to.1).with_mods(mods));
            assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![], "Escape from {from:?} only ends the gesture");
            assert!(!t.busy());
            assert_eq!(t.pointer(&c, &ev(PointerKind::Up, to.0, to.1).with_mods(mods)), vec![], "the release draws nothing");
            assert_eq!(t.options()["selected"], json!([0]), "the release selects nothing");
        }
        assert_eq!(t.key(&c, ToolKey::Escape, Mods::default()), vec![Action::SwitchTool("selection".into())]);
    }

    /// Scale Artwork with Artboard (#602): the resize asks for the art to scale, keeps the artboard's
    /// proportions, and never snaps to the art scaling with it.
    #[test]
    fn scale_art_resizes_proportionally_without_snapping_to_its_art() {
        let mut d = two_boards();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let art = vectorcraft_geom::shapes::rectangle(Rect::new(700.0, 50.0, 710.0, 60.0));
        d.insert(Some(l), 0, vectorcraft_doc::Node::path(id, art, vectorcraft_doc::Appearance::default_art())).unwrap();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        // Off: the right handle 3 pt past the art's right edge (710) lands on it; the height stays.
        let mut t = ArtboardTool { active: 1, ..Default::default() };
        t.pointer(&c, &ev(PointerKind::Down, 800.0, 100.0));
        let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 713.0, 100.0)));
        assert_eq!((v["width"].as_f64(), v["height"].as_f64(), v.get("scaleArt")), (Some(110.0), Some(200.0), None));
        t.pointer(&c, &ev(PointerKind::Up, 713.0, 100.0));
        // On: the art scales along, so it's no target, and the height follows the width.
        t.set_option("scaleArt", &json!(true));
        assert_eq!(t.options()["scaleArt"], true);
        t.pointer(&c, &ev(PointerKind::Down, 800.0, 100.0));
        let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 713.0, 100.0)));
        assert_eq!((v["width"].as_f64(), v["scaleArt"].as_bool()), (Some(113.0), Some(true)));
        assert!((v["height"].as_f64().unwrap() - 113.0).abs() < 1e-9);
    }

    /// Over an artboard's right or bottom edge the cursor shows Move and a click selects that
    /// artboard, as on its left or top edge.
    #[test]
    fn a_click_on_an_artboards_right_or_bottom_edge_selects_it() {
        let d = two_boards();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        for at in [Point::new(800.0, 100.0), Point::new(700.0, 200.0)] {
            let mut t = ArtboardTool::default();
            assert_eq!(t.cursor(&c, at, Mods::default()), Cursor::Move, "{at:?}");
            t.pointer(&c, &ev(PointerKind::Down, at.x, at.y));
            t.pointer(&c, &ev(PointerKind::Up, at.x, at.y));
            assert_eq!(t.options()["selected"], json!([1]), "{at:?}");
        }
    }

    /// A proportional resize never asks for a side under 1 pt, which the command would lengthen
    /// alone (scaling the art unevenly): both sides grow by one factor instead.
    #[test]
    fn a_proportional_resize_keeps_its_proportions_at_the_smallest_size() {
        let mut d = two_boards();
        d.artboards[1].rect = Rect::new(600.0, 0.0, 800.0, 20.0);
        let (s, p) = (Selection::default(), paint());
        let mut c = cx(&d, &s, &p);
        c.smart_guides = false;
        let mut t = ArtboardTool { active: 1, scale_art: true, ..Default::default() };
        // The right handle to 2.5 % of the width: 5 × 0.5, grown to 10 × 1 about the left middle.
        t.pointer(&c, &ev(PointerKind::Down, 800.0, 10.0));
        let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 605.0, 10.0)));
        let got = ["x", "y", "width", "height"].map(|k| v[k].as_f64().unwrap());
        assert!(got.iter().zip([600.0, 9.5, 10.0, 1.0]).all(|(a, b)| (a - b).abs() < 1e-9), "{got:?}");
    }

    #[test]
    fn bleed_edges_snap_to_bleed_edges() {
        let mut d = two_boards();
        d.setup.bleed = [10.0; 4];
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = ArtboardTool::default();
        // Artboard 2's bleed (left edge 590) lands on artboard 1's bleed (right edge 510): dragged
        // 77 pt left it's 3 pt off, nothing else is within reach.
        assert_eq!(drag_board2(&c, &mut t, -77.0, 0.0)["dx"], -80.0);
        t.pointer(&c, &ev(PointerKind::Up, 623.0, 100.0));
        // Resizing: the left handle's bleed edge snaps the same way.
        t.pointer(&c, &ev(PointerKind::Down, 600.0, 100.0));
        let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 523.0, 100.0)));
        assert_eq!((v["x"].as_f64(), v["width"].as_f64()), (Some(520.0), Some(280.0)));
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
        assert_eq!(a[1], Action::Preview("artboard.move".into(), json!({"index": 1, "dx": 10.0, "dy": 20.0, "moveArt": true, "copy": false})));
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
        // Delete removes the active artboard, ahead of the Clear shortcut.
        assert!(t.claims_key(&cx, ToolKey::Delete) && t.claims_key(&cx, ToolKey::Backspace));
        assert_eq!(t.key(&cx, ToolKey::Delete, Mods::default()), vec![Action::Exec("artboard.delete".into(), json!({"index": 1}))]);
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::SwitchTool("selection".into())]);
    }

    #[test]
    fn an_alt_drag_copy_snaps_to_the_artboard_it_leaves_behind() {
        let d = two_boards();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Mods::default() };
        let mut t = ArtboardTool::default();
        // A copy of artboard 2 dragged 203 pt right: its left edge 3 pt off the original's right
        // edge (800), which stays put and is a target.
        t.pointer(&c, &ev(PointerKind::Down, 700.0, 100.0).with_mods(alt));
        let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 903.0, 100.0).with_mods(alt)));
        assert_eq!((v["dx"].as_f64(), v["copy"].as_bool()), (Some(200.0), Some(true)));
        // Released Alt: a plain move, and the artboard's own old edge is no target.
        let v = preview_params(&t.pointer(&c, &ev(PointerKind::Drag, 903.0, 100.0)));
        assert_eq!((v["dx"].as_f64(), v["copy"].as_bool()), (Some(203.0), Some(false)));
    }

    #[test]
    fn alt_drag_moves_a_copy_of_the_artboard() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Mods::default() };
        let moved =
            |dx: f64, copy: bool| Action::Preview("artboard.move".into(), json!({"index": 0, "dx": dx, "dy": 0.0, "moveArt": true, "copy": copy}));
        let mut t = ArtboardTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 100.0, 100.0).with_mods(alt));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 100.0).with_mods(alt));
        assert_eq!(a, vec![Action::Begin("Duplicate Artboard".into()), moved(50.0, true)]);
        // Alt released mid-drag: a plain move after all, and back.
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 160.0, 100.0));
        assert_eq!(a, vec![Action::Cancel, Action::Begin("Move Artboard".into()), moved(60.0, false)]);
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 170.0, 100.0).with_mods(alt));
        assert_eq!(a, vec![Action::Cancel, Action::Begin("Duplicate Artboard".into()), moved(70.0, true)]);
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 170.0, 100.0).with_mods(alt)), vec![Action::Commit]);
    }
}
