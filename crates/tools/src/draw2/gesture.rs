//! Freehand gesture tools: Pencil (N), Paintbrush (B), Blob Brush (Shift+B), Eraser (Shift+E),
//! Knife, Smooth, Path Eraser and Join.
//!
//! Pencil and Paintbrush preview the fitted path live (`path.freehand` re-applied on the snapshot)
//! and commit on release; with Live curve fitting off, the Pencil's preview joins the stroke's
//! points with straight segments and the release fits them; Alt on release (or ending near the start) closes the path, and starting
//! near an end of a selected open path continues it. Shift held at the press draws a straight
//! Pencil line at 45° steps from the Constrain Angle instead, and Alt held at the press (with Alt
//! key toggles to Smooth Tool off) draws a straight line at any angle. The Pencil's new path starts and ends on
//! Smart Guides ([`DrawSnap`]: an anchor, a path, in line with the art), hovering too; the points
//! between follow the hand. The Paintbrush records the pen pressure of each point (a stroke a pen
//! pressed less than fully on sends `[x, y, pressure]` points) for pressure-sensitive brushes, and
//! the pen's tilt and barrel rotation where the pen reads them (`[x, y, pressure, tiltX, tiltY,
//! rotation]` points, `null` for what a point lacks) for brushes that vary with them. The Blob
//! Brush and Eraser also record pressure when their Size varies with Pressure. The other tools
//! collect the drag polyline (shown as an overlay) and run one command on release. With Shift
//! held, the Blob Brush's points stay on the horizontal or
//! vertical line through the press point. Alt held at the press, or before the Blob Brush's drag
//! has moved, switches the Blob Brush to the Smooth tool: the drag smooths the selected paths.
//! Alt held at a Paintbrush press with a selected path also smooths; Alt pressed after its stroke
//! begins closes the path on release.
//! Alt held at the Knife's press makes it cut along the straight line from the press point to the
//! pointer, even if Alt is released during the drag. Shift on each drag or release constrains that
//! line to 45° steps from the canvas axes.

mod area;

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{Affine, BezPath, FillRule, ParamCurve, PathData, PathSeg, Point, Rect, Vec2, shapes};
use vectorcraft_pathops as po;

use super::{FEEDBACK, blob_area, blob_tip, eraser_tip, points_json, polyline};
use crate::guides::DrawSnap;
use crate::{Action, Cursor, Mods, Overlay, PenPose, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};
use area::{AreaKey, AreaPreview};

/// How far from its drag the Smooth tool smooths, in screen pixels. Alt smoothing with the
/// Pencil, Paintbrush or Blob Brush uses the same radius.
const SMOOTH_RADIUS: f64 = 12.0;

/// The most points of a drag that one step of its band adds, as when the band is outlined again
/// at a new zoom.
const BAND_STEP: usize = 32;
/// The most steps one call adds to a band: a band outlined again for a long drag shows in full
/// over several frames ([`Tool::overlays_pending`]).
const BAND_STEPS_PER_CALL: usize = 8;
/// The side of the squares a smoothing drag's band is kept in, in radii: a step of the drag adds
/// its area only to the squares it reaches, so the work of a step does not grow with the drag.
const BAND_TILE_RADII: f64 = 8.0;
/// Where the grid of those squares starts, in squares from the origin, so that the edge of the
/// band of a drag along whole coordinates does not run along a side of a square.
const BAND_TILE_OFFSET: f64 = 0.381_966;
/// The most squares one segment of a drag is checked against: a segment longer than 2,048 squares
/// is kept outside the squares, and shows as the two sides of its band.
const BAND_TILE_SAMPLES: f64 = 4096.0;
/// The side of the cells of a coarser grid, in squares: each cell lists the squares and the long
/// segments in it, so that a new square or long segment is checked only against those near it.
const BAND_CELL_TILES: i64 = 1024;
/// The most points, half a cell apart, along a long segment at which its cells are listed: a
/// longer segment is listed in no cell and checked against every square and long segment.
const BAND_CELL_SAMPLES: f64 = 4096.0;
/// The most work one call does on the band's long segments: comparisons of a long segment with a
/// square or another long segment, and pieces of an outline tested. The rest waits for the next
/// call ([`Tool::overlays_pending`]), and the band shown until then is the last one finished.
const BAND_WORK_PER_CALL: usize = 20_000;

/// The finest Fidelity (screen pixels) a Pencil or Paintbrush stroke is fitted at while it is drawn:
/// a fit's cost grows as its Fidelity shrinks (at 0.01 pt a long pen stroke lagged, #4565), and
/// a preview within half a pixel of the stroke looks the same. The release fits it at its Fidelity.
const PREVIEW_FIDELITY_PX: f64 = 0.5;
/// The Smooth tool's Fidelity (screen pixels) until Smooth Tool Options sets another.
const SMOOTH_FIDELITY: f64 = 2.5;

/// The `fidelity` (points) the Pencil and the Paintbrush send `path.freehand` for `px` screen pixels
/// at the current zoom, within the range the command takes
/// ([`super::MIN_FIT_TOLERANCE`]..[`super::MAX_FIT_TOLERANCE`]).
fn fit_tolerance(cx: &ToolContext, px: f64) -> f64 {
    cx.tol(px).clamp(super::MIN_FIT_TOLERANCE, super::MAX_FIT_TOLERANCE)
}

pub struct GestureTool {
    /// How many points the last preview of the stroke had ([`crate::previews_at`]); 0 when it was
    /// fitted coarser than the stroke kept ([`PREVIEW_FIDELITY_PX`]).
    previewed: usize,
    id: &'static str,
    points: Vec<Point>,
    /// The pen pressure of each of `points` (the Paintbrush, and the Blob Brush and the Eraser
    /// with `size_pressure`).
    pressures: Vec<f32>,
    /// The pen's tilt and rotation at each of `points` (the Paintbrush only).
    poses: Vec<Option<PenPose>>,
    active: bool,
    began: bool,
    /// A release where the pointer last dragged to would close the stroke because it ends near
    /// its start (Close Paths When Ends Are Within): the cursor shows the closing ring.
    near_start: bool,
    /// Pencil/Paintbrush continuing a selected open path: (path, continue from its start).
    extend: Option<(NodeId, bool)>,
    /// A Pencil stroke continuing a selected open path: the other paths selected at the press,
    /// whose open ends it connects to.
    connect_to: Vec<NodeId>,
    /// Pencil/Paintbrush starting on a selected path away from its ends: (path, Edit Selected
    /// Paths' distance in points); the stroke redraws the section it ends on (#2850).
    redraw: Option<(NodeId, f64)>,
    /// Fit tolerance (Pencil/Paintbrush/Smooth/Blob Brush "Fidelity"): in screen pixels for the
    /// Pencil and the Paintbrush, so a stroke keeps as many points at any zoom (#5729), and for the
    /// Smooth tool, so a drag smooths as much at any zoom; in points for the Blob Brush.
    pub fidelity: f64,
    /// Brush / eraser diameter in points (the Eraser's across its tip's long axis).
    pub size: f64,
    /// Blob Brush and Eraser Tool Options' Size set to Pressure (else Fixed): the tip is `size` −
    /// `size_variation` wide at the lightest pen pressure and `size` + `size_variation` at the
    /// heaviest.
    pub size_pressure: bool,
    /// Blob Brush and Eraser Tool Options' Size Variation, in points.
    pub size_variation: f64,
    /// The angle of the Blob Brush or Eraser tip's long axis, in degrees counter-clockwise from horizontal.
    pub angle: f64,
    /// The Blob Brush or Eraser tip's short axis in percent of its long one (100: round).
    pub roundness: f64,
    /// Fill new pencil/brush strokes.
    pub fill: bool,
    /// "Keep selected": select the path a Pencil, Paintbrush or Blob Brush stroke draws, continues
    /// or merges into (on for the Pencil and Paintbrush, off for the Blob Brush).
    pub keep_selected: bool,
    /// "Alt key toggles to Smooth Tool": holding Alt with the Pencil tool smooths selected paths
    /// instead of closing the path (Pencil only, off by default). Paintbrush Alt smoothing with a
    /// selected path and Blob Brush Alt smoothing do not use this toggle.
    pub alt_smooth: bool,
    /// "Live preview" (Pencil only, on by default): the stroke shows with its fill and stroke as it
    /// is drawn. Off, only a thin line follows the pen while drawing, and the path takes its fill
    /// and stroke on release.
    pub live_preview: bool,
    /// "Live curve fitting": the Pencil's preview fits the stroke while it is drawn; off, the
    /// preview shows the points joined by straight segments and the release fits them (Pencil
    /// only, on by default).
    pub live_fit: bool,
    /// The Smooth tool's Fidelity (screen pixels), used when Alt switches the Pencil, Paintbrush or
    /// Blob Brush to Smooth. The engine sets it from Smooth Tool Options (`smoothFidelity`).
    smooth_fidelity: f64,
    /// Active drag is smoothing with the Pencil, Paintbrush or Blob Brush (Alt held).
    smoothing: bool,
    /// The pointer has moved from the press point, by any distance (a move too short to add a
    /// point counts).
    moved: bool,
    /// Active Pencil drag draws a straight line (Shift or Alt held at the press), or active Knife
    /// drag cuts along one (Alt held at the press): the stroke is the press point and the pointer.
    straight: bool,
    /// The straight line is not constrained to 45° steps: the Pencil's Alt line, whose end snaps to
    /// Smart Guides as a stroke's end does, or the Knife's line while Shift is up.
    unconstrained: bool,
    /// Axis selected during the current uninterrupted Shift interval, in degrees.
    shift_line: Option<f64>,
    /// "Merge only with selection": a Blob Brush stroke merges only with selected paths.
    pub merge_only_selection: bool,
    /// "Edit selected paths within N pixels".
    pub edit_within: f64,
    /// "Close paths when ends are within N pixels".
    pub close_within: f64,
    /// Smart Guides for the Pencil's ends and the press of a straight Knife cut.
    snap: DrawSnap,
    /// Where the pointer went down, before any snap: a straight Knife cut released there cuts
    /// nothing, even when its press snapped elsewhere.
    pressed: Point,
    /// Where the Pencil's stroke began, before its start snapped, and how far snapping moved it:
    /// the samples near the start take part of that move ([`ease`]).
    start_shift: (Point, Vec2),
    /// Where the pointer was for the last sample taken, before easing moved it: the steps between
    /// samples, and whether the release adds one, are measured from there.
    last_raw: Point,
    /// Where the pointer was for each of `points` (the Pencil's ends bend the stroke from these).
    raw: Vec<Point>,
    /// Where the Blob Brush's or the Eraser's tip outline shows: the pointer over the canvas,
    /// unless Cmd/Ctrl lends a selection tool or Alt the Smooth tool.
    tip_at: Option<Point>,
    /// A pen pressed less than fully at a point of the stroke whose pressure is kept.
    light: bool,
    /// The length of the stroke, in points.
    length: f64,
    /// The outline of the area a Blob Brush or Eraser drag covers, as far as it is built
    /// ([`AreaPreview`]).
    area: Cell<Option<AreaPreview>>,
    /// The last [`Tool::overlays`] left part of that outline for later calls.
    area_pending: Cell<bool>,
    /// The band the smoothing drag has shown so far ([`GestureTool::smooth_band`]).
    band: Cell<Option<SmoothBand>>,
    /// The band last shown does not cover the whole drag yet; the next call adds to it.
    band_pending: Cell<bool>,
}

