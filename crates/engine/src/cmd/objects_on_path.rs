//! Object › Objects on Path (#2545): objects attached to a path and spread along it.
//!
//! `object.objectsOnPath.attach` puts the selected objects and a path in an Objects on Path group:
//! a group with a [`ObjectsOnPath`] record ([`vectorcraft_doc::objects_on_path`]), the path at its
//! bottom. Called without a path (Object › Objects on Path › Attach) it chooses the Objects on Path
//! tool, whose click on a path attaches the selected objects to that path. The objects go along the
//! path column by column from left to right, top to bottom in each column, spread evenly from its
//! first anchor and turned to follow it. Pivot and Rotate come from the command's params, otherwise
//! from the tool's options (Objects on Path Tool Options). `object.objectsOnPath.options` sets the
//! group's Pivot, Rotate and Space (the distance between the objects along the path, and where the
//! first one attaches), and `object.objectsOnPath.expand` makes it a plain group.
//! `object.objectsOnPath.selectMember` chooses one object of a selected group (its Select/Move
//! widget), and `object.objectsOnPath.moveMember` moves one object to another's place along the
//! path, the objects between it and that place each moving one place toward where it was; hidden
//! and locked objects keep their places. `object.objectsOnPath.detach` takes one object out of its
//! group, just above it, and the others move up the path.
//!
//! After every edit, [`refresh`] puts the objects of each Objects on Path group that changed back
//! on its path: editing the path moves them, an object moved on its own goes back to its place,
//! objects added to or taken out of the group join the end of the path or leave the record, and
//! art that replaces an object at its index (Break Link to Symbol) keeps its place.
//!
//! Behavior: <https://helpx.adobe.com/illustrator/using/objects-on-path.html> (the tool and
//! Object › Objects on Path › Attach; the order on the path, the attachment point, the distribution,
//! the objects squared to the path; Pivot and Rotate; path edits realign the objects; Detach and
//! the objects that move up the path after it; Expand).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::objects_on_path::{CENTER, Held, ObjectsOnPath, attachable_path, held, is_objects_on_path, place};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::Rect;

use super::edit::{roots_of, selected_roots};
use super::{CommandSpec, EngineError, Result, Session, bad, check_numbers, checked_id, checked_ids_param, cmd, has_doc, has_selection};

