//! Brushes (Brushes panel, Paintbrush, Expand) and Symbols (Symbols panel, symbolism tools).
//!
//! Brush definitions live in `vectorcraft-brush` and are stored in `Document::unknown["brushes"]`.
//! Symbol definitions are `Document::symbols`; symbols made here are *normalised*: the art is
//! centred on the origin and scaled (non-uniformly, strokes untouched) into the ±10 pt box that
//! `Node::geometric_bounds` assumes for instances, and each instance's `xf` scales it back. The
//! natural size is kept in `Document::unknown["symbolSizes"]` so new instances come out right.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_brush::{self as brush, Brush};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, Document, FillLayer, Node, NodeId, NodeKind, Symbol};
use vectorcraft_geom::{Affine, Point, Vec2};

use super::edit::selected_roots;
use super::*;
use crate::EngineError;

const SIZES_KEY: &str = vectorcraft_doc::SYMBOL_SIZES;
const CURRENT_SYMBOL: &str = "currentSymbol";
/// Half the instance box `Node::geometric_bounds` uses for symbol instances.
const HALF: f64 = vectorcraft_doc::SYMBOL_HALF;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        // ----- brushes -----
        cmd!(query "brush.list", "Brushes", [], None, "{} → {brushes: [{name, type}], current}", has_doc, brush_list),
        cmd!(query "brush.get", "Brush Definition", [], None, "{name} → the brush definition (JSON)", has_doc, brush_get),
        cmd!(
            "brush.apply",
            "Apply Brush",
            [],
            None,
            "{name, ids?} apply a brush to the strokes of the selected paths (adds a stroke if missing) and make it current → {ids}",
            has_doc,
            brush_apply
        ),
        cmd!("brush.remove", "Remove Brush Stroke", [], None, "{ids?} remove brushes from the selected strokes → {ids}", has_selection, brush_remove),
        cmd!(
            "brush.setCurrent",
            "Current Brush",
            [],
            None,
            "{name: string|null} the brush the Paintbrush tool paints with (not an undo step)",
            has_doc,
            brush_set_current
        ),
        cmd!(
            "brush.new",
            "New Brush…",
            [],
            None,
            "{type: calligraphic|scatter|art|pattern|bristle, name?, params?: {…definition fields}, ids?} new brush; art/scatter/pattern brushes take the selected art (pattern: side tile) unless params give it → {name}",
            has_doc,
            brush_new
        ),
        cmd!("brush.delete", "Delete Brush", [], None, "{name} delete a brush; strokes using it become plain strokes", has_doc, brush_delete),
        cmd!("brush.duplicate", "Duplicate Brush", [], None, "{name, newName?} → {name}", has_doc, brush_duplicate),
        cmd!(
            "brush.options",
            "Brush Options…",
            [],
            None,
            "{name, params?: {…fields to change (brush.get shows them; values outside their ranges are clamped). calligraphic: angle −180..180°, roundness 0..100%, size 0..1296 pt, variation: [angle°, roundness%, size pt], modes: [fixed|random|pressure for angle, roundness, size] (pressure goes from value − variation at the lightest pen pressure to value + variation at the heaviest). scatter: size [a, b] 1..10000%, spacing [a, b] 1..10000%, scatter [a, b] −1000..1000%, rotation [a, b] −180..180°, modes: [fixed|random|pressure for size, spacing, scatter, rotation] (fixed: a; random: between a and b; pressure: a at the lightest pen pressure to b at the heaviest), rotation_relative_to_path. art: width 1..1000%, scale: {mode: proportional|stretch|betweenGuides, start, end: 0..1 of the art length}, direction: leftToRight|rightToLeft|topToBottom|bottomToTop, flip_along, flip_across. pattern: scale 1..10000%, spacing 0..10000%, fit: stretch|addSpace|approximate, flip_along, flip_across. scatter, art and pattern: colorization: {method: none|tints|tintsAndShades|hueShift, key: colour (hueShift: the art colour that becomes the stroke colour)}}, newName?} edit a brush definition (strokes using it update) → {name}",
            has_doc,
            brush_options
        ),
        cmd!(
            "object.expandBrush",
            "Expand Brush Strokes",
            [],
            None,
            "{ids?} replace brushed paths (in the selection) with groups of the brush art → {ids}",
            has_selection,
            expand_brush
        ),
        cmd!(
            "brush.freehand",
            "Paintbrush",
            [],
            None,
            "{…path.freehand params, brush} paint a freehand stroke with a brush → {id}",
            has_doc,
            brush_freehand
        ),
        // ----- symbols -----
        cmd!(query "symbol.list", "Symbols", [], None, "{} → {symbols: [{name, instances, size: [w,h]}], current}", has_doc, symbol_list),
        cmd!(
            "symbol.new",
            "New Symbol…",
            [],
            None,
            "{name?, ids?} make a symbol from the selection and replace it with an instance → {name, id}",
            has_selection,
            symbol_new
        ),
        cmd!(
            "symbol.place",
            "Place Symbol Instance",
            [],
            None,
            "{name?, x?, y?} place an instance centred at (x, y) (default: artboard centre; name default: current) → {id}",
            has_doc,
            symbol_place
        ),
        cmd!(
            "symbol.breakLink",
            "Break Link to Symbol",
            [],
            None,
            "{ids?} expand selected instances into plain art → {ids}",
            has_selection,
            symbol_break_link
        ),
        cmd!(
            "symbol.edit",
            "Edit Symbol",
            [],
            None,
            "{id?} break the link of the selected instance so its art can be edited; finish with symbol.update → {name, ids}",
            has_selection,
            symbol_edit
        ),
        cmd!(
            "symbol.update",
            "Redefine Symbol",
            [],
            None,
            "{name, ids?} replace the symbol's art with the selection (instances update; the selection becomes an instance) → {name, id}",
            has_selection,
            symbol_update
        ),
        cmd!(
            "symbol.delete",
            "Delete Symbol",
            [],
            None,
            "{name, expandInstances?: true} delete a symbol; its instances are expanded (or deleted with false)",
            has_doc,
            symbol_delete
        ),
        cmd!("symbol.duplicate", "Duplicate Symbol", [], None, "{name, newName?} → {name}", has_doc, symbol_duplicate),
        cmd!(
            "symbol.replace",
            "Replace Symbol",
            [],
            None,
            "{name, ids?} swap the symbol of the selected instances → {ids}",
            has_selection,
            symbol_replace
        ),
        cmd!(
            "symbol.setCurrent",
            "Current Symbol",
            [],
            None,
            "{name} the symbol the symbolism tools spray (not an undo step)",
            has_doc,
            symbol_set_current
        ),
        cmd!(query "symbol.selectInstances", "Select All Instances", [], None, "{name} select every instance of a symbol → {count}", has_doc, symbol_select_instances),
        cmd!(
            "symbol.spray",
            "Symbol Sprayer",
            [],
            None,
            "{points: [[x,y]…], name?, radius?: 40, density?: 1..10 (5), scale?: 1, alt?: bool (remove instances)} spray instances into a symbol set → {id, count}",
            has_doc,
            symbol_spray
        ),
        cmd!(
            "symbol.adjust",
            "Symbolism Tool",
            [],
            None,
            "{tool: shift|scrunch|size|spin|stain|screen|style, points: [[x,y]…], radius?: 40, intensity?: 1..10 (5), alt?: bool, color?, style?: name} adjust instances near the points (selection, else all) → {count}",
            has_doc,
            symbol_adjust
        ),
    ]
}

