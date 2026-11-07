//! Grid Repeat's on-canvas handles (#2691): a handle below the grid adds or removes rows, one right
//! of it columns, one past its bottom-right corner both, and a slider between the first two
//! columns (rows) sets the horizontal (vertical) spacing, as the public "Set grid repeat options"
//! page describes. Drawing and hit testing share the document-space geometry of
//! [`GridHandles::of`]; a drag previews the existing Repeat Options command and commits as one undo
//! step.
use serde_json::{Value, json};
use vectorcraft_doc::pattern::tile_offset;
use vectorcraft_doc::{Document, NodeId, NodeKind, OrientedBox, RepeatKind, Selection};
use vectorcraft_geom::{Point, Rect, Vec2};

use crate::bbox::Handle;
use crate::{Action, Cursor, Mods, ToolContext};

/// The radius of the row, column and corner handles (screen points).
pub const HANDLE_RADIUS: f32 = 5.0;

/// How far (screen points) the row, column and corner handles sit outside the bounding box, clear
/// of its own handles and rotate zones.
pub const HANDLE_GAP: f64 = 22.0;

/// Half the length of a spacing slider's bar (screen points).
pub const SLIDER_HALF: f32 = 7.0;

/// How near (screen points) a press must come to a handle to take it.
const REACH: f64 = 8.0;

/// The smallest tile (source size plus spacing) a spacing slider leaves, in points.
const MIN_TILE: f64 = 0.01;

/// The most rows or columns a grid repeat has (as `object.repeat.options` clamps them).
const MAX_COUNT: f64 = 500.0;

/// A control of [`GridHandles`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridPart {
    /// Below the grid: drag down for more rows, up for fewer.
    Rows,
    /// Right of the grid: drag right for more columns, left for fewer.
    Cols,
    /// Past the bottom-right corner: rows and columns together.
    Corner,
    /// Between the first two columns: the horizontal spacing.
    HSpacing,
    /// Between the first two rows: the vertical spacing.
    VSpacing,
}

impl GridPart {
    /// Whether the control draws as a slider bar rather than a round handle.
    pub fn is_slider(self) -> bool {
        matches!(self, Self::HSpacing | Self::VSpacing)
    }

    /// The bounding-box handle whose resize cursor the control shows.
    fn handle(self) -> Handle {
        match self {
            Self::Rows | Self::VSpacing => Handle::Bottom,
            Self::Cols | Self::HSpacing => Handle::Right,
            Self::Corner => Handle::BottomRight,
        }
    }
}

/// The controls of a selected grid repeat, in document space.
#[derive(Clone, Copy, Debug)]
pub struct GridHandles {
    pub id: NodeId,
    rows: u32,
    cols: u32,
    h_spacing: f64,
    v_spacing: f64,
    /// The source's bounds (the first tile).
    source: Rect,
    /// From one column to the next (x) and one row to the next (y).
    step: Vec2,
    /// How much `step` moves for each point of spacing (0.75 along a hex grid's staggered axis).
    gain: Vec2,
    pub rows_at: Point,
    pub cols_at: Point,
    pub corner_at: Point,
    /// With two columns or more.
    pub h_slider: Option<Point>,
    /// With two rows or more.
    pub v_slider: Option<Point>,
}

