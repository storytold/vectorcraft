//! Shape Builder (Shift+M), Live Paint Bucket (K) and Live Paint Selection (Shift+L).
//!
//! The tools only emit commands (`shapeBuilder.merge`, `livePaint.fill`, `livePaint.strokeEdge`,
//! `select.*`); the planar arrangement comes from `vectorcraft_pathops::shape_builder` (Shape
//! Builder) and `vectorcraft_pathops::live_paint` (Live Paint).
//!
//! **Live Paint groups** are ordinary groups (no special node kind): the group's name starts with
//! [`LIVE_PAINT_PREFIX`]; its first child is a hidden group named [`SOURCES_NAME`] holding the
//! original paths (the planar map's inputs, kept for Release / Merge / re-computation), followed
//! by the face paths (named [`FACE_NAME`], fill only) and the edge paths (named [`EDGE_NAME`],
//! stroke only, open). Everything else in the document sees a plain group of paths, so rendering,
//! export, transforms and undo need no special cases.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use serde_json::{Value, json};
use vectorcraft_color::{Paint, Swatch};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect, Shape as _, SubPath};
use vectorcraft_pathops as po;

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey, json_ids};

// ---------- Shape Builder ----------

pub use po::SHAPE_BUILDER_MAX_GAP;

mod cache;
mod live_paint;
mod lookup;
mod map;

use cache::{BACKGROUND_SEGMENTS, CacheKey, RegionCache};
#[cfg(test)]
use live_paint::LpHover;
use live_paint::lp_color;
pub use live_paint::{LP_SELECT_WIDTH, LP_SELECT_WIDTH_RANGE, LivePaintBucketTool, LivePaintSelectionTool};
pub use lookup::{
    EDGE_NAME, FACE_NAME, Face, LIVE_PAINT_NAME, LIVE_PAINT_PREFIX, SOURCES_NAME, builder_members, drawn_outlines, edge_near, edges, face_at, faces,
    is_live_paint, leaf_shapes, live_paint_at, live_paint_takes, node_outline, outline_snap, path_contains, shape_builder_shapes, shapes_for,
    sources, visible_edge_near, visible_face_at,
};
pub use map::{
    BuilderMap, HATCH, HIGHLIGHT_RED, Hit, MergeGeometry, MergeInput, PreparedMerge, Rest, Touched, hatch, maps_built, merge_geometry, merge_inputs,
    merges_computed, sample_polyline, sorted_roots,
};
/// Small, Medium and Large gap lengths in points.
pub const GAP_LENGTHS: [f64; 3] = [3.0, 6.0, 12.0];

/// Shape Builder Tool Options (double-click the tool; `tool.setOption`), kept across tool switches
/// and saved with the preferences.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeBuilderOptions {
    /// Consider Open Filled Paths as Closed (`openFilledAsClosed`, on): a filled open path bounds a
    /// region as if an invisible edge closed it. When it is off, the path cuts the regions it crosses
    /// as a line does.
    pub open_filled_as_closed: bool,
    /// Pick Color From Color Swatches (`pickColorFrom: "swatches"`; off: `"artwork"`): a merge
    /// fills the new shape with the current fill, and the Left and Right arrow keys step the current
    /// fill through the document's color swatches. With Artwork, the shape takes the look of the
    /// object where the drag starts, else of the one where it ends, else of the front-most selected
    /// object.
    pub swatches: bool,
    /// Cursor Swatch Preview (`cursorSwatchPreview`, off): with Color Swatches, the current fill
    /// and the swatches before and after it show as chips above the pointer while no drag is on.
    pub cursor_swatch_preview: bool,
    /// Selection Straight Line (`selection: "straightLine"`; off: `"freeform"`): a drag touches
    /// what the straight line from where it started to the pointer crosses, not what the pointer
    /// passed over.
    pub straight_line: bool,
    /// Highlight Fill (`highlightFill`, on): in merge mode, the regions under the pointer or
    /// touched by the drag are shaded with a gray mesh. When it is off, they are not highlighted.
    pub highlight_fill: bool,
    /// Close gaps up to `gap_length` while finding regions.
    pub gap_detection: bool,
    /// Gap Length in points (at most [`SHAPE_BUILDER_MAX_GAP`]).
    pub gap_length: f64,
    /// In Merge Mode, Clicking Stroke Splits the Path (`clickStrokeSplitsPath`, off): in merge mode,
    /// a click on a stroke splits its path into the run under the pointer
    /// ([`BuilderMap::stroke_at`]) and the rest, and the pointer shows scissors over such a stroke.
    pub click_stroke_splits: bool,
    /// Highlight Stroke When Editable (`highlightStroke`, on): with `click_stroke_splits`, the run
    /// of stroke a click would split is drawn in `stroke_color` while the pointer is over it.
    pub highlight_stroke: bool,
    /// The color of that highlight (`highlightColor`: `#rrggbb` or a layer color's name; the layer
    /// color Red by default).
    pub stroke_color: [u8; 3],
}

