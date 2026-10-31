//! Use Preview Bounds: objects measured as they are drawn, with their geometry effects.
//!
//! [`drawn_art`] gives an object whose geometry effects change what it draws as plain art. A path
//! or compound path is given as its effected geometry, and a fill or stroke with geometry effects
//! of its own paints its own geometry. Type, images, live objects and symbol instances are given as
//! their reshaped art ([`reshape`]), and a symbol instance is also given as its reshaped art when
//! its symbol's art has geometry effects. Groups and layers with geometry or Pathfinder effects of
//! their own, or with a fill or stroke of their own that has geometry effects or paints members
//! with geometry effects, are given as their evaluated art ([`evaluate_container`]). Blends,
//! envelopes and repeats whose objects have geometry effects are given as their evaluated art
//! ([`expanded_live_group`]), with each of those objects given as its own art. A clip group or
//! layer whose clipping path has strokes with geometry effects is given as that path's geometry
//! before its effects, which the renderer clips by, and those strokes as drawn. Fills and
//! strokes that paint nothing (hidden, or painted None) are left out. [`bounds_of`],
//! [`oriented_bounds`] and [`anchor_bounds`] measure the visual bounds of that art, strokes
//! included, where [`Document::bounds_of`], [`Document::oriented_bounds`] and
//! [`Selection::anchor_bounds`] measure the objects themselves. Raster effects (shadows, glows,
//! blurs) are not measured. In one object, the art of at most 4,096 symbol instances is measured
//! with its geometry effects, and the other instances in it are measured without them.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_doc::color::Paint;
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Node, NodeId, NodeKind, OrientedBox, Selection, SymbolBoxes};
use vectorcraft_geom::{Affine, PathData, Rect, Shape, union_opt};

use crate::group::has_own_paint;
use crate::reshape::{geometry_effects, leaf_geometry};
use crate::{
    GeomContext, apply_geometry_with, evaluate_container, expanded_live_group, has_geometry, has_item_geometry, has_pathfinder, needs_outline,
    outline_art, paints, reshape,
};

/// Most symbol instances in one object whose art [`Drawn`] measures with its geometry effects:
/// past it, the other instances in the object are measured without geometry effects.
const MAX_INSTANCES: usize = 4096;

/// The bounds of `ids`: with `preview`, their visual bounds as drawn ([`visual_bounds`]), else
/// their geometric bounds ([`Document::bounds_of`]).
pub fn bounds_of(doc: &Document, ids: &[NodeId], preview: bool) -> Option<Rect> {
    if !preview {
        return doc.bounds_of(ids, false);
    }
    let symbols = doc.symbol_boxes();
    let drawn = Drawn::new(&symbols);
    ids.iter().filter_map(|id| doc.node(*id)).fold(None, |acc, n| union_opt(acc, drawn.next().visual_bounds(n)))
}

/// The bounding box of `ids` square to their shared angle ([`Document::bbox_angle`]), measured as
/// [`bounds_of`] measures them: [`Document::oriented_bounds`] with `preview` false.
pub fn oriented_bounds(doc: &Document, ids: &[NodeId], preview: bool) -> Option<OrientedBox> {
    if !preview {
        return doc.oriented_bounds(ids, false);
    }
    let angle = doc.bbox_angle(ids);
    if angle == 0.0 {
        return bounds_of(doc, ids, true).map(OrientedBox::aligned);
    }
    let to_local = OrientedBox { angle, rect: Rect::ZERO }.to_doc().inverse();
    let symbols = doc.symbol_boxes();
    let drawn = Drawn::new(&symbols);
    let rect = ids.iter().filter_map(|id| doc.node(*id)).fold(None, |acc, n| union_opt(acc, drawn.next().bounds_in(n, to_local)))?;
    Some(OrientedBox { angle, rect })
}

/// With anchors selected with Direct Selection, the box around those anchors and the objects
/// selected whole, measured by [`bounds_of`]. `None` without any ([`Selection::anchor_bounds`]).
pub fn anchor_bounds(selection: &Selection, doc: &Document, preview: bool) -> Option<Rect> {
    if selection.anchors.is_empty() {
        return None;
    }
    union_opt(bounds_of(doc, &selection.whole(), preview), selection.anchors_box(doc))
}

/// The visual bounds of `n` as drawn: those of its [`drawn_art`], else
/// [`Node::visual_bounds_with`].
pub fn visual_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    Drawn::new(symbols).visual_bounds(n)
}

/// [`visual_bounds`] of `n` as `a` would place it, without changing it ([`Node::bounds_in`]):
/// the art is drawn where `n` is and then placed by `a`.
pub fn bounds_in(symbols: &SymbolBoxes, n: &Node, a: Affine) -> Option<Rect> {
    Drawn::new(symbols).bounds_in(n, a)
}