impl GridHandles {
    /// The controls of a single editable grid repeat selected whole, placed outside `frame` (the
    /// bounding box the Selection tool shows; its geometric bounds without one) at `zoom`.
    pub fn of(doc: &Document, selection: &Selection, frame: Option<OrientedBox>, zoom: f64) -> Option<Self> {
        let [id] = selection.objects.as_slice() else { return None };
        if !selection.anchors.is_empty() || !selection.segments.is_empty() || !doc.is_editable(*id) || !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        let n = doc.node(*id)?;
        let NodeKind::Repeat(spec) = &n.kind else { return None };
        let RepeatKind::Grid { h_spacing, v_spacing, rows, cols, grid_type, .. } = spec.kind else { return None };
        if !h_spacing.is_finite() || !v_spacing.is_finite() {
            return None;
        }
        let source = spec.source_bounds()?;
        // As `repeat_transforms` lays the tiles out.
        let (w, h) = ((source.width() + h_spacing).max(1e-3), (source.height() + v_spacing).max(1e-3));
        let step = Vec2::new(tile_offset(grid_type, w, h, 1, 0).x, tile_offset(grid_type, w, h, 0, 1).y);
        let gain = Vec2::new(tile_offset(grid_type, 1.0, 1.0, 1, 0).x, tile_offset(grid_type, 1.0, 1.0, 0, 1).y);
        if ![step.x, step.y, gain.x, gain.y].iter().all(|v| v.is_finite() && *v > 0.0) {
            return None;
        }
        let frame = match frame {
            Some(f) => aabb(f.corners())?,
            None => n.geometric_bounds_with(&doc.symbol_boxes())?,
        };
        let gap = HANDLE_GAP / zoom;
        let (rows, cols) = (rows.clamp(1, 500), cols.clamp(1, 500));
        let g = Self {
            id: *id,
            rows,
            cols,
            h_spacing,
            v_spacing,
            source,
            step,
            gain,
            rows_at: Point::new(frame.center().x, frame.y1 + gap),
            cols_at: Point::new(frame.x1 + gap, frame.center().y),
            corner_at: Point::new(frame.x1 + gap, frame.y1 + gap),
            // Halfway between the right edge of the first column and the left edge of the second,
            // level with the first tile; likewise between the first two rows.
            h_slider: (cols > 1).then(|| Point::new(source.x0 + (source.width() + step.x) / 2.0, source.center().y)),
            v_slider: (rows > 1).then(|| Point::new(source.center().x, source.y0 + (source.height() + step.y) / 2.0)),
        };
        g.parts().iter().all(|(_, p)| finite(*p)).then_some(g)
    }

    /// Every control with where it is: the round handles, then the sliders there are.
    pub fn parts(&self) -> Vec<(GridPart, Point)> {
        let mut v = vec![(GridPart::Rows, self.rows_at), (GridPart::Cols, self.cols_at), (GridPart::Corner, self.corner_at)];
        v.extend(self.h_slider.map(|p| (GridPart::HSpacing, p)));
        v.extend(self.v_slider.map(|p| (GridPart::VSpacing, p)));
        v
    }

    /// The control under `p` (the nearest within reach) and the grid's controls, unless a
    /// bounding-box handle is there, which keeps its scaling.
    fn hit(cx: &ToolContext, p: Point) -> Option<(Self, GridPart)> {
        if !cx.show_bbox || !finite(p) {
            return None;
        }
        let g = Self::of(cx.doc, cx.selection, crate::select::selection_box(cx), cx.zoom)?;
        let reach = cx.tol(REACH);
        let (part, _) =
            g.parts().into_iter().map(|(part, q)| (part, q.distance(p))).filter(|(_, d)| *d <= reach).min_by(|a, b| a.1.total_cmp(&b.1))?;
        (!crate::select::bbox_handle_at(cx, p)).then_some((g, part))
    }

    /// The Repeat Options that dragging `part` by `d` (document points) from the press gives.
    fn options(&self, part: GridPart, d: Vec2) -> Value {
        let count = |n: u32, by: f64, step: f64| (f64::from(n) + (by / step).round()).clamp(1.0, MAX_COUNT) as u32;
        // The slider sits halfway across the gap, which widens by `gain` for each point of spacing:
        // the spacing that keeps the slider under the pointer.
        let spacing = |s: f64, by: f64, gain: f64, size: f64| ((s + 2.0 * by / gain).max(MIN_TILE - size) * 100.0).round() / 100.0;
        match part {
            GridPart::Rows => json!({ "rows": count(self.rows, d.y, self.step.y) }),
            GridPart::Cols => json!({ "cols": count(self.cols, d.x, self.step.x) }),
            GridPart::Corner => json!({ "rows": count(self.rows, d.y, self.step.y), "cols": count(self.cols, d.x, self.step.x) }),
            GridPart::HSpacing => json!({ "hSpacing": spacing(self.h_spacing, d.x, self.gain.x, self.source.width()) }),
            GridPart::VSpacing => json!({ "vSpacing": spacing(self.v_spacing, d.y, self.gain.y, self.source.height()) }),
        }
    }
}