fn ids_json(ids: &[NodeId]) -> Value {
    Value::Array(ids.iter().map(|i| Value::from(i.0)).collect())
}

fn roots(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    match ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])) {
        Some(mut v) => {
            // Explicit ids: only existing, non-layer objects.
            let d = &s.doc()?.doc;
            v.retain(|id| d.node(*id).is_some_and(|n| !n.is_layer()));
            v.dedup();
            Ok(v)
        }
        None => selected_roots(s),
    }
}

fn name_param<'a>(p: &'a Value, cmd: &str) -> Result<&'a str> {
    str_param(p, "name").filter(|n| !n.trim().is_empty()).ok_or_else(|| bad(cmd, "missing `name`"))
}

// ---------- brushes ----------

/// Brushable nodes (paths and compound paths) under `ids`.
fn brush_targets(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out = vec![];
    for id in ids {
        let Some(n) = d.node(*id) else { continue };
        n.walk(&mut |c| {
            let own = matches!(c.kind, NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. });
            let in_compound = d.parent_of(c.id).and_then(|p| d.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. }));
            if own && !in_compound && !out.contains(&c.id) {
                out.push(c.id);
            }
        });
    }
    out
}

fn brush_list(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let list: Vec<Value> = brush::library(d).iter().map(|b| json!({"name": b.name, "type": b.kind.type_id()})).collect();
    Ok(json!({ "brushes": list, "current": brush::current(d) }))
}

fn brush_get(s: &mut Session, p: &Value) -> Result<Value> {
    let name = name_param(p, "brush.get")?;
    let b = brush::find(&s.doc()?.doc, name).ok_or_else(|| EngineError::Other(format!("no brush named `{name}`")))?;
    Ok(serde_json::to_value(b).unwrap_or_default())
}

