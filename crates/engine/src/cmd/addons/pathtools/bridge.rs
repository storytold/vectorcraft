//! Smooth Bridge: two selected circles joined into one outline by two concave arcs that touch both
//! circles, or three or more each joined the same way to the others within reach and united where
//! the bridges connect, replacing the circles or added above them.

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
/// The gap (pt) added to the smaller circle's radius to give the arcs' radius: the range.
const GAPS: (f64, f64) = (0.0, 1000.0);
/// How far apart (pt) the outlines of two of three or more circles may be for a bridge between
/// them: by default, and the range.
const REACH: f64 = 10.0;
const REACHES: (f64, f64) = (0.0, 10000.0);

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.pathtools.bridge",
        "Smooth Bridge",
        ["Object", "Plug-ins", "Path Precision Tools"],
        None,
        "{radius?: % 1..1000 (100; the radius of each bridge's two arcs as a percentage of the smaller circle's radius: larger makes a wider, gentler bridge), gap?: pt 0..1000 (instead of radius: the arcs' radius is the smaller circle's radius plus this gap, so each arc stays this gap away from a circle of the smaller circle's size about the arc's center; for equal circles this gap apart that is where a third circle this gap from both sits, as in a triangular grid, so the channels around an empty place in such a grid are one width; giving both is an error), reach?: pt 0..10000 (10; with three or more circles: how far apart two circles' outlines may be for a bridge between them, overlapping ones included; two circles are bridged at any distance), keepOriginals?: bool (false: each outline replaces its circles, painted as the topmost of them; true: it goes directly above that circle and the circles stay), ids?: [id] (default: the selection)} join circles (paths whose outline is a circle, not hidden or locked) into metaball (aka meatball) shapes. Two circles become one closed outline: around each circle on its side away from the other, and along two concave arcs, one on each side, that touch both circles so the outline runs on smoothly into them. With three or more, every two circles within reach are bridged the same way, and the bridges that connect are united into one shape each (holes smaller than 1% of the smallest circle's area are filled); a pair within reach but too far apart for the radius gets no bridge, and so does one whose neck would be thinner than the precision its shape is united at, and a circle bridged to no other is left as it is. Overlapping and touching circles are joined too. One circle inside another (among three or more, any such pair within reach) is an error; so are two circles too far apart for the radius (the arcs would not reach both, or would cross), or three or more with no pair the arcs can join, which suggests a larger radius or gap when the largest one would bridge them; so are no two circles within reach, more than 2000 circles and more than 8000 bridges. One undo step; the outlines (and the circles left as they are) are selected → {ids, alone}, with {id} too for two circles",
        has_selection,
        bridge
    )]
}

