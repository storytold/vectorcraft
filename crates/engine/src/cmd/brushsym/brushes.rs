use super::*;

// ---------- brushes ----------

/// Brushable nodes (paths and compound paths) under `ids`.
pub(crate) fn brush_targets(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
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

pub(super) fn brush_list(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let list: Vec<Value> = brush::library(d).iter().map(|b| json!({"name": b.name, "type": b.kind.type_id()})).collect();
    Ok(json!({ "brushes": list, "current": brush::current(d) }))
}

pub(super) fn brush_get(s: &mut Session, p: &Value) -> Result<Value> {
    let name = name_param(p, "brush.get")?;
    let b = brush::find(&s.doc()?.doc, name).ok_or_else(|| EngineError::Other(format!("no brush named `{name}`")))?;
    Ok(serde_json::to_value(b).unwrap_or_default())
}

/// The art `brush.new` takes from `ids` or `id`, else from the selection ([`selection_art`]).
pub(super) fn brush_selection_art(s: &mut Session, p: &Value) -> Result<Value> {
    let art = selection_art(s, p)?;
    Ok(json!({ "art": art.map(|n| serde_json::to_value(&n).unwrap_or_default()) }))
}

/// The stroke paint a brush gives paths without one: the current stroke colour, else black.
pub(crate) fn stroke_paint(s: &Session) -> Paint {
    if s.paint.stroke.is_none() { Paint::solid(Color::BLACK) } else { s.paint.stroke.clone() }
}

/// Paint paths `ids` of `d` with brush `name` (a path without a stroke takes `stroke`).
pub(crate) fn apply_brush(d: &mut Document, ids: &[NodeId], name: &str, stroke: &Paint) {
    for id in ids {
        let Some(n) = d.node_mut(*id) else { continue };
        if n.appearance.stroke().is_none_or(|st| st.paint.is_none()) {
            n.appearance.set_stroke(stroke.clone());
        }
        if let Some(st) = n.appearance.stroke_mut() {
            if st.width <= 0.0 {
                st.width = 1.0;
            }
            st.brush = Some(name.to_string());
        }
    }
}

pub(super) fn brush_apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.apply";
    let name = name_param(p, C)?.to_string();
    if brush::find(&s.doc()?.doc, &name).is_none() {
        return Err(bad(C, format!("no brush named `{name}`")));
    }
    let ids = brush_targets(&s.doc()?.doc, &roots(s, p)?);
    let stroke = stroke_paint(s);
    s.edit("Apply Brush", |d, _| {
        d.unknown.insert(brush::CURRENT_KEY.into(), json!(name));
        apply_brush(d, &ids, &name, &stroke);
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

pub(super) fn brush_remove(s: &mut Session, p: &Value) -> Result<Value> {
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

pub(super) fn brush_set_current(s: &mut Session, p: &Value) -> Result<Value> {
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

/// The definition fields that `params` change field by field when the brush has them as objects:
/// `{colorization: {method: "hueShift"}}` keeps the key color, and an art brush's `scale` with only
/// `start` keeps `end`.
const MERGED_FIELDS: [&str; 2] = ["colorization", "scale"];

fn merge(base: &mut Value, patch: &Value) {
    if let (Some(b), Some(pt)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in pt {
            match (b.get_mut(k).and_then(Value::as_object_mut), v.as_object()) {
                (Some(inner), Some(fields)) if MERGED_FIELDS.contains(&k.as_str()) => {
                    for (f, fv) in fields {
                        inner.insert(f.clone(), fv.clone());
                    }
                }
                _ => {
                    b.insert(k.clone(), v.clone());
                }
            }
        }
    }
}

/// Sets brush `to` on the strokes in `items` that paint with brush `from`.
fn set_stroke_brush(items: &mut [AppearanceItem], from: &str, to: Option<&str>) {
    for it in items {
        if let AppearanceItem::Stroke(st) = it
            && st.brush.as_deref() == Some(from)
        {
            st.brush = to.map(str::to_string);
        }
    }
}

/// Whether a stroke in `n` (its opacity masks included) paints with brush `name`.
fn paints_with(n: &Node, name: &str) -> bool {
    let mut hit = false;
    vectorcraft_doc::swatches::walk_with_masks(n, &mut |m| {
        hit |= m.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(st) if st.brush.as_deref() == Some(name)));
    });
    hit
}

/// Applies [`set_stroke_brush`] to every stroke in `n` and its opacity masks. Only the objects that
/// hold such a stroke, and those that hold them, are copied.
fn set_brush_in(n: &mut Node, from: &str, to: Option<&str>) {
    set_stroke_brush(&mut n.appearance.items, from, to);
    if let Some(mask) = n.extras_if_any().and_then(|e| e.mask.as_mut())
        && paints_with(&mask.art, from)
    {
        set_brush_in(Arc::make_mut(&mut mask.art), from, to);
    }
    for c in n.children_mut().into_iter().flatten() {
        if paints_with(c, from) {
            set_brush_in(Arc::make_mut(c), from, to);
        }
    }
}

/// Returns the art a brush is made of ([`Brush::art`]) for editing.
fn brush_art_mut(b: &mut Brush) -> Vec<&mut Node> {
    use vectorcraft_brush::BrushKind;
    match &mut b.kind {
        BrushKind::Scatter(s) => vec![&mut s.art],
        BrushKind::Art(a) => vec![&mut a.art],
        BrushKind::Pattern(p) => {
            std::iter::once(&mut p.side).chain([&mut p.outer_corner, &mut p.inner_corner, &mut p.start, &mut p.end].into_iter().flatten()).collect()
        }
        BrushKind::Calligraphic(_) | BrushKind::Bristle(_) => vec![],
    }
}

/// Sets brush `to` (`None`: no brush) on every stroke that paints with brush `from`: in the art
/// (opacity masks included), symbols, graph designs, pattern tiles and the tiles that pattern
/// editing's Cancel puts back, graphic styles and the art of the brushes in `lib`.
fn rename_brush_strokes(d: &mut Document, lib: &mut [Brush], from: &str, to: Option<&str>) {
    let trees = d.layers.iter_mut().chain(d.symbols.iter_mut().map(|s| &mut s.art)).chain(d.graph_designs.iter_mut().map(|g| &mut g.art));
    let patterns = d.patterns.iter_mut().chain(d.pattern_edit.iter_mut().filter_map(|pe| pe.original.as_mut()));
    for n in trees.chain(patterns.flat_map(|p| &mut p.art)) {
        if paints_with(n, from) {
            set_brush_in(Arc::make_mut(n), from, to);
        }
    }
    for g in &mut d.graphic_styles {
        set_stroke_brush(&mut g.appearance.items, from, to);
    }
    for n in lib.iter_mut().flat_map(brush_art_mut) {
        set_brush_in(n, from, to);
    }
}

fn save_library(s: &mut Session, label: &str, mut lib: Vec<Brush>, renames: &[(String, Option<String>)]) -> Result<()> {
    s.edit(label, |d, _| {
        for (from, to) in renames {
            rename_brush_strokes(d, &mut lib, from, to.as_deref());
            if brush::current(d).as_deref() == Some(from.as_str()) {
                match to {
                    Some(t) => d.unknown.insert(brush::CURRENT_KEY.into(), json!(t)),
                    None => d.unknown.remove(brush::CURRENT_KEY),
                };
            }
        }
        brush::store(d, &lib);
        Ok(())
    })
}

pub(super) fn brush_new(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.new";
    let ty = str_param(p, "type").ok_or_else(|| bad(C, "missing `type`"))?;
    let art_key = match ty {
        "calligraphic" | "bristle" => None,
        "scatter" | "art" => Some("art"),
        "pattern" => Some("side"),
        o => return Err(bad(C, format!("unknown brush type `{o}`"))),
    };
    let mut lib = brush::library(&s.doc()?.doc);
    let given = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty());
    let base_name = given.map_or_else(|| format!("New {} Brush", ty[..1].to_uppercase() + &ty[1..]), str::to_string);
    let name = brush::unique_name(&lib, &base_name);
    if ty == "bristle" {
        bristle_name_fits(C, &name)?;
    }
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
    save_library(s, "New Brush", lib, &[])?;
    Ok(json!({ "name": name }))
}

/// Refuses `name` for a bristle brush when it has more than [`Bristle::MAX_NAME_CHARS`] characters.
fn bristle_name_fits(cmd: &str, name: &str) -> Result<()> {
    let (count, max) = (name.chars().count(), Bristle::MAX_NAME_CHARS);
    if count > max {
        return Err(bad(cmd, format!("a bristle brush name can have at most {max} characters: `{name}` has {count}")));
    }
    Ok(())
}

/// A brush definition from command params, its values kept within their ranges ([`Brush::from_json`]).
fn parse_brush(cmd: &str, v: Value) -> Result<Brush> {
    Brush::from_json(v).map_err(|e| bad(cmd, format!("invalid brush: {e}")))
}

pub(super) fn brush_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.delete";
    let names = names_param(p, C)?;
    let mut lib = brush::library(&s.doc()?.doc);
    if let Some(name) = names.iter().find(|n| !lib.iter().any(|b| &b.name == *n)) {
        return Err(bad(C, format!("no brush named `{name}`")));
    }
    lib.retain(|b| !names.contains(&b.name));
    let renames: Vec<(String, Option<String>)> = names.into_iter().map(|n| (n, None)).collect();
    save_library(s, "Delete Brush", lib, &renames)?;
    ok()
}

/// Moves the brushes in `name` or `names`, in that order, to index `to` among the brushes of their
/// type in library order: before the first brush of that type at or after `to` that isn't moving,
/// else after the last brush of that type that isn't moving. Brushes of different types, or a `type`
/// that isn't theirs, are an error. A move that leaves the order as it was records no undo step.
pub(super) fn brush_move(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.move";
    const WITHIN: &str = "brushes move only within their type";
    let names = names_param(p, C)?;
    check_numbers(p, &["to"], C)?;
    let to = match p.get("to").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_u64().map(|i| usize::try_from(i).unwrap_or(usize::MAX)).ok_or_else(|| bad(C, "`to` must be a whole number"))?),
        None => None,
    };
    let old = brush::library(&s.doc()?.doc);
    let mut lib = old.clone();
    // `names_param` refuses an empty list, so the loop sets `kind`.
    let mut kind = "";
    for name in &names {
        let b = lib.iter().find(|b| &b.name == name).ok_or_else(|| bad(C, format!("no brush named `{name}`")))?;
        if !kind.is_empty() && kind != b.kind.type_id() {
            return Err(bad(C, WITHIN));
        }
        kind = b.kind.type_id();
    }
    if p.get("type").filter(|v| !v.is_null()).is_some_and(|t| t.as_str() != Some(kind)) {
        return Err(bad(C, WITHIN));
    }
    let same: Vec<&Brush> = lib.iter().filter(|b| b.kind.type_id() == kind).collect();
    let before = anchor(&same, to, &names, |b| &b.name);
    let first = lib.iter().position(|b| names.contains(&b.name)).unwrap_or(lib.len());
    let mut items = vec![];
    for name in &names {
        if let Some(i) = lib.iter().position(|b| &b.name == name) {
            items.push(lib.remove(i));
        }
    }
    let at = match before {
        Some(a) => lib.iter().position(|b| b.name == a),
        None => lib.iter().rposition(|b| b.kind.type_id() == kind).map(|i| i + 1),
    };
    let at = at.unwrap_or(first).min(lib.len());
    lib.splice(at..at, items);
    if lib != old {
        save_library(s, "Move Brush", lib, &[])?;
    }
    Ok(json!({ "moved": names.len() }))
}

