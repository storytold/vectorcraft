//! Symbol editing mode (Edit Symbol, a double-click on an instance or on a symbol in the Symbols
//! panel).
//!
//! The symbol's art moves onto a temporary isolated layer, placed where the instance it was
//! entered from shows it (from the panel: at the center of the active artboard, or of the view in a
//! document without artboards), where every tool and command can edit it while the rest of the
//! document is dimmed. After each edit [`sync`] writes the layer's art back into the symbol, so
//! every instance updates live. Leaving the mode
//! (`symbol.exitEditing`, Escape, the isolation bar's back button, a double-click outside the art)
//! removes the layer, and saving or exporting never writes it ([`Document::without_edit_modes`]).
//! Symbol editing is not entered while a pattern or an opacity mask is edited, and neither of those
//! is entered while a symbol is edited.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, Selection, SymbolEdit, SymbolRegistration};
use vectorcraft_geom::{Affine, Point};

use super::brushsym::{natural_size, place_xf};
use super::{EngineError, Result, Session, bad, id_param, number_or, ok, str_param};

/// The name of the undo step that enters symbol editing mode.
const ENTER: &str = "Edit Symbol";

/// The symbol instances among the selected objects, members of selected groups included.
fn selected_instances(st: &crate::DocState) -> Vec<NodeId> {
    let mut out = vec![];
    for id in &st.selection.objects {
        if let Some(n) = st.doc.node(*id) {
            n.walk(&mut |c| {
                if matches!(c.kind, NodeKind::SymbolInstance { .. }) && !out.contains(&c.id) {
                    out.push(c.id);
                }
            });
        }
    }
    out
}

/// The symbol and transform of instance `id`.
fn instance_of(d: &Document, id: NodeId) -> Option<(String, Affine)> {
    match &d.node(id)?.kind {
        NodeKind::SymbolInstance { symbol, xf } => Some((symbol.clone(), *xf)),
        _ => None,
    }
}

/// Where editing places the art: the instance's own transform, or for one that flattens the art,
/// a placement at the symbol's natural size around the same point.
fn editing_xf(d: &Document, name: &str, xf: Affine) -> Affine {
    let det = xf.determinant();
    if det.is_finite() && det.abs() > 1e-12 { xf } else { place_xf(xf * Point::ZERO, natural_size(d, name), SymbolRegistration::CENTER) }
}

