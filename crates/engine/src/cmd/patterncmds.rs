//! Pattern swatches (Object → Pattern, Pattern Options panel) and Repeat (Object → Repeat).
//!
//! **Pattern editing mode** works like isolation: `object.pattern.make` / `object.pattern.edit`
//! put a copy of the tile art into a temporary top layer (`Document::pattern_edit` records it),
//! which the renderer draws alone with dimmed copies around it; hit testing only reaches that
//! layer. `object.pattern.done` stores the layer's art as the pattern definition,
//! `object.pattern.cancel` restores the previous definition. Both remove the layer.
//!
//! **Repeat** objects are live nodes (`NodeKind::Repeat`) holding the source art and the
//! arrangement; `vectorcraft_doc::pattern` evaluates the instances.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::pattern::{self, Overlap, PATTERN_EDIT_LAYER, PatternDef, PatternEdit, RepeatKind, RepeatSpec, TileType, pattern_paint};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, Selection};
use vectorcraft_geom::Rect;

use super::edit::selected_roots;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        // ---------- Pattern ----------
        cmd!(
            "object.pattern.make",
            "Make",
            ["Object", "Pattern"],
            None,
            "{ids?, name?, tileType?: grid|brickByRow|brickByColumn|hexByColumn|hexByRow, brickOffset?: 0..1 (0.5), width?, height?, edit?: bool (true)} make a pattern swatch from the selection and enter pattern editing mode → {name}",
            has_selection,
            pattern_make
        ),
        cmd!(
            "object.pattern.edit",
            "Edit Pattern",
            ["Object", "Pattern"],
            Some("Cmd+Shift+F8"),
            "{name?} edit a pattern (default: the selected object's fill pattern) in pattern editing mode",
            can_edit_pattern,
            pattern_edit
        ),
        cmd!(
            "object.pattern.done",
            "Done",
            [],
            None,
            "{} leave pattern editing mode, saving the tile art into the pattern",
            in_pattern_edit,
            pattern_done
        ),
        cmd!("object.pattern.cancel", "Cancel", [], None, "{} leave pattern editing mode, discarding changes", in_pattern_edit, pattern_cancel),
        cmd!(
            "object.pattern.saveCopy",
            "Save a Copy",
            [],
            None,
            "{name?} while editing, save the current tile as a new pattern swatch → {name}",
            in_pattern_edit,
            pattern_save_copy
        ),
        cmd!(
            "pattern.options",
            "Pattern Options",
            ["Window", "Pattern Options"],
            None,
            "{name? (default: the pattern being edited), newName?, tileType?, brickOffset?, width?, height?, hSpacing?, vSpacing?, sizeTileToArt?: bool, overlap?: {h: left|right, v: top|bottom}, copies?: 3|5|7|9, dimCopies?: %, showTileEdge?: bool}",
            has_doc,
            pattern_options
        ),
        cmd!(
            "pattern.delete",
            "Delete Pattern",
            [],
            None,
            "{name} delete a pattern swatch (objects painted with it lose that paint)",
            has_doc,
            pattern_delete
        ),
        cmd!(query "pattern.list", "List Patterns", [], None, "{} → {patterns: [{name, tileType, width, height, ...}], editing?}", has_doc, pattern_list),
        cmd!(
            "pattern.transform",
            "Transform Patterns",
            [],
            None,
            "{ids?, matrix: [a,b,c,d,e,f]} transform only the pattern placement of the objects' pattern paints (Transform Patterns)",
            has_selection_or_ids,
            pattern_transform
        ),
        // ---------- Repeat ----------
        cmd!(
            "object.repeat.radial",
            "Radial",
            ["Object", "Repeat"],
            None,
            "{ids?, instances?: n (8), radius?: pt} radial repeat of the selection (or switch the selected repeat to radial) → {id}",
            has_selection,
            repeat_radial
        ),
        cmd!(
            "object.repeat.grid",
            "Grid",
            ["Object", "Repeat"],
            None,
            "{ids?, hSpacing?: pt, vSpacing?: pt (¼ of the art size), rows?: n (3), cols?: n (3)} grid repeat of the selection → {id}",
            has_selection,
            repeat_grid
        ),
        cmd!(
            "object.repeat.mirror",
            "Mirror",
            ["Object", "Repeat"],
            None,
            "{ids?, angle?: deg (90 = vertical axis), offset?: pt (10)} mirror repeat of the selection → {id}",
            has_selection,
            repeat_mirror
        ),
        cmd!(
            "object.repeat.release",
            "Release",
            ["Object", "Repeat"],
            None,
            "{} release the selected repeats back to their source art → {ids}",
            has_repeat,
            repeat_release
        ),
        cmd!(
            "object.repeat.options",
            "Options…",
            ["Object", "Repeat"],
            None,
            "{instances?, radius?, reverseOverlap?, startAngle?, endAngle?, center?: [x,y] | hSpacing?, vSpacing?, rows?, cols?, gridType?, flipRows?, flipCols? | angle?} set the options of the selected repeats",
            has_repeat,
            repeat_options
        ),
        cmd!(
            "object.repeat.expand",
            "Expand Repeat",
            [],
            None,
            "{} replace the selected repeats by groups of their instances → {ids}",
            has_repeat,
            repeat_expand
        ),
    ]
}

