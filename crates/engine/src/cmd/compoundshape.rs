//! Pathfinder panel › compound shapes (`object.compoundShape.*`): Make Compound Shape (what
//! Alt-clicking a Shape Mode button does), a member's shape mode, Release and Expand Compound
//! Shape. A compound shape is a group marked so ([`Node::is_compound_shape`]) whose members each
//! have a [`ShapeMode`]; `vectorcraft-effects` draws it as the area they make
//! ([`effects::compound_shape_area`]).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, ShapeMode};
use vectorcraft_render::effects;

use super::edit::{roots_of, selected_roots};
use super::pathops::{pf_leaves, shape_node, style_of};
use super::{CommandSpec, EngineError, Result, Session, bad, cmd, has_multi};

const MODE_NAMES: [&str; 4] = ["add", "subtract", "intersect", "exclude"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.compoundShape.make",
            "Make Compound Shape",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), mode?: \"add\"|\"subtract\"|\"intersect\"|\"exclude\" (default add; Alt-clicking a Shape Mode button gives its mode)} the two or more selected objects (paths, compound paths, groups, compound shapes, type) become the members of one compound shape, each with the shape mode, in their stacking order where the frontmost was: it draws as the area they make from the backmost up (open paths closed), painted with the appearance and transparency of the topmost member in Add, Intersect or Exclude mode (with every member in Subtract, the backmost), and the members stay editable (Direct Selection, Layers panel, isolation mode) → {id}",
            has_multi,
            make
        ),
        cmd!(
            "object.compoundShape.setMode",
            "Shape Mode",
            ["Window", "Pathfinder"],
            None,
            "{mode: \"add\"|\"subtract\"|\"intersect\"|\"exclude\", ids?: [id…] (selected first; default: the selection)} the members of compound shapes that the selected objects are or are in take the shape mode (what clicking a Shape Mode button does with members selected); the backmost member's mode doesn't change the shape → {ids}",
            has_member,
            set_mode
        ),
        cmd!(
            "object.compoundShape.release",
            "Release Compound Shape",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection)} the selected compound shapes (and those holding the selected objects) become their members again, as they were before Make Compound Shape, selected → {ids}",
            has_compound_shape,
            release
        ),
        cmd!(
            "object.compoundShape.expand",
            "Expand Compound Shape",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection)} the selected compound shapes (and those holding the selected objects) become the path, or compound path, of the area they draw, painted as they are, selected → {ids}",
            has_compound_shape,
            expand
        ),
    ]
}

/// The innermost compound shape holding `id` (not `id` itself), with the member of it that `id` is
/// or is in.
fn holder(d: &Document, id: NodeId) -> Option<(NodeId, NodeId)> {
    let up = d.ancestry(id)?;
    up.windows(2).rev().find_map(|w| match w {
        [shape, member] if d.node(*shape).is_some_and(Node::is_compound_shape) => Some((*shape, *member)),
        _ => None,
    })
}

/// The compound shape `id` is or is in (the innermost).
fn shape_of(d: &Document, id: NodeId) -> Option<NodeId> {
    if d.node(id)?.is_compound_shape() { Some(id) } else { holder(d, id).map(|(shape, _)| shape) }
}

/// The compound shapes the selection is or is in, outermost only, in paint order.
fn selected_shapes(s: &Session) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    let mut shapes: Vec<NodeId> = vec![];
    for id in st.selection.in_paint_order(&st.doc) {
        if let Some(shape) = shape_of(&st.doc, id).filter(|shape| !shapes.contains(shape)) {
            shapes.push(shape);
        }
    }
    Ok(roots_of(&st.doc, shapes))
}

/// The members of compound shapes that the selected objects are or are in.
fn selected_members(s: &Session) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    let mut members: Vec<NodeId> = vec![];
    for id in &st.selection.objects {
        if let Some((_, member)) = holder(&st.doc, *id).filter(|(_, m)| !members.contains(m)) {
            members.push(member);
        }
    }
    Ok(members)
}

/// The shape modes of the compound-shape members the selection is or is in (none when it is in
/// none): the Pathfinder panel lights the Shape Mode button they share.
pub fn selected_member_modes(s: &Session) -> Vec<ShapeMode> {
    let Some(st) = s.active() else { return vec![] };
    let members = selected_members(s).unwrap_or_default();
    members.iter().filter_map(|id| st.doc.node(*id)).map(|n| n.extras().shape_mode).collect()
}

/// The selection is or is in a compound shape.
fn has_compound_shape(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.selection.objects.iter().any(|id| shape_of(&st.doc, *id).is_some()) { Ok(()) } else { Err("select a compound shape".into()) }
}

/// The selection is or is in a member of a compound shape.
fn has_member(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.selection.objects.iter().any(|id| holder(&st.doc, *id).is_some()) { Ok(()) } else { Err("select members of a compound shape".into()) }
}

