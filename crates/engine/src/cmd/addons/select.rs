//! Selection Tools (`addon.select.*`), a built-in add-on: Select by Artboards selects the objects
//! overlapping, fully inside or fully outside one or more chosen artboards (#5525), and Select by
//! Link Status selects the images and placed documents whose file is linked, embedded, missing or
//! modified (#5526), saying which ones it had to leave out and why. Its submenu (Object › Plug-ins
//! › Selection Tools), the Links panel's menu items and the dialogs are the UI's.

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::{LockedArtboards, Node, NodeId, NodeKind};
use vectorcraft_geom::Point;

use super::super::links::{Status, statuses};
use super::super::select::{is_selectable, on_artboard, selectable};
use super::super::{CommandSpec, bad, checked_ids_param, cmd, has_doc, point_indices, switch_or, text_param};
use crate::{DocState, Result, Session};

/// How Select by Artboards relates an object to the chosen artboards (`relation`).
pub const RELATIONS: [&str; 3] = ["overlap", "inside", "outside"];

/// The link statuses Select by Link Status takes (`status`): a file found unchanged, pixels kept
/// in the document, a file not found, a file changed since it was read.
pub const LINK_STATUSES: [&str; 4] = ["linked", "embedded", "missing", "modified"];

/// The add-on's commands: Select by Artboards and Select by Link Status.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.select.byArtboards",
            "Select by Artboards",
            ["Object", "Plug-ins", "Selection Tools"],
            None,
            "{artboards: [index…] (0-based, at least one), relation?: \"overlap\" (default: on at least one of them, as select.allOnArtboard takes it: the bounds overlap the artboard or the centre is on it) | \"inside\" (the bounds lie wholly within one of them, edges included) | \"outside\" (on none of them)} select those objects among the ones Select All takes (in isolation mode, the isolated container's); hidden and locked objects, objects on hidden, locked or template layers and art held by a locked artboard are left out. Bounds are geometric, a symbol instance's measured by its art → {count}",
            has_doc,
            by_artboards
        ),
        cmd!(
            "addon.select.byLinkStatus",
            "Select by Link Status",
            ["Object", "Plug-ins", "Selection Tools"],
            None,
            "{status: \"linked\" (its file found unchanged) | \"embedded\" | \"missing\" | \"modified\" (its file changed since it was read) | [several of them] (at least one), ids?: [object ids] (pick only among these; default: every image and placed document in the layers), add?: false (add to the selection instead of replacing it), zoom?: false (then zoom to the selection: the desktop app runs addon.arrange.zoomSelection when something was selected; elsewhere the result's zoom flag asks the caller to)} select the image objects and placed documents whose link has that status, as links.list reports it. Those that are hidden or locked, inside a hidden or locked object or layer, on a template layer, on a locked artboard while nothing is isolated, or outside the isolated group or layer are left out and listed with the reason. Only the selection changes, not the document → {count (how many it selected), selected: [ids] (in stacking order, bottom first), skipped: [{id, reason: hidden|locked|template|isolation}], zoom?: true}",
            has_doc,
            by_link_status
        ),
    ]
}

/// `addon.select.byArtboards`: select the objects Select All takes that overlap, lie inside or lie
/// outside the chosen artboards.
fn by_artboards(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.select.byArtboards";
    let relation = text_param(p, "relation", C)?.unwrap_or("overlap");
    if !RELATIONS.contains(&relation) {
        return Err(bad(C, "`relation` must be overlap, inside or outside"));
    }
    let listed = p.get("artboards").ok_or_else(|| bad(C, "missing `artboards`"))?;
    let indices = point_indices(listed).ok_or_else(|| bad(C, "`artboards` must be a list of artboard indices"))?;
    if indices.is_empty() {
        return Err(bad(C, "choose at least one artboard"));
    }
    let st = s.doc()?;
    let rects =
        indices.iter().map(|i| st.doc.artboards.get(*i).map(|a| a.rect)).collect::<Option<Vec<_>>>().ok_or_else(|| bad(C, "no such artboard"))?;
    let symbols = st.doc.symbol_boxes();
    let ids: Vec<NodeId> = selectable(&st.doc, st.isolation)
        .into_iter()
        .filter(|id| {
            let Some(n) = st.doc.node(*id) else { return false };
            match relation {
                "inside" => n.geometric_bounds_with(&symbols).is_some_and(|b| {
                    rects.iter().any(|r| {
                        vectorcraft_doc::artboard_holds(*r, Point::new(b.x0, b.y0)) && vectorcraft_doc::artboard_holds(*r, Point::new(b.x1, b.y1))
                    })
                }),
                "outside" => !rects.iter().any(|r| on_artboard(n, &symbols, *r)),
                _ => rects.iter().any(|r| on_artboard(n, &symbols, *r)),
            }
        })
        .collect();
    s.select(|_, sel| sel.set(ids))?;
    Ok(json!({ "count": s.doc()?.selection.objects.len() }))
}

