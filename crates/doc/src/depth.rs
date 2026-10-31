//! How deep objects may nest.
//!
//! The tree is walked by recursion everywhere (drawing, bounds, hit tests, saving, exporting), so
//! a tree of unbounded depth overflows the stack: an abort no guard catches. No document deeper
//! than [`MAX_DEPTH`] is ever kept, nor one whose symbols and patterns draw deeper than
//! [`MAX_DRAWN_DEPTH`] (a symbol holding an instance of itself, or a pattern painting its own
//! tile, draws without end): importers and the native reader merge deeper objects into the group
//! around them and leave such instances and paints out
//! ([`Document::limit_depth`], with [`merged_note`] and [`symbols_note`] as their warnings), and
//! the engine refuses an edit that would nest deeper ([`Document::too_deep`]).

use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_color::Paint;

use crate::{Document, Node, NodeKind};

/// The deepest an object may be: a top-level layer is 1 deep, what it holds 2, and so on (the art
/// of an opacity mask is one deeper than the object it masks). A symbol's or pattern's art counts
/// from 1 again.
///
/// A native file writes three levels of JSON for each level of objects, and its reader stops at
/// 128 levels: 32 leaves the deepest object 29 of them for its own data, so every document that
/// can be made can be saved and opened again.
pub const MAX_DEPTH: usize = 32;

/// Most objects the symbol instances drawn in one frame or written in one export may hold
/// together. Instances of symbols that hold instances of others draw every level for each instance
/// above it, so a few levels of a few instances each multiply past what [`MAX_DEPTH`] stops; past
/// it, further instances aren't drawn.
pub const MAX_SYMBOL_NODES: u64 = 16_000_000;

/// The deepest an object may be drawn: as [`MAX_DEPTH`], the art of a symbol counted under its
/// instance and a pattern's tile under the object it paints (and what those use, under them).
/// Symbols a few objects deep nest in one another dozens deep within it.
pub const MAX_DRAWN_DEPTH: usize = 4 * MAX_DEPTH;

/// The warning of a file whose objects were nested deeper than [`MAX_DEPTH`].
pub fn merged_note() -> String {
    format!("objects nested more than {MAX_DEPTH} deep were merged into the group around them")
}

/// The warning of a file with symbols or patterns that use themselves or are drawn deeper than
/// [`MAX_DRAWN_DEPTH`].
pub fn symbols_note() -> String {
    format!(
        "symbol instances and pattern paints inside their own symbol or pattern, or drawn more than {MAX_DRAWN_DEPTH} objects deep, were left out"
    )
}

/// Run `f` where recursion has room: on a thread of its own with a 16 MB stack (the main thread
/// has 1 MB on Windows), or here where there are no threads (the web) or none can be started. For
/// readers whose recursion is capped but heavy; a panic in `f` goes on to the caller.
pub fn with_deep_stack<T: Send>(f: impl Fn() -> T + Sync) -> T {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let threaded = std::thread::scope(|s| {
            let thread = std::thread::Builder::new().stack_size(16 << 20).spawn_scoped(s, &f).ok()?;
            Some(thread.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
        });
        if let Some(done) = threaded {
            return done;
        }
    }
    f()
}

