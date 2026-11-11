use super::dynamic::{fit_overrides, type_id, type_param};
use super::{
    Affine, AppearanceItem, Arc, CURRENT_SYMBOL, Color, Document, EngineError, FillLayer, HALF, Node, NodeId, NodeKind, Paint, Point, Result,
    SIZES_KEY, Session, Symbol, SymbolExportType, SymbolNineSlice, SymbolRegistration, Value, Vec2, bad, brush, color_param, ids_json, json,
    name_param, number_or, ok, remove_each, roots, selection_art, separate, set_unknown, str_param, switch_or,
};

// ---------- symbols ----------

pub(crate) fn natural_size(d: &Document, name: &str) -> (f64, f64) {
    stored_natural_size(d, name).unwrap_or((2.0 * HALF, 2.0 * HALF))
}

/// The natural size `d` records for symbol `name` (`None` when it records none).
pub(crate) fn stored_natural_size(d: &Document, name: &str) -> Option<(f64, f64)> {
    d.unknown.get(SIZES_KEY).and_then(|m| m.get(name)).and_then(|v| Some((v.get(0)?.as_f64()?, v.get(1)?.as_f64()?)))
}

pub(crate) fn set_natural_size(d: &mut Document, name: &str, size: Option<(f64, f64)>) {
    let m = d.unknown.entry(SIZES_KEY.to_string()).or_insert_with(|| json!({}));
    if let Some(o) = m.as_object_mut() {
        match size {
            Some((w, h)) => o.insert(name.into(), json!([w, h])),
            None => o.remove(name),
        };
    }
}

/// Instance transform for a symbol of natural size `(w, h)` whose registration point is at `at`.
pub(crate) fn place_xf(at: Point, (w, h): (f64, f64), reg: SymbolRegistration) -> Affine {
    let c = at - reg.offset_from_center(w, h);
    Affine::translate(c.to_vec2()) * Affine::scale_non_uniform(w / (2.0 * HALF), h / (2.0 * HALF))
}

fn registration_param(p: &Value, cmd: &str) -> Result<Option<SymbolRegistration>> {
    match p.get("registration") {
        None => Ok(None),
        Some(v) => v.as_u64().and_then(SymbolRegistration::from_u64).map(Some).ok_or_else(|| bad(cmd, "registration must be 0..8")),
    }
}

fn export_type_param(p: &Value, cmd: &str) -> Result<Option<SymbolExportType>> {
    match str_param(p, "exportType") {
        None => Ok(None),
        Some(s) => SymbolExportType::from_id(s).map(Some).ok_or_else(|| bad(cmd, format!("exportType must be movieClip or graphic, not `{s}`"))),
    }
}

/// `nineSlice` (bool) and `nineSliceGuides` ([left, top, right, bottom] fractions of the natural
/// size): `None` leaves the symbol alone, `Some(None)` turns 9-slice off, `Some(Some(g))` turns it
/// on with guides `g` (the symbol's current ones, else a quarter in from each edge, when none are
/// given).
fn nine_slice_param(p: &Value, cmd: &str, current: Option<SymbolNineSlice>) -> Result<Option<Option<SymbolNineSlice>>> {
    let on = match p.get("nineSlice") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(cmd, "nineSlice must be true or false"))?),
    };
    let guides =
        match p.get("nineSliceGuides") {
            None | Some(Value::Null) => None,
            Some(v) => Some(SymbolNineSlice::from_value(v).ok_or_else(|| {
                bad(cmd, "nineSliceGuides must be [left, top, right, bottom], each 0..1, with left + right < 1 and top + bottom < 1")
            })?),
        };
    Ok(match (on, guides) {
        (Some(false), _) => Some(None),
        (None, None) => None,
        (_, Some(g)) => Some(Some(g)),
        (Some(true), None) => Some(Some(current.unwrap_or_default())),
    })
}

fn nine_slice_json(d: &Document, name: &str) -> Value {
    d.symbol_nine_slice(name).map_or(Value::Null, SymbolNineSlice::to_value)
}

fn rename_natural_size(d: &mut Document, from: &str, to: &str) {
    if from == to {
        return;
    }
    let size = d.unknown.get(SIZES_KEY).and_then(|m| m.get(from)).cloned();
    set_natural_size(d, from, None);
    if let Some(v) = size
        && let (Some(w), Some(h)) = (v.get(0).and_then(Value::as_f64), v.get(1).and_then(Value::as_f64))
    {
        set_natural_size(d, to, Some((w, h)));
    }
}

