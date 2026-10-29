use super::{Arc, BTreeSet, Document, Effect, ImageBlob, Knockout, Node, NodeKind, Rect, effects, outline_strokes};

// ---------- plain art ----------

/// Converts art into what the compositor works on.
pub(super) struct Scratch {
    /// A copy of the document the art comes from (outlining allocates ids in it).
    pub(super) doc: Document,
    pub(super) brushes: Vec<vectorcraft_brush::Brush>,
}

impl Scratch {
    /// `n` as plain filled shapes: type outlined, symbol instances replaced by their art, live
    /// objects evaluated and strokes outlined (brush strokes as their art), each keeping the
    /// object's transparency.
    pub(super) fn plain(&mut self, n: &Node) -> Node {
        if !n.visible {
            return n.clone();
        }
        let made = match &n.kind {
            NodeKind::Text(_) => outlined_text(n),
            NodeKind::SymbolInstance { symbol, .. } => {
                let art = self.doc.symbols.iter().find(|s| &s.name == symbol).map(|s| vectorcraft_render::instance_art(&s.art, n));
                art.and_then(|a| effects::outline_art(Some(&self.doc), n, Some(&a))).map(|g| carry(n, g))
            }
            // A placed document stays one: its art's resources aren't the document's.
            NodeKind::PlacedDocument(_) => None,
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) => {
                let hook: &dyn Fn(&Node) -> Option<Node> = &effects::outline_text;
                let hooks = vectorcraft_doc::live::Hooks { outline: Some(hook), symbols: Some(&self.doc), ..Default::default() };
                Some(carry(n, vectorcraft_doc::live::expanded_group_hooks(n, hooks)))
            }
            NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => outline_strokes(&mut self.doc, &self.brushes, n),
            _ => None,
        };
        let mut m = match made {
            // The pieces of one object never knock each other out.
            Some(m) if m.is_container() => Node { knockout: Knockout::Off, ..m },
            Some(m) => m,
            None => n.clone(),
        };
        if !matches!(m.kind, NodeKind::Compound { .. })
            && let Some(ch) = m.children_mut()
        {
            for c in ch.iter_mut() {
                *c = Arc::new(self.plain(c));
            }
        }
        m
    }
}

/// Type as outlines painted like it ([`effects::outline_text`]), keeping its transparency.
pub(super) fn outlined_text(n: &Node) -> Option<Node> {
    effects::outline_text(n).map(|g| Node { knockout: Knockout::Off, ..carry(n, g) })
}

/// `to` (art made from `from`) with `from`'s name, transparency, opacity mask and effects.
fn carry(from: &Node, mut to: Node) -> Node {
    to.name = from.name.clone();
    to.opacity = from.opacity;
    to.blend = from.blend;
    to.isolate = from.isolate;
    to.knockout_shape = from.knockout_shape;
    to.extras_mut().mask = from.extras().mask.clone();
    to.appearance.effects = from.appearance.effects.clone();
    to
}

/// The keys of `doc`'s images that have see-through pixels (an alpha channel that isn't opaque
/// everywhere).
pub(crate) fn see_through_images(doc: &Document) -> BTreeSet<String> {
    let see_through = |b: &ImageBlob| {
        b.mime != "image/jpeg" && image::load_from_memory(&b.bytes).is_ok_and(|i| i.color().has_alpha() && i.to_rgba8().pixels().any(|p| p[3] < 255))
    };
    doc.images.iter().filter(|(_, b)| see_through(b)).map(|(k, _)| k.clone()).collect()
}

/// `n` shows an image of `keys` (itself or inside).
pub(super) fn draws_image(n: &Node, keys: &BTreeSet<String>) -> bool {
    !keys.is_empty()
        && n.visible
        && match &n.kind {
            NodeKind::Image(im) => keys.contains(&im.key),
            _ => n.children().is_some_and(|ch| ch.iter().any(|c| draws_image(c, keys))),
        }
}

fn visible_fx(fx: &[Effect]) -> bool {
    fx.iter().any(|e| e.visible)
}

/// Effects on the object or on one of its fills or strokes (raster effects, once baked).
pub(super) fn has_effects(n: &Node) -> bool {
    visible_fx(&n.appearance.effects) || n.appearance.items.iter().any(|i| visible_fx(i.effects()))
}

pub(super) fn masked(n: &Node) -> bool {
    n.extras().mask.as_ref().is_some_and(|m| !m.disabled)
}

/// Where `n` can paint: its visual bounds grown by its effects (and a margin for antialiasing).
pub(super) fn reach(n: &Node) -> Option<Rect> {
    if !n.visible {
        return None;
    }
    let b = match n.children() {
        Some(ch) if !n.clips() && !matches!(n.kind, NodeKind::Compound { .. }) => ch.iter().filter_map(|c| reach(c)).reduce(|a, b| a.union(b))?,
        _ => n.visual_bounds()?,
    };
    let fx = n.appearance.items.iter().map(|i| effects::outset(i.effects(), b)).fold(effects::outset(&n.appearance.effects, b), f64::max);
    Some(b.inflate(fx + 1.0, fx + 1.0))
}

/// Indices of `bounds` grouped by overlap (transitively), each group in index order.
pub(super) fn overlapping(bounds: &[Option<Rect>]) -> Vec<Vec<usize>> {
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..bounds.len()).collect();
    let mut order: Vec<(usize, Rect)> = bounds.iter().enumerate().filter_map(|(i, b)| b.map(|b| (i, b))).collect();
    order.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0));
    // Sweep left to right, comparing each box with those still open at its left edge.
    let mut open: Vec<(usize, Rect)> = vec![];
    for (i, b) in order {
        open.retain(|(_, o)| o.x1 >= b.x0);
        for (j, o) in &open {
            if o.y0 <= b.y1 && b.y0 <= o.y1 {
                let (a, c) = (find(&mut parent, i), find(&mut parent, *j));
                parent[a.max(c)] = a.min(c);
            }
        }
        open.push((i, b));
    }
    let mut groups: Vec<Vec<usize>> = vec![];
    let mut slot: Vec<Option<usize>> = vec![None; bounds.len()];
    for i in 0..bounds.len() {
        let r = find(&mut parent, i);
        match slot[r] {
            Some(g) => groups[g].push(i),
            None => {
                slot[r] = Some(groups.len());
                groups.push(vec![i]);
            }
        }
    }
    groups
}
