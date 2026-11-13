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

/// `n` shows an image of `keys` (itself or inside), a color-masked image's masked pixels counted as
/// what it shows.
pub(super) fn draws_image(n: &Node, keys: &BTreeSet<String>) -> bool {
    !keys.is_empty()
        && n.visible
        && match &n.kind {
            NodeKind::Image(im) => keys.contains(&im.key) || Document::color_mask_of(n).is_some_and(|m| m.made_for(im) && keys.contains(&m.key)),
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

/// Where `n` paints: its visual bounds grown by its effects.
pub(super) fn painted(n: &Node) -> Option<Rect> {
    if !n.visible {
        return None;
    }
    let b = match n.children() {
        Some(ch) if !n.clips() && !matches!(n.kind, NodeKind::Compound { .. }) => ch.iter().filter_map(|c| painted(c)).reduce(|a, b| a.union(b))?,
        _ => n.visual_bounds()?,
    };
    let fx = n.appearance.items.iter().map(|i| effects::outset(i.effects(), b)).fold(effects::outset(&n.appearance.effects, b), f64::max);
    Some(b.inflate(fx, fx))
}

/// Where `n` can paint: where it paints ([`painted`]) and a margin of 1 pt for antialiasing,
/// once, however deep its groups nest.
pub(super) fn reach(n: &Node) -> Option<Rect> {
    painted(n).map(|b| b.inflate(1.0, 1.0))
}

type Cancelled = vectorcraft_doc::cancel::Cancelled;

/// Whether `b` has finite coordinates and isn't inside out.
pub(super) fn finite(b: &Rect) -> bool {
    [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) && b.x0 <= b.x1 && b.y0 <= b.y1
}

/// Whether boxes `a` and `b` overlap or touch.
fn touch(a: &Rect, b: &Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Indices joined into sets, each set knowing whether it holds an object with transparency.
struct Sets {
    parent: Vec<usize>,
    seeded: Vec<bool>,
}

impl Sets {
    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    /// Joins the sets of `a` and `b`: → the set they are now, named by its lowest index.
    fn join(&mut self, a: usize, b: usize) -> usize {
        let (a, b) = (self.find(a), self.find(b));
        let (lo, hi) = (a.min(b), a.max(b));
        self.parent[hi] = lo;
        self.seeded[lo] |= self.seeded[hi];
        lo
    }

    /// The sets, each in index order, by their first index.
    fn groups(&mut self) -> Vec<Vec<usize>> {
        let mut groups: Vec<Vec<usize>> = vec![];
        let mut slot: Vec<Option<usize>> = vec![None; self.parent.len()];
        for i in 0..self.parent.len() {
            let r = self.find(i);
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
pub(super) fn overlapping(bounds: &[Option<Rect>]) -> Result<Vec<Vec<usize>>, Cancelled> {
    Ok(sweep(bounds, None)?.groups())
}

/// [`overlapping`]'s sweep. With `see_through`, two boxes are joined only when one of them is
/// see-through, and an open box is closed early only inside a later see-through box.
fn sweep(bounds: &[Option<Rect>], see_through: Option<&[bool]>) -> Result<Sets, Cancelled> {
    let mut sets = Sets { parent: (0..bounds.len()).collect(), seeded: (0..bounds.len()).map(|i| see_through.is_some_and(|s| s[i])).collect() };
    let joins = |a: usize, b: usize| see_through.is_none_or(|s| s[a] || s[b]);
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
        // Whatever overlaps a box inside this one overlaps this one, and joins it when this one is
        // see-through.
        let shadows = see_through.is_none_or(|s| s[boxes[k].0]);
        for &o in &met {
            if joins(boxes[k].0, boxes[o].0) {
                sets.join(boxes[k].0, boxes[o].0);
            }
            let ob = boxes[o].1;
            if shadows && ob.x1 <= b.x1 && b.y0 <= ob.y0 && ob.y1 <= b.y1 {
                set(&mut low, leaf[o], f64::NEG_INFINITY);
            }
        }
        set(&mut low, leaf[k], b.y1);
    }
    Ok(sets)
}

/// Boxes in blocks of nearby ones, each block with the box around it, to find the boxes a box
/// touches.
struct Blocks {
    order: Vec<usize>,
    boxes: Vec<Option<Rect>>,
    around: Vec<Rect>,
}

impl Blocks {
    const SIZE: usize = 32;

    /// The finite boxes of `boxes`, in vertical strips by their centers and in blocks down each
    /// strip.
    fn new(boxes: &[Option<Rect>]) -> Self {
        let boxes: Vec<Option<Rect>> = boxes.iter().map(|b| b.filter(finite)).collect();
        let mut order: Vec<usize> = (0..boxes.len()).filter(|&i| boxes[i].is_some()).collect();
        let center = |i: usize| boxes[i].map_or_else(Default::default, |b| b.center());
        order.sort_by(|&a, &b| center(a).x.total_cmp(&center(b).x));
        let strip = ((order.len() as f64 / Self::SIZE as f64).sqrt().ceil() as usize).max(1) * Self::SIZE;
        for s in order.chunks_mut(strip) {
            s.sort_by(|&a, &b| center(a).y.total_cmp(&center(b).y));
        }
        let around = order.chunks(Self::SIZE).filter_map(|c| c.iter().filter_map(|&i| boxes[i]).reduce(|a, b| a.union(b))).collect();
        Self { order, boxes, around }
    }

    /// Puts the indices of the boxes that touch `q` in `out`. `work` counts the boxes looked at;
    /// fails only when cancelled.
    fn touching(&self, q: &Rect, out: &mut Vec<usize>, work: &mut usize) -> Result<(), Cancelled> {
        out.clear();
        for (b, around) in self.around.iter().enumerate() {
            *work += 1;
            if work.is_multiple_of(4096) {
                vectorcraft_doc::cancel::check()?;
            }
            if !touch(around, q) {
                continue;
            }
            for &i in &self.order[b * Self::SIZE..((b + 1) * Self::SIZE).min(self.order.len())] {
                *work += 1;
                if self.boxes[i].is_some_and(|r| touch(&r, q)) {
                    out.push(i);
                }
            }
        }
        Ok(())
    }
}

/// Objects (in paint order) grouped for flattening, each group in paint order and the groups by
/// their first object; a group takes the place of its topmost object. `reach` is where each
/// object can paint ([`reach`]), `painted` where it paints ([`painted`]), and `see_through` whether
/// it has transparency. An object whose box is missing or not finite is a group of its own.
///
/// An object joins every see-through object its reach touches. An object without transparency
/// that touches only others without transparency stays a group of its own, unless a group would
/// change its stacking order with it: it joins (and groups join each other) where its reach
/// touches a member's and moving the members to their group's top would put the two the other way
/// round. With a `fringe` (how far a group's image can reach past
/// its art and cover what is under it), an object or group under a group's top whose painted box
/// that far around a member's touches joins it too. Fails only when cancelled
/// ([`vectorcraft_doc::cancel`]).
pub(super) fn flatten_groups(
    reach: &[Option<Rect>],
    painted: &[Option<Rect>],
    see_through: &[bool],
    fringe: f64,
) -> Result<Vec<Vec<usize>>, Cancelled> {
    let n = reach.len();
    let mut sets = sweep(reach, Some(see_through))?;
    let mut members: Vec<Vec<usize>> = vec![vec![]; n];
    for i in 0..n {
        let r = sets.find(i);
        members[r].push(i);
    }
    // The topmost object of each set (by its name).
    let mut top: Vec<usize> = (0..n).map(|r| members[r].last().copied().unwrap_or(r)).collect();
    let index = Blocks::new(reach);
    // Each member of a set with transparency is looked at.
    let mut work: Vec<usize> = vec![];
    for i in 0..n {
        let r = sets.find(i);
        if sets.seeded[r] {
            work.push(i);
        }
    }
    let (mut near, mut looked) = (vec![], 0usize);
    while let Some(i) = work.pop() {
        let Some(q) = reach[i].filter(finite) else { continue };
        // Painted boxes within the fringe have reaches within it too.
        index.touching(&q.inflate(fringe, fringe), &mut near, &mut looked)?;
        for &k in &near {
            let (ri, rk) = (sets.find(i), sets.find(k));
            if ri == rk {
                continue;
            }
            let (ti, tk) = (top[ri], top[rk]);
            let crossed = reach[k].is_some_and(|b| touch(&q, &b)) && (i < k) != (ti < tk);
            let covered = fringe > 0.0 && tk < ti && painted[i].zip(painted[k]).is_some_and(|(a, b)| touch(&a.inflate(fringe, fringe), &b));
            if !crossed && !covered {
                continue;
            }
            // The members of a set whose top moves are looked at again, from there.
            let t = ti.max(tk);
            if ti != t {
                work.extend_from_slice(&members[ri]);
            }
            if tk != t {
                work.extend_from_slice(&members[rk]);
            }
            let r = sets.join(ri, rk);
            let moved = std::mem::take(&mut members[if r == ri { rk } else { ri }]);
            members[r].extend(moved);
            top[r] = t;
        }
    }
    Ok(sets.groups())
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

    /// A filled rectangle, unstroked.
    fn filled(id: u64, r: Rect) -> Node {
        let look = vectorcraft_doc::Appearance::basic(
            vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0)),
            vectorcraft_color::Paint::None,
            0.0,
        );
        Node::path(vectorcraft_doc::NodeId(id), vectorcraft_geom::shapes::rectangle(r), look)
    }

    /// The antialiasing margin goes around the outermost group once, not once per group.
    #[test]
    fn reach_adds_its_margin_once_however_deep_the_groups() {
        let r = Rect::new(10.0, 20.0, 30.0, 50.0);
        let inner = Node::group(vectorcraft_doc::NodeId(2), vec![Arc::new(filled(1, r))]);
        let outer = Node::group(vectorcraft_doc::NodeId(3), vec![Arc::new(inner)]);
        assert_eq!(painted(&outer), Some(r));
        assert_eq!(reach(&outer), Some(r.inflate(1.0, 1.0)));
        assert_eq!(reach(&filled(1, r)), reach(&outer));
    }

    /// The group of each object, the topmost object of each group and whether it is see-through.
    fn sets_of(groups: &[Vec<usize>], n: usize, see_through: &[bool]) -> (Vec<usize>, Vec<usize>, Vec<bool>) {
        let mut of = vec![usize::MAX; n];
        for (g, ids) in groups.iter().enumerate() {
            for &i in ids {
                of[i] = g;
            }
        }
        let top = groups.iter().map(|g| *g.last().unwrap()).collect();
        let seeded = groups.iter().map(|g| g.iter().any(|&i| see_through[i])).collect();
        (of, top, seeded)
    }

    /// What [`flatten_groups`] must give: every object in one group, in paint order, the groups by
    /// their first object; see-through objects with all that their reach touches; groups without
    /// transparency of one object; no two objects that paint over each other the other way round
    /// once each group takes its top's place; nothing within `fringe` of a member of a group under
    /// that group's top.
    fn check(reach: &[Option<Rect>], painted: &[Option<Rect>], see: &[bool], fringe: f64, groups: &[Vec<usize>], what: &str) {
        let n = reach.len();
        let mut all: Vec<usize> = groups.iter().flatten().copied().collect();
        all.sort_unstable();
        assert_eq!(all, (0..n).collect::<Vec<_>>(), "{what}: every object once");
        assert!(groups.iter().all(|g| g.is_sorted()) && groups.windows(2).all(|w| w[0][0] < w[1][0]), "{what}: in order");
        let (of, top, seeded) = sets_of(groups, n, see);
        for (g, ids) in groups.iter().enumerate() {
            assert!(seeded[g] || ids.len() == 1, "{what}: {ids:?} has no transparency");
        }
        for i in 0..n {
            for j in 0..n {
                if i == j || of[i] == of[j] {
                    continue;
                }
                let (gi, gj) = (of[i], of[j]);
                if let (Some(a), Some(b)) = (reach[i], reach[j]) {
                    assert!(!(touch(&a, &b) && (see[i] || see[j])), "{what}: {i} and {j} touch, one see-through");
                    if seeded[gi] && touch(&a, &b) {
                        assert_eq!(i < j, top[gi] < top[gj], "{what}: {i} and {j} change places");
                    }
                }
                let (Some(a), Some(b)) = (painted[i], painted[j]) else { continue };
                if seeded[gi] && fringe > 0.0 && touch(&a.inflate(fringe, fringe), &b) {
                    assert!(top[gj] > top[gi], "{what}: {j} under the image of {i}'s group");
                }
            }
        }
    }

    /// Boxes of many sizes, some see-through, group as they must, with and without a fringe.
    #[test]
    fn flatten_groups_keep_stacking_order_and_leave_opaque_art_alone() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |m: f64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1000) as f64 / 1000.0 * m
        };
        for round in 0..60 {
            let n = 4 + round;
            let painted: Vec<Option<Rect>> = (0..n)
                .map(|i| {
                    if i % 13 == 5 {
                        return None;
                    }
                    let (x, y) = (next(120.0).round(), next(120.0).round());
                    Some(Rect::new(x, y, x + next(25.0).round(), y + next(25.0).round()))
                })
                .collect();
            let reach: Vec<Option<Rect>> = painted.iter().map(|b| b.map(|b| b.inflate(1.0, 1.0))).collect();
            let see: Vec<bool> = (0..n).map(|_| next(1.0) < 0.15).collect();
            for fringe in [0.0, 0.7] {
                let groups = flatten_groups(&reach, &painted, &see, fringe).unwrap();
                check(&reach, &painted, &see, fringe, &groups, &format!("round {round}, fringe {fringe}"));
            }
        }
    }

    /// A row of opaque boxes whose reaches touch, with see-through ones at its ends and far from
    /// it: only the boxes next to a see-through one join it.
    #[test]
    fn an_opaque_row_stays_apart_from_the_see_through_boxes_at_its_ends() {
        let n = 200;
        // 10 pt boxes 1.5 pt apart: their 1 pt reaches overlap.
        let painted: Vec<Option<Rect>> = (0..n).map(|i| Some(Rect::new(11.5 * i as f64, 0.0, 11.5 * i as f64 + 10.0, 10.0))).collect();
        let reach: Vec<Option<Rect>> = painted.iter().map(|b| b.map(|b| b.inflate(1.0, 1.0))).collect();
        let mut see = vec![false; n];
        see[0] = true;
        see[n - 1] = true;
        let groups = flatten_groups(&reach, &painted, &see, 0.0).unwrap();
        check(&reach, &painted, &see, 0.0, &groups, "row");
        assert_eq!(groups[0], vec![0, 1]);
        assert_eq!(groups.last(), Some(&vec![n - 2, n - 1]));
        assert_eq!(groups.len(), n - 2);
        // Without transparency every box is a group of its own.
        assert_eq!(flatten_groups(&reach, &painted, &vec![false; n], 0.0).unwrap().len(), n);
    }

    /// An opaque box between a member and its group's top that paints over the member joins, and
    /// so on along a chain; one under the member stays, but for one within the fringe.
    #[test]
    fn boxes_the_group_would_cross_join_it() {
        // In paint order: D under A, A, B over A, C over B, then the see-through T over A.
        let d = Rect::new(40.0, 80.0, 65.0, 120.0);
        let a = Rect::new(60.0, 50.0, 105.0, 90.0);
        let b = Rect::new(20.0, 20.0, 65.0, 55.0);
        let c = Rect::new(0.0, 0.0, 25.0, 25.0);
        let t = Rect::new(100.0, 50.0, 140.0, 90.0);
        let painted = [Some(d), Some(a), Some(b), Some(c), Some(t)];
        let reach: Vec<Option<Rect>> = painted.iter().map(|b| b.map(|b| b.inflate(1.0, 1.0))).collect();
        let see = [false, false, false, false, true];
        let groups = flatten_groups(&reach, &painted, &see, 0.0).unwrap();
        assert_eq!(groups, vec![vec![0], vec![1, 2, 3, 4]]);
        // C away from B: it stays.
        let far = [Some(d), Some(a), Some(b), Some(Rect::new(0.0, 0.0, 10.0, 10.0)), Some(t)];
        let reach_far: Vec<Option<Rect>> = far.iter().map(|b| b.map(|b| b.inflate(1.0, 1.0))).collect();
        assert_eq!(flatten_groups(&reach_far, &far, &see, 0.0).unwrap(), vec![vec![0], vec![1, 2, 4], vec![3]]);
        // B 0.5 pt above A, not overlapping it: their antialiased edges can share pixels, so B joins.
        let gap = [Some(d), Some(a), Some(Rect::new(20.0, 20.0, 65.0, 49.5)), Some(Rect::new(0.0, 0.0, 10.0, 10.0)), Some(t)];
        let reach_gap: Vec<Option<Rect>> = gap.iter().map(|b| b.map(|b| b.inflate(1.0, 1.0))).collect();
        assert_eq!(flatten_groups(&reach_gap, &gap, &see, 0.0).unwrap(), vec![vec![0], vec![1, 2, 4], vec![3]]);
        // D a little left of A, under it: the fringe takes it in.
        let near = [Some(Rect::new(40.0, 80.0, 59.8, 120.0)), Some(a), Some(b), Some(c), Some(t)];
        let reach_near: Vec<Option<Rect>> = near.iter().map(|b| b.map(|b| b.inflate(1.0, 1.0))).collect();
        assert_eq!(flatten_groups(&reach_near, &near, &see, 0.0).unwrap()[0], vec![0]);
        assert_eq!(flatten_groups(&reach_near, &near, &see, 0.24).unwrap(), vec![vec![0, 1, 2, 3, 4]]);
    }
}
