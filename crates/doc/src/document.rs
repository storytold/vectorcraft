use super::artboard::{Artboard, LockedArtboards};
use super::graphic_style::default_graphic_styles;
use super::guides::{GridPrefs, RulerMode};
use super::limits::{DocError, FORMAT_VERSION, MAX_COORD, MAX_NEXT_ID};
use super::units::{ColorMode, Unit};
use super::{
    Arc, BTreeMap, ColorProfiles, DocMetadata, DocSetup, Document, LAYER_COLORS, LayerColor, Node, NodeId, NodeKind, PatternDef, Places, Point,
    RasterEffectsSettings, Rect, Vec2,
};

impl Document {
    /// Is everything but the layers the same in `other`, as the next version of the document after
    /// an edit of its art? The id counter and the id index are left out, and art outside the layers
    /// (symbols, patterns, graph designs) compares by identity, which an edit keeps for what it
    /// doesn't change: quick for every frame of an animation.
    pub fn same_outside_layers(&self, other: &Self) -> bool {
        let same_art = |a: &[Arc<Node>], b: &[Arc<Node>]| a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b));
        let same_pattern = |a: &PatternDef, b: &PatternDef| {
            same_art(&a.art, &b.art) && PatternDef { art: Vec::new(), ..a.clone() } == PatternDef { art: Vec::new(), ..b.clone() }
        };
        let Self {
            version,
            title,
            template,
            units,
            ruler_origin,
            rulers,
            artboard_ruler_origins,
            color_mode,
            artboards,
            layers: _,
            swatches,
            swatch_groups,
            graphic_styles,
            char_styles,
            para_styles,
            text_threads,
            symbols,
            graph_designs,
            guides,
            views,
            saved_selections,
            variables,
            grid,
            raster_effects_ppi,
            images,
            patterns,
            pattern_edit,
            mask_edit,
            next_id: _,
            unknown,
            page_isolate,
            page_knockout,
            spot_use_lab,
            setup,
            paste_remembers_layers,
            metadata,
            raster_effects,
            last_view,
            transparency_grid,
            layers_open,
            color_profiles,
            export_settings,
            extra,
            slices,
            slices_clip_to_artboard,
            print_setup,
            assets,
            svg_scripts,
            puppet,
            new_art,
            dimensions,
            retired_artboard_id: _,
            index: _,
        } = self;
        *version == other.version
            && *title == other.title
            && *template == other.template
            && *units == other.units
            && *ruler_origin == other.ruler_origin
            && *rulers == other.rulers
            && *artboard_ruler_origins == other.artboard_ruler_origins
            && *color_mode == other.color_mode
            && *artboards == other.artboards
            && *swatches == other.swatches
            && *swatch_groups == other.swatch_groups
            && *graphic_styles == other.graphic_styles
            && *char_styles == other.char_styles
            && *para_styles == other.para_styles
            && *text_threads == other.text_threads
            && symbols.len() == other.symbols.len()
            && symbols.iter().zip(&other.symbols).all(|(a, b)| a.name == b.name && Arc::ptr_eq(&a.art, &b.art))
            && graph_designs.len() == other.graph_designs.len()
            && graph_designs.iter().zip(&other.graph_designs).all(|(a, b)| a.name == b.name && Arc::ptr_eq(&a.art, &b.art))
            && *guides == other.guides
            && *views == other.views
            && *saved_selections == other.saved_selections
            && *variables == other.variables
            && *grid == other.grid
            && *raster_effects_ppi == other.raster_effects_ppi
            && *images == other.images
            && patterns.len() == other.patterns.len()
            && patterns.iter().zip(&other.patterns).all(|(a, b)| same_pattern(a, b))
            && match (pattern_edit, &other.pattern_edit) {
                (Some(a), Some(b)) => {
                    a.pattern == b.pattern
                        && a.layer == b.layer
                        && match (&a.original, &b.original) {
                            (Some(a), Some(b)) => same_pattern(a, b),
                            (a, b) => a.is_none() && b.is_none(),
                        }
                }
                (a, b) => a.is_none() && b.is_none(),
            }
            && *mask_edit == other.mask_edit
            && *unknown == other.unknown
            && *page_isolate == other.page_isolate
            && *page_knockout == other.page_knockout
            && *spot_use_lab == other.spot_use_lab
            && *setup == other.setup
            && *paste_remembers_layers == other.paste_remembers_layers
            && *metadata == other.metadata
            && *raster_effects == other.raster_effects
            && *last_view == other.last_view
            && *transparency_grid == other.transparency_grid
            && *layers_open == other.layers_open
            && *color_profiles == other.color_profiles
            && *export_settings == other.export_settings
            && *extra == other.extra
            && *slices == other.slices
            && *slices_clip_to_artboard == other.slices_clip_to_artboard
            && *print_setup == other.print_setup
            && *assets == other.assets
            && *svg_scripts == other.svg_scripts
            && match (puppet, &other.puppet) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (a, b) => a.is_none() && b.is_none(),
            }
            && *new_art == other.new_art
            && *dimensions == other.dimensions
    }

    /// Document Raster Effects Settings › Resolution in ppi, 1 to 2400 (72 when the stored value
    /// isn't a number).
    pub fn raster_ppi(&self) -> f64 {
        Some(self.raster_effects_ppi).filter(|v| v.is_finite()).unwrap_or(72.0).clamp(1.0, 2400.0)
    }

    /// The ruler zero point, or zero on both axes when either stored coordinate is invalid.
    /// Rulers, coordinate fields and guide snapping use the same supported range.
    pub fn ruler_origin_or_zero(&self) -> Point {
        let origin = self.ruler_origin;
        if origin.is_finite() && origin.x.abs() <= MAX_COORD && origin.y.abs() <= MAX_COORD { origin } else { Point::ZERO }
    }

    /// Where the rulers measure from with artboard `index` active: [`Self::ruler_origin_or_zero`]
    /// with global rulers; with artboard rulers that artboard's top-left corner moved by its own
    /// offset (the first artboard's when `index` is out of range, the global zero point when the
    /// document has no artboard or the result is out of the supported range).
    pub fn ruler_origin_for(&self, index: usize) -> Point {
        if self.rulers == RulerMode::Global {
            return self.ruler_origin_or_zero();
        }
        let Some(ab) = self.artboards.get(index).or_else(|| self.artboards.first()) else { return self.ruler_origin_or_zero() };
        let corner = Point::new(ab.rect.x0, ab.rect.y0);
        let offset = self.artboard_ruler_origins.get(&ab.id).copied().unwrap_or(Vec2::ZERO);
        let origin = corner + offset;
        if origin.is_finite() && origin.x.abs() <= MAX_COORD && origin.y.abs() <= MAX_COORD {
            origin
        } else if corner.is_finite() && corner.x.abs() <= MAX_COORD && corner.y.abs() <= MAX_COORD {
            corner
        } else {
            Point::ZERO
        }
    }

    /// A new RGB document with one artboard of `size` and one layer ("Layer 1").
    pub fn new(width: f64, height: f64) -> Self {
        Self::new_with_mode(width, height, ColorMode::Rgb)
    }

    /// [`Document::new`] in colour `mode`, with that mode's default swatches.
    pub fn new_with_mode(width: f64, height: f64, mode: ColorMode) -> Self {
        let (swatches, swatch_groups) = vectorcraft_color::default_swatches(mode.model());
        let mut d = Self {
            version: FORMAT_VERSION,
            title: "Untitled-1".into(),
            template: false,
            units: Unit::Points,
            ruler_origin: Point::ZERO,
            rulers: RulerMode::Artboard,
            artboard_ruler_origins: BTreeMap::new(),
            color_mode: mode,
            artboards: vec![Artboard {
                id: 1,
                name: "Artboard 1".into(),
                rect: Rect::new(0.0, 0.0, width, height),
                locked: false,
                show_center_mark: false,
                show_cross_hairs: false,
                show_video_safe_areas: false,
                video_ruler_pixel_aspect_ratio: 1.0,
                color: None,
            }],
            layers: vec![],
            swatches: swatches.into(),
            swatch_groups: swatch_groups.into(),
            graphic_styles: default_graphic_styles().into(),
            char_styles: Default::default(),
            para_styles: Default::default(),
            text_threads: vec![],
            symbols: vec![],
            graph_designs: vec![],
            guides: vec![],
            views: vec![],
            saved_selections: Default::default(),
            variables: Default::default(),
            grid: GridPrefs::default(),
            raster_effects_ppi: 72.0,
            images: BTreeMap::new(),
            patterns: vec![],
            pattern_edit: None,
            mask_edit: None,
            next_id: 1,
            unknown: Default::default(),
            page_isolate: false,
            page_knockout: false,
            spot_use_lab: true,
            setup: DocSetup::default(),
            paste_remembers_layers: false,
            metadata: DocMetadata::default(),
            raster_effects: RasterEffectsSettings::default(),
            last_view: None,
            transparency_grid: false,
            layers_open: None,
            color_profiles: ColorProfiles::default(),
            export_settings: Default::default(),
            extra: Default::default(),
            slices: vec![],
            slices_clip_to_artboard: true,
            print_setup: None,
            assets: vec![],
            svg_scripts: vec![],
            puppet: None,
            new_art: Default::default(),
            dimensions: false,
            retired_artboard_id: Default::default(),
            index: Default::default(),
        };
        let id = d.alloc_id();
        d.layers.push(Arc::new(Node::layer(id, "Layer 1", LayerColor::Preset(0))));
        d
    }

    /// Allocate a fresh node id.
    pub fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        // Never reached from a counter [`Self::fix_next_id`] has seen (it stays under
        // [`MAX_NEXT_ID`]); saturating so that no counter can overflow.
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub fn peek_next_id(&self) -> u64 {
        self.next_id
    }

    /// Ensure `next_id` is above every id in the tree (after deserializing foreign data).
    pub fn fix_next_id(&mut self) {
        // Ids from [`MAX_NEXT_ID`] up (a damaged or hand-made file: #1718) are left out, as is a
        // counter up there: new ids start under them and never reach them, where counting on from
        // the largest would run out and hand out ids the document already uses.
        let mut max = 0;
        let mut see = |id: u64| {
            if id < MAX_NEXT_ID {
                max = max.max(id);
            }
        };
        for l in &self.layers {
            l.walk(&mut |n| see(n.id.0));
        }
        self.slices.iter().for_each(|s| see(s.id.0));
        self.assets.iter().for_each(|a| see(a.id));
        let counter = Some(self.next_id).filter(|n| *n <= MAX_NEXT_ID).unwrap_or(0);
        self.next_id = counter.max(max.saturating_add(1));
    }

    /// Find a node anywhere in the tree.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.index_path(id).and_then(|p| self.node_at(&p))
    }

    /// [`Document::node`] without building the lookup cache when there is none: for a look at a
    /// document kept in the undo history ([`Document::drop_lookup_cache`]).
    pub fn node_uncached(&self, id: NodeId) -> Option<&Node> {
        match self.index.is_built() {
            true => self.node(id),
            false => self.walk_index_path(id).and_then(|p| self.node_at(&p)),
        }
    }

    /// Let go of the lookup cache ([`id_index`]), as a document kept in the undo history does: a
    /// table per step would add up. Lookups still work; the first one builds the cache again.
    pub fn drop_lookup_cache(&self) {
        self.index.clear();
    }

    /// Whether the lookup cache ([`id_index`]) is built (see [`Document::drop_lookup_cache`]).
    pub fn has_lookup_cache(&self) -> bool {
        self.index.is_built()
    }

    /// Index path from the top-level layer list to `id` (e.g. `[0, 3, 1]`): the first place in
    /// tree order where `id` appears.
    pub fn index_path(&self, id: NodeId) -> Option<Vec<usize>> {
        // The cached path, while the node there still has the id ([`id_index`]).
        let cached = self.index.get();
        match cached.as_ref().and_then(|m| m.get(&id)) {
            Some(Some(p)) if self.node_at(&p).is_some_and(|n| n.id == id) => return Some(p),
            Some(None) => return self.walk_index_path(id),
            _ => {}
        }
        // A stale path (the tree changed), an id the cache doesn't have (deleted or added since),
        // or no cache yet: walk, and rebuild now and then.
        let found = self.walk_index_path(id);
        let stale = cached.as_ref().is_some_and(|m| m.contains_key(&id));
        if cached.is_none() || ((stale || found.is_some()) && self.index.missed()) {
            self.index.rebuild(&self.layers);
        }
        found
    }

    /// [`Document::index_path`] by walking the tree.
    pub(crate) fn walk_index_path(&self, id: NodeId) -> Option<Vec<usize>> {
        fn find(nodes: &[Arc<Node>], id: NodeId, path: &mut Vec<usize>) -> bool {
            for (i, n) in nodes.iter().enumerate() {
                path.push(i);
                if n.id == id {
                    return true;
                }
                if let Some(ch) = n.children()
                    && find(ch, id, path)
                {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut p = vec![];
        find(&self.layers, id, &mut p).then_some(p)
    }

    /// [`Document::index_path`] of every id in `ids` that is in the tree, in one walk of it: an
    /// edit of many objects looks them all up for the cost of one search instead of one each. Like
    /// `index_path`, an id that appears twice maps to its first place in tree order.
    pub fn index_paths(&self, ids: impl IntoIterator<Item = NodeId>) -> std::collections::HashMap<NodeId, Vec<usize>> {
        use std::collections::{HashMap, HashSet};
        fn walk(nodes: &[Arc<Node>], want: &HashSet<NodeId>, path: &mut Vec<usize>, out: &mut HashMap<NodeId, Vec<usize>>) -> bool {
            for (i, n) in nodes.iter().enumerate() {
                path.push(i);
                if want.contains(&n.id) && !out.contains_key(&n.id) {
                    out.insert(n.id, path.clone());
                    if out.len() == want.len() {
                        return true;
                    }
                }
                if let Some(ch) = n.children()
                    && walk(ch, want, path, out)
                {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut want: HashSet<NodeId> = ids.into_iter().collect();
        let mut out = HashMap::with_capacity(want.len());
        // Cached paths that still check out ([`id_index`]); the rest take one walk. For a good
        // share of the document, one walk is quicker than checking each cached path.
        if let Some(cache) = self.index.get().filter(|c| want.len().saturating_mul(8) < c.len()) {
            want.retain(|id| match cache.get(id) {
                Some(Some(p)) if self.node_at(&p).is_some_and(|n| n.id == *id) => {
                    out.insert(*id, p);
                    false
                }
                _ => true,
            });
        }
        if !want.is_empty() {
            let mut rest = HashMap::with_capacity(want.len());
            walk(&self.layers, &want, &mut Vec::new(), &mut rest);
            out.extend(rest);
        }
        out
    }

    /// The node at index path `path` (as [`Document::index_path`] gives it).
    pub fn node_at(&self, path: &[usize]) -> Option<&Node> {
        let (first, rest) = path.split_first()?;
        let mut cur: &Node = self.layers.get(*first)?;
        for &i in rest {
            cur = cur.children()?.get(i)?;
        }
        Some(cur)
    }

    /// Mutable access to the node at index path `path`; clones the Arc path (copy-on-write), as
    /// [`Document::node_mut`] does.
    pub fn node_mut_at(&mut self, path: &[usize]) -> Option<&mut Node> {
        let (first, rest) = path.split_first()?;
        let mut cur: &mut Arc<Node> = self.layers.get_mut(*first)?;
        for &i in rest {
            cur = Arc::make_mut(cur).children_mut()?.get_mut(i)?;
        }
        Some(Arc::make_mut(cur))
    }

    /// [`Document::ancestry`] of the node at index path `path`.
    pub fn ancestry_at(&self, path: &[usize]) -> Option<Vec<NodeId>> {
        let mut out = Vec::with_capacity(path.len());
        let mut nodes = &self.layers;
        for &i in path {
            let n = nodes.get(i)?;
            out.push(n.id);
            if let Some(ch) = n.children() {
                nodes = ch;
            }
        }
        Some(out)
    }

    /// Ids from the top-level layer down to `id` inclusive.
    pub fn ancestry(&self, id: NodeId) -> Option<Vec<NodeId>> {
        let path = self.index_path(id)?;
        let mut out = Vec::with_capacity(path.len());
        let mut nodes = &self.layers;
        for &i in &path {
            let n = &nodes[i];
            out.push(n.id);
            if let Some(ch) = n.children() {
                nodes = ch;
            }
        }
        Some(out)
    }

    pub fn parent_of(&self, id: NodeId) -> Option<NodeId> {
        let a = self.ancestry(id)?;
        (a.len() >= 2).then(|| a[a.len() - 2])
    }

    /// The top-level layer that contains `id`.
    pub fn layer_of(&self, id: NodeId) -> Option<NodeId> {
        self.ancestry(id).and_then(|a| a.first().copied())
    }

    /// Mutable access to a node; clones the Arc path (copy-on-write).
    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let path = self.index_path(id)?;
        let mut cur: &mut Arc<Node> = &mut self.layers[path[0]];
        for &i in &path[1..] {
            cur = &mut Arc::make_mut(cur).children_mut()?[i];
        }
        Some(Arc::make_mut(cur))
    }

    /// Children list of a container (or the top-level layer list for `None`).
    pub fn children_mut(&mut self, parent: Option<NodeId>) -> Result<&mut Vec<Arc<Node>>, DocError> {
        match parent {
            None => Ok(&mut self.layers),
            Some(p) => self.node_mut(p).ok_or(DocError::NoNode(p))?.children_mut().ok_or(DocError::NotContainer(p)),
        }
    }

    pub fn children(&self, parent: Option<NodeId>) -> Option<&Vec<Arc<Node>>> {
        match parent {
            None => Some(&self.layers),
            Some(p) => self.node(p)?.children(),
        }
    }

    /// Insert `node` into `parent` at `index` (clamped; `usize::MAX` = top).
    pub fn insert(&mut self, parent: Option<NodeId>, index: usize, node: Node) -> Result<NodeId, DocError> {
        let id = node.id;
        let ch = self.children_mut(parent)?;
        let i = index.min(ch.len());
        ch.insert(i, Arc::new(node));
        Ok(id)
    }

    /// Remove a node and return it.
    pub fn remove(&mut self, id: NodeId) -> Result<Arc<Node>, DocError> {
        let parent = self.parent_of(id);
        let path = self.index_path(id).ok_or(DocError::NoNode(id))?;
        let idx = *path.last().ok_or(DocError::NoNode(id))?;
        let ch = self.children_mut(parent)?;
        if idx >= ch.len() {
            return Err(DocError::NoNode(id));
        }
        Ok(ch.remove(idx))
    }

    /// Move a node to `parent` at `index` (index measured after removal).
    pub fn move_node(&mut self, id: NodeId, parent: Option<NodeId>, index: usize) -> Result<(), DocError> {
        if let Some(p) = parent
            && self.ancestry(p).is_some_and(|a| a.contains(&id))
        {
            return Err(DocError::Invalid("cannot move an object into itself".into()));
        }
        let n = self.remove(id)?;
        let ch = self.children_mut(parent)?;
        let i = index.min(ch.len());
        ch.insert(i, n);
        Ok(())
    }

    /// Position of `id` within its parent: (parent, index, sibling count).
    pub fn position(&self, id: NodeId) -> Option<(Option<NodeId>, usize, usize)> {
        let parent = self.parent_of(id);
        let idx = *self.index_path(id)?.last()?;
        let n = self.children(parent)?.len();
        Some((parent, idx, n))
    }

    /// Remove the nodes `ids` names and return them in the order of `ids`, in one pass over the
    /// children of each parent. Each id is listed once and none lies inside another listed node, as
    /// in a list of top-level objects. For an id that is not in the tree, is listed twice or lies
    /// inside another listed node, it fails with `NoNode` and leaves the document as it was.
    pub fn remove_many(&mut self, ids: &[NodeId]) -> Result<Vec<Arc<Node>>, DocError> {
        let places = self.places(ids)?;
        let removed = self.take(places, ids.len())?;
        self.index.clear();
        Ok(removed)
    }

    /// Remove the nodes `ids` names, as [`Document::remove_many`] does, and put the node that `make`
    /// builds from them (in the order of `ids`) in the parent of the last of them, at that node's
    /// index less the number of listed nodes below it there. Returns the new node's id.
    pub fn wrap(&mut self, ids: &[NodeId], make: impl FnOnce(Vec<Arc<Node>>) -> Node) -> Result<NodeId, DocError> {
        let places = self.places(ids)?;
        let last = ids.len().checked_sub(1);
        let found = places.iter().find_map(|(up, kids)| kids.iter().find(|(_, k)| Some(*k) == last).map(|(i, _)| (up.clone(), *i)));
        let Some((up, i)) = found else { return Err(DocError::Invalid("nothing to wrap".into())) };
        // The parent and each node it is in move down by the listed nodes before them.
        let below = |parent: &[usize], index: usize| places.get(parent).map_or(0, |kids| kids.partition_point(|(j, _)| *j < index));
        let to: Vec<usize> = up.iter().enumerate().map(|(l, &j)| j.saturating_sub(below(&up[..l], j))).collect();
        let at = i.saturating_sub(below(&up, i));
        let node = make(self.take(places, ids.len())?);
        let id = node.id;
        let children = self.children_at(&to).ok_or_else(|| DocError::Invalid("the container of the listed objects is gone".into()))?;
        let at = at.min(children.len());
        children.insert(at, Arc::new(node));
        self.index.clear();
        Ok(id)
    }

    /// Put each list of nodes in `items` directly above its anchor (`replace`: in its place), in
    /// list order, in one pass over the children of each parent. Each anchor is listed once and none
    /// lies inside another. For an anchor that is not in the tree, is listed twice or lies inside
    /// another listed anchor, it fails with `NoNode` and leaves the document as it was.
    pub fn insert_beside(&mut self, items: Vec<(NodeId, Vec<Node>)>, replace: bool) -> Result<(), DocError> {
        let anchors: Vec<NodeId> = items.iter().map(|(id, _)| *id).collect();
        let places = self.places(&anchors)?;
        let mut lists: Vec<Vec<Node>> = items.into_iter().map(|(_, nodes)| nodes).collect();
        self.edit_children(places, |k, anchor, children| {
            if !replace {
                children.push(anchor);
            }
            let nodes = lists.get_mut(k).map(std::mem::take).unwrap_or_default();
            children.extend(nodes.into_iter().map(Arc::new));
        })?;
        self.index.clear();
        Ok(())
    }

    /// Where each of `ids` is ([`Places`]). Fails with `NoNode` for an id that is not in the tree,
    /// is listed twice or lies inside another listed node.
    fn places(&self, ids: &[NodeId]) -> Result<Places, DocError> {
        let paths = self.index_paths(ids.iter().copied());
        let mut places = Places::new();
        for (k, id) in ids.iter().enumerate() {
            let (&i, up) = paths.get(id).and_then(|p| p.split_last()).ok_or(DocError::NoNode(*id))?;
            match places.get_mut(up) {
                Some(kids) => kids.push((i, k)),
                None => {
                    places.insert(up.to_vec(), vec![(i, k)]);
                }
            }
        }
        for kids in places.values_mut() {
            kids.sort_unstable();
        }
        // An id listed twice has one path, so its parent lists the same child twice. A listed node
        // lies inside another when its parent's path, or a part of it from the top, is the path of a
        // listed node.
        let twice = places.values().flat_map(|kids| kids.windows(2)).find(|w| w[0].0 == w[1].0).map(|w| w[1].1);
        let listed = |path: &[usize]| {
            path.split_last().is_some_and(|(i, up)| places.get(up).is_some_and(|kids| kids.binary_search_by_key(i, |(j, _)| *j).is_ok()))
        };
        let nested = places.iter().find(|(up, _)| (1..=up.len()).any(|l| listed(&up[..l])));
        let inner = nested.and_then(|(_, kids)| kids.first()).map(|(_, k)| *k);
        if let Some(id) = twice.or(inner).and_then(|k| ids.get(k)) {
            return Err(DocError::NoNode(*id));
        }
        Ok(places)
    }

    /// Take the nodes `places` lists out of their parents and return them in the order of the list,
    /// which is `len` long.
    fn take(&mut self, places: Places, len: usize) -> Result<Vec<Arc<Node>>, DocError> {
        let mut taken = vec![None; len];
        self.edit_children(places, |k, node, _| {
            if let Some(t) = taken.get_mut(k) {
                *t = Some(node);
            }
        })?;
        Ok(taken.into_iter().flatten().collect())
    }

    /// Rebuild the children of each parent in `places` once, the parents later in tree order
    /// first. `each` gets each listed child, its place in the list and the parent's new children,
    /// onto which it pushes what takes the child's place.
    fn edit_children(&mut self, places: Places, mut each: impl FnMut(usize, Arc<Node>, &mut Vec<Arc<Node>>)) -> Result<(), DocError> {
        for (up, kids) in places.into_iter().rev() {
            let children = self.children_at(&up).ok_or_else(|| DocError::Invalid("a container of the listed objects is gone".into()))?;
            let old = std::mem::take(children);
            children.reserve(old.len());
            let mut kids = kids.into_iter().peekable();
            for (i, node) in old.into_iter().enumerate() {
                match kids.next_if(|(j, _)| *j == i) {
                    Some((_, k)) => each(k, node, children),
                    None => children.push(node),
                }
            }
        }
        Ok(())
    }

    /// The children of the node at index path `path`, or the top-level layers for an empty path.
    fn children_at(&mut self, path: &[usize]) -> Option<&mut Vec<Arc<Node>>> {
        match path {
            [] => Some(&mut self.layers),
            _ => self.node_mut_at(path)?.children_mut(),
        }
    }

    /// Default target layer: the topmost visible, unlocked layer.
    pub fn default_layer(&self) -> Option<NodeId> {
        self.layers.iter().rev().find(|l| l.visible && !l.locked).or(self.layers.last()).map(|l| l.id)
    }

    /// Add a new top-level layer above all others.
    pub fn add_layer(&mut self, name: Option<&str>) -> NodeId {
        let id = self.alloc_id();
        let n = self.layers.len();
        let name = name.map(str::to_string).unwrap_or_else(|| self.next_layer_name());
        self.layers.push(Arc::new(Node::layer(id, &name, LayerColor::Preset((n % LAYER_COLORS.len()) as u8))));
        id
    }

    /// The default name of a new layer or sublayer, "Layer N". Layers and sublayers share the
    /// numbering: it starts after the number of layers there are and skips any layer's name.
    pub fn next_layer_name(&self) -> String {
        let (mut count, mut taken) = (0, std::collections::HashSet::new());
        self.walk(|n| {
            if n.is_layer() {
                count += 1;
                taken.extend(n.name.as_deref());
            }
        });
        (count + 1..).map(|i| format!("Layer {i}")).find(|name| !taken.contains(name.as_str())).unwrap_or_default()
    }

    /// Visit every node depth first in paint order (bottom to top).
    pub fn walk<'a>(&'a self, mut f: impl FnMut(&'a Node)) {
        for l in &self.layers {
            l.walk(&mut f);
        }
    }

    pub fn node_count(&self) -> usize {
        self.layers.iter().map(|l| l.count()).sum()
    }

    /// Is the node (and every ancestor) visible and unlocked, and not on a locked artboard
    /// ([`Document::on_locked_artboard`])?
    pub fn is_editable(&self, id: NodeId) -> bool {
        self.is_editable_with(&self.locked_artboards(), id)
    }

    /// [`Document::is_editable`] with the locked artboards `locks` of this document, measured
    /// once for many ids.
    pub fn is_editable_with(&self, locks: &LockedArtboards, id: NodeId) -> bool {
        self.ancestry(id).is_some_and(|a| a.iter().all(|i| self.node(*i).is_some_and(|n| n.visible && !n.locked))) && !locks.holds_id(id)
    }

    /// [`Document::is_editable`] for many ids: the art on the locked artboards is measured once
    /// for all of them, a group once for all its members.
    pub fn editable(&self) -> impl Fn(NodeId) -> bool + '_ {
        let locks = self.locked_artboards();
        move |id| self.is_editable_with(&locks, id)
    }

    /// The locked artboards, for asking which art is on them ([`LockedArtboards::holds`]).
    pub fn locked_artboards(&self) -> LockedArtboards<'_> {
        self.locked_artboards_except(usize::MAX)
    }

    /// [`Document::locked_artboards`] leaving out artboard `index`.
    pub fn locked_artboards_except(&self, index: usize) -> LockedArtboards<'_> {
        // Art flush with an edge from outside can overlap it by a rounding error, as in
        // `art_bounds_on` (#1295).
        let rects: Vec<Rect> =
            self.artboards.iter().enumerate().filter(|(i, a)| a.locked && *i != index).map(|(_, a)| a.rect.inflate(-1e-6, -1e-6)).collect();
        // The objects of pattern editing mode's layer, a sublayer's objects included.
        fn objects(n: &Node, out: &mut std::collections::HashSet<NodeId>) {
            for c in n.children().into_iter().flatten() {
                if c.is_layer() {
                    objects(c, out);
                } else {
                    out.insert(c.id);
                }
            }
        }
        let mut pattern_art = std::collections::HashSet::new();
        if let Some(layer) = self.pattern_edit.as_ref().filter(|_| !rects.is_empty()).and_then(|e| self.layers.iter().find(|l| l.id == e.layer)) {
            objects(layer, &mut pattern_art);
        }
        LockedArtboards { rects, symbols: self.symbol_boxes(), new_art: self.new_art.get(), pattern_art, held: Default::default() }
    }

    /// Is `id` on a locked artboard ([`LockedArtboards::holds_id`])? A layer or sublayer is on none.
    pub fn on_locked_artboard(&self, id: NodeId) -> bool {
        self.locked_artboards().holds_id(id)
    }

    /// The object of a layer or sublayer that holds node `id`: `id` itself when it is one, else
    /// the group or other object around it right inside a layer. None for a layer or sublayer.
    pub fn layer_object(&self, id: NodeId) -> Option<&Node> {
        self.layer_object_at(&self.index_path(id)?)
    }

    /// [`Document::layer_object`] for the node at index path `path`.
    pub fn layer_object_at(&self, path: &[usize]) -> Option<&Node> {
        // Down the path to the first node that isn't a layer.
        let mut nodes = &self.layers;
        for &i in path {
            let n = nodes.get(i)?;
            if !n.is_layer() {
                return Some(n.as_ref());
            }
            nodes = n.children()?;
        }
        None
    }

    pub fn is_visible(&self, id: NodeId) -> bool {
        self.ancestry(id).is_some_and(|a| a.iter().all(|i| self.node(*i).is_some_and(|n| n.visible)))
    }

    /// The innermost layer or sublayer containing `id` (`id` itself when it is a layer).
    pub fn layer_containing(&self, id: NodeId) -> Option<NodeId> {
        let a = self.ancestry(id)?;
        a.iter().rev().copied().find(|i| self.node(*i).is_some_and(Node::is_layer))
    }

    /// Every art object the Selection tool can reach (Select All): the visible, unlocked objects
    /// of the visible, unlocked, non-template layers, looking through sublayers (bottom first),
    /// except those on a locked artboard.
    pub fn selectable_art(&self) -> Vec<NodeId> {
        let locks = self.locked_artboards();
        self.layers
            .iter()
            .filter(|l| l.visible && !l.locked && !l.is_template())
            .flat_map(|l| l.layer_art(true))
            .filter(|id| locks.is_empty() || !self.node(*id).is_some_and(|n| locks.holds(n)))
            .collect()
    }

    /// Colour of the innermost layer or sublayer containing `id` (selection highlight colour).
    pub fn layer_color(&self, id: NodeId) -> [u8; 3] {
        let l = self.layer_containing(id).and_then(|l| self.node(l));
        match l.map(|n| &n.kind) {
            Some(NodeKind::Layer { color, .. }) => color.rgb(),
            _ => LAYER_COLORS[0].1,
        }
    }
}
