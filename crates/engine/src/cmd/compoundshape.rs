//! Compound shapes (`object.compoundShape.*`): live Shape Modes. Alt-click a Shape Mode in the
//! Pathfinder panel (or Make Compound Shape in its menu) combines the selected objects into a
//! compound shape whose members keep their own geometry and paint and combine by their own mode
//! ([`ShapeMode`]), evaluated on the fly (`vectorcraft_effects::evaluate_compound_shape`). A
//! member's mode changes from the Layers panel or by clicking a Shape Mode while it is selected
//! (Direct or Group Selection); Release gives the members back, Expand bakes the outline into a
//! path or compound path. (Illustrator's compound shapes, Affinity's compounds.)

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, ShapeMode};
use vectorcraft_render::effects;

use super::edit::roots_of;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.compoundShape.make",
            "Make Compound Shape",
            ["Object", "Compound Shape"],
            None,
            "{ids?: [id] (the selection), mode?: \"add\"|\"subtract\"|\"intersect\"|\"exclude\" (\"add\")} combine the objects (back → front: paths, compound paths, groups, type, compound shapes, live objects) into a live compound shape. Each member takes `mode` (subtract: the bottom one adds, the others subtract) and the compound takes the paint the matching Shape Mode keeps: the front-most object's (the back-most's for subtract). When every object is a member of a compound shape, sets their mode instead → {id} (or {count})",
            has_selection,
            make
        ),
        cmd!(
            "object.compoundShape.release",
            "Release Compound Shape",
            ["Object", "Compound Shape"],
            None,
            "{ids?: [id] (the selection)} the compound shapes give back their members, each with its own paint (and mode reset to add) → {ids} of the members",
            has_compound_shape,
            release
        ),
        cmd!(
            "object.compoundShape.expand",
            "Expand Compound Shape",
            ["Object", "Compound Shape"],
            None,
            "{ids?: [id] (the selection)} replace the compound shapes by their outline: a path, or a compound path when it has several pieces or holes, with the compound's paint and transparency → {ids}",
            has_compound_shape,
            expand
        ),
        cmd!(
            "object.compoundShape.setMode",
            "Shape Mode",
            ["Object", "Compound Shape"],
            None,
            "{ids?: [id] (the selection), mode: \"add\"|\"subtract\"|\"intersect\"|\"exclude\"} how the compound-shape members combine with the members below them (the bottom member's mode is ignored) → {count}",
            has_selection,
            set_mode
        ),
    ]
}

/// A compound shape is selected.
fn has_compound_shape(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    let st = s.active().ok_or("no document open")?;
    let any = st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::CompoundShape { .. })));
    if any { Ok(()) } else { Err("select a compound shape".into()) }
}

fn mode_param(cmd: &str, p: &Value, default: Option<ShapeMode>) -> Result<ShapeMode> {
    match p.get("mode") {
        None | Some(Value::Null) => default.ok_or_else(|| bad(cmd, "missing `mode`")),
        Some(v) => {
            v.as_str().and_then(ShapeMode::from_id).ok_or_else(|| bad(cmd, "`mode` must be \"add\", \"subtract\", \"intersect\" or \"exclude\""))
        }
    }
}

/// The objects a command acts on: `ids`, or the selection (in paint order).
fn targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    Ok(match ids_param(p, "ids") {
        Some(ids) => st.doc.paint_order(ids),
        None => st.selection.in_paint_order(&st.doc),
    })
}

/// Is `id` a member of a compound shape?
fn is_member(d: &Document, id: NodeId) -> bool {
    d.parent_of(id).and_then(|p| d.node(p)).is_some_and(|n| matches!(n.kind, NodeKind::CompoundShape { .. }))
}

/// Can `n` be a member of a compound shape (does it cover a region)?
fn can_be_member(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Path { guide, .. } => !guide,
        NodeKind::Compound { .. } | NodeKind::CompoundShape { .. } | NodeKind::Text(_) => true,
        NodeKind::Group { .. } => true,
        NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) => true,
        NodeKind::Layer { .. } | NodeKind::Image(_) | NodeKind::SymbolInstance { .. } | NodeKind::PlacedDocument(_) => false,
    }
}

