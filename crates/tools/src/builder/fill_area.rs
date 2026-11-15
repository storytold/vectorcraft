//! The Fill Area tool: a click inside an area that visible, unlocked paths enclose adds a new
//! closed path of that area with the current fill and stroke (`shapeBuilder.copyRegion`). The
//! paths around the area stay as they are, and nothing needs to be selected.

use super::lookup::path_contains;
use super::*;

/// The area [`AreaFinder::find`] finds under a point.
#[derive(Clone, Debug, PartialEq)]
pub struct EnclosedArea {
    /// The area's outline, as a Shape Builder merge of that one region makes it.
    pub path: PathData,
    /// The object directly in a layer that holds the front-most path along the area (its outline
    /// touches the area's outline, or it covers the area): the new path goes directly above it, so
    /// it never goes inside a group, a Live Paint group, a Shaper group or a clipping group.
    pub above: NodeId,
}

/// How many times [`AreaFinder::find`] widens the part of the document it maps before it maps
/// every path.
const WINDOW_ROUNDS: usize = 24;

/// The side (pt) of the first window [`AreaFinder::find`] maps around a point that no path's bounds
/// hold.
const FIRST_WINDOW: f64 = 32.0;

/// How many times [`AreaFinder::find`] makes the window four times larger when the paths in it hold
/// no area under the point, before it maps every path: a 32 pt window reaches 8,192 pt.
const NEAR_ROUNDS: usize = 4;

/// Finds the area under a point among the paths of some objects, mapping only the paths near the
/// point first: an area that lies inside a window is bounded by the paths whose bounds reach the
/// window, so the paths outside it cannot change it. When the maps of [`NEAR_ROUNDS`] larger windows
/// have no area under the point, or the area does not fit after [`WINDOW_ROUNDS`] widenings, the
/// finder maps every path. When a window's paths have more segments than one Shape Builder map
/// takes, the paths with the most segments are left out until the rest fit, and an area is taken
/// only when no path left out reaches it. When every path together has more segments than that,
/// an area no window finds is not found. The map of every path, once a search has needed it, is
/// kept for the next search.
pub struct AreaFinder {
    /// The inputs back → front, as [`shape_builder_shapes`] gives them, with their bounds and the
    /// object directly in a layer that holds each one ([`outermost`]).
    inputs: Vec<(po::Shape, Rect, NodeId)>,
    /// The bounds of every input.
    total: Option<Rect>,
    gap: f64,
    /// The segments of all the inputs.
    segments: usize,
    /// Whether all the inputs fit the segment limit of one Shape Builder map. Past it a map keeps
    /// only the filled areas and loses the lines between them, so no map of every input is made.
    fits: bool,
    /// The map of every input, with the index of each of its shapes in `inputs`.
    full: Option<(BuilderMap, Vec<usize>)>,
}

/// Does `r` hold `p`, edges included?
fn holds(r: &Rect, p: Point) -> bool {
    p.x >= r.x0 && p.x <= r.x1 && p.y >= r.y0 && p.y <= r.y1
}

