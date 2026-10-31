//! Symbol instances measured by their symbol's art: [`SymbolBoxes`] looks each symbol's art up in
//! the document and measures it once, and an instance's bounds are its art's box placed by the
//! instance's transform ([`Node::geometric_bounds_with`]).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use vectorcraft_geom::{Affine, Rect};

use crate::{AppearanceItem, Document, Node, NodeKind, SYMBOL_HALF};

/// The box a symbol instance is measured by when its symbol's art has no box
/// ([`SymbolBoxes::art`]), and in [`Node::geometric_bounds`]: the 20 × 20 square around the origin
/// that symbols made in the app keep their art in.
pub(crate) const SYMBOL_SQUARE: Rect = Rect::new(-SYMBOL_HALF, -SYMBOL_HALF, SYMBOL_HALF, SYMBOL_HALF);

/// Instances of a symbol whose art holds symbols nested this many deep are measured by
/// [`SYMBOL_SQUARE`] instead of their art.
const MAX_DEPTH: usize = 32;

/// Most effect reaches one set of boxes measures for transforms that place the art of symbols
/// holding instances: past it, an estimate stands in ([`SymbolBoxes::effects_reach`]).
const MAX_REACHES: usize = 4096;

/// A symbol's art measured in symbol space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ArtBox {
    /// The art's geometric bounds.
    pub(crate) geometric: Rect,
    /// The art's visual bounds (its strokes included), never smaller than `geometric`.
    pub(crate) visual: Rect,
    /// The farthest any stroke in the art can paint from its path ([`crate::Appearance::outset`]).
    pub(crate) reach: f64,
    /// Whether the art, or the art of a symbol an instance in it shows, has a visible effect.
    pub(crate) effects: bool,
    /// The widest stroke of the objects with a visible brushed stroke in the art, or in the art
    /// of a symbol an instance in it shows: `None` when no object there has one.
    pub(crate) brush: Option<f64>,
}

/// The art boxes of a document's symbols, which [`Node::geometric_bounds_with`],
/// [`Node::visual_bounds_with`] and [`Node::reach_bounds_with`] measure symbol instances by. Each
/// symbol's art is measured the first time an instance of it is measured; make a new one after the
/// symbols change.
pub struct SymbolBoxes<'a> {
    doc: &'a Document,
    /// The art boxes measured so far, by symbol name (`None` for a symbol whose instances are
    /// measured by [`SYMBOL_SQUARE`]).
    measured: RefCell<HashMap<String, Option<ArtBox>>>,
    /// How deep symbols nest in the art of each symbol checked so far, by symbol name
    /// ([`Self::depth`]).
    depths: RefCell<HashMap<String, Option<usize>>>,
    /// Set while symbol art or a click's box is measured, and always in boxes made by
    /// [`Self::quick`]: instances are then measured by their art boxes alone
    /// ([`Self::placed_visual`] returns `None`).
    quick: Cell<bool>,
    /// Set in boxes made by [`Self::with_brush_art`], and cleared while symbol art is measured
    /// ([`Self::brush_pad`]).
    brush_art: Cell<bool>,
    /// Cleared in boxes made by [`Self::without_art_strokes`], and set while symbol art is
    /// measured ([`Self::art_strokes`]).
    art_strokes: Cell<bool>,
    /// The effect reaches measured so far, by symbol name and the bits of the linear part of the
    /// transform that places the art ([`Self::effects_reach`]).
    reaches: RefCell<HashMap<(String, [u64; 4]), f64>>,
    /// How many of them are of symbols whose art holds instances.
    holders: Cell<usize>,
    /// The farthest effect reach measured so far for each symbol.
    farthest: RefCell<HashMap<String, f64>>,
    /// The effect reach of each symbol's art with no transform, measured past [`MAX_REACHES`]
    /// ([`Self::unplaced_reach`]).
    unplaced: RefCell<HashMap<String, f64>>,
    /// How many times [`Self::measure_art`] has run a measure on symbol art ([`Self::art_measured`]).
    runs: Cell<usize>,
}