/// The link statuses `p` asks for (`status`: one, or a list of them), each once.
fn link_statuses(p: &Value, cmd: &str) -> Result<Vec<&'static str>> {
    let given: Vec<&Value> = match p.get("status") {
        None | Some(Value::Null) => return Err(bad(cmd, "missing `status`: linked, embedded, missing or modified")),
        Some(Value::Array(v)) => v.iter().collect(),
        Some(v) => vec![v],
    };
    let mut out = vec![];
    for v in given {
        let s = v.as_str().ok_or_else(|| bad(cmd, "`status` must be linked, embedded, missing or modified, or a list of them"))?;
        let known =
            LINK_STATUSES.iter().find(|k| **k == s).ok_or_else(|| bad(cmd, format!("status `{s}`: linked, embedded, missing or modified")))?;
        if !out.contains(known) {
            out.push(*known);
        }
    }
    if out.is_empty() {
        return Err(bad(cmd, "choose at least one status"));
    }
    Ok(out)
}

/// The link status of every image object and placed document in the layers of `st`, as
/// `links.list` reports it (`ok` named `linked`), in stacking order (bottom first). Each file is
/// looked at once.
pub(crate) fn link_status_of_objects(st: &DocState) -> Vec<(NodeId, &'static str)> {
    let linked = statuses(st);
    let mut out = vec![];
    st.doc.walk(|n| {
        if matches!(n.kind, NodeKind::Image(_) | NodeKind::PlacedDocument(_)) {
            let status = match linked.get(&n.id).map(|(s, ..)| *s) {
                Some(Status::Ok) => "linked",
                Some(Status::Modified) => "modified",
                Some(Status::Missing) => "missing",
                None => "embedded",
            };
            out.push((n.id, status));
        }
    });
    out
}

/// Why object `id` of `st` can't be selected now ([`is_selectable`]): it is outside the isolated
/// group or layer, on a template layer, hidden (or inside something hidden), or locked (or inside
/// something locked, or on a locked artboard); `None` when it can be. `locks` is the document's
/// [`vectorcraft_doc::Document::locked_artboards`].
fn skip_reason(st: &DocState, locks: &LockedArtboards, id: NodeId) -> Option<&'static str> {
    let d = &st.doc;
    if is_selectable(d, st.isolation, locks, id) {
        return None;
    }
    let ancestry = d.ancestry(id).unwrap_or_default();
    // Outside the isolated container, or the isolated container itself (its art is what's taken).
    if let Some(c) = st.isolation.and_then(|i| d.node(i))
        && ancestry.iter().position(|i| *i == c.id).is_none_or(|at| at + 1 == ancestry.len() && c.is_container())
    {
        return Some("isolation");
    }
    let held: Vec<&Node> = ancestry.iter().filter_map(|i| d.node(*i)).collect();
    Some(if held.iter().any(|n| n.is_template()) {
        "template"
    } else if held.iter().any(|n| !n.visible) {
        "hidden"
    } else {
        // A locked object, group or layer, or the locked artboard that holds it.
        "locked"
    })
}

/// `addon.select.byLinkStatus`: select the images and placed documents whose link has one of the
/// statuses asked for (among `ids` when given), replacing the selection or adding to it, and list
/// those left out with the reason ([`skip_reason`]).
fn by_link_status(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.select.byLinkStatus";
    let wanted = link_statuses(p, C)?;
    let add = switch_or(p, "add", false, C)?;
    let zoom = switch_or(p, "zoom", false, C)?;
    let only: Option<BTreeSet<NodeId>> = match p.get("ids") {
        None | Some(Value::Null) => None,
        Some(_) => Some(checked_ids_param(s, p, "ids", C)?.into_iter().collect()),
    };
    let st = s.doc()?;
    let locks = st.doc.locked_artboards();
    let (mut picked, mut skipped) = (vec![], vec![]);
    for (id, status) in link_status_of_objects(st) {
        if !wanted.contains(&status) || only.as_ref().is_some_and(|o| !o.contains(&id)) {
            continue;
        }
        match skip_reason(st, &locks, id) {
            None => picked.push(id),
            Some(reason) => skipped.push(json!({ "id": id.0, "reason": reason })),
        }
    }
    s.select(|_, sel| {
        if add {
            for id in &picked {
                sel.add(*id);
            }
        } else {
            sel.set(picked.iter().copied());
        }
    })?;
    let mut out = json!({ "count": picked.len(), "selected": picked.iter().map(|id| id.0).collect::<Vec<_>>(), "skipped": skipped });
    if zoom {
        out["zoom"] = json!(true);
    }
    Ok(out)
}
