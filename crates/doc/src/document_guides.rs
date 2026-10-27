use super::document_ext::free_id;
use super::guides::Guide;
use super::*;

impl Document {
    /// Union of the geometric bounds of `ids` (their visual bounds when `visual`), with symbol
    /// instances measured by their art ([`Node::geometric_bounds_with`]).
    pub fn bounds_of(&self, ids: &[NodeId], visual: bool) -> Option<Rect> {
        let symbols = self.symbol_boxes();
        ids.iter().filter_map(|id| self.node(*id)).fold(None, |acc, n| {
            vectorcraft_geom::union_opt(acc, if visual { n.visual_bounds_with(&symbols) } else { n.geometric_bounds_with(&symbols) })
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
        // As in `art_on_artboard`: a millionth of a point covers rounding at the edges.
        let rect = rect.inflate(1e-6, 1e-6);
        fn meets(b: Option<Rect>, r: Rect) -> bool {
            b.is_some_and(|b| b.x0 <= r.x1 && r.x0 <= b.x1 && b.y0 <= r.y1 && r.y0 <= b.y1)
        }
        fn has(c: &Node, s: &SymbolBoxes, rect: Rect) -> bool {
            match &c.kind {
                NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                    let Some((clip, rest)) = children.split_first() else { return false };
                    let paint = clip.clip_paint();
                    if (paint.fill.is_some() || paint.stroke.is_some()) && meets(clip.visual_bounds_with(s), rect) {
                        return true;
                    }
                    // The members show only inside the clip's box.
                    let Some(b) = c.visual_bounds_with(s).filter(|b| meets(Some(*b), rect)) else { return false };
                    let inside = Rect::new(b.x0.max(rect.x0), b.y0.max(rect.y0), b.x1.min(rect.x1), b.y1.min(rect.y1));
                    rest.iter().any(|m| has(m, s, inside))
                }
                NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => children.iter().any(|m| has(m, s, rect)),
                NodeKind::Path { guide: true, .. } => false,
                _ => meets(c.visual_bounds_with(s), rect),
            }
        }
        let symbols = self.symbol_boxes();
        self.layers.iter().any(|l| has(l, &symbols, rect))
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

    /// Where ruler guide `g` runs along its line (the y range of a vertical guide): across its
    /// artboard for an artboard guide, None (the whole canvas) for a canvas guide or one whose
    /// artboard is gone.
    pub fn guide_span(&self, g: &Guide) -> Option<(f64, f64)> {
        let r = self.artboards.iter().find(|a| Some(a.id) == g.artboard)?.rect;
        Some(if g.vertical { (r.y0, r.y1) } else { (r.x0, r.x1) })
    }

    /// The layer (or sublayer) ruler guide `g` is on, if it is on one this document has.
    fn guide_layer(&self, g: &Guide) -> Option<NodeId> {
        g.layer.filter(|l| self.node(*l).is_some_and(Node::is_layer))
    }

    /// Is ruler guide `g` shown: on no layer, or on a layer that (with the layers around it) is
    /// visible? A guide whose layer is gone shows as one on no layer.
    pub fn guide_shown(&self, g: &Guide) -> bool {
        self.guide_layer(g).is_none_or(|l| self.is_visible(l))
    }

    /// Can ruler guide `g` be picked, moved and deleted: shown, and its layer unlocked?
    pub fn guide_editable(&self, g: &Guide) -> bool {
        self.guide_layer(g).is_none_or(|l| self.is_editable(l))
    }

    /// Put the ruler guides on no layer this document has (from files saved before guides had
    /// layers, or of layers merged into `layer`) on layer `layer`. Returns how many.
    pub fn rehome_guides(&mut self, layer: NodeId) -> usize {
        let lost: Vec<bool> = self.guides.iter().map(|g| self.guide_layer(g).is_none()).collect();
        let mut n = 0;
        for (g, lost) in self.guides.iter_mut().zip(lost) {
            if lost {
                g.layer = Some(layer);
                n += 1;
            }
        }
        n
    }

    /// Put the ruler guides on no layer (from files saved before guides had layers), or on one that
    /// is gone, on the top visible, unlocked layer ([`Self::default_layer`]). Returns how many.
    pub fn adopt_guides(&mut self) -> usize {
        self.default_layer().map_or(0, |home| self.rehome_guides(home))
    }

    /// Delete the ruler guides whose layer was deleted (they go with it), keeping `sel`'s guides in
    /// step. Guides on no layer stay. Returns how many went.
    pub fn drop_guides_of_deleted_layers(&mut self, sel: &mut Selection) -> usize {
        let gone: Vec<bool> = self.guides.iter().map(|g| g.layer.is_some() && self.guide_layer(g).is_none()).collect();
        self.retain_guides(sel, |i, _| !gone.get(i).copied().unwrap_or(false))
    }

    /// Copy the ruler guides of layer `from` and of its sublayers onto their copies in `copy` (made
    /// from `from` by [`Self::reid`]: the same tree with fresh ids), as duplicating a layer copies
    /// the objects on it. Returns how many.
    pub fn copy_layer_guides(&mut self, from: &Node, copy: &Node) -> usize {
        let layers = |n: &Node| {
            let mut out = vec![];
            n.walk(&mut |c| {
                if c.is_layer() {
                    out.push(c.id);
                }
            });
            out
        };
        let pairs: Vec<(NodeId, NodeId)> = layers(from).into_iter().zip(layers(copy)).collect();
        let copies: Vec<Guide> = self
            .guides
            .iter()
            .filter_map(|g| {
                let to = pairs.iter().find(|(a, _)| Some(*a) == g.layer)?.1;
                Some(Guide { layer: Some(to), ..g.clone() })
            })
            .collect();
        let n = copies.len();
        self.guides.extend(copies);
        n
    }

    /// The ruler guides on layer `layer` (not its sublayers'), by index.
    pub fn guides_on(&self, layer: NodeId) -> impl Iterator<Item = (usize, &Guide)> + '_ {
        self.guides.iter().enumerate().filter(move |(_, g)| g.layer == Some(layer))
    }

    /// Does ruler guide `g` run past `p` (up to `tol` beyond its ends)?
    pub fn guide_passes(&self, g: &Guide, p: Point, tol: f64) -> bool {
        let along = if g.vertical { p.y } else { p.x };
        self.guide_span(g).is_none_or(|(a, b)| along >= a - tol && along <= b + tol)
    }

    /// Keep the ruler guides `keep` accepts (by index), the selected ones left keeping their
    /// place among the rest (the selected art stays selected). Returns how many went.
    pub fn retain_guides(&mut self, sel: &mut Selection, keep: impl Fn(usize, &Guide) -> bool) -> usize {
        let kept: Vec<bool> = self.guides.iter().enumerate().map(|(i, g)| keep(i, g)).collect();
        // Where each kept guide ends up.
        let (mut to, mut n) = (Vec::with_capacity(kept.len()), 0);
        for k in &kept {
            to.push(k.then_some(n));
            n += usize::from(*k);
        }
        sel.guides = sel.guides.iter().filter_map(|i| to.get(*i).copied().flatten()).collect();
        sel.key_guide = sel.key_guide.and_then(|i| to.get(i).copied().flatten());
        let before = self.guides.len();
        let mut flags = kept.into_iter();
        self.guides.retain(|_| flags.next().unwrap_or(true));
        before - self.guides.len()
    }

    /// Move the guides of artboard `id` by `d` (along with their artboard).
    pub fn move_artboard_guides(&mut self, id: u32, d: Vec2) {
        for g in self.guides.iter_mut().filter(|g| g.artboard == Some(id)) {
            *g = g.moved(d);
        }
    }

    /// Map the guides of artboard `id` through `xf`, an axis-aligned scale and move (Scale Artwork
    /// with Artboard).
    pub fn map_artboard_guides(&mut self, id: u32, xf: Affine) {
        for g in self.guides.iter_mut().filter(|g| g.artboard == Some(id)) {
            g.pos = if g.vertical { (xf * Point::new(g.pos, 0.0)).x } else { (xf * Point::new(0.0, g.pos)).y };
        }
    }

    /// Copy the guides of artboard `from` onto artboard `to`, `d` away (with a copy of their
    /// artboard).
    pub fn copy_artboard_guides(&mut self, from: u32, to: u32, d: Vec2) {
        let copies: Vec<Guide> =
            self.guides.iter().filter(|g| g.artboard == Some(from)).map(|g| Guide { artboard: Some(to), ..g.moved(d) }).collect();
        self.guides.extend(copies);
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
        }
        let mut renamed = std::collections::HashMap::new();
        if let Some(ch) = n.children_mut() {
            let old: Vec<Arc<Node>> = std::mem::take(ch);
            *ch = old.iter().map(|c| Arc::new(self.reid(c))).collect();
            renamed.extend(old.iter().zip(ch.iter()).map(|(o, c)| (o.id, c.id)));
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