/// The band a smoothing drag smooths, for the drag's first `used` points at `radius`, kept as its
/// area in each square of a grid that it reaches.
struct SmoothBand {
    radius: f64,
    used: usize,
    first: Point,
    last: Point,
    tiles: BTreeMap<(i64, i64), BandTile>,
    /// The segments of the drag too long for the squares ([`BAND_TILE_SAMPLES`]).
    long: Vec<LongSegment>,
    /// The squares and long segments listed in each cell of a coarser grid ([`BAND_CELL_TILES`]).
    cells: BTreeMap<(i64, i64), BandCell>,
    /// The long segments listed in no cell ([`BAND_CELL_SAMPLES`]).
    wide: Vec<usize>,
    /// The squares near a long segment whose `cut` is to be made again.
    dirty_tiles: BTreeSet<(i64, i64)>,
    /// The long segments whose `drawn` is to be made again.
    dirty_long: BTreeSet<usize>,
    /// The band changed since `shown` was made.
    changed: bool,
    /// The outline last finished.
    shown: Option<BezPath>,
    /// The work done on long segments in the current call ([`BAND_WORK_PER_CALL`]).
    work: usize,
}

/// The squares and long segments (indexes into [`SmoothBand::long`]) in one cell of the coarser grid.
#[derive(Default)]
struct BandCell {
    tiles: Vec<(i64, i64)>,
    long: Vec<usize>,
}

/// The part of a band in one square of the grid, and its outline less the square's sides.
struct BandTile {
    area: PathData,
    outline: BezPath,
    /// The long segments whose band can reach the square (indexes into [`SmoothBand::long`]).
    near_long: Vec<usize>,
    /// `outline` less its parts inside the band of a long segment in `near_long`.
    cut: BezPath,
}

/// A segment of a drag too long for the squares, from `a` to `b`, with the part of it outside the
/// squares (`from` and `to`: 0 at `a`, 1 at `b`): the squares hold its band for a square's side
/// from each end ([`SmoothBand::add_long`]).
struct LongSegment {
    a: Point,
    b: Point,
    from: f64,
    to: f64,
    /// The squares its band can reach.
    near: BTreeSet<(i64, i64)>,
    /// The other long segments whose band can meet its band.
    meets: Vec<usize>,
    /// The parts of the two sides of its band that show.
    drawn: BezPath,
}

impl LongSegment {
    /// The two sides of its band at `radius`, over its part outside the squares.
    fn sides(&self, radius: f64) -> [vectorcraft_geom::Line; 2] {
        let d = self.b - self.a;
        let s = Vec2::new(-d.y, d.x) * (radius / d.hypot());
        let (p, q) = (self.a.lerp(self.b, self.from), self.a.lerp(self.b, self.to));
        [vectorcraft_geom::Line::new(p + s, q + s), vectorcraft_geom::Line::new(p - s, q - s)]
    }
}

impl SmoothBand {
    /// The band of a drag that starts at `first`, before it has moved.
    fn new(first: Point, radius: f64) -> Self {
        Self {
            radius,
            used: 1,
            first,
            last: first,
            tiles: BTreeMap::new(),
            long: vec![],
            cells: BTreeMap::new(),
            wide: vec![],
            dirty_tiles: BTreeSet::new(),
            dirty_long: BTreeSet::new(),
            changed: true,
            shown: None,
            work: 0,
        }
    }

    /// The cell of the coarser grid that holds square `key`.
    fn cell_of((ix, iy): (i64, i64)) -> (i64, i64) {
        (ix.div_euclid(BAND_CELL_TILES), iy.div_euclid(BAND_CELL_TILES))
    }