/// The paint a compound shape made with `n` as the key object takes: its own appearance, a type
/// object's first characters' paint, a group's front-most member's.
fn paint_of(n: &Node) -> Appearance {
    match &n.kind {
        NodeKind::Text(t) if n.appearance.items.is_empty() => t.first_style().appearance(),
        NodeKind::Group { children, .. } if n.appearance.items.is_empty() => {
            children.iter().rev().filter(|c| c.visible).map(|c| paint_of(c)).find(|a| !a.items.is_empty()).unwrap_or_default()
        }
        _ => Appearance { items: n.appearance.items.clone(), ..Default::default() },
    }
}

fn set_modes(s: &mut Session, ids: &[NodeId], mode: ShapeMode) -> Result<Value> {
    let label = format!("Shape Mode: {}", mode.label());
    let count = s.edit(&label, |d, _| {
        let mut n = 0;
        for id in ids {
            if is_member(d, *id)
                && let Some(m) = d.node_mut(*id)
            {
                m.shape_mode = mode;
                n += 1;
            }
        }
        Ok(n)
    })?;
    Ok(json!({ "count": count }))
}

fn make(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.compoundShape.make";
    let mode = mode_param(C, p, Some(ShapeMode::Add))?;
    let picked = targets(s, p)?;
    // Members selected on their own (Direct or Group Selection): their mode changes.
    {
        let d = &s.doc()?.doc;
        if !picked.is_empty() && picked.iter().all(|id| is_member(d, *id)) {
            return set_modes(s, &picked, mode);
        }
    }
    let roots = roots_of(&s.doc()?.doc, picked);
    if roots.len() < 2 {
        return Err(EngineError::Other("Make Compound Shape: select two or more objects".into()));
    }
    let (Some(&bottom), Some(&top)) = (roots.first(), roots.last()) else {
        return Err(EngineError::Other("Make Compound Shape: select two or more objects".into()));
    };
    let id = s.edit("Make Compound Shape", |d, sel| {
        let mut members = vec![];
        for id in &roots {
            let n = d.node(*id).ok_or(EngineError::NoNode(*id))?;
            if !can_be_member(n) {
                return Err(EngineError::Other(
                    "compound shapes can only contain paths, compound paths, groups, type, compound shapes and live objects".into(),
                ));
            }
            members.push(n.clone());
        }
        // The paint the matching Shape Mode keeps.
        let key = d.node(if mode == ShapeMode::Subtract { bottom } else { top }).ok_or(EngineError::NoNode(top))?;
        let (appearance, opacity, blend) = (paint_of(key), key.opacity, key.blend);
        let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let cid = d.alloc_id();
        let children = members
            .into_iter()
            .enumerate()
            .map(|(i, mut m)| {
                m.shape_mode = if mode == ShapeMode::Subtract && i == 0 { ShapeMode::Add } else { mode };
                Arc::new(m)
            })
            .collect();
        let mut c = Node::new(cid, NodeKind::CompoundShape { children });
        (c.appearance, c.opacity, c.blend) = (appearance, opacity, blend);
        d.insert(par, idx + 1, c)?;
        for id in &roots {
            d.remove(*id)?;
        }
        sel.set([cid]);
        Ok(cid)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// The selected (or given) compound shapes; selected members stand for their compound shape.
fn compound_shapes(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let picked = targets(s, p)?;
    let d = &s.doc()?.doc;
    let mut out: Vec<NodeId> = vec![];
    for id in picked {
        let id = if is_member(d, id) && !matches!(d.node(id).map(|n| &n.kind), Some(NodeKind::CompoundShape { .. })) {
            d.parent_of(id).unwrap_or(id)
        } else {
            id
        };
        if d.node(id).is_some_and(|n| matches!(n.kind, NodeKind::CompoundShape { .. })) && !out.contains(&id) {
            out.push(id);
        }
    }
    if out.is_empty() {
        return Err(EngineError::Other("select a compound shape".into()));
    }
    Ok(out)
}

fn release(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = compound_shapes(s, p)?;
    let out = s.edit("Release Compound Shape", |d, sel| {
        let mut out = vec![];
        for id in &ids {
            let Some(n) = d.node(*id).cloned() else { continue };
            let NodeKind::CompoundShape { children } = &n.kind else { continue };
            let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
            d.remove(*id)?;
            for (k, c) in children.iter().enumerate() {
                let mut c = (**c).clone();
                c.shape_mode = ShapeMode::Add;
                out.push(c.id);
                d.insert(par, idx + k, c)?;
            }
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": out.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// Compound shape `n` as plain art with ids from `d`: a path, or a compound path for several
/// pieces, painted and blended like `n`. `None` when it covers nothing.
pub(crate) fn expanded(d: &mut Document, n: &Node) -> Option<Node> {
    let path = effects::compound_shape_path(n)?;
    if path.is_empty() {
        return None;
    }
    let art = super::pathops::shape_node(d, path, None);
    let mut art = effects::carry_transparency(n, Node { id: n.id, appearance: n.appearance.clone(), name: n.name.clone(), ..art });
    art.graphic_style = n.graphic_style;
    Some(art)
}

/// Every compound shape in the subtrees of `roots` (the outermost of nested ones) replaced by its
/// outline ([`expanded`]; one that covers nothing goes).
pub(crate) fn expand_under(d: &mut Document, roots: &[NodeId]) -> Result<()> {
    let mut found = vec![];
    for r in roots {
        let Some(n) = d.node(*r) else { continue };
        outermost(n, &mut found);
    }
    for id in found {
        let Some(n) = d.node(id).cloned() else { continue };
        match expanded(d, &n) {
            Some(art) => *d.node_mut(id).ok_or(EngineError::NoNode(id))? = art,
            None => {
                d.remove(id)?;
            }
        }
    }
    Ok(())
}

fn outermost(n: &Node, out: &mut Vec<NodeId>) {
    if matches!(n.kind, NodeKind::CompoundShape { .. }) {
        out.push(n.id);
        return;
    }
    for c in n.children().into_iter().flatten() {
        outermost(c, out);
    }
}

fn expand(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = compound_shapes(s, p)?;
    let out = s.edit("Expand Compound Shape", |d, sel| {
        let mut out = vec![];
        for id in &ids {
            let n = d.node(*id).cloned().ok_or(EngineError::NoNode(*id))?;
            let art = expanded(d, &n).ok_or_else(|| EngineError::Other("Expand Compound Shape: the compound shape is empty".into()))?;
            *d.node_mut(*id).ok_or(EngineError::NoNode(*id))? = art;
            out.push(*id);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": out.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn set_mode(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.compoundShape.setMode";
    let mode = mode_param(C, p, None)?;
    let ids = targets(s, p)?;
    if !ids.iter().any(|id| s.doc().is_ok_and(|st| is_member(&st.doc, *id))) {
        return Err(EngineError::Other("select members of a compound shape".into()));
    }
    set_modes(s, &ids, mode)
}

/// The Shape Mode a Pathfinder operation stands for.
pub(crate) fn mode_of(op: vectorcraft_pathops::PathfinderOp) -> Option<ShapeMode> {
    use vectorcraft_pathops::PathfinderOp as P;
    match op {
        P::Unite => Some(ShapeMode::Add),
        P::MinusFront => Some(ShapeMode::Subtract),
        P::Intersect => Some(ShapeMode::Intersect),
        P::Exclude => Some(ShapeMode::Exclude),
        _ => None,
    }
}

/// When every selected object is a compound-shape member, clicking a Shape Mode sets their mode
/// (`None`: run the operation as usual).
pub(crate) fn shape_mode_on_members(s: &mut Session, op: vectorcraft_pathops::PathfinderOp) -> Option<Result<Value>> {
    let mode = mode_of(op)?;
    let picked = targets(s, &json!({})).ok()?;
    let d = &s.doc().ok()?.doc;
    if picked.is_empty() || !picked.iter().all(|id| is_member(d, *id)) {
        return None;
    }
    Some(set_modes(s, &picked, mode))
}