impl Document {
    /// [`SymbolBoxes`] for this document's symbols, none measured yet.
    pub fn symbol_boxes(&self) -> SymbolBoxes<'_> {
        SymbolBoxes {
            doc: self,
            measured: RefCell::default(),
            depths: RefCell::default(),
            quick: Cell::new(false),
            brush_art: Cell::new(false),
            art_strokes: Cell::new(true),
            reaches: RefCell::default(),
            holders: Cell::new(0),
            farthest: RefCell::default(),
            unplaced: RefCell::default(),
            runs: Cell::new(0),
        }
    }
}

impl<'a> SymbolBoxes<'a> {
    /// The document whose symbols these are.
    pub fn document(&self) -> &'a Document {
        self.doc
    }

    /// Makes these boxes measure every symbol instance as [`Node::reach_bounds_with`] does, by its
    /// art box alone. An instance that scales or turns stroked art then measures a little larger
    /// than with [`Node::visual_bounds_with`], and its art is not transformed to measure it.
    pub fn quick(self) -> Self {
        self.quick.set(true);
        self
    }

    /// Makes these boxes grow the visual box of each symbol instance whose art holds brushed
    /// strokes by as far as their brush art can paint past it ([`Self::brush_pad`]). The visual
    /// boxes of the groups, layers and live objects that hold such an instance grow with it.
    pub fn with_brush_art(self) -> Self {
        self.brush_art.set(true);
        self
    }

    /// Makes these boxes measure the visual box of each symbol instance without the strokes in
    /// its art: the art's geometric box placed by the instance's transform, grown by the
    /// instance's own strokes. The strokes in the art keep their weight when the instance scales.
    /// The visual boxes of the groups, layers and live objects that hold an instance do not include
    /// those strokes either.
    pub fn without_art_strokes(self) -> Self {
        self.art_strokes.set(false);
        self
    }

    /// Whether instances are measured with the strokes in their art: false in boxes made by
    /// [`Self::without_art_strokes`], except while symbol art is measured.
    pub(crate) fn art_strokes(&self) -> bool {
        self.art_strokes.get()
    }

    /// The art box of the symbol named `name`, with the instances in the art measured by their own
    /// art boxes. Returns `None` when [`Self::depth`] is `None` and when the art has no finite
    /// bounds: instances of the symbol are then measured by [`SYMBOL_SQUARE`].
    pub(crate) fn art(&self, name: &str) -> Option<ArtBox> {
        if let Some(b) = self.measured.borrow().get(name) {
            return *b;
        }
        let b = self.measure_art(name, |art| self.in_art(|| art_box(art, self))).flatten();
        self.measured.borrow_mut().insert(name.to_string(), b);
        b
    }

    /// Runs `measure` on the art of the symbol named `name`. Returns `None` when [`Self::depth`]
    /// is `None`, without running it.
    pub fn measure_art<R>(&self, name: &str, measure: impl FnOnce(&Arc<Node>) -> R) -> Option<R> {
        self.depth(name)?;
        let sym = self.doc.symbols.iter().find(|s| s.name == name)?;
        self.runs.set(self.runs.get() + 1);
        Some(measure(&sym.art))
    }

    /// How many times [`Self::measure_art`] has run a measure on symbol art for these boxes, the
    /// measures behind the cached art boxes and placed visual boxes included.
    pub fn art_measured(&self) -> usize {
        self.runs.get()
    }

    /// The geometric box of the art of the symbol named `name` in symbol space, with the instances
    /// in the art measured by their own art boxes. `None` when [`Self::art`] is `None`: instances of
    /// the symbol are then measured by [`SYMBOL_SQUARE`].
    pub fn art_bounds(&self, name: &str) -> Option<Rect> {
        self.art(name).map(|a| a.geometric)
    }

    /// Whether the art of the symbol named `name`, or of a symbol an instance in it shows, has a
    /// visible effect (raster or geometry, the object's own or a fill's or stroke's). False when
    /// [`Self::art`] is `None`.
    pub fn has_effects(&self, name: &str) -> bool {
        self.art(name).is_some_and(|a| a.effects)
    }

    /// How far the effects in the art of the symbol named `name` paint past that art when a
    /// transform with the linear part `linear` places it, as `measure` measures it for a linear
    /// part. `measure` runs once for each symbol and linear part. For symbols whose art holds
    /// instances, it runs [`MAX_REACHES`] times at most for these boxes. After that, a linear part
    /// not measured yet takes the reach with no transform, times √2 and the larger of 1 and the
    /// linear part's Frobenius norm, or the farthest reach measured for the symbol when that is
    /// farther; `measure` runs once more for each of those symbols whose reach with no transform
    /// was not measured yet. The estimate holds for effects that paint a fixed distance past the
    /// art, or a distance in proportion to its size.
    pub fn effects_reach(&self, name: &str, linear: [f64; 4], measure: &dyn Fn([f64; 4]) -> f64) -> f64 {
        let key = (name.to_string(), linear.map(f64::to_bits));
        if let Some(r) = self.reaches.borrow().get(&key) {
            return *r;
        }
        let holder = self.depth(name) != Some(0);
        if holder && self.holders.get() >= MAX_REACHES {
            let norm = linear.iter().map(|v| v * v).sum::<f64>().sqrt();
            let estimate = self.unplaced_reach(name, measure) * std::f64::consts::SQRT_2 * norm.max(1.0);
            return self.farthest.borrow().get(name).map_or(estimate, |f| f.max(estimate));
        }
        if holder {
            self.holders.set(self.holders.get() + 1);
        }
        let r = measure(linear);
        self.reaches.borrow_mut().insert(key, r);
        let mut farthest = self.farthest.borrow_mut();
        let f = farthest.entry(name.to_string()).or_insert(r);
        *f = f.max(r);
        r
    }

    /// [`Self::effects_reach`] with no transform, measured once for each symbol, also past
    /// [`MAX_REACHES`].
    fn unplaced_reach(&self, name: &str, measure: &dyn Fn([f64; 4]) -> f64) -> f64 {
        const IDENTITY: [f64; 4] = [1.0, 0.0, 0.0, 1.0];
        if let Some(r) = self.reaches.borrow().get(&(name.to_string(), IDENTITY.map(f64::to_bits))) {
            return *r;
        }
        if let Some(r) = self.unplaced.borrow().get(name) {
            return *r;
        }
        let r = measure(IDENTITY);
        self.unplaced.borrow_mut().insert(name.to_string(), r);
        r
    }

    /// The widest stroke of the objects with a visible brushed stroke in the art of the symbol
    /// named `name`, or in the art of a symbol an instance in it shows. `None` when no object there
    /// has one, and when [`Self::art`] is `None`.
    pub fn brush_width(&self, name: &str) -> Option<f64> {
        self.art(name)?.brush
    }

    /// How far the visual box of an instance of the symbol named `name` grows in boxes made by
    /// [`Self::with_brush_art`]: `16 × max(w, 1) + 8` for the widest stroke `w` that
    /// [`Self::brush_width`] finds, as far as the renderer grows the cull bounds of a brushed path
    /// (`brush_fx::cull_bounds` in vectorcraft-render). 0 in other boxes, while symbol art is
    /// measured, and when the art has no brushed stroke.
    pub(crate) fn brush_pad(&self, name: &str) -> f64 {
        if !self.brush_art.get() {
            return 0.0;
        }
        self.brush_width(name).map_or(0.0, |w| 16.0 * w.max(1.0) + 8.0)
    }

    /// How deep symbols nest in the art of the symbol named `name`: 0 when the art holds no
    /// instance of a symbol in the document, 1 when the symbols its instances show hold none, and
    /// so on. Returns `None` when the document has no such symbol, when the symbol or a symbol
    /// nested in its art holds an instance of itself at any depth, and when symbols nest
    /// [`MAX_DEPTH`] deep: instances of the symbol are then measured by [`SYMBOL_SQUARE`]. The
    /// answer depends only on the document's symbols.
    fn depth(&self, name: &str) -> Option<usize> {
        if let Some(d) = self.depths.borrow().get(name) {
            return *d;
        }
        let mut learned = HashMap::new();
        let d = self.find_depth(name, &mut learned);
        let mut depths = self.depths.borrow_mut();
        depths.extend(learned);
        depths.insert(name.to_string(), d);
        d
    }

    /// Finds [`Self::depth`] for the symbol named `name` by walking the symbols its art's instances
    /// show, depth first, without recursing. `learned` collects what the walk finds out about the
    /// other symbols it reaches: the depth of each one it walks to the end, and `None` for those on
    /// the way down to a symbol that holds an instance of itself or nests too deep.
    fn find_depth(&self, name: &str, learned: &mut HashMap<String, Option<usize>>) -> Option<usize> {
        let art = |n: &str| self.doc.symbols.iter().find(|s| s.name == n).map(|s| &s.art);
        // The symbols on the way down from `name`: each with the symbols its art shows that are
        // still to be walked, and the deepest nesting found under it so far.
        let mut path: Vec<(String, Vec<String>, usize)> = vec![(name.to_string(), shown(self.doc, art(name)?), 0)];
        loop {
            let (_, todo, _) = path.last_mut()?;
            let Some(next) = todo.pop() else {
                // Everything under it is walked.
                let (done, _, d) = path.pop()?;
                if d >= MAX_DEPTH {
                    learned.insert(done, None);
                    return give_up(&path, learned);
                }
                learned.insert(done, Some(d));
                let Some((_, _, up)) = path.last_mut() else { return Some(d) };
                *up = (*up).max(d + 1);
                continue;
            };
            let known = learned.get(&next).copied().or_else(|| self.depths.borrow().get(&next).copied());
            match known {
                Some(Some(d)) => {
                    if let Some((_, _, deepest)) = path.last_mut() {
                        *deepest = (*deepest).max(d + 1);
                    }
                }
                // It holds an instance of itself or nests too deep, and so does every symbol on
                // the way down to it.
                Some(None) => return give_up(&path, learned),
                // A symbol that holds an instance of itself.
                None if path.iter().any(|(n, ..)| *n == next) => return give_up(&path, learned),
                // Symbols nested MAX_DEPTH deep under `name`.
                None if path.len() >= MAX_DEPTH => return None,
                None => {
                    if let Some(a) = art(&next) {
                        let todo = shown(self.doc, a);
                        path.push((next, todo, 0));
                    }
                }
            }
        }
    }

    /// The visual bounds of symbol `name`'s art drawn as an instance with transform `xf` draws it,
    /// its strokes at their own weight, the instances in it measured by their art boxes. `None`
    /// when [`Self::depth`] is `None`, in boxes made by [`Self::quick`], and while symbol art or a
    /// click's box is measured.
    pub(crate) fn placed_visual(&self, name: &str, xf: Affine) -> Option<Rect> {
        if self.quick.get() {
            return None;
        }
        self.measure_art(name, |art| {
            let mut art = (**art).clone();
            art.transform(xf, false);
            self.in_art(|| art.visual(Some(self)))
        })
        .flatten()
    }

    /// Runs `f` with instances measured by their art boxes alone.
    pub(crate) fn quickly<R>(&self, f: impl FnOnce() -> R) -> R {
        let was = self.quick.replace(true);
        let r = f();
        self.quick.set(was);
        r
    }

    /// Runs `f` as symbol art is measured: instances measured by their art boxes alone, with the
    /// strokes in their art, and without [`Self::brush_pad`], which the instance that shows the
    /// art adds once.
    fn in_art<R>(&self, f: impl FnOnce() -> R) -> R {
        let pad = self.brush_art.replace(false);
        let strokes = self.art_strokes.replace(true);
        let r = self.quickly(f);
        self.brush_art.set(pad);
        self.art_strokes.set(strokes);
        r
    }
}