/// `n` as plain art with the geometry effects in it applied, at every depth, symbol instances
/// measured by `symbols`. Raster effects stay on the art. A group, layer or intertwine without a
/// clip is rebuilt with the drawn art of each member that has one. A clip group or layer becomes a
/// group of its clipping path without strokes and the drawn art of the path's strokes, when they
/// have one. `None` when no geometry effect in `n` changes what it draws.
pub fn drawn_art(symbols: &SymbolBoxes, n: &Node) -> Option<Node> {
    Drawn::new(symbols).drawn_art(n)
}

/// Whether [`drawn_art`] gives `n` art of its own, or would give one to an object in it. A symbol
/// instance in `n` counts when its symbol's art has a geometry effect.
pub fn draws_geometry(symbols: &SymbolBoxes, n: &Node) -> bool {
    Drawn::new(symbols).draws_geometry(n)
}

/// One measurement: whether the art of each symbol checked so far draws geometry, by symbol name,
/// and how many more instances' art it measures with its geometry effects in the current object ([`MAX_INSTANCES`]).
struct Drawn<'s, 'd> {
    symbols: &'s SymbolBoxes<'d>,
    draws: RefCell<HashMap<String, bool>>,
    left: Cell<usize>,
}

impl<'s, 'd> Drawn<'s, 'd> {
    fn new(symbols: &'s SymbolBoxes<'d>) -> Self {
        Drawn { symbols, draws: RefCell::default(), left: Cell::new(MAX_INSTANCES) }
    }

    /// Sets the count of instances left back to [`MAX_INSTANCES`], for the next object.
    fn next(&self) -> &Self {
        self.left.set(MAX_INSTANCES);
        self
    }

    /// See [`visual_bounds`].
    fn visual_bounds(&self, n: &Node) -> Option<Rect> {
        match self.drawn_art(n) {
            Some(art) => art.visual_bounds_with(self.symbols),
            None => n.visual_bounds_with(self.symbols),
        }
    }

    /// See [`bounds_in`].
    fn bounds_in(&self, n: &Node, a: Affine) -> Option<Rect> {
        if a == Affine::IDENTITY {
            return self.visual_bounds(n);
        }
        match self.drawn_art(n) {
            Some(art) => art.bounds_in(self.symbols, a, true),
            None => n.bounds_in(self.symbols, a, true),
        }
    }

    /// See [`drawn_art`].
    fn drawn_art(&self, n: &Node) -> Option<Node> {
        let Some(mut art) = self.own_art(n) else { return self.clip_art(n).or_else(|| self.with_drawn_members(n)) };
        // The art's members can carry geometry effects of their own: type's fills and strokes, the
        // members of an evaluated group, the clipping path of an evaluated clip group and the
        // objects in a symbol's art.
        if let Some(m) = self.clip_art(&art).or_else(|| self.with_drawn_members(&art)) {
            art = m;
        }
        Some(art)
    }

    /// See [`draws_geometry`].
    fn draws_geometry(&self, n: &Node) -> bool {
        let mut found = false;
        n.walk(&mut |c| {
            found = found
                || match &c.kind {
                    NodeKind::SymbolInstance { symbol, .. } => !geometry_effects(c).is_empty() || self.symbol_draws(symbol),
                    _ => reshapes(c),
                };
        });
        found
    }

    /// Whether the art of the symbol named `name` has effects and [`Self::draws_geometry`], worked
    /// out once for each name.
    fn symbol_draws(&self, name: &str) -> bool {
        if let Some(d) = self.draws.borrow().get(name) {
            return *d;
        }
        let d = self.symbols.has_effects(name) && self.symbols.measure_art(name, |art| self.draws_geometry(art)).unwrap_or(false);
        self.draws.borrow_mut().insert(name.to_string(), d);
        d
    }