fn brush_apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.apply";
    let name = name_param(p, C)?.to_string();
    if brush::find(&s.doc()?.doc, &name).is_none() {
        return Err(bad(C, format!("no brush named `{name}`")));
    }
    let ids = brush_targets(&s.doc()?.doc, &roots(s, p)?);
    let stroke = if s.paint.stroke.is_none() { Paint::solid(Color::BLACK) } else { s.paint.stroke.clone() };
    s.edit("Apply Brush", |d, _| {
        d.unknown.insert(brush::CURRENT_KEY.into(), json!(name));
        for id in &ids {
            let Some(n) = d.node_mut(*id) else { continue };
            if n.appearance.stroke().is_none_or(|st| st.paint.is_none()) {
                n.appearance.set_stroke(stroke.clone());
            }
            if let Some(st) = n.appearance.stroke_mut() {
                if st.width <= 0.0 {
                    st.width = 1.0;
                }
                st.brush = Some(name.clone());
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn brush_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = brush_targets(&s.doc()?.doc, &roots(s, p)?);
    s.edit("Remove Brush Stroke", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                for it in &mut n.appearance.items {
                    if let AppearanceItem::Stroke(st) = it {
                        st.brush = None;
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

/// Set a document preference outside the undo history.
fn set_unknown(s: &mut Session, key: &str, v: Option<Value>) -> Result<()> {
    let st = s.doc_mut()?;
    let d = Arc::make_mut(&mut st.doc);
    match v {
        Some(v) => d.unknown.insert(key.into(), v),
        None => d.unknown.remove(key),
    };
    Ok(())
}

fn brush_set_current(s: &mut Session, p: &Value) -> Result<Value> {
    match str_param(p, "name") {
        Some(n) => {
            if brush::find(&s.doc()?.doc, n).is_none() {
                return Err(bad("brush.setCurrent", format!("no brush named `{n}`")));
            }
            set_unknown(s, brush::CURRENT_KEY, Some(json!(n)))?;
        }
        None => set_unknown(s, brush::CURRENT_KEY, None)?,
    }
    ok()
}

/// The selection as one art node (a group for several objects), ids kept.
pub(crate) fn selection_art(s: &Session, p: &Value) -> Result<Option<Node>> {
    let d = &s.doc()?.doc;
    let ids = roots(s, p)?;
    let nodes: Vec<Node> = ids.iter().filter_map(|id| d.node(*id).cloned()).collect();
    Ok(match nodes.len() {
        0 => None,
        1 => nodes.into_iter().next(),
        _ => Some(Node::group(NodeId(0), nodes.into_iter().map(Arc::new).collect())),
    })
}

fn merge(base: &mut Value, patch: &Value) {
    if let (Some(b), Some(pt)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in pt {
            b.insert(k.clone(), v.clone());
        }
    }
}

fn save_library(s: &mut Session, label: &str, lib: Vec<Brush>, rename: Option<(String, Option<String>)>) -> Result<()> {
    s.edit(label, |d, _| {
        brush::store(d, &lib);
        if let Some((from, to)) = rename {
            let ids: Vec<NodeId> = {
                let mut v = vec![];
                d.walk(|n| {
                    if n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(st) if st.brush.as_deref() == Some(from.as_str()))) {
                        v.push(n.id);
                    }
                });
                v
            };
            for id in ids {
                if let Some(n) = d.node_mut(id) {
                    for it in &mut n.appearance.items {
                        if let AppearanceItem::Stroke(st) = it
                            && st.brush.as_deref() == Some(from.as_str())
                        {
                            st.brush = to.clone();
                        }
                    }
                }
            }
            if brush::current(d).as_deref() == Some(from.as_str()) {
                match to {
                    Some(t) => d.unknown.insert(brush::CURRENT_KEY.into(), json!(t)),
                    None => d.unknown.remove(brush::CURRENT_KEY),
                };
            }
        }
        Ok(())
    })
}

fn brush_new(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.new";
    let ty = str_param(p, "type").ok_or_else(|| bad(C, "missing `type`"))?;
    let art_key = match ty {
        "calligraphic" | "bristle" => None,
        "scatter" | "art" => Some("art"),
        "pattern" => Some("side"),
        o => return Err(bad(C, format!("unknown brush type `{o}`"))),
    };
    let mut lib = brush::library(&s.doc()?.doc);
    let base_name = str_param(p, "name").map(str::to_string).unwrap_or_else(|| format!("New {} Brush", ty[..1].to_uppercase() + &ty[1..]));
    let name = brush::unique_name(&lib, &base_name);
    let mut v = json!({"name": name, "type": ty});
    if let Some(params) = p.get("params") {
        merge(&mut v, params);
        v["name"] = json!(name);
        v["type"] = json!(ty);
    }
    if let Some(k) = art_key
        && v.get(k).is_none()
    {
        let art = selection_art(s, p)?.ok_or_else(|| bad(C, format!("{ty} brushes need selected art (or params.{k})")))?;
        if art.geometric_bounds().is_none_or(|b| b.width() <= 1e-6) {
            return Err(bad(C, "the selected art has no width"));
        }
        v[k] = serde_json::to_value(&art).unwrap_or_default();
    }
    lib.push(parse_brush(C, v)?);
    save_library(s, "New Brush", lib, None)?;
    Ok(json!({ "name": name }))
}

/// A brush definition from command params, its values kept within their ranges.
fn parse_brush(cmd: &str, v: Value) -> Result<Brush> {
    let mut b: Brush = serde_json::from_value(v).map_err(|e| bad(cmd, format!("invalid brush: {e}")))?;
    b.sanitize();
    Ok(b)
}

fn brush_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = name_param(p, "brush.delete")?.to_string();
    let mut lib = brush::library(&s.doc()?.doc);
    let n = lib.len();
    lib.retain(|b| b.name != name);
    if lib.len() == n {
        return Err(bad("brush.delete", format!("no brush named `{name}`")));
    }
    save_library(s, "Delete Brush", lib, Some((name, None)))?;
    ok()
}

fn brush_duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let name = name_param(p, "brush.duplicate")?;
    let mut lib = brush::library(&s.doc()?.doc);
    let mut b = lib.iter().find(|b| b.name == name).cloned().ok_or_else(|| bad("brush.duplicate", format!("no brush named `{name}`")))?;
    b.name = brush::unique_name(&lib, str_param(p, "newName").unwrap_or(&format!("{name} copy")));
    let out = b.name.clone();
    lib.push(b);
    save_library(s, "Duplicate Brush", lib, None)?;
    Ok(json!({ "name": out }))
}

fn brush_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.options";
    let name = name_param(p, C)?.to_string();
    let mut lib = brush::library(&s.doc()?.doc);
    let i = lib.iter().position(|b| b.name == name).ok_or_else(|| bad(C, format!("no brush named `{name}`")))?;
    let mut v = serde_json::to_value(&lib[i]).unwrap_or_default();
    if let Some(params) = p.get("params") {
        merge(&mut v, params);
    }
    let new_name = str_param(p, "newName").filter(|n| !n.trim().is_empty() && *n != name).map(str::to_string);
    if let Some(nn) = &new_name {
        if lib.iter().any(|b| &b.name == nn) {
            return Err(bad(C, format!("a brush named `{nn}` exists")));
        }
        v["name"] = json!(nn);
    } else {
        v["name"] = json!(name);
    }
    v["type"] = json!(lib[i].kind.type_id());
    lib[i] = parse_brush(C, v)?;
    let out = lib[i].name.clone();
    let rename = new_name.map(|nn| (name, Some(nn)));
    save_library(s, "Brush Options", lib, rename)?;
    Ok(json!({ "name": out }))
}

fn expand_brush(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = brush_targets(&s.doc()?.doc, &roots(s, p)?);
    let ids: Vec<NodeId> = ids.into_iter().filter(|id| s.doc().ok().and_then(|st| st.doc.node(*id)).is_some_and(brush::has_brush)).collect();
    if ids.is_empty() {
        return Err(EngineError::Other("Expand: no brush strokes selected".into()));
    }
    let out = s.edit("Expand", |d, sel| {
        let mut out = vec![];
        for id in &ids {
            let Some(n) = d.node(*id).cloned() else { continue };
            let Some(g) = brush::expand(d, &n) else { continue };
            let g = d.reid(&g);
            let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
            let gid = g.id;
            d.remove(*id)?;
            d.insert(par, idx, g)?;
            out.push(gid);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&out) }))
}

fn brush_freehand(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "brush").map(str::to_string).filter(|n| s.doc().is_ok_and(|st| brush::find(&st.doc, n).is_some()));
    let spec = find_command("path.freehand").ok_or_else(|| EngineError::UnknownCommand("path.freehand".into()))?;
    let r = (spec.run)(s, p)?;
    let (Some(name), Some(id)) = (name, id_param(&r, "id")) else { return Ok(r) };
    s.edit("Paintbrush", |d, _| {
        if let Some(st) = d.node_mut(id).and_then(|n| n.appearance.stroke_mut()) {
            st.brush = Some(name.clone());
        }
        Ok(())
    })?;
    Ok(r)
}

