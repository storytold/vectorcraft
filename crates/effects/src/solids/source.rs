//! The art a 3D effect builds its solids from when its object, or a member of its group, shows art
//! it does not hold as paths: a symbol instance its symbol's art placed by the instance, a blend its
//! key objects and steps, a clipping group (or a layer with a clipping mask) its art inside the
//! clipping path. Each becomes a plain group of that art, keeping its transparency, so the solids
//! build from what the object shows. Images build no solid: they are traced first.

use std::borrow::Cow;
use std::sync::Arc;

use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, Node, NodeKind};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_pathops as po;

use super::{MAX_MEMBERS, geometry, is_container, profile, region, solid_paints};
use crate::GeomContext;

/// The deepest nesting of symbol instances, blends and clipping groups resolved for one 3D effect.
const MAX_DEPTH: usize = 64;

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
/// a clipping path too complex to cut the art with. `label` is the effect's name.
pub(crate) fn resolve<'a>(doc: Option<&Document>, n: &'a Node, label: &str) -> Result<Cow<'a, Node>, String> {
    if !needs(n) {
        return Ok(Cow::Borrowed(n));
    }
    let mut r = Resolver { doc, label, leaves: 0 };
    let children = r.content(n, 0)?;
    let mut out = n.clone();
    out.kind = NodeKind::Group { children, clip: false };
    Ok(Cow::Owned(out))
}

struct Resolver<'a> {
    doc: Option<&'a Document>,
    label: &'a str,
    /// The objects other than groups and layers kept so far, counted against [`MAX_MEMBERS`].
    leaves: usize,
}

impl Resolver<'_> {
    /// The art `n` shows, as the members of a plain group.
    fn content(&mut self, n: &Node, depth: usize) -> Result<Vec<Arc<Node>>, String> {
        let label = self.label;
        if depth > MAX_DEPTH {
            return Err(self.too_deep());
        }
        match &n.kind {
            NodeKind::SymbolInstance { symbol, .. } => {
                let art = crate::outline_object(self.doc, n).ok_or_else(|| {
                    format!("{label} can't build a solid from the instance of symbol \"{symbol}\": the symbol's art isn't available")
                })?;
                self.list(kids(&art), depth + 1)
            }
            NodeKind::Blend { children, spec } => {
                // Counted before the steps are made, so a blend past the limit is never expanded.
                let made = vectorcraft_doc::blend::blend_steps(children, spec).into_iter().fold(children.len(), usize::saturating_add);
                if self.leaves.saturating_add(made) > MAX_MEMBERS {
                    return Err(self.too_many());
                }
                let steps: Vec<Arc<Node>> = crate::expand_live(self.doc, n).into_iter().map(Arc::new).collect();
                self.list(&steps, depth + 1)
            }
            _ if n.clips() => {
                let Some((first, rest)) = kids(n).split_first() else { return Ok(vec![]) };
                let clip = clip_area(first);
                let inner = self.list(rest, depth + 1)?;
                let mut out = vec![];
                for c in &inner {
                    if let Some(c) = self.clipped(c, &clip, depth + 1)? {
                        out.push(Arc::new(c));
                    }
                }
                Ok(out)
            }
            _ => self.list(kids(n), depth + 1),
        }
    }

    /// The visible members of `children` as plain art: images refused, guides left out, symbol
    /// instances, blends and clipping groups resolved into groups with their transparency, groups
    /// holding them rebuilt.
    fn list(&mut self, children: &[Arc<Node>], depth: usize) -> Result<Vec<Arc<Node>>, String> {
        let mut out = Vec::with_capacity(children.len());
        for c in children {
            if !c.visible || matches!(c.kind, NodeKind::Path { guide: true, .. }) {
                continue;
            }
            if matches!(c.kind, NodeKind::Image(_)) {
                return Err(image_refusal(self.label));
            }
            if needs(c) {
                let kids = self.content(c, depth)?;
                out.push(Arc::new(wrap(c, kids)));
                continue;
            }
            if !is_container(c) {
                self.leaves += 1;
                if self.leaves > MAX_MEMBERS {
                    return Err(self.too_many());
                }
            }
            out.push(c.clone());
        }
        Ok(out)
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
    /// the clip and each stroke as the part of its outline inside it, filled in the stroke's paint;
    /// `None` when nothing of it is inside. Objects that build no solid stay as they are.
    fn clipped(&self, n: &Node, clip: &(PathData, FillRule), depth: usize) -> Result<Option<Node>, String> {
        if depth > MAX_DEPTH {
            return Err(self.too_deep());
        }
        if is_container(n) {
            let mut kids = vec![];
            for c in n.children().into_iter().flatten() {
                if let Some(c) = self.clipped(c, clip, depth + 1)? {
                    kids.push(Arc::new(c));
                }
            }
            return Ok((!kids.is_empty()).then(|| wrap(n, kids)));
        }
        let Some((path, rule)) = profile(n) else { return Ok(Some(n.clone())) };
        let path = geometry(&path, &n.appearance.effects, &GeomContext::of(n));
        let cut = |area: &PathData, rule: FillRule| -> Result<Option<PathData>, String> {
            let inside =
                po::try_boolean(area, rule, &clip.0, clip.1, po::BoolOp::Intersect, po::DEFAULT_PRECISION).map_err(|_| self.too_complex())?;
            Ok(inside.bounds().is_some().then_some(inside))
        };
        let items = solid_paints(n);
        let fills: Vec<AppearanceItem> = items.iter().filter(|i| matches!(i, AppearanceItem::Fill(_))).cloned().collect();
        // The fills' piece goes where the top fill is, the one Extrude & Bevel and Inflate build
        // their solid from, so the strokes above and below it keep their places.
        let top_fill = items.iter().rposition(|i| matches!(i, AppearanceItem::Fill(_)));
        let mut pieces = vec![];
        for (k, item) in items.iter().enumerate() {
            match item {
                AppearanceItem::Fill(_) if Some(k) == top_fill => {
                    if let Some(area) = cut(&path, rule)? {
                        pieces.push(Arc::new(shape(n, area, FillRule::NonZero, fills.clone())));
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
