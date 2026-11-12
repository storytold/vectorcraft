//! Window › 3D and Materials › Quick Actions › Expand as Wireframes: each object with a 3D effect
//! becomes a group of stroked paths, the edges of the faces its effect draws.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::appearance::AppearanceItem;
use vectorcraft_doc::{Appearance, LineJoin, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_render::effects;

use super::{CommandSpec, Result, Session, bad, checked_ids_or_selection, cmd, has_doc};

const C: &str = "effect.expandAsWireframes";

/// The name of the undo step.
const LABEL: &str = "Expand as Wireframes";

/// The weight of a wireframe edge's stroke, in points (VectorCraft's own).
pub const WIRE_WEIGHT: f64 = 0.5;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "effect.expandAsWireframes",
        "Expand as Wireframes",
        [],
        None,
        "{ids?: [..] (default: the selected objects)} replace each object that has a visible 3D effect (Extrude & Bevel, Extrude & Bevel (Classic), Revolve, Inflate or Rotate, on the object or on its fills and strokes) with a group of paths: the edges of the faces the effect draws, as the object is turned and projected now, each edge once (edges that meet end to end make one path, closed where it comes back to its start; curved edges stay curves; geometry effects after the 3D effect, on the object or on a fill or stroke, reshape them too), stroked 0.5 pt black with round joins and no fill (VectorCraft's own); the group takes the object's place, id, name, opacity, blending mode, opacity mask, attributes, slice and text wrap and is selected (a layer stays a layer holding the edges, which are selected); objects without a visible 3D effect are left as they are; one undo step. An error, with nothing changed, when no object is selected or given, when none has a visible 3D effect, when an object's solid can't be built or the canvas draws its source art in its place, or when one object's wireframe has more than 1,000,000 edges → {ids}",
        has_doc,
        expand_as_wireframes
    )]
}

/// A wireframe edge's appearance: a 0.5 pt black stroke with round joins and no fill.
fn wire_appearance() -> Appearance {
    let mut stroke = StrokeLayer::new(Paint::solid(Color::BLACK), WIRE_WEIGHT);
    stroke.join = LineJoin::Round;
    Appearance { items: vec![AppearanceItem::Stroke(stroke)], ..Default::default() }
}

fn expand_as_wireframes(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = checked_ids_or_selection(s, p, C)?;
    if roots.is_empty() {
        return Err(bad(C, "select an object with a 3D effect, or give its id in ids"));
    }
    // Every wireframe is built before the edit, so a failure leaves the document and its history as
    // they are.
    let d = &s.doc()?.doc;
    let mut wires = vec![];
    for id in roots {
        let Some(n) = d.node(id) else { continue };
        if !effects::has_revolve(n) && !effects::has_solid(n) {
            continue;
        }
        let wire = effects::three_d_wireframe_in(Some(d), n).map_err(|e| bad(C, format!("object {}: {e}", id.0)))?;
        wires.push((id, wire));
    }
    if wires.is_empty() {
        return Err(bad(C, "no selected or given object has a visible 3D effect"));
    }
    let ids: Vec<NodeId> = s.edit(LABEL, |d, sel| {
        let mut out = vec![];
        let mut selected = vec![];
        for (id, wire) in wires {
            let Some(source) = d.node(id) else { continue };
            if matches!(source.kind, NodeKind::Layer { .. }) {
                // A layer stays a layer: its members become the edges, and its 3D effect is removed.
                let mut layer = source.clone();
                let children: Vec<Arc<Node>> = wire.into_iter().map(|path| Arc::new(Node::path(d.alloc_id(), path, wire_appearance()))).collect();
                selected.extend(children.iter().map(|c| c.id));
                if let Some(members) = layer.children_mut() {
                    *members = children;
                }
                layer.appearance = Default::default();
                if let Some(slot) = d.node_mut(id) {
                    *slot = layer;
                    out.push(id);
                }
                continue;
            }
            // The group keeps the object's name, visibility, lock, bounding box angle, transparency
            // (opacity, blending mode, isolation, knockout and opacity mask), attributes, slice and text
            // wrap.
            let mut group = Node::group(id, vec![]);
            group.name = source.name.clone();
            group.visible = source.visible;
            group.locked = source.locked;
            group.opacity = source.opacity;
            group.blend = source.blend;
            group.isolate = source.isolate;
            group.knockout = source.knockout;
            group.knockout_shape = source.knockout_shape;
            group.bbox_angle = source.bbox_angle;
            let extras = source.extras();
            let (mask, attrs, slice, wrap) = (extras.mask.clone(), extras.attrs.clone(), extras.slice.clone(), extras.wrap);
            if mask.is_some() || attrs.is_some() || slice.is_some() || wrap.is_some() {
                let mut e = group.extras_mut();
                e.mask = mask;
                e.attrs = attrs;
                e.slice = slice;
                e.wrap = wrap;
            }
            let children = wire.into_iter().map(|path| Arc::new(Node::path(d.alloc_id(), path, wire_appearance()))).collect();
            group.kind = NodeKind::Group { children, clip: false };
            if let Some(slot) = d.node_mut(id) {
                *slot = group;
                out.push(id);
                selected.push(id);
            }
        }
        sel.set(selected);
        Ok(out)
    })?;
    Ok(json!({ "ids": ids.iter().map(|id| id.0).collect::<Vec<_>>() }))
}

#[cfg(test)]
mod tests;