/// Do `a` and `b` overlap or touch?
fn meet(a: &Rect, b: &Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Does `outer` hold all of `inner`?
fn within(inner: &Rect, outer: &Rect) -> bool {
    inner.x0 >= outer.x0 && inner.x1 <= outer.x1 && inner.y0 >= outer.y0 && inner.y1 <= outer.y1
}

/// The object directly in a layer (or sublayer) that holds `id`: `id` itself when a layer holds it.
fn outermost(doc: &Document, id: NodeId) -> NodeId {
    let Some(line) = doc.ancestry(id) else { return id };
    let layer = line.iter().rposition(|a| doc.node(*a).is_some_and(|n| n.is_layer()));
    layer.and_then(|k| line.get(k + 1)).copied().unwrap_or(id)
}

impl AreaFinder {
    /// The finder over the paths of `roots` (layers or objects) that the Shape Builder takes:
    /// visible, unlocked paths, compound paths and Live Paint faces and edges that draw, not on a
    /// locked artboard. Gaps up to `gap` close, and with `open_filled_as_closed` a filled open path
    /// bounds an area as if an invisible edge closed it.
    pub fn new(doc: &Document, roots: &[NodeId], gap: f64, open_filled_as_closed: bool) -> Self {
        let roots = sorted_roots(doc, roots);
        let (shapes, leaves) = shape_builder_shapes(doc, &roots, open_filled_as_closed);
        let editable = doc.editable();
        let inputs: Vec<(po::Shape, Rect, NodeId)> = shapes
            .into_iter()
            .zip(leaves)
            .filter(|(_, l)| editable(l.id))
            .filter_map(|(s, l)| {
                let b = s.path.bounds()?;
                Some((s, b, outermost(doc, l.id)))
            })
            .collect();
        let total = inputs.iter().map(|(_, b, _)| *b).reduce(|a, b| a.union(b));
        let gap = if gap.is_finite() { gap.clamp(0.0, SHAPE_BUILDER_MAX_GAP) } else { 0.0 };
        let segments: usize = inputs.iter().flat_map(|(s, ..)| &s.path.subpaths).map(|sp| sp.segment_count()).sum();
        Self { inputs, total, gap, segments, fits: segments <= po::SHAPE_BUILDER_MAX_SEGMENTS, full: None }
    }

    /// The area under `p`: the smallest region of the map of the paths that holds `p`. None when
    /// `p` is in no enclosed area.
    pub fn find(&mut self, p: Point) -> Option<EnclosedArea> {
        if let Some((map, picked)) = &self.full {
            return self.area_in(map, picked, p);
        }
        let total = self.total?;
        // A gap bridge reaches at most `gap` past a path, so a window takes in every path within
        // that of it.
        let reach = self.gap + 1.0;
        if !holds(&total.inflate(reach, reach), p) {
            return None;
        }
        let mut window = self
            .inputs
            .iter()
            .filter(|(_, b, _)| holds(&b.inflate(reach, reach), p))
            .map(|(_, b, _)| *b)
            .reduce(|a, b| a.union(b))
            .unwrap_or_else(|| Rect::from_center_size(p, (FIRST_WINDOW, FIRST_WINDOW)));
        let mut near = 0;
        let mut every_path = false;
        // The paths left out of the windows' maps to keep them within the segment limit, by index.
        let mut left_out = vec![false; self.inputs.len()];
        let mut any_left_out = false;
        for round in 0..=WINDOW_ROUNDS {
            let last = every_path || round == WINDOW_ROUNDS;
            if last && !self.fits {
                return None;
            }
            let zone = window.inflate(reach, reach);
            let mut picked: Vec<usize> = (0..self.inputs.len())
                .filter(|&i| last || (!left_out.get(i).is_some_and(|o| *o) && self.inputs.get(i).is_some_and(|(_, b, _)| meet(b, &zone))))
                .collect();
            // Past the segment limit a map keeps only the filled areas, so the paths with the most
            // segments stay out of the window's map.
            if !last {
                for i in self.over_limit(&picked) {
                    if let Some(o) = left_out.get_mut(i) {
                        *o = true;
                        any_left_out = true;
                    }
                }
                picked.retain(|&i| !left_out.get(i).is_some_and(|o| *o));
            }
            let every = picked.len() == self.inputs.len();
            let shapes: Vec<po::Shape> = picked.iter().filter_map(|&i| self.inputs.get(i).map(|(s, ..)| s.clone())).collect();
            let map = BuilderMap::of_shapes(shapes, false, self.gap);
            let bounds = map.region_at(p).and_then(|k| map.regions.get(k)).map(|(.., rb)| *rb);
            if every {
                let area = self.area_in(&map, &picked, p);
                self.full = Some((map, picked));
                return area;
            }
            let before = window;
            match bounds {
                // A path left out that reaches the area could cut it, and only the map of every
                // path holds both.
                Some(rb) if within(&rb, &window) => {
                    let reached = rb.inflate(reach, reach);
                    let cut = any_left_out && self.inputs.iter().zip(&left_out).any(|((_, b, _), out)| *out && meet(b, &reached));
                    if !cut {
                        return self.area_in(&map, &picked, p);
                    }
                    every_path = true;
                }
                Some(rb) => window = window.union(rb),
                // No area of the paths near the point holds it: an area that does is bounded by
                // paths farther away. The window grows four times over a few rounds, and then the
                // next round maps every path.
                None if near < NEAR_ROUNDS => {
                    near += 1;
                    let (w, h) = (window.width().max(FIRST_WINDOW) * 4.0, window.height().max(FIRST_WINDOW) * 4.0);
                    window = window.union(Rect::from_center_size(window.center(), (w, h)));
                }
                None => window = total,
            }
            // A window that did not grow would give the same map again.
            if window == before {
                every_path = true;
            }
        }
        None
    }

    /// The inputs among `picked` to leave out of a map so that the rest fit the segment limit of
    /// one Shape Builder map: the ones with the most segments first. Empty when all of `picked` fit.
    fn over_limit(&self, picked: &[usize]) -> Vec<usize> {
        let segments = |i: usize| self.inputs.get(i).map_or(0, |(s, ..)| s.path.subpaths.iter().map(|sp| sp.segment_count()).sum::<usize>());
        let mut sum: usize = picked.iter().map(|&i| segments(i)).sum();
        let mut by_size: Vec<(usize, usize)> = picked.iter().map(|&i| (segments(i), i)).collect();
        by_size.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
        let mut out = vec![];
        for (n, i) in by_size {
            if sum <= po::SHAPE_BUILDER_MAX_SEGMENTS {
                break;
            }
            sum -= n;
            out.push(i);
        }
        out
    }

    /// The area under `p` in `map`, made of the inputs `picked` (in its order).
    fn area_in(&self, map: &BuilderMap, picked: &[usize], p: Point) -> Option<EnclosedArea> {
        let (region, ..) = map.region_at(p).and_then(|k| map.regions.get(k))?;
        let path = po::FaceMerger::with_gap(&map.shapes, self.gap).merge(&[region]);
        if path.is_empty() {
            return None;
        }
        let anchors: Vec<Point> = region.path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)).collect();
        // The front-most shape that covers the area or that an anchor of its outline lies on. A
        // shape within the gap of an anchor counts only when no shape is along the area itself,
        // so a path near the area that bounds none of it does not decide where the new path goes.
        let along = |gap: f64| {
            map.shapes.iter().enumerate().rev().find(|(i, s)| {
                let tol = outline_snap(&s.path) + gap;
                region.sources.contains(i)
                    || s.path.bounds().is_some_and(|b| {
                        let b = b.inflate(tol, tol);
                        anchors.iter().any(|q| holds(&b, *q) && s.path.nearest(*q).is_some_and(|(.., d)| d <= tol))
                    })
            })
        };
        let k = along(0.0).or_else(|| along(self.gap)).map_or(map.shapes.len().checked_sub(1)?, |(i, _)| i);
        let above = picked.get(k).and_then(|&i| self.inputs.get(i)).map(|(.., id)| *id)?;
        Some(EnclosedArea { path, above })
    }
}

