//! The art a 3D effect builds its solids from when its object, or a member of its group, shows art
//! it does not hold as paths: a symbol instance its symbol's art placed by the instance, a blend its
//! key objects and steps, a clipping group (or a layer with a clipping mask) its art inside the
//! clipping path. Each becomes a plain group of that art, keeping its transparency, so the solids
//! build from what the object shows. Images build no solid: they are traced first.

use std::borrow::Cow;
use std::sync::Arc;

use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, Node, NodeKind};
use vectorcraft_geom::{FillRule, PathData, Rect};
use vectorcraft_pathops as po;

use super::{MAX_MEMBERS, Scope, geometry, is_container, members, profile, region, reshape_members, solid_paints};
use crate::{GeomContext, is_geometry};

/// The deepest nesting of symbol instances, blends and clipping groups resolved for one 3D effect.
const MAX_DEPTH: usize = 64;

/// The most objects resolving one 3D effect's art may visit or make: every object listed (hidden
/// ones and guides included), every symbol instance, blend and clipping group resolved, every blend
/// step made and every object cut to a clipping path. A symbol that holds instances of another
/// repeats that symbol's art at every level, so art with few visible objects can still take this
/// many steps.
pub(super) const MAX_WORK: usize = 1 << 18;

/// The error for an image a 3D effect is asked to build from.
pub(crate) fn image_refusal(label: &str) -> String {
    format!("{label} can't build a solid from an image: trace it first with Object › Image Trace › Make.")
}

/// Is `n` a symbol instance, a blend or a clipping group or layer?
pub(crate) fn resolves(n: &Node) -> bool {
    matches!(n.kind, NodeKind::SymbolInstance { .. } | NodeKind::Blend { .. }) || n.clips()
}

/// Is `n`, or a visible member of `n` at any depth of its groups and layers, one [`resolves`] takes?
fn needs(n: &Node) -> bool {
    let mut stack = vec![n];
    while let Some(n) = stack.pop() {
        if resolves(n) {
            return true;
        }
        if is_container(n) {
            stack.extend(kids(n).iter().filter(|c| c.visible).map(|c| &**c));
        }
    }
    false
}

/// `n` as a 3D effect builds from it: unchanged when neither it nor a visible member of its groups
/// is a symbol instance, a blend or a clipping group, otherwise a group with `n`'s id, appearance
/// and transparency holding that art (a symbol instance's symbol art from `doc`). An error names
/// what can't be resolved: an image, a missing symbol, art nested or numbering past the limits, or
/// a clipping path too complex to cut the art with. `label` is the effect's name. The art is cut
/// for the solid effect `n` carries ([`resolve_for`]).
pub(crate) fn resolve<'a>(doc: Option<&Document>, n: &'a Node, label: &str) -> Result<Cow<'a, Node>, String> {
    let every_fill = super::solid(&n.appearance.effects).is_some_and(|(_, fx)| fx.id == super::ROTATE);
    resolve_for(doc, n, label, every_fill)
}

/// [`resolve`] for the 3D effect that builds from `n`: `every_fill` for Rotate, which turns every
/// fill, so each fill a clipping path cuts keeps its own place among the strokes. Otherwise the
/// effect builds from the top fill, and the fills cut go together in the top fill's place.
pub(crate) fn resolve_for<'a>(doc: Option<&Document>, n: &'a Node, label: &str, every_fill: bool) -> Result<Cow<'a, Node>, String> {
    if !needs(n) {
        return Ok(Cow::Borrowed(n));
    }
    let mut r = Resolver { doc, label, every_fill, leaves: 0, work: 0, shown: None };
    let children = r.content(n, 0);
    #[cfg(test)]
    WORK.with(|w| w.set(r.work));
    let mut out = n.clone();
    out.kind = NodeKind::Group { children: children?, clip: false };
    Ok(Cow::Owned(out))
}

