//! The Layers panel menu's operations on rows: Merge Selected, Flatten Artwork, Release to Layers,
//! Reverse Order, Template, Hide/Outline/Lock Others and their Show/Preview/Unlock All
//! counterparts, Locate Object. Rows default to the highlighted ones, else the current layer (see
//! [`super::layer`]).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};

use super::layer::{make_layer, row_roots, rows_param};
use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "layer.merge",
            "Merge Selected",
            ["Window", "Layers"],
            None,
            "{ids?} (default: the highlighted rows) merge two or more rows into the layer listed (highlighted) last: the other layers' contents and the other rows move into it, keeping their stacking order, and the emptied layers go. One undo step → {id}",
            has_doc,
            merge
        ),
        cmd!(
            "layer.flatten",
            "Flatten Artwork",
            ["Window", "Layers"],
            None,
            "{id?} (default: the highlighted row's top-level layer, else the current one's) move the art of every other visible top-level layer into this one, keeping the stacking order; hidden layers and their art are deleted, template layers stay. One undo step → {id, discarded}",
            has_doc,
            flatten
        ),
        cmd!(
            "layer.releaseToLayers",
            "Release to Layers (Sequence)",
            ["Window", "Layers"],
            None,
            "{id?: layer or group (default: the highlighted row, else the current layer), build?: bool} each object in it goes into a new sublayer of its own, in stacking order (a group's objects go into sublayers of its layer, where the group was). `build`: Release to Layers (Build), each new sublayer holds copies of the objects up to its own, so the layers add up. One undo step → {ids}",
            has_doc,
            release
        ),
        cmd!(
            "layer.releaseToLayersBuild",
            "Release to Layers (Build)",
            ["Window", "Layers"],
            None,
            "{id?} layer.releaseToLayers with build: true → {ids}",
            has_doc,
            |s, p| {
                let mut p = p.clone();
                if let Some(m) = p.as_object_mut() {
                    m.insert("build".into(), json!(true));
                } else {
                    p = json!({"build": true});
                }
                release(s, &p)
            }
        ),
        cmd!(
            "layer.reverse",
            "Reverse Order",
            ["Window", "Layers"],
            None,
            "{ids?} (default: the highlighted rows) reverse the stacking order of two or more rows that share a layer or group; each keeps the places the others held. One undo step",
            has_doc,
            reverse
        ),
        cmd!(
            "layer.template",
            "Template",
            ["Window", "Layers"],
            None,
            "{ids?, on?: bool (default: toggle the first)} make layers templates (locked, dimmed, left out of print and export) or ordinary layers again (layer.setProps template) → {on}",
            has_doc,
            template
        ),
        cmd!(
            "layer.hideOthers",
            "Hide Others",
            ["Window", "Layers"],
            None,
            "{ids?} hide every top-level layer but those holding the highlighted rows (else the current layer). One undo step → {count}",
            has_doc,
            |s, p| others(s, p, Flag::Visible, false)
        ),
        cmd!("layer.showAll", "Show All Layers", ["Window", "Layers"], None, "{} show every layer and sublayer → {count}", has_doc, |s, _| all(
            s,
            Flag::Visible,
            true
        )),
        cmd!(
            "layer.outlineOthers",
            "Outline Others",
            ["Window", "Layers"],
            None,
            "{ids?} turn Preview off (outline view) for every top-level layer but those holding the highlighted rows (else the current layer) → {count}",
            has_doc,
            |s, p| others(s, p, Flag::Preview, false)
        ),
        cmd!(
            "layer.previewAll",
            "Preview All Layers",
            ["Window", "Layers"],
            None,
            "{} turn Preview on for every layer and sublayer → {count}",
            has_doc,
            |s, _| all(s, Flag::Preview, true)
        ),
        cmd!(
            "layer.lockOthers",
            "Lock Others",
            ["Window", "Layers"],
            None,
            "{ids?} lock every top-level layer but those holding the highlighted rows (else the current layer) → {count}",
            has_doc,
            |s, p| others(s, p, Flag::Locked, true)
        ),
        cmd!(
            "layer.unlockAll",
            "Unlock All Layers",
            ["Window", "Layers"],
            None,
            "{} unlock every layer and sublayer → {count}",
            has_doc,
            |s, _| all(s, Flag::Locked, false)
        ),
        cmd!(
            "layer.locate",
            "Locate Object",
            ["Window", "Layers"],
            None,
            "{id?} (default: the last selected object) highlight the object's row; the Layers panel opens the layers around it and scrolls to it → {id, ancestry: [layer…, id]}",
            has_doc,
            locate
        ),
    ]
}