/// The paths the Fill Area tool finds areas among: every layer's.
pub fn every_layer(doc: &Document) -> Vec<NodeId> {
    doc.layers.iter().map(|l| l.id).collect()
}

/// Fill Area (in the Live Paint Bucket's group): a click inside an area that visible, unlocked
/// paths on any layer enclose adds a new closed path of that area with the current fill and stroke
/// (`shapeBuilder.copyRegion`), directly above the front-most object along it. The paths stay as
/// they are and nothing needs to be selected. The area under the pointer is outlined while it
/// hovers. Gap Detection and Consider Open Filled Paths as Closed are the Shape Builder's options.
#[derive(Default)]
pub struct FillAreaTool {
    gap_detection: bool,
    gap_length: f64,
    /// Consider Open Filled Paths as Closed turned off: a filled open path cuts areas as a line
    /// does.
    open_filled_cuts: bool,
    /// The finder for the document as it is, with the key it was made for. Away while a search
    /// runs on another thread.
    finder: Option<(FinderKey, AreaFinder)>,
    /// The area under the pointer, with the document revision it was found in.
    hover: Option<((u64, u64), EnclosedArea)>,
    /// Where the pointer last moved over the canvas.
    at: Option<Point>,
    /// A hover search running on another thread.
    job: Option<HoverJob>,
    /// The key whose search on another thread failed: hovering does not search it again.
    failed: Option<FinderKey>,
    /// A press added the path as a preview, which the release keeps.
    pressing: bool,
}

