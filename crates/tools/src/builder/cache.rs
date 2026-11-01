use super::lookup::{drawn_outlines, faces_of, shape_builder_shapes, shapes_for};
use super::map::{BuilderMap, sorted_roots};
use super::{Arc, Document, NodeId, PathData, Point, Receiver, ToolContext, TryRecvError, po};

/// What a [`RegionCache`] map was built for: the document's address and revision, the selected ids
/// and whether filled open paths count as closed.
pub(super) type CacheKey = (usize, (u64, u64), Vec<NodeId>, bool);

/// Hovering builds the map of inputs with more segments than this on another thread
/// ([`RegionCache::ready`]); smaller maps are built on the calling thread.
const BACKGROUND_SEGMENTS: usize = 128;

/// A map being built on another thread: the key it is for, the drawn outlines of its inputs, and
/// the channel it arrives on.
struct Pending {
    key: CacheKey,
    outlines: Vec<PathData>,
    done: Receiver<BuilderMap>,
}

/// The planar map of the current selection, cached by document identity, revision, selection and
/// whether filled open paths count as closed.
#[derive(Default)]
pub(super) struct RegionCache {
    pub(super) key: Option<CacheKey>,
    /// The map, which the Shape Builder's merge command takes too ([`Tool::shape_builder_map`]).
    pub(super) map: Arc<BuilderMap>,
    /// Live Paint faces (every path an edge, filled or not) rather than Shape Builder regions.
    live: bool,
    /// Gap Detection distance; changing tool options invalidates the cache.
    pub(super) gap: f64,
    /// The outlines of the Shape Builder's inputs as they are drawn (see [`BuilderMap::stroke_at`]).
    pub(super) outlines: Vec<PathData>,
    /// The map being built on another thread, if any.
    pending: Option<Pending>,
    /// The key of the last map whose build on another thread failed.
    failed: Option<CacheKey>,
    /// How many times the map has been built.
    #[cfg(test)]
    pub(super) builds: usize,
    /// How many of those builds ran on another thread.
    #[cfg(test)]
    pub(super) background: usize,
}

/// The map of `shapes`: Live Paint faces with `live`, else the Shape Builder map with its edges,
/// closing gaps up to `gap`.
fn build_map(shapes: Vec<po::Shape>, live: bool, gap: f64) -> BuilderMap {
    if live {
        let regions = if shapes.is_empty() { vec![] } else { faces_of(po::live_paint(&shapes).0) };
        BuilderMap { regions, ..Default::default() }
    } else {
        BuilderMap::of_shapes(shapes, true, gap)
    }
}

/// Builds the map of `shapes` ([`build_map`]) on a new thread; None when no thread starts.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_build(shapes: Vec<po::Shape>, live: bool, gap: f64) -> Option<Receiver<BuilderMap>> {
    let (tx, done) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("vectorcraft-region-map".into()).spawn(move || {
        // The tool may have dropped the receiver meanwhile (another tool, another selection).
        let _ = tx.send(build_map(shapes, live, gap));
    });
    spawned.is_ok().then_some(done)
}

/// The web build has no threads.
#[cfg(target_arch = "wasm32")]
fn spawn_build(_shapes: Vec<po::Shape>, _live: bool, _gap: f64) -> Option<Receiver<BuilderMap>> {
    None
}

impl RegionCache {
    pub(super) fn live_paint() -> Self {
        Self { live: true, ..Default::default() }
    }

    fn key_for(cx: &ToolContext, open_filled_as_closed: bool) -> CacheKey {
        (cx.doc as *const Document as usize, cx.revision, cx.selection.objects.clone(), open_filled_as_closed)
    }

    /// The inputs of the selection's map, back → front, and their drawn outlines (Shape Builder
    /// only).
    fn inputs(&self, cx: &ToolContext, open_filled_as_closed: bool) -> (Vec<po::Shape>, Vec<PathData>) {
        let roots = sorted_roots(cx.doc, &cx.selection.objects);
        if self.live {
            (shapes_for(cx.doc, &roots).0, vec![])
        } else {
            let (shapes, leaves) = shape_builder_shapes(cx.doc, &roots, open_filled_as_closed);
            let outlines = drawn_outlines(cx.doc, &leaves);
            (shapes, outlines)
        }
    }