/// Enter symbol editing mode (`symbol.edit`).
pub(crate) fn enter(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.edit";
    let st = s.doc()?;
    if st.doc.mask_edit.is_some() {
        return Err(bad(C, "stop editing the opacity mask first"));
    }
    if st.doc.pattern_edit.is_some() {
        return Err(bad(C, "finish editing the pattern first"));
    }
    let given = id_param(p, "id").map(|i| vec![i]).or_else(|| super::ids_param(p, "ids").filter(|v| !v.is_empty()));
    let (name, xf, instance) = match given {
        Some(ids) => {
            let symbols = ids.iter().map(|id| instance_of(&st.doc, *id)).collect::<Option<Vec<_>>>();
            let symbols = symbols.ok_or_else(|| bad(C, "select a symbol instance"))?;
            let (name, xf) = symbols.first().cloned().ok_or_else(|| bad(C, "select a symbol instance"))?;
            if symbols.iter().any(|(n, _)| *n != name) {
                return Err(bad(C, "select instances of one symbol"));
            }
            let xf = editing_xf(&st.doc, &name, xf);
            (name, xf, ids.first().copied())
        }
        None => {
            let picked = if str_param(p, "name").is_some() { vec![] } else { selected_instances(st) };
            let symbols: Vec<(String, Affine)> = picked.iter().filter_map(|id| instance_of(&st.doc, *id)).collect();
            match symbols.first() {
                Some((first, xf)) => {
                    if symbols.iter().any(|(n, _)| n != first) {
                        return Err(bad(C, "select instances of one symbol"));
                    }
                    (first.clone(), editing_xf(&st.doc, first, *xf), picked.first().copied())
                }
                None => {
                    let d = &st.doc;
                    let name = match str_param(p, "name") {
                        Some(n) if d.symbols.iter().any(|x| x.name == n) => n.to_string(),
                        Some(n) => return Err(bad(C, format!("no symbol named `{n}`"))),
                        None => d
                            .unknown
                            .get(super::brushsym::CURRENT_SYMBOL)
                            .and_then(Value::as_str)
                            .filter(|n| d.symbols.iter().any(|x| x.name == *n))
                            .or_else(|| d.symbols.first().map(|x| x.name.as_str()))
                            .map(str::to_string)
                            .ok_or_else(|| EngineError::Other("the document has no symbols".into()))?,
                    };
                    let artboard = p.get("artboard").and_then(Value::as_u64).map_or(0, super::index);
                    let center = match d.artboards.get(artboard).or(d.artboards.first()) {
                        Some(a) => a.rect.center(),
                        None => super::point_param(p, "viewCenter").unwrap_or(Point::ZERO),
                    };
                    let at = Point::new(number_or(p, "x", center.x, C)?, number_or(p, "y", center.y, C)?);
                    (name.clone(), place_xf(at, natural_size(d, &name), SymbolRegistration::CENTER), None)
                }
            }
        }
    };
    // Editing another symbol (one from the Symbols panel, or through the API): the current edit
    // ends first.
    if st.doc.symbol_edit.is_some() {
        if instance.is_some_and(|i| st.doc.symbol_edit.as_ref().is_some_and(|e| st.doc.ancestry(i).unwrap_or_default().contains(&e.layer))) {
            return Err(bad(C, "exit isolation mode first"));
        }
        leave(s, &json!({}))?;
    }
    if let Some(i) = s.doc()?.isolation {
        super::settle_interaction(s)?;
        super::distortcmds::finish_edit_text(s.doc_mut()?, i);
    }
    s.end_text_edit();
    let st = s.doc()?;
    let back = Return { redo: st.history.redo.clone(), before: Some(st.doc.clone()) };
    let (isolation, active_layer) = (st.isolation, st.active_layer);
    let layer = s.edit(ENTER, |d, sel| {
        let layer = begin(d, sel, &name, xf, instance)?;
        if let Some(e) = d.symbol_edit.as_mut() {
            e.back_isolation = isolation;
            e.back_layer = active_layer;
        }
        Ok(layer)
    })?;
    s.doc_mut()?.symbol_edit_return = back;
    show_editing(s, layer)?;
    let mut out = json!({ "name": name, "layer": layer.0 });
    if let Some(i) = instance {
        out["id"] = json!(i.0);
    }
    Ok(out)
}

/// A group whose members editing spreads out on the layer: nothing of its own that they would lose
/// while they are edited.
fn spreads(art: &Node) -> bool {
    matches!(art.kind, NodeKind::Group { clip: false, .. })
        && art.has_default_transparency()
        && art.appearance.items.is_empty()
        && art.appearance.effects.is_empty()
}

/// Enter symbol editing for `name` inside an edit: copy its art, placed by `xf`, onto a new
/// editing layer and select it. Returns the layer.
fn begin(d: &mut Document, sel: &mut Selection, name: &str, xf: Affine, instance: Option<NodeId>) -> Result<NodeId> {
    let art =
        d.symbols.iter().find(|x| x.name == name).map(|x| x.art.clone()).ok_or_else(|| EngineError::Other(format!("no symbol named `{name}`")))?;
    let group = spreads(&art);
    let parts = match art.children() {
        Some(ch) if group => ch.clone(),
        _ => vec![art],
    };
    let layer = d.add_layer(Some(name));
    let mut ids = Vec::with_capacity(parts.len());
    for part in &parts {
        let mut copy = d.reid(part);
        copy.transform(xf, false);
        ids.push(d.insert(Some(layer), usize::MAX, copy)?);
    }
    // Copied type whose layout caches `reid` dropped has them again.
    super::typecmd::refresh_bounds_of(d, &ids);
    d.symbol_edit = Some(SymbolEdit { symbol: name.to_string(), layer, instance, xf, group, changed: false, back_isolation: None, back_layer: None });
    sel.set(ids);
    Ok(layer)
}