/// Is anything in `now` (each `depth` deep) deeper than [`MAX_DEPTH`], or `over` (given its
/// depth)? Without recursion: the trees may be of any depth. With `before` (the same list before
/// an edit, within the caps), only what the edit changed is looked at: a subtree still shared with
/// it, in the same place, is as it was.
fn deeper(now: &[Arc<Node>], before: Option<&[Arc<Node>]>, depth: usize, over: &dyn Fn(&Node, usize) -> bool) -> bool {
    type Pending<'a> = Vec<(&'a Node, Option<&'a Node>, usize)>;
    fn push<'a>(stack: &mut Pending<'a>, now: &'a [Arc<Node>], before: Option<&'a [Arc<Node>]>, depth: usize) {
        for (i, n) in now.iter().enumerate() {
            let was = before.and_then(|b| b.get(i));
            if !was.is_some_and(|w| Arc::ptr_eq(w, n)) {
                stack.push((n, was.map(|w| &**w), depth));
            }
        }
    }
    let mut stack = vec![];
    push(&mut stack, now, before, depth);
    while let Some((n, was, d)) = stack.pop() {
        if d > MAX_DEPTH || over(n, d) {
            return true;
        }
        if let Some(ch) = n.children() {
            push(&mut stack, ch, was.and_then(Node::children).map(Vec::as_slice), d + 1);
        }
        if let Some(m) = &n.extras().mask {
            let was = was.and_then(|w| w.extras().mask.as_ref()).map(|m| std::slice::from_ref(&m.art));
            push(&mut stack, std::slice::from_ref(&m.art), was, d + 1);
        }
    }
    false
}

/// [`deeper`] for the nesting alone.
fn nested_deeper(now: &[Arc<Node>], depth: usize) -> bool {
    deeper(now, None, depth, &|_, _| false)
}

/// Merge what is deeper than [`MAX_DEPTH`] in `nodes` (each `depth` deep) into the group around
/// it: a container at the cap is replaced by what it holds (a clipping group's clipping path and
/// an opacity mask too deep are left out); what it held stays hidden or locked as it was. Returns
/// whether anything changed. Recurses no deeper than the cap.
fn limit(nodes: &mut Vec<Arc<Node>>, depth: usize) -> bool {
    let mut changed = false;
    let mut i = 0;
    while let Some(n) = nodes.get_mut(i) {
        if !nested_deeper(std::slice::from_ref(n), depth) {
            i += 1;
            continue;
        }
        changed = true;
        let n = Arc::make_mut(n);
        if n.extras().mask.as_ref().is_some_and(|m| nested_deeper(std::slice::from_ref(&m.art), depth + 1)) {
            n.extras_mut().mask = None;
        }
        if depth < MAX_DEPTH {
            if let Some(ch) = n.children_mut() {
                limit(ch, depth + 1);
            }
            i += 1;
            continue;
        }
        let clip = matches!(n.kind, NodeKind::Group { clip: true, .. } | NodeKind::Layer { clip: true, .. });
        let (visible, locked) = (n.visible, n.locked);
        let mut held = n.children_mut().map(std::mem::take).unwrap_or_default();
        if clip && !held.is_empty() {
            held.remove(0);
        }
        for c in &mut held {
            if (!visible && c.visible) || (locked && !c.locked) {
                let c = Arc::make_mut(c);
                c.visible &= visible;
                c.locked |= locked;
            }
        }
        // What it held takes its place, and is looked at in its turn.
        nodes.splice(i..=i, held);
    }
    changed
}

/// A symbol (`false`) or a pattern (`true`) of the document, by name: what art draws by name, an
/// instance its symbol's art and a pattern paint its tile's.
type Def<'a> = (bool, &'a str);

/// How deep each symbol's and pattern's art is drawn, the art of the symbols and patterns it uses
/// included (drawn under the object that uses them): `None` for one that uses itself, in its art
/// or through what that uses, or is drawn deeper than [`MAX_DRAWN_DEPTH`].
type Heights<'a> = HashMap<Def<'a>, Option<usize>>;

/// What `n` itself draws by name: the symbol it is an instance of, the patterns that paint it.
fn uses(n: &Node) -> impl Iterator<Item = Def<'_>> {
    let symbol = match &n.kind {
        NodeKind::SymbolInstance { symbol, .. } => Some((false, symbol.as_str())),
        _ => None,
    };
    let patterns = crate::swatches::node_paints(n).filter_map(|p| match p {
        Paint::Pattern { pattern, .. } => Some((true, pattern.as_str())),
        _ => None,
    });
    symbol.into_iter().chain(patterns)
}