// ---------- symbols ----------

fn natural_size(d: &Document, name: &str) -> (f64, f64) {
    d.unknown
        .get(SIZES_KEY)
        .and_then(|m| m.get(name))
        .and_then(|v| Some((v.get(0)?.as_f64()?, v.get(1)?.as_f64()?)))
        .unwrap_or((2.0 * HALF, 2.0 * HALF))
}

fn set_natural_size(d: &mut Document, name: &str, size: Option<(f64, f64)>) {
    let m = d.unknown.entry(SIZES_KEY.to_string()).or_insert_with(|| json!({}));
    if let Some(o) = m.as_object_mut() {
        match size {
            Some((w, h)) => o.insert(name.into(), json!([w, h])),
            None => o.remove(name),
        };
    }
}

/// Instance transform for a symbol of natural size `(w, h)` centred at `c`.
fn place_xf(c: Point, (w, h): (f64, f64)) -> Affine {
    Affine::translate(c.to_vec2()) * Affine::scale_non_uniform(w / (2.0 * HALF), h / (2.0 * HALF))
}

/// Normalise art for a symbol definition: returns (art, natural size, centre in the document).
fn normalize(mut art: Node) -> Result<(Node, (f64, f64), Point)> {
    let b = art.geometric_bounds().ok_or_else(|| EngineError::Other("the art has no bounds".into()))?;
    let (w, h) = (b.width().max(1.0), b.height().max(1.0));
    let c = b.center();
    let n = Affine::scale_non_uniform(2.0 * HALF / w, 2.0 * HALF / h) * Affine::translate(-c.to_vec2());
    art.transform(n, false);
    Ok((art, (w, h), c))
}

