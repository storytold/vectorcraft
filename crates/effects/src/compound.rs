//! Compound shapes ([`NodeKind::CompoundShape`]): the members combine by their shape modes into
//! one outline, evaluated live (Illustrator's compound shapes, Affinity's compounds). Each member
//! acts as one region: a path or compound path under its fill rule, type as its glyph outlines, a
//! group as the union of its members, a nested compound shape as its own outline, a live object
//! as its evaluated art. Members fold onto the bottom one in paint order ([`ShapeMode::fold`]), in
//! one sweep. The result is painted with the compound shape's own appearance.

use vectorcraft_doc::{Node, NodeKind, ShapeMode};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_pathops as po;

/// Most members a compound shape evaluates (the rest are left out), so a pasted or crafted file
/// can't make one sweep unbounded.
const MAX_MEMBERS: usize = 10_000;

/// The regions member `n` covers, each under its fill rule (several for a group or type).
fn member_regions(n: &Node, out: &mut Vec<(PathData, FillRule)>) {
    if !n.visible {
        return;
    }
    match &n.kind {
        NodeKind::Path { guide: true, .. } => {}
        NodeKind::Path { .. } | NodeKind::Compound { .. } => out.extend(crate::reshape::leaf_geometry(n)),
        NodeKind::CompoundShape { .. } => {
            if let Some(p) = compound_shape_path(n) {
                out.push((p, FillRule::NonZero));
            }
        }
        NodeKind::Text(t) => {
            let (_, all) = crate::reshape::glyph_outlines(t);
            if !all.elements().is_empty() {
                out.push((PathData::from_bezpath(&all).transformed(t.xf), FillRule::NonZero));
            }
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            // A clip group covers what its clipping path does.
            if n.clips() {
                if let Some(c) = children.first() {
                    member_regions(c, out);
                }
            } else {
                for c in children.iter().skip(usize::from(n.shaper.is_some())) {
                    member_regions(c, out);
                }
            }
        }
        _ if vectorcraft_doc::live::is_live(n) => member_regions(&crate::reshape::outlined_members(n), out),
        // Images, symbol instances: no region of their own.
        _ => {}
    }
}

fn finite(p: &PathData) -> bool {
    !p.is_empty() && p.subpaths.iter().all(|sp| sp.anchors.iter().all(|a| a.p.x.is_finite() && a.p.y.is_finite()))
}

/// The outline of compound shape `n` (filled non-zero); `None` when `n` isn't one. Members that
/// cover nothing take no part.
pub fn compound_shape_path(n: &Node) -> Option<PathData> {
    let NodeKind::CompoundShape { children } = &n.kind else { return None };
    let mut shapes: Vec<(PathData, FillRule)> = vec![];
    let mut modes: Vec<ShapeMode> = vec![];
    for c in children.iter().take(MAX_MEMBERS) {
        let mut parts = vec![];
        member_regions(c, &mut parts);
        parts.retain(|(p, _)| finite(p));
        let shape = match parts.len() {
            0 => continue,
            1 => parts.swap_remove(0),
            _ => {
                let refs: Vec<(&PathData, FillRule)> = parts.iter().map(|(p, r)| (p, *r)).collect();
                (po::unite_all(&refs), FillRule::NonZero)
            }
        };
        shapes.push(shape);
        modes.push(c.shape_mode);
    }
    if shapes.is_empty() {
        return Some(PathData::default());
    }
    let refs: Vec<(&PathData, FillRule)> = shapes.iter().map(|(p, r)| (p, *r)).collect();
    Some(po::boolean_n(&refs, |inside| ShapeMode::fold(inside, &modes)))
}

/// Compound shape `n` evaluated: a path (several subpaths for holes and pieces, filled non-zero)
/// with `n`'s id, name and appearance (its fills, strokes and effects). Its transparency and
/// opacity mask stay with the caller, as for other live objects. `None` when `n` isn't one.
pub fn evaluate_compound_shape(n: &Node) -> Option<Node> {
    let path = compound_shape_path(n)?;
    let mut out = Node::path(n.id, path, n.appearance.clone());
    out.name = n.name.clone();
    Some(out)
}

/// `art` (evaluated from `from`) with `from`'s visibility, lock, transparency and opacity mask.
pub fn carry_transparency(from: &Node, mut art: Node) -> Node {
    art.visible = from.visible;
    art.locked = from.locked;
    art.opacity = from.opacity;
    art.blend = from.blend;
    art.isolate = from.isolate;
    art.knockout = from.knockout;
    art.knockout_shape = from.knockout_shape;
    art.mask = from.mask.clone();
    art.attrs = from.attrs.clone();
    art.wrap = from.wrap;
    art.shape_mode = from.shape_mode;
    art
}