/// [`Heights`] of the document's symbols and patterns. Without recursion: they may use one
/// another in a chain of any length. Their own art is within [`MAX_DEPTH`].
fn heights(doc: &Document) -> Heights<'_> {
    // Each one's own depth, and what its art uses with the deepest use of each.
    type Own<'a> = (usize, Vec<(Def<'a>, usize)>);
    let arts = (doc.symbols.iter().map(|s| ((false, s.name.as_str()), std::slice::from_ref(&s.art))))
        .chain(doc.patterns.iter().map(|p| ((true, p.name.as_str()), &p.art[..])));
    let mut own: HashMap<Def, Own> = HashMap::new();
    for (def, art) in arts {
        let (mut deepest, mut used) = (0, HashMap::<Def, usize>::new());
        let mut stack: Vec<(&Node, usize)> = art.iter().map(|n| (&**n, 1)).collect();
        while let Some((n, d)) = stack.pop() {
            deepest = deepest.max(d);
            for u in uses(n) {
                let at = used.entry(u).or_default();
                *at = (*at).max(d);
            }
            stack.extend(n.children().into_iter().flatten().chain(n.extras().mask.as_ref().map(|m| &m.art)).map(|c| (&**c, d + 1)));
        }
        // (The first of two of one name is the one drawn.)
        own.entry(def).or_insert((deepest, used.into_iter().collect()));
    }
    // Depth first over what uses what. One met again while it is being worked out (`None` in its
    // entry) uses itself.
    let mut found: HashMap<Def, Option<Option<usize>>> = HashMap::new();
    let starts: Vec<Def> = own.keys().copied().collect();
    for start in starts {
        if found.contains_key(&start) {
            continue;
        }
        // (which, the next of its uses to look at, the deepest found so far: `None` once it can't be drawn)
        let mut path: Vec<(Def, usize, Option<usize>)> = vec![(start, 0, own.get(&start).map(|o| o.0))];
        found.insert(start, None);
        while let Some(&(def, next, deepest)) = path.last() {
            let used = own.get(&def).map_or(&[][..], |o| &o.1);
            let Some(&(other, at)) = used.get(next) else {
                // Everything it uses is worked out.
                let deepest = deepest.filter(|d| *d <= MAX_DRAWN_DEPTH);
                found.insert(def, Some(deepest));
                path.pop();
                if let Some((user, next, held)) = path.last_mut() {
                    let at = own.get(user).and_then(|o| o.1.get(next.wrapping_sub(1))).map_or(1, |u| u.1);
                    *held = held.zip(deepest).map(|(h, d)| h.max(at + d));
                }
                continue;
            };
            let known = found.get(&other).copied();
            if known.is_none() && own.contains_key(&other) {
                found.insert(other, None);
            }
            let Some((_, next, deepest)) = path.last_mut() else { break };
            *next += 1;
            match known {
                // Not in the document: it draws nothing.
                None if !own.contains_key(&other) => {}
                None => path.push((other, 0, own.get(&other).map(|o| o.0))),
                Some(Some(Some(d))) => *deepest = deepest.map(|h| h.max(at + d)),
                // It uses itself (it is on the way down here), or can't be drawn.
                Some(None | Some(None)) => *deepest = None,
            }
        }
    }
    found.into_iter().map(|(def, h)| (def, h.flatten())).collect()
}

/// Can't `used`, by an object `depth` deep, be drawn there?
fn undrawable(heights: &Heights, used: Def, depth: usize) -> bool {
    heights.get(&used).is_some_and(|h| h.is_none_or(|h| depth + h > MAX_DRAWN_DEPTH))
}

