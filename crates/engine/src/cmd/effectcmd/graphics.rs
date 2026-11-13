//! Window › 3D and Materials › Materials › Graphics: art kept as a document symbol (Add as Single
//! Graphic) and mapped onto 3D objects (Extrude & Bevel's caps, a plane, Revolve's side). The
//! graphics an object shows are the `graphics` parameter of its 3D effect, which `effect.setParams`
//! also edits.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Effect, Node};
use vectorcraft_render::effects;

use super::super::appearance::{appearance_targets, index_param, item_target};
use super::super::brushsym::{normalize, selection_art, store_symbol};
use super::super::{CommandSpec, EngineError, NodeId, Result, Session, bad, cmd, has_doc, has_selection, str_param};
use super::{effect_targets, ids_json};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "threeD.addGraphic",
            "Add as Single Graphic",
            [],
            None,
            "{name?: string (default Graphic; made unique), ids?|id?} keep the selection's art as a new document symbol, listed among the 3D and Materials panel's Graphics; the selection stays as it is → {name}",
            has_selection,
            add_graphic
        ),
        cmd!(
            "threeD.mapGraphic",
            "Map Graphic",
            [],
            None,
            "{symbol: name of a document symbol, face?: front|back (front; Plane has only a front; the back reads left to right as seen from behind; on Revolve the front is its swept side, which is the only face), x?: pt, y?: pt (0, 0: the offset of the graphic's center from the center of the object's bounds, in the face's own plane; on Revolve from the middle of the side unrolled flat, x around the axis and y along the profile), scale?: % (1..10000; default 100, or less so the graphic fits inside the object, on Revolve inside the half of the side seen from the front), rotation?: degrees (-360..360, 0; counterclockwise), index?: int (the 3D effect's position; default: each object's first 3D effect, its own before its fills' and strokes'), item?: appearance item index|null (refused without index; as effect.setParams), ids?|id?} add a graphic to the 3D effect of each selected object, drawn on that face turned, foreshortened and shaded with it and clipped to it (on Revolve wrapped around the side, and drawn only where its outside faces the viewer); every object's 3D effect must be Extrude & Bevel (on a solid cap), Rotate (the Plane type) or Revolve; one undo step → {ids, graphic}",
            has_doc,
            map_graphic
        ),
    ]
}

