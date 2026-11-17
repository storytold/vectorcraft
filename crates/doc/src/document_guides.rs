use super::document_ext::free_id;
use super::guides::Guide;
use super::limits::MAX_NEXT_ID;
use super::node::{held_guides, map_held, strip_held};
use super::{Affine, Arc, Document, Node, NodeId, NodeKind, PatternDef, Point, Rect, Selection, SymbolBoxes, Vec2, artboard_holds};
use std::collections::{BTreeMap, HashSet};

impl Document {
    /// Union of the geometric bounds of `ids` (their visual bounds when `visual`), with symbol
    /// instances measured by their art ([`Node::geometric_bounds_with`]).
    pub fn bounds_of(&self, ids: &[NodeId], visual: bool) -> Option<Rect> {
        self.bounds_of_with(ids, visual, &self.symbol_boxes())
    }

    /// [`bounds_of`](Document::bounds_of) measuring symbol instances from `symbols`, a cache shared
    /// across calls: measuring many instances one by one otherwise measures their art again for
    /// each.
    pub fn bounds_of_with(&self, ids: &[NodeId], visual: bool, symbols: &crate::SymbolBoxes) -> Option<Rect> {
        ids.iter().filter_map(|id| self.node(*id)).fold(None, |acc, n| {
            vectorcraft_geom::union_opt(acc, if visual { n.visual_bounds_with(symbols) } else { n.geometric_bounds_with(symbols) })
        })
    }

    /// Visual bounds of all art, with symbol instances measured by their art.
    pub fn art_bounds(&self) -> Option<Rect> {
        let symbols = self.symbol_boxes();
        self.layers.iter().fold(None, |acc, l| vectorcraft_geom::union_opt(acc, l.visual_bounds_with(&symbols)))
    }

    /// Artboard index containing point `p` (topmost = last), its edges included
    /// ([`Document::artboard_holding`]).
    pub fn artboard_at(&self, p: Point) -> Option<usize> {
        self.artboard_holding(p, true)
    }

    /// Index of the artboard that holds point `p`: the first (the last when `topmost`) that holds it
    /// inside or on its left or top edge, else the first (the last) that holds it by
    /// [`artboard_holds`]: on its right or bottom edge, or within a millionth of a point outside an
    /// edge. On an edge two artboards share, the artboard to the right of it or below it holds the
    /// point, except at the end of the edge that lies on that artboard's bottom or right edge, such
    /// as the point where both artboards' bottom edges meet; there the first (the last) holds it.
    pub fn artboard_holding(&self, p: Point, topmost: bool) -> Option<usize> {
        let pick = |on: &dyn Fn(Rect) -> bool| {
            let on = |i: &usize| self.artboards.get(*i).is_some_and(|a| on(a.rect));
            let mut all = 0..self.artboards.len();
            if topmost { all.rfind(on) } else { all.find(on) }
        };
        pick(&|r| r.contains(p)).or_else(|| pick(&|r| artboard_holds(r, p)))
    }

    /// The art that moves with an artboard at `rect`: top-level objects (children of layers and
    /// sublayers) lying entirely inside it, with symbol instances measured by their art. Locked and
    /// hidden objects and layers stay put unless `locked_and_hidden` (Selection & Anchor Display ›
    /// Move Locked and Hidden Artwork with Artboard).
    pub fn art_on_artboard(&self, rect: Rect, locked_and_hidden: bool) -> Vec<NodeId> {
        // Art drawn flush with the artboard can end up a rounding error outside it (a background
        // 3e-14 pt past the top edge, #1295): a millionth of a point is far below anything visible.
        let rect = rect.inflate(1e-6, 1e-6);
        fn collect(n: &Node, s: &SymbolBoxes, rect: Rect, all: bool, out: &mut Vec<NodeId>) {
            for c in n.children().into_iter().flatten().filter(|c| c.rides_with_artboard(all)) {
                if c.is_layer() {
                    collect(c, s, rect, all, out);
                } else if let Some(b) = c.geometric_bounds_with(s)
                    // Inside it, edges included: art flush with the right or bottom edge is on it too
                    // (`Rect::contains` leaves those out, #890).
                    && b.x0 >= rect.x0
                    && b.y0 >= rect.y0
                    && b.x1 <= rect.x1
                    && b.y1 <= rect.y1
                {
                    out.push(c.id);
                }
            }
        }
        let symbols = self.symbol_boxes();
        let mut art = vec![];
        for l in self.layers.iter().filter(|l| l.rides_with_artboard(locked_and_hidden)) {
            collect(l, &symbols, rect, locked_and_hidden, &mut art);
        }
        art
    }

    /// Does any artwork lie on `rect` (an artboard)? Artwork is any object under the layers,
    /// hidden and locked ones and those on template layers included, whose visual bounds (symbol
    /// instances measured by their art) meet `rect`; touching its edge counts. A group (not a clip
    /// group) counts by its members, so one whose members sit on two other artboards leaves the one
    /// between them empty. A clip group or clipping layer counts by its members inside its clip (and
    /// by its clipping path where that paints). Guides (paths made into guides too) aren't artwork
    /// (Delete Empty Artboards).
    pub fn artboard_has_art(&self, rect: Rect) -> bool {
        self.art_meets(&self.symbol_boxes(), rect, true)
    }

    /// Is there a rect in `rects` (artboards) with no artwork on it, by [`Document::artboard_has_art`]?
    /// The symbols are measured once for all of them. A first pass rules art out by a quick box
    /// (paths and compound paths: their control points grown by the farthest their strokes reach),
    /// so a rect away from every path is found without measuring along any; only when every rect
    /// may hold art is each one measured.
    pub fn any_artboard_without_art(&self, rects: impl IntoIterator<Item = Rect>) -> bool {
        let rects: Vec<Rect> = rects.into_iter().collect();
        let symbols = self.symbol_boxes();
        rects.iter().any(|r| !self.art_meets(&symbols, *r, false)) || rects.iter().any(|r| !self.art_meets(&symbols, *r, true))
    }