/// Leave symbol editing mode (`symbol.exitEditing`); a no-op outside it. `object.exitIsolation` runs
/// it too while a symbol is edited. Isolation and the current layer go back to what they were
/// before. When nothing changed the art since editing was entered, the entering step is taken back
/// instead of a step being added, so the document and its redo steps are as they were.
pub(crate) fn leave(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(se) = s.doc()?.doc.symbol_edit.clone() else { return ok() };
    super::settle_interaction(s)?;
    super::distortcmds::finish_edit_text(s.doc_mut()?, se.layer);
    s.end_text_edit();
    let st = s.doc_mut()?;
    let unchanged = !st.doc.symbol_edit.as_ref().is_some_and(|e| e.changed);
    let Return { redo, before } = std::mem::take(&mut st.symbol_edit_return);
    // Saved while editing: the saved file holds no editing layer, so leaving keeps it saved.
    let saved_here = st.saved_doc.symbol_edit.as_ref().is_some_and(|e| e.layer == se.layer);
    let clean = !st.is_dirty();
    // Not after edits that were undone (their redo steps hold the editing layer), nor after a save
    // (taking the entering step back would undo what the save changed). The last step is the
    // entering step only when it starts from the document entering started from: a batch step
    // that entered after other edits starts from before them.
    if unchanged
        && !saved_here
        && st.history.redo.is_empty()
        && let Some(entry) = st.history.undo.last()
        && before.as_ref().is_some_and(|b| Arc::ptr_eq(b, &entry.doc))
        && let Some(entry) = st.history.undo.pop()
    {
        let now = std::mem::replace(&mut st.doc, entry.doc);
        // What changed outside the undo history while editing (the current symbol and brush, the
        // perspective grid's view, export settings, the grid preferences) stays as it is now.
        if st.doc.unknown != now.unknown || st.doc.export_settings != now.export_settings || st.doc.grid != now.grid {
            let d = Arc::make_mut(&mut st.doc);
            d.unknown = now.unknown.clone();
            d.export_settings = now.export_settings.clone();
            d.grid = now.grid.clone();
        }
        st.selection = Arc::unwrap_or_clone(entry.selection);
        st.selection.prune(&st.doc);
        st.history.redo = redo;
        st.revision += 1;
    } else {
        s.edit("Exit Symbol Editing Mode", |d, sel| {
            finish(d, sel);
            Ok(())
        })?;
        if saved_here && clean {
            s.doc_mut()?.mark_saved();
        }
    }
    leave_editing(s, &se)?;
    Ok(json!({ "name": se.symbol }))
}

/// Leave symbol editing inside an edit (no-op when not editing): drop the editing layer and select
/// the instance editing was entered from. The symbol already holds the layer's art ([`sync`]).
pub(crate) fn finish(d: &mut Document, sel: &mut Selection) {
    let Some(se) = d.symbol_edit.take() else { return };
    let _ = d.remove(se.layer);
    match se.instance.filter(|i| d.node(*i).is_some()) {
        Some(i) => sel.set([i]),
        None => sel.clear(),
    }
}

/// The redo steps entering symbol editing mode cleared, which leaving without an edit gives back,
/// and the document entering started from, which tells the entering step apart from any other.
#[derive(Clone, Debug, Default)]
pub(crate) struct Return {
    redo: Vec<crate::HistoryEntry>,
    before: Option<Arc<Document>>,
}

/// Isolate the editing layer `layer` and draw into it.
fn show_editing(s: &mut Session, layer: NodeId) -> Result<()> {
    let st = s.doc_mut()?;
    st.isolation = Some(layer);
    st.active_layer = Some(layer);
    st.revision += 1;
    Ok(())
}