/// The `mode` param: one of [`MODE_NAMES`], or `default` when it is missing (`None`: required).
fn mode_param(p: &Value, cmd: &str, default: Option<ShapeMode>) -> Result<ShapeMode> {
    match super::object::choice(p, cmd, "mode", &MODE_NAMES)? {
        Some(name) => ShapeMode::parse(name),
        None => default,
    }
    .ok_or_else(|| bad(cmd, format!("`mode` must be one of {}", MODE_NAMES.join(", "))))
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

fn make(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.compoundShape.make";
    let mode = mode_param(p, C, Some(ShapeMode::Add))?;
    let roots = selected_roots(s)?;
    if roots.len() < 2 {
        return Err(bad(C, "select at least two objects"));
    }
    let d = &s.doc()?.doc;
    let mut members = Vec::with_capacity(roots.len());
    for id in &roots {
        let n = d.node(*id).ok_or(EngineError::NoNode(*id))?;
        if !matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } | NodeKind::Group { .. } | NodeKind::Text(_)) {
            return Err(bad(C, "compound shapes take paths, compound paths, groups, compound shapes and type"));
        }
        members.push(n);
    }
    // The paint and transparency of the topmost member in Add, Intersect or Exclude mode: as they
    // all have `mode`, the topmost, or the backmost when they all subtract.
    let order: Vec<&Node> = if mode == ShapeMode::Subtract { members } else { members.into_iter().rev().collect() };
    let style = order.into_iter().find_map(|n| {
        let mut lv = vec![];
        pf_leaves(n, &mut lv);
        lv.last().map(style_of)
    });
    let id = s.edit("Make Compound Shape", |d, sel| {
        let gid = d.alloc_id();
        let id = d.wrap(&roots, |children| {
            let children = children
                .into_iter()
                .map(|c| {
                    let mut c = Arc::unwrap_or_clone(c);
                    c.extras_mut().shape_mode = mode;
                    Arc::new(c)
                })
                .collect();
            let mut g = Node::group(gid, children);
            g.extras_mut().compound_shape = true;
            if let Some(style) = &style {
                style.apply(&mut g);
            }
            g
        })?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn set_mode(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.compoundShape.setMode";
    let mode = mode_param(p, C, None)?;
    let members = selected_members(s)?;
    if members.is_empty() {
        return Err(bad(C, "select members of a compound shape"));
    }
    // Members already in the mode leave the document and its undo history as they are.
    let d = &s.doc()?.doc;
    if members.iter().all(|id| d.node(*id).is_some_and(|n| n.extras().shape_mode == mode)) {
        return Ok(ids_json(&members));
    }
    s.edit("Shape Mode", |d, _| {
        for id in &members {
            d.node_mut(*id).ok_or(EngineError::NoNode(*id))?.extras_mut().shape_mode = mode;
        }
        Ok(())
    })?;
    Ok(ids_json(&members))
}

fn release(s: &mut Session, _: &Value) -> Result<Value> {
    let shapes = selected_shapes(s)?;
    if shapes.is_empty() {
        return Err(bad("object.compoundShape.release", "select a compound shape"));
    }
    let ids = s.edit("Release Compound Shape", |d, sel| {
        let (mut items, mut released) = (vec![], vec![]);
        for id in &shapes {
            let members = d.node(*id).and_then(Node::children).ok_or(EngineError::NoNode(*id))?;
            let out: Vec<Node> = members
                .iter()
                .map(|c| {
                    let mut c = (**c).clone();
                    c.extras_mut().shape_mode = ShapeMode::Add;
                    released.push(c.id);
                    c
                })
                .collect();
            items.push((*id, out));
        }
        d.insert_beside(items, true)?;
        sel.set(released.iter().copied());
        Ok(released)
    })?;
    Ok(ids_json(&ids))
}

fn expand(s: &mut Session, _: &Value) -> Result<Value> {
    const LABEL: &str = "Expand Compound Shape";
    let shapes = selected_shapes(s)?;
    if shapes.is_empty() {
        return Err(bad("object.compoundShape.expand", "select a compound shape"));
    }
    let ids = s.edit(LABEL, |d, sel| {
        let (mut items, mut made) = (vec![], vec![]);
        for id in &shapes {
            let shape = d.node(*id).cloned().ok_or(EngineError::NoNode(*id))?;
            let area = effects::compound_shape_area(&shape, effects::text_outliner())
                .ok_or_else(|| EngineError::Other("the compound shape is too complex to expand".into()))?;
            if area.is_empty() {
                return Err(EngineError::Other(format!("{LABEL} produced an empty result")));
            }
            let mut n = shape_node(d, area, None);
            let mut appearance = (*shape.appearance).clone();
            appearance.contents_index = None;
            n.appearance = appearance.into();
            (n.opacity, n.blend, n.name) = (shape.opacity, shape.blend, shape.name.clone());
            made.push(n.id);
            items.push((*id, vec![n]));
        }
        d.insert_beside(items, true)?;
        sel.set(made.iter().copied());
        Ok(made)
    })?;
    Ok(ids_json(&ids))
}