/// The color Highlight Stroke When Editable draws in by default: the layer color Red.
pub const STROKE_HIGHLIGHT: [u8; 3] = [0xff, 0x4f, 0x4f];

impl Default for ShapeBuilderOptions {
    fn default() -> Self {
        Self {
            open_filled_as_closed: true,
            swatches: false,
            cursor_swatch_preview: false,
            straight_line: false,
            highlight_fill: true,
            gap_detection: false,
            gap_length: GAP_LENGTHS[0],
            click_stroke_splits: false,
            highlight_stroke: true,
            stroke_color: STROKE_HIGHLIGHT,
        }
    }
}

/// The document's color swatches in the Swatches panel's order (the swatches outside groups, then
/// each group's): the solid colors, without None.
pub fn color_swatches(doc: &Document) -> Vec<&Swatch> {
    let groups = doc.swatch_groups.iter().flat_map(|g| &g.swatches);
    doc.swatches.iter().chain(groups).filter(|s| matches!(s.paint, Paint::Solid { .. })).collect()
}

/// Is `fill` swatch `s`: linked to it by name, or the same paint?
fn is_swatch(s: &Swatch, fill: &Paint) -> bool {
    s.paint == *fill || matches!(fill, Paint::Solid { swatch: Some(name), tint, .. } if *tint >= 1.0 && *name == s.name)
}

/// The index in `list` of the swatch `fill` is: `last` (the swatch picked last) while `fill` is still
/// that swatch, so swatches of the same color are each reached in turn; otherwise the first swatch
/// it is.
fn swatch_index(list: &[&Swatch], fill: &Paint, last: Option<usize>) -> Option<usize> {
    last.filter(|&i| list.get(i).is_some_and(|s| is_swatch(s, fill))).or_else(|| list.iter().position(|s| is_swatch(s, fill)))
}

/// The index of the swatch of `list` after (`forward`) or before the one `fill` is (see
/// [`swatch_index`]), wrapping around; the first or the last one when `fill` is none of them.
fn step_swatch(list: &[&Swatch], fill: &Paint, last: Option<usize>, forward: bool) -> Option<usize> {
    let n = list.len();
    match (swatch_index(list, fill, last), forward) {
        (Some(i), true) => Some((i + 1) % n),
        (Some(i), false) => Some((i + n - 1) % n),
        (None, true) => (n > 0).then_some(0),
        (None, false) => n.checked_sub(1),
    }
}

/// Shape Builder tool: drag across regions to merge them. Holding Alt (Option) switches to erase
/// mode while it is held: the pointer shows a minus, what a click or drag would delete highlights in
/// red, and the gesture deletes the regions it touches, the pieces of open paths and the edges
/// (outline pieces between the points where paths meet) it passes over. With Shift held at the
/// press, the drag draws a rectangular marquee and touches the regions it reaches (see
/// [`BuilderMap::regions_in_marquee`]): they merge, or with Alt they are deleted. Escape during a
/// drag cancels it. Its options are [`ShapeBuilderOptions`].
#[derive(Default)]
pub struct ShapeBuilderTool {
    cache: RegionCache,
    hover: Option<Hit>,
    points: Vec<Point>,
    touched: Touched,
    dragging: bool,
    /// A Shift-drag's marquee: where the drag started and where the pointer is.
    marquee: Option<(Point, Point)>,
    /// Erase mode: Alt held while hovering, or when the drag started.
    alt: bool,
    /// Where the pointer last moved (the cursor swatch preview sits above it).
    at: Option<Point>,
    /// The index in [`color_swatches`] of the swatch the arrow keys picked last.
    picked: Option<usize>,
    opts: ShapeBuilderOptions,
    /// With In Merge Mode, Clicking Stroke Splits the Path: the run of stroke under the pointer,
    /// which a click splits off.
    stroke: Option<SubPath>,
    /// The pointer went more than [`CLICK_TOL_PX`] from the press during the drag, so the gesture
    /// is not a click.
    far: bool,
    /// The merge of the last gesture over a large selection, being computed on another thread.
    merging: Option<PendingMerge>,
    /// That merge once computed, for `shapeBuilder.merge` ([`Tool::shape_builder_merge`]).
    prepared: Option<Arc<PreparedMerge>>,
    /// A gesture made while the map of a large selection is being built on another thread.
    deferred: Option<Deferred>,
}