/// Return to the document after leaving the editing session `se`: to the isolation and current
/// layer from before it, while they are still there.
pub(crate) fn leave_editing(s: &mut Session, se: &SymbolEdit) -> Result<()> {
    let st = s.doc_mut()?;
    st.symbol_edit_return = Return::default();
    go_back(st, se);
    st.revision += 1;
    Ok(())
}

/// Out of the editing session `se`: the isolation and current layer from before it, else none and
/// the default layer.
pub(crate) fn go_back(st: &mut crate::DocState, se: &SymbolEdit) {
    st.isolation = se.back_isolation.filter(|i| st.doc.node(*i).is_some());
    st.active_layer = se.back_layer.filter(|l| st.doc.node(*l).is_some_and(|n| n.is_layer())).or_else(|| st.doc.default_layer());
}

/// After undo or redo, isolation follows the restored document into or out of symbol editing
/// (`was` is the editing session before, if any), so it never names a layer that is gone. A group
/// isolated inside the symbol's art stays isolated while it is there.
pub(crate) fn follow_history(st: &mut crate::DocState, was: Option<&SymbolEdit>) {
    match st.doc.symbol_edit.as_ref().map(|e| e.layer) {
        Some(layer) => {
            let inside = st.isolation.is_some_and(|i| i != layer && st.doc.ancestry(i).is_some_and(|a| a.contains(&layer)));
            if !inside {
                st.isolation = Some(layer);
            }
            st.active_layer = Some(layer);
        }
        None => {
            // Out of the mode: isolation on the editing layer or inside it, and a current layer
            // that is gone, go back to what they were before it.
            let Some(was) = was else { return };
            let gone = |id: Option<NodeId>| id.is_some_and(|i| i == was.layer || st.doc.node(i).is_none());
            if gone(st.isolation) || gone(st.active_layer) {
                go_back(st, was);
            }
        }
    }
}

/// Does `n` hold an instance of symbol `name`, directly or through the art of the symbols it holds
/// instances of?
fn holds_symbol(d: &Document, n: &Node, name: &str, seen: &mut HashSet<String>) -> bool {
    let mut found = false;
    let mut nested = vec![];
    n.walk(&mut |c| {
        if let NodeKind::SymbolInstance { symbol, .. } = &c.kind {
            if symbol == name {
                found = true;
            } else if !seen.contains(symbol) {
                nested.push(symbol.clone());
            }
        }
    });
    if found {
        return true;
    }
    for symbol in nested {
        if seen.insert(symbol.clone())
            && let Some(art) = d.symbols.iter().find(|x| x.name == symbol).map(|x| x.art.clone())
            && holds_symbol(d, &art, name, seen)
        {
            return true;
        }
    }
    false
}

/// The object at `path` (child indices, paint order) among `nodes` and the groups they hold.
fn at_path<'a>(nodes: &'a [Arc<Node>], path: &[u32]) -> Option<&'a Node> {
    let (first, rest) = path.split_first()?;
    let mut n: &Node = nodes.get(*first as usize)?;
    for i in rest {
        n = n.children()?.get(*i as usize)?;
    }
    Some(n)
}

/// The path of object `id` among `nodes` and the groups they hold, at most `depth` levels down.
fn path_of(nodes: &[Arc<Node>], id: NodeId, depth: usize) -> Option<Vec<u32>> {
    if depth == 0 {
        return None;
    }
    for (i, n) in nodes.iter().enumerate() {
        let i = u32::try_from(i).ok()?;
        if n.id == id {
            return Some(vec![i]);
        }
        if let Some(mut p) = n.children().and_then(|ch| path_of(ch, id, depth - 1)) {
            p.insert(0, i);
            return Some(p);
        }
    }
    None
}

