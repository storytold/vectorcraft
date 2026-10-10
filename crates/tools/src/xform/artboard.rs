//! Artboard tool (Shift+O).
//!
//! Click an artboard to make it active (dashed bounds, 8 handles and its name). Drag inside moves
//! it (with its artwork when the `moveArt` option is on, Shift constrains; Alt moves a copy and
//! leaves the artboard where it was), drag a handle resizes it
//! (Shift proportional, Alt from centre; with the `scaleArt` option on, Scale Artwork with Artboard,
//! always proportional, and the art fully inside scales with it), drag on the pasteboard draws a new artboard, Delete removes
//! the active one (Copy, Cut and Paste take it with its art: `artboard.copy`) and Escape returns to
//! the Selection tool. Moving and resizing snap like drawing
//! does: to whole pixels, to the grid, or with Smart Guides to other artboards, their bleed and
//! objects (never to the dragged artboard or the art moving or scaling with it); the artboard's own bleed edges
//! snap too. A new artboard's corners snap as drawn points do ([`DrawSnap`]).

use serde_json::{Value, json};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use super::{BLUE, polygon, rect_corners};
use crate::bbox::{Handle, hit_handle, move_delta, scale_for_drag};
use crate::guides::{DrawSnap, Leave, Targets};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

#[derive(Clone, Copy, Debug)]
enum Drag {
    /// `rect` is the artboard as it was when the drag started: during the drag the document
    /// already shows the preview, so it can't be read back from there. `copy`: Alt was held at the
    /// last drag event (the step under way is a duplicate).
    Move {
        index: usize,
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
    },
}

pub struct ArtboardTool {
    /// Index of the active artboard.
    pub active: usize,
    /// Move artwork with the artboard.
    pub move_art: bool,
    /// Scale artwork with the artboard: a resize takes the art fully inside it along, and keeps the
    /// artboard's proportions.
    pub scale_art: bool,
    drag: Option<Drag>,
    preview: Option<Rect>,
    /// Smart Guide targets, gathered when a move or resize starts.
    targets: Option<Targets>,
    /// The targets for an Alt-drag copy: the original artboard and its art stay where they are.
    copy_targets: Option<Targets>,
    /// Smart Guides shown while dragging.
    guides: Vec<Overlay>,
    /// Smart Guides for the corners of a new artboard.
    draw: DrawSnap,
}

impl Default for ArtboardTool {
    fn default() -> Self {
        Self {
            active: 0,
            move_art: true,
            scale_art: false,
            drag: None,
            preview: None,
            targets: None,
            copy_targets: None,
            guides: vec![],
            draw: DrawSnap::default(),
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
        self.preview = None;
        self.targets = None;
        self.copy_targets = None;
        self.guides.clear();
        self.draw.clear();
    }
}

/// An artboard's shortest side (as `artboard.setProps` keeps it).
const MIN_SIDE: f64 = 1.0;

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
        match (ev.kind, self.drag) {
            (PointerKind::Down, _) => {
                if let Some(r) = self.active_rect(cx)
                    && let Some(handle) = hit_handle(r, p, cx.tol(5.0))
                {
                    self.drag = Some(Drag::Resize { index: self.active, handle, rect: r, grab: handle.pos(r) - p, began: false });
                    return vec![];
                }
                if let Some(i) = cx.doc.artboard_at(p)
                    && let Some(a) = cx.doc.artboards.get(i)
                {
                    self.active = i;
                    self.drag = Some(Drag::Move { index: i, start: p, rect: a.rect, began: false, copy: false });
                } else {
                    let p = self.draw.press(cx, p, &[], None);
                    self.drag = Some(Drag::Create { start: p, cur: p });
                }
                vec![]
            }
            (PointerKind::Drag, Some(Drag::Move { index, start, rect, began, copy })) => {
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
                self.drag = Some(Drag::Move { index, start, rect, began: true, copy: m.alt });
                let d = self.snap_move(cx, rect, move_delta(start, p, m.shift), m.alt);
                self.preview = Some(rect + d);
                out.push(Action::Preview(
                    "artboard.move".into(),
                    json!({ "index": index, "dx": d.x, "dy": d.y, "moveArt": self.move_art, "copy": m.alt }),
                ));
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
            (PointerKind::Drag, Some(Drag::Create { start, .. })) => {
                let cur = self.draw.drag(cx, p, Some(&Leave::diagonal(start, m.shift)));
                self.drag = Some(Drag::Create { start, cur });
                vec![]
            }
            (PointerKind::Up, Some(d)) => {
                self.end_drag();
                match d {
                    Drag::Move { began: true, copy, .. } => {
                        // The copy, last in the previewed document, becomes the active artboard.
                        if copy {
                            self.active = cx.doc.artboards.len().saturating_sub(1);
                        }
                        vec![Action::Commit]
                    }
                    Drag::Resize { began: true, .. } => vec![Action::Commit],
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
                self.end_drag();
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

    /// Delete/Backspace remove the active artboard ahead of the Clear shortcut, which would only
    /// delete selected art; with one artboard left (it can't be deleted) they stay the shortcut's.
    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::Delete | ToolKey::Backspace)
            && self.drag.is_none()
            && cx.doc.artboards.len() > 1
            && self.active < cx.doc.artboards.len()
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Delete | ToolKey::Backspace => {
                if !self.claims_key(cx, key) {
                    return vec![];
                }
                let i = self.active;
                self.active = i.saturating_sub(1);
                vec![Action::Exec("artboard.delete".into(), json!({ "index": i }))]
            }
            ToolKey::Escape => {
                let busy = matches!(self.drag, Some(Drag::Move { began: true, .. } | Drag::Resize { began: true, .. }));
                self.end_drag();
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
            o.extend_from_slice(self.draw.guides());
            let r = Rect::from_points(start, cur);
            o.push(Overlay::Marquee(r));
            if cx.measurement_labels {
                o.push(Overlay::Measure { p: cur + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: cx.size_label(r.width(), r.height()) });
            }
            return o;
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
        json!({ "active": self.active, "moveArt": self.move_art, "scaleArt": self.scale_art })
    }

    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "active" => self.active = value.as_u64().unwrap_or(0) as usize,
            "moveArt" => self.move_art = value.as_bool().unwrap_or(true),
            "scaleArt" => self.scale_art = value.as_bool().unwrap_or(false),
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