/// A Shape Builder gesture made while the map of the selection was being built on another
/// thread: the map key and the inputs it was made over, its pointer events, which [`Tool::poll`]
/// replays once the map is there, and whether the release has come.
struct Deferred {
    key: CacheKey,
    shapes: Vec<po::Shape>,
    events: Vec<PointerEvent>,
    released: bool,
}

/// A Shape Builder merge being computed on another thread: the map key it is for (the document,
/// its revision and the selection), the `shapeBuilder.merge` params that apply it, what it was
/// computed from, and the channel the result arrives on.
struct PendingMerge {
    key: CacheKey,
    params: Value,
    map: Arc<BuilderMap>,
    touched: Touched,
    gap: f64,
    inputs: Vec<MergeInput>,
    done: Receiver<MergeGeometry>,
}

/// Computes [`merge_geometry`] on a new thread; None when no thread starts.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_merge(map: Arc<BuilderMap>, touched: Touched, gap: f64, inputs: Vec<MergeInput>) -> Option<Receiver<MergeGeometry>> {
    let (tx, done) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("vectorcraft-shape-builder-merge".into()).spawn(move || {
        // The tool may have dropped the receiver meanwhile (another tool, another document).
        let _ = tx.send(merge_geometry(&map, &touched, gap, &inputs));
    });
    spawned.is_ok().then_some(done)
}

/// The web build has no threads: the merge runs in the command.
#[cfg(target_arch = "wasm32")]
fn spawn_merge(_map: Arc<BuilderMap>, _touched: Touched, _gap: f64, _inputs: Vec<MergeInput>) -> Option<Receiver<MergeGeometry>> {
    None
}

/// How close (screen px) the pointer must come to a line piece or an edge to delete it.
const LINE_TOL_PX: f64 = 4.0;

/// How far (screen px) the pointer may move between press and release for the gesture to be a
/// click, which with In Merge Mode, Clicking Stroke Splits the Path splits the stroke at the press.
const CLICK_TOL_PX: f64 = 3.0;

impl ShapeBuilderTool {
    fn gap(&self) -> f64 {
        if self.opts.gap_detection { self.opts.gap_length } else { 0.0 }
    }

    /// With In Merge Mode, Clicking Stroke Splits the Path and not in erase mode (`alt`): the run of
    /// stroke a click at `p` splits off ([`BuilderMap::stroke_at`]).
    fn stroke_at(&mut self, cx: &ToolContext, p: Point, alt: bool) -> Option<SubPath> {
        if alt || !self.opts.click_stroke_splits {
            return None;
        }
        self.cache.get(cx, self.opts.open_filled_as_closed);
        self.cache.map.stroke_at(&self.cache.outlines, p, cx.tol(LINE_TOL_PX)).map(|(_, run)| run)
    }

    /// What the pointer at `p` is over: a stroke a click would split, else what a gesture there
    /// touches. While the map of a large selection is being built on another thread, nothing.
    fn hover_at(&mut self, cx: &ToolContext, p: Point) {
        if self.cache.ready(cx, self.opts.open_filled_as_closed).is_none() {
            self.stroke = None;
            self.hover = None;
            return;
        }
        // Over a stroke a click would split, the tool highlights the stroke and not the region.
        self.stroke = self.stroke_at(cx, p, self.alt);
        self.hover = match self.stroke {
            Some(_) => None,
            None => self.cache.get(cx, self.opts.open_filled_as_closed).builder_hit(p, Self::line_tol(cx, self.alt)),
        };
    }

    fn line_tol(cx: &ToolContext, alt: bool) -> Option<f64> {
        alt.then(|| cx.tol(LINE_TOL_PX))
    }

