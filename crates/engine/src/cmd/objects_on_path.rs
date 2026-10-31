//! Object › Objects on Path (#2545): objects attached to a path and spread along it.
//!
//! `object.objectsOnPath.attach` puts the selected objects and a path in an Objects on Path group:
//! a group with a [`ObjectsOnPath`] record ([`vectorcraft_doc::objects_on_path`]), the path at its
//! bottom. Called without a path (Object › Objects on Path › Attach) it chooses the Objects on Path
//! tool, whose click on a path attaches the selected objects to that path. The objects go along the
//! path column by column from left to right, top to bottom in each column, spread evenly from its
//! first anchor and turned to follow it. `object.objectsOnPath.options` sets the group's Pivot and
//! Rotate, and `object.objectsOnPath.expand` makes it a plain group.
//!
//! After every edit, [`refresh`] puts the objects of each Objects on Path group that changed back
//! on its path: editing the path moves them, an object moved on its own goes back to its place,
//! objects added to or taken out of the group join the end of the path or leave the record, and
//! art that replaces an object at its index (Break Link to Symbol) keeps its place.
//!
//! Behavior: <https://helpx.adobe.com/illustrator/using/objects-on-path.html> (the tool and
//! Object › Objects on Path › Attach; the order on the path, the attachment point, the distribution,
//! the objects squared to the path; Pivot and Rotate; path edits realign the objects; Expand).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::objects_on_path::{CENTER, Held, ObjectsOnPath, attachable_path, held, is_objects_on_path, place};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::Rect;

use super::edit::{roots_of, selected_roots};
use super::{CommandSpec, EngineError, Result, Session, bad, check_numbers, checked_id, checked_ids_param, cmd, has_selection};

/// The id of the Objects on Path tool.
pub const TOOL: &str = "objectsOnPath";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.objectsOnPath.attach",
            "Attach",
            ["Object", "Objects on Path"],
            None,
            "{path?: id of the path to attach to (without it: choose the Objects on Path tool, whose click on a path attaches the selection to it), ids?: the objects to attach (default: the selection; the path is left out of it), pivot?: 0-8 (the reference point of each object that attaches to the path, row by row from the top left; default 4, the center), rotate?: degrees counter-clockwise from upright on the path (default 0)} put the objects and the path in an Objects on Path group: the path at the bottom, the objects spread evenly along it from its first anchor (the last one at the end of an open path) in their order on the canvas, column by column from left to right and top to bottom in each column, each turned to follow the path; editing the path moves them → {id}",
            has_selection,
            attach
        ),
        cmd!(
            "object.objectsOnPath.options",
            "Objects on Path Options",
            [],
            None,
            "{ids?: Objects on Path groups or objects in them (default: the selection), pivot?: 0-8 (the reference point of each object that attaches to the path, row by row from the top left; 4: the center), rotate?: degrees counter-clockwise from upright on the path} set how the objects of the groups attach → {ids}",
            has_objects_on_path,
            options
        ),
        cmd!(
            "object.objectsOnPath.expand",
            "Expand",
            ["Object", "Objects on Path"],
            None,
            "{ids?: Objects on Path groups or objects in them (default: the selection)} make the groups plain groups of the path and the objects where they are → {ids}",
            has_objects_on_path,
            expand
        ),
    ]
}

// ---------- predicates ----------

/// The Objects on Path groups of `ids`: the ones among them, and the innermost one around each
/// object in one.
fn groups_of(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out = vec![];
    for id in ids {
        let group = d.ancestry(*id).unwrap_or_default().into_iter().rev().find(|a| d.node(*a).is_some_and(is_objects_on_path));
        if let Some(g) = group
            && !out.contains(&g)
        {
            out.push(g);
        }
    }
    out
}

/// The Objects on Path groups of the selection ([`groups_of`]).
fn selected_groups(s: &Session) -> Vec<NodeId> {
    s.active().map(|st| groups_of(&st.doc, &st.selection.objects)).unwrap_or_default()
}

/// The groups a command acts on: those of `ids` when given, otherwise those of the selection.
fn target_groups(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    if p.get("ids").is_none_or(Value::is_null) {
        return Ok(selected_groups(s));
    }
    let ids = checked_ids_param(s, p, "ids", cmd)?;
    let groups = groups_of(&s.doc()?.doc, &ids);
    if groups.is_empty() {
        return Err(bad(cmd, "`ids` must name Objects on Path groups or objects in them"));
    }
    Ok(groups)
}

fn has_objects_on_path(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    if selected_groups(s).is_empty() { Err("select an Objects on Path group".into()) } else { Ok(()) }
}

/// The Objects on Path group around `id` (not `id` itself), if any.
fn group_around(d: &Document, id: NodeId) -> Option<NodeId> {
    d.ancestry(id).unwrap_or_default().into_iter().filter(|a| *a != id).find(|a| d.node(*a).is_some_and(is_objects_on_path))
}