/// Adds a copy of each brush in `name` or `names` at the end of the library. `newName`, given
/// without `names`, names the copy.
pub(super) fn brush_duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.duplicate";
    let names = names_param(p, C)?;
    let before = brush::library(&s.doc()?.doc);
    let mut lib = before.clone();
    let mut out = vec![];
    for name in &names {
        let mut b = before.iter().find(|b| &b.name == name).cloned().ok_or_else(|| bad(C, format!("no brush named `{name}`")))?;
        let given = str_param(p, "newName").map(str::trim).filter(|n| !n.is_empty() && p.get("names").is_none());
        let new_name = given.map_or_else(|| format!("{} copy", name.trim()), str::to_string);
        b.name = brush::unique_name(&lib, &new_name);
        out.push(b.name.clone());
        lib.push(b);
    }
    save_library(s, "Duplicate Brush", lib, &[])?;
    Ok(if p.get("names").is_some() { json!({ "names": out }) } else { json!({ "name": out.first() }) })
}

/// Adds the names of the brushes the strokes of `items` paint with to `out`.
fn stroke_brushes(items: &[AppearanceItem], out: &mut Vec<String>) {
    for it in items {
        if let AppearanceItem::Stroke(st) = it
            && let Some(b) = &st.brush
        {
            out.push(b.clone());
        }
    }
}