/// Records every symbol on `path` in `learned` as one whose instances are measured by
/// [`SYMBOL_SQUARE`]. Returns `None`.
fn give_up(path: &[(String, Vec<String>, usize)], learned: &mut HashMap<String, Option<usize>>) -> Option<usize> {
    for (n, ..) in path {
        learned.insert(n.clone(), None);
    }
    None
}

/// The names of the symbols of `doc` that the instances in `art` show, each once.
fn shown(doc: &Document, art: &Node) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = vec![];
    art.walk(&mut |n| {
        if let NodeKind::SymbolInstance { symbol, .. } = &n.kind
            && seen.insert(symbol.as_str())
            && doc.symbols.iter().any(|s| s.name == *symbol)
        {
            names.push(symbol.clone());
        }
    });
    names
}

/// Measures symbol art `art`, with the instances in it measured by `symbols`.
fn art_box(art: &Arc<Node>, symbols: &SymbolBoxes) -> Option<ArtBox> {
    let finite = |r: &Rect| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite());
    let geometric = art.geometric(Some(symbols)).filter(finite)?;
    let visual = art.visual(Some(symbols)).filter(finite).map_or(geometric, |v| v.union(geometric));
    let reach = crate::live::max_outset(std::slice::from_ref(art), Some(symbols));
    let mut effects = false;
    let mut brush: Option<f64> = None;
    art.walk(&mut |n| {
        let mut own = n.appearance.effects.iter().chain(n.appearance.items.iter().flat_map(|i| i.effects().iter()));
        effects |= own.any(|e| e.visible);
        let strokes = || n.appearance.items.iter().filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s) } else { None });
        if strokes().any(|s| s.visible && s.brush.is_some()) {
            brush = Some(strokes().fold(brush.unwrap_or(0.0), |w, s| w.max(s.width)));
        }
        if let NodeKind::SymbolInstance { symbol, .. } = &n.kind {
            effects |= symbols.has_effects(symbol);
            if let Some(w) = symbols.brush_width(symbol) {
                brush = Some(brush.map_or(w, |b| b.max(w)));
            }
        }
    });
    Some(ArtBox { geometric, visual, reach, effects, brush })
}