    /// Starts computing the merge `params` describe on another thread when the map of the
    /// selection is current and has more than [`BACKGROUND_SEGMENTS`] segments; `false` when it
    /// did not (small selections, the web build), and the command computes it.
    fn start_merge(&mut self, cx: &ToolContext, params: &Value) -> bool {
        let key = RegionCache::key_for(cx, self.opts.open_filled_as_closed);
        if cfg!(target_arch = "wasm32") || params.get("splitStroke").is_some() || self.cache.key.as_ref() != Some(&key) {
            return false;
        }
        let map = Arc::clone(&self.cache.map);
        let segments: usize = map.shapes.iter().map(|s| s.path.subpaths.iter().map(|sp| sp.segment_count()).sum::<usize>()).sum();
        if segments <= BACKGROUND_SEGMENTS {
            return false;
        }
        // What the command reads from its params, read the same way.
        let num = |v: &Value| v.as_f64().unwrap_or(f64::NAN);
        let pts: Vec<Point> = params["points"].as_array().map(|a| a.iter().map(|q| Point::new(num(&q[0]), num(&q[1]))).collect()).unwrap_or_default();
        let marquee = params["rect"].as_array().filter(|r| r.len() == 4).map(|r| {
            let (x, y) = (num(&r[0]), num(&r[1]));
            (Point::new(x, y), Point::new(x + num(&r[2]), y + num(&r[3])))
        });
        let piece_tol = (params["erase"] == json!(true)).then(|| params["tolerance"].as_f64().unwrap_or(2.0).clamp(0.0, 1000.0));
        let touched = map.merge_touched(&pts, marquee, piece_tol);
        if touched.is_empty() {
            return false;
        }
        let roots = sorted_roots(cx.doc, &cx.selection.objects);
        let (_, leaves) = shape_builder_shapes(cx.doc, &roots, self.opts.open_filled_as_closed);
        let inputs = merge_inputs(&leaves, self.opts.open_filled_as_closed);
        let gap = self.gap();
        let Some(done) = spawn_merge(Arc::clone(&map), touched.clone(), gap, inputs.clone()) else { return false };
        self.merging = Some(PendingMerge { key, params: params.clone(), map, touched, gap, inputs, done });
        true
    }

    /// Takes the merge being computed on another thread once it has arrived, or with `wait` once
    /// it arrives: the action that applies it, while the document, the selection and its inputs are
    /// those it was computed for. When one of them changed (an edit or a selection made other than
    /// by a command, which applies the merge first), the merge is dropped at once, without waiting
    /// for the thread. A copy of the document (a save stamping its dates) and a new revision that leaves
    /// the inputs as they were keep the merge. When the thread failed, the command computes the
    /// merge and reports what went wrong.
    fn poll_merge(&mut self, cx: &ToolContext, wait: bool) -> Vec<Action> {
        let Some(mut m) = self.merging.take() else { return vec![] };
        let now = RegionCache::key_for(cx, m.key.3);
        if (now.1.0, &now.2) != (m.key.1.0, &m.key.2) {
            return vec![];
        }
        // A new revision that leaves the selection's inputs as they were (a view setting or a
        // preference that redraws the document) keeps the merge.
        if now.1 != m.key.1 {
            if !self.inputs_are(cx, m.key.3, &m.map.shapes) {
                return vec![];
            }
            m.key = now;
        }
        let got = if wait { m.done.recv().map_err(|_| TryRecvError::Disconnected) } else { m.done.try_recv() };
        let geometry = match got {
            Ok(g) => Some(g),
            Err(TryRecvError::Empty) => {
                self.merging = Some(m);
                return vec![];
            }
            Err(TryRecvError::Disconnected) => None,
        };
        self.prepared = geometry.map(|geometry| Arc::new(PreparedMerge { map: m.map, touched: m.touched, gap: m.gap, inputs: m.inputs, geometry }));
        vec![Action::Exec("shapeBuilder.merge".into(), m.params)]
    }

    /// Is the map of the selection being built on another thread (it is not current yet)?
    fn map_building(&mut self, cx: &ToolContext) -> bool {
        self.cache.ready(cx, self.opts.open_filled_as_closed).is_none() && self.cache.pending.is_some()
    }

    /// Are the inputs of the selection in `cx` `shapes`? The map, or the map being built, is filed
    /// under the key of `cx` first when they are its inputs ([`RegionCache::rekey`]).
    fn inputs_are(&mut self, cx: &ToolContext, open_filled_as_closed: bool, shapes: &[po::Shape]) -> bool {
        let _ = self.cache.rekey(cx, open_filled_as_closed);
        let key = RegionCache::key_for(cx, open_filled_as_closed);
        if self.cache.key.as_ref() == Some(&key) {
            return self.cache.shapes == shapes;
        }
        if let Some(p) = self.cache.pending.as_ref().filter(|p| p.key == key) {
            return p.shapes == shapes;
        }
        let roots = sorted_roots(cx.doc, &cx.selection.objects);
        shape_builder_shapes(cx.doc, &roots, open_filled_as_closed).0 == shapes
    }

