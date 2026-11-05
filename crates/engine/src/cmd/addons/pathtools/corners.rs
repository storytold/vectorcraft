//! The Corner Tool's commands: the app's Live Corners ([`LiveCorners`]) cut, resized and removed on
//! chosen anchors, each edit one undo step.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::{LiveCorners, NodeId, NodeKind};
use vectorcraft_geom::shapes::CornerKind;
use vectorcraft_tools::addons::corner_tool::MAX_RADIUS;

use super::super::super::object::recut;
use super::super::super::{CommandSpec, bad, checked_id, checked_ids_or_selection, cmd, has_doc, number_or};
use super::{check_size, paths_under};
use crate::{Result, Session};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.pathtools.cornerSet",
            "Set Corners",
            [],
            None,
            "{ids?: [id…] (the selection), anchors?: [index…] (of each path's outline with no corners cut; else its direct-selected anchors, or all of them), items?: [{id, anchors?}…] (instead of ids: each path with its own anchors), radius: pt 0..1000000 (drawn no larger than fits), kind?: \"round\"|\"invertedRound\"|\"chamfer\" (each corner's own), every?: \"all\"|\"odd\"|\"even\" (default all: of those anchors, only the odd- or even-numbered ones, counting each path's anchors from 1)} cut live corners at the anchors that can have one, as one undo step → {ids, corners (how many)}",
            has_doc,
            corner_set
        ),
        cmd!(
            "addon.pathtools.cornerRemove",
            "Remove Corners",
            [],
            None,
            "{ids?: [id…] (the selection), anchors?: [index…] (of each path's outline with no corners cut; else its direct-selected anchors, or all of them), items?: [{id, anchors?}…] (instead of ids: each path with its own anchors)} make those live corners sharp again, as one undo step → {ids, corners (how many)}",
            has_doc,
            corner_remove
        ),
    ]
}

/// The `anchors` of `p`.
fn anchors_param(p: &Value, cmd: &str) -> Result<Option<BTreeSet<usize>>> {
    let Some(v) = p.get("anchors").filter(|v| !v.is_null()) else { return Ok(None) };
    let list = v.as_array().ok_or_else(|| bad(cmd, format!("`anchors` must list anchor indices (0, 1, 2…), not {v}")))?;
    list.iter()
        .map(|k| {
            k.as_u64()
                .and_then(|k| usize::try_from(k).ok())
                .ok_or_else(|| bad(cmd, format!("`anchors` must list anchor indices (0, 1, 2…), not {k}")))
        })
        .collect::<Result<_>>()
        .map(Some)
}

/// Which of the chosen anchors `cornerSet` takes, by their number along the path's uncut outline
/// (the first anchor is number 1).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Every {
    All,
    /// The 1st, 3rd, 5th… anchors: indices 0, 2, 4….
    Odd,
    /// The 2nd, 4th, 6th… anchors: indices 1, 3, 5….
    Even,
}

impl Every {
    /// The `every` of `p` (default: all).
    fn of(p: &Value, cmd: &str) -> Result<Self> {
        match p.get("every").filter(|v| !v.is_null()) {
            None => Ok(Self::All),
            Some(v) => match v.as_str() {
                Some("all") => Ok(Self::All),
                Some("odd") => Ok(Self::Odd),
                Some("even") => Ok(Self::Even),
                _ => Err(bad(cmd, format!("`every` must be \"all\", \"odd\" or \"even\", not {v}"))),
            },
        }
    }

    /// Does it take the anchor at `index`? Anchors that take no corner still count.
    fn takes(self, index: usize) -> bool {
        match self {
            Self::All => true,
            Self::Odd => index.is_multiple_of(2),
            Self::Even => !index.is_multiple_of(2),
        }
    }

    /// The error when none of the chosen corners is one it takes.
    fn none(self) -> &'static str {
        match self {
            Self::All => "choose anchors where two sides meet at an angle: only those take a corner",
            Self::Odd => "none of the chosen corners is at an odd-numbered anchor (the 1st, 3rd, 5th…)",
            Self::Even => "none of the chosen corners is at an even-numbered anchor (the 2nd, 4th, 6th…)",
        }
    }
}

