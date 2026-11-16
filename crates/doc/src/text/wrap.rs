//! Object → Text Wrap: area type flows around the wrap objects stacked above it in the same layer.

use std::sync::Arc;

use vectorcraft_geom::{PathData, shapes};

use super::{TextKind, TextObject, WrapShape};
use crate::{Document, Node, NodeId, NodeKind};

/// Outline of a wrap object in document coordinates: its paths, or its bounds for other art.
fn outline(n: &Node, out: &mut PathData) {
    match &n.kind {
        NodeKind::Path { path, guide: false, .. } => {
            out.subpaths.extend(path.subpaths.iter().filter(|sp| sp.closed || sp.anchors.len() > 2).cloned())
        }
        NodeKind::Compound { children, .. } | NodeKind::Group { children, clip: false } => {
            for c in children.iter().filter(|c| c.visible) {
                outline(c, out);
            }
        }
        // A clip group wraps by its clipping path.
        NodeKind::Group { children, clip: true } => {
            if let Some(c) = children.first() {
                outline(c, out);
            }
        }
        _ => {
            if let Some(b) = n.geometric_bounds() {
                out.subpaths.extend(shapes::rectangle(b).subpaths);
            }
        }
    }
}

/// The outline area type wraps around for wrap object `n`, in document coordinates, as
/// [`resolve_wrap`] makes it: a path's subpaths that are closed or have more than two anchors, those
/// of the shown members of a group or compound path, a clip group's clipping object's, and the
/// bounds of other art. Empty when `n` has none.
pub fn wrap_outline(n: &Node) -> PathData {
    let mut pd = PathData::default();
    outline(n, &mut pd);
    pd
}

enum Item<'a> {
    Wrap(&'a Node),
    Text(&'a Node),
}

fn collect<'a>(n: &'a Node, out: &mut Vec<Item<'a>>) {
    if !n.visible {
        return;
    }
    if n.extras().wrap.is_some() && !matches!(n.kind, NodeKind::Layer { .. }) {
        out.push(Item::Wrap(n));
        return;
    }
    match &n.kind {
        NodeKind::Text(t) if matches!(t.kind, TextKind::Area { .. }) => out.push(Item::Text(n)),
        NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => {
            for c in children {
                collect(c, out);
            }
        }
        _ => {}
    }
}

/// How much [`resolve_wrap`] would copy into the wrap shapes of every area text object in `doc`:
/// each wrap object's outline (its anchors, plus one for the shape) once for each area text object
/// below it in its layer.
pub fn wrap_work(doc: &Document) -> usize {
    let mut total = 0usize;
    for layer in &doc.layers {
        let mut items = vec![];
        collect(layer, &mut items);
        let mut above = 0usize;
        for it in items.iter().rev() {
            match it {
                Item::Wrap(n) => {
                    let mut pd = PathData::default();
                    outline(n, &mut pd);
                    if !pd.is_empty() {
                        let anchors: usize = pd.subpaths.iter().map(|sp| sp.anchors.len()).sum();
                        above = above.saturating_add(anchors.saturating_add(1));
                    }
                }
                Item::Text(_) => total = total.saturating_add(above),
            }
        }
    }
    total
}

/// Each area text object of `layer`, from the top down, with the wrap objects above it in the
/// layer whose outlines aren't empty, topmost first, each with the shape its outline makes in
/// document coordinates. Each wrap object's outline is made once.
fn each_text<'a>(layer: &'a Node, mut f: impl FnMut(&'a Node, &[(&'a Node, WrapShape)])) {
    let mut items = vec![];
    collect(layer, &mut items);
    let mut above: Vec<(&Node, WrapShape)> = vec![];
    for it in items.iter().rev() {
        match it {
            Item::Wrap(n) => {
                let mut pd = PathData::default();
                outline(n, &mut pd);
                if !pd.is_empty() {
                    above.push((n, WrapShape { path: pd, wrap: n.extras().wrap.unwrap_or_default() }));
                }
            }
            Item::Text(n) => f(n, &above),
        }
    }
}

/// The wrap shapes of text object `t` (text space) from `above`, the wrap objects above it as
/// [`each_text`] gives them: bottom first.
fn shapes_of(t: &TextObject, above: &[(&Node, WrapShape)]) -> Vec<WrapShape> {
    let inv = t.xf.inverse();
    above
        .iter()
        .rev()
        .map(|(_, s)| {
            let mut path = s.path.clone();
            path.transform(inv);
            WrapShape { path, wrap: s.wrap }
        })
        .collect()
}

/// Each area text object on `doc`'s layers that [`resolve_wrap`] gives wrap shapes, with the wrap
/// objects whose outlines make them, bottom first, and those shapes (text space), as
/// [`resolve_wrap`] works them out. The copying is [`wrap_work`].
pub fn wrap_sources<'a>(doc: &'a Document, mut f: impl FnMut(&'a Node, Vec<&'a Node>, Vec<WrapShape>)) {
    for layer in &doc.layers {
        each_text(layer, |n, above| {
            if let NodeKind::Text(t) = &n.kind
                && !above.is_empty()
            {
                f(n, above.iter().rev().map(|(s, _)| *s).collect(), shapes_of(t, above));
            }
        });
    }
}

/// Resolve the wrap shapes (`TextObject::wrap`, text space) of each area text object from the
/// outlines of the wrap objects above it in its layer. With `before`, the layers it holds unchanged
/// are skipped; without it, every layer is resolved. Returns the area text objects whose shapes
/// changed, whose cached bounds are then out of date.
pub fn resolve_wrap(before: Option<&Document>, doc: &mut Document) -> Vec<NodeId> {
    let mut updates: Vec<(NodeId, Vec<WrapShape>)> = vec![];
    for (idx, layer) in doc.layers.iter().enumerate() {
        if let Some(before) = before
            && (before.layers.get(idx).is_some_and(|b| Arc::ptr_eq(b, layer))
                || before.layers.iter().any(|b| b.id == layer.id && Arc::ptr_eq(b, layer)))
        {
            continue;
        }
        each_text(layer, |n, above| {
            if let NodeKind::Text(t) = &n.kind {
                let shapes = shapes_of(t, above);
                if shapes != t.wrap {
                    updates.push((n.id, shapes));
                }
            }
        });
    }
    let mut changed = Vec::with_capacity(updates.len());
    for (id, shapes) in updates {
        if let Some(NodeKind::Text(t)) = doc.node_mut(id).map(|n| &mut n.kind) {
            t.wrap = shapes;
            changed.push(id);
        }
    }
    changed
}