    /// Is a gesture made while the map was being built released and waiting for the map?
    fn deferred_released(&self) -> bool {
        self.deferred.as_ref().is_some_and(|d| d.released)
    }

    /// Replays the released gesture made while the map was being built once the map is there, or
    /// with `wait` once it is built, and with `wait` also waits for the merge it starts: the
    /// actions the gesture gives. When the document, the selection or its inputs changed since the
    /// gesture, it is dropped; a new revision that leaves them as they were keeps it.
    fn replay_deferred(&mut self, cx: &ToolContext, wait: bool) -> Vec<Action> {
        if !self.deferred_released() {
            return vec![];
        }
        let ofac = self.opts.open_filled_as_closed;
        let now = RegionCache::key_for(cx, ofac);
        if let Some(mut d) = self.deferred.take() {
            if (d.key.1.0, &d.key.2) != (now.1.0, &now.2) || (d.key.1 != now.1 && !self.inputs_are(cx, ofac, &d.shapes)) {
                return vec![];
            }
            d.key = now;
            self.deferred = Some(d);
        }
        if wait {
            self.cache.get(cx, self.opts.open_filled_as_closed);
        } else if self.map_building(cx) {
            return vec![];
        }
        let Some(d) = self.deferred.take() else { return vec![] };
        let mut acts = vec![];
        for ev in &d.events {
            acts.extend(self.pointer(cx, ev));
        }
        if wait {
            acts.extend(self.poll_merge(cx, true));
        }
        acts
    }

    /// Adds what the drag from `from` to `to` touches, picked as `shapeBuilder.merge` picks it.
    fn touch(&mut self, cx: &ToolContext, from: Option<Point>, to: Point) {
        let pts: Vec<Point> = from.into_iter().chain([to]).collect();
        let tol = Self::line_tol(cx, self.alt);
        let touched = self.cache.get(cx, self.opts.open_filled_as_closed).builder_hits_along(&pts, tol);
        self.touched.extend(&touched);
    }

    /// Selection Straight Line: the drag touches what the line from its first point to `p` crosses.
    fn straight_to(&mut self, cx: &ToolContext, p: Point) {
        let Some(&start) = self.points.first() else { return };
        self.touched = Touched::default();
        if start == p {
            self.points = vec![start];
            self.touch(cx, None, start);
        } else {
            self.points = vec![start, p];
            self.touch(cx, Some(start), p);
        }
    }

    /// A Shift-drag: the drag touches the regions the marquee from where it started to `p` reaches;
    /// while the pointer is where the drag started, what a click there touches.
    fn marquee_to(&mut self, cx: &ToolContext, p: Point) {
        let Some((start, _)) = self.marquee else { return };
        self.marquee = Some((start, p));
        self.touched = Touched::default();
        if start == p {
            self.touch(cx, None, start);
        } else {
            self.touched.regions = self.cache.get(cx, self.opts.open_filled_as_closed).regions_in_marquee(start, p);
        }
    }

    /// The cursor swatch preview: the swatch before the current fill, the current fill (ringed)
    /// and the swatch after it, side by side above the pointer.
    fn swatch_chips(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some(at) = self.at else { return vec![] };
        let list = color_swatches(cx.doc);
        let fill = &cx.paint.fill;
        let rgba = |p: &Paint| match p {
            Paint::Solid { color, .. } => Some(color.to_rgba8(1.0)),
            _ => None,
        };
        let chip = |forward| step_swatch(&list, fill, self.picked, forward).and_then(|i| rgba(&list.get(i)?.paint));
        let (before, after) = (chip(false), chip(true));
        let y = at.y - cx.tol(22.0);
        [(before, -16.0, false), (rgba(fill), 0.0, true), (after, 16.0, false)]
            .into_iter()
            .filter_map(|(color, dx, selected)| Some(Overlay::Swatch { p: Point::new(at.x + cx.tol(dx), y), color: color?, selected }))
            .collect()
    }
}