// ---------- predicates ----------

fn editing(s: &Session) -> Option<&PatternEdit> {
    s.active()?.doc.pattern_edit.as_ref()
}

fn in_pattern_edit(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    editing(s).map(|_| ()).ok_or_else(|| "not in pattern editing mode".into())
}

fn can_edit_pattern(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if editing(s).is_some() {
        return Err("already editing a pattern".into());
    }
    if s.active().is_some_and(|st| st.doc.patterns.is_empty()) { Err("the document has no patterns".into()) } else { Ok(()) }
}

fn has_selection_or_ids(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)
}

fn selected_repeats(s: &Session) -> Vec<NodeId> {
    let Some(st) = s.active() else { return vec![] };
    let mut out = vec![];
    for id in &st.selection.objects {
        for a in st.doc.ancestry(*id).unwrap_or_default().into_iter().rev() {
            if st.doc.node(a).is_some_and(pattern::is_repeat) {
                if !out.contains(&a) {
                    out.push(a);
                }
                break;
            }
        }
    }
    out
}

fn has_repeat(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    if selected_repeats(s).is_empty() { Err("select a repeat".into()) } else { Ok(()) }
}

// ---------- helpers ----------

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

fn roots(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    match ids_param(p, "ids") {
        Some(ids) => {
            let st = s.doc()?;
            let mut sel = Selection::default();
            sel.set(ids.iter().copied().filter(|id| st.doc.node(*id).is_some_and(|n| !n.is_layer())));
            Ok(sel.in_paint_order(&st.doc))
        }
        None => selected_roots(s),
    }
}

fn unique_pattern_name(d: &Document, base: &str) -> String {
    if d.pattern(base).is_none() && d.swatch(base).is_none() {
        return base.to_string();
    }
    free_name(2, |i| format!("{base} {i}"), |n| d.pattern(n).is_none() && d.swatch(n).is_none())
}

fn next_pattern_name(d: &Document) -> String {
    free_name(1, |i| format!("New Pattern {i}"), |n| d.pattern(n).is_none() && d.swatch(n).is_none())
}

/// The first `name(i)`, counting from `from`, that `free` accepts.
fn free_name(from: u64, name: impl Fn(u64) -> String, free: impl Fn(&str) -> bool) -> String {
    let mut i = from;
    loop {
        let n = name(i);
        if free(&n) {
            return n;
        }
        i = i.saturating_add(1);
    }
}

