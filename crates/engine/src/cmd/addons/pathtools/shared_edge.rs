//! The Shared Edge Merge Tool's command and Keep Shared Points Together: with that option on, the
//! add-on's own commands that move an anchor (Slide Anchor Along Path, Extend Path, Set Anchor) take
//! the anchors coincident with it along.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};
use vectorcraft_doc::{AnchorRef, Document, NodeId, NodeKind};
use vectorcraft_geom::{PathData, Point, Vec2};
use vectorcraft_pathops::addons::pathtools as pt;
use vectorcraft_tools::addons::shared_edge::{DIAMETER, DIAMETER_RANGE, ID};

use super::super::super::{CommandSpec, always, bad, checked_id, cmd, finite_numbers, has_doc, number_or, switch_or, text_param};
use super::{edit_whole, set_path};
use crate::{Result, Session};

#[cfg(test)]
mod tests;

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.pathtools.mergeEdge",
            "Merge Shared Edge",
            [],
            None,
            "{a, b: two path ids (they needn't be selected), at: [x, y] (where), diameter?: pt 0.25..200 (20; how near `at` the edge is looked for), tolerance?: pt 0..3 (0.002; how near two anchors count as coincident), follow?: \"average\" (half way between the two)|\"a\"|\"b\" (keep that path's outline), ends?: bool (false: true makes an end even where the paths could merge)} where the paths touch at two coincident anchors, one each side of `at`, replace the stretches between them with one shared edge; stretches much longer one than the other (by over 10%) or leaving an end at over 120° aren't merged. Otherwise make an end there: two near anchors meet, and an anchor only one path has there is matched by a new one on the other. Each path stays closed or open; one undo step → {a, b, made: \"edge\"|\"end\"|\"nothing\" (merged already)}",
            has_doc,
            merge_edge
        ),
        cmd!(
            "addon.pathtools.movePointsTogether",
            "Keep Shared Points Together",
            [],
            None,
            "{enabled?: bool (default: the opposite of now)} Keep Shared Points Together: whether anchors coincident (within the Shared Edge Merge Tool's `tolerance`) with one that Slide Anchor Along Path, Extend Path or Set Anchor moves move with it, on any editable path (the `together` option of tool `sharedEdge`, kept with the preferences; off by default) → {enabled}",
            always,
            move_points_together
        ),
    ]
}

/// A path a merge may edit.
fn merge_path<'a>(d: &'a Document, id: NodeId, cmd: &str) -> Result<&'a PathData> {
    if !d.is_editable(id) {
        return Err(bad(cmd, "the path is locked or hidden"));
    }
    match d.node(id).map(|n| &n.kind) {
        Some(NodeKind::Path { path, guide: false, .. }) => Ok(path),
        _ => Err(bad(cmd, "`a` and `b` must be two different paths")),
    }
}

fn merge_edge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.mergeEdge";
    let a = checked_id(s, p.get("a").unwrap_or(&Value::Null), C)?;
    let b = checked_id(s, p.get("b").unwrap_or(&Value::Null), C)?;
    if a == b {
        return Err(bad(C, "`a` and `b` must be two different paths"));
    }
    let [x, y] = p.get("at").and_then(finite_numbers::<2>).ok_or_else(|| bad(C, "`at` must be [x, y]"))?;
    let at = Point::new(x, y);
    let diameter = number_or(p, "diameter", DIAMETER, C)?;
    if !(DIAMETER_RANGE.0..=DIAMETER_RANGE.1).contains(&diameter) {
        return Err(bad(C, "`diameter` must be from 0.25 to 200 pt"));
    }
    let tolerance = number_or(p, "tolerance", pt::COINCIDENCE, C)?;
    if !(0.0..=pt::MAX_COINCIDENCE).contains(&tolerance) {
        return Err(bad(C, "`tolerance` must be from 0 to 3 pt"));
    }
    let follow = match text_param(p, "follow", C)? {
        None => pt::Follow::Average,
        Some(f) => pt::Follow::parse(f).ok_or_else(|| bad(C, "`follow` must be \"average\", \"a\" or \"b\""))?,
    };
    let ends = switch_or(p, "ends", false, C)?;
    let d = &s.doc()?.doc;
    let (pa, pb) = (merge_path(d, a, C)?, merge_path(d, b, C)?);
    let radius = diameter / 2.0;
    let edge = if ends { None } else { pt::stretches_at(pa, pb, at, radius, tolerance) };
    let plan = edge.or_else(|| pt::ends_at(pa, pb, at, radius)).ok_or_else(|| bad(C, "the paths have nothing to merge there"))?;
    let (na, nb) = pt::apply_merge(pa, pb, &plan, follow).ok_or_else(|| bad(C, "the paths have nothing to merge there"))?;
    let changed: Vec<(NodeId, PathData)> =
        [(a, pa, na), (b, pb, nb)].into_iter().filter(|(_, old, new)| *old != new).map(|(id, _, new)| (id, new)).collect();
    let made = match plan {
        _ if changed.is_empty() => "nothing",
        pt::MergePlan::Edge { .. } => "edge",
        pt::MergePlan::Ends { .. } => "end",
    };
    edit_whole(s, "Merge Shared Edge", changed)?;
    Ok(json!({ "a": a.0, "b": b.0, "made": made }))
}