impl Tool for ShapeBuilderTool {
    fn id(&self) -> &'static str {
        "shapeBuilder"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        // The command has taken the merge computed for the last gesture by now.
        self.prepared = None;
        match ev.kind {
            PointerKind::Move => {
                self.alt = ev.mods.alt;
                self.at = Some(p);
                self.hover_at(cx, p);
                vec![]
            }
            PointerKind::Down => {
                // While a merge is being computed, a press starts no gesture.
                if cx.selection.is_empty() || self.editing() {
                    return vec![];
                }
                self.dragging = true;
                self.alt = ev.mods.alt;
                self.marquee = ev.mods.shift.then_some((p, p));
                self.points = vec![p];
                self.far = false;
                self.touched = Touched::default();
                // While the map is being built on another thread, the gesture is recorded and
                // replayed once the map is there.
                if self.map_building(cx) {
                    let key = RegionCache::key_for(cx, self.opts.open_filled_as_closed);
                    let shapes = match self.cache.pending.as_ref().filter(|p| p.key == key) {
                        Some(p) => p.shapes.clone(),
                        None => self.cache.inputs(cx, self.opts.open_filled_as_closed).0,
                    };
                    self.deferred = Some(Deferred { key, shapes, events: vec![*ev], released: false });
                    return vec![];
                }
                self.touch(cx, None, p);
                vec![]
            }
            PointerKind::Drag if self.dragging => {
                self.at = Some(p);
                if let Some(d) = &mut self.deferred {
                    // The drag's line or marquee shows meanwhile.
                    d.events.push(*ev);
                    if let Some((a, _)) = self.marquee {
                        self.marquee = Some((a, p));
                    } else if self.opts.straight_line {
                        self.points.truncate(1);
                        self.points.push(p);
                    } else if self.points.last().is_none_or(|q| q.distance(p) >= cx.tol(1.0)) {
                        self.points.push(p);
                    }
                    return vec![];
                }
                if self.points.first().is_some_and(|s| s.distance(p) > cx.tol(CLICK_TOL_PX)) {
                    self.far = true;
                }
                if self.marquee.is_some() {
                    self.marquee_to(cx, p);
                    return vec![];
                }
                if self.opts.straight_line {
                    self.straight_to(cx, p);
                    return vec![];
                }
                let last = self.points.last().copied();
                if last.is_none_or(|q| q.distance(p) >= cx.tol(1.0)) {
                    self.touch(cx, last, p);
                    self.points.push(p);
                }
                vec![]
            }
            PointerKind::Up if self.dragging => {
                self.dragging = false;
                if let Some(d) = &mut self.deferred {
                    d.events.push(*ev);
                    d.released = true;
                    self.points.clear();
                    self.marquee = None;
                    return vec![];
                }
                if self.marquee.is_some() {
                    self.marquee_to(cx, p);
                } else if self.opts.straight_line {
                    self.straight_to(cx, p);
                } else if self.points.last().is_none_or(|q| *q != p) {
                    let last = self.points.last().copied();
                    self.touch(cx, last, p);
                    self.points.push(p);
                }
                let points = std::mem::take(&mut self.points);
                // The mode the drag started in, which its highlight showed.
                let erase = self.alt;
                // A marquee of no size is a click.
                let marquee = self.marquee.take().filter(|(a, b)| a != b);
                // With In Merge Mode, Clicking Stroke Splits the Path, a click on a stroke splits it
                // where the press was; a marquee splits nothing.
                let press = points.first().copied().filter(|s| marquee.is_none() && !self.far && s.distance(p) <= cx.tol(CLICK_TOL_PX));
                let split = press.filter(|q| self.stroke_at(cx, *q, erase).is_some());
                if std::mem::take(&mut self.touched).is_empty() && split.is_none() {
                    return vec![];
                }
                self.stroke = None;
                let roots = sorted_roots(cx.doc, &cx.selection.objects);
                let points = split.map_or(points, |q| vec![q]);
                let pts: Vec<Value> = points.iter().map(|q| json!([q.x, q.y])).collect();
                let mut params = json!({ "ids": json_ids(&roots), "points": pts, "erase": erase, "gap": self.gap() });
                if let Some((a, b)) = marquee
                    && let Some(o) = params.as_object_mut()
                {
                    o.remove("points");
                    o.insert("rect".into(), json!([a.x, a.y, b.x - a.x, b.y - a.y]));
                }
                if let Some(t) = Self::line_tol(cx, erase) {
                    params["tolerance"] = json!(t);
                }
                if split.is_some() {
                    params["splitStroke"] = json!(true);
                    params["tolerance"] = json!(cx.tol(LINE_TOL_PX));
                }
                if !self.opts.open_filled_as_closed {
                    params["openFilledAsClosed"] = json!(false);
                }
                if self.opts.swatches {
                    params["pickColorFrom"] = json!("swatches");
                }
                // A large selection's merge is computed on another thread; `poll` applies it.
                if self.start_merge(cx, &params) {
                    return vec![];
                }
                vec![Action::Exec("shapeBuilder.merge".into(), params)]
            }
            _ => vec![],
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        // While a merge is being computed, what its gesture touched stays highlighted.
        let map = self.merging.as_ref().map_or(&self.cache.map, |m| &m.map);
        // The mode of the gesture being merged, else the drag's or the hover's.
        let alt = self.merging.as_ref().filter(|_| !self.dragging).map_or(self.alt, |m| m.params["erase"] == json!(true));
        let color = if alt { HIGHLIGHT_RED } else { HATCH };
        let touched = if self.dragging { Some(&self.touched) } else { self.merging.as_ref().map(|m| &m.touched) };
        let show: Vec<Hit> = if let Some(t) = touched {
            let regions = t.regions.iter().map(|&i| Hit::Region(i));
            regions.chain(t.lines.iter().map(|&i| Hit::Line(i))).chain(map.erased_edges(t).into_iter().map(Hit::Edge)).collect()
        } else {
            self.hover.into_iter().collect()
        };
        for hit in show {
            match hit {
                // Highlight Fill off: merge mode leaves the regions as they are.
                Hit::Region(_) if !alt && !self.opts.highlight_fill => {}
                Hit::Region(i) => {
                    if let Some((_, bp, _)) = map.regions.get(i) {
                        out.push(Overlay::Path { path: hatch(bp, cx.tol(4.0)), color, width: 1.0, dashed: false });
                        out.push(Overlay::Path { path: bp.clone(), color, width: 1.0, dashed: false });
                    }
                }
                Hit::Line(i) | Hit::Edge(i) => {
                    let pieces = if matches!(hit, Hit::Line(_)) { &map.lines } else { &map.edges };
                    if let Some((l, _)) = pieces.get(i) {
                        out.push(Overlay::Path { path: l.path.to_bezpath(), color: HIGHLIGHT_RED, width: 3.0, dashed: false });
                    }
                }
            }
        }
        if self.dragging
            && let Some((a, b)) = self.marquee.filter(|(a, b)| a != b)
        {
            out.push(Overlay::Marquee(Rect::from_points(a, b)));
        }
        if self.dragging && self.points.len() > 1 {
            let mut bp = BezPath::new();
            bp.move_to(self.points[0]);
            for q in &self.points[1..] {
                bp.line_to(*q);
            }
            out.push(Overlay::Path { path: bp, color, width: 1.0, dashed: false });
        }
        if self.opts.swatches && self.opts.cursor_swatch_preview && !self.dragging {
            out.extend(self.swatch_chips(cx));
        }
        // Highlight Stroke When Editable draws the stroke a click would split.
        if let Some(run) = self.stroke.as_ref().filter(|_| self.opts.highlight_stroke && !self.dragging) {
            let mut path = BezPath::new();
            run.to_bezpath_into(&mut path);
            out.push(Overlay::Path { path, color: self.opts.stroke_color, width: 3.0, dashed: false });
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, m: Mods) -> Cursor {
        // A drag keeps the mode it started in; hovering follows the key.
        let erase = if self.dragging { self.alt } else { m.alt };
        if self.editing() {
            // The merge of the last gesture, or the map it waits for, is being computed.
            Cursor::Busy
        } else if erase {
            Cursor::ShapeBuilderErase
        } else if self.stroke.is_some() && !self.dragging {
            // Over a stroke a click splits.
            Cursor::Scissors
        } else {
            Cursor::ShapeBuilder
        }
    }

    fn busy(&self) -> bool {
        self.dragging
    }

    fn waiting(&self) -> bool {
        self.cache.waiting() || self.merging.is_some() || self.deferred.is_some()
    }

    fn editing(&self) -> bool {
        self.merging.is_some() || self.deferred_released()
    }

    fn finish_edit(&mut self, cx: &ToolContext) -> Vec<Action> {
        let mut acts = self.replay_deferred(cx, true);
        acts.extend(self.poll_merge(cx, true));
        acts
    }

    /// Choosing another tool or another document applies the merge being computed first.
    fn deactivate(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.finish_edit(cx)
    }

    fn poll(&mut self, cx: &ToolContext, wait: bool) -> Vec<Action> {
        let mut acts = self.poll_merge(cx, wait);
        if !acts.is_empty() || self.dragging {
            return acts;
        }
        // The map that arrives may be for an earlier revision; the hover then starts the current
        // one's, which `wait` waits for too.
        while self.cache.waiting() {
            self.cache.settle(wait);
            if let Some(p) = self.at {
                self.hover_at(cx, p);
            }
            if !wait {
                break;
            }
        }
        // A gesture made while the map was being built goes on once it is there.
        acts.extend(self.replay_deferred(cx, wait));
        acts
    }

    fn shape_builder_map(&self) -> Option<(Arc<BuilderMap>, f64)> {
        self.cache.key.is_some().then(|| (Arc::clone(&self.cache.map), self.cache.gap))
    }

    fn shape_builder_merge(&self) -> Option<Arc<PreparedMerge>> {
        self.prepared.clone()
    }

    /// With Pick Color From Color Swatches, the Left and Right arrow keys pick the color swatch
    /// before or after the current fill. Escape during a drag cancels it: the release after it
    /// merges and deletes nothing.
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        (self.opts.swatches && !self.dragging && matches!(key, ToolKey::Left | ToolKey::Right)) || (self.dragging && key == ToolKey::Escape)
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        // While a merge is being computed, the arrows leave the current fill it takes as it was.
        if !self.claims_key(cx, key) || self.editing() {
            return vec![];
        }
        if key == ToolKey::Escape {
            self.dragging = false;
            self.points.clear();
            self.touched = Touched::default();
            // A gesture recorded while the map is being built is canceled with it.
            self.deferred = None;
            (self.marquee, self.hover, self.far) = (None, None, false);
            return vec![];
        }
        let list = color_swatches(cx.doc);
        let Some((i, s)) = step_swatch(&list, &cx.paint.fill, self.picked, key == ToolKey::Right).and_then(|i| Some((i, list.get(i)?))) else {
            return vec![];
        };
        self.picked = Some(i);
        // The current fill only (no `ids`): the selected art keeps its own.
        vec![Action::Exec("paint.setFill".into(), json!({ "swatch": s.name, "ids": [] }))]
    }