    /// [`Document::artboard_has_art`] of `rect` when `exact`. Otherwise whether art may lie on it:
    /// paths and compound paths count when their quick box meets it, so a false answer is exact and
    /// a true one may not be.
    fn art_meets(&self, symbols: &SymbolBoxes, rect: Rect, exact: bool) -> bool {
        // As in `art_on_artboard`: a millionth of a point covers rounding at the edges.
        let rect = rect.inflate(1e-6, 1e-6);
        fn meets(b: Option<Rect>, r: Rect) -> bool {
            b.is_some_and(|b| b.x0 <= r.x1 && r.x0 <= b.x1 && b.y0 <= r.y1 && r.y0 <= b.y1)
        }
        /// A box that holds the visual bounds of a path, or of a compound path of paths, found
        /// without measuring along its shape: its control points' box grown by the farthest its
        /// strokes reach ([`crate::Appearance::outset`], as in [`Node::reach_bounds`]), with a
        /// margin for rounding. None for other objects, and when the reach is not finite.
        fn quick_box(c: &Node) -> Option<Rect> {
            let b = match &c.kind {
                NodeKind::Path { path, .. } => path.control_bounds()?,
                NodeKind::Compound { children, .. } => children.iter().try_fold(None, |acc, m| {
                    let NodeKind::Path { path, .. } = &m.kind else { return None };
                    Some(vectorcraft_geom::union_opt(acc, path.control_bounds()))
                })??,
                _ => return None,
            };
            let o = c.appearance.outset();
            let o = o * (1.0 + 1e-9) + 1e-6;
            o.is_finite().then(|| b.inflate(o, o))
        }
        /// Does `c`'s visual box meet `rect`? Without `exact`, a quick box that meets it answers yes.
        fn art_box_meets(c: &Node, s: &SymbolBoxes, rect: Rect, exact: bool) -> bool {
            match quick_box(c).map(|b| meets(Some(b), rect)) {
                Some(false) => false,
                Some(true) if !exact => true,
                _ => meets(c.visual_bounds_with(s), rect),
            }
        }
        fn has(c: &Node, s: &SymbolBoxes, rect: Rect, exact: bool) -> bool {
            match &c.kind {
                NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                    let Some((clip, rest)) = children.split_first() else { return false };
                    let paint = clip.clip_paint();
                    if (paint.fill.is_some() || paint.stroke.is_some()) && art_box_meets(clip, s, rect, exact) {
                        return true;
                    }
                    if !exact {
                        // The part of `rect` inside the clip's box lies inside `rect`.
                        return rest.iter().any(|m| has(m, s, rect, false));
                    }
                    // The members show only inside the clip's box.
                    let Some(b) = c.visual_bounds_with(s).filter(|b| meets(Some(*b), rect)) else { return false };
                    let inside = Rect::new(b.x0.max(rect.x0), b.y0.max(rect.y0), b.x1.min(rect.x1), b.y1.min(rect.y1));
                    rest.iter().any(|m| has(m, s, inside, true))
                }
                NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => children.iter().any(|m| has(m, s, rect, exact)),
                NodeKind::Path { guide: true, .. } | NodeKind::RulerGuide { .. } => false,
                _ => art_box_meets(c, s, rect, exact),
            }
        }
        self.layers.iter().any(|l| has(l, symbols, rect, exact))
    }

    /// The art that moves, scales or is cut with artboard `index`: [`Document::art_on_artboard`]
    /// without the objects on another locked artboard, which stay unless `locked_and_hidden`, as
    /// locked art does.
    pub fn art_moving_with(&self, index: usize, locked_and_hidden: bool) -> Vec<NodeId> {
        let Some(rect) = self.artboards.get(index).map(|a| a.rect) else { return vec![] };
        let mut art = self.art_on_artboard(rect, locked_and_hidden);
        let locks = self.locked_artboards_except(index);
        if !locked_and_hidden && !locks.is_empty() {
            art.retain(|id| !self.node(*id).is_some_and(|n| locks.holds(n)));
        }
        art
    }

    /// Put the ruler guide nodes a file holds where ruler guides go: directly in a layer or
    /// sublayer, above its art. A ruler guide below art in a layer moves up to the layer's other
    /// guides, keeping their order. A ruler guide anywhere else (at the top level, in a group or
    /// another object, in opacity-mask art, in a symbol, a graph design or a pattern) is removed.
    /// Returns how many were removed.
    pub fn tidy_ruler_guides(&mut self) -> usize {
        let mut gone = self.layers.len();
        self.layers.retain(|l| !l.is_ruler_guide());
        gone -= self.layers.len();
        for l in &mut self.layers {
            if let Some(found) = untidy(l, &Node::is_ruler_guide) {
                gone += tidy_layer(Arc::make_mut(l), found);
            }
        }
        let others = self
            .symbols
            .iter_mut()
            .map(|s| &mut s.art)
            .chain(self.graph_designs.iter_mut().map(|g| &mut g.art))
            .chain(self.patterns.iter_mut().flat_map(|p| p.art.iter_mut()))
            .chain(self.pattern_edit.iter_mut().filter_map(|pe| pe.original.as_mut()).flat_map(|p| p.art.iter_mut()));
        for root in others {
            if root.is_ruler_guide() {
                *root = Arc::new(Node::group(root.id, vec![]));
                gone += 1;
            } else if let Some(found) = guides_below(root, &Node::is_ruler_guide) {
                gone += drop_guides_below(Arc::make_mut(root), found);
            }
        }
        gone
    }

    /// Make the saved ruler guides (read from a file: [`Document::guides_to_saved`]'s list) into
    /// [`NodeKind::RulerGuide`] nodes, each at the top of its layer, in list order. A guide on a
    /// layer or sublayer the document has goes on it; one on no layer, or on a layer that is gone,
    /// goes on the top visible, unlocked layer ([`Self::default_layer`]), or on a new "Layer 1"
    /// when there is none. A guide keeps its saved id when no other node, slice or guide has it,
    /// and its name, lock and visibility. The saved list is emptied, so running this again does
    /// nothing. Returns how many guides became nodes.
    pub fn guides_into_nodes(&mut self) -> usize {
        let saved = std::mem::take(&mut self.saved_guides);
        if saved.is_empty() {
            return 0;
        }
        let mut used = HashSet::new();
        self.walk(|n| {
            used.insert(n.id);
        });
        used.extend(self.slices.iter().map(|s| s.id));
        used.extend(self.assets.iter().map(|a| NodeId(a.id)));
        let kept: Vec<Option<NodeId>> = saved.iter().map(|g| g.id.filter(|id| id.0 > 0 && id.0 < MAX_NEXT_ID && used.insert(*id))).collect();
        if let Some(max) = kept.iter().flatten().map(|id| id.0).max() {
            self.raise_next_id(max.saturating_add(1));
        }
        let mut home = None;
        // The guides of each layer, in list order.
        let mut by_layer: BTreeMap<NodeId, Vec<Arc<Node>>> = BTreeMap::new();
        let mut order = vec![];
        for (g, id) in saved.iter().zip(kept) {
            let layer = match g.layer.filter(|l| self.node(*l).is_some_and(Node::is_layer)) {
                Some(l) => l,
                None => match home {
                    Some(h) => h,
                    None => {
                        let h = self.default_layer().unwrap_or_else(|| self.add_layer(None));
                        home = Some(h);
                        h
                    }
                },
            };
            let id = id.unwrap_or_else(|| self.alloc_id());
            let mut n = Node::new(id, NodeKind::RulerGuide { vertical: g.vertical, pos: g.pos, artboard: g.artboard });
            n.name = g.name.clone();
            n.locked = g.locked;
            n.visible = !g.hidden;
            if !by_layer.contains_key(&layer) {
                order.push(layer);
            }
            by_layer.entry(layer).or_default().push(Arc::new(n));
        }
        for layer in order {
            let guides = by_layer.remove(&layer).unwrap_or_default();
            if let Ok(ch) = self.children_mut(Some(layer)) {
                ch.extend(guides);
            }
        }
        saved.len()
    }

    /// Take the ruler guide nodes out of the layers and put them in the saved list, by ascending
    /// id, each naming its layer, after any entries not yet made into nodes: the form a file keeps
    /// them in, readable by versions from before ruler guides were nodes. For saving a copy of
    /// the document; [`Self::guides_into_nodes`] reverses it.
    pub fn guides_to_saved(&mut self) {
        let guides = self.ruler_guides();
        if guides.is_empty() {
            return;
        }
        for l in &mut self.layers {
            if let Some(held) = held_guides(l, &Node::is_ruler_guide) {
                strip_held(Arc::make_mut(l), held);
            }
        }
        self.saved_guides.extend(guides);
    }

    /// Every ruler guide, by ascending id, with the layer or sublayer it is on, its id, name and
    /// its own lock and visibility. A guide's place in this list is its index in the guide
    /// commands.
    pub fn ruler_guides(&self) -> Vec<Guide> {
        if let Some(listed) = self.guide_cache.get(&self.layers) {
            return listed.as_ref().clone();
        }
        fn visit(layer: &Node, out: &mut Vec<Guide>) {
            for c in layer.children().into_iter().flatten() {
                if let Some(g) = guide_of(c, layer.id) {
                    out.push(g);
                } else if c.is_layer() {
                    visit(c, out);
                }
            }
        }
        let mut out = vec![];
        for l in &self.layers {
            visit(l, &mut out);
        }
        out.sort_by_key(|g| g.id);
        self.guide_cache.set(&self.layers, &out);
        out
    }

    /// Ruler guide `id`, as [`Self::ruler_guides`] lists it. None when `id` is not a ruler guide
    /// on a layer.
    pub fn ruler_guide(&self, id: NodeId) -> Option<Guide> {
        let layer = self.parent_of(id)?;
        guide_of(self.node(id)?, layer)
    }

    /// Change ruler guide `id` with `f` (its line, its artboard; the layer, id, name, lock and
    /// visibility fields are not read back). Returns false when `id` is not a ruler guide.
    pub fn edit_ruler_guide(&mut self, id: NodeId, f: impl FnOnce(&mut Guide)) -> bool {
        let Some(mut g) = self.ruler_guide(id) else { return false };
        f(&mut g);
        let Some(n) = self.node_mut(id) else { return false };
        n.kind = NodeKind::RulerGuide { vertical: g.vertical, pos: g.pos, artboard: g.artboard };
        true
    }

    /// Add ruler guide `g` as a node with a new id at the top of its layer: `g.layer` when the
    /// document has that layer or sublayer, else the top visible, unlocked layer
    /// ([`Self::default_layer`]), or a new "Layer 1" when there is none. Its name, lock and
    /// visibility come from `g`. Returns the new id.
    pub fn add_ruler_guide(&mut self, g: &Guide) -> NodeId {
        let layer = match g.layer.filter(|l| self.node(*l).is_some_and(Node::is_layer)) {
            Some(l) => l,
            None => self.default_layer().unwrap_or_else(|| self.add_layer(None)),
        };
        let id = self.alloc_id();
        let mut n = Node::new(id, NodeKind::RulerGuide { vertical: g.vertical, pos: g.pos, artboard: g.artboard });
        n.name = g.name.clone();
        n.locked = g.locked;
        n.visible = !g.hidden;
        if let Ok(ch) = self.children_mut(Some(layer)) {
            ch.push(Arc::new(n));
        }
        id
    }

    /// Remove node `id`, the temporary layer of opacity-mask, symbol or pattern editing, keeping the
    /// ruler guides on it and on its sublayers: they go to the top of the top visible, unlocked
    /// layer left ([`Self::default_layer`]), or of a new "Layer 1" when no layer is left, where a
    /// saved guide whose layer is gone opens. Returns the removed node, without the guides.
    pub fn remove_edit_layer(&mut self, id: NodeId) -> Result<Arc<Node>, crate::DocError> {
        let mut layer = self.remove(id)?;
        let Some(held) = held_guides(&layer, &Node::is_ruler_guide) else { return Ok(layer) };
        let mut guides = vec![];
        layer.walk(&mut |c| {
            if c.is_ruler_guide() {
                guides.push(Arc::new(c.clone()));
            }
        });
        strip_held(Arc::make_mut(&mut layer), held);
        let home = self.default_layer().unwrap_or_else(|| self.add_layer(None));
        self.children_mut(Some(home))?.extend(guides);
        Ok(layer)
    }

    /// Where ruler guide `g` runs along its line (the y range of a vertical guide): across its
    /// artboard for an artboard guide, None (the whole canvas) for a canvas guide or one whose
    /// artboard is gone.
    pub fn guide_span(&self, g: &Guide) -> Option<(f64, f64)> {
        let r = self.artboards.iter().find(|a| Some(a.id) == g.artboard)?.rect;
        Some(if g.vertical { (r.y0, r.y1) } else { (r.x0, r.x1) })
    }

    /// The ruler guide node `g` names, if this document has it.
    fn guide_node(&self, g: &Guide) -> Option<NodeId> {
        g.id.filter(|id| self.node(*id).is_some_and(Node::is_ruler_guide))
    }

    /// The layer (or sublayer) ruler guide `g` is on, if it is on one this document has.
    fn guide_layer(&self, g: &Guide) -> Option<NodeId> {
        g.layer.filter(|l| self.node(*l).is_some_and(Node::is_layer))
    }

    /// Is ruler guide `g` shown: its node, and the layers it is on, visible? A guide that is not a
    /// node of this document shows unless hidden itself or on a hidden layer.
    pub fn guide_shown(&self, g: &Guide) -> bool {
        match self.guide_node(g) {
            Some(id) => self.is_visible(id),
            None => !g.hidden && self.guide_layer(g).is_none_or(|l| self.is_visible(l)),
        }
    }

    /// Can ruler guide `g` be picked, moved and deleted: shown, and neither it nor its layers
    /// locked?
    pub fn guide_editable(&self, g: &Guide) -> bool {
        match self.guide_node(g) {
            Some(id) => self.is_editable(id),
            None => !g.hidden && !g.locked && self.guide_layer(g).is_none_or(|l| self.is_editable(l)),
        }
    }

    /// The ruler guides directly on layer `layer` (not its sublayers'), by ascending id.
    pub fn guides_on(&self, layer: NodeId) -> Vec<Guide> {
        let mut out: Vec<Guide> = self.children(Some(layer)).into_iter().flatten().filter_map(|c| guide_of(c, layer)).collect();
        out.sort_by_key(|g| g.id);
        out
    }

    /// Does ruler guide `g` run past `p` (up to `tol` beyond its ends)?
    pub fn guide_passes(&self, g: &Guide, p: Point, tol: f64) -> bool {
        let along = if g.vertical { p.y } else { p.x };
        self.guide_span(g).is_none_or(|(a, b)| along >= a - tol && along <= b + tol)
    }

    /// Keep the ruler guides `keep` accepts, deleting the others; `sel` forgets the deleted ones
    /// (the selected art stays selected). Returns how many went.
    pub fn retain_guides(&mut self, sel: &mut Selection, keep: impl Fn(&Guide) -> bool) -> usize {
        let gone: HashSet<NodeId> = self.ruler_guides().into_iter().filter(|g| !keep(g)).filter_map(|g| g.id).collect();
        if gone.is_empty() {
            return 0;
        }
        let pick = |n: &Node| n.is_ruler_guide() && gone.contains(&n.id);
        for l in &mut self.layers {
            if let Some(held) = held_guides(l, &pick) {
                strip_held(Arc::make_mut(l), held);
            }
        }
        sel.guides.retain(|id| !gone.contains(id));
        if sel.key_guide.is_some_and(|k| gone.contains(&k)) {
            sel.key_guide = None;
        }
        gone.len()
    }

    /// Change the line of every ruler guide on artboard `id` with `f` (whether it is vertical, its
    /// position).
    fn edit_artboard_guides(&mut self, id: u32, f: &mut impl FnMut(bool, &mut f64)) {
        let pick = |n: &Node| matches!(n.kind, NodeKind::RulerGuide { artboard: Some(a), .. } if a == id);
        for l in &mut self.layers {
            if let Some(held) = held_guides(l, &pick) {
                map_held(Arc::make_mut(l), held, f);
            }
        }
    }

    /// Move the guides of artboard `id` by `d` (along with their artboard).
    pub fn move_artboard_guides(&mut self, id: u32, d: Vec2) {
        self.edit_artboard_guides(id, &mut |vertical, pos| *pos += if vertical { d.x } else { d.y });
    }

    /// Map the guides of artboard `id` through `xf`, an axis-aligned scale and move (Scale Artwork
    /// with Artboard).
    pub fn map_artboard_guides(&mut self, id: u32, xf: Affine) {
        self.edit_artboard_guides(id, &mut |vertical, pos| {
            *pos = if vertical { (xf * Point::new(*pos, 0.0)).x } else { (xf * Point::new(0.0, *pos)).y };
        });
    }

    /// Copy the guides of artboard `from` onto artboard `to`, `d` away (with a copy of their
    /// artboard), each on its guide's layer with a new id.
    pub fn copy_artboard_guides(&mut self, from: u32, to: u32, d: Vec2) {
        let copies: Vec<Guide> =
            self.ruler_guides().into_iter().filter(|g| g.artboard == Some(from)).map(|g| Guide { artboard: Some(to), ..g.moved(d) }).collect();
        for g in &copies {
            self.add_ruler_guide(g);
        }
    }

    /// An id for a new artboard: one no artboard has, above the ids given out before and the ids of
    /// the artboards deleted from the document ([`Document::retired_artboard_id`]). The id counts
    /// as given out, so an artboard made after an undo does not take the id of one the undo took
    /// away.
    pub fn next_artboard_id(&self) -> u32 {
        let id = free_id(self.artboards.iter().map(|a| a.id).chain([self.retired_artboard_id.get()]));
        self.retired_artboard_id.raise(id);
        id
    }

    /// [`Document::next_artboard_id`] when `top` is the highest id the artboards have (`None`: there
    /// are none). It reads no artboard unless that id is `u32::MAX`, so a caller that adds many
    /// artboards and keeps `top` up to date gets each id in constant time.
    pub fn next_artboard_id_above(&self, top: Option<u32>) -> u32 {
        match top.unwrap_or(0).max(self.retired_artboard_id.get()).checked_add(1) {
            Some(id) => {
                self.retired_artboard_id.raise(id);
                id
            }
            None => self.next_artboard_id(),
        }
    }

    /// Record that the artboard with `id` was deleted: no later artboard takes its id.
    pub fn retire_artboard_id(&self, id: u32) {
        self.retired_artboard_id.raise(id);
    }

    pub fn pattern(&self, name: &str) -> Option<&PatternDef> {
        self.patterns.iter().find(|p| p.name == name)
    }

    pub fn pattern_mut(&mut self, name: &str) -> Option<&mut PatternDef> {
        self.patterns.iter_mut().find(|p| p.name == name)
    }

    /// Deep-clone `node` with fresh ids for it and all descendants, its images' own pixels taken
    /// in ([`Self::adopt_pixels`]).
    pub fn reid(&mut self, node: &Node) -> Node {
        let mut n = node.clone();
        n.id = self.alloc_id();
        if let NodeKind::Image(im) = &mut n.kind {
            self.adopt_image_pixels(im);
        }
        // A copied annotation must never bind its source ids to unrelated art in a different
        // document. Keep the measured geometry; new measurements can be linked after pasting.
        if let Some(e) = n.extras_if_any()
            && let Some(dimension) = &mut e.dimension
        {
            dimension.source = None;
        }
        // A copy of a threaded frame is in no thread, so its first paragraph starts in it and its
        // layout caches are dropped. A caller that keeps the copy as an object fills them again.
        if let NodeKind::Text(t) = &mut n.kind
            && t.first_para_continues
        {
            t.first_para_continues = false;
            t.cached_bounds = None;
            t.cached_baselines.clear();
            t.cached_ink = None;
        }
        // Opacity-mask art is stored art too: the copy's gets ids of its own.
        if let Some(art) = n.extras().mask.as_ref().map(|m| Arc::clone(&m.art)) {
            let art = Arc::new(self.reid(&art));
            if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
                m.art = art;
            }
        }
        let on_path = n.extras().objects_on_path.is_some();
        let mut renamed = std::collections::HashMap::new();
        if let Some(ch) = n.children_mut() {
            let old: Vec<Arc<Node>> = std::mem::take(ch);
            *ch = old.iter().map(|c| Arc::new(self.reid(c))).collect();
            if on_path {
                renamed.extend(old.iter().zip(ch.iter()).map(|(o, c)| (o.id, c.id)));
            }
        }
        // An Objects on Path group names its path and its objects by id: the copy names the copies.
        if let Some(e) = n.extras_if_any()
            && let Some(on_path) = &mut e.objects_on_path
        {
            on_path.rename(&renamed);
        }
        n
    }
}