/// Add pattern `def` and its swatch.
pub(crate) fn add_pattern(d: &mut Document, def: PatternDef) {
    d.swatches.push(vectorcraft_color::Swatch { name: def.name.clone(), paint: pattern_paint(&def.name), global: false, spot: false });
    d.patterns.push(def);
}

/// Fresh-id deep copies of `nodes`.
fn copies(d: &mut Document, nodes: &[Arc<Node>]) -> Vec<Node> {
    nodes.iter().map(|n| d.reid(n)).collect()
}

/// Enter pattern editing mode for `name`: a top layer with copies of the tile art.
fn enter_edit(d: &mut Document, sel: &mut Selection, name: &str, original: Option<PatternDef>) -> Result<NodeId> {
    let art = d.pattern(name).map(|p| p.art.clone()).ok_or_else(|| EngineError::Other(format!("no pattern `{name}`")))?;
    let layer = d.add_layer(Some(PATTERN_EDIT_LAYER));
    let nodes = copies(d, &art);
    let mut ids = vec![];
    for n in nodes {
        ids.push(n.id);
        d.insert(Some(layer), usize::MAX, n)?;
    }
    d.pattern_edit = Some(PatternEdit { pattern: name.to_string(), layer, original });
    sel.set(ids);
    Ok(layer)
}

/// Leave editing mode: drop the layer, return the edit record.
fn leave_edit(d: &mut Document, sel: &mut Selection) -> Result<(PatternEdit, Vec<Arc<Node>>)> {
    let pe = d.pattern_edit.take().ok_or_else(|| EngineError::Other("not in pattern editing mode".into()))?;
    let art = match d.remove(pe.layer) {
        Ok(l) => l.children().cloned().unwrap_or_default(),
        Err(_) => vec![],
    };
    sel.clear();
    Ok((pe, art))
}

fn after_edit_mode(s: &mut Session, layer: Option<NodeId>) -> Result<()> {
    let st = s.doc_mut()?;
    st.isolation = layer;
    st.active_layer = layer.or_else(|| st.doc.default_layer());
    st.revision += 1;
    Ok(())
}

fn tile_type_param(p: &Value, cmd: &str, current: TileType) -> Result<TileType> {
    let off = p.get("brickOffset").and_then(Value::as_f64).map(|o| if o > 1.0 { o / 100.0 } else { o });
    match str_param(p, "tileType") {
        Some(t) => TileType::parse(t, off).ok_or_else(|| bad(cmd, format!("unknown tileType `{t}` ({})", TileType::IDS.join("|")))),
        None => Ok(match (current, off) {
            (TileType::BrickByRow { .. }, Some(o)) => TileType::BrickByRow { offset: o },
            (TileType::BrickByColumn { .. }, Some(o)) => TileType::BrickByColumn { offset: o },
            (c, _) => c,
        }),
    }
}

// ---------- Pattern commands ----------

fn pattern_make(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.pattern.make";
    if editing(s).is_some() {
        return Err(bad(C, "already in pattern editing mode"));
    }
    let roots = roots(s, p)?;
    if roots.is_empty() {
        return Err(bad(C, "select the art for the pattern tile"));
    }
    let tt = tile_type_param(p, C, TileType::Grid)?;
    let (w, h) = (p.get("width").and_then(Value::as_f64), p.get("height").and_then(Value::as_f64));
    if w.is_some_and(|v| !(v.is_finite() && v > 0.0)) || h.is_some_and(|v| !(v.is_finite() && v > 0.0)) {
        return Err(bad(C, "width/height must be > 0"));
    }
    let edit = bool_or(p, "edit", true);
    let wanted = str_param(p, "name").map(str::to_string);
    let name = s.edit("Make Pattern", |d, sel| {
        let name = match &wanted {
            Some(n) => unique_pattern_name(d, n),
            None => next_pattern_name(d),
        };
        let src: Vec<Arc<Node>> = roots.iter().filter_map(|id| d.node(*id).cloned()).map(Arc::new).collect();
        let art: Vec<Arc<Node>> = copies(d, &src).into_iter().map(Arc::new).collect();
        let mut def = PatternDef::new(&name, art);
        def.tile_type = tt;
        let c = def.tile.center();
        def.tile = Rect::from_center_size(c, (w.unwrap_or(def.tile.width()).max(1e-3), h.unwrap_or(def.tile.height()).max(1e-3)));
        add_pattern(d, def);
        if edit {
            enter_edit(d, sel, &name, None)?;
        }
        Ok(name)
    })?;
    if edit {
        let layer = editing(s).map(|e| e.layer);
        after_edit_mode(s, layer)?;
    }
    Ok(json!({ "name": name }))
}