#[cfg(test)]
mod tests {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_geom::shapes;

    use super::*;
    use crate::{Appearance, Symbol};

    /// A rectangle filled black, with a `stroke` pt black stroke (0: none).
    fn rect(d: &mut Document, r: Rect, stroke: f64) -> Node {
        let paint = if stroke > 0.0 { Paint::solid(Color::BLACK) } else { Paint::None };
        Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::solid(Color::BLACK), paint, stroke))
    }

    fn instance(d: &mut Document, symbol: &str, xf: Affine) -> Node {
        Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: symbol.into(), xf })
    }

    /// Symbol `name` holding `art` in a group.
    fn symbol(d: &mut Document, name: &str, art: Vec<Node>) {
        let group = Node::group(d.alloc_id(), art.into_iter().map(Arc::new).collect());
        d.symbols.push(Symbol { name: name.into(), art: Arc::new(group) });
    }

    #[test]
    fn an_instance_measures_as_its_art_placed_by_its_transform() {
        let mut d = Document::new(200.0, 200.0);
        let bar = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 10.0), 2.0);
        symbol(&mut d, "Bar", vec![bar]);
        let moved = instance(&mut d, "Bar", Affine::translate((20.0, 30.0)));
        let scaled = instance(&mut d, "Bar", Affine::scale(3.0));
        let s = d.symbol_boxes();
        assert_eq!(moved.geometric_bounds_with(&s), Some(Rect::new(20.0, 30.0, 40.0, 40.0)));
        // Without the document: the 20 × 20 square around the origin, moved.
        assert_eq!(moved.geometric_bounds(), Some(Rect::new(10.0, 20.0, 30.0, 40.0)));
        // The 2 pt stroke paints 1 pt around the bar.
        assert_eq!(moved.visual_bounds_with(&s), Some(Rect::new(19.0, 29.0, 41.0, 41.0)));
        assert_eq!(moved.reach_bounds_with(&s), moved.visual_bounds_with(&s));
        // Scaled 3 times: the geometry scales, the stroke keeps its weight.
        assert_eq!(scaled.geometric_bounds_with(&s), Some(Rect::new(0.0, 0.0, 60.0, 30.0)));
        assert_eq!(scaled.visual_bounds_with(&s), Some(Rect::new(-1.0, -1.0, 61.0, 31.0)));
        // A click's box: the geometric box grown by the farthest the stroke can reach (a mitered
        // corner up to its miter limit).
        let reach = d.symbols[0].art.children().unwrap()[0].appearance.outset();
        assert_eq!(scaled.reach_bounds_with(&s), Some(Rect::new(0.0, 0.0, 60.0, 30.0).inflate(reach, reach)));
    }

    #[test]
    fn missing_symbols_and_symbols_inside_themselves_measure_as_the_square() {
        let mut d = Document::new(200.0, 200.0);
        let missing = instance(&mut d, "Nowhere", Affine::translate((50.0, 50.0)));
        assert_eq!(missing.geometric_bounds_with(&d.symbol_boxes()), Some(Rect::new(40.0, 40.0, 60.0, 60.0)));
        // A symbol whose art holds an instance of itself 100 pt to the right, and a symbol whose
        // art holds an instance of that one.
        let square = rect(&mut d, Rect::new(0.0, 0.0, 10.0, 10.0), 0.0);
        let inner = instance(&mut d, "Loop", Affine::translate((100.0, 0.0)));
        symbol(&mut d, "Loop", vec![square.clone(), inner]);
        let looped = instance(&mut d, "Loop", Affine::IDENTITY);
        symbol(&mut d, "Holder", vec![square, looped]);
        let outer = instance(&mut d, "Loop", Affine::IDENTITY);
        let doubled = instance(&mut d, "Loop", Affine::scale(2.0));
        let holder = instance(&mut d, "Holder", Affine::translate((50.0, 50.0)));
        let s = d.symbol_boxes();
        assert_eq!(outer.geometric_bounds_with(&s), Some(Rect::new(-10.0, -10.0, 10.0, 10.0)));
        assert_eq!(outer.visual_bounds_with(&s), Some(Rect::new(-10.0, -10.0, 10.0, 10.0)));
        assert_eq!(doubled.visual_bounds_with(&s), Some(Rect::new(-20.0, -20.0, 20.0, 20.0)));
        assert_eq!(holder.geometric_bounds_with(&s), Some(Rect::new(40.0, 40.0, 60.0, 60.0)));
    }

    #[test]
    fn nested_instances_measure_by_their_own_symbols_art() {
        let mut d = Document::new(200.0, 200.0);
        let dot = rect(&mut d, Rect::new(0.0, 0.0, 4.0, 4.0), 0.0);
        symbol(&mut d, "Dot", vec![dot]);
        let right = instance(&mut d, "Dot", Affine::translate((50.0, 0.0)));
        let below = instance(&mut d, "Dot", Affine::translate((0.0, 50.0)));
        symbol(&mut d, "Pair", vec![right, below]);
        let pair = instance(&mut d, "Pair", Affine::translate((100.0, 100.0)));
        assert_eq!(pair.geometric_bounds_with(&d.symbol_boxes()), Some(Rect::new(100.0, 100.0, 154.0, 154.0)));
    }

    #[test]
    fn each_symbols_art_box_is_measured_once() {
        // `Pair` holds two instances of `Dot`; three instances of `Pair` are measured twice each.
        let mut d = Document::new(200.0, 200.0);
        let dot = rect(&mut d, Rect::new(0.0, 0.0, 4.0, 4.0), 0.0);
        symbol(&mut d, "Dot", vec![dot]);
        let right = instance(&mut d, "Dot", Affine::translate((50.0, 0.0)));
        let below = instance(&mut d, "Dot", Affine::translate((0.0, 50.0)));
        symbol(&mut d, "Pair", vec![right, below]);
        let pairs: Vec<Node> = (0..3).map(|i| instance(&mut d, "Pair", Affine::translate((f64::from(i) * 10.0, 0.0)))).collect();
        let s = d.symbol_boxes().quick();
        assert_eq!(s.art_measured(), 0);
        for _ in 0..2 {
            for p in &pairs {
                p.geometric_bounds_with(&s);
            }
        }
        assert_eq!(s.art_measured(), 2, "the art of Pair and of Dot");
    }

    #[test]
    fn symbols_nested_32_deep_measure_as_the_square_whatever_was_measured_first() {
        // A chain of 40 symbols, each holding a 1 × 1 square and an instance of the next one 1 pt to
        // the right: symbols nest 39 deep in Link 0's art and 31 deep in Link 8's.
        let mut d = Document::new(200.0, 200.0);
        for i in 0..40 {
            let unit = rect(&mut d, Rect::new(0.0, 0.0, 1.0, 1.0), 0.0);
            let next = instance(&mut d, &format!("Link {}", i + 1), Affine::translate((1.0, 0.0)));
            symbol(&mut d, &format!("Link {i}"), if i < 39 { vec![unit, next] } else { vec![unit] });
        }
        let link0 = instance(&mut d, "Link 0", Affine::IDENTITY);
        let link7 = instance(&mut d, "Link 7", Affine::IDENTITY);
        let link8 = instance(&mut d, "Link 8", Affine::IDENTITY);
        let (square, chain) = (Some(Rect::new(-10.0, -10.0, 10.0, 10.0)), Some(Rect::new(0.0, 0.0, 32.0, 1.0)));
        let s = d.symbol_boxes();
        assert_eq!(link0.geometric_bounds_with(&s), square);
        assert_eq!(link8.geometric_bounds_with(&s), chain);
        assert_eq!(link7.geometric_bounds_with(&s), square);
        let s = d.symbol_boxes();
        assert_eq!(link8.geometric_bounds_with(&s), chain);
        assert_eq!(link0.geometric_bounds_with(&s), square);
    }

    #[test]
    fn brushed_strokes_are_found_in_the_art_and_in_nested_symbols_art() {
        let mut d = Document::new(200.0, 200.0);
        let mut line = rect(&mut d, Rect::new(0.0, 0.0, 10.0, 10.0), 3.0);
        line.appearance.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
        symbol(&mut d, "Brushed", vec![line]);
        let plain = rect(&mut d, Rect::new(0.0, 0.0, 10.0, 10.0), 5.0);
        let inner = instance(&mut d, "Brushed", Affine::IDENTITY);
        symbol(&mut d, "Holder", vec![plain.clone(), inner]);
        symbol(&mut d, "Plain", vec![plain]);
        let s = d.symbol_boxes();
        assert_eq!(s.brush_width("Brushed"), Some(3.0));
        assert_eq!(s.brush_width("Holder"), Some(3.0), "the 5 pt stroke has no brush");
        assert_eq!(s.brush_width("Plain"), None);
    }

    #[test]
    fn boxes_with_brush_art_grow_instances_of_brushed_art_once() {
        let mut d = Document::new(200.0, 200.0);
        // A line from (0, 40) to (0, 60) with a 1 pt brushed stroke, and a symbol whose art holds an
        // instance of it.
        let mut stroke = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
        stroke.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
        let line = Node::path(d.alloc_id(), shapes::line((0.0, 40.0).into(), (0.0, 60.0).into()), stroke);
        symbol(&mut d, "Nib", vec![line]);
        let inner = instance(&mut d, "Nib", Affine::IDENTITY);
        symbol(&mut d, "Holder", vec![inner]);
        let nib = instance(&mut d, "Nib", Affine::translate((-5.0, 0.0)));
        let holder = instance(&mut d, "Holder", Affine::translate((-5.0, 0.0)));
        let group = Node::group(d.alloc_id(), vec![Arc::new(nib.clone())]);
        let repeat = Node::new(d.alloc_id(), NodeKind::Repeat(crate::RepeatSpec::mirror(vec![Arc::new(nib.clone())], 90.0, 10.0)));
        let (plain, brushed) = (d.symbol_boxes(), d.symbol_boxes().with_brush_art());
        // 16 × 1 + 8 for the 1 pt stroke.
        for n in [&nib, &holder, &group, &repeat] {
            let b = n.visual_bounds_with(&plain).unwrap();
            assert_eq!(n.visual_bounds_with(&brushed), Some(b.inflate(24.0, 24.0)), "{}", n.kind_label());
        }
        assert_eq!(nib.geometric_bounds_with(&brushed), nib.geometric_bounds_with(&plain));
    }

    #[test]
    fn effect_reaches_are_measured_once_for_each_linear_part_and_up_to_a_limit() {
        // `Leaf` holds a square; `Holder` and `Other` each hold an instance of `Leaf`.
        let mut d = Document::new(10.0, 10.0);
        let square = rect(&mut d, Rect::new(0.0, 0.0, 1.0, 1.0), 0.0);
        symbol(&mut d, "Leaf", vec![square]);
        for name in ["Holder", "Other"] {
            let leaf = instance(&mut d, "Leaf", Affine::IDENTITY);
            symbol(&mut d, name, vec![leaf]);
        }
        let s = d.symbol_boxes();
        let runs = &Cell::new(0);
        // A measure that finds `reach` for every linear part.
        let measure = |reach: f64| {
            move |_: [f64; 4]| {
                runs.set(runs.get() + 1);
                reach
            }
        };
        let unscaled = [1.0, 0.0, 0.0, 1.0];
        assert_eq!(s.effects_reach("Holder", unscaled, &measure(3.0)), 3.0);
        assert_eq!(s.effects_reach("Holder", unscaled, &measure(5.0)), 3.0, "measured once");
        assert_eq!(runs.get(), 1);
        for i in 1..MAX_REACHES {
            s.effects_reach("Holder", [1.0, 0.0, 0.0, 1.0 + i as f64], &measure(1.0));
        }
        assert_eq!(runs.get(), MAX_REACHES);
        // Past the limit: the reach with no transform, times √2 and the larger of 1 and the
        // Frobenius norm (√8 for a scale of 2, √2 with no transform, √0.5 for a scale of 0.5). That
        // reach is measured once more for a symbol not measured before.
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(near(s.effects_reach("Holder", [2.0, 0.0, 0.0, 2.0], &measure(9.0)), 12.0));
        assert!(near(s.effects_reach("Other", unscaled, &measure(9.0)), 18.0));
        assert!(near(s.effects_reach("Other", [0.5, 0.0, 0.0, 0.5], &measure(7.0)), 9.0 * std::f64::consts::SQRT_2));
        assert_eq!(runs.get(), MAX_REACHES + 1);
        // A symbol whose art holds no instance is measured past the limit too.
        assert_eq!(s.effects_reach("Leaf", [2.0, 0.0, 0.0, 2.0], &measure(7.0)), 7.0);
        assert_eq!(runs.get(), MAX_REACHES + 2);
    }
}