/// A layer flag the panel menu sets on many layers.
#[derive(Clone, Copy)]
enum Flag {
    Visible,
    Locked,
    Preview,
}

impl Flag {
    fn get(self, n: &Node) -> bool {
        match (self, &n.kind) {
            (Flag::Visible, _) => n.visible,
            (Flag::Locked, _) => n.locked,
            (Flag::Preview, NodeKind::Layer { preview, .. }) => *preview,
            (Flag::Preview, _) => true,
        }
    }
    fn set(self, n: &mut Node, v: bool) {
        match (self, &mut n.kind) {
            (Flag::Visible, _) => n.visible = v,
            (Flag::Locked, _) => n.locked = v,
            (Flag::Preview, NodeKind::Layer { preview, .. }) => *preview = v,
            (Flag::Preview, _) => {}
        }
    }
    fn label(self, on: bool) -> &'static str {
        match (self, on) {
            (Flag::Visible, true) => "Show All Layers",
            (Flag::Visible, false) => "Hide Others",
            (Flag::Locked, true) => "Lock Others",
            (Flag::Locked, false) => "Unlock All Layers",
            (Flag::Preview, true) => "Preview All Layers",
            (Flag::Preview, false) => "Outline Others",
        }
    }
}

/// Set `flag` to `v` on `ids` (layers that differ) in one undo step labelled `label` → how many
/// changed.
fn set_flag(s: &mut Session, ids: Vec<NodeId>, flag: Flag, v: bool, label: &str) -> Result<Value> {
    let d = &s.doc()?.doc;
    let ids: Vec<NodeId> = ids.into_iter().filter(|id| d.node(*id).is_some_and(|n| flag.get(n) != v)).collect();
    if !ids.is_empty() {
        s.edit(label, |d, sel| {
            for id in &ids {
                if let Some(n) = d.node_mut(*id) {
                    flag.set(n, v);
                }
            }
            sel.deselect_uneditable(d);
            Ok(())
        })?;
    }
    Ok(json!({ "count": ids.len() }))
}

/// The top-level layers holding the rows a command acts on.
fn kept_layers(s: &Session, p: &Value) -> Result<HashSet<NodeId>> {
    let ids = rows_param(s, p)?;
    let d = &s.doc()?.doc;
    Ok(ids.iter().filter_map(|id| d.layer_of(*id)).collect())
}

fn others(s: &mut Session, p: &Value, flag: Flag, v: bool) -> Result<Value> {
    let keep = kept_layers(s, p)?;
    let ids: Vec<NodeId> = s.doc()?.doc.layers.iter().map(|l| l.id).filter(|l| !keep.contains(l)).collect();
    set_flag(s, ids, flag, v, flag.label(v))
}

fn all(s: &mut Session, flag: Flag, v: bool) -> Result<Value> {
    let mut ids = vec![];
    s.doc()?.doc.walk(|n| {
        if n.is_layer() {
            ids.push(n.id);
        }
    });
    set_flag(s, ids, flag, v, flag.label(v))
}

fn template(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = rows_param(s, p)?;
    let d = &s.doc()?.doc;
    let ids: Vec<NodeId> = ids.into_iter().filter(|id| d.node(*id).is_some_and(Node::is_layer)).collect();
    let Some(first) = ids.first().and_then(|id| d.node(*id)) else { return Err(bad("layer.template", "select a layer")) };
    let on = bool_or(p, "on", !first.is_template());
    s.execute("layer.setProps", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "template": on}))?;
    Ok(json!({ "on": on }))
}