    /// The art the geometry effects of `n` itself give it, or that a symbol instance gets from the
    /// geometry effects in its symbol's art. A group or layer with fills or strokes of its own that
    /// paint, around members with geometry effects, is evaluated too: its fills and strokes paint
    /// the members' geometry before their effects. A blend, envelope or repeat whose objects have
    /// geometry effects is evaluated as the renderer evaluates it ([`expanded_live_group`]), and
    /// the objects in its evaluated art keep their geometry effects. `None` when it has none, and
    /// for a symbol instance past [`MAX_INSTANCES`].
    fn own_art(&self, n: &Node) -> Option<Node> {
        let inner = || n.children().is_some_and(|ch| ch.iter().any(|c| self.draws_geometry(c)));
        let painted = || has_own_paint(n) && inner();
        match &n.kind {
            NodeKind::Group { .. } | NodeKind::Layer { .. } if reshapes(n) || painted() => evaluate_container(Some(self.symbols.document()), n),
            NodeKind::Path { .. } | NodeKind::Compound { .. } if reshapes(n) => path_art(n),
            NodeKind::SymbolInstance { symbol, .. } => {
                if geometry_effects(n).is_empty() && !self.symbol_draws(symbol) {
                    return None;
                }
                let left = self.left.get().checked_sub(1)?;
                self.left.set(left);
                let art = self.symbols.measure_art(symbol, Arc::clone)?;
                // An instance in perspective is projected by `reshape`.
                let doc = Some(self.symbols.document());
                reshape(doc, n, Some(&*art)).or_else(|| outline_art(doc, n, Some(&*art)))
            }
            _ if reshapes(n) => reshape(Some(self.symbols.document()), n, None),
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) if inner() => {
                Some(expanded_live_group(Some(self.symbols.document()), n))
            }
            _ => None,
        }
    }

    /// `n`, a group, layer or intertwine without a clip, with each member that has a
    /// [`Self::drawn_art`] replaced by it. `None` for any other object and when no member has one.
    fn with_drawn_members(&self, n: &Node) -> Option<Node> {
        if n.clips() || !matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. } | NodeKind::Intertwine { .. }) {
            return None;
        }
        let drawn: Vec<(usize, Node)> = n.children()?.iter().enumerate().filter_map(|(i, c)| Some((i, self.drawn_art(c)?))).collect();
        if drawn.is_empty() {
            return None;
        }
        let mut m = n.clone();
        let children = m.children_mut()?;
        for (i, c) in drawn {
            if let Some(slot) = children.get_mut(i) {
                *slot = Arc::new(c);
            }
        }
        Some(m)
    }

    /// Clip group or layer `n` as its clipping path's strokes draw it: a group of the clipping path
    /// without its strokes, measured as the region it clips to, and its strokes ([`Node::clip_paint`])
    /// as their [`Self::drawn_art`]. The renderer clips by the clipping path's geometry before its
    /// geometry effects ([`crate::clip_outline`]) and paints the path's fills inside that region and
    /// its strokes over the clipped art, unclipped. `None` for any other object, when the clipping
    /// path is hidden and when its strokes have no drawn art.
    fn clip_art(&self, n: &Node) -> Option<Node> {
        if !n.clips() {
            return None;
        }
        let clip = n.children()?.first().filter(|c| c.visible)?;
        let strokes = self.drawn_art(&clip.clip_paint().stroke?)?;
        Some(Node::group(n.id, vec![Arc::new(unstroked(clip)), Arc::new(strokes)]))
    }
}

/// Clipping path `clip` without its strokes or its characters' strokes.
fn unstroked(clip: &Node) -> Node {
    let mut n = clip.clone();
    n.appearance.items.retain(AppearanceItem::is_fill);
    if let NodeKind::Text(t) = &mut n.kind {
        for r in &mut t.runs {
            r.style.stroke = Paint::None;
        }
    }
    n
}

/// Whether `n` itself has a geometry effect that [`Drawn::own_art`] applies, its own or one of its
/// fill's or stroke's (a group or layer: also a Pathfinder effect; a symbol instance: only its own).
fn reshapes(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Group { .. } | NodeKind::Layer { .. } => {
            has_pathfinder(n) || !geometry_effects(n).is_empty() || n.appearance.items.iter().any(|i| paints(i) && has_geometry(i.effects()))
        }
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => {
            has_geometry(&n.appearance.effects) || n.appearance.items.iter().any(|i| paints(i) && has_geometry(i.effects()))
        }
        _ => needs_outline(n) && (!geometry_effects(n).is_empty() || has_item_geometry(n)),
    }
}

/// Path or compound path `n` as a group of paths: its geometry after its own geometry effects
/// painted by its fills and strokes without geometry effects, and that geometry after the
/// geometry effects of each other fill or stroke that paints, painted by that fill or stroke, as
/// the renderer draws them. The first path is left out when only fills or strokes with geometry
/// effects paint. The paths keep `n`'s id and fill rule and carry no effects.
fn path_art(n: &Node) -> Option<Node> {
    let (path, rule) = leaf_geometry(n)?;
    let ctx = GeomContext::of(n);
    let g = apply_geometry_with(&n.appearance.effects, &path, path.to_bezpath().bounding_box(), &ctx);
    let piece = |geometry: PathData, items: Vec<AppearanceItem>| {
        let mut ap: Appearance = (*n.appearance).clone();
        ap.items = items;
        ap.effects.clear();
        for i in &mut ap.items {
            i.effects_mut().clear();
        }
        let mut p = Node::path(n.id, geometry, ap);
        if let NodeKind::Path { rule: r, .. } = &mut p.kind {
            *r = rule;
        }
        Arc::new(p)
    };
    let (own, plain): (Vec<&AppearanceItem>, Vec<&AppearanceItem>) = n.appearance.items.iter().partition(|i| paints(i) && has_geometry(i.effects()));
    let mut pieces = vec![];
    if own.is_empty() || plain.iter().any(|i| paints(i)) {
        pieces.push(piece(g.clone(), plain.into_iter().cloned().collect()));
    }
    let bounds = g.to_bezpath().bounding_box();
    for item in own {
        let geometry = apply_geometry_with(item.effects(), &g, bounds, &ctx.item(item));
        pieces.push(piece(geometry, vec![item.clone()]));
    }
    Some(Node::group(n.id, pieces))
}