/// What a walk picked in a node: its children by index in order, each with what was picked
/// inside it, and what was picked in its opacity-mask art. Empty for a node picked itself.
#[derive(Debug, Default)]
struct Picked {
    children: Vec<(usize, Picked)>,
    mask: Option<Box<Picked>>,
}

impl Picked {
    fn is_empty(&self) -> bool {
        self.children.is_empty() && self.mask.is_none()
    }

    /// `self`, or `None` when nothing was picked.
    fn some(self) -> Option<Self> {
        (!self.is_empty()).then_some(self)
    }
}

/// `n` picked itself when it is a ruler guide by `is_guide`, else the ruler guides below it
/// ([`guides_below`]).
fn guide_or_below(n: &Node, is_guide: &impl Fn(&Node) -> bool) -> Option<Picked> {
    if is_guide(n) { Some(Picked::default()) } else { guides_below(n, is_guide) }
}

/// The ruler guides below `n`, by `is_guide`: those among its members and its opacity-mask art
/// (each with nothing inside), and the members and mask art holding one further down (with what
/// they hold), or `None` when there is none. `is_guide` looks at each node below `n` once, and at
/// nothing inside a ruler guide.
fn guides_below(n: &Node, is_guide: &impl Fn(&Node) -> bool) -> Option<Picked> {
    let mask = n.extras().mask.as_deref().and_then(|m| guide_or_below(&m.art, is_guide)).map(Box::new);
    let children = n.children().into_iter().flatten().enumerate().filter_map(|(i, c)| guide_or_below(c, is_guide).map(|p| (i, p))).collect();
    Picked { children, mask }.some()
}