fn unique_symbol_name(d: &Document, base: &str) -> String {
    let taken = |n: &str| d.symbols.iter().any(|s| s.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

fn instances_of(d: &Document, name: Option<&str>) -> Vec<NodeId> {
    let mut v = vec![];
    d.walk(|n| {
        if let NodeKind::SymbolInstance { symbol, .. } = &n.kind
            && name.is_none_or(|x| x == symbol)
        {
            v.push(n.id);
        }
    });
    v
}

fn symbol_names(d: &Document) -> Vec<String> {
    d.symbols.iter().map(|s| s.name.clone()).collect()
}

fn current_symbol(d: &Document) -> Option<String> {
    d.unknown.get(CURRENT_SYMBOL).and_then(Value::as_str).filter(|n| d.symbols.iter().any(|s| s.name == *n)).map(str::to_string)
}

fn symbol_list(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let list: Vec<Value> = d
        .symbols
        .iter()
        .map(|sym| {
            let (w, h) = natural_size(d, &sym.name);
            json!({"name": sym.name, "instances": instances_of(d, Some(&sym.name)).len(), "size": [w, h]})
        })
        .collect();
    Ok(json!({ "symbols": list, "current": current_symbol(d).or_else(|| symbol_names(d).into_iter().next()) }))
}

/// Replace the root nodes `ids` with one instance of `name` (art already stored). Returns the id.
fn replace_with_instance(d: &mut Document, ids: &[NodeId], name: &str, xf: Affine) -> Result<NodeId> {
    let top = *ids.last().ok_or_else(|| EngineError::Other("nothing selected".into()))?;
    let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
    let id = d.alloc_id();
    d.insert(par, idx + 1, Node::new(id, NodeKind::SymbolInstance { symbol: name.into(), xf }))?;
    for r in ids {
        d.remove(*r)?;
    }
    Ok(id)
}

fn symbol_new(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = roots(s, p)?;
    let art = selection_art(s, p)?.ok_or_else(|| EngineError::Other("nothing selected".into()))?;
    let (art, size, c) = normalize(art)?;
    let name = unique_symbol_name(&s.doc()?.doc, str_param(p, "name").filter(|n| !n.trim().is_empty()).unwrap_or("New Symbol"));
    let id = s.edit("New Symbol", |d, sel| {
        let art = d.reid(&art);
        d.symbols.push(Symbol { name: name.clone(), art: Arc::new(art) });
        set_natural_size(d, &name, Some(size));
        d.unknown.insert(CURRENT_SYMBOL.into(), json!(name));
        let id = replace_with_instance(d, &ids, &name, place_xf(c, size))?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "name": name, "id": id.0 }))
}

fn resolve_symbol(d: &Document, p: &Value, cmd: &str) -> Result<String> {
    match str_param(p, "name") {
        Some(n) if d.symbols.iter().any(|s| s.name == n) => Ok(n.to_string()),
        Some(n) => Err(bad(cmd, format!("no symbol named `{n}`"))),
        None => {
            current_symbol(d).or_else(|| symbol_names(d).into_iter().next()).ok_or_else(|| EngineError::Other("the document has no symbols".into()))
        }
    }
}

fn symbol_place(s: &mut Session, p: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let name = resolve_symbol(d, p, "symbol.place")?;
    let centre = d.artboards.first().map(|a| a.rect.center()).unwrap_or(Point::ZERO);
    let c = Point::new(f64_or(p, "x", centre.x), f64_or(p, "y", centre.y));
    let size = natural_size(d, &name);
    let parent = s.doc()?.target_parent()?;
    let id = s.edit("Place Symbol Instance", |d, sel| {
        let id = d.alloc_id();
        d.insert(parent, usize::MAX, Node::new(id, NodeKind::SymbolInstance { symbol: name.clone(), xf: place_xf(c, size) }))?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// The plain art of instance `inst` in document space (fresh ids).
fn expanded_instance(d: &mut Document, inst: &Node) -> Option<Node> {
    let NodeKind::SymbolInstance { symbol, xf } = &inst.kind else { return None };
    let sym = d.symbols.iter().find(|s| &s.name == symbol)?;
    let mut art = (*sym.art).clone();
    if let Some(f) = inst.appearance.fill()
        && let Some(c) = f.paint.color()
    {
        brush::tint_node(&mut art, &c, f.opacity);
    }
    art.transform(*xf, false);
    let mut art = d.reid(&art);
    if inst.opacity < 1.0 || inst.blend != vectorcraft_color::BlendMode::Normal {
        if !matches!(art.kind, NodeKind::Group { .. }) {
            let id = d.alloc_id();
            art = Node::group(id, vec![Arc::new(art)]);
        }
        art.opacity *= inst.opacity;
        art.blend = inst.blend;
    }
    Some(art)
}

fn break_links(d: &mut Document, ids: &[NodeId]) -> Result<Vec<NodeId>> {
    let mut out = vec![];
    for id in ids {
        let Some(inst) = d.node(*id).cloned() else { continue };
        let Some(art) = expanded_instance(d, &inst) else { continue };
        let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
        let aid = art.id;
        d.remove(*id)?;
        d.insert(par, idx, art)?;
        out.push(aid);
    }
    Ok(out)
}

fn selected_instances(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let mut out = vec![];
    for id in roots(s, p)? {
        if let Some(n) = d.node(id) {
            n.walk(&mut |c| {
                if matches!(c.kind, NodeKind::SymbolInstance { .. }) {
                    out.push(c.id);
                }
            });
        }
    }
    Ok(out)
}

fn symbol_break_link(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_instances(s, p)?;
    if ids.is_empty() {
        return Err(EngineError::Other("no symbol instances selected".into()));
    }
    let out = s.edit("Break Link to Symbol", |d, sel| {
        let out = break_links(d, &ids)?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&out) }))
}

fn symbol_edit(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_instances(s, p)?;
    let first = *ids.first().ok_or_else(|| EngineError::Other("select a symbol instance".into()))?;
    let name = match &s.doc()?.doc.node(first).map(|n| n.kind.clone()) {
        Some(NodeKind::SymbolInstance { symbol, .. }) => symbol.clone(),
        _ => return Err(EngineError::Other("select a symbol instance".into())),
    };
    let out = s.edit("Edit Symbol", |d, sel| {
        let out = break_links(d, &[first])?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "name": name, "ids": ids_json(&out) }))
}

fn symbol_update(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.update";
    let name = name_param(p, C)?.to_string();
    if !s.doc()?.doc.symbols.iter().any(|x| x.name == name) {
        return Err(bad(C, format!("no symbol named `{name}`")));
    }
    let ids = roots(s, p)?;
    let art = selection_art(s, p)?.ok_or_else(|| EngineError::Other("nothing selected".into()))?;
    let (art, size, c) = normalize(art)?;
    let old = natural_size(&s.doc()?.doc, &name);
    let id = s.edit("Redefine Symbol", |d, sel| {
        let art = d.reid(&art);
        if let Some(sym) = d.symbols.iter_mut().find(|x| x.name == name) {
            sym.art = Arc::new(art);
        }
        set_natural_size(d, &name, Some(size));
        let fix = Affine::scale_non_uniform(size.0 / old.0, size.1 / old.1);
        for i in instances_of(d, Some(&name)) {
            if let Some(NodeKind::SymbolInstance { xf, .. }) = d.node_mut(i).map(|n| &mut n.kind) {
                *xf *= fix;
            }
        }
        let id = replace_with_instance(d, &ids, &name, place_xf(c, size))?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "name": name, "id": id.0 }))
}

