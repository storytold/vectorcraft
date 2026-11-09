//! Dynamic symbols (Symbol Options › Symbol Type): an instance of a dynamic symbol changes the
//! appearance of objects in the symbol's art and stays linked to the symbol
//! (`symbol.instance.setAppearance`, `symbol.instance.reset`). The changes live on the instance
//! ([`vectorcraft_doc::symbol_overrides`]); editing the symbol keeps those whose object is still
//! there ([`fit_overrides`]).

use std::sync::Arc;

use vectorcraft_color::BlendMode;
use vectorcraft_doc::symbol_overrides::{self, SymbolOverride};

use super::{Document, EngineError, Node, NodeId, NodeKind, Result, Session, Value, bad, check_numbers, ids_json, json, roots, selected_instances};

/// The `type` param, Symbol Options › Symbol Type: `Some(true)` for dynamic.
pub(super) fn type_param(p: &Value, cmd: &str) -> Result<Option<bool>> {
    let Some(v) = p.get("type").filter(|v| !v.is_null()) else { return Ok(None) };
    match v.as_str() {
        Some("static") => Ok(Some(false)),
        Some("dynamic") => Ok(Some(true)),
        _ => Err(bad(cmd, format!("type must be static or dynamic, not {v}"))),
    }
}

/// The `type` a symbol reports.
pub(super) fn type_id(dynamic: bool) -> &'static str {
    if dynamic { "dynamic" } else { "static" }
}

/// Fit the appearance changes of instances `ids` to their symbols: an instance of a static (or
/// missing) symbol keeps none, an instance of a dynamic one those naming an object of its art.
pub(super) fn fit_overrides(d: &mut Document, ids: &[NodeId]) {
    for id in ids {
        let art = match d.node(*id) {
            Some(n) if !n.extras().symbol_overrides.is_empty() => match &n.kind {
                NodeKind::SymbolInstance { symbol, .. } => d.symbols.iter().find(|s| s.name == *symbol).filter(|s| s.dynamic).map(|s| s.art.clone()),
                _ => continue,
            },
            _ => continue,
        };
        let Some(n) = d.node_mut(*id) else { continue };
        let mut e = n.extras_mut();
        match art {
            Some(art) => symbol_overrides::retain_parts(&mut e.symbol_overrides, &art),
            None => e.symbol_overrides.clear(),
        }
    }
}

/// The instance `id` names (else the one selected object) and its symbol's art; the symbol must
/// be dynamic.
fn dynamic_instance(s: &Session, p: &Value, cmd: &str) -> Result<(NodeId, Arc<Node>)> {
    let d = &s.doc()?.doc;
    let not_one = || EngineError::Other("select a symbol instance".into());
    let [id] = roots(s, p)?[..] else { return Err(not_one()) };
    let Some(NodeKind::SymbolInstance { symbol, .. }) = d.node(id).map(|n| &n.kind) else { return Err(not_one()) };
    let sym = d.symbols.iter().find(|x| x.name == *symbol).ok_or_else(|| bad(cmd, format!("no symbol named `{n}`", n = symbol)))?;
    if !sym.dynamic {
        let name = &sym.name;
        return Err(bad(cmd, format!("`{name}` is a static symbol: set its Symbol Type to Dynamic first")));
    }
    Ok((id, sym.art.clone()))
}

/// The selected object of a dynamic symbol instance's art (Direct Selection): the instance and the
/// object's path in the symbol's art.
pub(crate) fn selected_part(s: &Session) -> Result<Option<(NodeId, Vec<u32>)>> {
    Ok(s.doc()?.selection.symbol_part.as_ref().map(|sp| (sp.instance, sp.path.clone())))
}

/// Change instance `id`'s own appearance for the object at `path` of its art by `f`, as one undo
/// step named `label`; nothing is recorded when the change leaves it as it was. Replies how many
/// objects the instance changes.
pub(crate) fn edit_part(s: &mut Session, id: NodeId, path: &[u32], label: &str, f: impl FnOnce(&mut SymbolOverride)) -> Result<usize> {
    let list = s.doc()?.doc.node(id).ok_or(EngineError::NoNode(id))?.extras().symbol_overrides.clone();
    let old = list.iter().find(|o| o.path == path).cloned().unwrap_or_else(|| SymbolOverride { path: path.to_vec(), ..Default::default() });
    let mut new = old.clone();
    f(&mut new);
    if new == old {
        return Ok(list.len());
    }
    s.edit(label, |d, _| {
        let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
        let mut e = n.extras_mut();
        let list = &mut e.symbol_overrides;
        list.retain(|o| o.path != path);
        if !new.is_empty() {
            list.push(new);
        }
        Ok(list.len())
    })
}