/// Remove the ruler guides below `n` that [`guides_below`] `found`, in its members and its
/// opacity-mask art (`n` itself stays). Mask art that is a ruler guide becomes an empty group.
/// Only the nodes holding one are copied, and only those are visited. Returns how many went.
fn drop_guides_below(n: &mut Node, found: Picked) -> usize {
    let mut gone = drop_mask_guides(n, found.mask);
    if let Some(ch) = n.children_mut() {
        let mut found = found.children.into_iter().peekable();
        let mut next = 0;
        ch.retain_mut(|c| {
            let at = next;
            next += 1;
            let Some((_, inner)) = found.next_if(|(i, _)| *i == at) else { return true };
            if inner.is_empty() {
                gone += 1;
                return false;
            }
            gone += drop_guides_below(Arc::make_mut(c), inner);
            true
        });
    }
    gone
}

/// Remove the ruler guides `found` in the opacity-mask art of `n` ([`guides_below`]'s mask
/// part). Mask art that is a ruler guide becomes an empty group. Returns how many went.
fn drop_mask_guides(n: &mut Node, found: Option<Box<Picked>>) -> usize {
    let Some(found) = found else { return 0 };
    let Some(mask) = n.extras_if_any().and_then(|e| e.mask.as_deref_mut()) else { return 0 };
    if found.is_empty() {
        mask.art = Arc::new(Node::group(mask.art.id, vec![]));
        1
    } else {
        drop_guides_below(Arc::make_mut(&mut mask.art), *found)
    }
}