fn symbol_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.delete";
    let name = name_param(p, C)?.to_string();
    if !s.doc()?.doc.symbols.iter().any(|x| x.name == name) {
        return Err(bad(C, format!("no symbol named `{name}`")));
    }
    let expand = bool_or(p, "expandInstances", true);
    s.edit("Delete Symbol", |d, _| {
        let inst = instances_of(d, Some(&name));
        if expand {
            break_links(d, &inst)?;
        } else {
            for i in inst {
                d.remove(i)?;
            }
        }
        d.symbols.retain(|x| x.name != name);
        set_natural_size(d, &name, None);
        if d.unknown.get(CURRENT_SYMBOL).and_then(Value::as_str) == Some(name.as_str()) {
            d.unknown.remove(CURRENT_SYMBOL);
        }
        Ok(())
    })?;
    ok()
}

fn symbol_duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.duplicate";
    let name = name_param(p, C)?.to_string();
    let d = &s.doc()?.doc;
    let sym = d.symbols.iter().find(|x| x.name == name).cloned().ok_or_else(|| bad(C, format!("no symbol named `{name}`")))?;
    let new = unique_symbol_name(d, str_param(p, "newName").unwrap_or(&format!("{name} copy")));
    let size = natural_size(d, &name);
    s.edit("Duplicate Symbol", |d, _| {
        let art = d.reid(&sym.art);
        d.symbols.push(Symbol { name: new.clone(), art: Arc::new(art) });
        set_natural_size(d, &new, Some(size));
        Ok(())
    })?;
    Ok(json!({ "name": new }))
}

