//! Stencil Bridges (`addon.cutting.bridges`), a command of the built-in Cutting Tools add-on: it
//! cuts bridges out of the selected art so that a stencil cut from it holds together. Each island
//! of the stencil sheet (the inside of an O, a hole in a shape) stays joined to the sheet by strips
//! of a set width across the art (<https://en.wikipedia.org/wiki/Stencil> describes islands and
//! bridges). The art becomes one compound path with the gaps cut, and type is measured by its
//! glyph outlines. The original goes to a new hidden layer or is replaced. The geometry is
//! `vectorcraft_pathops::addons::stencil`.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_pathops::addons::stencil::{BRIDGE_WIDTH_RANGE, BridgeDirection, BridgeSpec, MAX_BRIDGES_PER_ISLAND, stencil_bridges};

use super::super::super::edit::selected_roots;
use super::super::super::object::choice;
use super::super::super::pathops::{length_given, node_path, pf_leaves, shape_node, style_of};
use super::super::super::{CommandSpec, bad, cmd, has_selection, number_or, switch_or};
use crate::{EngineError, Result, Session};

/// Stencil Bridges' command (the id in [`specs`]).
pub const BRIDGES: &str = "addon.cutting.bridges";
/// The ways bridges run, as `direction` takes them.
pub const DIRECTIONS: [&str; 3] = ["vertical", "horizontal", "shortest"];
/// A bridge's width when none is given, in points.
pub const DEFAULT_WIDTH: f64 = 2.0;
/// The name of the hidden layer the original art goes to (numbered when it is taken).
const ORIGINAL_LAYER: &str = "Stencil Original";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.cutting.bridges",
        "Stencil Bridges…",
        ["Object", "Plug-ins", "Cutting Tools"],
        None,
        "{width?: pt 0.01..1000 (2), count?: whole number 1..16 of bridges each island gets (1; fewer where it has no room and more where other islands are joined to the sheet through it), direction?: \"vertical\" | \"horizontal\" | \"shortest\" (the shortest crossing among 12 directions 15° apart) (\"vertical\"), keepOriginal?: true (the selected objects that make up the art go to a new hidden layer named \"Stencil Original\"; false: they are replaced)} cut bridges out of the selected art so that a stencil cut from it holds together: every island (an area the art encloses, such as the inside of an O or a hole in a shape) is joined to the area around the art by strips `width` wide across the art, the shortest crossings first, then spread around the island. The art is measured by the area its outlines enclose, whatever their paint (open paths closed as a fill closes them; type by its glyph outlines; groups by their members; hidden objects left out) and becomes one compound path with the gaps cut, painted as the topmost selected shape, in the place of the topmost selected object that has outlines. Selected objects with no outlines in the art stay where they are. Art with no islands, or none a bridge of the width fits, is left as it is, with a warning and no undo step; islands the cut result still encloses are counted in `unbridged`, with a warning. Art that reaches more than 10000000 pt from the origin, has more than 200000 anchors, whose combined area has more than 4000 outlines, whose outlines need more than 200000 straight segments, or that takes more work to measure than a fixed budget allows is refused. One undo step; the new path is selected → {id, islands, bridges, unbridged, original (the hidden layer's id, or null), warning?}",
        has_selection,
        bridges
    )]
}