/// Take out of `nodes` (each `depth` deep) the uses `bad` picks (given the depth of the object):
/// an instance goes, a pattern paint becomes no paint. Returns whether anything changed. For trees
/// within [`MAX_DEPTH`].
fn strip(nodes: &mut Vec<Arc<Node>>, depth: usize, bad: &dyn Fn(Def, usize) -> bool) -> bool {
    let any = |n: &Node, d: usize| uses(n).any(|u| bad(u, d));
    let before = nodes.len();
    nodes.retain(|n| !matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if bad((false, symbol.as_str()), depth)));
    let mut changed = nodes.len() != before;
    for n in nodes.iter_mut() {
        if !deeper(std::slice::from_ref(n), None, depth, &any) {
            continue;
        }
        changed = true;
        let n = Arc::make_mut(n);
        if uses(n).any(|u| bad(u, depth)) {
            for p in crate::swatches::node_paints_mut(n) {
                if matches!(p, Paint::Pattern { pattern, .. } if bad((true, pattern.as_str()), depth)) {
                    *p = Paint::None;
                }
            }
        }
        // An opacity mask whose art is such an instance goes with it.
        let mut mask = n.extras().mask.as_ref().map(|m| vec![m.art.clone()]);
        if let Some(art) = &mut mask
            && strip(art, depth + 1, bad)
        {
            match art.pop() {
                Some(art) => {
                    if let Some(m) = n.extras_mut().mask.as_mut() {
                        m.art = art;
                    }
                }
                None => n.extras_mut().mask = None,
            }
        }
        if let Some(ch) = n.children_mut() {
            strip(ch, depth + 1, bad);
        }
    }
    changed
}

impl Document {
    /// Is anything nested deeper than [`MAX_DEPTH`], or drawn deeper than [`MAX_DRAWN_DEPTH`]
    /// (a symbol or pattern that uses itself is)? With `before` (this document before an edit,
    /// itself within the caps), only what the edit changed is looked at.
    pub fn too_deep(&self, before: Option<&Document>) -> bool {
        fn symbol(d: &Document, i: usize) -> Option<&[Arc<Node>]> {
            d.symbols.get(i).map(|s| std::slice::from_ref(&s.art))
        }
        fn pattern(d: &Document, i: usize) -> Option<&[Arc<Node>]> {
            d.patterns.get(i).map(|p| &p.art[..])
        }
        let nested = |_: &Node, _: usize| false;
        if (0..self.symbols.len()).any(|i| symbol(self, i).is_some_and(|art| deeper(art, before.and_then(|b| symbol(b, i)), 1, &nested)))
            || (0..self.patterns.len()).any(|i| pattern(self, i).is_some_and(|art| deeper(art, before.and_then(|b| pattern(b, i)), 1, &nested)))
        {
            return true;
        }
        if self.symbols.is_empty() && self.patterns.is_empty() {
            return deeper(&self.layers, before.map(|b| &b.layers[..]), 1, &nested);
        }
        let heights = heights(self);
        if heights.values().any(Option::is_none) {
            return true;
        }
        // Symbols and patterns draw as deep as before when none changed; else every use of them
        // is looked at again.
        let same = |x: &[Arc<Node>], y: &[Arc<Node>]| x.len() == y.len() && x.iter().zip(y).all(|(x, y)| Arc::ptr_eq(x, y));
        let before = before.filter(|b| {
            b.symbols.len() == self.symbols.len()
                && b.patterns.len() == self.patterns.len()
                && b.symbols.iter().zip(&self.symbols).all(|(x, y)| x.name == y.name && Arc::ptr_eq(&x.art, &y.art))
                && b.patterns.iter().zip(&self.patterns).all(|(x, y)| x.name == y.name && same(&x.art, &y.art))
        });
        let over = |n: &Node, d: usize| uses(n).any(|u| undrawable(&heights, u, d));
        deeper(&self.layers, before.map(|b| &b.layers[..]), 1, &over)
    }