/// Each instance's appearance changes to objects of symbol `name`'s art follow those objects
/// through an edit of the editing layer: `old` and `new` are the layer's objects before and after
/// with whether the symbol's art is a group of them (else it is the one object). A change to an
/// object the edit removed names no object afterwards.
fn follow_objects(d: &mut Document, name: &str, old: (&[Arc<Node>], bool), new: (&[Arc<Node>], bool)) {
    let depth = vectorcraft_doc::symbol_overrides::MAX_PATH_DEPTH + 1;
    for id in super::brushsym::instances_of(d, Some(name)) {
        let Some(n) = d.node_mut(id) else { continue };
        if n.extras().symbol_overrides.is_empty() {
            continue;
        }
        let mut e = n.extras_mut();
        e.symbol_overrides.retain_mut(|o| {
            // The art's own group stays the art's root.
            if old.1 && o.path.is_empty() {
                return true;
            }
            let on_layer: Vec<u32> = if old.1 { o.path.clone() } else { std::iter::once(0).chain(o.path.iter().copied()).collect() };
            let moved = at_path(old.0, &on_layer).and_then(|obj| path_of(new.0, obj.id, depth)).and_then(|p| match new.1 {
                true => Some(p),
                false => p.split_first().filter(|(first, _)| **first == 0).map(|(_, rest)| rest.to_vec()),
            });
            match moved {
                Some(p) => {
                    o.path = p;
                    true
                }
                None => false,
            }
        });
    }
}

/// Write the editing layer's art back into the symbol (called after every edit, with the document
/// as it was before); each instance of a dynamic symbol keeps its appearance changes to the objects
/// the art still has. Leaves the mode when the symbol or the layer is gone. Refuses art that holds
/// an instance of the symbol itself.
pub(crate) fn sync(before: &Document, d: &mut Document) -> Result<()> {
    let Some(se) = d.symbol_edit.clone() else { return Ok(()) };
    let art: Option<Vec<Arc<Node>>> = d.node(se.layer).and_then(|l| l.children().cloned());
    let (Some(art), Some(index)) = (art, d.symbols.iter().position(|x| x.name == se.symbol)) else {
        d.symbol_edit = None;
        let _ = d.remove(se.layer);
        return Ok(());
    };
    // Entering, and edits that left the layer's art alone, write nothing.
    let same = |b: &[Arc<Node>]| b.len() == art.len() && b.iter().zip(&art).all(|(a, b)| Arc::ptr_eq(a, b));
    let (old, old_group) = match before.symbol_edit.as_ref().filter(|b| b.layer == se.layer) {
        None => return Ok(()),
        Some(_) if before.node(se.layer).and_then(|l| l.children()).is_some_and(|b| same(b)) => return Ok(()),
        Some(b) => (before.node(se.layer).and_then(|l| l.children().cloned()).unwrap_or_default(), b.group),
    };
    if art.iter().any(|n| holds_symbol(d, n, &se.symbol, &mut HashSet::new())) {
        return Err(EngineError::Other("a symbol can't hold an instance of itself".into()));
    }
    let inv = se.xf.inverse();
    let parts: Vec<Arc<Node>> = art
        .iter()
        .map(|n| {
            let mut n = (**n).clone();
            n.transform(inv, false);
            Arc::new(n)
        })
        .collect();
    // Once the layer's objects make a group, later edits give that group its members.
    let (new_art, group) = if se.group {
        let mut g = (*d.symbols[index].art).clone();
        if let Some(ch) = g.children_mut() {
            *ch = parts;
        }
        (g, true)
    } else {
        match <[Arc<Node>; 1]>::try_from(parts) {
            Ok([one]) => (Arc::unwrap_or_clone(one), false),
            Err(parts) => {
                let id = d.alloc_id();
                (Node::group(id, parts), true)
            }
        }
    };
    d.symbols[index].art = Arc::new(new_art);
    follow_objects(d, &se.symbol, (&old, old_group), (&art, group));
    super::brushsym::fit_instances_of(d, &se.symbol);
    if let Some(e) = d.symbol_edit.as_mut() {
        e.changed = true;
        e.group = group;
    }
    Ok(())
}