fn symbol_replace(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.replace";
    let name = name_param(p, C)?.to_string();
    if !s.doc()?.doc.symbols.iter().any(|x| x.name == name) {
        return Err(bad(C, format!("no symbol named `{name}`")));
    }
    let ids = selected_instances(s, p)?;
    if ids.is_empty() {
        return Err(EngineError::Other("no symbol instances selected".into()));
    }
    s.edit("Replace Symbol", |d, _| {
        let new = natural_size(d, &name);
        for id in &ids {
            let old = match d.node(*id).map(|n| &n.kind) {
                Some(NodeKind::SymbolInstance { symbol, .. }) => natural_size(d, symbol),
                _ => continue,
            };
            if let Some(NodeKind::SymbolInstance { symbol, xf }) = d.node_mut(*id).map(|n| &mut n.kind) {
                *symbol = name.clone();
                *xf *= Affine::scale_non_uniform(new.0 / old.0, new.1 / old.1);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn symbol_set_current(s: &mut Session, p: &Value) -> Result<Value> {
    let name = resolve_symbol(&s.doc()?.doc, p, "symbol.setCurrent")?;
    set_unknown(s, CURRENT_SYMBOL, Some(json!(name)))?;
    ok()
}

fn symbol_select_instances(s: &mut Session, p: &Value) -> Result<Value> {
    let name = resolve_symbol(&s.doc()?.doc, p, "symbol.selectInstances")?;
    let ids = instances_of(&s.doc()?.doc, Some(&name));
    let n = ids.len();
    s.select(|_, sel| sel.set(ids))?;
    Ok(json!({ "count": n }))
}

// ---------- symbolism tools ----------

fn points(p: &Value, cmd: &str) -> Result<Vec<Point>> {
    let a = p.get("points").and_then(Value::as_array).ok_or_else(|| bad(cmd, "missing points [[x,y]…]"))?;
    let v: Vec<Point> = a.iter().filter_map(|q| Some(Point::new(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?))).collect();
    if v.is_empty() || v.iter().any(|q| !q.x.is_finite() || !q.y.is_finite()) {
        return Err(bad(cmd, "points must be finite [x,y] pairs"));
    }
    Ok(v)
}

/// Deterministic per-point jitter in [-1, 1]² (so previews re-applied on the snapshot agree).
fn jitter(i: usize, q: Point) -> (f64, f64, f64) {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ i as u64;
    for v in [(q.x * 16.0).round() as i64 as u64, (q.y * 16.0).round() as i64 as u64] {
        h ^= v;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        h ^= h >> 29;
    }
    let f = |k: u32| ((h.rotate_left(k * 21) & 0xffff) as f64 / 65535.0) * 2.0 - 1.0;
    (f(0), f(1), f(2))
}

fn is_symbol_set(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. }) && n.name.as_deref() == Some("Symbol Set")
}

fn symbol_spray(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.spray";
    let pts = points(p, C)?;
    let st = s.doc()?;
    let name = resolve_symbol(&st.doc, p, C)?;
    let radius = f64_or(p, "radius", 40.0).clamp(1.0, 2000.0);
    let density = f64_or(p, "density", 5.0).clamp(1.0, 10.0);
    let scale = f64_or(p, "scale", 1.0).clamp(0.01, 100.0);
    let alt = bool_or(p, "alt", false);
    let size = natural_size(&st.doc, &name);
    let set = st.selection.objects.first().copied().filter(|id| st.selection.len() == 1 && st.doc.node(*id).is_some_and(is_symbol_set));
    // Only a new Symbol Set needs a layer that takes new art.
    let parent = if alt || set.is_some() { st.insertion_parent() } else { st.target_parent()? };
    if alt {
        // Remove instances of this symbol under the brush (within the set, or anywhere).
        let d = &st.doc;
        let scope: Vec<NodeId> = match set {
            Some(g) => d.node(g).and_then(|n| n.children()).map(|c| c.iter().map(|c| c.id).collect()).unwrap_or_default(),
            None => instances_of(d, Some(&name)),
        };
        let doomed: Vec<NodeId> = scope
            .into_iter()
            .filter(|id| {
                d.node(*id).is_some_and(|n| {
                    matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if *symbol == name)
                        && n.geometric_bounds().is_some_and(|b| pts.iter().any(|q| q.distance(b.center()) <= radius))
                })
            })
            .collect();
        let n = doomed.len();
        if n > 0 {
            s.edit("Symbol Sprayer", |d, _| {
                for id in &doomed {
                    d.remove(*id)?;
                }
                Ok(())
            })?;
        }
        return Ok(json!({ "count": n }));
    }
    // Keep instances at least this far apart along the stroke (denser = closer).
    let min_gap = size.0.max(size.1) * scale * (1.15 - density * 0.1).max(0.1);
    let mut placed: Vec<Point> = vec![];
    for (i, q) in pts.iter().enumerate() {
        let (jx, jy, _) = jitter(i, *q);
        let c = Point::new(q.x + jx * radius * 0.5, q.y + jy * radius * 0.5);
        if placed.last().is_some_and(|l| l.distance(c) < min_gap) && i > 0 {
            continue;
        }
        placed.push(c);
    }
    let count = placed.len();
    let id = s.edit("Symbol Sprayer", |d, sel| {
        let mut nodes = vec![];
        for (i, c) in placed.iter().enumerate() {
            let (_, _, js) = jitter(i + 7, *c);
            let k = scale * (1.0 + js * 0.1);
            let id = d.alloc_id();
            nodes.push(Node::new(id, NodeKind::SymbolInstance { symbol: name.clone(), xf: place_xf(*c, size) * Affine::scale(k) }));
        }
        let gid = match set {
            Some(g) => {
                for n in nodes {
                    d.insert(Some(g), usize::MAX, n)?;
                }
                g
            }
            None => {
                let gid = d.alloc_id();
                let mut g = Node::group(gid, nodes.into_iter().map(Arc::new).collect());
                g.name = Some("Symbol Set".into());
                d.insert(parent, usize::MAX, g)?;
                gid
            }
        };
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0, "count": count }))
}

fn instance_center(n: &Node) -> Option<Point> {
    match &n.kind {
        NodeKind::SymbolInstance { xf, .. } => Some(*xf * Point::ZERO),
        _ => None,
    }
}

fn symbol_adjust(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.adjust";
    let tool = str_param(p, "tool").ok_or_else(|| bad(C, "missing `tool`"))?.to_string();
    if !["shift", "scrunch", "size", "spin", "stain", "screen", "style"].contains(&tool.as_str()) {
        return Err(bad(C, format!("unknown tool `{tool}`")));
    }
    let pts = points(p, C)?;
    let radius = f64_or(p, "radius", 40.0).clamp(1.0, 2000.0);
    let intensity = f64_or(p, "intensity", 5.0).clamp(1.0, 10.0) / 10.0;
    let alt = bool_or(p, "alt", false);
    let st = s.doc()?;
    let color = match p.get("color").and_then(color_value) {
        Some(c) => c,
        None => s.paint.fill.color().unwrap_or(Color::BLACK),
    };
    let style_paint = match str_param(p, "style") {
        Some(n) => st.doc.graphic_styles.iter().find(|g| g.name == n).map(|g| g.appearance.fill_paint()),
        None => st.doc.graphic_styles.get(1).or(st.doc.graphic_styles.first()).map(|g| g.appearance.fill_paint()),
    };
    let candidates: Vec<NodeId> = {
        let d = &st.doc;
        let mut v = vec![];
        if st.selection.is_empty() {
            v = instances_of(d, None);
        } else {
            for id in &st.selection.objects {
                if let Some(n) = d.node(*id) {
                    n.walk(&mut |c| {
                        if matches!(c.kind, NodeKind::SymbolInstance { .. }) && !v.contains(&c.id) {
                            v.push(c.id);
                        }
                    });
                }
            }
        }
        v.retain(|id| d.is_editable(*id));
        v
    };
    let label = match tool.as_str() {
        "shift" => "Symbol Shifter",
        "scrunch" => "Symbol Scruncher",
        "size" => "Symbol Sizer",
        "spin" => "Symbol Spinner",
        "stain" => "Symbol Stainer",
        "screen" => "Symbol Screener",
        _ => "Symbol Styler",
    };
    let count = s.edit(label, |d, _| {
        let mut touched = std::collections::BTreeSet::new();
        for (j, q) in pts.iter().enumerate() {
            let delta = if j > 0 { *q - pts[j - 1] } else { Vec2::ZERO };
            for id in &candidates {
                let Some(n) = d.node_mut(*id) else { continue };
                let Some(c) = instance_center(n) else { continue };
                let dist = c.distance(*q);
                if dist > radius {
                    continue;
                }
                let w = intensity * (1.0 - dist / radius).max(0.0);
                if w <= 0.0 {
                    continue;
                }
                touched.insert(*id);
                let about = |m: Affine| Affine::translate(c.to_vec2()) * m * Affine::translate(-c.to_vec2());
                match tool.as_str() {
                    "shift" => n.transform(Affine::translate(delta * w.min(1.0)), false),
                    "scrunch" => {
                        let v = (*q - c) * (0.15 * w) * if alt { -1.0 } else { 1.0 };
                        n.transform(Affine::translate(v), false);
                    }
                    "size" => {
                        let k = 1.0 + 0.08 * w;
                        n.transform(about(Affine::scale(if alt { 1.0 / k } else { k })), false);
                    }
                    "spin" => {
                        // Turn towards the drag direction; a click spins a little.
                        let a = if delta.hypot() > 1e-9 { (c - *q).cross(delta).signum() * 0.1 * w } else { 0.05 * w };
                        n.transform(about(Affine::rotate(if alt { -a } else { a })), false);
                    }
                    "stain" | "style" => {
                        let paint = if tool == "style" { style_paint.clone().unwrap_or(Paint::solid(color)) } else { Paint::solid(color) };
                        if tool == "style" && alt {
                            n.appearance.items.clear();
                            continue;
                        }
                        let cur = n.appearance.fill().filter(|f| f.paint == paint).map(|f| f.opacity).unwrap_or(0.0);
                        let step = if tool == "style" { 1.0 } else { 0.15 * w as f32 };
                        let o = if alt { (cur - step).max(0.0) } else { (cur + step).min(1.0) };
                        n.appearance.items.retain(|i| !matches!(i, AppearanceItem::Fill(_)));
                        if o > 0.0 {
                            let mut f = FillLayer::new(paint);
                            f.opacity = o;
                            n.appearance.items.insert(0, AppearanceItem::Fill(f));
                        }
                    }
                    _ => {
                        let step = (0.1 * w) as f32;
                        n.opacity = if alt { (n.opacity + step).min(1.0) } else { (n.opacity - step).max(0.05) };
                    }
                }
            }
        }
        Ok(touched.len())
    })?;
    Ok(json!({ "count": count }))
}
