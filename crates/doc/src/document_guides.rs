use super::document_ext::free_id;
use super::guides::Guide;
use super::limits::MAX_NEXT_ID;
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
            if layer_untidy(l) {
                gone += tidy_layer(Arc::make_mut(l));
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
            } else if any_guide(root) {
                gone += drop_guides_below(Arc::make_mut(root));
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
            if l.holds_ruler_guides() {
                Arc::make_mut(l).strip_ruler_guides();
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
        if !layer.holds_ruler_guides() {
            return Ok(layer);
        }
        let mut guides = vec![];
        layer.walk(&mut |c| {
            if c.is_ruler_guide() {
                guides.push(Arc::new(c.clone()));
            }
        });
        Arc::make_mut(&mut layer).strip_ruler_guides();
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
        fn drop(layer: &mut Arc<Node>, gone: &HashSet<NodeId>) {
            fn holds(n: &Node, gone: &HashSet<NodeId>) -> bool {
                n.is_layer() && n.children().is_some_and(|ch| ch.iter().any(|c| gone.contains(&c.id) || holds(c, gone)))
            }
            if !holds(layer, gone) {
                return;
            }
            let Some(ch) = Arc::make_mut(layer).children_mut() else { return };
            ch.retain(|c| !(c.is_ruler_guide() && gone.contains(&c.id)));
            for c in ch.iter_mut().filter(|c| c.is_layer()) {
                drop(c, gone);
            }
        }
        for l in &mut self.layers {
            drop(l, &gone);
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
        fn visit(layer: &mut Arc<Node>, id: u32, f: &mut impl FnMut(bool, &mut f64)) {
            fn holds(n: &Node, id: u32) -> bool {
                n.is_layer()
                    && n.children().is_some_and(|ch| {
                        ch.iter().any(|c| matches!(c.kind, NodeKind::RulerGuide { artboard: Some(a), .. } if a == id) || holds(c, id))
                    })
            }
            if !holds(layer, id) {
                return;
            }
            let Some(ch) = Arc::make_mut(layer).children_mut() else { return };
            for c in ch.iter_mut() {
                if matches!(c.kind, NodeKind::RulerGuide { artboard: Some(a), .. } if a == id) {
                    if let NodeKind::RulerGuide { vertical, pos, .. } = &mut Arc::make_mut(c).kind {
                        f(*vertical, pos);
                    }
                } else if c.is_layer() {
                    visit(c, id, f);
                }
            }
        }
        for l in &mut self.layers {
            visit(l, id, f);
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

/// Does `n`, or anything below it (opacity-mask art included), hold a ruler guide?
fn any_guide(n: &Node) -> bool {
    let mut any = false;
    crate::swatches::walk_with_masks(n, &mut |m| any |= m.is_ruler_guide());
    any
}

/// Remove every ruler guide below `n`, in its members and its opacity-mask art (`n` itself
/// stays). Mask art that is a ruler guide becomes an empty group. Returns how many went.
fn drop_guides_below(n: &mut Node) -> usize {
    let mut gone = drop_mask_guides(n);
    if let Some(ch) = n.children_mut() {
        let before = ch.len();
        ch.retain(|c| !c.is_ruler_guide());
        gone += before - ch.len();
        for c in ch.iter_mut().filter(|c| any_guide(c)) {
            gone += drop_guides_below(Arc::make_mut(c));
        }
    }
    gone
}

/// Remove every ruler guide in the opacity-mask art of `n`. Mask art that is a ruler guide
/// becomes an empty group. Returns how many went.
fn drop_mask_guides(n: &mut Node) -> usize {
    let Some(mask) = n.extras_if_any().and_then(|e| e.mask.as_deref_mut()) else { return 0 };
    if mask.art.is_ruler_guide() {
        mask.art = Arc::new(Node::group(mask.art.id, vec![]));
        1
    } else if any_guide(&mask.art) {
        drop_guides_below(Arc::make_mut(&mut mask.art))
    } else {
        0
    }
}

/// Does layer `l` hold a ruler guide out of place: below art, in an object or in mask art, or so
/// in one of its sublayers?
fn layer_untidy(l: &Node) -> bool {
    let Some(ch) = l.children() else { return false };
    let start = Node::guide_run_start(ch);
    l.extras().mask.as_deref().is_some_and(|m| any_guide(&m.art))
        || ch.iter().take(start).any(|c| c.is_ruler_guide() || if c.is_layer() { layer_untidy(c) } else { any_guide(c) })
}

/// [`Document::tidy_ruler_guides`] for layer `l`. Returns how many guides were removed.
fn tidy_layer(l: &mut Node) -> usize {
    let mut gone = drop_mask_guides(l);
    let Some(ch) = l.children_mut() else { return gone };
    for c in ch.iter_mut() {
        if c.is_layer() {
            if layer_untidy(c) {
                gone += tidy_layer(Arc::make_mut(c));
            }
        } else if !c.is_ruler_guide() && any_guide(c) {
            gone += drop_guides_below(Arc::make_mut(c));
        }
    }
    if ch.iter().take(Node::guide_run_start(ch)).any(|c| c.is_ruler_guide()) {
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