fn pattern_edit(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.pattern.edit";
    let name = match str_param(p, "name") {
        Some(n) => n.to_string(),
        None => {
            // The selected object's fill (or stroke) pattern.
            let st = s.doc()?;
            st.selection
                .objects
                .iter()
                .filter_map(|id| st.doc.node(*id))
                .find_map(|n| {
                    n.appearance.items.iter().find_map(|it| match it {
                        vectorcraft_doc::AppearanceItem::Fill(f) => pattern_name(&f.paint),
                        vectorcraft_doc::AppearanceItem::Stroke(sk) => pattern_name(&sk.paint),
                    })
                })
                .ok_or_else(|| bad(C, "give a pattern name or select an object painted with a pattern"))?
        }
    };
    let orig = s.doc()?.doc.pattern(&name).cloned().ok_or_else(|| bad(C, format!("no pattern `{name}`")))?;
    let layer = s.edit("Edit Pattern", |d, sel| enter_edit(d, sel, &name, Some(orig)))?;
    after_edit_mode(s, Some(layer))?;
    Ok(json!({ "name": name }))
}

fn pattern_name(p: &Paint) -> Option<String> {
    match p {
        Paint::Pattern { pattern, .. } => Some(pattern.clone()),
        _ => None,
    }
}

fn pattern_done(s: &mut Session, _: &Value) -> Result<Value> {
    let name = s.edit("Done Editing Pattern", |d, sel| {
        let (pe, art) = leave_edit(d, sel)?;
        if let Some(def) = d.pattern_mut(&pe.pattern) {
            def.art = art;
            if def.size_tile_to_art {
                def.fit_tile_to_art();
            }
        }
        Ok(pe.pattern)
    })?;
    after_edit_mode(s, None)?;
    Ok(json!({ "name": name }))
}

fn pattern_cancel(s: &mut Session, _: &Value) -> Result<Value> {
    s.edit("Cancel Pattern Editing", |d, sel| {
        let (pe, _) = leave_edit(d, sel)?;
        match pe.original {
            Some(orig) => {
                if let Some(def) = d.pattern_mut(&pe.pattern) {
                    *def = orig;
                }
            }
            None => {
                d.patterns.retain(|p| p.name != pe.pattern);
                d.swatches.retain(|sw| !matches!(&sw.paint, Paint::Pattern { pattern, .. } if *pattern == pe.pattern));
            }
        }
        Ok(())
    })?;
    after_edit_mode(s, None)?;
    ok()
}

fn pattern_save_copy(s: &mut Session, p: &Value) -> Result<Value> {
    let wanted = str_param(p, "name").map(str::to_string);
    let name = s.edit("Save a Copy", |d, _| {
        let pe = d.pattern_edit.clone().ok_or_else(|| EngineError::Other("not in pattern editing mode".into()))?;
        let mut def = d.pattern(&pe.pattern).cloned().ok_or_else(|| EngineError::Other("pattern missing".into()))?;
        let art: Vec<Arc<Node>> = d.node(pe.layer).and_then(|l| l.children().cloned()).unwrap_or_default();
        let name = unique_pattern_name(d, wanted.as_deref().unwrap_or(&format!("{} copy", pe.pattern)));
        def.name = name.clone();
        def.art = copies(d, &art).into_iter().map(Arc::new).collect();
        if def.size_tile_to_art {
            def.fit_tile_to_art();
        }
        add_pattern(d, def);
        Ok(name)
    })?;
    Ok(json!({ "name": name }))
}

