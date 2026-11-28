use super::lookup::{drawn_outlines, faces_of, shape_builder_shapes, shapes_for};
use super::map::{BuilderMap, sorted_roots};
use super::{Arc, Document, NodeId, PathData, Point, Receiver, ToolContext, TryRecvError, po};

/// What a [`RegionCache`] map was built for: the document's address and revision, the selected ids
/// and whether filled open paths count as closed.
pub(super) type CacheKey = (usize, (u64, u64), Vec<NodeId>, bool);

/// Hovering builds the map of inputs with more segments than this on another thread
/// ([`RegionCache::ready`]); smaller maps are built on the calling thread.
pub(super) const BACKGROUND_SEGMENTS: usize = 128;

/// A map being built on another thread: the key it is for, its inputs and their drawn outlines,
/// and the channel it arrives on.
pub(super) struct Pending {
    pub(super) key: CacheKey,
    pub(super) shapes: Vec<po::Shape>,
    outlines: Vec<PathData>,
    done: Receiver<BuilderMap>,
}

/// The planar map of the current selection, cached by document identity, revision, selection and
/// whether filled open paths count as closed. When only the document's identity or revision
/// changed and the selection's inputs did not (a view setting, a preference, a copy of the
/// document), the map is kept under the new key ([`RegionCache::rekey`]).
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
    /// The inputs the map was built from.
    pub(super) shapes: Vec<po::Shape>,
    /// The last key whose inputs [`Self::rekey`] compared with the map's.
    compared: Option<CacheKey>,
    /// The map being built on another thread, if any.
    pub(super) pending: Option<Pending>,
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

    pub(super) fn key_for(cx: &ToolContext, open_filled_as_closed: bool) -> CacheKey {
        (cx.doc as *const Document as usize, cx.revision, cx.selection.objects.clone(), open_filled_as_closed)
    }

    /// The inputs of the selection's map, back → front, and their drawn outlines (Shape Builder
    /// only).
    pub(super) fn inputs(&self, cx: &ToolContext, open_filled_as_closed: bool) -> (Vec<po::Shape>, Vec<PathData>) {
        let roots = sorted_roots(cx.doc, &cx.selection.objects);
        if self.live {
            (shapes_for(cx.doc, &roots).0, vec![])
        } else {
            let (shapes, leaves) = shape_builder_shapes(cx.doc, &roots, open_filled_as_closed);
            let outlines = drawn_outlines(cx.doc, &leaves);
            (shapes, outlines)
        }
    }

    fn install(&mut self, key: CacheKey, map: BuilderMap, shapes: Vec<po::Shape>, outlines: Vec<PathData>) {
        self.map = Arc::new(map);
        self.shapes = shapes;
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
            let inputs = self.rekey(cx, open_filled_as_closed);
            if self.key.as_ref() == Some(&key) {
                return &self.map;
            }
            let built = self.pending.take().filter(|p| p.key == key).and_then(|p| Some((p.done.recv().ok()?, p.shapes, p.outlines)));
            let (map, shapes, outlines) = match built {
                Some(b) => {
                    #[cfg(test)]
                    {
                        self.background += 1;
                    }
                    b
                }
                None => {
                    let (shapes, outlines) = inputs.unwrap_or_else(|| self.inputs(cx, open_filled_as_closed));
                    (build_map(shapes.clone(), self.live, self.gap), shapes, outlines)
                }
            };
            self.install(key, map, shapes, outlines);
        }
        &self.map
    }

    /// When the map, or the map being built, is for the selection of `cx` but for another revision
    /// or address of the document, and the selection's inputs and their outlines are those it is
    /// made from, files it under the key of `cx`: nothing it is made from changed. Returns the
    /// inputs it computed when that did not file a map (they changed), for the caller to build
    /// from.
    pub(super) fn rekey(&mut self, cx: &ToolContext, open_filled_as_closed: bool) -> Option<(Vec<po::Shape>, Vec<PathData>)> {
        let key = Self::key_for(cx, open_filled_as_closed);
        let other = |k: &CacheKey| *k != key && (&k.2, k.3) == (&key.2, key.3);
        if self.pending.as_ref().is_some_and(|p| p.key == key) {
            return None;
        }
        let installed = self.key.as_ref().is_some_and(other);
        let building = self.pending.as_ref().is_some_and(|p| other(&p.key));
        // The inputs are compared once for each key.
        if (!installed && !building) || self.compared.as_ref() == Some(&key) {
            return None;
        }
        self.compared = Some(key.clone());
        let (shapes, outlines) = self.inputs(cx, open_filled_as_closed);
        if installed && self.shapes == shapes && self.outlines == outlines {
            self.key = Some(key);
            return None;
        }
        if let Some(p) = self.pending.as_mut().filter(|p| other(&p.key) && p.shapes == shapes && p.outlines == outlines) {
            p.key = key;
            return None;
        }
        Some((shapes, outlines))
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
                self.install(p.key, map, p.shapes, p.outlines);
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
        let inputs = self.rekey(cx, open_filled_as_closed);
        if self.key.as_ref() == Some(&key) {
            return Some(&self.map);
        }
        if self.pending.is_some() || self.failed.as_ref() == Some(&key) {
            return None;
        }
        let (shapes, outlines) = inputs.unwrap_or_else(|| self.inputs(cx, open_filled_as_closed));
        let segments: usize = shapes.iter().map(|s| s.path.subpaths.iter().map(|sp| sp.segment_count()).sum::<usize>()).sum();
        if segments > BACKGROUND_SEGMENTS && cfg!(not(target_arch = "wasm32")) {
            if let Some(done) = spawn_build(shapes.clone(), self.live, self.gap) {
                self.pending = Some(Pending { key, shapes, outlines, done });
                return None;
            }
            // No thread started: build here.
            return Some(self.get(cx, open_filled_as_closed));
        }
        let map = build_map(shapes.clone(), self.live, self.gap);
        self.install(key, map, shapes, outlines);
        Some(&self.map)
    }

    pub(super) fn at(&mut self, cx: &ToolContext, p: Point) -> Option<usize> {
        self.get(cx, true).region_at(p)
    }
}