    /// The cells around points half a cell apart along the segment from `a` to `b`: every cell
    /// holding a square its band can reach, or a point within the radius of its band. `None` for
    /// a segment longer than [`BAND_CELL_SAMPLES`] half cells or reaching far outside any document.
    fn cells_along(&self, a: Point, b: Point) -> Option<BTreeSet<(i64, i64)>> {
        let n = (a.distance(b) / (0.5 * BAND_CELL_TILES as f64 * self.side())).ceil().max(1.0);
        if !n.is_finite() || n > BAND_CELL_SAMPLES {
            return None;
        }
        let mut cells = BTreeSet::new();
        for k in 0..=(n as u32) {
            let (cx, cy) = Self::cell_of(self.tile_of(a.lerp(b, f64::from(k) / n))?);
            for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                cells.insert((cx + dx, cy + dy));
            }
        }
        Some(cells)
    }

    /// Whether the band of the segment from `a` to `b` can reach square `key`.
    fn reaches(&self, key: (i64, i64), a: Point, b: Point) -> bool {
        distance_to_segment(self.tile_rect(key).center(), a, b) <= self.radius + 0.5 * std::f64::consts::SQRT_2 * self.side()
    }

    /// Whether the segment from `a` to `b` is too long for the squares: longer than
    /// [`BAND_TILE_SAMPLES`] half squares.
    fn too_long(&self, a: Point, b: Point) -> bool {
        let n = a.distance(b) / (0.5 * self.side());
        n.is_finite() && n > BAND_TILE_SAMPLES
    }

    /// Add the band of the segment from `a` to `b`, too long for the squares: the squares take its
    /// band for a square's side from each end, where they can, and the rest is kept as the
    /// segment.
    fn add_long(&mut self, a: Point, b: Point) {
        let d = b - a;
        let stub = d * (self.side() / d.hypot());
        let from = if self.grow(&[a, a + stub]).is_some() { self.side() / d.hypot() } else { 0.0 };
        let to = if self.grow(&[b - stub, b]).is_some() { 1.0 - self.side() / d.hypot() } else { 1.0 };
        let i = self.long.len();
        // The squares and long segments listed in its cells, or all of them.
        let mut tiles = BTreeSet::new();
        let mut others: BTreeSet<usize> = self.wide.iter().copied().collect();
        match self.cells_along(a, b) {
            Some(cells) => {
                self.work += cells.len();
                for c in cells {
                    let cell = self.cells.entry(c).or_default();
                    tiles.extend(cell.tiles.iter().copied());
                    others.extend(cell.long.iter().copied());
                    cell.long.push(i);
                }
            }
            None => {
                tiles.extend(self.tiles.keys().copied());
                others.extend(0..i);
                self.wide.push(i);
            }
        }
        self.work += tiles.len() + others.len();
        let near: BTreeSet<_> = tiles.into_iter().filter(|&k| self.reaches(k, a, b)).collect();
        for k in &near {
            if let Some(t) = self.tiles.get_mut(k) {
                t.near_long.push(i);
                self.dirty_tiles.insert(*k);
            }
        }
        let reach = 2.0 * self.radius + 2e-3 * self.radius;
        let meets: Vec<usize> = others.into_iter().filter(|&j| self.long.get(j).is_some_and(|o| segment_distance(a, b, o.a, o.b) <= reach)).collect();
        for &j in &meets {
            if let Some(o) = self.long.get_mut(j) {
                o.meets.push(i);
                self.dirty_long.insert(j);
            }
        }
        self.long.push(LongSegment { a, b, from, to, near, meets, drawn: BezPath::new() });
        self.dirty_long.insert(i);
        self.last = b;
        self.changed = true;
    }

    /// The side of the grid's squares.
    fn side(&self) -> f64 {
        BAND_TILE_RADII * self.radius
    }

    /// The square of the grid `key` names.
    fn tile_rect(&self, (ix, iy): (i64, i64)) -> Rect {
        let s = self.side();
        let (x0, y0) = ((ix as f64 + BAND_TILE_OFFSET) * s, (iy as f64 + BAND_TILE_OFFSET) * s);
        Rect::new(x0, y0, x0 + s, y0 + s)
    }

    /// The square of the grid that holds `p`; `None` far outside any document.
    fn tile_of(&self, p: Point) -> Option<(i64, i64)> {
        let s = self.side();
        let (x, y) = ((p.x / s - BAND_TILE_OFFSET).floor(), (p.y / s - BAND_TILE_OFFSET).floor());
        (x.abs() < 1e15 && y.abs() < 1e15).then_some((x as i64, y as i64))
    }

    /// The squares within the radius of `piece`: those around points along it half a square apart.
    fn tiles_near(&self, piece: &[Point]) -> Option<BTreeSet<(i64, i64)>> {
        let mut keys = BTreeSet::new();
        for w in piece.windows(2) {
            let n = (w[0].distance(w[1]) / (0.5 * self.side())).ceil().max(1.0);
            if !n.is_finite() || n > BAND_TILE_SAMPLES {
                return None;
            }
            for k in 0..=(n as u32) {
                let (ix, iy) = self.tile_of(w[0].lerp(w[1], f64::from(k) / n))?;
                for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    keys.insert((ix + dx, iy + dy));
                }
            }
        }
        Some(keys)
    }

    /// Add the area within the radius of `piece`, the drag from the last point covered so far, to
    /// the squares it reaches. `None`, with the band unchanged, when the outlines cannot be
    /// combined.
    fn grow(&mut self, piece: &[Point]) -> Option<()> {
        let (r, nz) = (self.radius, FillRule::NonZero);
        let end = *piece.last()?;
        // Within a hundredth of the radius at any zoom.
        let precision = (0.01 * r).min(po::DEFAULT_PRECISION);
        let keys = self.tiles_near(piece)?;
        let added = blob_area(piece, 2.0 * r, 0.0, 100.0);
        let mut changed = vec![];
        for key in keys {
            let rect = self.tile_rect(key);
            let clip = po::try_boolean(&added, nz, &shapes::rectangle(rect), nz, po::BoolOp::Intersect, precision).ok()?;
            if clip.is_empty() {
                continue;
            }
            let area = match self.tiles.get(&key) {
                Some(t) => po::try_boolean(&t.area, nz, &clip, nz, po::BoolOp::Union, precision).ok().filter(|a| !a.is_empty())?,
                None => clip,
            };
            changed.push((key, area));
        }
        // A square's outline depends on the squares across its sides, so those are outlined
        // again too.
        let mut redo = BTreeSet::new();
        for (key @ (ix, iy), area) in changed {
            redo.extend([(ix, iy), (ix - 1, iy), (ix + 1, iy), (ix, iy - 1), (ix, iy + 1)]);
            let near_long = match self.tiles.get(&key) {
                Some(t) => t.near_long.clone(),
                None => {
                    // The long segments listed in the square's cell, and those in no cell.
                    let cell = self.cells.entry(Self::cell_of(key)).or_default();
                    cell.tiles.push(key);
                    let mut listed: BTreeSet<usize> = cell.long.iter().copied().collect();
                    listed.extend(self.wide.iter().copied());
                    let near: Vec<usize> = listed.into_iter().filter(|&i| self.long.get(i).is_some_and(|l| self.reaches(key, l.a, l.b))).collect();
                    for &i in &near {
                        if let Some(l) = self.long.get_mut(i) {
                            l.near.insert(key);
                        }
                    }
                    near
                }
            };
            // The sides of those long segments are cut where the square's area holds them.
            self.dirty_long.extend(near_long.iter().copied());
            self.tiles.insert(key, BandTile { area, outline: BezPath::new(), near_long, cut: BezPath::new() });
        }
        self.changed = true;
        let outlines: Vec<_> = redo.into_iter().filter(|k| self.tiles.contains_key(k)).map(|k| (k, self.tile_outline(k))).collect();
        for (key, outline) in outlines {
            if let Some(t) = self.tiles.get_mut(&key) {
                t.outline = outline;
                if !t.near_long.is_empty() {
                    self.dirty_tiles.insert(key);
                }
            }
        }
        self.last = end;
        Some(())
    }

    /// The band's outline: the last one finished. While the band has changed since, the parts it
    /// changed are made again, until [`BAND_WORK_PER_CALL`] is spent in the call; when all are
    /// made, the outline is joined from them ([`SmoothBand::make_outline`]).
    fn outline(&mut self) -> Option<BezPath> {
        while self.changed {
            if !self.dirty_tiles.is_empty() || !self.dirty_long.is_empty() {
                if self.work >= BAND_WORK_PER_CALL {
                    break;
                }
                if let Some(k) = self.dirty_tiles.pop_first() {
                    let (cut, work) = self.tile_cut(k);
                    self.work += work;
                    if let Some(t) = self.tiles.get_mut(&k) {
                        t.cut = cut;
                    }
                } else if let Some(i) = self.dirty_long.pop_first() {
                    let (drawn, work) = self.long_drawn(i);
                    self.work += work;
                    if let Some(l) = self.long.get_mut(i) {
                        l.drawn = drawn;
                    }
                }
            } else {
                self.shown = Some(self.make_outline());
                self.changed = false;
            }
        }
        self.shown.clone()
    }

    /// The band's outline: every square's, joined, then the parts that show of the two sides of
    /// the band of each segment too long for the squares. A square near such a segment shows its
    /// `cut` ([`SmoothBand::tile_cut`]), and such a segment its `drawn` ([`SmoothBand::long_drawn`]).
    fn make_outline(&self) -> BezPath {
        let tiles = self.tiles.values().flat_map(|t| (if t.near_long.is_empty() { &t.outline } else { &t.cut }).elements().iter().copied());
        BezPath::from_vec(tiles.chain(self.long.iter().flat_map(|l| l.drawn.elements().iter().copied())).collect())
    }

    /// The outline of square `key` less its parts inside the band of a long segment near it (more
    /// than a thousandth of the radius in), and the work that took.
    fn tile_cut(&self, key: (i64, i64)) -> (BezPath, usize) {
        let Some(t) = self.tiles.get(&key) else { return (BezPath::new(), 1) };
        let (r, eps) = (self.radius, 1e-3 * self.radius);
        let near: Vec<&LongSegment> = t.near_long.iter().filter_map(|&i| self.long.get(i)).collect();
        let mut out = BezPath::new();
        let mut pen = None;
        let mut work = 1;
        for seg in t.outline.segments() {
            // The pieces of the segment between the points where it crosses a side of the band
            // of a long segment, each left out when it lies inside that band.
            let mut ts: Vec<f64> = near
                .iter()
                .flat_map(|l| l.sides(r))
                .flat_map(|line| seg.intersect_line(line))
                .map(|x| x.segment_t)
                .filter(|t| *t > 0.0 && *t < 1.0)
                .collect();
            ts.extend([0.0, 1.0]);
            ts.sort_by(f64::total_cmp);
            ts.dedup();
            work += near.len() * ts.len();
            for w in ts.windows(2) {
                let piece = if ts.len() == 2 { seg } else { seg.subsegment(w[0]..w[1]) };
                let m = piece.eval(0.5);
                if near.iter().any(|l| distance_to_segment(m, l.a, l.b) < r - eps) {
                    pen = None;
                } else {
                    push_seg(&mut out, &mut pen, piece);
                }
            }
        }
        (out, work)
    }

    /// The parts that show of the two sides of the band of long segment `i`, outside the squares'
    /// part of it, and the work that took. Left out: the parts inside the band area of a square,
    /// inside the band of another long segment (more than a thousandth of the radius in), and on
    /// a side of an earlier long segment, which already shows. Only the squares and long segments
    /// near it are checked.
    fn long_drawn(&self, i: usize) -> (BezPath, usize) {
        let Some(l) = self.long.get(i) else { return (BezPath::new(), 1) };
        let (r, eps) = (self.radius, 1e-3 * self.radius);
        let others: Vec<(usize, &LongSegment)> = l.meets.iter().filter_map(|&j| self.long.get(j).map(|o| (j, o))).collect();
        // Whether a square's band area holds `q`.
        let in_tiles = |q: Point| {
            self.tile_of(q).and_then(|k| self.tiles.get(&k)).is_some_and(|t| vectorcraft_geom::Shape::winding(&t.area.to_bezpath(), q) != 0)
        };
        let mut out = BezPath::new();
        let mut work = 1;
        for line in l.sides(r) {
            // Where the side crosses the band area of a square it passes, a side of another long
            // segment's band, or the end of a side it runs along.
            let mut ts = vec![0.0, 1.0];
            for t in l.near.iter().filter_map(|k| self.tiles.get(k)) {
                ts.extend(t.area.to_bezpath().segments().flat_map(|seg| seg.intersect_line(line)).map(|x| x.line_t));
            }
            // The stretches of the side inside the band of another long segment, and those on a
            // side of an earlier one.
            let mut inside = vec![];
            let mut along = vec![];
            for &(j, other) in &others {
                inside.extend(capsule_span(line, other.a, other.b, r - eps));
                for s in other.sides(r) {
                    ts.extend(PathSeg::Line(s).intersect_line(line).into_iter().map(|x| x.line_t));
                    if j < i {
                        ts.extend(
                            [s.p0, s.p1]
                                .into_iter()
                                .filter(|&p| distance_to_segment(p, line.p0, line.p1) <= eps)
                                .map(|p| (p - line.p0).dot(line.p1 - line.p0) / (line.p1 - line.p0).hypot2()),
                        );
                        along.extend(capsule_span(line, s.p0, s.p1, eps));
                    }
                }
            }
            let (inside, along) = (merge_spans(inside), merge_spans(along));
            ts.retain(|t| (0.0..=1.0).contains(t));
            ts.sort_by(f64::total_cmp);
            ts.dedup();
            work += l.near.len() + others.len() + ts.len();
            let mut pen = None;
            for w in ts.windows(2) {
                let (p, q) = (line.p0.lerp(line.p1, w[0]), line.p0.lerp(line.p1, w[1]));
                let mid = 0.5 * (w[0] + w[1]);
                let hidden = in_tiles(p.midpoint(q)) || spans_cover(&inside, mid, mid) || spans_cover(&along, w[0], w[1]);
                if !hidden {
                    push_seg(&mut out, &mut pen, PathSeg::Line(vectorcraft_geom::Line::new(p, q)));
                }
            }
        }
        (out, work)
    }

    /// The outline of the band's part in square `key`, less the parts along a side of the square
    /// (within a thousandth of the radius) where the band goes on across that side: where the
    /// square across it holds band area a thousandth of the radius past it.
    fn tile_outline(&self, key: (i64, i64)) -> BezPath {
        let Some(tile) = self.tiles.get(&key) else { return BezPath::new() };
        let (rect, eps) = (self.tile_rect(key), 1e-3 * self.radius);
        let (ix, iy) = key;
        // A point's distance across a side (x for a side that runs up and down) and along it.
        let split = |upright: bool, p: Point| if upright { (p.x, p.y) } else { (p.y, p.x) };
        // Each side with band area across it: whether it runs up and down, where it runs, the
        // direction away from this square, the area across, and where along the side that area's
        // outline has a point on the side (where the area across can start or stop).
        let sides: Vec<_> = [
            (true, rect.x0, (ix - 1, iy), -1.0),
            (true, rect.x1, (ix + 1, iy), 1.0),
            (false, rect.y0, (ix, iy - 1), -1.0),
            (false, rect.y1, (ix, iy + 1), 1.0),
        ]
        .into_iter()
        .filter_map(|(upright, at, k, away)| {
            let area = self.tiles.get(&k)?.area.to_bezpath();
            let cuts: Vec<f64> = area
                .segments()
                .flat_map(|s| [s.start(), s.end()])
                .map(|p| split(upright, p))
                .filter(|&(off, _)| (off - at).abs() <= eps)
                .map(|(_, along)| along)
                .collect();
            Some((upright, at, away, area, cuts))
        })
        .collect();
        let mut out = BezPath::new();
        let mut pen = None;
        for seg in tile.area.to_bezpath().segments() {
            let (a, b, mid) = (seg.start(), seg.end(), seg.eval(0.5));
            let Some((upright, at, away, area, cuts)) = sides.iter().find(|s| [a, b, mid].iter().all(|&p| (split(s.0, p).0 - s.1).abs() <= eps))
            else {
                push_seg(&mut out, &mut pen, seg);
                continue;
            };
            // The parts of the segment between the points where the area across starts or stops
            // (more than `eps` from its ends), each kept unless the area across holds the point
            // `eps` past the side beside it: the square across holds band area there exactly when
            // its own edge of the band lies farther out than `eps`, and so shows.
            let (la, lb) = (split(*upright, a).1, split(*upright, b).1);
            let mut ts: Vec<f64> = cuts
                .iter()
                .filter(|&&c| (c - la) * (c - lb) < 0.0 && (c - la).abs() > eps && (c - lb).abs() > eps)
                .map(|&c| (c - la) / (lb - la))
                .collect();
            ts.sort_by(f64::total_cmp);
            let whole = ts.is_empty();
            ts.insert(0, 0.0);
            ts.push(1.0);
            for w in ts.windows(2) {
                let (p, q) = (a.lerp(b, w[0]), a.lerp(b, w[1]));
                let m = if whole { mid } else { p.midpoint(q) };
                let past = at + away * eps;
                let probe = if *upright { Point::new(past, m.y) } else { Point::new(m.x, past) };
                if vectorcraft_geom::Shape::winding(area, probe) != 0 {
                    pen = None;
                } else if whole {
                    push_seg(&mut out, &mut pen, seg);
                } else {
                    push_seg(&mut out, &mut pen, PathSeg::Line(vectorcraft_geom::Line::new(p, q)));
                }
            }
        }
        out
    }
}