fn bridges(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = BRIDGES;
    let width = length_given(p, "width", C)?.unwrap_or(DEFAULT_WIDTH);
    let (lo, hi) = BRIDGE_WIDTH_RANGE;
    if !(lo..=hi).contains(&width) {
        return Err(bad(C, format!("`width` must be from {lo} to {hi} pt")));
    }
    let count = number_or(p, "count", 1.0, C)?;
    if count.fract() != 0.0 || !(1.0..=MAX_BRIDGES_PER_ISLAND as f64).contains(&count) {
        return Err(bad(C, format!("`count` must be a whole number from 1 to {MAX_BRIDGES_PER_ISLAND}")));
    }
    let direction = match choice(p, C, "direction", &DIRECTIONS)?.unwrap_or("vertical") {
        "horizontal" => BridgeDirection::Horizontal,
        "shortest" => BridgeDirection::Shortest,
        _ => BridgeDirection::Vertical,
    };
    let keep = switch_or(p, "keepOriginal", true, C)?;
    let selected = selected_roots(s)?;
    let (bridged, style, roots) = {
        let d = &s.doc()?.doc;
        // The art's outlines, the selected objects they come from, and the topmost object that
        // has some, whose paint the result takes.
        let mut parts: Vec<(PathData, FillRule)> = vec![];
        let mut roots = vec![];
        let mut top = None;
        for id in selected {
            let n = d.node(id).ok_or(EngineError::NoNode(id))?;
            let mut leaves = vec![];
            shown_leaves(d, n, &mut leaves);
            let before = parts.len();
            for leaf in leaves {
                if let Some(part) = node_path(&leaf).filter(|(p, _)| !p.is_empty()) {
                    parts.push(part);
                    top = Some(leaf);
                }
            }
            if parts.len() > before {
                roots.push(id);
            }
        }
        let Some(top) = top else {
            let label = "Stencil Bridges";
            return Err(EngineError::Other(format!("{label}: select paths, compound paths, groups or text")));
        };
        let refs: Vec<(&PathData, FillRule)> = parts.iter().map(|(p, r)| (p, *r)).collect();
        let spec = BridgeSpec { width, count: count as usize, direction };
        let bridged = stencil_bridges(&refs, spec).map_err(|e| match e {
            vectorcraft_pathops::PathOpsError::TooComplex => {
                EngineError::Other("Stencil Bridges: the selected art is too large or too detailed to bridge".into())
            }
            e => EngineError::Other(e.to_string()),
        })?;
        (bridged, style_of(&top), roots)
    };
    let summary = |id: Option<NodeId>, original: Option<NodeId>| {
        json!({
            "id": id.map(|i| i.0), "islands": bridged.islands, "bridges": bridged.bridges, "unbridged": bridged.unbridged,
            "original": original.map(|i| i.0),
        })
    };
    if bridged.islands == 0 {
        let mut r = summary(None, None);
        // The warnings are written as `EngineError::Other` so that the i18n message scan
        // (`untranslated_report`) finds them.
        r["warning"] = json!(EngineError::Other("The selected art has no islands to bridge".into()).to_string());
        return Ok(r);
    }
    if bridged.bridges == 0 {
        let mut r = summary(None, None);
        r["warning"] = json!(EngineError::Other("No bridge of this width fits across the selected art".into()).to_string());
        return Ok(r);
    }
    let path = bridged.path.clone();
    let (id, original) = s.edit("Stencil Bridges", |d, sel| {
        let mut made = shape_node(d, path, None);
        style.apply(&mut made);
        let mut taken = vec![];
        let id = d.wrap(&roots, |nodes| {
            taken = nodes;
            made
        })?;
        let original = if keep { Some(hide_originals(d, taken)?) } else { None };
        sel.set([id]);
        Ok((id, original))
    })?;
    let mut r = summary(Some(id), original);
    if bridged.unbridged > 0 {
        r["warning"] = json!(EngineError::Other("Some islands could not be joined to the area around the art".into()).to_string());
    }
    Ok(r)
}

/// The leaf shapes of `n` as [`pf_leaves`] gives them, leaving out hidden objects and the members
/// of hidden groups.
fn shown_leaves(d: &Document, n: &Node, out: &mut Vec<Node>) {
    if !n.visible {
        return;
    }
    match &n.kind {
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } if !n.is_compound_shape() => {
            for c in children {
                shown_leaves(d, c, out);
            }
        }
        _ => pf_leaves(d, n, out),
    }
}

/// Put `nodes` in a new hidden layer above the others, in their order, and return its id.
fn hide_originals(d: &mut Document, nodes: Vec<Arc<Node>>) -> Result<NodeId> {
    let mut taken = std::collections::HashSet::new();
    d.walk(|n| {
        if n.is_layer() {
            taken.extend(n.name.clone());
        }
    });
    let name = std::iter::once(ORIGINAL_LAYER.to_string())
        .chain((2..).map(|i| format!("{ORIGINAL_LAYER} {i}")))
        .find(|name| !taken.contains(name))
        .unwrap_or_else(|| ORIGINAL_LAYER.to_string());
    let layer = d.add_layer(Some(&name));
    for (i, n) in nodes.into_iter().enumerate() {
        d.insert(Some(layer), i, Arc::unwrap_or_clone(n))?;
    }
    d.node_mut(layer).ok_or(EngineError::NoNode(layer))?.visible = false;
    Ok(layer)
}