/// The corners a request names: each path with the corners (anchor indices of its uncut outline)
/// that `every` takes.
fn named_corners(s: &Session, p: &Value, cmd: &str, every: Every) -> Result<Vec<(NodeId, BTreeSet<usize>)>> {
    let shared = anchors_param(p, cmd)?;
    // Each path with its own `anchors`, or sharing the request's.
    let mut own: Vec<Option<BTreeSet<usize>>> = vec![];
    let ids: Vec<NodeId> = match p.get("items").filter(|v| !v.is_null()) {
        None => paths_under(&s.doc()?.doc, &checked_ids_or_selection(s, p, cmd)?),
        Some(Value::Array(items)) => {
            let mut ids = vec![];
            for it in items {
                ids.push(checked_id(s, it.get("id").unwrap_or(&Value::Null), cmd)?);
                own.push(anchors_param(it, cmd)?);
            }
            ids
        }
        Some(v) => return Err(bad(cmd, format!("`items` must be a list of {{id, anchors?}}, not {v}"))),
    };
    let st = s.doc()?;
    let d = &st.doc;
    check_size(ids.iter().filter_map(|id| d.node(*id)?.path_data()), cmd)?;
    let editable = d.editable();
    let (mut out, mut chosen) = (vec![], false);
    for (i, id) in ids.iter().enumerate() {
        let Some(lc) = d.node(*id).filter(|_| editable(*id)).and_then(LiveCorners::of) else { continue };
        let corners: BTreeSet<usize> = match own.get(i).and_then(Option::as_ref).or(shared.as_ref()) {
            Some(given) => {
                let n = lc.base.anchor_count();
                if let Some(past) = given.iter().find(|k| **k >= n) {
                    return Err(bad(cmd, format!("object {id} has {n} anchors, so no anchor {past}")));
                }
                given.iter().copied().filter(|k| lc.corner(*k).is_some()).collect()
            }
            None => st.selection.partial(*id).filter(|a| !a.is_empty()).map_or_else(|| lc.all(), |a| lc.corners_of(a)),
        };
        chosen |= !corners.is_empty();
        let corners: BTreeSet<usize> = corners.into_iter().filter(|k| every.takes(*k)).collect();
        if !corners.is_empty() {
            out.push((*id, corners));
        }
    }
    if out.is_empty() {
        return Err(bad(cmd, if chosen { every.none() } else { Every::All.none() }));
    }
    Ok(out)
}

/// Cut `radius` and `kind` on `corners` as one step (none when they have them already).
fn edit_corners(s: &mut Session, label: &str, corners: Vec<(NodeId, BTreeSet<usize>)>, radius: f64, kind: Option<CornerKind>) -> Result<Value> {
    let d = &s.doc()?.doc;
    let same = corners.iter().all(|(id, picked)| {
        d.node(*id)
            .and_then(LiveCorners::of)
            .is_some_and(|c| picked.iter().all(|k| (c.drawn_radius(*k) - radius).abs() < 1e-9 && kind.is_none_or(|kd| c.kind(*k) == kd)))
    });
    let out = json!({
        "ids": corners.iter().map(|(id, _)| id.0).collect::<Vec<_>>(),
        "corners": corners.iter().map(|(_, c)| c.len()).sum::<usize>(),
    });
    if !same {
        s.edit(label, |d, sel| {
            for (id, picked) in corners {
                let Some(NodeKind::Path { path, live, .. }) = d.node_mut(id).map(|n| &mut n.kind) else { continue };
                // A cut changes the anchors, which is all a segment selection goes by.
                sel.segments.remove(&id);
                recut(path, live, sel.anchors.get_mut(&id), &picked, Some(radius), kind);
            }
            Ok(())
        })?;
    }
    Ok(out)
}

fn corner_set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.cornerSet";
    let radius = number_or(p, "radius", f64::NAN, C)?;
    if !(0.0..=MAX_RADIUS).contains(&radius) {
        return Err(bad(C, "`radius` must be from 0 to 1000000 pt"));
    }
    let kind = p
        .get("kind")
        .filter(|v| !v.is_null())
        .map(|v| {
            serde_json::from_value::<CornerKind>(v.clone())
                .map_err(|_| bad(C, format!("`kind` must be \"round\", \"invertedRound\" or \"chamfer\", not {v}")))
        })
        .transpose()?;
    let every = Every::of(p, C)?;
    let corners = named_corners(s, p, C, every)?;
    edit_corners(s, "Set Corners", corners, radius, kind)
}

fn corner_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let corners = named_corners(s, p, "addon.pathtools.cornerRemove", Every::All)?;
    edit_corners(s, "Remove Corners", corners, 0.0, Some(CornerKind::Round))
}