/// The ruler guides, by `is_guide`, that layer `l` holds out of place, or `None` when it holds
/// none: those below art (each with nothing inside), those in its mask art and in its objects
/// ([`guides_below`]), and its sublayers holding one out of place (with what they hold).
/// `is_guide` looks at each node once; the guides at the end of a layer are in place and are
/// not looked at.
fn untidy(l: &Node, is_guide: &impl Fn(&Node) -> bool) -> Option<Picked> {
    let ch = l.children()?;
    let mask = l.extras().mask.as_deref().and_then(|m| guide_or_below(&m.art, is_guide)).map(Box::new);
    let children = ch
        .iter()
        .enumerate()
        .take(Node::guide_run_start(ch))
        .filter_map(|(i, c)| {
            let found = if is_guide(c) {
                Some(Picked::default())
            } else if c.is_layer() {
                untidy(c, is_guide)
            } else {
                guides_below(c, is_guide)
            };
            found.map(|p| (i, p))
        })
        .collect();
    Picked { children, mask }.some()
}

/// [`Document::tidy_ruler_guides`] for layer `l`, whose guides out of place [`untidy`] `found`.
/// Only the nodes holding one are copied, and only those are visited. Returns how many guides
/// were removed.
fn tidy_layer(l: &mut Node, found: Picked) -> usize {
    let mut gone = drop_mask_guides(l, found.mask);
    let Some(ch) = l.children_mut() else { return gone };
    let mut below_art = false;
    for (i, inner) in found.children {
        let Some(c) = ch.get_mut(i) else { continue };
        if inner.is_empty() {
            below_art = true;
        } else if c.is_layer() {
            gone += tidy_layer(Arc::make_mut(c), inner);
        } else {
            gone += drop_guides_below(Arc::make_mut(c), inner);
        }
    }
    if below_art {
        let (guides, art): (Vec<_>, Vec<_>) = std::mem::take(ch).into_iter().partition(|c| c.is_ruler_guide());
        *ch = art;
        ch.extend(guides);
    }
    gone
}