/// The document revision, the gap and the open-filled rule an [`AreaFinder`] was made with.
type FinderKey = ((u64, u64), f64, bool);

/// A search for the area under `p` on another thread, which hands back the finder it took.
struct HoverJob {
    key: FinderKey,
    p: Point,
    done: std::sync::mpsc::Receiver<(AreaFinder, Option<EnclosedArea>)>,
}

/// Runs `finder.find(p)` on another thread; gives the finder back when no thread starts.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_find(finder: AreaFinder, p: Point) -> Result<std::sync::mpsc::Receiver<(AreaFinder, Option<EnclosedArea>)>, Box<AreaFinder>> {
    let (tx, done) = std::sync::mpsc::channel();
    let (back_tx, back) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("vectorcraft-fill-area".into()).spawn(move || {
        // The finder comes in through a channel, so a thread that does not start leaves it here.
        if let Ok(mut f) = back.recv() {
            let area = AreaFinder::find(&mut f, p);
            // The tool may have dropped the receiver meanwhile (another tool, a new revision).
            let _ = tx.send((f, area));
        }
    });
    match spawned {
        Ok(_) => match back_tx.send(finder) {
            Ok(()) => Ok(done),
            Err(e) => Err(Box::new(e.0)),
        },
        Err(_) => Err(Box::new(finder)),
    }
}

/// The web build has no threads.
#[cfg(target_arch = "wasm32")]
fn spawn_find(finder: AreaFinder, _p: Point) -> Result<std::sync::mpsc::Receiver<(AreaFinder, Option<EnclosedArea>)>, Box<AreaFinder>> {
    Err(Box::new(finder))
}

impl FillAreaTool {
    /// A tool with the Shape Builder's default options.
    pub fn new() -> Self {
        Self { gap_length: GAP_LENGTHS[0], ..Self::default() }
    }

    fn gap(&self) -> f64 {
        if self.gap_detection { self.gap_length } else { 0.0 }
    }

    fn open_filled_as_closed(&self) -> bool {
        !self.open_filled_cuts
    }

    fn finder_key(&self, cx: &ToolContext) -> FinderKey {
        (cx.revision, self.gap(), self.open_filled_as_closed())
    }

    /// The finder for the document as `cx` has it, made again when the key changed.
    fn take_finder(&mut self, cx: &ToolContext) -> AreaFinder {
        let key = self.finder_key(cx);
        match self.finder.take() {
            Some((k, f)) if k == key => f,
            _ => AreaFinder::new(cx.doc, &every_layer(cx.doc), key.1, key.2),
        }
    }

    /// The hover area when it is current and holds `p`.
    fn hover_holding(&self, cx: &ToolContext, p: Point) -> Option<&EnclosedArea> {
        self.hover.as_ref().filter(|(rev, a)| *rev == cx.revision && path_contains(&a.path, FillRule::NonZero, p)).map(|(_, a)| a)
    }

    /// The area under `p`, found here and now (a click). A search running on another thread is
    /// waited for first, so its finder is not made again.
    fn area_at(&mut self, cx: &ToolContext, p: Point) -> Option<EnclosedArea> {
        if let Some(a) = self.hover_holding(cx, p) {
            return Some(a.clone());
        }
        while self.job.is_some() {
            self.settle(cx, true);
        }
        let key = self.finder_key(cx);
        let mut finder = self.take_finder(cx);
        let area = finder.find(p);
        self.finder = Some((key, finder));
        area
    }

    /// Looks for the area under `p` for the hover. A search of more than [`BACKGROUND_SEGMENTS`]
    /// segments runs on another thread, and [`Tool::poll`] takes its result; while one runs, a new
    /// position waits for it.
    fn hover_at(&mut self, cx: &ToolContext, p: Point) {
        self.at = Some(p);
        if self.hover_holding(cx, p).is_some() {
            return;
        }
        // The outline of an area the pointer left goes at once, while the next search runs.
        self.hover = None;
        if self.job.is_some() {
            return;
        }
        let key = self.finder_key(cx);
        if self.failed == Some(key) {
            self.hover = None;
            return;
        }
        let mut finder = self.take_finder(cx);
        if finder.segments > BACKGROUND_SEGMENTS {
            match spawn_find(finder, p) {
                Ok(done) => {
                    self.job = Some(HoverJob { key, p, done });
                    return;
                }
                Err(f) => finder = *f,
            }
        }
        self.hover = finder.find(p).map(|a| (key.0, a));
        self.finder = Some((key, finder));
    }