    /// [`Self::limit_depth`] → whether objects were merged, and whether uses of symbols and
    /// patterns were left out.
    fn limit_depths(&mut self) -> (bool, bool) {
        let mut merged = limit(&mut self.layers, 1);
        for s in &mut self.symbols {
            // The art itself is 1 deep, so it stays the one object it is.
            let mut art = vec![s.art.clone()];
            if limit(&mut art, 1)
                && let Some(one) = art.pop()
            {
                s.art = one;
                merged = true;
            }
        }
        for p in &mut self.patterns {
            merged |= limit(&mut p.art, 1);
        }
        if self.symbols.is_empty() && self.patterns.is_empty() {
            return (merged, false);
        }
        // The symbols and patterns that can't be drawn lose what their art uses: each is then as
        // deep as its own art, and what uses it can be drawn.
        let (symbols, patterns): (Vec<usize>, Vec<usize>) = {
            let heights = heights(self);
            let bad = |def: Def| heights.get(&def).is_some_and(Option::is_none);
            (
                (0..self.symbols.len()).filter(|i| self.symbols.get(*i).is_some_and(|s| bad((false, s.name.as_str())))).collect(),
                (0..self.patterns.len()).filter(|i| self.patterns.get(*i).is_some_and(|p| bad((true, p.name.as_str())))).collect(),
            )
        };
        let mut left_out = !(symbols.is_empty() && patterns.is_empty());
        let all = |_: Def, _: usize| true;
        for i in symbols {
            let Some(s) = self.symbols.get_mut(i) else { continue };
            let mut art = vec![s.art.clone()];
            strip(&mut art, 1, &all);
            // (An instance alone was the whole art: an empty group stands in.)
            s.art = art.pop().unwrap_or_else(|| Arc::new(Node::group(s.art.id, vec![])));
        }
        for i in patterns {
            if let Some(p) = self.patterns.get_mut(i) {
                strip(&mut p.art, 1, &all);
            }
        }
        // Then the uses drawn too deep where they are.
        let mut layers = self.layers.clone();
        left_out |= {
            let heights = heights(self);
            strip(&mut layers, 1, &|used, depth| undrawable(&heights, used, depth))
        };
        self.layers = layers;
        (merged, left_out)
    }

    /// Bring the document within the caps: merge the objects nested deeper than [`MAX_DEPTH`]
    /// into the group around them, and leave out the uses of symbols and patterns that can't be
    /// drawn (inside their own symbol or pattern, or deeper than [`MAX_DRAWN_DEPTH`]). What an
    /// importer does to what it read. Returns whether anything changed.
    pub fn limit_depth(&mut self) -> bool {
        let (merged, left_out) = self.limit_depths();
        merged || left_out
    }

    /// [`Self::limit_depth`], adding [`merged_note`] and [`symbols_note`] to an importer's
    /// `warnings` for what it did.
    pub fn limit_depth_noting(&mut self, warnings: &mut Vec<String>) {
        let (merged, left_out) = self.limit_depths();
        warnings.extend(merged.then(merged_note));
        warnings.extend(left_out.then(symbols_note));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeId, OpacityMask};

    /// One path in `levels` groups, each holding a second path beside the next group.
    fn nested(levels: usize) -> Arc<Node> {
        let mut id = 0;
        let mut next = || {
            id += 1;
            NodeId(id)
        };
        let path = |id| Arc::new(Node::path(id, Default::default(), Default::default()));
        let mut inner = path(next());
        for _ in 0..levels {
            inner = Arc::new(Node::group(next(), vec![inner, path(next())]));
        }
        inner
    }

    fn doc(art: Arc<Node>) -> Document {
        let mut d = Document::new(100.0, 100.0);
        if let Some(ch) = d.layers.first_mut().and_then(|l| Arc::make_mut(l).children_mut()) {
            ch.push(art);
        }
        d
    }

    #[test]
    fn a_document_at_the_cap_is_left_alone() {
        // The layer, the groups and the path in them: MAX_DEPTH deep.
        let mut d = doc(nested(MAX_DEPTH - 2));
        let before = d.clone();
        assert!(!d.too_deep(None));
        assert!(!d.limit_depth());
        assert_eq!(d, before);
    }

