//! Smooth Bridge: two selected circles joined into one outline by two concave arcs that touch both
//! circles, replacing the circles or added above them.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::PathData;
use vectorcraft_pathops::addons::pathtools as pt;

use super::super::super::pathops::{Beside, remove_each};
use super::super::super::{CommandSpec, bad, checked_ids_or_selection, cmd, has_selection, switch_or};
use super::given;
use crate::{EngineError, Result, Session};

#[cfg(test)]
mod tests;

/// The command, and its undo step's name.
const C: &str = "addon.pathtools.bridge";
const LABEL: &str = "Smooth Bridge";
/// The arcs' radius as a percentage of the smaller circle's radius: by default, and the range.
const RADIUS: f64 = 100.0;
const RADII: (f64, f64) = (1.0, 1000.0);

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.pathtools.bridge",
        "Smooth Bridge",
        ["Object", "Plug-ins", "Path Precision Tools"],
        None,
        "{radius?: % 1..1000 (100; the radius of the bridge's two arcs as a percentage of the smaller circle's radius: larger makes a wider, gentler bridge), keepOriginals?: bool (false: the outline replaces the two circles, painted as the upper one; true: it goes directly above the upper one and the circles stay), ids?: [id, id] (default: the selection)} join two circles (paths whose outline is a circle, not hidden or locked) into one closed outline: around each circle on its side away from the other, and along two concave arcs, one on each side, that touch both circles so the outline runs on smoothly into them. Overlapping and touching circles are joined too; one circle inside the other, or circles too far apart for the radius (the arcs would not reach both, or would cross), is an error, which suggests a larger radius when one up to 1000% would bridge them. One undo step; the outline is selected → {id}",
        has_selection,
        bridge
    )]
}

fn bridge(s: &mut Session, p: &Value) -> Result<Value> {
    let percent = given(p, "radius", Some(RADII), C)?.unwrap_or(RADIUS);
    let keep = switch_or(p, "keepOriginals", false, C)?;
    let roots = checked_ids_or_selection(s, p, C)?;
    let circles: Vec<(NodeId, pt::Circle)> = {
        let d = &s.doc()?.doc;
        let editable = d.editable();
        d.paint_order(roots.iter().copied())
            .into_iter()
            .filter_map(|id| match &d.node(id)?.kind {
                NodeKind::Path { path, guide: false, clipping: false, .. } if editable(id) => Some((id, pt::circle_of(path)?)),
                _ => None,
            })
            .collect()
    };
    let ([(below, a), (above, b)], 2) = (circles.as_slice(), roots.len()) else {
        return Err(bad(C, "select two circles to bridge"));
    };
    let (below, above) = (*below, *above);
    let at = |percent: f64| pt::smooth_bridge(*a, *b, a.radius.min(b.radius) * percent / 100.0);
    let outline = match at(percent) {
        Ok(sp) => PathData::single(sp),
        Err(pt::BridgeError::Inside) => return Err(bad(C, "one circle is inside the other")),
        // A larger radius bridges circles a smaller one cannot, so only suggest one that is allowed.
        Err(pt::BridgeError::TooFar) if at(RADII.1).is_ok() => {
            return Err(bad(C, "the circles are too far apart for this radius: raise the radius"));
        }
        Err(pt::BridgeError::TooFar) => return Err(bad(C, "the circles are too far apart for this radius")),
        Err(pt::BridgeError::Bad) => return Err(bad(C, "select two circles to bridge")),
    };
    let id = s.edit(LABEL, |d, sel| {
        let id = if keep {
            // A copy of the upper circle (its appearance) with the outline as its path, right above
            // it; the circles stay as they are, live shapes included.
            // Fresh ids throughout, its opacity mask's art included.
            let upper = d.node(above).cloned().ok_or(EngineError::NoNode(above))?;
            let mut n = d.reid(&upper);
            if let NodeKind::Path { path, live, .. } = &mut n.kind {
                *path = outline;
                *live = None;
            }
            let id = n.id;
            let mut beside = Beside::above(d, &[above]);
            beside.put(d, above, vec![n])?;
            beside.finish(d)?;
            id
        } else {
            // The upper circle becomes the outline (a plain path now) and the lower one goes.
            if let Some(NodeKind::Path { path, live, .. }) = d.node_mut(above).map(|n| &mut n.kind) {
                *path = outline;
                *live = None;
            }
            remove_each(d, &[below])?;
            above
        };
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}