fn bridge(s: &mut Session, p: &Value) -> Result<Value> {
    let percent = given(p, "radius", Some(RADII), C)?;
    let gap = given(p, "gap", Some(GAPS), C)?;
    let reach = given(p, "reach", Some(REACHES), C)?.unwrap_or(REACH);
    if percent.is_some() && gap.is_some() {
        // As the parameter check words it (`EXCLUSIVE_KEYS`), for a caller that skips that check.
        let (key, other) = ("radius", "gap");
        return Err(bad(C, format!("give `{key}` or `{other}`, not both")));
    }
    let arc = match gap {
        Some(gap) => pt::ArcRadius::Gap(gap),
        None => pt::ArcRadius::Share(percent.unwrap_or(RADIUS) / 100.0),
    };
    let keep = switch_or(p, "keepOriginals", false, C)?;
    let mut roots = checked_ids_or_selection(s, p, C)?;
    // An id given twice is one circle.
    roots.sort_unstable();
    roots.dedup();
    if roots.len() > pt::MAX_BRIDGED_CIRCLES {
        return Err(bad(C, "select at most 2000 circles to bridge"));
    }
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
    if circles.len() != roots.len() || circles.len() < 2 {
        return Err(bad(C, "select two or more circles to bridge"));
    }
    let only: Vec<pt::Circle> = circles.iter().map(|(_, c)| *c).collect();
    // Each shape: the circles it joins, in paint order, and its outline.
    let (shapes, alone): (Vec<(Vec<NodeId>, PathData)>, Vec<NodeId>) = match only.as_slice() {
        [a, b] => match pt::smooth_bridge(*a, *b, arc.between(*a, *b)) {
            Ok(sp) => (vec![(roots_of(&circles, &[0, 1]), PathData::single(sp))], vec![]),
            Err(e) => return Err(refusal(e, arc, |largest| pt::smooth_bridge(*a, *b, largest.between(*a, *b)).is_ok())),
        },
        _ => {
            let made = pt::bridge_many(&only, arc, reach).map_err(|e| match e {
                pt::ManyError::TooManyCircles => bad(C, "select at most 2000 circles to bridge"),
                pt::ManyError::TooManyBridges => bad(C, "that makes too many bridges (at most 8000): lower the reach"),
                pt::ManyError::NoneInReach => bad(C, "no two circles are within reach: raise the reach"),
                pt::ManyError::TooComplex => bad(C, "the bridged circles are too complex to unite"),
                // Under the many-circle rules: a pair the largest radius or gap bridges on its own
                // can still have a neck too thin to unite among the others.
                pt::ManyError::Pair(_, _, e) => refusal(e, arc, |largest| pt::bridges_any(&only, largest, reach)),
                pt::ManyError::Bad => bad(C, "select two or more circles to bridge"),
            })?;
            let shapes = made.joined.into_iter().map(|j| (roots_of(&circles, &j.members), j.outline)).collect();
            (shapes, roots_of(&circles, &made.alone))
        }
    };
    let made = s.edit(LABEL, |d, sel| {
        let mut made = Vec::with_capacity(shapes.len());
        if keep {
            // A copy of each shape's topmost circle (its appearance) with the outline as its path,
            // right above it; the circles stay as they are, live shapes included. Fresh ids
            // throughout, an opacity mask's art included.
            let tops: Vec<NodeId> = shapes.iter().filter_map(|(members, _)| members.last().copied()).collect();
            let mut beside = Beside::above(d, &tops);
            for ((_, outline), top) in shapes.into_iter().zip(tops) {
                let upper = d.node(top).cloned().ok_or(EngineError::NoNode(top))?;
                let mut n = d.reid(&upper);
                if let NodeKind::Path { path, live, .. } = &mut n.kind {
                    *path = outline;
                    *live = None;
                }
                made.push(n.id);
                beside.put(d, top, vec![n])?;
            }
            beside.finish(d)?;
        } else {
            // Each shape's topmost circle becomes its outline (a plain path now) and the others go.
            let mut gone = vec![];
            for (members, outline) in shapes {
                let Some((&top, rest)) = members.split_last() else { continue };
                if let Some(NodeKind::Path { path, live, .. }) = d.node_mut(top).map(|n| &mut n.kind) {
                    *path = outline;
                    *live = None;
                }
                made.push(top);
                gone.extend_from_slice(rest);
            }
            remove_each(d, &gone)?;
        }
        sel.set(made.iter().chain(&alone).copied());
        Ok(made)
    })?;
    let mut out = json!({ "ids": made.iter().map(|id| id.0).collect::<Vec<_>>(), "alone": alone.iter().map(|id| id.0).collect::<Vec<_>>() });
    if let ([id], 2) = (made.as_slice(), only.len()) {
        out["id"] = json!(id.0);
    }
    Ok(out)
}

/// The ids of `circles` at indices `at`, in paint order (the circles are listed in paint order).
fn roots_of(circles: &[(NodeId, pt::Circle)], at: &[usize]) -> Vec<NodeId> {
    at.iter().filter_map(|&i| circles.get(i)).map(|(id, _)| *id).collect()
}

/// The error for circles that arcs of `arc` can't bridge. A larger radius or gap bridges circles a
/// smaller one cannot, so it suggests one only when `bridges` says the largest allowed would.
fn refusal(e: pt::BridgeError, arc: pt::ArcRadius, bridges: impl FnOnce(pt::ArcRadius) -> bool) -> EngineError {
    let largest = match arc {
        pt::ArcRadius::Share(_) => pt::ArcRadius::Share(RADII.1 / 100.0),
        pt::ArcRadius::Gap(_) => pt::ArcRadius::Gap(GAPS.1),
    };
    let would = e == pt::BridgeError::TooFar && bridges(largest);
    match (e, arc, would) {
        (pt::BridgeError::Inside, ..) => bad(C, "one circle is inside the other"),
        (pt::BridgeError::TooFar, pt::ArcRadius::Share(_), true) => bad(C, "the circles are too far apart for this radius: raise the radius"),
        (pt::BridgeError::TooFar, pt::ArcRadius::Share(_), false) => bad(C, "the circles are too far apart for this radius"),
        (pt::BridgeError::TooFar, pt::ArcRadius::Gap(_), true) => bad(C, "the circles are too far apart for this gap: raise the gap"),
        (pt::BridgeError::TooFar, pt::ArcRadius::Gap(_), false) => bad(C, "the circles are too far apart for this gap"),
        (pt::BridgeError::Bad, ..) => bad(C, "select two or more circles to bridge"),
    }
}