    /// Takes the result of the hover search on another thread when it is done (`wait`: waits for
    /// it), and starts the search for where the pointer moved meanwhile.
    fn settle(&mut self, cx: &ToolContext, wait: bool) {
        let Some(job) = self.job.take() else { return };
        let got = if wait {
            job.done.recv().ok()
        } else {
            match job.done.try_recv() {
                Ok(v) => Some(v),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.job = Some(job);
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
            }
        };
        let Some((finder, area)) = got else {
            // The search failed (a bug in the planar map): hovering does not search it again, and
            // a click searches here, which reports the failure.
            self.failed = Some(job.key);
            self.hover = None;
            return;
        };
        let current = job.key == self.finder_key(cx);
        if current {
            self.finder = Some((job.key, finder));
        }
        // The area found serves wherever the pointer moved meanwhile, as long as it holds it.
        let at = self.at;
        self.hover = area.filter(|a| current && at.is_some_and(|p| path_contains(&a.path, FillRule::NonZero, p))).map(|a| (job.key.0, a));
        if let Some(p) = at
            && self.hover.is_none()
            && (p != job.p || !current)
        {
            self.hover_at(cx, p);
        }
    }
}

impl Tool for FillAreaTool {
    fn id(&self) -> &'static str {
        "fillArea"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Move => {
                self.hover_at(cx, ev.pos);
                vec![]
            }
            PointerKind::Down => {
                // A click in no enclosed area does nothing.
                let found = self.area_at(cx, ev.pos).is_some();
                self.hover = None;
                if !found {
                    return vec![];
                }
                let mut params = json!({ "point": [ev.pos.x, ev.pos.y], "gap": self.gap() });
                if !self.open_filled_as_closed() {
                    params["openFilledAsClosed"] = json!(false);
                }
                // The path shows at once and is kept on the release; a press that is dropped (a
                // long or canceled touch, a second finger) or Escape before the release takes it
                // back.
                self.pressing = true;
                vec![Action::Begin("Fill Area".into()), Action::Preview("shapeBuilder.copyRegion".into(), params)]
            }
            PointerKind::Up if std::mem::take(&mut self.pressing) => vec![Action::Commit],
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && std::mem::take(&mut self.pressing) { vec![Action::Cancel] } else { vec![] }
    }

    fn busy(&self) -> bool {
        self.pressing
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        self.hover
            .iter()
            .filter(|(rev, _)| *rev == cx.revision)
            .map(|(_, a)| Overlay::Path { path: a.path.to_bezpath(), color: HIGHLIGHT_RED, width: 3.0, dashed: false })
            .collect()
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        if self.hover_holding(cx, p).is_some() { Cursor::LivePaintBucketFace } else { Cursor::LivePaintBucket }
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
        self.at = None;
    }

    /// The area under a still pointer changes with the document and the view.
    fn wants_rehover(&self) -> bool {
        true
    }

    fn waiting(&self) -> bool {
        self.job.is_some()
    }

    fn poll(&mut self, cx: &ToolContext, wait: bool) -> Vec<Action> {
        self.settle(cx, wait);
        while wait && self.job.is_some() {
            self.settle(cx, true);
        }
        vec![]
    }

    /// The Shape Builder's Gap Detection, Gap Length and Consider Open Filled Paths as Closed,
    /// which the tool shares.
    fn options(&self) -> Value {
        json!({
            "gapDetection": self.gap_detection,
            "gapLength": self.gap_length,
            "openFilledAsClosed": self.open_filled_as_closed(),
        })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "gapDetection" => self.gap_detection = v.as_bool().unwrap_or(self.gap_detection),
            "gapLength" => {
                if let Some(n) = v.as_f64().filter(|n| n.is_finite()) {
                    self.gap_length = n.clamp(0.0, SHAPE_BUILDER_MAX_GAP);
                }
            }
            "openFilledAsClosed" => {
                if let Some(b) = v.as_bool() {
                    self.open_filled_cuts = !b;
                }
            }
            _ => {}
        }
        self.hover = None;
    }
}