/// Adds the names of the brushes the strokes in `n` (its opacity masks included) paint with to
/// `out`.
fn art_brushes(n: &Node, out: &mut Vec<String>) {
    vectorcraft_doc::swatches::walk_with_masks(n, &mut |m| stroke_brushes(&m.appearance.items, out));
}

/// The brushes no stroke paints with. Strokes in the art (opacity masks included), symbols, graph
/// designs, pattern tiles and graphic styles count, and so do the strokes in the art of every brush
/// that a counted stroke paints with.
pub(super) fn brush_unused(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let lib = brush::library(d);
    let mut todo = vec![];
    let trees = d.layers.iter().chain(d.symbols.iter().map(|s| &s.art)).chain(d.graph_designs.iter().map(|g| &g.art));
    trees.chain(d.patterns.iter().flat_map(|p| &p.art)).for_each(|n| art_brushes(n, &mut todo));
    d.graphic_styles.iter().for_each(|g| stroke_brushes(&g.appearance.items, &mut todo));
    let mut used = BTreeSet::new();
    while let Some(name) = todo.pop() {
        if !used.contains(&name) {
            lib.iter().filter(|b| b.name == name).flat_map(Brush::art).for_each(|n| art_brushes(n, &mut todo));
            used.insert(name);
        }
    }
    let names: Vec<&str> = lib.iter().map(|b| b.name.as_str()).filter(|n| !used.contains(*n)).collect();
    Ok(json!({ "names": names }))
}