    fn install(&mut self, key: CacheKey, map: BuilderMap, outlines: Vec<PathData>) {
        self.map = Arc::new(map);
        self.outlines = outlines;
        self.key = Some(key);
        #[cfg(test)]
        {
            self.builds += 1;
        }
    }

    /// Forgets the map and any map being built (the tool options changed).
    pub(super) fn clear(&mut self) {
        self.key = None;
        self.pending = None;
        self.failed = None;
    }

    /// Is a map being built on another thread?
    pub(super) fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// The map, built again only when the key changes, with filled open paths closed when
    /// `open_filled_as_closed` (Shape Builder only). A Shape Builder map has the outline edges too,
    /// so hovering and erasing use the same map. Waits for a map being built on another thread for
    /// the same key.
    pub(super) fn get(&mut self, cx: &ToolContext, open_filled_as_closed: bool) -> &BuilderMap {
        let key = Self::key_for(cx, open_filled_as_closed);
        if self.key.as_ref() != Some(&key) {
            let built = self.pending.take().filter(|p| p.key == key).and_then(|p| Some((p.done.recv().ok()?, p.outlines)));
            let (map, outlines) = match built {
                Some(b) => {
                    #[cfg(test)]
                    {
                        self.background += 1;
                    }
                    b
                }
                None => {
                    let (shapes, outlines) = self.inputs(cx, open_filled_as_closed);
                    (build_map(shapes, self.live, self.gap), outlines)
                }
            };
            self.install(key, map, outlines);
        }
        &self.map
    }

    /// Takes the map being built on another thread once it has arrived, or with `wait` once it
    /// arrives, as the map for the key it was built for.
    pub(super) fn settle(&mut self, wait: bool) {
        let Some(p) = self.pending.take() else { return };
        let got = if wait { p.done.recv().map_err(|_| TryRecvError::Disconnected) } else { p.done.try_recv() };
        match got {
            Ok(map) => {
                #[cfg(test)]
                {
                    self.background += 1;
                }
                self.install(p.key, map, p.outlines);
            }
            // The build failed (a bug in the planar map): hovering does not build it again, and a
            // gesture builds it on the calling thread, which reports the failure.
            Err(TryRecvError::Disconnected) => self.failed = Some(p.key),
            Err(TryRecvError::Empty) => self.pending = Some(p),
        }
    }

    /// The map as [`Self::get`] gives it, without waiting: when the key changes and the inputs have
    /// more than [`BACKGROUND_SEGMENTS`] segments, the map is built on another thread, and this is
    /// None until it arrives ([`Self::waiting`] holds meanwhile). A map being built for an earlier
    /// key is let finish before the next one starts.
    pub(super) fn ready(&mut self, cx: &ToolContext, open_filled_as_closed: bool) -> Option<&BuilderMap> {
        self.settle(false);
        let key = Self::key_for(cx, open_filled_as_closed);
        if self.key.as_ref() == Some(&key) {
            return Some(&self.map);
        }
        if self.pending.is_some() || self.failed.as_ref() == Some(&key) {
            return None;
        }
        let (shapes, outlines) = self.inputs(cx, open_filled_as_closed);
        let segments: usize = shapes.iter().map(|s| s.path.subpaths.iter().map(|sp| sp.segment_count()).sum::<usize>()).sum();
        if segments > BACKGROUND_SEGMENTS && cfg!(not(target_arch = "wasm32")) {
            if let Some(done) = spawn_build(shapes, self.live, self.gap) {
                self.pending = Some(Pending { key, outlines, done });
                return None;
            }
            // No thread started: build here.
            return Some(self.get(cx, open_filled_as_closed));
        }
        let map = build_map(shapes, self.live, self.gap);
        self.install(key, map, outlines);
        Some(&self.map)
    }

    pub(super) fn at(&mut self, cx: &ToolContext, p: Point) -> Option<usize> {
        self.get(cx, true).region_at(p)
    }
}