/// Normalise art for a symbol definition: returns (art, natural size, centre in the document).
pub(crate) fn normalize(mut art: Node) -> Result<(Node, (f64, f64), Point)> {
    let b = art.geometric_bounds().ok_or_else(|| EngineError::Other("the art has no bounds".into()))?;
    let (w, h) = (b.width().max(1.0), b.height().max(1.0));
    let c = b.center();
    let n = Affine::scale_non_uniform(2.0 * HALF / w, 2.0 * HALF / h) * Affine::translate(-c.to_vec2());
    art.transform(n, false);
    Ok((art, (w, h), c))
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

/// The symbols an instance in `n` (its opacity masks included) places, pushed onto `out`.
fn art_symbols(n: &Node, out: &mut Vec<String>) {
    vectorcraft_doc::swatches::walk_with_masks(n, &mut |m| {
        if let NodeKind::SymbolInstance { symbol, .. } = &m.kind {
            out.push(symbol.clone());
        }
    });
}

/// Select All Unused (#4918): the symbols nothing places. Instances in the art (opacity masks
/// included), graph designs, pattern tiles and the tiles that pattern editing's Cancel puts back,
/// and the art of the document's brushes count, and so do the instances in the art of every symbol
/// a counted instance places. A symbol placed only inside an unused symbol is unused too.
pub(super) fn symbol_unused(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let mut todo = vec![];
    let lib = brush::library(d);
    let patterns = d.patterns.iter().chain(d.pattern_edit.iter().filter_map(|pe| pe.original.as_ref()));
    let trees = d.layers.iter().map(|l| &**l).chain(d.graph_designs.iter().map(|g| &*g.art));
    trees.chain(patterns.flat_map(|p| p.art.iter().map(|n| &**n))).chain(lib.iter().flat_map(|b| b.art())).for_each(|n| art_symbols(n, &mut todo));
    let mut used = std::collections::BTreeSet::new();
    while let Some(name) = todo.pop() {
        if !used.contains(&name) {
            d.symbols.iter().filter(|sym| sym.name == name).for_each(|sym| art_symbols(&sym.art, &mut todo));
            used.insert(name);
        }
    }
    let names: Vec<&str> = d.symbols.iter().map(|sym| sym.name.as_str()).filter(|n| !used.contains(*n)).collect();
    Ok(json!({ "names": names }))
}

fn symbol_names(d: &Document) -> Vec<String> {
    d.symbols.iter().map(|s| s.name.clone()).collect()
}

fn current_symbol(d: &Document) -> Option<String> {
    d.unknown.get(CURRENT_SYMBOL).and_then(Value::as_str).filter(|n| d.symbols.iter().any(|s| s.name == *n)).map(str::to_string)
}

pub(super) fn symbol_list(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let list: Vec<Value> = d
        .symbols
        .iter()
        .map(|sym| {
            let (w, h) = natural_size(d, &sym.name);
            json!({
                "name": sym.name,
                "instances": instances_of(d, Some(&sym.name)).len(),
                "size": [w, h],
                "registration": d.symbol_registration(&sym.name).0,
                "exportType": d.symbol_export_type(&sym.name).id(),
                "nineSlice": nine_slice_json(d, &sym.name),
                "type": type_id(sym.dynamic),
            })
        })
        .collect();
    Ok(json!({ "symbols": list, "current": current_symbol(d).or_else(|| symbol_names(d).into_iter().next()) }))
}

/// Store `art` ([`normalize`]d, of natural size `size`) as symbol `name`, which becomes the
/// current symbol.
pub(crate) fn store_symbol(d: &mut Document, art: &Node, name: &str, size: (f64, f64)) {
    let art = d.reid(art);
    d.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(art) });
    set_natural_size(d, name, Some(size));
    d.unknown.insert(CURRENT_SYMBOL.into(), json!(name));
}

/// Replace the root nodes `ids` with one instance of symbol `name` (already stored) placed by
/// `xf`. Frames of threaded text among them leave their threads with the text they show. Returns
/// the instance's id.
pub(crate) fn instance_in_place(d: &mut Document, ids: &[NodeId], name: &str, xf: Affine) -> Result<NodeId> {
    let threaded = crate::cmd::threads::frames(d);
    let id = replace_with_instance(d, ids, name, xf)?;
    crate::cmd::threads::leave_threads(d, &threaded);
    Ok(id)
}