/// Move the contents of `others` (layers give their children, anything else itself; none inside
/// another) into layer `into`, keeping the document's stacking order, then delete the emptied
/// layers.
fn merge_into(d: &mut Document, into: NodeId, others: &[NodeId]) -> Result<()> {
    let own: Vec<NodeId> =
        d.node(into).and_then(|n| n.children()).map(|c| c.iter().map(|c| c.id).filter(|c| !others.contains(c)).collect()).unwrap_or_default();
    let mut moved = vec![];
    for id in others {
        let n = d.node(*id).ok_or(EngineError::NoNode(*id))?;
        if n.is_layer() {
            moved.extend(n.children().into_iter().flatten().map(|c| c.id));
        } else {
            moved.push(*id);
        }
    }
    // Where everything is now: the merged layer stacks its contents in this order.
    let keys: HashMap<NodeId, Vec<usize>> = own.iter().chain(&moved).filter_map(|id| Some((*id, d.index_path(*id)?))).collect();
    let mut taken = vec![];
    for id in &moved {
        taken.push(d.remove(*id)?);
    }
    for id in others {
        if d.node(*id).is_some() {
            d.remove(*id)?;
        }
    }
    let ch = d.children_mut(Some(into))?;
    let mut all = std::mem::take(ch);
    all.extend(taken);
    all.sort_by(|a, b| keys.get(&a.id).cmp(&keys.get(&b.id)));
    *ch = all;
    // The merged layers' guides go to it too.
    d.rehome_guides(into);
    Ok(())
}

fn merge(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.merge";
    let ids = rows_param(s, p)?;
    let st = s.doc()?;
    let d = &st.doc;
    let mut seen = HashSet::new();
    let ids: Vec<NodeId> = ids.into_iter().filter(|id| d.node(*id).is_some() && seen.insert(*id)).collect();
    if ids.len() < 2 {
        return Err(bad(C, "highlight two or more rows to merge"));
    }
    let into =
        ids.iter().rev().copied().find(|id| d.node(*id).is_some_and(Node::is_layer)).ok_or_else(|| bad(C, "highlight a layer to merge into"))?;
    let around = d.ancestry(into).unwrap_or_default();
    // (Rows inside the target layer merge too: only rows inside other listed rows go with those.)
    let rest: Vec<NodeId> = ids.iter().copied().filter(|id| *id != into).collect();
    let others = row_roots(d, &rest);
    if others.iter().any(|o| around.contains(o)) {
        return Err(bad(C, "a layer can't be merged into its own sublayer"));
    }
    s.edit("Merge Selected", |d, sel| {
        merge_into(d, into, &others)?;
        sel.prune(d);
        Ok(())
    })?;
    let st = s.doc_mut()?;
    st.active_layer = Some(into);
    st.layer_rows = vec![into];
    Ok(json!({ "id": into.0 }))
}

fn flatten(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.flatten";
    let st = s.doc()?;
    let d = &st.doc;
    let from = id_param(p, "id").or_else(|| st.highlighted_rows().last().copied()).or(st.current_layer());
    let into = from.and_then(|id| d.layer_of(id)).ok_or_else(|| bad(C, "no layer"))?;
    if !d.node(into).is_some_and(|n| n.visible) {
        return Err(EngineError::Other("the layer to flatten into is hidden".into()));
    }
    let others: Vec<NodeId> = d.layers.iter().filter(|l| l.id != into && l.visible && !l.is_template()).map(|l| l.id).collect();
    let hidden: Vec<NodeId> = d.layers.iter().filter(|l| l.id != into && !l.visible && !l.is_template()).map(|l| l.id).collect();
    s.edit("Flatten Artwork", |d, sel| {
        for h in &hidden {
            d.remove(*h)?;
        }
        // Hidden layers are discarded with their guides.
        d.drop_guides_of_deleted_layers(sel);
        merge_into(d, into, &others)?;
        sel.prune(d);
        Ok(())
    })?;
    let st = s.doc_mut()?;
    st.active_layer = Some(into);
    st.layer_rows = vec![into];
    Ok(json!({ "id": into.0, "discarded": hidden.len() }))
}