// ---------- Attach ----------

/// `pivot` and `rotate` from `p`, each checked.
fn pivot_rotate(p: &Value, cmd: &str, pivot: u8, rotate: f64) -> Result<(u8, f64)> {
    check_numbers(p, &["pivot", "rotate"], cmd)?;
    let pivot = match p.get("pivot").filter(|v| !v.is_null()) {
        None => pivot,
        Some(v) => v.as_u64().filter(|n| *n <= 8).map(|n| n as u8).ok_or_else(|| bad(cmd, "`pivot` must be a whole number from 0 to 8"))?,
    };
    let rotate = match p.get("rotate").and_then(Value::as_f64) {
        None => rotate,
        Some(r) if r.is_finite() => r,
        Some(_) => return Err(bad(cmd, "`rotate` must be a finite number")),
    };
    Ok((pivot, rotate))
}

/// `ids` in their order along the path: column by column from left to right, top to bottom in each
/// column. A column is a run of objects, taken by their left edges, that each start left of the
/// right edge of the column so far. Objects without bounds go last. Symbol instances are measured
/// by their art.
fn path_order(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let symbols = d.symbol_boxes();
    let mut placed: Vec<(NodeId, Rect)> = vec![];
    let mut rest = vec![];
    for id in ids {
        match d.node(*id).and_then(|n| n.geometric_bounds_with(&symbols)) {
            Some(b) => placed.push((*id, b)),
            None => rest.push(*id),
        }
    }
    placed.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0));
    let mut columns: Vec<(f64, Vec<(NodeId, Rect)>)> = vec![];
    for (id, b) in placed {
        match columns.last_mut() {
            Some((right, column)) if b.x0 < *right => {
                *right = right.max(b.x1);
                column.push((id, b));
            }
            _ => columns.push((b.x1, vec![(id, b)])),
        }
    }
    let mut out = vec![];
    for (_, mut column) in columns {
        column.sort_by(|a, b| a.1.y0.total_cmp(&b.1.y0));
        out.extend(column.into_iter().map(|(id, _)| id));
    }
    out.extend(rest);
    out
}

/// Why the path `path` can't take objects, if it can't.
fn path_refused(d: &Document, path: NodeId) -> Option<&'static str> {
    let n = d.node(path)?;
    if attachable_path(n).is_none() {
        return Some("pick a path to attach the objects to (not a guide, a compound path or type)");
    }
    if group_around(d, path).is_some() {
        return Some("the path is already in an Objects on Path group");
    }
    match held(d, path)? {
        Held::Live => Some("a path inside a compound path, blend, envelope, repeat or Intertwine group can't take objects"),
        Held::Clipping => Some("a clipping path can't take objects"),
    }
}

/// Why `n` can't be attached to a path, if it can't (the help page names graphs).
fn object_refused(d: &Document, n: &Node) -> Option<&'static str> {
    if n.is_layer() {
        return Some("layers can't be attached to a path: select the objects in them");
    }
    if group_around(d, n.id).is_some() {
        return Some("an object already on a path can't be attached again");
    }
    match held(d, n.id) {
        Some(Held::Live) => return Some("objects inside a blend, envelope, repeat or Intertwine group can't be attached to a path"),
        Some(Held::Clipping) => return Some("a clipping path can't be attached to a path"),
        None => {}
    }
    let mut graph = false;
    n.walk(&mut |c| graph |= c.extras().graph.is_some());
    graph.then_some("graphs can't be attached to a path")
}