/// The refusal of anything but a solid color fill or stroke change while an object of a symbol
/// instance's art is selected ([`selected_part`]), and of selecting any other object of the art.
pub(crate) fn part_only() -> EngineError {
    EngineError::Other("Only the solid color fill and stroke of a path, compound path or type selected in a symbol instance can change".into())
}

pub(super) fn symbol_instance_select_part(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.instance.selectPart";
    if p.get("id").is_none() {
        return Err(bad(C, "missing id"));
    }
    let (id, art) = dynamic_instance(s, p, C)?;
    let path = path_param(p, &art, C)?;
    // Direct Selection passes over hidden, locked and template objects, and over everything in a
    // group or layer that is: the instance in the document, and the object in the symbol's art.
    let editable = |n: &Node| n.visible && !n.locked && !n.is_template();
    if !s.doc()?.doc.is_editable(id) || !(0..=path.len()).all(|k| symbol_overrides::part(&art, &path[..k]).is_some_and(editable)) {
        return Err(bad(C, "the object is hidden or locked"));
    }
    if !symbol_overrides::part(&art, &path).is_some_and(symbol_overrides::takes_part_paint) {
        return Err(part_only());
    }
    let shown = json!(path);
    s.select(|_, sel| sel.set_symbol_part(id, path))?;
    Ok(json!({ "id": id.0, "path": shown }))
}

/// The `path` param: child indices from the art's root, naming an object of `art`.
fn path_param(p: &Value, art: &Node, cmd: &str) -> Result<Vec<u32>> {
    let path: Option<Vec<u32>> =
        p.get("path").and_then(Value::as_array).and_then(|a| a.iter().map(|i| i.as_u64().and_then(|i| u32::try_from(i).ok())).collect());
    let path = path.ok_or_else(|| bad(cmd, "path must be a list of child indices"))?;
    if symbol_overrides::part(art, &path).is_none() {
        let shown = json!(path);
        return Err(bad(cmd, format!("no object at path {shown} in the symbol's art")));
    }
    Ok(path)
}

/// The refusal of `key` (`fill` or `stroke`) for `n`, an object of a dynamic symbol's art, when `n`
/// does not draw that paint of its own; `None` when it does. Paths (not guides),
/// compound paths, type, groups and layers draw them; a symbol instance draws its fill as a stain
/// on its art ([`solid_fill_check`]) and no stroke; an image draws one only when that fill or
/// stroke has a Convert to Shape or Outline Object effect.
fn paint_refusal(n: &Node, key: &str, cmd: &str) -> Option<EngineError> {
    Some(match &n.kind {
        NodeKind::Path { guide: true, .. } => bad(cmd, format!("{key} cannot be set on a guide")),
        NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Text(_) | NodeKind::Group { .. } | NodeKind::Layer { .. } => return None,
        NodeKind::Image(_) => {
            let effects = if key == "fill" { n.appearance.fill().map(|f| &f.effects) } else { n.appearance.stroke().map(|s| &s.effects) };
            if effects.is_some_and(|e| vectorcraft_render::effects::gives_image_shape(e)) {
                return None;
            }
            bad(cmd, format!("{key} cannot be set on an image without a Convert to Shape or Outline Object effect on it"))
        }
        NodeKind::SymbolInstance { .. } if key == "fill" => return None,
        NodeKind::SymbolInstance { .. } => bad(cmd, "stroke cannot be set on a symbol instance"),
        NodeKind::Blend { .. } => bad(cmd, format!("{key} cannot be set on a blend")),
        NodeKind::Envelope { .. } => bad(cmd, format!("{key} cannot be set on an envelope")),
        NodeKind::Mesh(_) => bad(cmd, format!("{key} cannot be set on a gradient mesh")),
        NodeKind::Repeat(_) => bad(cmd, format!("{key} cannot be set on a Repeat")),
        NodeKind::PlacedDocument(_) => bad(cmd, format!("{key} cannot be set on a placed document")),
        NodeKind::Intertwine { .. } => bad(cmd, format!("{key} cannot be set on an Intertwine group")),
    })
}

/// Refuse a `fill` or `stroke` other than null that the object at `path` of `art` does not draw
/// ([`paint_refusal`]); null, which takes a change back, is taken for every object.
fn check_paint_kinds(p: &Value, art: &Node, path: &[u32], cmd: &str) -> Result<()> {
    let Some(n) = symbol_overrides::part(art, path) else { return Ok(()) };
    for key in ["fill", "stroke"] {
        if p.get(key).is_some_and(|v| !v.is_null())
            && let Some(e) = paint_refusal(n, key, cmd)
        {
            return Err(e);
        }
    }
    Ok(())
}