/// The id of the Objects on Path tool.
pub const TOOL: &str = "objectsOnPath";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.objectsOnPath.attach",
            "Attach",
            ["Object", "Objects on Path"],
            None,
            "{path?: id of the path to attach to (without it: choose the Objects on Path tool, whose click on a path attaches the selection to it), ids?: the objects to attach (default: the selection; the path is left out of it), pivot?: 0-8 (the reference point of each object that attaches to the path, row by row from the top left; default: the Objects on Path tool's Pivot, 4, the center, until its options change it), rotate?: degrees counter-clockwise from upright on the path (default: the tool's Rotate, 0 until its options change it)} put the objects and the path in an Objects on Path group: the path at the bottom, the objects spread evenly along it from its first anchor (the last one at the end of an open path) in their order on the canvas, column by column from left to right and top to bottom in each column, each turned to follow the path; editing the path moves them → {id}",
            has_selection,
            attach
        ),
        cmd!(
            "object.objectsOnPath.options",
            "Objects on Path Options",
            [],
            None,
            "{ids?: Objects on Path groups or objects in them (default: the selection), pivot?: 0-8 (the reference point of each object that attaches to the path, row by row from the top left; 4: the center), rotate?: degrees counter-clockwise from upright on the path, spacing?: points along the path from each object's attachment point to the next one's (0 or more, kept to what the path has room for; or \"auto\", the default: spread evenly with the last object at the end of an open path), start?: points along the path from its first anchor to the first object's attachment point (default 0)} set how the objects of the groups attach and are spaced → {ids}",
            has_objects_on_path,
            options
        ),
        cmd!(
            "object.objectsOnPath.selectMember",
            "Select Object on Path",
            [],
            None,
            "{member: id of an object of an Objects on Path group} select the group alone with that object chosen, as a click on the object's Select/Move widget does; any other selection change clears the choice → {group, member}",
            has_doc,
            select_member
        ),
        cmd!(
            "object.objectsOnPath.moveMember",
            "Move Object on Path",
            [],
            None,
            "{member: id of an object of an Objects on Path group, target: id of another object of the same group, group?: the group's id (when given, it must be the group of both); neither object nor the group may be hidden or locked} move the object to the target's place along the path; the objects between them each move one place toward where it was, except hidden and locked objects, which keep their places. Only the order along the path changes: Space, the start, Pivot, Rotate and the order in the Layers panel stay. A target that is the object itself changes nothing → {group, member, index}",
            has_doc,
            move_member
        ),
        cmd!(
            "object.objectsOnPath.detach",
            "Detach Object from Path",
            [],
            None,
            "{member?: id of an object of an Objects on Path group (default: the object chosen with the selected group's Select/Move widget); neither the object nor the group may be hidden or locked} take one object off the path, out of its group and just above the group in the Layers panel, where it stays as drawn and is selected alone. The group's other objects move up the path in their order: with Auto Space they spread evenly again, and a Space value and the start are kept. The group and its path stay, even with one object or none left → {group, member}",
            has_doc,
            detach
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

/// The Objects on Path groups of the selection ([`groups_of`]). None while an object of a dynamic
/// symbol instance's art is selected.
fn selected_groups(s: &Session) -> Vec<NodeId> {
    s.active().filter(|st| st.selection.symbol_part.is_none()).map(|st| groups_of(&st.doc, &st.selection.objects)).unwrap_or_default()
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
    super::edit::no_symbol_part(s)?;
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
    // Without them in `p`, the tool's options (Objects on Path Tool Options) give Pivot and Rotate.
    let saved = s.tool_options_of(TOOL);
    let tool_pivot = saved.get("pivot").and_then(Value::as_u64).filter(|n| *n <= 8).map_or(CENTER, |n| n as u8);
    let tool_rotate = saved.get("rotate").and_then(Value::as_f64).filter(|r| r.is_finite()).map_or(0.0, |r| r.clamp(-360.0, 360.0));
    let (pivot, rotate) = pivot_rotate(p, C, tool_pivot, tool_rotate)?;
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

/// `spacing` and `start` from `p`, each checked: `spacing` takes points (0 or more) or `"auto"`
/// (`None`), and `start` points along the path.
fn spacing_start(p: &Value, cmd: &str, spacing: Option<f64>, start: f64) -> Result<(Option<f64>, f64)> {
    let spacing = match p.get("spacing").filter(|v| !v.is_null()) {
        None => spacing,
        Some(Value::String(a)) if a == "auto" => None,
        Some(v) => Some(
            v.as_f64()
                .filter(|g| g.is_finite() && *g >= 0.0)
                .ok_or_else(|| bad(cmd, "`spacing` must be a number of points, 0 or more, or \"auto\""))?,
        ),
    };
    let start = match p.get("start").filter(|v| !v.is_null()) {
        None => start,
        Some(v) => v.as_f64().filter(|s| s.is_finite()).ok_or_else(|| bad(cmd, "`start` must be a finite number"))?,
    };
    Ok((spacing, start))
}

fn options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.objectsOnPath.options";
    let groups = target_groups(s, p, C)?;
    pivot_rotate(p, C, CENTER, 0.0)?;
    spacing_start(p, C, None, 0.0)?;
    s.edit("Objects on Path Options", |d, _| {
        for g in &groups {
            let n = d.node_mut(*g).ok_or(EngineError::NoNode(*g))?;
            let Some(rec) = n.extras().objects_on_path.as_deref().cloned() else { continue };
            let (pivot, rotate) = pivot_rotate(p, C, rec.pivot, rec.rotate)?;
            let (spacing, start) = spacing_start(p, C, rec.spacing, rec.start)?;
            n.extras_mut().objects_on_path = Some(Box::new(ObjectsOnPath { pivot, rotate, spacing, start, ..rec }));
        }
        Ok(())
    })?;
    Ok(json!({ "ids": groups.iter().map(|g| g.0).collect::<Vec<_>>() }))
}

// ---------- Select/Move ----------

/// The Objects on Path group whose object `key` of `p` names, checked: an object listed in the
/// group's record (not its path), in a group that is not hidden or locked.
fn member_param(s: &Session, p: &Value, key: &str, cmd: &str) -> Result<(NodeId, NodeId)> {
    let v = p.get(key).filter(|v| !v.is_null()).ok_or_else(|| bad(cmd, format!("missing `{key}`")))?;
    let id = checked_id(s, v, cmd)?;
    let d = &s.doc()?.doc;
    let not_on_path = || bad(cmd, format!("`{key}` must name an object attached to the path of an Objects on Path group"));
    let g =
        d.ancestry(id).unwrap_or_default().into_iter().rev().nth(1).filter(|g| d.node(*g).is_some_and(is_objects_on_path)).ok_or_else(not_on_path)?;
    let listed = d.node(g).and_then(|n| n.extras().objects_on_path.as_deref()).is_some_and(|rec| rec.members.iter().any(|m| m.id == id));
    if !listed {
        return Err(not_on_path());
    }
    if !d.is_editable(g) {
        return Err(bad(cmd, "the Objects on Path group is hidden or locked"));
    }
    if !d.is_editable(id) {
        return Err(bad(cmd, format!("`{key}` names an object that is hidden or locked")));
    }
    Ok((g, id))
}

fn select_member(s: &mut Session, p: &Value) -> Result<Value> {
    let (g, member) = member_param(s, p, "member", "object.objectsOnPath.selectMember")?;
    s.select(|_, sel| sel.set_on_path_member(g, member))?;
    Ok(json!({ "group": g.0, "member": member.0 }))
}

fn move_member(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.objectsOnPath.moveMember";
    let (g, member) = member_param(s, p, "member", C)?;
    let (target_group, target) = member_param(s, p, "target", C)?;
    if target_group != g {
        return Err(bad(C, "`target` must be an object of the same Objects on Path group as `member`"));
    }
    if let Some(v) = p.get("group").filter(|v| !v.is_null())
        && checked_id(s, v, C)? != g
    {
        return Err(bad(C, "`group` must be the Objects on Path group of `member` and `target`"));
    }
    let at = |s: &Session| -> Result<usize> {
        let rec = s.doc()?.doc.node(g).and_then(|n| n.extras().objects_on_path.as_deref().cloned()).ok_or(EngineError::NoNode(g))?;
        rec.members.iter().position(|m| m.id == member).ok_or(EngineError::NoNode(member))
    };
    // Onto its own place: nothing changes, and no undo step is recorded.
    if target == member {
        return Ok(json!({ "group": g.0, "member": member.0, "index": at(s)? }));
    }
    // Hidden and locked objects between them keep their places.
    let index = s.edit("Move Object on Path", |d, _| {
        let mut rec = d.node(g).and_then(|n| n.extras().objects_on_path.as_deref().cloned()).ok_or(EngineError::NoNode(g))?;
        let fixed: HashSet<NodeId> = {
            let editable = d.editable();
            rec.members.iter().map(|m| m.id).filter(|&id| !editable(id)).collect()
        };
        let index = rec.move_member(member, target, |id| fixed.contains(&id)).ok_or(EngineError::NoNode(member))?;
        let n = d.node_mut(g).ok_or(EngineError::NoNode(g))?;
        n.extras_mut().objects_on_path = Some(Box::new(rec));
        Ok(index)
    })?;
    Ok(json!({ "group": g.0, "member": member.0, "index": index }))
}

// ---------- Detach ----------

/// The object chosen with the Select/Move widget of the one selected Objects on Path group.
fn chosen_member(s: &Session) -> Option<NodeId> {
    let st = s.active()?;
    match st.selection.objects[..] {
        [_] => st.selection.on_path_member,
        _ => None,
    }
}

fn detach(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.objectsOnPath.detach";
    const CHOOSE: &str = "choose the object to detach with its Select/Move widget";
    let (g, member) = if p.get("member").is_none_or(Value::is_null) {
        let chosen = chosen_member(s).ok_or_else(|| bad(C, CHOOSE))?;
        let found = member_param(s, &json!({ "member": chosen.0 }), "member", C)?;
        if s.active().is_none_or(|st| st.selection.objects[..] != [found.0]) {
            return Err(bad(C, CHOOSE));
        }
        found
    } else {
        member_param(s, p, "member", C)?
    };
    // The object goes just above the group, in the group's parent, and keeps where it is drawn.
    // When the group is itself an object on another group's path, the object goes above that
    // group instead (and so on outward), so it does not join that path. The refresh after the edit
    // removes it from the group's record and moves the others up the path.
    s.edit("Detach Object from Path", |d, sel| {
        let mut above = g;
        while let Some(p) = d.parent_of(above).filter(|p| d.node(*p).is_some_and(is_objects_on_path)) {
            above = p;
        }
        let (parent, index, _) = d.position(above).ok_or(EngineError::NoNode(above))?;
        d.move_node(member, parent, index + 1)?;
        sel.set([member]);
        Ok(())
    })?;
    Ok(json!({ "group": g.0, "member": member.0 }))
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
/// Returns the groups that placing changed.
pub(crate) fn refresh(before: &Document, doc: &mut Document) -> Vec<NodeId> {
    let mut groups = vec![];
    changed_groups(&before.layers, &doc.layers, &mut groups);
    let mut placed = vec![];
    for id in groups.iter().rev() {
        follow_replacements(before, doc, *id);
        if place_in(doc, *id) {
            placed.push(*id);
        }
    }
    placed
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
/// Returns whether the group changed.
fn place_in(doc: &mut Document, id: NodeId) -> bool {
    let Some(n) = doc.node(id) else { return false };
    let mut g = n.clone();
    if place(&mut g, &doc.symbol_boxes())
        && let Some(slot) = doc.node_mut(id)
    {
        *slot = g;
        return true;
    }
    false
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