/// `threeD.addGraphic`: stores the selection's art as a symbol without replacing it with an
/// instance.
fn add_graphic(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "threeD.addGraphic";
    if let Some(key) = p.as_object().and_then(|m| m.keys().find(|k| !["name", "ids", "id"].contains(&k.as_str()))) {
        return Err(bad(C, format!("unknown parameter `{key}`")));
    }
    if p.get("name").is_some_and(|v| v.as_str().is_none_or(|n| n.trim().is_empty())) {
        return Err(bad(C, "`name` must be a name that is not empty"));
    }
    let mut art = selection_art(s, p)?.ok_or_else(|| EngineError::Other("nothing selected".into()))?;
    crate::cmd::threads::keep_shown_text(&s.doc()?.doc, &mut art);
    let (art, size, _) = normalize(art)?;
    let name = s.doc()?.doc.unique_symbol_name(str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Graphic"));
    s.edit("Add as Single Graphic", |d, _| {
        store_symbol(d, &art, &name, size);
        Ok(())
    })?;
    Ok(json!({ "name": name }))
}

/// The 3D effect of `n` that a graphic goes on when none is named: its own first, then its fills'
/// and strokes', as the 3D and Materials panel finds it.
fn first_three_d(n: &Node) -> Option<(Option<usize>, usize)> {
    let at = |fx: &[Effect]| fx.iter().position(|e| effects::is_three_d(&e.id));
    at(&n.appearance.effects)
        .map(|i| (None, i))
        .or_else(|| n.appearance.items.iter().enumerate().find_map(|(k, item)| at(item.effects()).map(|i| (Some(k), i))))
}

/// `threeD.mapGraphic`: appends a graphic to the 3D effect of each target, after checking that every
/// target takes it.
fn map_graphic(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "threeD.mapGraphic";
    const KEYS: [&str; 10] = ["symbol", "face", "x", "y", "scale", "rotation", "index", "item", "ids", "id"];
    if let Some(key) = p.as_object().and_then(|m| m.keys().find(|k| !KEYS.contains(&k.as_str()))) {
        return Err(bad(C, format!("unknown parameter `{key}`")));
    }
    // An item names where the effect is in that item's list, so it takes an index too.
    if p.get("item").is_some() && p.get("index").is_none() {
        return Err(bad(C, "`item` needs `index`"));
    }
    // The graphic's own settings, checked as the effect checks them.
    let mut given = Map::new();
    for key in ["symbol", "face", "x", "y", "scale", "rotation"] {
        if let Some(v) = p.get(key) {
            given.insert(key.into(), v.clone());
        }
    }
    let base = effects::MappedGraphic::from_value(&Value::Object(given)).map_err(|e| bad(C, e))?;
    let d = &s.doc()?.doc;
    if !d.symbols.iter().any(|sym| sym.name == base.symbol) {
        return Err(bad(C, format!("no symbol named `{}`", base.symbol)));
    }
    let targets: Vec<(NodeId, Option<usize>, usize)> = if p.get("index").is_some() {
        let index = index_param(p, "index", C)?;
        let item = item_target(s, p, C)?;
        effect_targets(s, p, item, index, C)?
    } else {
        let mut out = vec![];
        for id in appearance_targets(s, p)? {
            let (item, index) = d.node(id).and_then(first_three_d).ok_or_else(|| bad(C, format!("object {} needs a 3D effect", id.0)))?;
            out.push((id, item, index));
        }
        out
    };
    if targets.is_empty() {
        return Err(bad(C, "select an object with a 3D effect"));
    }
    let mut edits = Vec::with_capacity(targets.len());
    for &(id, item, index) in &targets {
        let n = d.node(id).ok_or(EngineError::NoNode(id))?;
        let Some(effect) = n.appearance.effects_at(item).and_then(|fx| fx.get(index)).filter(|e| effects::is_three_d(&e.id)) else {
            return Err(bad(C, format!("object {} needs a 3D effect at index {index}", id.0)));
        };
        if !effects::takes_graphics(&effect.id) {
            return Err(bad(C, format!("object {}: {}", id.0, effects::GRAPHICS_NOT_TAKEN)));
        }
        // A hollow Extrude & Bevel keeps the graphics it has, to show when Cap is turned on again,
        // but has no cap to show a new one on.
        let merged = effects::merged_params(&effect.id, &effect.params);
        let extrude = (effect.id == effects::EXTRUDE).then(|| effects::extrude_options(&merged));
        if extrude.is_some_and(|o| o.depth > 0.0 && !o.cap) {
            return Err(bad(C, format!("object {}: a hollow Extrude & Bevel has no caps: turn on Cap to map a graphic", id.0)));
        }
        // With a depth of 0 the art is a plane, which has only a front.
        if base.face == effects::PlanarFace::Back && extrude.is_some_and(|o| o.depth <= 0.0) {
            return Err(bad(C, format!("object {}: with a depth of 0 the object has one face: map its graphics onto the front", id.0)));
        }
        let mut graphic = base.clone();
        if p.get("scale").is_none()
            && let (Some(frame), Some(art)) = (effects::graphic_frame_at(Some(d), n, item, index), effects::graphic_bounds(d, &graphic.symbol))
        {
            graphic.scale = frame.fitting_scale(art);
        }
        let params = effects::with_graphic(&effects::merged_params(&effect.id, &effect.params), &graphic);
        effects::validate_three_d_at_in(Some(d), n, &effect.id, &params, (item, index)).map_err(|e| bad(C, e))?;
        edits.push((id, item, index, graphic));
    }
    let first = edits.first().map(|(.., g)| g.to_value());
    let ids = s.edit("Map Graphic", |d, _| {
        let mut done = vec![];
        for (id, item, index, graphic) in &edits {
            let Some(e) = d.node_mut(*id).and_then(|n| n.appearance.effects_mut(*item)).and_then(|fx| fx.get_mut(*index)) else { continue };
            let current = effects::upgrade_params(&e.id, &e.params);
            e.params = effects::with_graphic(&current, graphic);
            done.push(*id);
        }
        Ok(done)
    })?;
    Ok(json!({ "ids": ids_json(&ids), "graphic": first }))
}

#[cfg(test)]
#[path = "graphics_tests.rs"]
mod tests;