#[cfg(test)]
thread_local! {
    /// The objects the last [`resolve`] on this thread visited or made ([`Resolver::work`]).
    pub(super) static WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Resolver<'a> {
    doc: Option<&'a Document>,
    label: &'a str,
    /// Does each fill a clipping path cuts become a piece of its own ([`resolve_for`])?
    every_fill: bool,
    /// The objects other than groups and layers kept so far, counted against [`MAX_MEMBERS`].
    leaves: usize,
    /// The objects visited or made so far, counted against [`MAX_WORK`].
    work: usize,
    /// Inside clipping groups, the box of what their clipping paths show together.
    shown: Option<Rect>,
}

impl Resolver<'_> {
    /// Count `steps` more objects visited or made, or an error past [`MAX_WORK`].
    fn spend(&mut self, steps: usize) -> Result<(), String> {
        self.work = self.work.saturating_add(steps);
        if self.work > MAX_WORK {
            return Err(self.too_much_work());
        }
        Ok(())
    }

    fn too_much_work(&self) -> String {
        format!("{} can't build solids from this art: its symbol instances, blends and clipping groups hold too many objects", self.label)
    }

    /// `f` run on the art of `c`. The effects of an object or group can move its art into a clip,
    /// so inside one with effects no image is left out for lying outside the clip ([`Self::shown`]).
    fn inside<T>(&mut self, c: &Node, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let outer = self.shown;
        if has_effects(c) {
            self.shown = None;
        }
        let out = f(self);
        self.shown = outer;
        out
    }

    /// The art `n`, a symbol instance, a blend, a clipping group or layer, or a group holding one,
    /// shows, as the members of a plain group.
    fn content(&mut self, n: &Node, depth: usize) -> Result<Vec<Arc<Node>>, String> {
        let label = self.label;
        if depth > MAX_DEPTH {
            return Err(self.too_deep());
        }
        self.spend(1)?;
        match &n.kind {
            NodeKind::SymbolInstance { symbol, .. } => {
                // Counted before the art is placed, so a symbol past the limit is never copied.
                let held = self.doc.and_then(|d| d.symbols.iter().find(|s| s.name == *symbol)).map_or(0, |s| s.art.count_with_masks());
                if self.work.saturating_add(held) > MAX_WORK {
                    return Err(self.too_much_work());
                }
                let art = crate::outline_object(self.doc, n).ok_or_else(|| {
                    format!("{label} can't build a solid from the instance of symbol \"{symbol}\": the symbol's art isn't available")
                })?;
                Ok(self.list(kids(&art), depth + 1)?.0)
            }
            NodeKind::Blend { children, spec } => {
                // Counted before the steps are made, so a blend past the limit is never expanded.
                let made = vectorcraft_doc::blend::blend_steps(children, spec).into_iter().fold(children.len(), usize::saturating_add);
                if self.leaves.saturating_add(made) > MAX_MEMBERS {
                    return Err(self.too_many());
                }
                self.spend(made)?;
                let steps: Vec<Arc<Node>> = crate::expand_live(self.doc, n).into_iter().map(Arc::new).collect();
                Ok(self.list(&steps, depth + 1)?.0)
            }
            _ if n.clips() => {
                let Some((first, rest)) = kids(n).split_first() else { return Ok(vec![]) };
                if clip_nested_past(first, MAX_DEPTH.saturating_sub(depth)) {
                    return Err(self.too_deep());
                }
                let clip = clip_area(first);
                // An image wholly outside every clip around it shows nothing and is left out.
                let outer = self.shown;
                let here = clip.0.bounds().unwrap_or(Rect::ZERO);
                self.shown = Some(outer.map_or(here, |o| o.intersect(here)));
                let listed = self.list(rest, depth + 1);
                self.shown = outer;
                let (inner, _) = listed?;
                let mut out = vec![];
                for c in &inner {
                    if let Some(c) = self.clipped(c, &clip, depth + 1)? {
                        out.push(Arc::new(c));
                    }
                }
                Ok(out)
            }
            _ => Ok(self.list(kids(n), depth + 1)?.0),
        }
    }

    /// The visible members of `children` as plain art: images refused, guides left out, symbol
    /// instances, blends and clipping groups resolved into groups with their transparency and
    /// geometry effects, groups holding them rebuilt. Also whether any member changed: a group
    /// whose members did not is kept as it is. Each object is visited once.
    fn list(&mut self, children: &[Arc<Node>], depth: usize) -> Result<(Vec<Arc<Node>>, bool), String> {
        let mut out = Vec::with_capacity(children.len());
        let mut changed = false;
        for c in children {
            self.spend(1)?;
            if !c.visible || matches!(c.kind, NodeKind::Path { guide: true, .. }) {
                continue;
            }
            if matches!(c.kind, NodeKind::Image(_)) {
                // Only an image with no effects or fills and strokes of its own is measured by its
                // frame; any other one may draw inside the clip.
                let plain = !has_effects(c) && c.appearance.items.iter().all(|i| !i.visible());
                let hidden = plain && self.shown.is_some_and(|k| c.visual_bounds().is_none_or(|b| b.intersect(k).area() <= 0.0));
                if hidden {
                    changed = true;
                    continue;
                }
                return Err(image_refusal(self.label));
            }
            if resolves(c) {
                let kids = self.inside(c, |r| r.content(c, depth))?;
                out.push(Arc::new(with_effects(wrap(c, kids), c)));
                changed = true;
                continue;
            }
            if is_container(c) {
                // At the depth limit a group is kept, and not walked further, only when it holds no
                // group with members and nothing in it needs resolving.
                if depth >= MAX_DEPTH {
                    if nested_past(c, 1) || needs(c) {
                        return Err(self.too_deep());
                    }
                    out.push(c.clone());
                    continue;
                }
                let (kids, inner) = self.inside(c, |r| r.list(kids(c), depth + 1))?;
                if inner {
                    out.push(Arc::new(with_effects(wrap(c, kids), c)));
                    changed = true;
                } else {
                    out.push(c.clone());
                }
                continue;
            }
            self.leaves += 1;
            if self.leaves > MAX_MEMBERS {
                return Err(self.too_many());
            }
            out.push(c.clone());
        }
        Ok((out, changed))
    }

    fn too_complex(&self) -> String {
        format!("{}: the clipping path is too complex to cut the art with", self.label)
    }

    fn too_many(&self) -> String {
        format!("{} takes at most {MAX_MEMBERS} objects", self.label)
    }

    fn too_deep(&self) -> String {
        format!("{} can't build solids from art nested more than {MAX_DEPTH} levels deep", self.label)
    }

    /// `n`, plain art inside a clipping group, cut to `clip`: each shape's fills as its area inside
    /// the clip and each stroke as the part of its outline inside it, filled in the stroke's paint,
    /// after the geometry effects of the shape and of the groups it is in; hidden objects and guides
    /// left out. `None` when nothing of it is inside. Objects that build no solid stay as they are.
    fn clipped(&mut self, n: &Node, clip: &(PathData, FillRule), depth: usize) -> Result<Option<Node>, String> {
        if depth > MAX_DEPTH {
            return Err(self.too_deep());
        }
        self.spend(1)?;
        if is_container(n) {
            let mut kids = vec![];
            if n.appearance.effects.iter().any(|e| e.visible && is_geometry(&e.id)) {
                // The group's geometry effects reshape its members together before the clip cuts
                // them, as they are drawn: each member becomes its reshaped outline.
                if nested_past(n, MAX_DEPTH.saturating_sub(depth)) {
                    return Err(self.too_deep());
                }
                let mut shapes = members(n, self.label)?;
                reshape_members(n, &n.appearance.effects, &mut shapes);
                let mut cut = vec![];
                for s in shapes {
                    if let Some(c) = self.clipped(&shape(n, s.path, s.rule, s.items), clip, depth + 1)? {
                        cut.push((s.scopes, c));
                    }
                }
                kids = nest(n, cut, 0);
            } else {
                for c in n.children().into_iter().flatten() {
                    if !c.visible || matches!(c.kind, NodeKind::Path { guide: true, .. }) {
                        continue;
                    }
                    if let Some(c) = self.clipped(c, clip, depth + 1)? {
                        kids.push(Arc::new(c));
                    }
                }
            }
            return Ok((!kids.is_empty()).then(|| wrap(n, kids)));
        }
        let Some((path, rule)) = profile(n) else { return Ok(Some(n.clone())) };
        let items = solid_paints(n);
        // The fills' piece goes where the top fill is, the one Extrude & Bevel and Inflate build
        // their solid from, so the strokes above and below it keep their places. Rotate turns
        // every fill, so each fill is a piece in its own place.
        let top_fill = items.iter().rposition(|i| matches!(i, AppearanceItem::Fill(_)));
        let fill_pieces =
            if self.every_fill { items.iter().filter(|i| matches!(i, AppearanceItem::Fill(_))).count() } else { usize::from(top_fill.is_some()) };
        // Each cut makes a piece, counted before it is made: the shape was counted once already.
        let made = fill_pieces + items.iter().filter(|i| matches!(i, AppearanceItem::Stroke(_))).count();
        self.spend(made)?;
        self.leaves = self.leaves.saturating_add(made.saturating_sub(1));
        if self.leaves > MAX_MEMBERS {
            return Err(self.too_many());
        }
        let path = geometry(&path, &n.appearance.effects, &GeomContext::of(n));
        let cut = |area: &PathData, rule: FillRule| -> Result<Option<PathData>, String> {
            let inside =
                po::try_boolean(area, rule, &clip.0, clip.1, po::BoolOp::Intersect, po::DEFAULT_PRECISION).map_err(|_| self.too_complex())?;
            Ok(inside.bounds().is_some().then_some(inside))
        };
        let fills: Vec<AppearanceItem> = items.iter().filter(|i| matches!(i, AppearanceItem::Fill(_))).cloned().collect();
        // Every fill paints the same area: it is cut once.
        let mut fill_area: Option<Option<PathData>> = None;
        let mut pieces = vec![];
        for (k, item) in items.iter().enumerate() {
            match item {
                AppearanceItem::Fill(_) if self.every_fill || Some(k) == top_fill => {
                    let area = match &fill_area {
                        Some(area) => area.clone(),
                        None => fill_area.insert(cut(&path, rule)?).clone(),
                    };
                    if let Some(area) = area {
                        let painted = if self.every_fill { vec![item.clone()] } else { fills.clone() };
                        pieces.push(Arc::new(shape(n, area, FillRule::NonZero, painted)));
                    }
                }
                AppearanceItem::Fill(_) => {}
                AppearanceItem::Stroke(s) => {
                    let (outline, outline_rule) = region(&path, rule, item);
                    if let Some(area) = cut(&outline, outline_rule)? {
                        let mut fill = FillLayer::new(s.paint.clone());
                        fill.opacity = s.opacity;
                        fill.blend = s.blend;
                        pieces.push(Arc::new(shape(n, area, FillRule::NonZero, vec![AppearanceItem::Fill(fill)])));
                    }
                }
            }
        }
        Ok((!pieces.is_empty()).then(|| wrap(n, pieces)))
    }
}