fn release(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.releaseToLayers";
    let build = bool_or(p, "build", false);
    let st = s.doc()?;
    let id = match id_param(p, "id") {
        Some(id) => id,
        None => match st.highlighted_rows().as_slice() {
            [id] => *id,
            _ => st.current_layer().ok_or_else(|| bad(C, "no layer"))?,
        },
    };
    let n = st.doc.node(id).ok_or(EngineError::NoNode(id))?;
    let group = match n.kind {
        NodeKind::Layer { .. } => false,
        NodeKind::Group { .. } => true,
        _ => return Err(bad(C, "release a layer or a group")),
    };
    // Its objects, bottom first (sublayers stay as they are).
    let objects: Vec<NodeId> = n.children().into_iter().flatten().filter(|c| !c.is_layer()).map(|c| c.id).collect();
    if objects.is_empty() {
        return Err(EngineError::Other("there are no objects to release".into()));
    }
    let new = s.edit(if build { "Release to Layers (Build)" } else { "Release to Layers (Sequence)" }, |d, sel| {
        // A group's objects go into sublayers of the layer holding it, where it was.
        let (parent, mut index) = if group {
            let layer = d.layer_containing(id).ok_or(EngineError::NoNode(id))?;
            let (par, i, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
            if par == Some(layer) { (layer, i) } else { (layer, usize::MAX) }
        } else {
            (id, 0)
        };
        let mut originals = vec![];
        for o in &objects {
            originals.push(Arc::unwrap_or_clone(d.remove(*o)?));
        }
        if group {
            d.remove(id)?;
        }
        let mut new = vec![];
        let last = originals.len().saturating_sub(1);
        for k in 0..originals.len() {
            let mut layer = make_layer(d, None);
            let lid = layer.id;
            let held: Vec<Node> = if build {
                // Copies of the objects below and itself; the top layer keeps the originals.
                (0..=k).filter_map(|j| originals.get(j)).map(|o| if k == last { o.clone() } else { d.reid(o) }).collect()
            } else {
                originals.get(k).cloned().into_iter().collect()
            };
            if let Some(ch) = layer.children_mut() {
                ch.extend(held.into_iter().map(Arc::new));
            }
            let at = if group { index } else { index.min(d.children(Some(parent)).map_or(0, Vec::len)) };
            d.insert(Some(parent), at, layer)?;
            index = at.saturating_add(1);
            new.push(lid);
        }
        sel.prune(d);
        Ok(new)
    })?;
    s.doc_mut()?.layer_rows = new.clone();
    Ok(json!({ "ids": new.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn reverse(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.reverse";
    let ids = rows_param(s, p)?;
    let d = &s.doc()?.doc;
    let ids = row_roots(d, &ids);
    // Rows grouped by the layer or group holding them (None: top-level layers).
    let mut by_parent: Vec<(Option<NodeId>, Vec<usize>)> = vec![];
    for id in &ids {
        let Some((par, i, _)) = d.position(*id) else { continue };
        match by_parent.iter_mut().find(|(p, _)| *p == par) {
            Some((_, v)) => v.push(i),
            None => by_parent.push((par, vec![i])),
        }
    }
    by_parent.retain(|(_, v)| v.len() >= 2);
    if by_parent.is_empty() {
        return Err(bad(C, "highlight two or more rows in one layer or group"));
    }
    s.edit("Reverse Order", |d, _| {
        for (par, mut slots) in by_parent {
            slots.sort_unstable();
            let ch = d.children_mut(par)?;
            let nodes: Vec<Arc<Node>> = slots.iter().filter_map(|i| ch.get(*i).cloned()).collect();
            for (slot, n) in slots.iter().zip(nodes.into_iter().rev()) {
                if let Some(c) = ch.get_mut(*slot) {
                    *c = n;
                }
            }
        }
        Ok(())
    })?;
    ok()
}

fn locate(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc_mut()?;
    let id = id_param(p, "id").or_else(|| st.selection.objects.last().copied()).ok_or_else(|| bad("layer.locate", "nothing selected"))?;
    let ancestry = st.doc.ancestry(id).ok_or(EngineError::NoNode(id))?;
    st.layer_rows = vec![id];
    st.revision += 1;
    Ok(json!({ "id": id.0, "ancestry": ancestry.iter().map(|i| i.0).collect::<Vec<_>>() }))
}