/// The page-square bounds of `corners`.
fn aabb(corners: [Point; 4]) -> Option<Rect> {
    let r = corners.iter().skip(1).fold(Rect::from_points(corners[0], corners[0]), |r, p| r.union_pt(*p));
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()).then_some(r)
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The resize cursor of `part`, turned with the view.
fn part_cursor(cx: &ToolContext, part: GridPart) -> Cursor {
    crate::select::handle_cursor(part.handle(), -cx.view_angle())
}

/// The cursor over a grid repeat's control at `p`, if one is there.
pub fn cursor(cx: &ToolContext, p: Point) -> Option<Cursor> {
    let (_, part) = GridHandles::hit(cx, p)?;
    Some(part_cursor(cx, part))
}

/// A drag of one of a grid repeat's controls.
#[derive(Clone, Debug)]
pub struct GridDrag {
    grid: GridHandles,
    part: GridPart,
    start: Point,
    /// The options the press found, and the last ones previewed.
    from: Value,
    last: Option<Value>,
}

impl GridDrag {
    pub fn hit(cx: &ToolContext, p: Point, mods: Mods) -> Option<Self> {
        if mods.cmd || mods.ctrl || mods.alt {
            return None;
        }
        let (grid, part) = GridHandles::hit(cx, p)?;
        Some(Self { grid, part, start: p, from: grid.options(part, Vec2::ZERO), last: None })
    }

    /// Whether the drag has changed the grid (and so previews it).
    pub fn adjusting(&self) -> bool {
        self.last.is_some()
    }

    pub fn cursor(&self, cx: &ToolContext) -> Cursor {
        part_cursor(cx, self.part)
    }

    /// Preview the options the pointer at `p` gives, when they change; the first change opens
    /// the undo step.
    pub fn drag(&mut self, p: Point) -> Vec<Action> {
        if !finite(p) {
            return vec![];
        }
        let v = self.grid.options(self.part, p - self.start);
        if self.last.as_ref().unwrap_or(&self.from) == &v {
            return vec![];
        }
        let mut out = vec![];
        if self.last.is_none() {
            out.push(Action::Begin("Repeat Options".into()));
        }
        out.push(Action::Preview("object.repeat.options".into(), v.clone()));
        self.last = Some(v);
        out
    }

    /// Commit the drag; one that ends where it began leaves no undo step.
    pub fn finish(self) -> Vec<Action> {
        match self.last {
            Some(v) if v == self.from => vec![Action::Cancel],
            Some(_) => vec![Action::Commit],
            None => vec![],
        }
    }

