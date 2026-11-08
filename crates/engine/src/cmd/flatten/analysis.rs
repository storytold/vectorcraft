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
pub(crate) fn outlined_text(n: &Node) -> Option<Node> {
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

/// Indices of `bounds` grouped by overlap (transitively), each group in index order. Boxes that
/// only touch overlap; a box with a coordinate that isn't finite overlaps none.
///
/// A sweep left to right keeps the boxes still open at the current left edge in a tree by their
/// tops that knows the lowest bottom below each of its branches, so each box meets only the open
/// boxes it overlaps, not every open one (many objects stacked in a column, sharing their
/// horizontal extent, would otherwise compare every pair). An open box inside a later box of
/// its group, ending no further right, is closed early: whatever overlaps it overlaps that box.
/// Fails only when cancelled ([`vectorcraft_doc::cancel`]).
pub(super) fn overlapping(bounds: &[Option<Rect>]) -> Result<Vec<Vec<usize>>, vectorcraft_doc::cancel::Cancelled> {
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..bounds.len()).collect();
    let finite = |b: &Rect| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) && b.x0 <= b.x1 && b.y0 <= b.y1;
    let boxes: Vec<(usize, Rect)> = bounds.iter().enumerate().filter_map(|(i, b)| b.filter(finite).map(|b| (i, b))).collect();
    let n = boxes.len();
    // Each box's place in the tree: by top (then index).
    let mut by_top: Vec<usize> = (0..n).collect();
    by_top.sort_by(|&a, &b| boxes[a].1.y0.total_cmp(&boxes[b].1.y0).then(a.cmp(&b)));
    let tops: Vec<f64> = by_top.iter().map(|&k| boxes[k].1.y0).collect();
    let mut leaf = vec![0usize; n];
    for (pos, &k) in by_top.iter().enumerate() {
        leaf[k] = pos;
    }
    // The lowest bottom of the open boxes below each node (−∞: none open).
    let size = n.next_power_of_two().max(1);
    let mut low = vec![f64::NEG_INFINITY; 2 * size];
    let set = |low: &mut [f64], pos: usize, v: f64| {
        let mut i = pos + size;
        low[i] = v;
        while i > 1 {
            i /= 2;
            low[i] = low[2 * i].max(low[2 * i + 1]);
        }
    };
    let mut by_left: Vec<usize> = (0..n).collect();
    by_left.sort_by(|&a, &b| boxes[a].1.x0.total_cmp(&boxes[b].1.x0));
    let mut by_right: Vec<usize> = (0..n).collect();
    by_right.sort_by(|&a, &b| boxes[a].1.x1.total_cmp(&boxes[b].1.x1));
    let mut closing = 0;
    let mut stack: Vec<(usize, usize, usize)> = vec![];
    let mut met = vec![];
    let mut work = 0usize;
    for &k in &by_left {
        let b = boxes[k].1;
        // Close the boxes that end left of this one.
        while let Some(&c) = by_right.get(closing)
            && boxes[c].1.x1 < b.x0
        {
            set(&mut low, leaf[c], f64::NEG_INFINITY);
            closing += 1;
        }
        // The open boxes with their top at most this bottom and their bottom at least this top.
        let below = tops.partition_point(|t| *t <= b.y1);
        met.clear();
        stack.clear();
        stack.push((1, 0, size));
        while let Some((node, lo, hi)) = stack.pop() {
            work += 1;
            if work.is_multiple_of(4096) {
                vectorcraft_doc::cancel::check()?;
            }
            if lo >= below || low[node] < b.y0 {
                continue;
            }
            if hi - lo == 1 {
                met.push(by_top[lo]);
                continue;
            }
            let mid = (lo + hi) / 2;
            stack.push((2 * node + 1, mid, hi));
            stack.push((2 * node, lo, mid));
        }
        for &o in &met {
            let (a, c) = (find(&mut parent, boxes[k].0), find(&mut parent, boxes[o].0));
            parent[a.max(c)] = a.min(c);
            let ob = boxes[o].1;
            if ob.x1 <= b.x1 && b.y0 <= ob.y0 && ob.y1 <= b.y1 {
                set(&mut low, leaf[o], f64::NEG_INFINITY);
            }
        }
        set(&mut low, leaf[k], b.y1);
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
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What [`overlapping`] gives, comparing every pair.
    fn every_pair(bounds: &[Option<Rect>]) -> Vec<Vec<usize>> {
        let n = bounds.len();
        let mut group: Vec<usize> = (0..n).collect();
        let hit = |a: &Rect, b: &Rect| a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1;
        loop {
            let mut changed = false;
            for i in 0..n {
                for j in 0..n {
                    if let (Some(a), Some(b)) = (bounds[i], bounds[j])
                        && hit(&a, &b)
                        && group[j] < group[i]
                    {
                        group[i] = group[j];
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let mut out: Vec<Vec<usize>> = vec![];
        let mut seen = std::collections::HashMap::new();
        for (i, &root) in group.iter().enumerate() {
            let g = *seen.entry(root).or_insert_with(|| {
                out.push(vec![]);
                out.len() - 1
            });
            out[g].push(i);
        }
        out
    }

    /// Boxes of many sizes, nested, touching and apart, group as comparing every pair groups them.
    #[test]
    fn groups_match_comparing_every_pair() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |m: f64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1000) as f64 / 1000.0 * m
        };
        for round in 0..40 {
            let n = 5 + round * 4;
            let bounds: Vec<Option<Rect>> = (0..n)
                .map(|i| {
                    if i % 11 == 3 {
                        return None;
                    }
                    let (x, y) = (next(100.0).round(), next(100.0).round());
                    Some(Rect::new(x, y, x + next(30.0).round(), y + next(30.0).round()))
                })
                .collect();
            assert_eq!(overlapping(&bounds).unwrap(), every_pair(&bounds), "round {round}");
        }
    }

    /// A column of many boxes sharing their horizontal extent: each one meets only its
    /// neighbours, so they group (here, touching in pairs) without comparing every pair.
    #[test]
    fn a_tall_column_groups_by_its_neighbours() {
        let n = 200_000;
        let bounds: Vec<Option<Rect>> =
            (0..n).map(|i| Some(Rect::new(0.0, 3.0 * f64::from(i), 100.0, 3.0 * f64::from(i) + if i % 2 == 0 { 3.0 } else { 1.0 }))).collect();
        let groups = overlapping(&bounds).unwrap();
        assert_eq!(groups.len(), n as usize / 2);
        assert!(groups.iter().enumerate().all(|(g, ids)| ids == &[2 * g, 2 * g + 1]));
        // Boxes that aren't finite overlap nothing.
        let odd = [Some(Rect::new(0.0, 0.0, 10.0, 10.0)), Some(Rect::new(f64::NAN, 0.0, 10.0, 10.0)), Some(Rect::new(5.0, 5.0, 20.0, 20.0))];
        assert_eq!(overlapping(&odd).unwrap(), vec![vec![0, 2], vec![1]]);
    }
}