fn pattern_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "pattern.options";
    let name = match str_param(p, "name") {
        Some(n) => n.to_string(),
        None => editing(s).map(|e| e.pattern.clone()).ok_or_else(|| bad(C, "give `name` (or edit a pattern)"))?,
    };
    let cur = s.doc()?.doc.pattern(&name).cloned().ok_or_else(|| bad(C, format!("no pattern `{name}`")))?;
    let tt = tile_type_param(p, C, cur.tile_type)?;
    let num = |k: &str| p.get(k).and_then(Value::as_f64);
    for k in ["width", "height"] {
        if num(k).is_some_and(|v| !(v.is_finite() && v > 0.0)) {
            return Err(bad(C, format!("{k} must be > 0")));
        }
    }
    let new_name = str_param(p, "newName").map(str::to_string).filter(|n| !n.is_empty() && *n != name);
    if let Some(n) = &new_name
        && s.doc()?.doc.pattern(n).is_some()
    {
        return Err(bad(C, format!("a pattern named `{n}` exists")));
    }
    let overlap = match p.get("overlap") {
        Some(o) => Overlap {
            right_in_front: str_param(o, "h").map_or(cur.overlap.right_in_front, |v| v == "right"),
            bottom_in_front: str_param(o, "v").map_or(cur.overlap.bottom_in_front, |v| v == "bottom"),
        },
        None => cur.overlap,
    };
    let edit_layer = editing(s).filter(|e| e.pattern == name).map(|e| e.layer);
    let out_name = new_name.clone().unwrap_or(name.clone());
    s.edit("Pattern Options", |d, _| {
        // While editing, the tile art is the edit layer's.
        let live_art: Option<Vec<Arc<Node>>> = edit_layer.and_then(|l| d.node(l)).and_then(|l| l.children().cloned());
        let def = d.pattern_mut(&name).ok_or_else(|| EngineError::Other("pattern missing".into()))?;
        def.tile_type = tt;
        def.overlap = overlap;
        if let Some(v) = num("hSpacing") {
            def.h_spacing = v;
        }
        if let Some(v) = num("vSpacing") {
            def.v_spacing = v;
        }
        if let Some(v) = p.get("sizeTileToArt").and_then(Value::as_bool) {
            def.size_tile_to_art = v;
        }
        if let Some(v) = num("copies") {
            def.copies = (v as u32).clamp(1, 15) | 1;
        }
        if let Some(v) = num("dimCopies") {
            def.dim_copies = v.clamp(0.0, 100.0) as f32;
        }
        if let Some(v) = p.get("showTileEdge").and_then(Value::as_bool) {
            def.show_tile_edge = v;
        }
        if def.size_tile_to_art {
            let saved = std::mem::take(&mut def.art);
            def.art = live_art.clone().unwrap_or_else(|| saved.clone());
            def.fit_tile_to_art();
            def.art = saved;
        } else if num("width").is_some() || num("height").is_some() {
            let c = def.tile.center();
            def.tile = Rect::from_center_size(c, (num("width").unwrap_or(def.tile.width()), num("height").unwrap_or(def.tile.height())));
        }
        if let Some(nn) = &new_name {
            def.name = nn.clone();
            rename_pattern_uses(d, &name, nn);
        }
        Ok(())
    })?;
    Ok(json!({ "name": out_name }))
}

