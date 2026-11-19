//! Window › 3D and Materials › Materials › Graphics: art kept as a document symbol (Add as Single
//! Graphic) and mapped onto 3D objects (Extrude & Bevel's caps, a plane, Revolve's side). The
//! graphics an object shows are the `graphics` parameter of its 3D effect, which `effect.setParams`
//! also edits.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Effect, Node};
use vectorcraft_render::effects;

use super::super::appearance::{edit_targets, index_param, item_target};
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
            "{symbol: name of a document symbol, face?: front|back|side (front; Plane has only a front; the back reads left to right as seen from behind; side is Extrude & Bevel's walls and bevels; on Revolve the front is its swept side, which is the only face), x?: pt, y?: pt (0, 0: the offset of the graphic's center from the center of the object's bounds, in the face's own plane; on a side from the middle of the side unrolled flat: on Extrude & Bevel x along the outline, clockwise from its top, and y from the front toward the back; on Revolve x around the axis and y along the profile), scale?: % (1..10000; default 100, or less so the graphic fits inside the object, on Extrude & Bevel's side inside the side unrolled about x = 0, short of its cut, on Revolve inside the half of the side seen from the front), rotation?: degrees (-360..360, 0; counterclockwise), index?: int (the 3D effect's position; default: each object's first 3D effect, its own before its fills' and strokes'), item?: appearance item index|null (refused without index; as effect.setParams), ids?|id?} add a graphic to the 3D effect of each selected object, drawn on that face turned, foreshortened and shaded with it and clipped to it (on a side wrapped around it, and drawn only where its outside faces the viewer); every object's 3D effect must be Extrude & Bevel (on a solid cap, or on the side, hollow or not), Rotate (the Plane type) or Revolve; one undo step → {ids, graphic}",
            has_doc,
            map_graphic
        ),
        cmd!(
            "threeD.mapArt",
            "Map Art",
            [],
            None,
            "{surface?: integer (1..the object's surface count: Extrude & Bevel (Classic) numbers each solid's front cap and back cap (none on a hollow solid, cap false), then each side wall from corner to corner of the outline, then the bevel above each wall; Revolve (Classic)'s swept side is surface 1, in a group the side of the member with the longest profile, and the other members take no map), symbol?: name of a document symbol|null (with surface: the symbol mapped onto it; null removes the surface's map, also a map kept past the surface count, and is refused with x, y, scaleX, scaleY, rotation or fit true; omitted: the surface's map keeps its symbol, and a surface with no map is refused), x?: pt, y?: pt (-1000000..1000000; the offset of the art's center from the center of the surface laid out flat; a new map starts at 0, 0), scaleX?: %, scaleY?: % (1..10000; of the symbol's own width and height; a new map starts at 100), rotation?: degrees (-360..360; counterclockwise; a new map starts at 0), fit?: bool (false; true is Scale to Fit: the art is centered and scaled to fill the surface, each side on its own when the art is square to the surface, else in proportion; art with no area keeps its scales), clearAll?: bool (false; true removes every map first, so with surface only that surface's map is left), shade?: bool (Shade Artwork: the mapped art takes the tone and highlight of its face), invisible?: bool (Invisible Geometry: with at least one map, only the mapped art is drawn), index?: int (the 3D effect's position; default: each object's first Extrude & Bevel (Classic) or Revolve (Classic), its own before its fills' and strokes'), item?: appearance item index|null (refused without index; as effect.setParams), ids?|id?} set, edit or remove the art mapped onto the numbered surfaces of each selected object's 3D effect, which must be Extrude & Bevel (Classic) or Revolve (Classic) (Rotate (Classic) takes no map); one of surface, clearAll, shade or invisible is required; the maps are the effect's mapArt, mapShade and mapInvisible parameters; one undo step, and none when nothing changes → {ids: [the objects changed], maps: [the first object's maps]}",
            has_doc,
            map_art
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
        for id in edit_targets(s, p, C)? {
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
        // A hollow Extrude & Bevel keeps the graphics it has on its caps, to show when Cap is turned
        // on again, but has no cap to show a new one on. Its side takes graphics.
        let merged = effects::merged_params(&effect.id, &effect.params);
        let extrude = (effect.id == effects::EXTRUDE).then(|| effects::extrude_options(&merged));
        let side = base.face == effects::PlanarFace::Side;
        if !side && extrude.is_some_and(|o| o.depth > 0.0 && !o.cap) {
            return Err(bad(C, format!("object {}: a hollow Extrude & Bevel has no caps: turn on Cap to map a graphic", id.0)));
        }
        // With a depth of 0 the art is a plane, which has only a front.
        if base.face != effects::PlanarFace::Front && extrude.is_some_and(|o| o.depth <= 0.0) {
            return Err(bad(C, format!("object {}: with a depth of 0 the object has one face: map its graphics onto the front", id.0)));
        }
        let mut graphic = base.clone();
        if p.get("scale").is_none()
            && let Some(art) = effects::graphic_bounds(d, &graphic.symbol)
        {
            // A graphic on the side starts fitting inside the side unrolled about x = 0, short of
            // its cut; one on a cap inside the object's bounds.
            let fit = if side {
                effects::graphic_side_size(Some(d), n, item, index).map(|(w, h)| effects::graphic_fitting_scale(w, h, art))
            } else {
                effects::graphic_frame_at(Some(d), n, item, index).map(|frame| frame.fitting_scale(art))
            };
            if let Some(fit) = fit {
                graphic.scale = fit;
            }
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

/// The effect of `n` that Map Art edits when none is named: its first Extrude & Bevel (Classic) or
/// Revolve (Classic), its own before its fills' and strokes'.
fn first_mapped(n: &Node) -> Option<(Option<usize>, usize)> {
    let at = |fx: &[Effect]| fx.iter().position(|e| effects::takes_map_art(&e.id));
    at(&n.appearance.effects)
        .map(|i| (None, i))
        .or_else(|| n.appearance.items.iter().enumerate().find_map(|(k, item)| at(item.effects()).map(|i| (Some(k), i))))
}

/// `threeD.mapArt`: sets, edits or removes the maps of each target's classic 3D effect, after
/// checking that every target takes the change. Nothing is recorded when no target changes.
fn map_art(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "threeD.mapArt";
    const PLACING: [&str; 6] = ["symbol", "x", "y", "scaleX", "scaleY", "rotation"];
    const KEYS: [&str; 15] =
        ["surface", "symbol", "x", "y", "scaleX", "scaleY", "rotation", "fit", "clearAll", "shade", "invisible", "index", "item", "ids", "id"];
    if let Some(key) = p.as_object().and_then(|m| m.keys().find(|k| !KEYS.contains(&k.as_str()))) {
        return Err(bad(C, format!("unknown parameter `{key}`")));
    }
    if p.get("item").is_some() && p.get("index").is_none() {
        return Err(bad(C, "`item` needs `index`"));
    }
    let surface = match p.get("surface") {
        None => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| (1..=effects::MAX_MAP_SURFACE).contains(n))
                .ok_or_else(|| bad(C, "`surface` must be a whole number from 1 to the object's surface count"))?,
        ),
    };
    let fit = super::super::switch_or(p, "fit", false, C)?;
    let clear_all = super::super::switch_or(p, "clearAll", false, C)?;
    let flag = |key: &str| -> Result<Option<bool>> {
        match p.get(key) {
            None => Ok(None),
            Some(_) => super::super::switch_or(p, key, false, C).map(Some),
        }
    };
    let (shade, invisible) = (flag("shade")?, flag("invisible")?);
    match surface {
        None if fit || PLACING.iter().any(|k| p.get(*k).is_some()) => {
            return Err(bad(C, "`symbol`, `x`, `y`, `scaleX`, `scaleY`, `rotation` and `fit` need `surface`"));
        }
        None if !clear_all && shade.is_none() && invisible.is_none() => {
            return Err(bad(C, "give `surface`, `clearAll`, `shade` or `invisible`"));
        }
        _ => {}
    }
    let removes = p.get("symbol").is_some_and(Value::is_null);
    if removes && (fit || PLACING[1..].iter().any(|k| p.get(*k).is_some())) {
        return Err(bad(C, "`symbol: null` removes the map: leave out `x`, `y`, `scaleX`, `scaleY`, `rotation` and `fit`"));
    }
    let d = &s.doc()?.doc;
    let targets: Vec<(NodeId, Option<usize>, usize)> = if p.get("index").is_some() {
        let index = index_param(p, "index", C)?;
        let item = item_target(s, p, C)?;
        effect_targets(s, p, item, index, C)?
    } else {
        let mut out = vec![];
        for id in edit_targets(s, p, C)? {
            let found = d.node(id).and_then(first_mapped);
            let (item, index) = found.ok_or_else(|| bad(C, format!("object {}: {}", id.0, effects::MAP_ART_NOT_TAKEN)))?;
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
        if !effects::takes_map_art(&effect.id) {
            return Err(bad(C, format!("object {}: {}", id.0, effects::MAP_ART_NOT_TAKEN)));
        }
        let merged = effects::merged_params(&effect.id, &effect.params);
        let mut params = merged.clone();
        if clear_all {
            params[effects::MAP_ART] = json!([]);
        }
        if let Some(surface) = surface {
            // Art that can't be built has no surfaces: the effect's own check says why.
            let surfaces = match effects::map_surfaces_at(Some(d), n, item, index) {
                Some(surfaces) => surfaces,
                None => {
                    effects::validate_three_d_at_in(Some(d), n, &effect.id, &merged, (item, index)).map_err(|e| bad(C, e))?;
                    vec![]
                }
            };
            // Read before clearAll empties the list, so with clearAll the surface keeps its map.
            let current = effects::surface_maps(&merged).into_iter().find(|m| m.surface == surface);
            // A map kept past the surface count (the solid lost surfaces) can still be removed.
            let flat = surfaces.get(surface - 1);
            if flat.is_none() && !(removes && current.is_some()) {
                return Err(bad(C, format!("object {}: `surface` must be from 1 to {}, the object's surface count", id.0, surfaces.len())));
            }
            if removes {
                params = effects::with_surface_map(&params, surface, None);
            } else if let Some(flat) = flat {
                // The values given go over the surface's map, or over a new map's defaults.
                let mut given = match (&current, p.get("symbol")) {
                    (Some(m), _) => m.to_value(),
                    (None, Some(_)) => json!({ "surface": surface }),
                    (None, None) => return Err(bad(C, format!("object {}: surface {surface} has no map: give `symbol`", id.0))),
                };
                for key in PLACING {
                    if let Some(v) = p.get(key) {
                        given[key] = v.clone();
                    }
                }
                let mut map = effects::SurfaceMap::from_value(&given).map_err(|e| bad(C, e))?;
                // A map whose symbol was deleted keeps it while the request leaves `symbol` out; a
                // symbol the request names must exist.
                if !d.symbols.iter().any(|sym| sym.name == map.symbol) && (p.get("symbol").is_some() || current.is_none()) {
                    return Err(bad(C, format!("no symbol named `{}`", map.symbol)));
                }
                if fit {
                    // Art or a surface with no area has no size to fit: the map is centered and
                    // keeps its scales.
                    let scales = effects::graphic_bounds(d, &map.symbol).and_then(|art| map.fitting_scales(art, (flat.width, flat.height)));
                    let (scale_x, scale_y) = scales.unwrap_or((map.scale_x, map.scale_y));
                    map = effects::SurfaceMap { x: 0.0, y: 0.0, scale_x, scale_y, ..map };
                }
                params = effects::with_surface_map(&params, surface, Some(&map));
            }
        }
        for (key, value) in [(effects::MAP_SHADE, shade), (effects::MAP_INVISIBLE, invisible)] {
            if let Some(on) = value {
                params[key] = json!(on);
            }
        }
        effects::validate_three_d_at_in(Some(d), n, &effect.id, &params, (item, index)).map_err(|e| bad(C, e))?;
        let state = |p: &Value| {
            let on = |key: &str| p.get(key).and_then(Value::as_bool).unwrap_or(false);
            (effects::surface_maps(p), on(effects::MAP_SHADE), on(effects::MAP_INVISIBLE))
        };
        let maps: Vec<Value> = effects::surface_maps(&params).iter().map(effects::SurfaceMap::to_value).collect();
        let changed = state(&params) != state(&merged);
        edits.push((id, item, index, params, changed, maps));
    }
    let first = edits.first().map(|(.., maps)| maps.clone()).unwrap_or_default();
    if !edits.iter().any(|(.., changed, _)| *changed) {
        return Ok(json!({ "ids": [], "maps": first }));
    }
    let ids = s.edit("Map Art", |d, _| {
        let mut done = vec![];
        for (id, item, index, params, changed, _) in &edits {
            let Some(e) = d.node_mut(*id).and_then(|n| n.appearance.effects_mut(*item)).and_then(|fx| fx.get_mut(*index)).filter(|_| *changed) else {
                continue;
            };
            let mut current = effects::upgrade_params(&e.id, &e.params);
            for key in [effects::MAP_ART, effects::MAP_SHADE, effects::MAP_INVISIBLE] {
                if let (Some(v), Some(m)) = (params.get(key), current.as_object_mut()) {
                    m.insert(key.into(), v.clone());
                }
            }
            e.params = current;
            done.push(*id);
        }
        Ok(done)
    })?;
    Ok(json!({ "ids": ids_json(&ids), "maps": first }))
}

#[cfg(test)]
#[path = "graphics_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "map_art_tests.rs"]
mod map_art_tests;