/// Replace the root nodes `ids` with one instance of `name` (art already stored). Returns the id.
fn replace_with_instance(d: &mut Document, ids: &[NodeId], name: &str, xf: Affine) -> Result<NodeId> {
    let top = *ids.last().ok_or_else(|| EngineError::Other("nothing selected".into()))?;
    if separate(d, ids) {
        let id = d.alloc_id();
        return Ok(d.wrap(ids, |_| Node::new(id, NodeKind::SymbolInstance { symbol: name.into(), xf }))?);
    }
    let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
    let id = d.alloc_id();
    d.insert(par, idx + 1, Node::new(id, NodeKind::SymbolInstance { symbol: name.into(), xf }))?;
    for r in ids {
        d.remove(*r)?;
    }
    Ok(id)
}

pub(super) fn symbol_new(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.new";
    let ids = roots(s, p)?;
    let mut art = selection_art(s, p)?.ok_or_else(|| EngineError::Other("nothing selected".into()))?;
    crate::cmd::threads::keep_shown_text(&s.doc()?.doc, &mut art);
    let (art, size, c) = normalize(art)?;
    let name = s.doc()?.doc.unique_symbol_name(str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).unwrap_or("New Symbol"));
    let reg = registration_param(p, C)?.unwrap_or(SymbolRegistration::CENTER);
    let export = export_type_param(p, C)?.unwrap_or_default();
    let nine = nine_slice_param(p, C, None)?.flatten();
    let dynamic = type_param(p, C)?.unwrap_or(false);
    // Keep the art where it was: the geometric centre stays at `c`, whatever the registration is.
    let place = place_xf(c + reg.offset_from_center(size.0, size.1), size, reg);
    let id = s.edit("New Symbol", |d, sel| {
        store_symbol(d, &art, &name, size);
        if let Some(sym) = d.symbols.iter_mut().find(|x| x.name == name) {
            sym.dynamic = dynamic;
        }
        d.set_symbol_registration(&name, reg);
        d.set_symbol_export_type(&name, export);
        d.set_symbol_nine_slice(&name, nine);
        let id = instance_in_place(d, &ids, &name, place)?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({
        "name": name,
        "id": id.0,
        "registration": reg.0,
        "exportType": export.id(),
        "nineSlice": nine.map_or(Value::Null, SymbolNineSlice::to_value),
        "type": type_id(dynamic),
    }))
}

pub(super) fn symbol_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.options";
    let name = resolve_symbol(&s.doc()?.doc, p, C)?;
    let export = export_type_param(p, C)?;
    let reg = registration_param(p, C)?;
    let nine = nine_slice_param(p, C, s.doc()?.doc.symbol_nine_slice(&name))?;
    let dynamic = type_param(p, C)?;
    let new_name = str_param(p, "newName").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    if new_name.is_none() && export.is_none() && reg.is_none() && nine.is_none() && dynamic.is_none() {
        return Err(bad(C, "pass newName, registration, exportType, nineSlice and/or type"));
    }
    if let Some(ref n) = new_name
        && n != &name
        && s.doc()?.doc.symbols.iter().any(|x| x.name == *n)
    {
        return Err(bad(C, format!("a symbol named `{n}` already exists")));
    }
    let final_name = new_name.clone().unwrap_or_else(|| name.clone());
    s.edit("Symbol Options", |d, _| {
        if let Some(ref n) = new_name
            && n != &name
        {
            if let Some(sym) = d.symbols.iter_mut().find(|x| x.name == name) {
                sym.name = n.clone();
            }
            for id in instances_of(d, Some(&name)) {
                if let Some(NodeKind::SymbolInstance { symbol, .. }) = d.node_mut(id).map(|node| &mut node.kind) {
                    *symbol = n.clone();
                }
            }
            rename_natural_size(d, &name, n);
            d.rename_symbol_registration(&name, n);
            d.rename_symbol_export_type(&name, n);
            d.rename_symbol_nine_slice(&name, n);
            if d.unknown.get(CURRENT_SYMBOL).and_then(Value::as_str) == Some(name.as_str()) {
                d.unknown.insert(CURRENT_SYMBOL.into(), json!(n));
            }
            // Symbol editing mode follows the new name, and so does its layer.
            if let Some(e) = d.symbol_edit.as_mut().filter(|e| e.symbol == name) {
                e.symbol = n.clone();
                let layer = e.layer;
                if let Some(l) = d.node_mut(layer) {
                    l.name = Some(n.clone());
                }
            }
        }
        if let Some(r) = reg {
            d.set_symbol_registration(&final_name, r);
        }
        if let Some(kind) = export {
            d.set_symbol_export_type(&final_name, kind);
        }
        if let Some(g) = nine {
            d.set_symbol_nine_slice(&final_name, g);
        }
        if let Some(dynamic) = dynamic
            && let Some(sym) = d.symbols.iter_mut().find(|x| x.name == final_name)
        {
            // A static symbol's instances drop their own appearance changes.
            sym.dynamic = dynamic;
            fit_overrides(d, &instances_of(d, Some(&final_name)));
        }
        Ok(())
    })?;
    let d = &s.doc()?.doc;
    Ok(json!({
        "name": final_name,
        "registration": d.symbol_registration(&final_name).0,
        "exportType": d.symbol_export_type(&final_name).id(),
        "nineSlice": nine_slice_json(d, &final_name),
        "type": type_id(d.symbols.iter().any(|x| x.name == final_name && x.dynamic)),
    }))
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