/// Rename a pattern in swatches, paints and the edit record.
fn rename_pattern_uses(d: &mut Document, old: &str, new: &str) {
    let fix = |p: &mut Paint| {
        if let Paint::Pattern { pattern, .. } = p
            && pattern == old
        {
            *pattern = new.to_string();
        }
    };
    for sw in d.swatches.iter_mut().chain(d.swatch_groups.iter_mut().flat_map(|g| g.swatches.iter_mut())) {
        if sw.name == old {
            sw.name = new.to_string();
        }
        fix(&mut sw.paint);
    }
    if let Some(pe) = &mut d.pattern_edit
        && pe.pattern == old
    {
        pe.pattern = new.to_string();
    }
    let ids = nodes_using(d, Some(old));
    for id in ids {
        if let Some(n) = d.node_mut(id) {
            for it in n.appearance.items.iter_mut() {
                match it {
                    vectorcraft_doc::AppearanceItem::Fill(f) => fix(&mut f.paint),
                    vectorcraft_doc::AppearanceItem::Stroke(s) => fix(&mut s.paint),
                }
            }
        }
    }
}

fn nodes_using(d: &Document, name: Option<&str>) -> Vec<NodeId> {
    let mut ids = vec![];
    d.walk(|n| {
        if pattern::uses_pattern(n, name) {
            ids.push(n.id);
        }
    });
    ids
}

fn pattern_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "pattern.delete";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing name"))?.to_string();
    if s.doc()?.doc.pattern(&name).is_none() {
        return Err(bad(C, format!("no pattern `{name}`")));
    }
    if editing(s).is_some_and(|e| e.pattern == name) {
        return Err(bad(C, "finish editing the pattern first"));
    }
    let n = s.edit("Delete Pattern", |d, _| {
        d.patterns.retain(|p| p.name != name);
        let is_it = |sw: &vectorcraft_color::Swatch| matches!(&sw.paint, Paint::Pattern { pattern, .. } if *pattern == name);
        d.swatches.retain(|sw| !is_it(sw));
        for g in &mut d.swatch_groups {
            g.swatches.retain(|sw| !is_it(sw));
        }
        let ids = nodes_using(d, Some(&name));
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                for it in n.appearance.items.iter_mut() {
                    let p = match it {
                        vectorcraft_doc::AppearanceItem::Fill(f) => &mut f.paint,
                        vectorcraft_doc::AppearanceItem::Stroke(s) => &mut s.paint,
                    };
                    if matches!(p, Paint::Pattern { pattern, .. } if *pattern == name) {
                        *p = Paint::None;
                    }
                }
            }
        }
        Ok(ids.len())
    })?;
    Ok(json!({ "unpainted": n }))
}

fn pattern_list(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let pats: Vec<Value> = st
        .doc
        .patterns
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "tileType": p.tile_type.id(),
                "brickOffset": match p.tile_type { TileType::BrickByRow { offset } | TileType::BrickByColumn { offset } => offset, _ => 0.5 },
                "tile": [p.tile.x0, p.tile.y0, p.tile.width(), p.tile.height()],
                "width": p.tile.width(),
                "height": p.tile.height(),
                "hSpacing": p.h_spacing,
                "vSpacing": p.v_spacing,
                "sizeTileToArt": p.size_tile_to_art,
                "overlap": {"h": if p.overlap.right_in_front { "right" } else { "left" }, "v": if p.overlap.bottom_in_front { "bottom" } else { "top" }},
                "copies": p.copies,
                "dimCopies": p.dim_copies,
                "showTileEdge": p.show_tile_edge,
                "artCount": p.art.len(),
            })
        })
        .collect();
    let editing = st.doc.pattern_edit.as_ref().map(|e| json!({"name": e.pattern, "layer": e.layer.0}));
    Ok(json!({ "patterns": pats, "editing": editing }))
}