/// The area clipping path `first` keeps, as the renderer clips by it ([`crate::clip_outline`]): a
/// nested clipping group's own clipping path, the union of a group's members. Empty when it keeps
/// nothing.
fn clip_area(first: &Node) -> (PathData, FillRule) {
    crate::clip_outline(first).map_or_else(|| (PathData::default(), FillRule::NonZero), |(bp, rule)| (PathData::from_bezpath(&bp), rule))
}

/// `pieces`, each with the [`Scope`]s of the groups it was in, back in groups of `n`'s id with
/// those groups' opacity and blending mode: consecutive pieces sharing the scope at `level` go into
/// one group, so its transparency applies to them together. `level` stays below the nesting the
/// pieces came from ([`nested_past`]).
fn nest(n: &Node, pieces: Vec<(Vec<Scope>, Node)>, level: usize) -> Vec<Arc<Node>> {
    let mut out = vec![];
    let mut run: Vec<(Vec<Scope>, Node)> = vec![];
    let mut key = None;
    let close = |run: &mut Vec<(Vec<Scope>, Node)>, scope: Option<Scope>, out: &mut Vec<Arc<Node>>| {
        let Some(scope) = scope else { return };
        let mut g = Node::group(n.id, nest(n, std::mem::take(run), level + 1));
        g.opacity = scope.opacity;
        g.blend = scope.blend;
        out.push(Arc::new(g));
    };
    for (scopes, piece) in pieces {
        let here = scopes.get(level).copied();
        if here.map(|s| s.key) != key.map(|s: Scope| s.key) {
            close(&mut run, key, &mut out);
            key = here;
        }
        match here {
            Some(_) => run.push((scopes, piece)),
            None => out.push(Arc::new(piece)),
        }
    }
    close(&mut run, key, &mut out);
    out
}