/// Node `n`, on layer `layer`, as a [`Guide`] value, if it is a ruler guide.
fn guide_of(n: &Node, layer: NodeId) -> Option<Guide> {
    let NodeKind::RulerGuide { vertical, pos, artboard } = n.kind else { return None };
    Some(Guide { vertical, pos, artboard, layer: Some(layer), id: Some(n.id), name: n.name.clone(), locked: n.locked, hidden: !n.visible })
}

/// The list [`Document::ruler_guides`] last made, with weak pointers to the top-level layers it was
/// made from. While a weak pointer to a layer's allocation is held, nothing else can be at that
/// address, and the layer can't be changed in place (`Arc::get_mut` refuses it and `Arc::make_mut`
/// moves it), so any edit inside a layer, and any layer added, removed or replaced, gives a list of
/// layers the cache does not match. Clones of a document share the cached list.
#[derive(Default)]
pub(crate) struct GuideCache(std::sync::RwLock<Option<Listed>>);

/// The top-level layers a guide list was made from, and the list.
type Listed = (Vec<std::sync::Weak<Node>>, Arc<Vec<Guide>>);

impl GuideCache {
    /// The cached list, when it was made from `layers`.
    fn get(&self, layers: &[Arc<Node>]) -> Option<Arc<Vec<Guide>>> {
        let g = self.0.read().ok()?;
        let (from, listed) = g.as_ref()?;
        let same = from.len() == layers.len() && from.iter().zip(layers).all(|(w, l)| std::ptr::eq(w.as_ptr(), Arc::as_ptr(l)));
        same.then(|| listed.clone())
    }

    /// Keep `listed`, made from `layers`.
    fn set(&self, layers: &[Arc<Node>], listed: &[Guide]) {
        if let Ok(mut g) = self.0.write() {
            *g = Some((layers.iter().map(Arc::downgrade).collect(), Arc::new(listed.to_vec())));
        }
    }
}

impl Clone for GuideCache {
    fn clone(&self) -> Self {
        GuideCache(std::sync::RwLock::new(self.0.read().ok().and_then(|g| g.clone())))
    }
}