fn pattern_transform(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "pattern.transform";
    let m = matrix_param(p, "matrix").ok_or_else(|| bad(C, "missing `matrix` [a,b,c,d,e,f]"))?;
    let ids = targets(s, p)?;
    if ids.is_empty() {
        return Err(bad(C, "nothing selected"));
    }
    let n = s.edit("Transform Patterns", |d, _| {
        let mut all = vec![];
        for id in &ids {
            if let Some(n) = d.node(*id) {
                n.walk(&mut |c| {
                    if pattern::uses_pattern(c, None) {
                        all.push(c.id);
                    }
                });
            }
        }
        for id in &all {
            if let Some(n) = d.node_mut(*id) {
                pattern::transform_pattern_paints(n, m);
            }
        }
        Ok(all.len())
    })?;
    Ok(json!({ "count": n }))
}

// ---------- Repeat ----------

/// Make a repeat from the selection, or switch the selected repeat's kind.
fn make_repeat(s: &mut Session, p: &Value, label: &str, make: impl Fn(Vec<Arc<Node>>) -> RepeatSpec) -> Result<Value> {
    let existing = selected_repeats(s);
    if ids_param(p, "ids").is_none()
        && let [r] = existing.as_slice()
    {
        let r = *r;
        s.edit(label, |d, _| {
            let n = d.node_mut(r).ok_or(EngineError::NoNode(r))?;
            if let NodeKind::Repeat(spec) = &mut n.kind {
                spec.kind = make(spec.source.clone()).kind;
            }
            Ok(())
        })?;
        return Ok(json!({ "id": r.0 }));
    }
    let roots = roots(s, p)?;
    if roots.is_empty() {
        return Err(EngineError::Other("select the art to repeat".into()));
    }
    let id = s.edit(label, |d, sel| {
        let top = *roots.last().ok_or_else(|| EngineError::Other("select the art to repeat".into()))?;
        let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let nodes: Vec<Arc<Node>> = roots.iter().filter_map(|id| d.node(*id).cloned()).map(Arc::new).collect();
        let id = d.alloc_id();
        d.insert(par, idx + 1, Node::new(id, NodeKind::Repeat(make(nodes))))?;
        for r in &roots {
            d.remove(*r)?;
        }
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn repeat_radial(s: &mut Session, p: &Value) -> Result<Value> {
    let n = f64_or(p, "instances", 8.0);
    if !(1.0..=1000.0).contains(&n) {
        return Err(bad("object.repeat.radial", "instances must be 1..1000"));
    }
    let radius = p.get("radius").and_then(Value::as_f64).filter(|r| r.is_finite() && *r >= 0.0);
    make_repeat(s, p, "Radial Repeat", |src| RepeatSpec::radial(src, n as u32, radius))
}

fn repeat_grid(s: &mut Session, p: &Value) -> Result<Value> {
    let (hs, vs) = (p.get("hSpacing").and_then(Value::as_f64), p.get("vSpacing").and_then(Value::as_f64));
    let rows = p.get("rows").and_then(Value::as_u64).map(|v| v.clamp(1, 500) as u32);
    let cols = p.get("cols").and_then(Value::as_u64).map(|v| v.clamp(1, 500) as u32);
    make_repeat(s, p, "Grid Repeat", |src| {
        let b = vectorcraft_doc::live::nodes_bounds(&src).unwrap_or(Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut r = RepeatSpec::grid(src, hs.unwrap_or(b.width() / 4.0), vs.unwrap_or(b.height() / 4.0));
        if let RepeatKind::Grid { rows: rr, cols: cc, .. } = &mut r.kind {
            *rr = rows.unwrap_or(*rr);
            *cc = cols.unwrap_or(*cc);
        }
        r
    })
}

fn repeat_mirror(s: &mut Session, p: &Value) -> Result<Value> {
    let angle = f64_or(p, "angle", 90.0);
    let offset = f64_or(p, "offset", 10.0);
    make_repeat(s, p, "Mirror Repeat", |src| RepeatSpec::mirror(src, angle, offset))
}

fn repeat_release(s: &mut Session, _: &Value) -> Result<Value> {
    let reps = selected_repeats(s);
    let ids = s.edit("Release Repeat", |d, sel| {
        let mut out = vec![];
        for r in &reps {
            let Some(n) = d.node(*r).cloned() else { continue };
            let (par, idx, _) = d.position(*r).ok_or(EngineError::NoNode(*r))?;
            d.remove(*r)?;
            for (k, c) in n.children().into_iter().flatten().enumerate() {
                out.push(c.id);
                d.insert(par, idx + k, (**c).clone())?;
            }
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}

fn fix_ids(d: &mut Document, n: &mut Node) {
    if n.id == NodeId(0) {
        n.id = d.alloc_id();
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            fix_ids(d, Arc::make_mut(c));
        }
    }
}

fn repeat_expand(s: &mut Session, _: &Value) -> Result<Value> {
    let reps = selected_repeats(s);
    let ids = s.edit("Expand Repeat", |d, sel| {
        let mut out = vec![];
        for r in &reps {
            let Some(n) = d.node(*r).cloned() else { continue };
            let mut g = vectorcraft_doc::live::expanded_group(&n, None);

            // The group keeps the repeat's id; every generated instance node gets a fresh one.
            if let Some(ch) = g.children_mut() {
                for c in ch.iter_mut() {
                    fix_ids(d, Arc::make_mut(c));
                }
            }
            let (par, idx, _) = d.position(*r).ok_or(EngineError::NoNode(*r))?;
            d.remove(*r)?;
            d.insert(par, idx, g)?;
            out.push(*r);
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}

fn repeat_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.repeat.options";
    let reps = selected_repeats(s);
    let num = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
    let flag = |k: &str| p.get(k).and_then(Value::as_bool);
    if num("instances").is_some_and(|n| !(1.0..=1000.0).contains(&n)) {
        return Err(bad(C, "instances must be 1..1000"));
    }
    let grid_type = match str_param(p, "gridType") {
        Some(t) => Some(TileType::parse(t, num("brickOffset")).ok_or_else(|| bad(C, format!("unknown gridType `{t}`")))?),
        None => None,
    };
    let center = point_param(p, "center");
    s.edit("Repeat Options", |d, _| {
        for r in &reps {
            let Some(NodeKind::Repeat(spec)) = d.node_mut(*r).map(|n| &mut n.kind) else { continue };
            match &mut spec.kind {
                RepeatKind::Radial { instances, radius, center: c, reverse_overlap, start_angle, end_angle } => {
                    if let Some(v) = num("instances") {
                        *instances = v as u32;
                    }
                    if let Some(v) = num("radius") {
                        *radius = v.max(0.0);
                    }
                    if let Some(v) = flag("reverseOverlap") {
                        *reverse_overlap = v;
                    }
                    if let Some(v) = num("startAngle") {
                        *start_angle = v;
                    }
                    if let Some(v) = num("endAngle") {
                        *end_angle = v;
                    }
                    if let Some(v) = center {
                        *c = v;
                    }
                }
                RepeatKind::Grid { h_spacing, v_spacing, rows, cols, grid_type: gt, flip_rows, flip_cols } => {
                    if let Some(v) = num("hSpacing") {
                        *h_spacing = v;
                    }
                    if let Some(v) = num("vSpacing") {
                        *v_spacing = v;
                    }
                    if let Some(v) = num("rows") {
                        *rows = (v as u32).clamp(1, 500);
                    }
                    if let Some(v) = num("cols") {
                        *cols = (v as u32).clamp(1, 500);
                    }
                    if let Some(t) = grid_type {
                        *gt = t;
                    }
                    if let Some(v) = flag("flipRows") {
                        *flip_rows = v;
                    }
                    if let Some(v) = flag("flipCols") {
                        *flip_cols = v;
                    }
                }
                RepeatKind::Mirror { angle, center: c } => {
                    if let Some(v) = num("angle") {
                        *angle = v;
                    }
                    if let Some(v) = center {
                        *c = v;
                    }
                }
            }
        }
        Ok(())
    })?;
    ok()
}