    #[test]
    fn deeper_objects_are_merged_into_the_group_around_them() {
        for levels in [MAX_DEPTH - 1, MAX_DEPTH + 5, 100_000] {
            let mut d = doc(nested(levels));
            assert!(d.too_deep(None), "{levels}");
            assert!(d.limit_depth(), "{levels}");
            assert!(!d.too_deep(None), "{levels}");
            // Every path is kept, under the layer and the groups down to the cap.
            assert_eq!(d.node_count(), 1 + (MAX_DEPTH - 2) + (levels + 1), "{levels}");
            assert!(!d.limit_depth(), "nothing more to merge");
        }
    }

    #[test]
    fn a_clipping_path_and_a_mask_too_deep_are_left_out_and_hidden_art_stays_hidden() {
        let mut inner = Node::new(NodeId(9000), NodeKind::Group { children: vec![nested(0), nested(0)], clip: true });
        inner.visible = false;
        let mut art = Arc::new(inner);
        for i in 0..MAX_DEPTH {
            art = Arc::new(Node::group(NodeId(9001 + i as u64), vec![art]));
        }
        let mut masked = Node::group(NodeId(8000), vec![]);
        masked.extras_mut().mask = Some(Box::new(OpacityMask::new((*nested(MAX_DEPTH)).clone(), true)));
        let mut d = doc(art);
        if let Some(ch) = d.layers.first_mut().and_then(|l| Arc::make_mut(l).children_mut()) {
            ch.push(Arc::new(masked));
        }
        assert!(d.limit_depth());
        assert!(!d.too_deep(None));
        let mut leaves = vec![];
        d.walk(|n| {
            if n.children().is_none() {
                leaves.push(n.visible);
            }
        });
        assert_eq!(leaves, [false], "the clipping path is gone, what it clipped is still hidden");
        assert!(d.node(NodeId(8000)).is_some_and(|n| n.extras().mask.is_none()));
    }

    #[test]
    fn an_edit_is_checked_by_what_it_changed() {
        let before = doc(nested(MAX_DEPTH - 2));
        let mut wrapped = before.clone();
        if let Some(ch) = wrapped.layers.first_mut().and_then(|l| Arc::make_mut(l).children_mut()) {
            let all = std::mem::take(ch);
            ch.push(Arc::new(Node::group(NodeId(7000), all)));
        }
        assert!(wrapped.too_deep(Some(&before)), "the same art, one deeper");
        let mut beside = before.clone();
        if let Some(ch) = beside.layers.first_mut().and_then(|l| Arc::make_mut(l).children_mut()) {
            ch.push(nested(3));
        }
        assert!(!beside.too_deep(Some(&before)));
        assert!(!before.too_deep(Some(&before)));
    }

    #[test]
    fn symbols_and_patterns_are_limited_too() {
        let mut d = Document::new(10.0, 10.0);
        d.symbols.push(crate::Symbol { name: "s".into(), art: nested(MAX_DEPTH + 3) });
        assert!(d.too_deep(None));
        assert!(d.limit_depth());
        assert!(!d.too_deep(None));
        assert!(d.symbols.first().is_some_and(|s| s.art.children().is_some()), "still one object");
    }

    fn instance(id: u64, of: &str) -> Arc<Node> {
        Arc::new(Node::new(NodeId(id), NodeKind::SymbolInstance { symbol: of.into(), xf: Default::default() }))
    }

    /// `count` symbols, each `levels` groups around an instance of the next; the last one's of `last`.
    fn symbols(count: usize, levels: usize, last: &str) -> Document {
        let mut d = doc(instance(1, "s0"));
        for i in 0..count {
            let mut art = instance(2, &if i + 1 < count { format!("s{}", i + 1) } else { last.to_string() });
            for _ in 0..levels {
                art = Arc::new(Node::group(NodeId(3), vec![art]));
            }
            d.symbols.push(crate::Symbol { name: format!("s{i}"), art });
        }
        d
    }

    fn instances(d: &Document) -> usize {
        let mut n = 0;
        let mut count = |x: &Node| n += usize::from(matches!(x.kind, NodeKind::SymbolInstance { .. }));
        d.walk(&mut count);
        d.symbols.iter().for_each(|s| s.art.walk(&mut count));
        n
    }

