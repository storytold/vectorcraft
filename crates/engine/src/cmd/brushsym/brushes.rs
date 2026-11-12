use super::{
    AppearanceItem, Arc, BTreeSet, Bristle, Brush, BrushKind, Color, Document, EngineError, Node, NodeId, NodeKind, Paint, Result, Session, Value,
    anchor, bad, brush, check_numbers, find_command, id_param, ids_json, json, name_param, names_param, ok, roots, selection_art, set_unknown,
    str_param,
};

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
        if !has_width(&art, &s.doc()?.doc.symbol_boxes()) {
            return Err(bad(C, "the selected art has no width"));
        }
        v[k] = serde_json::to_value(&art).unwrap_or_default();
    }
    lib.push(parse_brush(C, v)?);
    save_library(s, "New Brush", lib, &[])?;
    Ok(json!({ "name": name }))
}

/// Whether art `n` has width for a brush to be made of, with symbol instances measured by their
/// symbols' art in `symbols`.
fn has_width(n: &Node, symbols: &vectorcraft_doc::SymbolBoxes) -> bool {
    n.geometric_bounds_with(symbols).is_some_and(|b| b.width() > 1e-6)
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
/// designs, pattern tiles and the tiles that pattern editing's Cancel puts back, and graphic styles
/// count, and so do the strokes in the art of every brush that a counted stroke paints with.
pub(super) fn brush_unused(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let lib = brush::library(d);
    let names = brush_unused_in(d, &lib);
    Ok(json!({ "names": names }))
}

/// The brushes of `lib` no stroke in `d` paints with, in library order ([`brush_unused`]).
fn brush_unused_in(d: &Document, lib: &[Brush]) -> Vec<String> {
    let mut todo = vec![];
    let trees = d.layers.iter().chain(d.symbols.iter().map(|s| &s.art)).chain(d.graph_designs.iter().map(|g| &g.art));
    let patterns = d.patterns.iter().chain(d.pattern_edit.iter().filter_map(|pe| pe.original.as_ref()));
    trees.chain(patterns.flat_map(|p| &p.art)).for_each(|n| art_brushes(n, &mut todo));
    d.graphic_styles.iter().for_each(|g| stroke_brushes(&g.appearance.items, &mut todo));
    let mut used = BTreeSet::new();
    while let Some(name) = todo.pop() {
        if !used.contains(&name) {
            lib.iter().filter(|b| b.name == name).flat_map(Brush::art).for_each(|n| art_brushes(n, &mut todo));
            used.insert(name);
        }
    }
    lib.iter().map(|b| b.name.clone()).filter(|n| !used.contains(n)).collect()
}

pub(super) fn brush_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.options";
    let name = name_param(p, C)?.to_string();
    let mut lib = brush::library(&s.doc()?.doc);
    let i = lib.iter().position(|b| b.name == name).ok_or_else(|| bad(C, format!("no brush named `{name}`")))?;
    let mut v = serde_json::to_value(&lib[i]).unwrap_or_default();
    if let Some(params) = p.get("params") {
        let original = v.clone();
        merge(&mut v, params);
        if lib[i].kind.type_id() == "pattern" {
            let tiles = [
                ("side_tile", "side"),
                ("outer_corner_tile", "outer_corner"),
                ("inner_corner_tile", "inner_corner"),
                ("start_tile", "start"),
                ("end_tile", "end"),
            ];
            // Resolve all Original choices against the brush before any tile can clear the art
            // another corner borrows. Keep the matching frame with each copied tile.
            let originals: Vec<_> = tiles
                .iter()
                .map(|(_, art)| {
                    let from = if art.ends_with("corner")
                        && original.get(art).is_none_or(Value::is_null)
                        && original.get(if *art == "outer_corner" { "outer_source" } else { "inner_source" }).is_none_or(Value::is_null)
                    {
                        if *art == "outer_corner" { "inner_corner" } else { "outer_corner" }
                    } else {
                        *art
                    };
                    (original.get(from).cloned().unwrap_or(Value::Null), original.get(format!("{from}_frame")).cloned().unwrap_or(Value::Null))
                })
                .collect();
            for ((tile, art), (original_art, original_frame)) in tiles.into_iter().zip(originals) {
                let frame = format!("{art}_frame");
                let swatch = format!("{art}_swatch");
                let Some(choice) = params.get(tile) else {
                    if params.get(art).is_some() {
                        if params.get(frame.as_str()).is_none() {
                            v[frame.as_str()] = Value::Null;
                        }
                        if params.get(swatch.as_str()).is_none() {
                            v[swatch.as_str()] = Value::Null;
                        }
                    }
                    continue;
                };
                let (choice, swatch_only) = if let Some(name) = choice.as_object() {
                    (name.get("pattern").and_then(Value::as_str).ok_or_else(|| bad(C, "invalid pattern tile choice"))?, true)
                } else {
                    (choice.as_str().ok_or_else(|| bad(C, "invalid pattern tile choice"))?, false)
                };
                if art == "side" && choice == "none" && !swatch_only {
                    return Err(bad(C, "You must choose a side pattern."));
                }
                if choice == "original" && !swatch_only {
                    v[art] = original_art;
                    v[frame.as_str()] = original_frame;
                    v[swatch.as_str()] = Value::Null;
                } else if !swatch_only && (choice == "none" || (art.ends_with("corner") && brush::CornerSource::parse(choice).is_some())) {
                    v[art] = Value::Null;
                    v[frame.as_str()] = Value::Null;
                    v[swatch.as_str()] = Value::Null;
                    if art.ends_with("corner") {
                        v[if art == "outer_corner" { "outer_source" } else { "inner_source" }] = json!(choice);
                    }
                } else {
                    let pattern = s.doc()?.doc.pattern(choice).ok_or_else(|| bad(C, format!("no pattern named `{choice}`")))?;
                    let copy = Node::new(NodeId(0), NodeKind::Group { children: pattern.art.clone(), clip: false });
                    v[art] = serde_json::to_value(copy).unwrap_or_default();
                    v[frame.as_str()] = serde_json::to_value(pattern.tile).unwrap_or_default();
                    v[swatch.as_str()] = json!(choice);
                    if art.ends_with("corner") {
                        v[if art == "outer_corner" { "outer_source" } else { "inner_source" }] = Value::Null;
                    }
                }
            }
        }
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

/// The name `brush.placeArt` gives each pattern brush tile it places, by tile, in the order it
/// places them; `brush.replaceArt` reads a group of that name as that tile.
const TILE_NAMES: [&str; 5] = ["Side Tile", "Outer Corner Tile", "Inner Corner Tile", "Start Tile", "End Tile"];

/// The space between the pattern brush tiles `brush.placeArt` places, in points.
const TILE_GAP: f64 = 10.0;

/// The message for a brush that is not made of art.
fn no_art(name: &str) -> String {
    format!("`{name}` has no art (only scatter, art and pattern brushes do)")
}

/// Tile `i` of `p` ([`TILE_NAMES`] order): its art and its repeat frame, when it has art.
fn tile(p: &brush::PatternBrush, i: usize) -> Option<(&Node, Option<vectorcraft_geom::Rect>)> {
    match i {
        0 => Some((&p.side, p.side_frame)),
        1 => p.outer_corner.as_ref().map(|n| (n, p.outer_corner_frame)),
        2 => p.inner_corner.as_ref().map(|n| (n, p.inner_corner_frame)),
        3 => p.start.as_ref().map(|n| (n, p.start_frame)),
        _ => p.end.as_ref().map(|n| (n, p.end_frame)),
    }
}

/// Gives tile `i` of `p` ([`TILE_NAMES`] order) the art `art` and the repeat frame `frame`. The tile
/// no longer comes from a pattern swatch, and a corner draws its own art.
fn set_tile(p: &mut brush::PatternBrush, i: usize, art: Node, frame: Option<vectorcraft_geom::Rect>) {
    match i {
        0 => (p.side, p.side_frame, p.side_swatch) = (art, frame, None),
        1 => (p.outer_corner, p.outer_corner_frame, p.outer_corner_swatch, p.outer_source) = (Some(art), frame, None, None),
        2 => (p.inner_corner, p.inner_corner_frame, p.inner_corner_swatch, p.inner_source) = (Some(art), frame, None, None),
        3 => (p.start, p.start_frame, p.start_swatch) = (Some(art), frame, None),
        _ => (p.end, p.end_frame, p.end_swatch) = (Some(art), frame, None),
    }
}

/// The art `brush.placeArt` places for brush `b`, in the brush's own coordinates: a scatter or art
/// brush's art, or a pattern brush's tiles in a row, each in a group named for its tile
/// ([`TILE_NAMES`]), with an unpainted rectangle at the back of a tile that has a repeat frame
/// marking that frame. `None` for a brush that is not made of art. Symbol instances in the art are
/// measured by their symbols' art in `symbols`.
fn placed_art(b: &Brush, symbols: &vectorcraft_doc::SymbolBoxes) -> Option<Node> {
    match &b.kind {
        BrushKind::Scatter(sc) => Some(sc.art.clone()),
        BrushKind::Art(a) => Some(a.art.clone()),
        BrushKind::Pattern(pb) => {
            let mut tiles = vec![];
            let mut x = 0.0;
            for (i, tile_name) in TILE_NAMES.iter().enumerate() {
                let Some((art, frame)) = tile(pb, i) else { continue };
                let mut children = vec![];
                if let Some(f) = frame {
                    let marker = Node::path(
                        NodeId(0),
                        vectorcraft_geom::shapes::rectangle(f),
                        vectorcraft_doc::Appearance::basic(Paint::None, Paint::None, 1.0),
                    );
                    children.push(Arc::new(marker));
                }
                children.push(Arc::new(art.clone()));
                let mut g = Node::group(NodeId(0), children);
                g.name = Some((*tile_name).to_string());
                let Some(r) = g.geometric_bounds_with(symbols) else { continue };
                // Each tile sits right of the one before it, centered on the line y = 0.
                g.transform(vectorcraft_geom::Affine::translate((x - r.x0, -r.center().y)), false);
                x += r.width() + TILE_GAP;
                tiles.push(Arc::new(g));
            }
            Some(Node::group(NodeId(0), tiles))
        }
        BrushKind::Calligraphic(_) | BrushKind::Bristle(_) => None,
    }
}

/// `brush.placeArt`: a copy of the art brush `name` is made of, centered on (`x`, `y`) (default: the
/// center of the first artboard) in the layer new art goes into, and selected, as one undo step.
/// Scatter and art brushes place their art; pattern brushes place their tiles ([`placed_art`]).
pub(super) fn brush_place_art(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.placeArt";
    let name = name_param(p, C)?.to_string();
    let d = &s.doc()?.doc;
    let center = d.artboards.first().map(|a| a.rect.center()).unwrap_or(super::Point::ZERO);
    let at = super::Point::new(super::number_or(p, "x", center.x, C)?, super::number_or(p, "y", center.y, C)?);
    let b = brush::find(d, &name).ok_or_else(|| bad(C, format!("no brush named `{name}`")))?;
    let symbols = d.symbol_boxes();
    let mut art = placed_art(&b, &symbols).ok_or_else(|| bad(C, no_art(&name)))?;
    if let Some(r) = art.geometric_bounds_with(&symbols) {
        art.transform(vectorcraft_geom::Affine::translate(at - r.center()), false);
    }
    let parent = s.doc()?.target_parent()?;
    let id = s.edit("Place Brush Art", |d, sel| {
        let art = d.reid(&art);
        let id = art.id;
        d.insert(parent, usize::MAX, art)?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// The repeat frame an unpainted rectangle at the back of a tile group marks ([`placed_art`]): the
/// rectangle's bounds, when `n` is a path of one closed subpath through the four corners of its
/// bounds, without curves, whose fills and strokes paint nothing and that has no other appearance.
fn frame_marker(n: &Node) -> Option<vectorcraft_geom::Rect> {
    let NodeKind::Path { path, guide: false, clipping: false, .. } = &n.kind else { return None };
    let unpainted = n.appearance.effects.is_empty()
        && n.appearance.items.iter().all(|i| match i {
            AppearanceItem::Fill(f) => f.paint.is_none(),
            AppearanceItem::Stroke(st) => st.paint.is_none(),
        });
    let [sp] = path.subpaths.as_slice() else { return None };
    let r = n.geometric_bounds()?;
    let corners = [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)];
    let at_corners = sp.anchors.len() == 4
        && sp.anchors.iter().all(|a| a.h_in == a.p && a.h_out == a.p)
        && corners.iter().all(|c| sp.anchors.iter().any(|a| (a.p.x, a.p.y) == *c));
    (unpainted && sp.closed && at_corners && r.width() > 1e-9 && r.height() > 1e-9).then_some(r)
}

/// The tiles in `art` for a pattern brush, by tile ([`TILE_NAMES`] order), with their repeat frames:
/// a group named for a tile, as `brush.placeArt` places them, is the art of its tile (an unpainted
/// rectangle at its back marks the tile's frame, [`frame_marker`], and is left out of the art; a
/// group left with one member gives that member), and so is each member of a group whose members
/// are all such groups; any other art is the side tile.
fn tiles_of(art: Node) -> Vec<(usize, Node, Option<vectorcraft_geom::Rect>)> {
    let named = |c: &Node| matches!(c.kind, NodeKind::Group { clip: false, .. }) && c.name.as_deref().is_some_and(|n| TILE_NAMES.contains(&n));
    let tiles = match &art.kind {
        _ if named(&art) => vec![Arc::new(art)],
        NodeKind::Group { children, clip: false } if !children.is_empty() && children.iter().all(|c| named(c)) => children.clone(),
        _ => return vec![(0, art, None)],
    };
    let mut out: Vec<(usize, Node, Option<vectorcraft_geom::Rect>)> = vec![];
    for t in tiles {
        let Some(i) = t.name.as_deref().and_then(|n| TILE_NAMES.iter().position(|x| *x == n)) else { continue };
        let mut g = (*t).clone();
        let mut frame = None;
        if let NodeKind::Group { children, .. } = &mut g.kind
            && let Some(f) = children.first().and_then(|c| frame_marker(c))
        {
            frame = Some(f);
            children.remove(0);
        }
        g.name = None;
        // The group `brush.placeArt` put around the tile's art: the art itself goes back, so a
        // tile does not gain a group each time it goes out and back.
        if let NodeKind::Group { children, clip: false } = &g.kind
            && let [only] = children.as_slice()
            && g == Node::group(g.id, children.clone())
        {
            g = (**only).clone();
        }
        // A tile named twice: the frontmost one counts.
        out.retain(|(j, ..)| *j != i);
        out.push((i, g, frame));
    }
    out
}

/// `brush.replaceArt`: the selected art (or `ids`/`id`; several objects become one group, as in
/// `brush.new`) becomes the art of scatter, art or pattern brush `name`, keeping its other options,
/// as one undo step. A pattern brush takes the tiles `brush.placeArt` places by their names
/// ([`tiles_of`]) and other art as its side tile. When strokes use the brush and `applyToStrokes` is
/// false, the brush and its strokes stay as they were and a copy of the brush with the new art is
/// added after it and becomes the current brush; otherwise the strokes redraw with the new art.
pub(super) fn brush_replace_art(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "brush.replaceArt";
    let name = name_param(p, C)?.to_string();
    let apply = super::switch_or(p, "applyToStrokes", true, C)?;
    let mut lib = brush::library(&s.doc()?.doc);
    let i = lib.iter().position(|b| b.name == name).ok_or_else(|| bad(C, format!("no brush named `{name}`")))?;
    if lib[i].art().is_empty() {
        return Err(bad(C, no_art(&name)));
    }
    let art = selection_art(s, p)?.ok_or_else(|| bad(C, "nothing selected"))?;
    let d = &s.doc()?.doc;
    let symbols = d.symbol_boxes();
    let check = |n: &Node| if has_width(n, &symbols) { Ok(()) } else { Err(bad(C, "the selected art has no width")) };
    let mut b = lib[i].clone();
    match &mut b.kind {
        BrushKind::Scatter(sc) => {
            check(&art)?;
            sc.art = art;
        }
        BrushKind::Art(a) => {
            check(&art)?;
            a.art = art;
        }
        BrushKind::Pattern(pb) => {
            for (t, art, frame) in tiles_of(art) {
                check(&art)?;
                set_tile(pb, t, art, frame);
            }
        }
        BrushKind::Calligraphic(_) | BrushKind::Bristle(_) => return Err(bad(C, no_art(&name))),
    }
    let used = !brush_unused_in(d, &lib).contains(&name);
    if apply || !used {
        lib[i] = b;
        save_library(s, "Replace Brush Art", lib, &[])?;
        return Ok(json!({ "name": name }));
    }
    b.name = brush::unique_name(&lib, &format!("{} copy", name.trim()));
    let copy = b.name.clone();
    lib.insert(i + 1, b);
    s.edit("Replace Brush Art", |d, _| {
        d.unknown.insert(brush::CURRENT_KEY.into(), json!(copy));
        brush::store(d, &lib);
        Ok(())
    })?;
    Ok(json!({ "name": copy }))
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
    let r = crate::cmd::run_checked(s, spec, p)?;
    let (Some(name), Some(id)) = (name, id_param(&r, "id")) else { return Ok(r) };
    s.edit("Paintbrush", |d, _| {
        if let Some(st) = d.node_mut(id).and_then(|n| n.appearance.stroke_mut()) {
            st.brush = Some(name.clone());
        }
        Ok(())
    })?;
    Ok(r)
}