    fn options(&self) -> Value {
        let o = &self.opts;
        let [r, g, b] = o.stroke_color;
        json!({
            "openFilledAsClosed": o.open_filled_as_closed,
            "pickColorFrom": if o.swatches { "swatches" } else { "artwork" },
            "cursorSwatchPreview": o.cursor_swatch_preview,
            "selection": if o.straight_line { "straightLine" } else { "freeform" },
            "highlightFill": o.highlight_fill,
            "gapDetection": o.gap_detection,
            "gapLength": o.gap_length,
            "clickStrokeSplitsPath": o.click_stroke_splits,
            "highlightStroke": o.highlight_stroke,
            "highlightColor": format!("#{r:02x}{g:02x}{b:02x}"),
        })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        let o = &mut self.opts;
        match key {
            "openFilledAsClosed" => o.open_filled_as_closed = v.as_bool().unwrap_or(o.open_filled_as_closed),
            "pickColorFrom" => o.swatches = v.as_str().map_or(o.swatches, |s| s == "swatches"),
            "cursorSwatchPreview" => o.cursor_swatch_preview = v.as_bool().unwrap_or(o.cursor_swatch_preview),
            "selection" => o.straight_line = v.as_str().map_or(o.straight_line, |s| s == "straightLine"),
            "highlightFill" => o.highlight_fill = v.as_bool().unwrap_or(o.highlight_fill),
            "gapDetection" => o.gap_detection = v.as_bool().unwrap_or(o.gap_detection),
            "gapLength" => {
                if let Some(n) = v.as_f64().filter(|n| n.is_finite()) {
                    o.gap_length = n.clamp(0.0, SHAPE_BUILDER_MAX_GAP);
                }
            }
            "clickStrokeSplitsPath" => o.click_stroke_splits = v.as_bool().unwrap_or(o.click_stroke_splits),
            "highlightStroke" => o.highlight_stroke = v.as_bool().unwrap_or(o.highlight_stroke),
            "highlightColor" => o.stroke_color = lp_color(v).unwrap_or(o.stroke_color),
            _ => {}
        }
        self.stroke = None;
        self.cache.clear();
        self.cache.gap = self.gap();
    }
}

/// Create a builder-family tool by id (None = not one of ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        "shapeBuilder" => Box::new(ShapeBuilderTool::default()),
        "livePaintBucket" => Box::new(LivePaintBucketTool::default()),
        "livePaintSelection" => Box::new(LivePaintSelectionTool::default()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