/// Does `n` carry a visible effect?
fn has_effects(n: &Node) -> bool {
    n.appearance.effects.iter().any(|e| e.visible)
}

/// Does the group `n` hold shown objects nested more than `levels` deep in groups and layers?
/// Only the members [`members`] walks are followed: hidden members are skipped, and the paths of
/// a compound path are one shape. Walked without recursion.
fn nested_past(n: &Node, levels: usize) -> bool {
    let mut stack = vec![(n, 0usize)];
    while let Some((n, level)) = stack.pop() {
        if level > levels {
            return true;
        }
        if level == 0 || is_container(n) {
            stack.extend(kids(n).iter().filter(|c| c.visible).map(|c| (&**c, level + 1)));
        }
    }
    false
}

/// Does the clipping path `n` reach objects nested more than `levels` deep along the members that
/// [`Node::clip_shapes`] visits: the first member of a clipping group, the shown members of other
/// groups and layers (not a group's shaper), and every member of a live object. Walked without
/// recursion.
fn clip_nested_past(n: &Node, levels: usize) -> bool {
    let mut stack = vec![(n, 0usize)];
    while let Some((n, level)) = stack.pop() {
        if level > levels {
            return true;
        }
        match &n.kind {
            NodeKind::Group { children, clip: true } => stack.extend(children.first().map(|c| (&**c, level + 1))),
            NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
                let skip = usize::from(n.extras().shaper.is_some());
                stack.extend(children.iter().skip(skip).filter(|c| c.visible).map(|c| (&**c, level + 1)));
            }
            NodeKind::Compound { .. } | NodeKind::SymbolInstance { .. } => {}
            _ => stack.extend(kids(n).iter().map(|c| (&**c, level + 1))),
        }
    }
    false
}

/// The members of `n`, none for an object without members.
fn kids(n: &Node) -> &[Arc<Node>] {
    n.children().map_or(&[], Vec::as_slice)
}

/// A path of `n`'s id on `path`, painted by `items` under `rule`.
fn shape(n: &Node, path: PathData, rule: FillRule, items: Vec<AppearanceItem>) -> Node {
    let mut piece = Node::path(n.id, path, Appearance { items, ..Default::default() });
    if let NodeKind::Path { rule: r, .. } = &mut piece.kind {
        *r = rule;
    }
    piece
}

/// A plain group of `children` with `n`'s id, visibility, opacity and blending mode.
fn wrap(n: &Node, children: Vec<Arc<Node>>) -> Node {
    let mut g = Node::group(n.id, children);
    g.name = n.name.clone();
    g.opacity = n.opacity;
    g.blend = n.blend;
    g
}

/// `g`, the group `n` was resolved into, with `n`'s appearance: a 3D effect's group reshapes its
/// members with its geometry effects ([`members`]), which read its stroke as they do when `n` is
/// drawn. A group's own fills and strokes build no solid.
fn with_effects(mut g: Node, n: &Node) -> Node {
    g.appearance = n.appearance.clone();
    g
}