/// Refuse `fill`, the paint `p` asks for the object at `path` of `art`, when that object is a
/// symbol instance and `fill` is a gradient or a pattern: an instance stains its art with a solid
/// fill only, so such a fill draws nothing.
fn solid_fill_check(p: &Value, art: &Node, path: &[u32], fill: Option<&vectorcraft_color::Paint>, cmd: &str) -> Result<()> {
    let asked = p.get("fill").is_some_and(|v| !v.is_null());
    let is_instance = symbol_overrides::part(art, path).is_some_and(|n| matches!(n.kind, NodeKind::SymbolInstance { .. }));
    let draws_nothing = |f: &vectorcraft_color::Paint| match f {
        vectorcraft_color::Paint::Gradient(_) => true,
        vectorcraft_color::Paint::Pattern { .. } => true,
        vectorcraft_color::Paint::None | vectorcraft_color::Paint::Solid { .. } => false,
    };
    if asked && is_instance && fill.is_some_and(draws_nothing) {
        return Err(bad(cmd, "fill on a symbol instance must be a solid color or none"));
    }
    Ok(())
}

/// `fill` or `stroke` of `p` for override `o`: absent leaves it, null clears it, a colour string or
/// an object of `paint.setFill`'s paint params sets it.
fn paint_change(s: &Session, p: &Value, key: &str, cmd: &str) -> Result<Option<Option<vectorcraft_color::Paint>>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    if v.is_null() {
        return Ok(Some(None));
    }
    let v = if v.is_string() { json!({ "color": v }) } else { v.clone() };
    let paint = crate::cmd::paint::paint_from(s, &v)?.ok_or_else(|| bad(cmd, "give color, none, swatch or gradient"))?;
    Ok(Some(Some(paint)))
}

/// The change `p` asks of the override `old` (for the object at `path`).
fn changed(s: &Session, p: &Value, old: SymbolOverride, cmd: &str) -> Result<SymbolOverride> {
    let mut o = old;
    if let Some(fill) = paint_change(s, p, "fill", cmd)? {
        o.fill = fill;
    }
    if let Some(stroke) = paint_change(s, p, "stroke", cmd)? {
        o.stroke = stroke;
    }
    check_numbers(p, &["opacity"], cmd)?;
    if let Some(v) = p.get("opacity") {
        o.opacity = v.as_f64().map(crate::cmd::opacitymask::percent);
    }
    if let Some(v) = p.get("blend") {
        o.blend = match v {
            Value::Null => None,
            b => Some(b.as_str().and_then(BlendMode::parse).ok_or_else(|| bad(cmd, format!("unknown blend mode {b}")))?),
        };
    }
    Ok(o)
}

pub(super) fn symbol_instance_set_appearance(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.instance.setAppearance";
    if !["fill", "stroke", "opacity", "blend"].iter().any(|k| p.get(k).is_some()) {
        return Err(bad(C, "give fill, stroke, opacity and/or blend"));
    }
    let (id, art) = dynamic_instance(s, p, C)?;
    let path = path_param(p, &art, C)?;
    check_paint_kinds(p, &art, &path, C)?;
    let d = &s.doc()?.doc;
    let old = d.node(id).and_then(|n| n.extras().symbol_overrides.iter().find(|o| o.path == path).cloned());
    let new = changed(s, p, old.unwrap_or_else(|| SymbolOverride { path: path.clone(), ..Default::default() }), C)?;
    solid_fill_check(p, &art, &path, new.fill.as_ref(), C)?;
    let count = edit_part(s, id, &path, "Symbol Instance Appearance", |o| *o = new)?;
    Ok(json!({ "id": id.0, "overrides": count }))
}

pub(super) fn symbol_instance_reset(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_instances(s, p)?;
    if ids.is_empty() {
        return Err(EngineError::Other("no symbol instances selected".into()));
    }
    let d = &s.doc()?.doc;
    let changed: Vec<NodeId> = ids.into_iter().filter(|id| d.node(*id).is_some_and(|n| !n.extras().symbol_overrides.is_empty())).collect();
    if !changed.is_empty() {
        s.edit("Reset Symbol Instance", |d, _| {
            for id in &changed {
                if let Some(n) = d.node_mut(*id) {
                    n.extras_mut().symbol_overrides.clear();
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({ "ids": ids_json(&changed) }))
}