pub(super) fn brush_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.options";
    let name = name_param(p, C)?.to_string();
    let mut lib = brush::library(&s.doc()?.doc);
    let i = lib.iter().position(|b| b.name == name).ok_or_else(|| bad(C, format!("no brush named `{name}`")))?;
    let mut v = serde_json::to_value(&lib[i]).unwrap_or_default();
    if let Some(params) = p.get("params") {
        merge(&mut v, params);
    }
    let new_name = str_param(p, "newName").map(str::trim).filter(|n| !n.is_empty() && *n != name).map(str::to_string);
    if let Some(nn) = &new_name {
        if lib.iter().any(|b| &b.name == nn) {
            return Err(bad(C, format!("a brush named `{nn}` exists")));
        }
        // A `newName` that only trims the white space around the saved name is taken at any length.
        if matches!(lib[i].kind, BrushKind::Bristle(_)) && nn != name.trim() {
            bristle_name_fits(C, nn)?;
        }
        v["name"] = json!(nn);
    } else {
        v["name"] = json!(name);
    }
    v["type"] = json!(lib[i].kind.type_id());
    lib[i] = parse_brush(C, v)?;
    let out = lib[i].name.clone();
    let rename = new_name.map(|nn| (name, Some(nn)));
    save_library(s, "Brush Options", lib, rename.as_slice())?;
    Ok(json!({ "name": out }))
}

pub(super) fn expand_brush(s: &mut Session, p: &Value) -> Result<Value> {
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

pub(super) fn brush_freehand(s: &mut Session, p: &Value) -> Result<Value> {
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