fn move_points_together(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.movePointsTogether";
    let now = s.tool_options_of(ID)["together"].as_bool().unwrap_or(false);
    let enabled = switch_or(p, "enabled", !now, C)?;
    s.set_tool_options(Some(ID), &Map::from_iter([("together".to_string(), json!(enabled))]));
    Ok(json!({ "enabled": enabled }))
}

/// Anchors to move along with others: each with how far.
pub(super) type SharedPoints = Vec<(NodeId, AnchorRef, Vec2)>;

/// With Keep Shared Points Together on, the anchors of editable paths coincident with an anchor
/// that `changed` (paths in their new form) moves, and not moving themselves. Subpaths whose
/// anchors `changed` adds or removes move none: their anchors stay put.
pub(super) fn shared_points<'a>(s: &Session, changed: impl IntoIterator<Item = (NodeId, &'a PathData)>) -> Result<SharedPoints> {
    let options = s.tool_options_of(ID);
    if options["together"].as_bool() != Some(true) {
        return Ok(vec![]);
    }
    let tol = options["tolerance"].as_f64().unwrap_or(pt::COINCIDENCE);
    let d = &s.doc()?.doc;
    let mut moved: BTreeSet<(NodeId, AnchorRef)> = BTreeSet::new();
    let mut from: Vec<(Point, Vec2)> = vec![];
    for (id, new) in changed {
        let Some(old) = d.node(id).and_then(|n| n.path_data()) else { continue };
        for (si, (o, n)) in old.subpaths.iter().zip(&new.subpaths).enumerate().filter(|(_, (o, n))| o.anchors.len() == n.anchors.len()) {
            for (ai, (x, y)) in o.anchors.iter().zip(&n.anchors).enumerate().filter(|(_, (x, y))| x.p != y.p) {
                moved.insert((id, (si, ai)));
                from.push((x.p, y.p - x.p));
            }
        }
    }
    let mut out = vec![];
    if from.is_empty() {
        return Ok(out);
    }
    from.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    let editable = d.editable();
    d.walk(|n| {
        let NodeKind::Path { path, guide: false, .. } = &n.kind else { return };
        if !editable(n.id) {
            return;
        }
        for (si, ai, a) in path.anchors() {
            let start = from.partition_point(|(p, _)| p.x < a.p.x - tol);
            let mut near = from.iter().skip(start).take_while(|(p, _)| p.x <= a.p.x + tol);
            if let Some((_, by)) = near.find(|(p, _)| p.distance(a.p) <= tol)
                && !moved.contains(&(n.id, (si, ai)))
            {
                out.push((n.id, (si, ai), *by));
            }
        }
    });
    Ok(out)
}

/// Move the anchors [`shared_points`] found; what they are on stops being a live shape.
pub(super) fn move_shared(d: &mut Document, shared: SharedPoints) {
    let mut by_path: BTreeMap<NodeId, Vec<(AnchorRef, Vec2)>> = BTreeMap::new();
    for (id, at, by) in shared {
        by_path.entry(id).or_default().push((at, by));
    }
    for (id, anchors) in by_path {
        let Some(mut np) = d.node(id).and_then(|n| n.path_data()).cloned() else { continue };
        for ((si, ai), by) in anchors {
            if let Some(a) = np.anchor_mut(si, ai) {
                a.translate(by);
            }
        }
        set_path(d, id, np);
    }
}