pub(super) fn symbol_place(s: &mut Session, p: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let name = resolve_symbol(d, p, "symbol.place")?;
    let centre = d.artboards.first().map(|a| a.rect.center()).unwrap_or(Point::ZERO);
    let at = Point::new(number_or(p, "x", centre.x, "symbol.place")?, number_or(p, "y", centre.y, "symbol.place")?);
    let size = natural_size(d, &name);
    let reg = d.symbol_registration(&name);
    let parent = s.doc()?.target_parent()?;
    let id = s.edit("Place Symbol Instance", |d, sel| {
        let id = d.alloc_id();
        d.insert(parent, usize::MAX, Node::new(id, NodeKind::SymbolInstance { symbol: name.clone(), xf: place_xf(at, size, reg) }))?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// The plain art of instance `inst` in document space (fresh ids), with the instance's own
/// appearance changes to a dynamic symbol's art when `own`, placed through its symbol's 9-slice
/// guides when it has them. The instance's opacity mask goes on the art, or on a group around it
/// when the art has a mask of its own.
fn expanded_instance(d: &mut Document, inst: &Node, own: bool) -> Option<Node> {
    let NodeKind::SymbolInstance { symbol, xf } = &inst.kind else { return None };
    let sym = d.symbols.iter().find(|s| &s.name == symbol)?;
    let mut art = if own { vectorcraft_doc::symbol_overrides::dynamic_art(&sym.art, inst) } else { (*sym.art).clone() };
    if let Some((color, amount)) = brush::stain(inst) {
        brush::tint_node(&mut art, &color, amount);
    }
    let art = d.place_symbol_art(symbol, &art, *xf)?;
    let mut art = d.reid(&art);
    if inst.opacity < 1.0 || inst.blend != vectorcraft_color::BlendMode::Normal {
        if !matches!(art.kind, NodeKind::Group { .. }) {
            let id = d.alloc_id();
            art = Node::group(id, vec![Arc::new(art)]);
        }
        art.opacity *= inst.opacity;
        art.blend = inst.blend;
    }
    if let Some(mask) = &inst.extras().mask {
        if art.extras().mask.is_some() {
            let id = d.alloc_id();
            art = Node::group(id, vec![Arc::new(art)]);
        }
        art.extras_mut().mask = Some(mask.clone());
    }
    Some(art)
}

/// The most objects [`expanded_instance`] makes of instance `inst` in `d` (`own` as there): its
/// symbol's object count with the art of its opacity masks, or
/// [`vectorcraft_doc::NineSliceMap::most_placed`] when it is placed through 9-slice guides, plus
/// the group that opacity, a blending mode or the instance's opacity mask adds, plus the art of
/// the instance's opacity mask. 0 for any other object.
fn expanded_bound(d: &Document, inst: &Node, own: bool) -> u64 {
    let NodeKind::SymbolInstance { symbol, xf } = &inst.kind else { return 0 };
    let Some(sym) = d.symbols.iter().find(|s| &s.name == symbol) else { return 0 };
    let dynamic = own && !inst.extras().symbol_overrides.is_empty();
    let most = match d.symbol_slice_map(symbol, *xf) {
        Some(m) if dynamic => m.most_placed(&vectorcraft_doc::symbol_overrides::dynamic_art(&sym.art, inst), d),
        Some(m) => m.most_placed(&sym.art, d),
        None if dynamic => vectorcraft_doc::symbol_overrides::dynamic_art(&sym.art, inst).count_with_masks() as u64,
        None => sym.art.count_with_masks() as u64,
    };
    let mask = inst.extras().mask.as_ref().map_or(0, |m| m.art.count_with_masks() as u64);
    most.saturating_add(1).saturating_add(mask)
}

/// Replace instances `ids` with their art ([`expanded_instance`], `own` as there). A hidden or
/// locked instance's art stays hidden or locked. Returns each replaced instance's id with its art's
/// id. Fails before any edit when the instances together would make more than
/// [`MAX_EXPAND_NODES`] objects.
fn break_links(d: &mut Document, ids: &[NodeId], own: bool) -> Result<Vec<(NodeId, NodeId)>> {
    let total = ids.iter().filter_map(|id| d.node(*id)).fold(0u64, |sum, n| sum.saturating_add(expanded_bound(d, n, own)));
    if total > MAX_EXPAND_NODES as u64 {
        return Err(EngineError::Other("Too many pieces to make at once".into()));
    }
    let mut out = vec![];
    for id in ids {
        let Some(inst) = d.node(*id).cloned() else { continue };
        let Some(mut art) = expanded_instance(d, &inst, own) else { continue };
        let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
        art.visible &= inst.visible;
        art.locked |= inst.locked;
        let aid = art.id;
        d.remove(*id)?;
        d.insert(par, idx, art)?;
        out.push((*id, aid));
    }
    Ok(out)
}

/// Object › Expand: the symbol instances under `roots` of `d` ([`expandable_instances`]) replaced
/// by their art, as Break Link places it, each keeping the instance's id and place. The instances
/// that art holds stay instances. A hidden or locked instance's art stays hidden or locked. When
/// the instances together would make more than [`MAX_EXPAND_NODES`] objects, all of them stay
/// instances. Returns whether any instance was expanded.
pub(crate) fn expand_instances_under(d: &mut Document, roots: &[NodeId]) -> Result<bool> {
    let ids = expandable_instances(d, roots);
    let total = ids.iter().filter_map(|id| d.node(*id)).fold(0u64, |sum, n| sum.saturating_add(expanded_bound(d, n, true)));
    if total > MAX_EXPAND_NODES as u64 {
        return Ok(false);
    }
    let mut any = false;
    for id in ids {
        let Some(inst) = d.node(id).cloned() else { continue };
        let Some(mut art) = expanded_instance(d, &inst, true) else { continue };
        let (par, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
        art.id = id;
        art.visible &= inst.visible;
        art.locked |= inst.locked;
        d.remove(id)?;
        d.insert(par, idx, art)?;
        any = true;
    }
    Ok(any)
}

/// The symbol instances under `roots` of `d` that Object › Expand turns into their art: those
/// without effects (Expand Appearance bakes the others), with no effects on their fills and
/// strokes, not in perspective, and whose symbol `d` holds. The instances in blends, repeats and
/// Intertwine groups are left to Expand, which turns those into plain groups.
pub(crate) fn expandable_instances(d: &Document, roots: &[NodeId]) -> Vec<NodeId> {
    fn collect(d: &Document, n: &Node, out: &mut Vec<NodeId>) {
        match &n.kind {
            NodeKind::SymbolInstance { symbol, .. } => {
                if n.appearance.effects.is_empty()
                    && n.appearance.items.iter().all(|i| i.effects().is_empty())
                    && n.projection().is_none()
                    && d.symbols.iter().any(|s| &s.name == symbol)
                {
                    out.push(n.id);
                }
            }
            NodeKind::Blend { .. } | NodeKind::Repeat(_) | NodeKind::Intertwine { .. } => {}
            _ => {
                for c in n.children().into_iter().flatten() {
                    collect(d, c, out);
                }
            }
        }
    }
    let mut out = vec![];
    for n in roots.iter().filter_map(|r| d.node(*r)) {
        collect(d, n, &mut out);
    }
    out
}

/// How many objects [`break_links`] and [`expand_instances_under`] may make at once.
const MAX_EXPAND_NODES: usize = 1_000_000;

pub(super) fn selected_instances(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
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

pub(super) fn symbol_break_link(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_instances(s, p)?;
    if ids.is_empty() {
        return Err(EngineError::Other("no symbol instances selected".into()));
    }
    let iso = s.doc()?.isolation;
    let out = s.edit("Break Link to Symbol", |d, sel| {
        let fresh = d.new_art.get();
        let made = break_links(d, &ids, true)?;
        // Art that is hidden or locked, or in a hidden or locked group or layer, is left out of the
        // selection, as is art on a locked artboard unless the instance it replaces was new art.
        let locks = d.locked_artboards();
        sel.set(made.iter().filter_map(|(inst, art)| {
            let keep = if fresh.contains(inst) {
                crate::cmd::select::is_selectable_on_any_artboard(d, iso, *art)
            } else {
                crate::cmd::select::is_selectable(d, iso, &locks, *art)
            };
            keep.then_some(*art)
        }));
        Ok(made.into_iter().map(|(_, art)| art).collect::<Vec<_>>())
    })?;
    Ok(json!({ "ids": ids_json(&out) }))
}

pub(super) fn symbol_update(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.update";
    let name = name_param(p, C)?.to_string();
    if !s.doc()?.doc.symbols.iter().any(|x| x.name == name) {
        return Err(bad(C, format!("no symbol named `{name}`")));
    }
    // Redefining the symbol being edited leaves symbol editing mode in the same step.
    let session = s.doc()?.doc.symbol_edit.clone().filter(|e| e.symbol == name);
    if session.is_some() {
        crate::cmd::settle_interaction(s)?;
        s.end_text_edit();
    }
    let ids = roots(s, p)?;
    // The art on the editing layer can't redefine it: the symbol would hold an instance of itself.
    if let Some(se) = &session {
        let d = &s.doc()?.doc;
        if ids.iter().any(|i| d.ancestry(*i).is_some_and(|a| a.contains(&se.layer))) {
            return Err(EngineError::Other("a symbol can't hold an instance of itself".into()));
        }
    }
    let mut art = selection_art(s, p)?.ok_or_else(|| EngineError::Other("nothing selected".into()))?;
    crate::cmd::threads::keep_shown_text(&s.doc()?.doc, &mut art);
    let (art, size, c) = normalize(art)?;
    let old = natural_size(&s.doc()?.doc, &name);
    let editing = session.is_some();
    let id = s.edit("Redefine Symbol", |d, sel| {
        if editing {
            crate::cmd::symboledit::finish(d, sel);
        }
        let art = d.reid(&art);
        if let Some(sym) = d.symbols.iter_mut().find(|x| x.name == name) {
            sym.art = Arc::new(art);
        }
        set_natural_size(d, &name, Some(size));
        let fix = Affine::scale_non_uniform(size.0 / old.0, size.1 / old.1);
        let instances = instances_of(d, Some(&name));
        for i in &instances {
            if let Some(NodeKind::SymbolInstance { xf, .. }) = d.node_mut(*i).map(|n| &mut n.kind) {
                *xf *= fix;
            }
        }
        // Each instance keeps its appearance changes to the objects the new art still has.
        fit_overrides(d, &instances);
        let reg = d.symbol_registration(&name);
        let id = instance_in_place(d, &ids, &name, place_xf(c + reg.offset_from_center(size.0, size.1), size, reg))?;
        sel.set([id]);
        Ok(id)
    })?;
    if let Some(se) = &session {
        crate::cmd::symboledit::leave_editing(s, se)?;
    }
    Ok(json!({ "name": name, "id": id.0 }))
}

/// How many levels of nested instances Delete Symbol expands before it gives up.
const MAX_DELETE_ROUNDS: usize = 256;

pub(super) fn symbol_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.delete";
    // `names` deletes several symbols in one undo step (the Symbols panel's Select All Unused, #4918).
    let names: Vec<String> = match p.get("names") {
        Some(v) => {
            let a = v.as_array().ok_or_else(|| bad(C, "names must be an array of symbol names"))?;
            let mut names = Vec::with_capacity(a.len());
            for n in a {
                let n = n.as_str().ok_or_else(|| bad(C, "names must be an array of symbol names"))?;
                if !names.iter().any(|x: &String| x == n) {
                    names.push(n.to_string());
                }
            }
            if names.is_empty() {
                return Err(bad(C, "names is empty"));
            }
            names
        }
        None => vec![name_param(p, C)?.to_string()],
    };
    let d = &s.doc()?.doc;
    if let Some(name) = names.iter().find(|n| !d.symbols.iter().any(|x| &x.name == *n)) {
        return Err(bad(C, format!("no symbol named `{name}`")));
    }
    let expand = switch_or(p, "expandInstances", true, C)?;
    // Deleting the symbol being edited leaves symbol editing mode in the same step.
    let session = s.doc()?.doc.symbol_edit.clone().filter(|e| names.contains(&e.symbol));
    let editing = session.is_some();
    if editing {
        crate::cmd::settle_interaction(s)?;
        s.end_text_edit();
    }
    let label = if names.len() == 1 { "Delete Symbol" } else { "Delete Symbols" };
    s.edit(label, |d, sel| {
        if editing {
            crate::cmd::symboledit::finish(d, sel);
        }
        // Every instance goes before any definition, whatever the order of `names`: expanding an
        // instance of one symbol can bring out instances of another being deleted (nested in its
        // art), which expand in turn while their definitions are still there. Each round expands
        // one level of nesting, so the rounds are capped.
        let of_names = |d: &Document| {
            let mut v = vec![];
            d.walk(|n| {
                if let NodeKind::SymbolInstance { symbol, .. } = &n.kind
                    && names.contains(symbol)
                {
                    v.push(n.id);
                }
            });
            v
        };
        for _ in 0..=MAX_DELETE_ROUNDS {
            let inst = of_names(d);
            if inst.is_empty() {
                break;
            }
            if expand {
                break_links(d, &inst, true)?;
            } else {
                remove_each(d, &inst)?;
            }
        }
        if !of_names(d).is_empty() {
            return Err(EngineError::Other("the symbols are nested too deeply to expand their instances".into()));
        }
        for name in &names {
            d.symbols.retain(|x| &x.name != name);
            set_natural_size(d, name, None);
            d.set_symbol_registration(name, SymbolRegistration::CENTER);
            d.clear_symbol_export_type(name);
            d.set_symbol_nine_slice(name, None);
            if d.unknown.get(CURRENT_SYMBOL).and_then(Value::as_str) == Some(name.as_str()) {
                d.unknown.remove(CURRENT_SYMBOL);
            }
        }
        Ok(())
    })?;
    if let Some(se) = &session {
        crate::cmd::symboledit::leave_editing(s, se)?;
    }
    ok()
}

pub(super) fn symbol_duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.duplicate";
    let name = name_param(p, C)?.to_string();
    let d = &s.doc()?.doc;
    let sym = d.symbols.iter().find(|x| x.name == name).cloned().ok_or_else(|| bad(C, format!("no symbol named `{name}`")))?;
    let given = str_param(p, "newName").map(str::trim).filter(|n| !n.is_empty());
    let new = d.unique_symbol_name(&given.map_or_else(|| format!("{} copy", name.trim()), str::to_string));
    let size = natural_size(d, &name);
    let reg = d.symbol_registration(&name);
    let export = d.symbol_export_type(&name);
    let nine = d.symbol_nine_slice(&name);
    s.edit("Duplicate Symbol", |d, _| {
        let art = d.reid(&sym.art);
        d.symbols.push(Symbol { dynamic: sym.dynamic, name: new.clone(), art: Arc::new(art) });
        set_natural_size(d, &new, Some(size));
        d.set_symbol_registration(&new, reg);
        d.set_symbol_export_type(&new, export);
        d.set_symbol_nine_slice(&new, nine);
        Ok(())
    })?;
    Ok(json!({ "name": new, "registration": reg.0, "exportType": export.id(), "nineSlice": nine.map_or(Value::Null, SymbolNineSlice::to_value) }))
}

pub(super) fn symbol_replace(s: &mut Session, p: &Value) -> Result<Value> {
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
            let (old, other) = match d.node(*id).map(|n| &n.kind) {
                Some(NodeKind::SymbolInstance { symbol, .. }) => (natural_size(d, symbol), *symbol != name),
                _ => continue,
            };
            let Some(n) = d.node_mut(*id) else { continue };
            if let NodeKind::SymbolInstance { symbol, xf } = &mut n.kind {
                *symbol = name.clone();
                *xf *= Affine::scale_non_uniform(new.0 / old.0, new.1 / old.1);
            }
            // An override's path names an object of the old symbol's art.
            if other && !n.extras().symbol_overrides.is_empty() {
                n.extras_mut().symbol_overrides.clear();
            }
        }
        fit_overrides(d, &ids);
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

/// How far a transform coefficient may be from Reset Transformation's for the instance to count as
/// already reset.
const RESET_TOLERANCE: f64 = 1e-9;

/// The transform Reset Transformation gives instance `n` of the document of `boxes`: its symbol's
/// own scale, without rotation, shear or reflection, with the center of its art's bounding box
/// where it is. `None` when [`reset_transformation_changes`] is false for `n`.
fn reset_transformation(boxes: &vectorcraft_doc::SymbolBoxes, n: &Node) -> Option<Affine> {
    let d = boxes.document();
    if !reset_transformation_changes(d, n) {
        return None;
    }
    let NodeKind::SymbolInstance { symbol, xf } = &n.kind else { return None };
    let scale = d.symbol_natural_xf(symbol);
    // The art's box in symbol space (the ±HALF square when the art cannot be measured): its
    // center keeps its place on the page.
    let art = boxes.art_bounds(symbol).unwrap_or_else(|| vectorcraft_geom::Rect::new(-HALF, -HALF, HALF, HALF));
    let center = xf.transform_rect_bbox(art).center();
    Some(Affine::translate(center - scale * art.center()) * scale)
}

/// Does Reset Transformation change `n`? True for an instance of a symbol `d` holds, not in
/// perspective, whose scale, rotation, shear or reflection differs from the symbol's own scale
/// (its stored natural size, or the art unscaled when that size is not positive and finite).
/// It does not measure the art.
pub fn reset_transformation_changes(d: &Document, n: &Node) -> bool {
    let NodeKind::SymbolInstance { symbol, xf } = &n.kind else { return false };
    if n.extras().perspective.is_some() || !d.symbols.iter().any(|s| s.name == *symbol) {
        return false;
    }
    let natural = d.symbol_natural_xf(symbol).as_coeffs();
    xf.as_coeffs()[..4].iter().zip(&natural[..4]).any(|(a, b)| (a - b).abs() > RESET_TOLERANCE)
}

pub(super) fn symbol_instance_reset_transform(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_instances(s, p)?;
    if ids.is_empty() {
        return Err(EngineError::Other("no symbol instances selected".into()));
    }
    let d = &s.doc()?.doc;
    let boxes = d.symbol_boxes();
    let changes: Vec<(NodeId, Affine)> = ids.iter().filter_map(|id| Some((*id, reset_transformation(&boxes, d.node(*id)?)?))).collect();
    if !changes.is_empty() {
        s.edit("Reset Transformation", |d, _| {
            for (id, new) in &changes {
                if let Some(NodeKind::SymbolInstance { xf, .. }) = d.node_mut(*id).map(|n| &mut n.kind) {
                    *xf = *new;
                }
            }
            Ok(())
        })?;
    }
    let changed: Vec<NodeId> = changes.iter().map(|(id, _)| *id).collect();
    Ok(json!({ "ids": ids_json(&changed) }))
}

pub(super) fn symbol_set_current(s: &mut Session, p: &Value) -> Result<Value> {
    let name = resolve_symbol(&s.doc()?.doc, p, "symbol.setCurrent")?;
    set_unknown(s, CURRENT_SYMBOL, Some(json!(name)))?;
    ok()
}

pub(super) fn symbol_select_instances(s: &mut Session, p: &Value) -> Result<Value> {
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

pub(super) fn symbol_spray(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.spray";
    let pts = points(p, C)?;
    let st = s.doc()?;
    let name = resolve_symbol(&st.doc, p, C)?;
    let radius = number_or(p, "radius", 40.0, C)?.clamp(1.0, 2000.0);
    let density = number_or(p, "density", 5.0, C)?.clamp(1.0, 10.0);
    let scale = number_or(p, "scale", 1.0, C)?.clamp(0.01, 100.0);
    let alt = switch_or(p, "alt", false, C)?;
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
            s.edit("Symbol Sprayer", |d, _| remove_each(d, &doomed))?;
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
    let reg = s.doc()?.doc.symbol_registration(&name);
    let id = s.edit("Symbol Sprayer", |d, sel| {
        let mut nodes = vec![];
        for (i, c) in placed.iter().enumerate() {
            let (_, _, js) = jitter(i + 7, *c);
            let k = scale * (1.0 + js * 0.1);
            let id = d.alloc_id();
            nodes.push(Node::new(id, NodeKind::SymbolInstance { symbol: name.clone(), xf: place_xf(*c, size, reg) * Affine::scale(k) }));
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

pub(super) fn symbol_adjust(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.adjust";
    let tool = str_param(p, "tool").ok_or_else(|| bad(C, "missing `tool`"))?.to_string();
    if !["shift", "scrunch", "size", "spin", "stain", "screen", "style"].contains(&tool.as_str()) {
        return Err(bad(C, format!("unknown tool `{tool}`")));
    }
    crate::cmd::paint::check_keep_model(p, C)?;
    let pts = points(p, C)?;
    let radius = number_or(p, "radius", 40.0, C)?.clamp(1.0, 2000.0);
    let intensity = number_or(p, "intensity", 5.0, C)?.clamp(1.0, 10.0) / 10.0;
    let alt = switch_or(p, "alt", false, C)?;
    // Refused while an object of a dynamic instance's art is selected: the instance stays as it is.
    crate::cmd::edit::refuse_symbol_part(s)?;
    let st = s.doc()?;
    let color = match color_param(p, "color", C)? {
        Some(c) => crate::cmd::paint::new_color(s, p, c),
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
        let editable = d.editable();
        v.retain(|id| editable(*id));
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
