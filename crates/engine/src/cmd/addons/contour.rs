//! Contour (`addon.contour.*`), a built-in add-on: the Contour tool's command. A Contour is a copy of a selected shape that
//! carries a live Offset Path effect, so it grows or shrinks the shape without baking anything: its
//! fill, stroke and effect stay editable, and Expand Appearance bakes it into plain paths. The
//! original stays where it is. Running the command on an object that already carries an Offset
//! Path effect (a Contour made earlier) changes that effect instead of nesting another one.

#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_render::effects;

use super::super::edit::{copy_above, selected_roots};
use super::super::pathops::{len_param, num_param};
use super::super::{CommandSpec, EngineError, NodeId, Result, Session, bad, cmd, has_selection, str_param};

const C: &str = "addon.contour.set";
/// The live effect that does the offsetting (the one Effect › Path › Offset Path applies).
const EFFECT: &str = "path.offsetPath";
/// The largest offset, in points; Offset Path takes the same.
const MAX_OFFSET: f64 = 1e5;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.contour.set",
        "Contour",
        [],
        None,
        "{offset: pt (negative insets), corners?: \"sharp\"|\"round\"|\"square\" (default sharp; square cuts each corner off flat), miterLimit?: 1..500 (4; sharp corners only)} make a copy of each selected path or shape directly above it with a live Offset Path effect, or, for a selected object that already has one (a Contour), change that effect: nothing is baked (Expand Appearance bakes it) and the original is kept → {ids, created, updated}",
        has_selection,
        contour
    )]
}

/// The Offset Path join a Contour corner style stands for.
fn joins_for(corners: Option<&str>) -> Result<&'static str> {
    match corners.map(str::to_ascii_lowercase).as_deref() {
        None | Some("sharp") => Ok("miter"),
        Some("round") => Ok("round"),
        Some("square") => Ok("bevel"),
        Some(other) => Err(bad(C, format!("`corners` must be sharp, round or square, not `{other}`"))),
    }
}

fn contour(s: &mut Session, p: &Value) -> Result<Value> {
    let offset = len_param(p, "offset").ok_or_else(|| bad(C, "missing `offset` (a distance in points)"))?;
    if offset.abs() > MAX_OFFSET {
        return Err(bad(C, "offset out of range"));
    }
    let joins = joins_for(str_param(p, "corners"))?;
    let miter = num_param(p, "miterLimit").unwrap_or(4.0).clamp(1.0, 500.0);
    let effect = effects::new_effect(EFFECT, &json!({"offset": offset, "joins": joins, "miterLimit": miter}))
        .ok_or_else(|| EngineError::Other("Contour: the Offset Path effect is not available".into()))?;
    let roots = selected_roots(s)?;
    let (updated, created) = s.edit("Contour", |d, sel| {
        // A shape that already carries Offset Path is a Contour: change it. Any other path or
        // shape gets a Contour copy above it. Type, images and the rest are left alone.
        let (mut again, mut fresh) = (vec![], vec![]);
        for id in roots {
            let Some(n) = d.node(id) else { continue };
            if !matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) {
                continue;
            }
            if n.appearance.effects.iter().any(|e| e.id == EFFECT) {
                again.push(id);
            } else {
                fresh.push(id);
            }
        }
        if again.is_empty() && fresh.is_empty() {
            return Err(EngineError::Other("Contour: select paths or shapes".into()));
        }
        for id in &again {
            if let Some(e) = d.node_mut(*id).and_then(|n| n.appearance.effects.iter_mut().rev().find(|e| e.id == EFFECT)) {
                e.params = effect.params.clone();
            }
        }
        let made = copy_above(d, &fresh, |_, _, copy| copy.appearance.effects.push(effect.clone()))?;
        sel.set(again.iter().chain(made.iter()).copied());
        Ok((again, made))
    })?;
    let list = |ids: &[NodeId]| ids.iter().map(|i| i.0).collect::<Vec<_>>();
    Ok(json!({ "ids": list(&[updated.as_slice(), created.as_slice()].concat()), "created": list(&created), "updated": list(&updated) }))
}