/// Add `seg` to `out`, starting a new subpath unless it starts at `pen`, where the last one ended.
fn push_seg(out: &mut BezPath, pen: &mut Option<Point>, seg: PathSeg) {
    let a = seg.start();
    if *pen != Some(a) {
        out.move_to(a);
    }
    match seg {
        PathSeg::Line(l) => out.line_to(l.p1),
        PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
        PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
    }
    *pen = Some(seg.end());
}

/// How far `q` is from the segment from `a` to `b`.
fn distance_to_segment(q: Point, a: Point, b: Point) -> f64 {
    let d = b - a;
    let t = if d.hypot2() > 0.0 { ((q - a).dot(d) / d.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
    q.distance(a + d * t)
}

/// How far the segment from `a` to `b` is from the segment from `c` to `d`.
fn segment_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    let side = |p: Point, q: Point, s: Point| (q - p).cross(s - p);
    if side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0 {
        return 0.0;
    }
    distance_to_segment(a, c, d).min(distance_to_segment(b, c, d)).min(distance_to_segment(c, a, b)).min(distance_to_segment(d, a, b))
}

/// The stretch of `line`, as `line_t` from and to (0 at its start, 1 at its end, and beyond),
/// within `rad` of the segment from `a` to `b`; `None` when no point of the line is.
fn capsule_span(line: vectorcraft_geom::Line, a: Point, b: Point, rad: f64) -> Option<(f64, f64)> {
    let (p, d) = (line.p0, line.p1 - line.p0);
    let dd = d.hypot2();
    if dd <= 0.0 || rad.is_nan() || rad < 0.0 {
        return None;
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    // The round ends.
    for c in [a, b] {
        let f = p - c;
        let (half_b, k) = (d.dot(f), f.hypot2() - rad * rad);
        let disc = half_b * half_b - dd * k;
        if disc >= 0.0 {
            let root = disc.sqrt();
            lo = lo.min((-half_b - root) / dd);
            hi = hi.max((-half_b + root) / dd);
        }
    }
    // The part between them: along the segment from 0 to its length, and across it within `rad`.
    let e = b - a;
    let len = e.hypot();
    if len > 0.0 {
        let u = e / len;
        let n = Vec2::new(-u.y, u.x);
        let (mut s0, mut s1) = (f64::NEG_INFINITY, f64::INFINITY);
        for (at, rate, min, max) in [((p - a).dot(u), d.dot(u), 0.0, len), ((p - a).dot(n), d.dot(n), -rad, rad)] {
            if rate == 0.0 {
                if at < min || at > max {
                    s1 = f64::NEG_INFINITY;
                }
            } else {
                let (x, y) = ((min - at) / rate, (max - at) / rate);
                s0 = s0.max(x.min(y));
                s1 = s1.min(x.max(y));
            }
        }
        if s0 <= s1 {
            lo = lo.min(s0);
            hi = hi.max(s1);
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// `spans` sorted and joined where they overlap.
fn merge_spans(mut spans: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    spans.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Whether one of `spans` (from [`merge_spans`]) holds everything from `from` to `to`.
fn spans_cover(spans: &[(f64, f64)], from: f64, to: f64) -> bool {
    let i = spans.partition_point(|s| s.0 <= from);
    i > 0 && spans.get(i - 1).is_some_and(|s| s.1 >= to)
}

/// How far, in multiples of the move, the samples near a snapped end follow it (#2958).
const EASE_SPAN: f64 = 4.0;

/// `p` moved by part of `shift`: all of it at `from`, none from [`EASE_SPAN`] times the shift's
/// length away, easing between. A stroke end that snaps (to the grid, a point or a guide) then
/// bends the stroke smoothly to it instead of jumping there and leaving a hook (#2958).
fn ease(p: Point, from: Point, shift: Vec2) -> Point {
    p + shift * weight(p, from, shift)
}

/// How much of `shift` a sample at `p` takes when the end at `from` moved by it ([`ease`]): 1 at
/// `from`, 0 from [`EASE_SPAN`] times the move away.
fn weight(p: Point, from: Point, shift: Vec2) -> f64 {
    let len = shift.hypot();
    if !(len > 1e-9 && len.is_finite()) {
        return 0.0;
    }
    (1.0 - (p.distance(from) / (EASE_SPAN * len)).min(1.0)).powi(2)
}

/// The paths a stroke can redraw for selected object `id`: the path itself, or a compound path's
/// member paths.
fn redraw_targets(cx: &ToolContext, id: NodeId) -> Vec<NodeId> {
    let Some(n) = cx.doc.node(id) else { return vec![] };
    match &n.kind {
        NodeKind::Path { .. } => vec![id],
        NodeKind::Compound { children, .. } => children.iter().filter(|c| c.path_data().is_some()).map(|c| c.id).collect(),
        _ => vec![],
    }
}

/// Whether the selection contains a path the Smooth command can act on: one with a subpath of three
/// or more anchors, or of two anchors joined by a curve.
fn has_selected_smooth_path(cx: &ToolContext) -> bool {
    let editable = cx.doc.editable();
    let smoothable = |sp: &vectorcraft_geom::SubPath| sp.anchors.len() >= 3 || (0..sp.segment_count()).any(|i| !super::segment_is_straight(sp, i));
    cx.selection.objects.iter().filter_map(|id| cx.doc.node(*id)).any(|n| {
        let mut found = false;
        n.walk(&mut |child| {
            if matches!(&child.kind, NodeKind::Path { guide: false, path, .. } if path.control_bounds().is_some() && path.subpaths.iter().any(smoothable))
                && editable(child.id)
            {
                found = true;
            }
        });
        found
    })
}

impl GestureTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "paintbrush" => "paintbrush",
            "blobBrush" => "blobBrush",
            "eraser" => "eraser",
            "knife" => "knife",
            "smooth" => "smooth",
            "pathEraser" => "pathEraser",
            "join" => "join",
            _ => "pencil",
        };
        let fidelity = if id == "smooth" { SMOOTH_FIDELITY } else { 1.5 };
        Self {
            id,
            previewed: 0,
            points: vec![],
            pressures: vec![],
            poses: vec![],
            active: false,
            began: false,
            near_start: false,
            extend: None,
            connect_to: vec![],
            redraw: None,
            fidelity,
            size: 10.0,
            size_pressure: false,
            size_variation: 0.0,
            angle: 0.0,
            roundness: 100.0,
            fill: false,
            keep_selected: id != "blobBrush",
            // Off, as Pencil Tool Options ships it: Alt keeps drawing a closed path until it is on.
            alt_smooth: false,
            live_preview: true,
            live_fit: true,
            smooth_fidelity: SMOOTH_FIDELITY,
            smoothing: false,
            moved: false,
            straight: false,
            unconstrained: false,
            shift_line: None,
            merge_only_selection: false,
            edit_within: 12.0,
            close_within: 15.0,
            snap: DrawSnap::default(),
            pressed: Point::ZERO,
            start_shift: (Point::ZERO, Vec2::ZERO),
            last_raw: Point::ZERO,
            raw: vec![],
            tip_at: None,
            light: false,
            length: 0.0,
            area: Cell::new(None),
            area_pending: Cell::new(false),
            band: Cell::new(None),
            band_pending: Cell::new(false),
        }
    }

    /// Do the ends of its strokes snap (the Pencil)?
    fn snaps(&self) -> bool {
        self.id == "pencil"
    }

    /// Does a straight Knife cut snap to anchors and paths? Only with Smart Guides on, which Snap to
    /// Pixel (for drawing) and Snap to Grid turn off (`PointSnap::new`). Its ends snap as the
    /// Pencil's straight line's do: the press, and the release unless Shift keeps it on a 45° step.
    fn knife_snaps(&self, cx: &ToolContext) -> bool {
        self.id == "knife" && self.straight && cx.smart_guides && !cx.snaps_to_pixel(crate::PixelOp::Drawing) && !cx.snap_to_grid
    }

    /// Does the end of a straight line snap: the Pencil's Alt line's, or a straight Knife cut's
    /// without Shift?
    fn line_end_snaps(&self, cx: &ToolContext) -> bool {
        self.straight && self.unconstrained && (self.snaps() || self.knife_snaps(cx))
    }

    /// Where a stroke or straight line released at `p` ends: where it snaps when its end snaps.
    fn release_end(&mut self, cx: &ToolContext, p: Point) -> Point {
        if (self.snaps() && !self.smoothing && !self.straight) || self.line_end_snaps(cx) { self.snap.drag(cx, p, None) } else { p }
    }

    /// Is a straight line with its end at `end` back at its press point (within a pixel), so that
    /// there is no line? A Knife cut also is when the pointer, at `pointer`, is back where it went
    /// down, though its press snapped elsewhere.
    fn back_at_press(&self, cx: &ToolContext, pointer: Point, end: Point) -> bool {
        self.points.first().is_some_and(|s| s.distance(end) < cx.tol(1.0)) || (self.id == "knife" && self.pressed.distance(pointer) < cx.tol(1.0))
    }

    fn draws_path(&self) -> bool {
        matches!(self.id, "pencil" | "paintbrush")
    }

    /// Does a stroke show as the path it draws while it is drawn? Not with the Pencil's Live preview
    /// off: then the drag shows as a thin line (an overlay) until the release.
    fn previews(&self) -> bool {
        self.id != "pencil" || self.live_preview
    }

    /// Add a stroke sample, with the pen's pressure, tilt and rotation of the event `ev` there.
    fn push(&mut self, p: Point, ev: &PointerEvent) {
        if let Some(last) = self.points.last() {
            self.length += last.distance(p);
        }
        self.points.push(p);
        if self.id == "paintbrush" || (self.varies_size() && self.size_pressure) {
            self.pressures.push(ev.pressure);
            self.light |= ev.pressure < 1.0;
        }
        if self.id == "paintbrush" {
            self.poses.push(ev.pen.map(PenPose::sanitized));
        }
    }

    /// Would a press with `m` held draw a straight Pencil line? Shift or Alt held, unless Alt lends
    /// the Smooth tool: the rule `PointerKind::Down` sets `straight` by.
    fn straight_at_press(&self, m: Mods) -> bool {
        self.id == "pencil" && !(self.alt_smooth && m.alt) && (m.shift || m.alt)
    }

    /// Would Alt held now switch the Blob Brush to the Smooth tool? It does before a press, and
    /// during a drag that has not moved from the press point yet.
    fn blob_alt_smooths(&self, m: Mods) -> bool {
        self.id == "blobBrush" && m.alt && !self.moved
    }

    /// Shift keeps an active straight Knife cut at 45° steps; without Shift it follows the pointer.
    /// Releasing Alt does not end a straight cut.
    fn knife_shift(&mut self, m: Mods) {
        if self.id == "knife" && self.straight {
            self.unconstrained = !m.shift;
        }
    }

    /// Constrain a straight Pencil line from the Constrain Angle, a straight Knife cut from the
    /// canvas axes, or the current Blob Brush sample. Blob Brush chooses an
    /// axis once per uninterrupted Shift interval; samples already drawn never move.
    /// A smoothing drag follows the pointer without a Shift constraint.
    fn constrained(&mut self, cx: &ToolContext, p: Point, m: Mods) -> Point {
        if self.straight && self.unconstrained {
            return p;
        }
        if self.straight
            && let Some(&start) = self.points.first()
        {
            let origin = if self.id == "knife" { 0.0 } else { cx.constrain_angle };
            return start + vectorcraft_geom::constrain_angle_from(p - start, 45.0, origin);
        }
        let Some(&start) = self.points.first().filter(|_| self.id == "blobBrush" && m.shift && !self.smoothing) else {
            self.shift_line = None;
            return p;
        };
        if self.shift_line.is_none() {
            // Raw press-to-pointer displacement in logical screen pixels along the constraint
            // axes. Zoom changes the document distance, not the activation tolerance.
            let v = p - start;
            let v = vectorcraft_geom::constrain_axes(cx.constrain_angle).map_or(Point::new(v.x, v.y), |axes| axes.inverse() * Point::new(v.x, v.y));
            let x = v.x.abs() * cx.zoom;
            let y = v.y.abs() * cx.zoom;
            self.shift_line = if x >= 1.0 && y < 0.5 {
                Some(cx.constrain_angle)
            } else if y >= 1.0 && x < 0.5 {
                Some(cx.constrain_angle + 90.0)
            } else {
                // No unconstrained trail while pending, including an initially diagonal drag.
                return self.points.last().copied().unwrap_or(start);
            };
        }
        start + vectorcraft_geom::constrain_angle_from(p - start, 180.0, self.shift_line.unwrap_or(cx.constrain_angle))
    }

    /// The Blob Brush's or the Eraser's tip at the pointer, before and while painting: its outline
    /// `size` across at its angle and roundness, at the zoom, black on a white halo so it reads on
    /// any art. Precise cursors show only the crosshair, and smoothing shows no tip.
    fn tip_outline(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some(at) = self.tip_at.filter(|_| !self.smoothing && !cx.precise_cursors) else { return vec![] };
        let tip = if self.id == "blobBrush" { blob_tip(self.angle, self.roundness) } else { eraser_tip(self.angle, self.roundness) };
        let r = self.size / 2.0;
        // A tolerance relative to the radius: four arcs at any size or zoom.
        let circle = kurbo::Shape::to_path(&kurbo::Circle::new(Point::ORIGIN, r), 0.1 * r.max(1e-6));
        let path = Affine::translate(at.to_vec2()) * tip * circle;
        vec![
            Overlay::Path { path: path.clone(), color: [0xff, 0xff, 0xff], width: 3.0, dashed: false },
            Overlay::Path { path, color: [0, 0, 0], width: 1.0, dashed: false },
        ]
    }

    /// A freehand stroke released with the pointer at `p` and its end at `end`: its samples bend
    /// to snapped ends, and the end is added, or moves the last step when that was where the
    /// button went up.
    fn release_freehand(&mut self, cx: &ToolContext, p: Point, end: Point, ev: &PointerEvent) {
        // Was the last step away from where the button went up (before the samples bend)?
        let away = !self.points.is_empty() && self.last_raw.distance(p) >= cx.tol(1.0);
        self.bend_to_snapped_ends(p, end);
        if away {
            self.push(end, ev);
        } else if self.snaps()
            && !self.smoothing
            && self.points.len() > 1
            && let Some(last) = self.points.last_mut()
        {
            // The last step was where the button went up: it moves where that snaps.
            *last = end;
        }
    }

    /// On release, bend the samples before the Pencil's snapped ends to them (#2958), the pointer
    /// let go at `p` and the stroke ending at `end`. Near both ends (a short stroke, or one whose
    /// ends snap together) a sample takes a share of each move that adds up to at most the whole,
    /// so none overshoots past either end (#3013 review). A smoothing pass (Alt, which may come
    /// mid-drag) works on where the hand went. Only the Pencil's ends snap; the other tools' points
    /// stay as they were placed (the Blob Brush's Shift moves earlier ones onto its line).
    fn bend_to_snapped_ends(&mut self, p: Point, end: Point) {
        if !self.snaps() {
            return;
        }
        let (end_shift, (from, start_shift)) = if self.smoothing { (Vec2::ZERO, (p, Vec2::ZERO)) } else { (end - p, self.start_shift) };
        for (q, r) in self.points.iter_mut().zip(&self.raw).skip(1) {
            let (mut ws, mut we) = (weight(*r, from, start_shift), weight(*r, p, end_shift));
            if ws + we > 1.0 {
                (ws, we) = (ws / (ws + we), we / (ws + we));
            }
            *q = *r + start_shift * ws + end_shift * we;
        }
        // A smoothing pass that began after a snapped press starts where the press was.
        if self.smoothing
            && let (Some(q), Some(r)) = (self.points.first_mut(), self.raw.first())
        {
            *q = *r;
        }
    }

    /// Forget the stroke.
    fn clear(&mut self) {
        self.moved = false;
        self.points.clear();
        self.pressures.clear();
        self.poses.clear();
        self.raw.clear();
        self.previewed = 0;
        self.shift_line = None;
        self.light = false;
        self.length = 0.0;
        self.area.set(None);
        self.area_pending.set(false);
        self.band.set(None);
        self.band_pending.set(false);
    }

    /// The outline of the band a smoothing drag smooths: every point within `radius` of the drag,
    /// round at both ends. The band shown last is kept and grown by the steps dragged since, at
    /// most [`BAND_STEP`] points at a time and [`BAND_STEPS_PER_CALL`] steps a call; a new radius
    /// (the zoom changed) or another drag starts it again. While the band covers less of the drag
    /// than it can, [`Tool::overlays_pending`] holds, so the host draws again and the band reaches
    /// the whole drag with the pointer still. When a step cannot be added whole, its segments are
    /// added one at a time: a segment longer than [`BAND_TILE_SAMPLES`] half squares is kept
    /// outside the squares, and one whose outlines cannot be combined adds nothing. Either way the
    /// band goes on past it. The work on such long segments stops for the call once
    /// [`BAND_WORK_PER_CALL`] is spent, and the drag is added to the band only once the outline of
    /// what it holds is finished.
    fn smooth_band(&self, radius: f64) -> Option<BezPath> {
        let pts = &self.points;
        let &first = pts.first()?;
        let mut b = self
            .band
            .take()
            .filter(|b| b.radius == radius && b.first == first && b.used.checked_sub(1).and_then(|i| pts.get(i)) == Some(&b.last))
            .unwrap_or_else(|| SmoothBand::new(first, radius));
        b.work = 0;
        let mut outline = b.outline();
        if !b.changed {
            for _ in 0..BAND_STEPS_PER_CALL {
                let Some(from) = b.used.checked_sub(1).filter(|_| b.used < pts.len() && b.work < BAND_WORK_PER_CALL) else { break };
                let mut to = (from + BAND_STEP).min(pts.len() - 1);
                let Some(piece) = pts.get(from..=to) else { break };
                if b.grow(piece).is_none() {
                    for (k, w) in piece.windows(2).enumerate() {
                        if b.work >= BAND_WORK_PER_CALL {
                            to = from + k;
                            break;
                        }
                        if b.too_long(w[0], w[1]) {
                            b.add_long(w[0], w[1]);
                        } else if b.grow(w).is_none() {
                            b.last = w[1];
                        }
                    }
                }
                b.used = to + 1;
            }
            outline = b.outline();
        }
        self.band_pending.set(b.used < pts.len() || b.changed);
        self.band.set(Some(b));
        outline.filter(|o| !o.elements().is_empty())
    }

    /// Does the tool's Size take Fixed or Pressure (the Blob Brush and the Eraser)?
    fn varies_size(&self) -> bool {
        matches!(self.id, "blobBrush" | "eraser")
    }

    /// The pen pressure of each point of a Blob Brush or Eraser stroke whose Size varies with
    /// Pressure, when a pen pressed less than fully anywhere along it (a stroke without pressure
    /// keeps its Size).
    fn size_pressures(&self) -> Option<&[f32]> {
        let pressed = self.pressures.len() == self.points.len() && self.light;
        (self.varies_size() && self.size_pressure && !self.smoothing && pressed).then_some(self.pressures.as_slice())
    }

    /// The stroke's points: `[x, y, pressure, tiltX, tiltY, rotation]` when a pen read its tilt or
    /// rotation anywhere along it (`null` for what a point lacks, and for the pressure of a stroke
    /// pressed fully all along), else `[x, y, pressure]` when a pen pressed less than fully
    /// anywhere (a mouse always presses fully), else `[x, y]`.
    fn stroke_json(&self) -> Value {
        let n = self.points.len();
        let pressed = self.pressures.len() == n && self.pressures.iter().any(|p| *p < 1.0);
        let posed = self.poses.len() == n && self.poses.iter().flatten().any(|q| q.tilt.is_some() || q.rotation.is_some());
        let round = |v: f32, k: f64| (f64::from(v) * k).round() / k;
        if posed {
            let points = self.points.iter().zip(&self.pressures).zip(&self.poses).map(|((p, f), q)| {
                let q = q.unwrap_or_default();
                let (tilt, rotation) = (q.tilt.map(|t| t.map(|v| round(v, 100.0))), q.rotation.map(|r| round(r, 100.0)));
                json!([p.x, p.y, pressed.then(|| round(*f, 1000.0)), tilt.map(|t| t[0]), tilt.map(|t| t[1]), rotation])
            });
            Value::Array(points.collect())
        } else if pressed {
            Value::Array(self.points.iter().zip(&self.pressures).map(|(p, f)| json!([p.x, p.y, round(*f, 1000.0)])).collect())
        } else {
            points_json(&self.points)
        }
    }

    fn label(&self) -> &'static str {
        match self.id {
            "paintbrush" => "Paintbrush",
            "blobBrush" => "Blob Brush",
            "eraser" => "Eraser",
            "knife" => "Knife",
            "smooth" => "Smooth",
            "pathEraser" => "Path Eraser",
            "join" => "Join",
            _ => "Pencil",
        }
    }

    /// The selected open path whose end is near `p` (Edit Selected Paths, off at 0 pixels), and
    /// that end.
    fn find_extend(&self, cx: &ToolContext, p: Point) -> Option<(NodeId, bool, Point)> {
        self.find_end(cx, p, &cx.selection.objects)
    }

    /// [`Self::find_extend`] among `ids`.
    fn find_end(&self, cx: &ToolContext, p: Point, ids: &[NodeId]) -> Option<(NodeId, bool, Point)> {
        if self.edit_within <= 0.0 {
            return None;
        }
        let tol = cx.tol(self.edit_within);
        let editable = cx.doc.editable();
        for id in ids {
            let Some(pd) = cx.doc.node(*id).and_then(|n| n.path_data()) else { continue };
            if !editable(*id) {
                continue;
            }
            let Some(sp) = pd.subpaths.iter().rev().find(|s| !s.closed && !s.anchors.is_empty()) else { continue };
            let (first, last) = (sp.anchors[0].p, sp.anchors[sp.anchors.len() - 1].p);
            if p.distance(last) <= tol {
                return Some((*id, false, last));
            }
            if p.distance(first) <= tol {
                return Some((*id, true, first));
            }
        }
        None
    }

    /// The selected path whose outline is near `p` (Edit Selected Paths, off at 0 pixels).
    fn find_redraw(&self, cx: &ToolContext, p: Point) -> Option<(NodeId, f64)> {
        let tol = cx.tol(self.edit_within);
        if self.edit_within <= 0.0 || !tol.is_finite() {
            return None;
        }
        // A selected compound path (a shape with a hole) redraws the member path the stroke starts
        // on, the nearest of them: a compound path has no path of its own.
        cx.selection.objects.iter().find_map(|id| {
            redraw_targets(cx, *id)
                .into_iter()
                .filter(|m| cx.doc.is_editable(*m))
                .filter_map(|m| Some((m, cx.doc.node(m)?.path_data()?.nearest(p)?.4)))
                .filter(|(_, d)| *d <= tol)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(m, _)| (m, tol))
        })
    }

    /// Does a stroke that ends at `end` end near its start (Close Paths When Ends Are Within, off
    /// at 0 pixels)?
    fn ends_near_start(&self, cx: &ToolContext, end: Point) -> bool {
        self.close_within > 0.0 && self.points.len() > 2 && self.points[0].distance(end) <= cx.tol(self.close_within)
    }

    /// Does a release now, with the stroke ending at `end`, close a freehand stroke because it ends
    /// near its start (`finish`)? A stroke that redraws a selected path closes only when the engine
    /// finds it does not end on that path, so it counts as not closing.
    fn release_closes_near_start(&self, cx: &ToolContext, end: Point) -> bool {
        self.draws_path() && !self.smoothing && !self.straight && self.extend.is_none() && self.redraw.is_none() && self.ends_near_start(cx, end)
    }

    /// Parameters of `path.freehand` for the current stroke, fitted at Fidelity on screen. Only the
    /// Pencil and the Paintbrush (`draws_path`) send it; the Blob Brush's Fidelity stays in points.
    pub fn freehand_params(&self, cx: &ToolContext, closed: bool) -> Value {
        let mut v = json!({
            "points": self.stroke_json(),
            "fidelity": fit_tolerance(cx, self.fidelity),
            "closed": closed,
            "style": if self.id == "paintbrush" { "brush" } else { "pencil" },
            "fill": self.fill,
            "keepSelected": self.keep_selected,
        });
        if let Some((id, start)) = self.extend {
            v["extend"] = json!({"id": id.0, "end": if start { "start" } else { "end" }});
        } else if let Some((id, within)) = self.redraw {
            v["redraw"] = json!({"id": id.0, "within": within});
        }
        v
    }

    fn finish(&mut self, cx: &ToolContext, m: Mods) -> Vec<Action> {
        let pts = &self.points;
        let has_sel = !cx.selection.is_empty();
        let tol = cx.tol(self.edit_within);
        let radius = cx.tol(SMOOTH_RADIUS);
        if self.smoothing {
            if has_sel && pts.len() >= 2 {
                return vec![Action::Exec(
                    "path.smoothRegion".into(),
                    json!({"points": points_json(pts), "radius": radius, "fidelity": cx.tol(self.smooth_fidelity)}),
                )];
            }
            return vec![];
        }
        match self.id {
            "pencil" | "paintbrush" => {
                if !self.began {
                    return vec![];
                }
                // Close Paths When Ends Are Within is off at 0 pixels.
                let near = pts.last().is_some_and(|&end| self.ends_near_start(cx, end));
                // A straight line has two points: closing it would draw it back over itself.
                let closed = !self.straight && ((m.alt && (self.id == "paintbrush" || !self.alt_smooth)) || (self.extend.is_none() && near));
                let mut params = self.freehand_params(cx, closed);
                // A Pencil stroke that continues a selected path and ends on an open end of another
                // path selected at the press connects the two.
                let connect = pts.last().and_then(|&end| self.find_end(cx, end, &self.connect_to));
                if let Some((other, at_start, _)) = connect {
                    params["connect"] = json!({"id": other.0, "end": if at_start { "start" } else { "end" }});
                }
                vec![Action::Preview("path.freehand".into(), params), Action::Commit]
            }
            "blobBrush" => vec![Action::Exec(
                "path.blob".into(),
                json!({
                    "points": self.stroke_json(),
                    "size": self.size,
                    "sizeMode": if self.size_pressure { "pressure" } else { "fixed" },
                    "sizeVariation": self.size_variation,
                    "fidelity": self.fidelity,
                    "angle": self.angle,
                    "roundness": self.roundness,
                    "keepSelected": self.keep_selected,
                    "mergeOnlySelection": self.merge_only_selection
                }),
            )],
            "eraser" => vec![Action::Exec(
                "path.eraseRegion".into(),
                json!({
                    "points": self.stroke_json(),
                    "size": self.size,
                    "sizeMode": if self.size_pressure { "pressure" } else { "fixed" },
                    "sizeVariation": self.size_variation,
                    "angle": self.angle,
                    "roundness": self.roundness
                }),
            )],
            "knife" if pts.len() >= 2 => vec![Action::Exec("path.knife".into(), json!({"points": points_json(pts)}))],
            "smooth" if has_sel && pts.len() >= 2 => {
                vec![Action::Exec(
                    "path.smoothRegion".into(),
                    json!({"points": points_json(pts), "radius": radius, "fidelity": cx.tol(self.fidelity)}),
                )]
            }
            "pathEraser" if has_sel => vec![Action::Exec("path.eraseSegments".into(), json!({"points": points_json(pts), "width": cx.tol(8.0)}))],
            "join" if pts.len() >= 2 => vec![Action::Exec("path.joinScrub".into(), json!({"points": points_json(pts), "tolerance": tol}))],
            _ => vec![],
        }
    }
}

impl Tool for GestureTool {
    fn id(&self) -> &'static str {
        self.id
    }
    fn busy(&self) -> bool {
        self.active
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        if self.varies_size() {
            self.tip_at = (!ev.mods.cmd && !self.blob_alt_smooths(ev.mods)).then_some(p);
        }
        match ev.kind {
            PointerKind::Move if self.snaps() => {
                self.snap.hover(cx, p, &[], None);
                vec![]
            }
            PointerKind::Down => {
                self.active = true;
                self.began = false;
                self.near_start = false;
                // Alt switches the Pencil when its toggle is on, the Blob Brush always, and the
                // Paintbrush when the press has a selected path.
                self.smoothing = ev.mods.alt
                    && (self.id == "blobBrush"
                        || (self.id == "pencil" && self.alt_smooth)
                        || (self.id == "paintbrush" && has_selected_smooth_path(cx)));
                // Alt draws a straight line only while it doesn't lend the Smooth tool; with Shift
                // too, the line keeps Shift's 45° steps. Alt makes the Knife cut along a straight line.
                self.straight = (self.id == "pencil" && !self.smoothing && (ev.mods.shift || ev.mods.alt)) || (self.id == "knife" && ev.mods.alt);
                self.unconstrained = self.straight && !ev.mods.shift;
                let extend = if self.draws_path() && !self.smoothing { self.find_extend(cx, p) } else { None };
                self.extend = extend.map(|(id, at_start, _)| (id, at_start));
                // The stroke's previews select the path it continues, so the others are kept now.
                self.connect_to = match self.extend {
                    Some((id, _)) if self.id == "pencil" => cx.selection.objects.iter().copied().filter(|o| *o != id).collect(),
                    _ => vec![],
                };
                self.redraw = if self.draws_path() && !self.smoothing && self.extend.is_none() { self.find_redraw(cx, p) } else { None };
                // A path continued starts at its end, one redrawn on it; a new one where it snaps.
                // A straight line is constrained from the end it continues, so it starts there.
                let start = if let Some((_, _, end)) = extend.filter(|_| self.straight) {
                    end
                } else if self.snaps() && !self.smoothing && self.extend.is_none() && self.redraw.is_none() {
                    self.snap.press(cx, p, &[], None)
                } else if self.knife_snaps(cx) {
                    // A straight Knife cut, its overlay and Shift's 45° steps start from the snapped point.
                    self.snap.press_on_paths(cx, p)
                } else {
                    p
                };
                self.start_shift = (p, start - p);
                self.last_raw = p;
                self.clear();
                self.pressed = p;
                self.push(start, ev);
                self.raw.push(p);
                vec![]
            }
            PointerKind::Drag => {
                if !self.active {
                    return vec![];
                }
                if (!self.began && !self.straight && self.id == "pencil" && self.alt_smooth && ev.mods.alt) || self.blob_alt_smooths(ev.mods) {
                    self.smoothing = true;
                }
                self.knife_shift(ev.mods);
                self.moved |= self.points.first().is_some_and(|start| p.distance(*start) > vectorcraft_geom::EPS);
                let p = self.constrained(cx, p, ev.mods);
                let mut end = p;
                if self.snaps() && !self.smoothing && !self.straight {
                    // Where the stroke would end: shown, taken on release.
                    end = self.snap.drag(cx, p, None);
                }
                let p = if self.line_end_snaps(cx) {
                    // Alt's line ends where a stroke would, and a Knife cut's on the anchors and
                    // paths its press snaps to: shown, taken on release.
                    self.snap.drag(cx, p, None)
                } else {
                    p
                };
                if self.straight {
                    if !self.line_end_snaps(cx) {
                        // The press's Smart Guides would stay on screen: the line's end doesn't snap.
                        self.snap.clear();
                    }
                    // The line runs from the press point to the pointer, wherever the drag went.
                    if self.back_at_press(cx, ev.pos, p) {
                        // Back at the press point there is no line: the one shown goes.
                        self.points.truncate(1);
                        self.raw.truncate(1);
                        return if std::mem::take(&mut self.began) { vec![Action::Cancel] } else { vec![] };
                    }
                    self.points.truncate(1);
                    self.raw.truncate(1);
                } else if (!self.points.is_empty() && self.last_raw.distance(p) < cx.tol(1.0)) || self.points.len() >= crate::MAX_STROKE_SAMPLES {
                    // A release here keeps the last point unless it snaps (`Up`), so the ring
                    // follows the point `finish` will test.
                    let kept = self.points.last().copied().filter(|l| l.distance(p) < cx.tol(1.0) && !self.snaps());
                    let release_end = kept.unwrap_or(end);
                    self.near_start = self.release_closes_near_start(cx, release_end);
                    return vec![];
                }
                let (from, shift) = self.start_shift;
                self.last_raw = p;
                self.raw.push(p);
                // A straight line's end is the pointer: only a freehand stroke eases into its start.
                self.push(if self.straight { p } else { ease(p, from, shift) }, ev);
                self.near_start = self.release_closes_near_start(cx, end);
                if !self.draws_path() || self.smoothing {
                    return vec![];
                }
                let mut out = vec![];
                if !self.began {
                    self.began = true;
                    out.push(Action::Begin(self.label().into()));
                }
                // With the Pencil's Live preview off a thin line shows instead (`overlays`).
                if self.previews() && crate::previews_at(self.points.len()) {
                    let mut v = self.freehand_params(cx, false);
                    let fidelity = self.fidelity.max(PREVIEW_FIDELITY_PX);
                    v["fidelity"] = json!(fit_tolerance(cx, fidelity));
                    // A preview fitted coarser than the stroke kept is stale (`deactivate`).
                    self.previewed = if fidelity > self.fidelity { 0 } else { self.points.len() };
                    if !self.live_fit {
                        // Live curve fitting off: the release fits the stroke (`finish`).
                        v["fit"] = json!(false);
                    }
                    out.push(Action::Preview("path.freehand".into(), v));
                }
                out
            }
            PointerKind::Up => {
                if !self.active {
                    return vec![];
                }
                // A release away from the press point counts as movement.
                self.moved |= self.points.first().is_some_and(|start| p.distance(*start) > vectorcraft_geom::EPS);
                // Alt pressed while the pointer stood still at the press point switches the release too.
                if self.blob_alt_smooths(ev.mods) {
                    self.smoothing = true;
                }
                // Alt only on release cannot turn a freehand Knife drag into a straight cut.
                self.knife_shift(ev.mods);
                let p = self.constrained(cx, p, ev.mods);
                let end = self.release_end(cx, p);
                self.snap.clear();
                let mut cancel = false;
                if self.straight {
                    self.points.truncate(1);
                    if !self.back_at_press(cx, ev.pos, end) {
                        self.push(end, ev);
                    } else {
                        cancel = self.began;
                    }
                } else {
                    self.release_freehand(cx, p, end, ev);
                }
                let out = if cancel { vec![Action::Cancel] } else { self.finish(cx, ev.mods) };
                self.active = false;
                self.began = false;
                self.near_start = false;
                self.smoothing = false;
                self.straight = false;
                self.unconstrained = false;
                self.clear();
                self.extend = None;
                self.connect_to.clear();
                self.redraw = None;
                out
            }
            _ => vec![],
        }
    }
    /// The freehand tools build their stroke from every position the pointer passed; the Join tool
    /// only needs where its scribble crosses the ends.
    fn wants_samples(&self) -> bool {
        matches!(self.id, "pencil" | "paintbrush" | "blobBrush" | "eraser" | "knife" | "smooth" | "pathEraser")
    }
    /// The outline of a long Blob Brush or Eraser drag, and a smoothing drag's band outlined again
    /// after a zoom change, reach the whole drag over several frames.
    fn overlays_pending(&self) -> bool {
        self.area_pending.get() || self.band_pending.get()
    }
    /// `[` and `]` are the Blob Brush's and the Eraser's: their size (#1661).
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::BracketLeft | ToolKey::BracketRight) && matches!(self.id, "blobBrush" | "eraser")
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape if self.active => {
                let began = self.began;
                self.snap.clear();
                self.active = false;
                self.began = false;
                self.near_start = false;
                self.smoothing = false;
                self.straight = false;
                self.unconstrained = false;
                self.clear();
                if began { vec![Action::Cancel] } else { vec![] }
            }
            ToolKey::BracketLeft if matches!(self.id, "blobBrush" | "eraser") => {
                self.size = (self.size - 1.0).max(1.0);
                vec![]
            }
            ToolKey::BracketRight if matches!(self.id, "blobBrush" | "eraser") => {
                self.size = (self.size + 1.0).min(1000.0);
                vec![]
            }
            _ => vec![],
        }
    }
    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.tip_at = None;
    }
    fn deactivate(&mut self, cx: &ToolContext) -> Vec<Action> {
        let began = self.began;
        self.tip_at = None;
        // Drags past the last preview are in the stroke kept, too, and with Live curve fitting
        // off the stroke kept is fitted.
        let stale = (began && (self.previewed != self.points.len() || !self.live_fit) && self.draws_path() && !self.smoothing)
            .then(|| Action::Preview("path.freehand".into(), self.freehand_params(cx, false)));
        self.snap.clear();
        self.active = false;
        self.near_start = false;
        self.began = false;
        self.smoothing = false;
        self.straight = false;
        self.unconstrained = false;
        self.clear();
        if began { stale.into_iter().chain([Action::Commit]).collect() } else { vec![] }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        self.area_pending.set(false);
        self.band_pending.set(false);
        let mut o = self.snap.guides().to_vec();
        // A path being drawn shows itself, except with the Pencil's Live preview off.
        if !self.active || self.points.len() < 2 || (self.began && self.previews()) {
            o.extend(self.tip_outline(cx));
            return o;
        }
        // A smoothing drag with a selected path it can smooth also shows, dashed, the band its
        // release smooths.
        if (self.id == "smooth" || self.smoothing)
            && has_selected_smooth_path(cx)
            && let Some(path) = self.smooth_band(cx.tol(SMOOTH_RADIUS))
        {
            o.push(Overlay::Path { path, color: FEEDBACK, width: 1.0, dashed: true });
        }
        // A Blob Brush drag that smooths shows the Smooth tool's thin line.
        let width = match self.id {
            "blobBrush" | "eraser" if !self.smoothing => (self.size * cx.zoom) as f32,
            _ => 1.0,
        };
        let color = if self.id == "eraser" { [0x80, 0x80, 0x80] } else { FEEDBACK };
        // A tip that is not round shows the outline of the area it covers, each step of the drag
        // as wide as the pen pressed there with Size set to Pressure. So does a round Blob Brush
        // tip: a line as wide as the brush would end square, where the stroke it adds is round at
        // both ends (#4343). The outline is built a bounded amount per call ([`AreaPreview`]).
        if matches!(self.id, "eraser" | "blobBrush") && (self.roundness < 100.0 || self.id == "blobBrush") && !self.smoothing {
            let pressures = self.size_pressures();
            let key = AreaKey {
                blob: self.id == "blobBrush",
                size: self.size,
                variation: self.size_variation,
                angle: self.angle,
                roundness: self.roundness,
                pressure: pressures.is_some(),
            };
            let mut area = self.area.take().filter(|a| a.fits(key, &self.points)).unwrap_or_else(|| AreaPreview::new(key, self.points[0]));
            area.step(&self.points, pressures);
            for path in area.paths(&self.points) {
                o.push(Overlay::Path { path, color, width: 1.0, dashed: false });
            }
            self.area_pending.set(area.pending(&self.points));
            self.area.set(Some(area));
        } else if let Some(pressures) = self.size_pressures() {
            // With Size set to Pressure, each step of the drag shows as wide as the pen pressed there.
            for (seg, p) in self.points.windows(2).zip(pressures.windows(2)) {
                let pressure = f64::from(p[0] + p[1]) / 2.0;
                let width = super::pressure_size(self.size, self.size_variation, pressure) * cx.zoom;
                o.push(Overlay::Path { path: polyline(seg), color, width: width as f32, dashed: false });
            }
        } else {
            o.push(Overlay::Path { path: polyline(&self.points), color, width, dashed: self.id == "knife" });
        }
        if cx.measurement_labels && self.id == "blobBrush" && !self.smoothing {
            let last = self.points[self.points.len() - 1];
            let length = self.length;
            o.push(Overlay::Measure { p: last + vectorcraft_geom::Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: format!("L: {}", cx.len(length)) });
        }
        // Over the stroke's footprint, which reaches the tip's edge at the pointer.
        o.extend(self.tip_outline(cx));
        o
    }
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        // An active smoothing drag, or Alt before a press that would switch the Pencil, Paintbrush
        // or Blob Brush to Smooth. A Paintbrush stroke already drawing keeps drawing with Alt.
        if self.smoothing
            || (self.id == "pencil" && self.alt_smooth && m.alt && !self.began)
            || (!self.active && self.id == "paintbrush" && m.alt && has_selected_smooth_path(cx))
            || self.blob_alt_smooths(m)
        {
            return Cursor::Smooth;
        }
        // Before a press that would draw a straight line: over an end of a selected open path,
        // the line continues that path from the end (`PointerKind::Down`).
        if !self.active && self.straight_at_press(m) {
            return if self.find_extend(cx, p).is_some() { Cursor::PencilContinue } else { Cursor::PencilStraight };
        }
        // Alt held at the release closes the path (`finish`).
        // A straight line stays open, so its release has nothing to close, and before a press Alt
        // gives the Pencil a straight line.
        if self.draws_path() && (self.id == "paintbrush" || !self.alt_smooth) && m.alt && !self.straight && (self.active || self.id != "pencil") {
            return if self.id == "paintbrush" { Cursor::PaintbrushClose } else { Cursor::PencilClose };
        }
        // A release near a freehand stroke's start closes it too (`finish`): the ring shows while
        // the pointer is there.
        if self.draws_path() && self.active && self.near_start {
            return if self.id == "paintbrush" { Cursor::PaintbrushClose } else { Cursor::PencilClose };
        }
        match self.id {
            "paintbrush" => Cursor::Paintbrush,
            "blobBrush" => Cursor::BlobBrush,
            "eraser" => Cursor::Eraser,
            "knife" => Cursor::Knife,
            "smooth" => Cursor::Smooth,
            "pathEraser" => Cursor::PathEraser,
            "join" => Cursor::Join,
            // Before a press that draws a new path the Pencil shows its star; over a selected path
            // that the press continues from an end or redraws (`PointerKind::Down`), it does not.
            _ if !self.active && self.find_extend(cx, p).is_none() && self.find_redraw(cx, p).is_none() => Cursor::PencilNew,
            _ => Cursor::Pencil,
        }
    }
    fn options(&self) -> Value {
        match self.id {
            "pencil" => {
                json!({
                    "fidelity": self.fidelity,
                    "fill": self.fill,
                    "altSmooth": self.alt_smooth,
                    "livePreview": self.live_preview,
                    "liveFit": self.live_fit,
                    "keepSelected": self.keep_selected,
                    "editWithin": self.edit_within,
                    "closeWithin": self.close_within
                })
            }
            "paintbrush" => {
                json!({
                    "fidelity": self.fidelity,
                    "fill": self.fill,
                    "keepSelected": self.keep_selected,
                    "editWithin": self.edit_within,
                    "closeWithin": self.close_within
                })
            }
            "smooth" => json!({"fidelity": self.fidelity}),
            "blobBrush" => {
                json!({
                    "fidelity": self.fidelity,
                    "size": self.size,
                    "sizeMode": if self.size_pressure { "pressure" } else { "fixed" },
                    "sizeVariation": self.size_variation,
                    "angle": self.angle,
                    "roundness": self.roundness,
                    "keepSelected": self.keep_selected,
                    "mergeOnlySelection": self.merge_only_selection
                })
            }
            "eraser" => json!({
                "size": self.size,
                "sizeMode": if self.size_pressure { "pressure" } else { "fixed" },
                "sizeVariation": self.size_variation,
                "angle": self.angle,
                "roundness": self.roundness
            }),
            _ => Value::Null,
        }
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "fidelity" => self.fidelity = v.as_f64().unwrap_or(self.fidelity).clamp(super::min_fidelity(self.id), 100.0),
            "size" => self.size = v.as_f64().unwrap_or(self.size).clamp(0.1, 1000.0),
            "sizeMode" if self.varies_size() => match v.as_str() {
                Some("pressure") => self.size_pressure = true,
                Some("fixed") => self.size_pressure = false,
                _ => {}
            },
            "sizeVariation" if self.varies_size() => self.size_variation = v.as_f64().unwrap_or(self.size_variation).clamp(0.0, 1000.0),
            "angle" => self.angle = v.as_f64().unwrap_or(self.angle).clamp(-180.0, 180.0),
            "roundness" => self.roundness = v.as_f64().unwrap_or(self.roundness).clamp(if self.id == "blobBrush" { 0.0 } else { 1.0 }, 100.0),
            "fill" => self.fill = v.as_bool().unwrap_or(self.fill),
            "altSmooth" => self.alt_smooth = v.as_bool().unwrap_or(self.alt_smooth),
            "livePreview" if self.id == "pencil" => self.live_preview = v.as_bool().unwrap_or(self.live_preview),
            "liveFit" if self.id == "pencil" => self.live_fit = v.as_bool().unwrap_or(self.live_fit),
            "smoothFidelity" if matches!(self.id, "pencil" | "paintbrush" | "blobBrush") => {
                self.smooth_fidelity = v.as_f64().unwrap_or(self.smooth_fidelity).clamp(super::min_fidelity("smooth"), 100.0)
            }
            "keepSelected" => self.keep_selected = v.as_bool().unwrap_or(self.keep_selected),
            "mergeOnlySelection" => self.merge_only_selection = v.as_bool().unwrap_or(self.merge_only_selection),
            "editWithin" => self.edit_within = v.as_f64().unwrap_or(self.edit_within).clamp(0.0, 100.0),
            "closeWithin" => self.close_within = v.as_f64().unwrap_or(self.close_within).clamp(0.0, 100.0),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
