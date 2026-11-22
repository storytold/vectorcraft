//! Add Inscribed Circle: the largest live circle inside each selected closed path or compound
//! path. The circle is the pathops crate's
//! ([`inscribed_circle`](vectorcraft_pathops::addons::pathtools::inscribed_circle)).

use serde_json::{Value, json};
use vectorcraft_doc::{Document, LiveShape, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, FillRule, PathData};
use vectorcraft_pathops::addons::pathtools::{self as pt, InscribedError};

use super::super::super::edit::selected_roots_to_transform;
use super::super::super::{CommandSpec, bad, cmd};
use super::bounding::whole_objects;
use crate::{Result, Session};

#[cfg(test)]
mod tests;

/// The command.
pub(crate) const COMMAND: &str = "addon.pathtools.inscribedCircle";
/// The undo step's name.
const LABEL: &str = "Add Inscribed Circle";
/// How far a live circle's four curves reach past a true circle, as a share of its radius: at
/// most 0.0273%, about 20° from each axis.
const CIRCLE_BULGE: f64 = 3e-4;

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.pathtools.inscribedCircle",
        "Add Inscribed Circle",
        ["Object", "Plug-ins", "Path Precision Tools"],
        None,
        "{} draw the largest live circle inside each selected closed path or compound path (a path of a compound path selected on its own counts as a path): the circle lies in the area the path fills under its fill rule, so holes and the overlaps of crossing subpaths count as the fill shows them, and before it is drawn its radius is within 0.02 pt of the largest (when the filled area's box is at least 10 pt across on its narrower side; for a smaller one, within a five-hundredth of that side; a curve whose control polygon is longer than 4096² times the flattening tolerance, half the search's precision (83,886 pt at 0.005 pt), is flattened more coarsely, and the circle can be smaller by as much; a curved hole narrower than the flattening tolerance can also make it smaller; a shape that flattening would close up or move off its curves is flattened finer, down to 1/65,536 of the tolerance). The drawn circle's four curves reach up to 0.03% of its radius past a true circle, so it is drawn 0.03% smaller and stays inside where it touches. Holes and gaps narrower than 1e-6 pt close up, and edges can move by as much, as in the other path operations. Strokes and effects are left out. The circles get the current paint and go on top of the current layer or of the isolated group, as other new art; inside an isolated compound path they join it and take its appearance; they are selected; one undo step. It is an error, with the document unchanged, when a selected object is not a closed path or compound path (\"object N is not a closed path or compound path\"; an open path has no inside), when it fills no area (\"object N fills no area\"), and when its outline has more than 262,144 anchors (in subpaths of two or more) or more than 262,144 straight pieces once flattened, or the searches of the selected objects together measure more than 134,217,728 distances from a point to an edge, or one search keeps more than 1,048,576 cells waiting, or the shape is so thin for its length that it flattens to no area even at the finest tolerance (\"object N is too complex to measure\") → {id: the first circle, ids}",
        whole_objects,
        inscribed_circle
    )]
}

/// The outline of path or compound path `id` and its fill rule; `None` for anything else, an open
/// path, or a compound path with an open member.
fn outline(d: &Document, id: NodeId) -> Option<(PathData, FillRule)> {
    let n = d.node(id)?;
    let (paths, rule): (Vec<&PathData>, FillRule) = match &n.kind {
        NodeKind::Path { path, rule, .. } => (vec![path], *rule),
        NodeKind::Compound { children, rule } => (children.iter().filter_map(|c| c.path_data()).collect(), *rule),
        _ => return None,
    };
    let subpaths: Vec<_> = paths.iter().flat_map(|p| p.subpaths.iter()).filter(|sp| !sp.anchors.is_empty()).collect();
    if subpaths.is_empty() || subpaths.iter().any(|sp| !sp.closed) {
        return None;
    }
    Some((PathData::new(subpaths.into_iter().cloned().collect()), rule))
}

fn inscribed_circle(s: &mut Session, _p: &Value) -> Result<Value> {
    // The roots leave out selected layers, which are not paths either.
    let st = s.doc()?;
    if let Some(layer) = st.selection.objects.iter().find(|id| st.doc.node(**id).is_some_and(Node::is_layer)) {
        return Err(bad(COMMAND, format!("object {} is not a closed path or compound path", layer.0)));
    }
    let roots = selected_roots_to_transform(s)?;
    let d = &s.doc()?.doc;
    let mut circles = vec![];
    // The searches share one work limit, so the command stays bounded however many objects are
    // selected.
    let mut work = pt::MAX_INSCRIBED_WORK;
    for id in roots {
        let value = id.0;
        let Some((path, rule)) = outline(d, id) else {
            return Err(bad(COMMAND, format!("object {value} is not a closed path or compound path")));
        };
        let c = pt::inscribed_circle_within(&path, rule, &mut work).map_err(|e| match e {
            InscribedError::NoArea => bad(COMMAND, format!("object {value} fills no area")),
            InscribedError::TooComplex => bad(COMMAND, format!("object {value} is too complex to measure")),
        })?;
        circles.push(c);
    }
    // Path art: inside an isolated compound path, the circles join it, as Tangent Circle's do.
    let parent = s.doc()?.target_parent_for_path()?;
    let look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    let ids = s.edit(LABEL, |d, sel| {
        let mut ids = vec![];
        for c in &circles {
            // Smaller by the curves' bulge, so the drawn circle stays inside where it touches.
            let (r, at) = (c.radius / (1.0 + CIRCLE_BULGE), c.center);
            let live = LiveShape::Ellipse { w: 2.0 * r, h: 2.0 * r, pie: (0.0, 360.0), xf: Affine::translate((at.x - r, at.y - r)) };
            let kind = NodeKind::Path { path: live.to_path(), rule: FillRule::NonZero, live: Some(Box::new(live)), clipping: false, guide: false };
            let mut n = Node::new(d.alloc_id(), kind);
            look.apply(d, &mut n);
            ids.push(n.id);
            d.insert(parent, usize::MAX, n)?;
        }
        sel.set(ids.iter().copied());
        Ok(ids)
    })?;
    let first = ids.first().map_or(0, |id| id.0);
    Ok(json!({ "id": first, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}