    pub fn cancel(self) -> Vec<Action> {
        if self.last.is_some() { vec![Action::Cancel] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{cx, doc_with_rect, paint};
    use std::sync::Arc;
    use vectorcraft_doc::pattern::RepeatSpec;

    /// [`doc_with_rect`]'s 100 × 100 square at (100, 100) as a 3 × 3 grid repeat with 20 pt of
    /// spacing, selected: tiles every 120 pt, the grid 340 pt square.
    fn grid() -> (Document, Selection) {
        let (mut doc, id) = doc_with_rect();
        let mut source = doc.node(id).unwrap().clone();
        source.id = doc.alloc_id();
        doc.node_mut(id).unwrap().kind = NodeKind::Repeat(RepeatSpec::grid(vec![Arc::new(source)], 20.0, 20.0));
        let mut selection = Selection::default();
        selection.set([id]);
        (doc, selection)
    }

    fn preview(out: &[Action]) -> Option<&Value> {
        out.iter().find_map(|a| match a {
            Action::Preview(cmd, v) if cmd == "object.repeat.options" => Some(v),
            _ => None,
        })
    }

    #[test]
    fn the_handles_sit_outside_the_grid_at_every_zoom() {
        let (doc, selection) = grid();
        for zoom in [0.5, 1.0, 4.0] {
            let g = GridHandles::of(&doc, &selection, None, zoom).unwrap();
            let gap = HANDLE_GAP / zoom;
            assert_eq!(g.rows_at, Point::new(270.0, 440.0 + gap), "{zoom}");
            assert_eq!(g.cols_at, Point::new(440.0 + gap, 270.0), "{zoom}");
            assert_eq!(g.corner_at, Point::new(440.0 + gap, 440.0 + gap), "{zoom}");
            // Halfway across the gaps between the first two columns and rows.
            assert_eq!(g.h_slider, Some(Point::new(210.0, 150.0)), "{zoom}");
            assert_eq!(g.v_slider, Some(Point::new(150.0, 210.0)), "{zoom}");
        }
        // Placed outside the bounding box the Selection tool shows.
        let frame = OrientedBox::aligned(Rect::new(90.0, 90.0, 450.0, 450.0));
        let g = GridHandles::of(&doc, &selection, Some(frame), 1.0).unwrap();
        assert_eq!(g.rows_at, Point::new(270.0, 450.0 + HANDLE_GAP));
    }

    #[test]
    fn no_handles_without_a_single_valid_grid_repeat() {
        let (mut doc, mut selection) = grid();
        for zoom in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(GridHandles::of(&doc, &selection, None, zoom).is_none(), "{zoom}");
        }
        let paint = paint();
        let mut c = cx(&doc, &selection, &paint);
        let g = GridHandles::of(&doc, &selection, None, 1.0).unwrap();
        assert!(cursor(&c, g.rows_at).is_some());
        c.show_bbox = false;
        assert!(cursor(&c, g.rows_at).is_none(), "the bounding box is hidden");
        let id = selection.objects[0];
        let other = doc.alloc_id();
        let mut second = doc.node(id).unwrap().clone();
        second.id = other;
        let layer = doc.layers[0].id;
        doc.insert(Some(layer), 1, second).unwrap();
        selection.set([id, other]);
        assert!(GridHandles::of(&doc, &selection, None, 1.0).is_none(), "two objects selected");
        selection.set([id]);
        let NodeKind::Repeat(spec) = &mut doc.node_mut(id).unwrap().kind else { panic!("a repeat") };
        spec.kind = RepeatKind::Mirror { angle: 90.0, center: Point::ZERO };
        assert!(GridHandles::of(&doc, &selection, None, 1.0).is_none(), "a mirror repeat");
    }

    #[test]
    fn a_one_by_one_grid_has_no_spacing_sliders() {
        let (mut doc, selection) = grid();
        let NodeKind::Repeat(spec) = &mut doc.node_mut(selection.objects[0]).unwrap().kind else { panic!("a repeat") };
        let RepeatKind::Grid { rows, cols, .. } = &mut spec.kind else { panic!("a grid") };
        (*rows, *cols) = (1, 1);
        let g = GridHandles::of(&doc, &selection, None, 1.0).unwrap();
        assert_eq!((g.h_slider, g.v_slider), (None, None));
        assert_eq!(g.parts().len(), 3);
    }

    #[test]
    fn dragging_the_handles_previews_rows_columns_and_spacing() {
        let (doc, selection) = grid();
        let paint = paint();
        let c = cx(&doc, &selection, &paint);
        let g = GridHandles::of(&doc, &selection, crate::select::selection_box(&c), 1.0).unwrap();
        // Two tile heights down: five rows, one undo step.
        let mut d = GridDrag::hit(&c, g.rows_at, Mods::default()).unwrap();
        assert!(d.drag(g.rows_at + Vec2::new(0.0, 30.0)).is_empty(), "less than half a tile changes nothing");
        assert!(!d.adjusting());
        let out = d.drag(g.rows_at + Vec2::new(3.0, 240.0));
        assert!(matches!(out.first(), Some(Action::Begin(l)) if l == "Repeat Options"), "{out:?}");
        assert_eq!(preview(&out), Some(&json!({"rows": 5})));
        assert!(d.drag(g.rows_at + Vec2::new(0.0, 250.0)).is_empty(), "the same count isn't previewed again");
        assert!(matches!(d.finish().as_slice(), [Action::Commit]));
        // Left past the first column: one column at least.
        let mut d = GridDrag::hit(&c, g.cols_at, Mods::default()).unwrap();
        assert_eq!(preview(&d.drag(g.cols_at - Vec2::new(900.0, 0.0))), Some(&json!({"cols": 1})));
        let mut d = GridDrag::hit(&c, g.corner_at, Mods::default()).unwrap();
        assert_eq!(preview(&d.drag(g.corner_at + Vec2::new(120.0, -120.0))), Some(&json!({"rows": 2, "cols": 4})));
        // The spacing slider stays under the pointer: 10 pt right is 20 pt more spacing.
        let mut d = GridDrag::hit(&c, g.h_slider.unwrap(), Mods::default()).unwrap();
        assert_eq!(preview(&d.drag(g.h_slider.unwrap() + Vec2::new(10.0, 4.0))), Some(&json!({"hSpacing": 40.0})));
        // Back where it began: no undo step.
        assert_eq!(preview(&d.drag(g.h_slider.unwrap())), Some(&json!({"hSpacing": 20.0})));
        assert!(matches!(d.finish().as_slice(), [Action::Cancel]));
        // Overlapping tiles keep a tile of at least 0.01 pt.
        let mut d = GridDrag::hit(&c, g.v_slider.unwrap(), Mods::default()).unwrap();
        assert_eq!(preview(&d.drag(g.v_slider.unwrap() - Vec2::new(0.0, 500.0))), Some(&json!({"vSpacing": -99.99})));
        assert!(matches!(d.cancel().as_slice(), [Action::Cancel]));
        // Alt, Cmd and Ctrl keep the Selection tool's own gestures.
        assert!(GridDrag::hit(&c, g.rows_at, Mods { alt: true, ..Mods::default() }).is_none());
    }

    #[test]
    fn a_bounding_box_handle_wins_where_they_overlap() {
        let (doc, selection) = grid();
        let paint = paint();
        let mut c = cx(&doc, &selection, &paint);
        // Zoomed far out (the 340 pt grid 6.8 px across), the spacing slider lies on the top-left
        // bounding-box handle, which keeps its scaling; the round handles stay clear of the box.
        c.zoom = 0.02;
        let g = GridHandles::of(&doc, &selection, crate::select::selection_box(&c), c.zoom).unwrap();
        let slider = g.h_slider.unwrap();
        assert!(crate::select::bbox_handle_at(&c, slider));
        assert!(GridDrag::hit(&c, slider, Mods::default()).is_none());
        assert!(cursor(&c, slider).is_none());
        assert!(!crate::select::bbox_handle_at(&c, g.rows_at));
        assert!(GridDrag::hit(&c, g.rows_at, Mods::default()).is_some());
        // The spacing slider, inside the box, is the grid's.
        c.zoom = 1.0;
        let g = GridHandles::of(&doc, &selection, crate::select::selection_box(&c), c.zoom).unwrap();
        assert!(GridDrag::hit(&c, g.h_slider.unwrap(), Mods::default()).is_some());
        assert_eq!(cursor(&c, g.h_slider.unwrap()), Some(Cursor::ResizeH));
        assert_eq!(cursor(&c, g.rows_at), Some(Cursor::ResizeV));
        assert_eq!(cursor(&c, g.corner_at), Some(Cursor::ResizeNwSe));
    }
}