fn attach(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.objectsOnPath.attach";
    let Some(path) = p.get("path").filter(|v| !v.is_null()) else {
        let view = s.last_view;
        s.select_tool(TOOL, view)?;
        return Ok(json!({ "tool": TOOL }));
    };
    let path = checked_id(s, path, C)?;
    let (pivot, rotate) = pivot_rotate(p, C, CENTER, 0.0)?;
    let roots = match p.get("ids") {
        Some(_) => roots_of(&s.doc()?.doc, checked_ids_param(s, p, "ids", C)?),
        None => selected_roots(s)?,
    };
    let ids: Vec<NodeId> = roots.into_iter().filter(|id| *id != path).collect();
    let order = {
        let d = &s.doc()?.doc;
        if let Some(why) = path_refused(d, path) {
            return Err(bad(C, why));
        }
        if ids.is_empty() {
            return Err(bad(C, "select the objects to attach to the path"));
        }
        for id in &ids {
            let n = d.node(*id).ok_or(EngineError::NoNode(*id))?;
            if d.ancestry(path).unwrap_or_default().contains(id) {
                return Err(bad(C, "the path can't be inside one of the objects to attach"));
            }
            if let Some(why) = object_refused(d, n) {
                return Err(bad(C, why));
            }
        }
        path_order(d, &ids)
    };
    let id = s.edit("Attach Objects on Path", |d, sel| {
        let members: Vec<Node> = order.iter().filter_map(|id| d.node(*id).cloned()).collect();
        let rec = ObjectsOnPath::new(path, &members.iter().collect::<Vec<_>>(), pivot, rotate);
        let mut wrapped = ids.clone();
        wrapped.push(path);
        let wrapped = d.paint_order(wrapped);
        let id = d.alloc_id();
        d.wrap(&wrapped, |mut children| {
            // The path at the bottom: the objects are drawn over it.
            if let Some(i) = children.iter().position(|c| c.id == path) {
                let p = children.remove(i);
                children.insert(0, p);
            }
            let mut g = Node::new(id, NodeKind::Group { children, clip: false });
            g.extras_mut().objects_on_path = Some(Box::new(rec));
            g
        })?;
        place_in(d, id);
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

// ---------- Options ----------

fn options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.objectsOnPath.options";
    let groups = target_groups(s, p, C)?;
    pivot_rotate(p, C, CENTER, 0.0)?;
    s.edit("Objects on Path Options", |d, _| {
        for g in &groups {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            let Some(rec) = n.extras().objects_on_path.as_deref().cloned() else { continue };
            let (pivot, rotate) = pivot_rotate(p, C, rec.pivot, rec.rotate)?;
            n.extras_mut().objects_on_path = Some(Box::new(ObjectsOnPath { pivot, rotate, ..rec }));
        }
        Ok(())
    })?;
    Ok(json!({ "ids": groups.iter().map(|g| g.0).collect::<Vec<_>>() }))
}

// ---------- Expand ----------

fn expand(s: &mut Session, p: &Value) -> Result<Value> {
    let groups = target_groups(s, p, "object.objectsOnPath.expand")?;
    s.edit("Expand Objects on Path", |d, sel| {
        for g in &groups {
            d.node_mut(*g).ok_or(EngineError::NoNode(*g))?.extras_mut().objects_on_path = None;
        }
        sel.set(groups.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "ids": groups.iter().map(|g| g.0).collect::<Vec<_>>() }))
}

// ---------- refresh ----------

/// After an edit: put the objects of every Objects on Path group that changed back on its path
/// ([`place`]), inner groups first. Only the parts of the tree that the edit changed are searched.
pub(crate) fn refresh(before: &Document, doc: &mut Document) {
    let mut groups = vec![];
    changed_groups(&before.layers, &doc.layers, &mut groups);
    for id in groups.iter().rev() {
        follow_replacements(before, doc, *id);
        place_in(doc, *id);
    }
}

/// Members of the group `id` that the edit replaced with new art at the same index keep their
/// place on the path ([`ObjectsOnPath::follow_replacements`]).
fn follow_replacements(before: &Document, doc: &mut Document, id: NodeId) {
    let (Some(old), Some(new)) = (before.node(id), doc.node(id)) else { return };
    let (Some(old), Some(new)) = (old.children(), new.children()) else { return };
    let Some(mut rec) = doc.node(id).and_then(|n| n.extras().objects_on_path.as_deref().cloned()) else { return };
    let unchanged = rec.clone();
    rec.follow_replacements(old, new);
    if rec != unchanged
        && let Some(n) = doc.node_mut(id)
    {
        n.extras_mut().objects_on_path = Some(Box::new(rec));
    }
}

/// [`place`] the Objects on Path group `id` of `doc`, measuring symbol instances by their art.
fn place_in(doc: &mut Document, id: NodeId) {
    let Some(n) = doc.node(id) else { return };
    let mut g = n.clone();
    if place(&mut g, &doc.symbol_boxes())
        && let Some(slot) = doc.node_mut(id)
    {
        *slot = g;
    }
}

/// The Objects on Path groups among `after` and inside them that are not the same nodes as before
/// (`before`: the same list before the edit), outer ones first.
fn changed_groups(before: &[Arc<Node>], after: &[Arc<Node>], out: &mut Vec<NodeId>) {
    let mut by_id: Option<HashMap<NodeId, &Arc<Node>>> = None;
    for (i, a) in after.iter().enumerate() {
        let b = match before.get(i) {
            Some(b) if b.id == a.id => Some(b),
            _ => by_id.get_or_insert_with(|| before.iter().map(|b| (b.id, b)).collect()).get(&a.id).copied(),
        };
        if b.is_some_and(|b| Arc::ptr_eq(b, a)) {
            continue;
        }
        if is_objects_on_path(a) {
            out.push(a.id);
        }
        if let Some(children) = a.children() {
            changed_groups(b.and_then(|b| b.children()).map_or(&[], Vec::as_slice), children, out);
        }
    }
}