/// A cache, not document content: two documents are equal whatever their caches hold.
impl PartialEq for GuideCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for GuideCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GuideCache")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Appearance, OpacityMask, Symbol};
    use std::cell::Cell;
    use vectorcraft_geom::shapes;

    /// The sublayers in [`nested`]'s chain: with the top-level layer around them and the objects in
    /// the innermost, the deepest a document goes ([`crate::MAX_DEPTH`]).
    const CHAIN: u64 = 30;

    fn rect(id: u64) -> Arc<Node> {
        Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art()))
    }

    fn guide(id: u64, artboard: Option<u32>) -> Arc<Node> {
        Arc::new(Node::new(NodeId(id), NodeKind::RulerGuide { vertical: false, pos: id as f64, artboard }))
    }

    fn group(id: u64, children: Vec<Arc<Node>>) -> Arc<Node> {
        Arc::new(Node::group(NodeId(id), children))
    }

    /// `n` with `art` as its opacity-mask art.
    fn masked(n: Arc<Node>, art: Arc<Node>) -> Arc<Node> {
        let mut n = (*n).clone();
        n.extras_mut().mask = Some(Box::new(OpacityMask { art, clip: true, invert: false, disabled: false, linked: true }));
        Arc::new(n)
    }

    /// A sublayer like a new document's first layer, holding `children`.
    fn sublayer(id: u64, children: Vec<Arc<Node>>) -> Arc<Node> {
        let mut l = (*Document::new(1.0, 1.0).layers[0]).clone();
        l.id = NodeId(id);
        if let Some(ch) = l.children_mut() {
            *ch = children;
        }
        Arc::new(l)
    }

    /// A document whose first layer holds `children`.
    fn doc(children: Vec<Arc<Node>>) -> Document {
        let mut d = Document::new(100.0, 100.0);
        if let Some(ch) = d.layers.first_mut().and_then(|l| Arc::make_mut(l).children_mut()) {
            *ch = children;
        }
        d
    }

    /// The ids of the nodes in the layers, in tree order.
    fn ids(d: &Document) -> Vec<u64> {
        fn walk(n: &Node, out: &mut Vec<u64>) {
            out.push(n.id.0);
            for c in n.children().into_iter().flatten() {
                walk(c, out);
            }
        }
        let mut out = vec![];
        for l in &d.layers {
            walk(l, &mut out);
        }
        out
    }

    /// [`CHAIN`] sublayers, each inside the one before and holding an object, a canvas guide and a
    /// guide of artboard 7 (the innermost: object 150, guides 200 and 201). Beside the chain, a
    /// sublayer with a canvas guide only. Returns the document and that sublayer.
    fn nested() -> (Document, Arc<Node>) {
        let mut chain = sublayer(100, vec![rect(150), guide(200, None), guide(201, Some(7))]);
        for depth in 1..CHAIN {
            chain = sublayer(100 + depth, vec![rect(600 + depth), chain, guide(300 + depth * 2, None), guide(301 + depth * 2, Some(7))]);
        }
        let still = sublayer(500, vec![rect(502), guide(501, None)]);
        (doc(vec![chain, Arc::clone(&still)]), still)
    }

    /// How many children the layers of [`nested`] hold: the first layer's 2, the untouched
    /// sublayer's 2, the innermost sublayer's 3 and 4 in each sublayer around it.
    const NESTED_CHILDREN: usize = 2 + 2 + 3 + (CHAIN as usize - 1) * 4;

    fn on_artboard_7(n: &Node) -> bool {
        matches!(n.kind, NodeKind::RulerGuide { artboard: Some(7), .. })
    }

    #[test]
    fn removing_guides_looks_at_each_child_of_nested_layers_once() {
        let (mut d, still) = nested();
        assert!(!d.clone().limit_depth(), "the chain is as deep as a document goes");
        let mut deeper = doc(vec![sublayer(999, d.layers[0].children().cloned().unwrap_or_default())]);
        assert!(deeper.limit_depth(), "a document nested deeper is brought within the cap when it is read");
        let mut whole = d.clone();
        let looked = Cell::new(0_usize);
        let pick = |n: &Node| {
            looked.set(looked.get() + 1);
            on_artboard_7(n)
        };
        let held = held_guides(&d.layers[0], &pick).unwrap();
        assert_eq!(looked.get(), NESTED_CHILDREN, "each child of the layers is looked at once");
        strip_held(Arc::make_mut(&mut d.layers[0]), held);
        assert_eq!(looked.get(), NESTED_CHILDREN, "removing looks at nothing again");
        assert!(d.layers[0].children().is_some_and(|c| Arc::ptr_eq(&c[1], &still)), "a sublayer with nothing to remove is not copied");

        // The command's own entry point removes the same guides.
        let before = whole.clone();
        let mut sel = Selection::default();
        sel.set_guides([NodeId(200), NodeId(201), NodeId(501)]);
        sel.objects.push(NodeId(150));
        sel.key_guide = Some(NodeId(201));
        assert_eq!(whole.retain_guides(&mut sel, |g| g.artboard != Some(7)), CHAIN as usize);
        assert_eq!(ids(&whole), ids(&d));
        assert_eq!(ids(&whole).len(), ids(&before).len() - CHAIN as usize);
        assert!(whole.ruler_guides().iter().all(|g| g.artboard.is_none()));
        assert_eq!(whole.ruler_guides().len(), CHAIN as usize + 1);
        assert_eq!((sel.objects.as_slice(), sel.guides.as_slice(), sel.key_guide), (&[NodeId(150)][..], &[NodeId(200), NodeId(501)][..], None));
        assert!(whole.layers[0].children().is_some_and(|c| Arc::ptr_eq(&c[1], &still)));
        // The document as it was (an undo step's copy) still holds every guide.
        assert_eq!(before.ruler_guides().len(), CHAIN as usize * 2 + 1);

        // Saving the guides as a list, and removing a layer but not its guides, take the guides
        // out of the nested layers too.
        let mut saved = before.clone();
        saved.guides_to_saved();
        assert_eq!(saved.saved_guides, before.ruler_guides());
        assert_eq!(ids(&saved).len(), 1 + CHAIN as usize * 2 + 2);
        assert!(saved.layers[0].children().is_some_and(|c| !Arc::ptr_eq(&c[1], &still) && c[1].children().is_some_and(|c| c.len() == 1)));
        let mut edited = before.clone();
        let chain = edited.remove_edit_layer(NodeId(100 + CHAIN - 1)).unwrap();
        assert!(!chain.holds_ruler_guides());
        assert_eq!(edited.ruler_guides().len(), before.ruler_guides().len(), "the removed layer's guides stay in the document");
        assert_eq!(edited.guides_on(edited.layers[0].id).len(), CHAIN as usize * 2);

        // Keeping every guide changes nothing and copies no layer.
        let kept = whole.clone();
        assert_eq!(whole.retain_guides(&mut sel, |_| true), 0);
        assert!(Arc::ptr_eq(&whole.layers[0], &kept.layers[0]));
        // Removing every guide leaves the objects.
        assert_eq!(whole.retain_guides(&mut sel, |_| false), CHAIN as usize + 1);
        assert!(whole.ruler_guides().is_empty() && sel.guides.is_empty());
        assert_eq!(ids(&whole).len(), 1 + CHAIN as usize * 2 + 2);
        assert_eq!(sel.objects, [NodeId(150)]);
        // A document without layers, and one without guides, have nothing to remove.
        let mut empty = Document::new(10.0, 10.0);
        assert_eq!(empty.retain_guides(&mut sel, |_| false), 0);
        empty.layers.clear();
        assert_eq!(empty.retain_guides(&mut sel, |_| false), 0);
    }

    #[test]
    fn moving_an_artboards_guides_looks_at_each_child_of_nested_layers_once() {
        let (mut d, still) = nested();
        let mut whole = d.clone();
        let looked = Cell::new(0_usize);
        let pick = |n: &Node| {
            looked.set(looked.get() + 1);
            on_artboard_7(n)
        };
        let held = held_guides(&d.layers[0], &pick).unwrap();
        assert_eq!(looked.get(), NESTED_CHILDREN, "each child of the layers is looked at once");
        let mut seen = vec![];
        map_held(Arc::make_mut(&mut d.layers[0]), held, &mut |vertical, pos: &mut f64| {
            seen.push((vertical, *pos));
            *pos += 9.0;
        });
        assert_eq!(looked.get(), NESTED_CHILDREN, "changing looks at nothing again");
        // A sublayer comes before its layer's own guides, so the innermost guide is changed first.
        let order: Vec<(bool, f64)> = [201.0].into_iter().chain((1..CHAIN).map(|depth| (301 + depth * 2) as f64)).map(|pos| (false, pos)).collect();
        assert_eq!(seen, order, "each guide of the artboard is changed once, in the order the guides lie in");
        assert!(d.layers[0].children().is_some_and(|c| Arc::ptr_eq(&c[1], &still)), "a sublayer with no guide of the artboard is not copied");

        // The public moves change the same guides by the same amount.
        let before = whole.clone();
        whole.move_artboard_guides(7, Vec2::new(5.0, 9.0));
        assert_eq!(whole, d);
        assert_eq!(ids(&whole), ids(&before));
        for g in whole.ruler_guides() {
            let id = g.id.map_or(0, |id| id.0) as f64;
            assert_eq!(g.pos, if g.artboard == Some(7) { id + 9.0 } else { id }, "guide {id}");
        }
        assert!(whole.layers[0].children().is_some_and(|c| Arc::ptr_eq(&c[1], &still)));
        assert_eq!(before.ruler_guide(NodeId(201)).map(|g| g.pos), Some(201.0), "the document as it was keeps its guides");
        whole.map_artboard_guides(7, Affine::scale(2.0));
        assert_eq!(whole.ruler_guide(NodeId(201)).map(|g| g.pos), Some(420.0));
        assert_eq!(whole.ruler_guide(NodeId(200)).map(|g| g.pos), Some(200.0), "a canvas guide stays");

        // An artboard without guides, and a document without layers, change nothing and copy no layer.
        let kept = whole.clone();
        whole.move_artboard_guides(99, Vec2::new(5.0, 9.0));
        assert!(Arc::ptr_eq(&whole.layers[0], &kept.layers[0]));
        let mut empty = Document::new(10.0, 10.0);
        empty.layers.clear();
        empty.move_artboard_guides(7, Vec2::new(5.0, 9.0));
        assert!(empty.layers.is_empty());
    }

    #[test]
    fn guides_in_objects_are_found_by_looking_at_each_node_once() {
        // Groups, each inside the one before and holding an object and a guide. Beside them, an
        // object whose mask art holds a guide, one whose mask art is a guide, and a group with none.
        let mut chain = group(100, vec![rect(150), guide(200, None)]);
        for depth in 1..CHAIN {
            chain = group(100 + depth, vec![rect(600 + depth), chain, guide(300 + depth, None)]);
        }
        let still = group(700, vec![rect(701)]);
        let root = group(
            1,
            vec![chain, masked(rect(800), group(801, vec![guide(802, None), rect(803)])), masked(rect(810), guide(811, None)), Arc::clone(&still)],
        );
        let looked = Cell::new(0_usize);
        let is_guide = |n: &Node| {
            looked.set(looked.get() + 1);
            n.is_ruler_guide()
        };
        // The chain's groups, objects and guides; a masked object, its mask group and the two in
        // it; a masked object and its mask guide; the last group and its object.
        let nodes = CHAIN as usize * 3 + 4 + 2 + 2;
        let found = guides_below(&root, &is_guide).unwrap();
        assert_eq!(looked.get(), nodes, "each node below the root is looked at once");
        let mut cleared = (*root).clone();
        assert_eq!(drop_guides_below(&mut cleared, found), CHAIN as usize + 2);
        assert_eq!(looked.get(), nodes, "removing looks at nothing again");
        assert!(guides_below(&cleared, &Node::is_ruler_guide).is_none());
        let ch = cleared.children().unwrap();
        assert!(Arc::ptr_eq(&ch[3], &still), "a group without guides is not copied");
        assert_eq!(
            ch[1].extras().mask.as_deref().map(|m| m.art.children().map(|c| c.iter().map(|c| c.id.0).collect::<Vec<_>>())),
            Some(Some(vec![803]))
        );
        assert_eq!(ch[2].extras().mask.as_deref().map(|m| (m.art.id, m.art.children().map(Vec::len))), Some((NodeId(811), Some(0))));

        // Opening a file removes the same guides from an object on a layer and from a symbol's art.
        let mut d = doc(vec![Arc::clone(&root), guide(900, None)]);
        d.symbols = vec![Symbol { name: "s".into(), art: Arc::clone(&root), dynamic: false }];
        assert_eq!(d.tidy_ruler_guides(), (CHAIN as usize + 2) * 2);
        assert_eq!(d.layers[0].children().map(|c| (*c[0]).clone()), Some(cleared.clone()));
        assert_eq!(*d.symbols[0].art, cleared);
        assert_eq!(d.ruler_guides().iter().map(|g| g.id).collect::<Vec<_>>(), [Some(NodeId(900))]);
    }

    #[test]
    fn guides_out_of_place_are_found_by_looking_at_each_node_once() {
        // Sublayers, each inside the one before, with their guides in place; the innermost also
        // holds a guide below its art. Beside them, a group holding a guide, a sublayer with its
        // guide in place, and a sublayer whose mask art holds a guide.
        let mut chain = sublayer(100, vec![guide(200, None), rect(150), guide(201, None)]);
        for depth in 1..CHAIN {
            chain = sublayer(100 + depth, vec![rect(600 + depth), chain, guide(300 + depth, None)]);
        }
        let still = sublayer(500, vec![rect(502), guide(501, None)]);
        let with_mask = masked(sublayer(510, vec![rect(513)]), group(511, vec![guide(512, None)]));
        let mut d = doc(vec![chain, group(700, vec![rect(701), guide(702, None)]), Arc::clone(&still), with_mask, guide(900, None)]);
        let mut whole = d.clone();
        let looked = Cell::new(0_usize);
        let is_guide = |n: &Node| {
            looked.set(looked.get() + 1);
            n.is_ruler_guide()
        };
        // Below the guides at the end of each layer: the first layer's 4, the group's 2, the
        // untouched sublayer's 1, the masked sublayer's mask group, its guide and the object, 2 in
        // the innermost sublayer and 2 in each sublayer around it.
        let nodes = 4 + 2 + 1 + 3 + 2 + (CHAIN as usize - 1) * 2;
        let found = untidy(&d.layers[0], &is_guide).unwrap();
        assert_eq!(looked.get(), nodes, "each node below the guides in place is looked at once");
        assert_eq!(tidy_layer(Arc::make_mut(&mut d.layers[0]), found), 2);
        assert_eq!(looked.get(), nodes, "tidying looks at nothing again");
        assert!(d.layers[0].children().is_some_and(|c| Arc::ptr_eq(&c[2], &still)), "a sublayer with its guides in place is not copied");
        assert!(untidy(&d.layers[0], &Node::is_ruler_guide).is_none());

        // Opening a file does the same.
        let before = whole.clone();
        assert_eq!(whole.tidy_ruler_guides(), 2);
        assert_eq!(whole, d);
        let all = ids(&whole);
        assert!(!all.contains(&702) && all.len() == ids(&before).len() - 1);
        let innermost = whole.node(NodeId(100)).and_then(|n| n.children()).map(|c| c.iter().map(|c| c.id.0).collect::<Vec<_>>());
        assert_eq!(innermost, Some(vec![150, 200, 201]), "a guide below art moves up to the other guides");
        assert!(whole.node(NodeId(510)).is_some_and(|n| n.extras().mask.as_deref().is_some_and(|m| m.art.children().is_some_and(Vec::is_empty))));
        // A tidy document is left alone, and no layer is copied.
        let kept = whole.clone();
        assert_eq!(whole.tidy_ruler_guides(), 0);
        assert!(Arc::ptr_eq(&whole.layers[0], &kept.layers[0]));
    }
}