    #[test]
    fn symbols_in_symbols_are_drawn_only_so_deep() {
        // Three objects deep each (two groups and the instance), under the instance on the layer.
        let fits = MAX_DRAWN_DEPTH / 3 - 1;
        let mut d = symbols(fits, 2, "none");
        assert!(!d.too_deep(None) && !d.limit_depth());
        let mut d = symbols(fits + 2, 2, "none");
        assert!(d.too_deep(None));
        let before = instances(&d);
        assert!(d.limit_depth() && !d.too_deep(None));
        assert!(instances(&d) < before && instances(&d) > 0, "only the instances drawn too deep go");
        // A chain of any length is measured, not walked by recursion.
        let mut d = symbols(100_000, 0, "none");
        assert!(d.too_deep(None) && d.limit_depth() && !d.too_deep(None));
    }

    #[test]
    fn a_symbol_holding_itself_loses_the_instance() {
        for (count, last) in [(1, "s0"), (3, "s0"), (3, "s1")] {
            let mut d = symbols(count, 1, last);
            assert!(d.too_deep(None), "{count} {last}");
            let mut warnings = vec![];
            d.limit_depth_noting(&mut warnings);
            assert_eq!(warnings, [symbols_note()]);
            assert!(!d.too_deep(None), "{count} {last}");
            assert!(d.symbols.iter().all(|s| s.art.children().is_some()), "the art is still a group");
        }
        // An edit that makes one hold itself is seen, whatever else stayed as it was.
        let before = symbols(2, 1, "none");
        let mut after = before.clone();
        after.symbols[1].art = Arc::new(Node::group(NodeId(9), vec![instance(8, "s0")]));
        assert!(!before.too_deep(None) && after.too_deep(Some(&before)));
    }

    /// A path painted with the pattern named `with`.
    fn painted(id: u64, with: &str) -> Arc<Node> {
        let fill = Paint::Pattern { pattern: with.into(), xf: Default::default() };
        Arc::new(Node::path(NodeId(id), Default::default(), crate::Appearance::basic(fill, Paint::None, 0.0)))
    }

    fn pattern_fills(d: &Document) -> usize {
        let mut n = 0;
        d.visit_paints(&mut |p| n += usize::from(matches!(p, Paint::Pattern { .. })));
        n
    }

    #[test]
    fn a_pattern_painting_its_own_tile_loses_the_paint() {
        // One that paints its own tile; two that paint each other's; one in a symbol it holds.
        let itself = |d: &mut Document| d.patterns.push(crate::PatternDef::new("p", vec![painted(2, "p")]));
        let each_other = |d: &mut Document| {
            d.patterns.push(crate::PatternDef::new("p", vec![painted(2, "q")]));
            d.patterns.push(crate::PatternDef::new("q", vec![painted(3, "p")]));
        };
        let through_a_symbol = |d: &mut Document| {
            d.patterns.push(crate::PatternDef::new("p", vec![instance(2, "s")]));
            d.symbols.push(crate::Symbol { name: "s".into(), art: painted(3, "p") });
        };
        for make in [&itself as &dyn Fn(&mut Document), &each_other, &through_a_symbol] {
            let mut d = doc(painted(1, "p"));
            make(&mut d);
            assert!(d.too_deep(None));
            let mut warnings = vec![];
            d.limit_depth_noting(&mut warnings);
            assert_eq!(warnings, [symbols_note()]);
            assert!(!d.too_deep(None));
            assert_eq!(pattern_fills(&d), 1, "the object on the layer keeps its pattern");
        }
        // A pattern whose tile is painted with another is fine.
        let mut d = doc(painted(1, "p"));
        d.patterns.push(crate::PatternDef::new("p", vec![painted(2, "q")]));
        d.patterns.push(crate::PatternDef::new("q", vec![painted(3, "none")]));
        assert!(!d.too_deep(None) && !d.limit_depth());
    }
}
