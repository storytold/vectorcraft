//! Select menu long tail (Same → text/symbol attributes, Object → Direction Handles / text kinds /
//! brush strokes, Save/recall selections), View → Guides, and File → Close All / Document Color
//! Mode / File Info.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, Guide, Node, NodeId, NodeKind, SavedSelection, TextKind};
use vectorcraft_geom::Vec2;

use super::edit::selected_roots;
use super::pathops::Beside;
use super::select::{first_selectable, is_selectable, selectable_ids, walk_selectable};
use super::{
    CommandSpec, EngineError, Point, Result, Session, bad, checked_ids_param, cmd, f64_req, has_doc, has_selection, id_param, number_or, ok,
    str_param, switch_or, text_param,
};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "select.same.symbolInstance",
            "Symbol Instance",
            ["Select", "Same"],
            None,
            "{} select the instances of the symbol of the first selected object that can be selected, including a group member selected on its own; when that object is not a symbol instance, such as a group, nothing is selected; objects that are hidden or locked, inside a hidden or locked object or layer, on a template layer, on a locked artboard while nothing is isolated (new art while it stays selected excepted), or outside the isolated group or layer are passed over → {count}",
            has_selection,
            |s, _| same(s, "select.same.symbolInstance", |a, b| match (&a.kind, &b.kind) {
                (NodeKind::SymbolInstance { symbol: x, .. }, NodeKind::SymbolInstance { symbol: y, .. }) => x == y,
                _ => false,
            })
        ),
        cmd!(
            "select.same.linkBlockSeries",
            "Link Block Series",
            ["Select", "Same"],
            None,
            "{} select every frame of the threads of the selected type objects (and of the type inside selected groups); type in no thread selects itself; type and frames that are hidden or locked, inside a hidden or locked object or layer, on a template layer, or outside the isolated group or layer neither count as selected nor are selected → {count}",
            has_text_selection,
            same_link_block_series
        ),
        cmd!("select.same.fontFamily", "Font Family", ["Select", "Same"], None, "{} → {count}", has_selection, |s, _| same_text(
            s,
            "select.same.fontFamily",
            |a, b| a.font_family == b.font_family
        )),
        cmd!("select.same.fontFamilyStyle", "Font Family & Style", ["Select", "Same"], None, "{} → {count}", has_selection, |s, _| same_text(
            s,
            "select.same.fontFamilyStyle",
            |a, b| a.font_family == b.font_family && a.font_style == b.font_style
        )),
        cmd!("select.same.fontFamilyStyleSize", "Font Family, Style & Size", ["Select", "Same"], None, "{} → {count}", has_selection, |s, _| {
            same_text(s, "select.same.fontFamilyStyleSize", |a, b| {
                a.font_family == b.font_family && a.font_style == b.font_style && (a.size - b.size).abs() < 1e-6
            })
        }),
        cmd!("select.same.fontSize", "Font Size", ["Select", "Same"], None, "{} → {count}", has_selection, |s, _| same_text(
            s,
            "select.same.fontSize",
            |a, b| (a.size - b.size).abs() < 1e-6
        )),
        cmd!("select.same.textFillColor", "Text Fill Color", ["Select", "Same"], None, "{} → {count}", has_selection, |s, _| same_text(
            s,
            "select.same.textFillColor",
            |a, b| a.fill == b.fill
        )),
        cmd!("select.same.textStrokeColor", "Text Stroke Color", ["Select", "Same"], None, "{} → {count}", has_selection, |s, _| same_text(
            s,
            "select.same.textStrokeColor",
            |a, b| a.stroke == b.stroke
        )),
        cmd!(
            "select.same.textFillAndStrokeColor",
            "Text Fill & Stroke Color",
            ["Select", "Same"],
            None,
            "{} select the type objects with characters whose fill and stroke both match the first character of the first selected type → {count}",
            has_selection,
            |s, _| same_text(s, "select.same.textFillAndStrokeColor", |a, b| a.fill == b.fill && a.stroke == b.stroke)
        ),
        cmd!(
            "select.object.directionHandles",
            "Direction Handles",
            ["Select", "Object"],
            None,
            "{} direct-select every anchor (showing all handles) of the selected paths and the paths inside the selected objects, leaving out paths that are hidden or locked, inside a hidden or locked object or layer, on a template layer, or outside the isolated group or layer → {anchors}",
            has_selection,
            direction_handles
        ),
        cmd!("select.object.pointText", "Point Text Objects", ["Select", "Object"], None, "{} → {count}", has_doc, |s, _| by_kind(
            s,
            |n| matches!(
                &n.kind,
                NodeKind::Text(t) if matches!(t.kind, TextKind::Point)
            )
        )),
        cmd!("select.object.areaText", "Area Text Objects", ["Select", "Object"], None, "{} → {count}", has_doc, |s, _| by_kind(s, |n| matches!(
            &n.kind,
            NodeKind::Text(t) if matches!(t.kind, TextKind::Area { .. })
        ))),
        cmd!("select.object.brushStrokes", "Brush Strokes", ["Select", "Object"], None, "{} → {count}", has_doc, |s, _| by_kind(s, |n| n
            .appearance
            .stroke()
            .is_some_and(|st| st.brush.is_some()))),
        cmd!(
            "select.object.bristleBrushStrokes",
            "Bristle Brush Strokes",
            ["Select", "Object"],
            None,
            "{} objects whose topmost stroke has a bristle brush (type bristle in brush.list), whatever the brush is named → {count}",
            has_doc,
            |s, _| {
                let brushes = vectorcraft_brush::library(&s.doc()?.doc);
                by_kind(s, |n| {
                    n.appearance
                        .stroke()
                        .and_then(|st| st.brush.as_deref())
                        .and_then(|name| brushes.iter().find(|b| b.name == name))
                        .is_some_and(|b| matches!(b.kind, vectorcraft_brush::BrushKind::Bristle(_)))
                })
            }
        ),
        cmd!(
            "select.save",
            "Save Selection…",
            ["Select"],
            None,
            "{name?} save the current selection under a name, in the document (default \"Selection N\"; an existing name is replaced; at most 25, names up to 255 characters) → {name}",
            has_selection,
            save_selection
        ),
        cmd!(
            "select.update",
            "Update Selection",
            ["Select"],
            None,
            "{} make the saved selection last recalled (`select.recall`) hold the selected objects instead of its own, in one undo step → {name}",
            has_recalled_selection,
            update_selection
        ),
        cmd!(
            "select.recall",
            "Recall Selection",
            ["Select"],
            None,
            "{name} select the objects of a saved selection, leaving out those deleted since and those that are hidden or locked, inside a hidden or locked object or layer, on a template layer, or outside the isolated group or layer → {count}",
            has_saved_selection,
            recall_selection
        ),
        cmd!(
            "select.editSaved",
            "Edit Selection…",
            ["Select"],
            None,
            "{name, newName?: rename, delete?: bool} | {edits: [{name, newName?, delete?}…]} rename or delete saved selections, all in one undo step; names are those before the edit and must stay unique",
            has_saved_selection,
            edit_saved
        ),
        cmd!(query "select.savedList", "Saved Selections", [], None, "{} → [name…] for the active document", has_doc, |s, _| {
            Ok(json!(s.doc()?.doc.saved_selections.iter().map(|x| x.name.clone()).collect::<Vec<_>>()))
        }),
        cmd!(
            "view.guides.make",
            "Make Guides",
            ["View", "Guides"],
            Some("Cmd+5"),
            "{} turn the selected paths into guides → {count}",
            has_selection,
            make_guides
        ),
        cmd!(
            "view.guides.release",
            "Release Guides",
            ["View", "Guides"],
            Some("Cmd+Alt+5"),
            "{index?: a ruler guide's index, ids?: [guide path id…]} turn guides into paths: index releases that ruler guide and ids release those guide paths (and the guide paths inside them); otherwise the selected guide paths and ruler guides are released, and with no guide selected every guide path is released; a ruler guide becomes a straight path with no fill or stroke, across its artboard or, for a canvas guide, across the bounds of all the artboards; index and ids need guides unlocked; one undo step, the released paths selected → {count}",
            has_doc,
            release_guides
        ),
        cmd!(
            query "view.guides.lock",
            "Lock Guides",
            ["View", "Guides"],
            Some("Cmd+Alt+;"),
            "{locked?: bool} toggle (or set) the session's guide lock, leaving the lockGuidesByDefault preference as it is; locking deselects the ruler guides and guide paths in every open document; guide.add and a guide dragged out of a ruler still make new guides while locked → {locked}",
            has_doc,
            lock_guides
        ),
        cmd!(
            "view.guides.clear",
            "Clear Guides",
            ["View", "Guides"],
            None,
            "{} delete all ruler guides and guide paths → {count}",
            has_doc,
            clear_guides
        ),
        cmd!(
            "guide.add",
            "Add Guide",
            [],
            None,
            "{vertical: bool, pos: pt (x for vertical, y for horizontal), artboard?: index (an artboard guide: it runs across that artboard only and moves, is copied and is deleted with it; default a canvas guide, across the whole canvas), layer?: id (the layer it is on, shown, hidden, locked and deleted with it; default the current layer)} → {index}",
            has_doc,
            guide_add
        ),
        cmd!(
            query "guide.list",
            "Guides",
            [],
            None,
            "{} → [{index, id (its node, which `layer.setProps` hides or locks on its own), vertical, pos, selected, artboard?: index (an artboard guide's), layer?: id (the layer it is on), hidden, locked (its own visibility and lock), shown, editable (it and its layer shown and unlocked)}…] the ruler guides",
            has_doc,
            guide_list
        ),
        cmd!(
            "guide.select",
            "Select Guides",
            [],
            None,
            "{indexes: [index…] (at most 10,000), toggle?: bool} select ruler guides on their own (deselecting the art); toggle adds or removes them instead, keeping the selected art (a Shift-click on a guide) → {selected: [index…]}",
            guides_unlocked,
            guide_select
        ),
        cmd!(
            "guide.remove",
            "Remove Guide",
            [],
            None,
            "{index?, indexes?: [index…] (at most 10,000)} delete ruler guide `index` (or guides `indexes`, as dragging their rows onto the Layers panel's trash does), else the selected ones (as Delete / edit.clear does), in one undo step; give `index` or `indexes`, not both, and an index given twice counts once; a hidden or locked guide, or one on a hidden or locked layer, is an error → {count}",
            guides_unlocked,
            guide_remove
        ),
        cmd!(
            "guide.move",
            "Move Guide",
            [],
            None,
            "{index, pos: pt} | {dx?, dy?: pt, copy?: bool} put ruler guide `index` at `pos`, or move the selected guides (vertical ones by dx, horizontal ones by dy); copy leaves them and selects the moved copies",
            guides_unlocked,
            guide_move
        ),
        cmd!(
            "guide.setLayer",
            "Move Guide to Layer",
            [],
            None,
            "{index?, indexes?: [index…] (at most 10,000), layer: id} put ruler guide `index` (or guides `indexes`), else the selected guides, on layer `layer` (a layer or sublayer), as dragging their rows onto that layer's row in the Layers panel does; give `index` or `indexes`, not both, and an index given twice counts once → {count}",
            guides_unlocked,
            guide_set_layer
        ),
        cmd!("file.closeAll", "Close All", ["File"], Some("Cmd+Alt+W"), "{} → {closed}", has_doc, close_all),
        cmd!(
            "file.documentColorMode",
            "Document Color Mode",
            ["File", "Document Color Mode"],
            None,
            "{mode: \"cmyk\"|\"rgb\", convert?: true (convert every color of the art (opacity masks, effect colors and graphs' series and label paints included), symbols, graph designs, pattern tiles, graphic styles (their effect colors included), brushes, swatches and the paints new art takes through the color settings, together in one undo step (Undo restores their original values; Redo reapplies the conversion); swatch links kept; Gray colors stay Gray, on the black plate), intent?, grays?: \"profile\" (default: RGB grays separate through the CMYK profile like any color, into light grays in cyan, magenta and yellow only, darker four-color grays and a rich black) | \"black\" (to CMYK, RGB grays with R = G = B go on the black plate only, K = their gray value)} → {changed: how many colors of the art, symbols, graph designs and pattern tiles changed; the colors of graphic styles and brushes are not counted}",
            has_doc,
            super::colormgmt::convert_mode
        ),
        cmd!(
            "file.info",
            "File Info…",
            ["File"],
            Some("Cmd+Alt+Shift+I"),
            "{title?, author?, authorTitle?, description?, keywords?: [string…]|\"a, b\" (each once), rating?: 0–5, copyrightStatus?: \"unknown\"|\"copyrighted\"|\"publicDomain\", copyrightNotice?, copyrightUrl?, created?, modified?, colorMode?, units?, artboards?, objects? (the last six only as read back; ignored)} set the File Info in one undo step (SVG with metadata, PDF and PNG exports carry it); no params → {title, author, authorTitle, description, keywords, rating, copyrightStatus, copyrightNotice, copyrightUrl, created, modified (ISO 8601 UTC or null; read-only), colorMode, units, artboards, objects}",
            has_doc,
            super::fileinfo::file_info
        ),
    ]
}

impl Session {
    /// View → Guides → Lock Guides state (canvas guide dragging should honour it).
    pub fn guides_locked(&self) -> bool {
        self.menu.guides_locked
    }

    /// Set View → Guides → Lock Guides. Locked guides can't stay selected, ruler guides or guide
    /// paths, in any open document ([`Session::select`] lets those go in the active one).
    pub(crate) fn set_guides_locked(&mut self, locked: bool) {
        self.menu.guides_locked = locked;
        if !locked {
            return;
        }
        let guides_selected =
            self.doc().is_ok_and(|st| !st.selection.guides.is_empty() || st.selection.objects.iter().any(|id| crate::is_guide_path(&st.doc, *id)));
        // It fails only without a document, and then no guide is selected.
        if guides_selected {
            let _ = self.select(|_, sel| sel.guides.clear());
        }
        let active = self.active;
        for (i, st) in self.docs.iter_mut().enumerate() {
            if Some(i) != active && crate::deselect_hidden(&self.menu, &st.doc, &mut st.selection) {
                st.revision += 1;
            }
        }
    }
}

// ---------- Select ----------

fn candidates(s: &Session, f: impl Fn(&Node) -> bool) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    let mut ids = vec![];
    walk_selectable(&st.doc, st.isolation, |n| {
        if !n.is_container() && f(n) {
            ids.push(n.id);
        }
        true
    });
    Ok(ids)
}

fn finish_same(s: &mut Session, cmd: &str, ids: Vec<NodeId>) -> Result<Value> {
    let from = s.doc()?.selection.objects.clone();
    s.select(|_, sel| sel.set(ids))?;
    let st = s.doc_mut()?;
    st.last_selection_cmd = Some((cmd.to_string(), json!({}), from));
    Ok(json!({ "count": st.selection.objects.len() }))
}

/// The objects `eq` matches with the [`first_selectable`] selected object.
fn same(s: &mut Session, cmd: &str, eq: fn(&Node, &Node) -> bool) -> Result<Value> {
    let r = first_selectable(s.doc()?).cloned().ok_or_else(|| bad(cmd, "nothing selected"))?;
    let ids = candidates(s, |n| eq(n, &r))?;
    finish_same(s, cmd, ids)
}

/// The first character style of the first type object [`each_selected_text`] gives.
fn first_text_style(s: &Session) -> Option<CharStyle> {
    let st = s.active()?;
    let mut out = None;
    each_selected_text(s, |id| {
        if let Some(NodeKind::Text(t)) = st.doc.node(id).map(|n| &n.kind) {
            out = Some(t.first_style());
        }
        out.is_none()
    });
    out
}

fn same_text(s: &mut Session, cmd: &str, eq: fn(&CharStyle, &CharStyle) -> bool) -> Result<Value> {
    let r = first_text_style(s).ok_or_else(|| bad(cmd, "select a text object"))?;
    let ids = candidates(s, |n| match &n.kind {
        NodeKind::Text(t) => t.runs.iter().any(|run| !run.text.is_empty() && eq(&run.style, &r)),
        _ => false,
    })?;
    finish_same(s, cmd, ids)
}

/// Call `f` with each type object among the selected objects and inside them, in selection order,
/// keeping only those [`walk_selectable`] visits: none that is hidden, locked, on a hidden, locked
/// or template layer, or outside the isolated group. A selected layer or sublayer, or a selected
/// group that holds the isolated group, gives the type in it that passes the same test. A type
/// object inside two selected objects is given twice. The walk stops when `f` returns false.
fn each_selected_text(s: &Session, mut f: impl FnMut(NodeId) -> bool) {
    fn inside(n: &Node, f: &mut impl FnMut(NodeId) -> bool) -> bool {
        if !n.visible || n.locked || n.is_template() {
            return true;
        }
        if matches!(n.kind, NodeKind::Text(_)) && !f(n.id) {
            return false;
        }
        n.children().into_iter().flatten().all(|c| inside(c, f))
    }
    fn from(n: &Node, selectable: &impl Fn(NodeId) -> bool, holds_isolated: &[NodeId], f: &mut impl FnMut(NodeId) -> bool) -> bool {
        if selectable(n.id) {
            return inside(n, f);
        }
        if n.is_layer() || holds_isolated.contains(&n.id) {
            return n.children().into_iter().flatten().all(|c| from(c, selectable, holds_isolated, f));
        }
        true
    }
    let Some(st) = s.active() else { return };
    let locks = st.doc.locked_artboards();
    let selectable = |id| is_selectable(&st.doc, st.isolation, &locks, id);
    // The isolated group and the groups and layers that hold it.
    let holds_isolated = st.isolation.and_then(|i| st.doc.ancestry(i)).unwrap_or_default();
    for id in &st.selection.objects {
        if let Some(n) = st.doc.node(*id)
            && !from(n, &selectable, &holds_isolated, &mut f)
        {
            return;
        }
    }
}

/// The type objects [`each_selected_text`] gives, each once, in selection order.
fn selected_text(s: &Session) -> Vec<NodeId> {
    let (mut out, mut seen) = (vec![], HashSet::new());
    each_selected_text(s, |id| {
        if seen.insert(id) {
            out.push(id);
        }
        true
    });
    out
}

/// A type object is selected, or is inside a selected object.
fn has_text_selection(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    let mut any = false;
    each_selected_text(s, |_| {
        any = true;
        false
    });
    if any { Ok(()) } else { Err("select a text object".into()) }
}

/// Select › Same › Link Block Series: the frames of the threads the selected type objects are
/// in, in thread order; a type object in no thread is its own series.
fn same_link_block_series(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "select.same.linkBlockSeries";
    let seeds = selected_text(s);
    if seeds.is_empty() {
        return Err(bad(C, "select a text object"));
    }
    let st = s.doc()?;
    let can = selectable_ids(&st.doc, st.isolation);
    // Each frame's thread: the first thread that lists it.
    let mut threads: HashMap<NodeId, &[NodeId]> = HashMap::new();
    for t in &st.doc.text_threads {
        for id in t {
            threads.entry(*id).or_insert(t.as_slice());
        }
    }
    let (mut ids, mut seen): (Vec<NodeId>, HashSet<NodeId>) = (vec![], HashSet::new());
    for seed in &seeds {
        let series = threads.get(seed).copied().unwrap_or(std::slice::from_ref(seed));
        for id in series {
            if can.contains(id) && seen.insert(*id) {
                ids.push(*id);
            }
        }
    }
    finish_same(s, C, ids)
}

fn by_kind(s: &mut Session, f: impl Fn(&Node) -> bool) -> Result<Value> {
    let ids = candidates(s, f)?;
    s.select(|_, sel| sel.set(ids))?;
    // The objects selected, after Lock Guides has left out guide paths.
    Ok(json!({ "count": s.doc()?.selection.objects.len() }))
}

fn direction_handles(s: &mut Session, _: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    let st = s.doc()?;
    let d = &st.doc;
    // Only paths that `walk_selectable` visits.
    let can = selectable_ids(d, st.isolation);
    let mut items: Vec<(NodeId, BTreeSet<(usize, usize)>)> = vec![];
    for r in &roots {
        if let Some(n) = d.node(*r) {
            n.walk(&mut |c| {
                if can.contains(&c.id)
                    && let Some(p) = c.path_data()
                {
                    items.push((c.id, p.anchors().map(|(si, ai, _)| (si, ai)).collect()));
                }
            });
        }
    }
    let total: usize = items.iter().map(|x| x.1.len()).sum();
    s.select(|_, sel| {
        sel.clear();
        for (id, refs) in items {
            sel.add(id);
            sel.anchors.insert(id, refs);
        }
    })?;
    Ok(json!({ "anchors": total }))
}

fn save_selection(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "select.save";
    let st = s.doc()?;
    let ids = st.selection.objects.clone();
    let saved = &st.doc.saved_selections;
    let name = text_param(p, "name", C)?.and_then(SavedSelection::clean_name).unwrap_or_else(|| SavedSelection::default_name(saved));
    let existing = saved.iter().position(|x| x.name == name);
    if existing.is_none() && saved.len() >= SavedSelection::MAX {
        return Err(bad(C, format!("a document keeps at most {} saved selections", SavedSelection::MAX)));
    }
    let mut slots = saved_ids(s)?;
    s.edit("Save Selection", |d, _| {
        let entry = SavedSelection { name: name.clone(), objects: ids };
        match existing.and_then(|i| d.saved_selections.get_mut(i)) {
            Some(x) => *x = entry,
            None => d.saved_selections.push(entry),
        }
        Ok(())
    })?;
    let st = s.doc_mut()?;
    if existing.is_none() {
        slots.push(st.recalled_selection.new_id());
    }
    st.recalled_selection.remember(&st.doc.saved_selections, slots);
    Ok(json!({ "name": name }))
}

fn recall_selection(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "select.recall";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing name"))?;
    let saved = &s.doc()?.doc.saved_selections;
    let i = saved.iter().position(|x| x.name == name).ok_or_else(|| bad(C, format!("no saved selection `{name}`")))?;
    let ids = saved.get(i).map(|x| x.objects.clone()).unwrap_or_default();
    let st = s.doc()?;
    let can = selectable_ids(&st.doc, st.isolation);
    s.select(|_, sel| sel.set(ids.into_iter().filter(|id| can.contains(id))))?;
    let st = s.doc_mut()?;
    st.recalled_selection.recall(&st.doc.saved_selections, i);
    Ok(json!({ "count": st.selection.len() }))
}

/// Recall and Edit Selection… need a document with at least one saved selection.
fn has_saved_selection(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.doc.saved_selections.is_empty() { Err("no saved selections".into()) } else { Ok(()) }
}

/// The session ids of the active document's saved selections ([`crate::RecalledSelection`]).
fn saved_ids(s: &mut Session) -> Result<Vec<u64>> {
    let st = s.doc_mut()?;
    Ok(st.recalled_selection.ids(&st.doc.saved_selections))
}

/// Where the saved selection last recalled is, while the document has it.
fn recalled_index(st: &crate::DocState) -> Option<usize> {
    st.recalled_selection.index_in(&st.doc.saved_selections)
}

/// Update Selection needs selected objects and a recalled saved selection the document still has.
fn has_recalled_selection(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    let st = s.active().ok_or("no document open")?;
    if recalled_index(st).is_some() { Ok(()) } else { Err("no saved selection recalled".into()) }
}

fn update_selection(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "select.update";
    let st = s.doc()?;
    let i = recalled_index(st).ok_or_else(|| bad(C, "no saved selection recalled"))?;
    let ids = st.selection.objects.clone();
    let name = st.doc.saved_selections.get(i).map(|x| x.name.clone()).unwrap_or_default();
    let slots = saved_ids(s)?;
    s.edit("Update Selection", |d, _| {
        if let Some(x) = d.saved_selections.get_mut(i) {
            x.objects = ids;
        }
        Ok(())
    })?;
    let st = s.doc_mut()?;
    st.recalled_selection.remember(&st.doc.saved_selections, slots);
    Ok(json!({ "name": name }))
}

fn edit_saved(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "select.editSaved";
    let edits = match p.get("edits").and_then(Value::as_array) {
        Some(list) => list.as_slice(),
        None => std::slice::from_ref(p),
    };
    let before = s.doc()?.doc.saved_selections.clone();
    // Edits name the selections as they are now, so they don't depend on each other's order.
    let mut after: Vec<Option<SavedSelection>> = before.iter().cloned().map(Some).collect();
    // Files may hold any number of saved selections: keep this linear.
    let mut index = std::collections::HashMap::with_capacity(before.len());
    for (i, x) in before.iter().enumerate() {
        index.entry(x.name.as_str()).or_insert(i);
    }
    for e in edits {
        let name = str_param(e, "name").ok_or_else(|| bad(C, "missing name"))?;
        let i = *index.get(name).ok_or_else(|| bad(C, format!("no saved selection `{name}`")))?;
        let delete = switch_or(e, "delete", false, C)?;
        let new_name = text_param(e, "newName", C)?;
        let Some(slot) = after.get_mut(i) else { continue };
        if delete {
            *slot = None;
        } else if let Some(n) = new_name.and_then(SavedSelection::clean_name)
            && let Some(x) = slot
        {
            x.name = n;
        }
    }
    let slots: Vec<u64> = saved_ids(s)?.into_iter().zip(&after).filter(|(_, x)| x.is_some()).map(|(id, _)| id).collect();
    let after: Vec<SavedSelection> = after.into_iter().flatten().collect();
    let mut names = std::collections::HashSet::with_capacity(after.len());
    for x in &after {
        if !names.insert(x.name.as_str()) {
            return Err(bad(C, format!("two saved selections would be named `{}`", x.name)));
        }
    }
    if *before != after {
        s.edit("Edit Selection", |d, _| {
            d.saved_selections = after.into();
            Ok(())
        })?;
        let st = s.doc_mut()?;
        st.recalled_selection.remember(&st.doc.saved_selections, slots);
    }
    ok()
}

// ---------- Guides ----------

fn guides_unlocked(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.menu.guides_locked { Err("guides are locked".into()) } else { Ok(()) }
}

fn set_guide_flag(n: &mut Node, on: bool, count: &mut usize) {
    match &mut n.kind {
        NodeKind::Path { guide, clipping: false, .. } => {
            if *guide != on {
                *guide = on;
                *count += 1;
            }
        }
        _ => {
            if let Some(ch) = n.children_mut() {
                for c in ch.iter_mut() {
                    set_guide_flag(Arc::make_mut(c), on, count);
                }
            }
        }
    }
}

fn make_guides(s: &mut Session, _: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    let n = s.edit("Make Guides", |d, sel| {
        let mut n = 0;
        for r in &roots {
            if let Some(node) = d.node_mut(*r) {
                set_guide_flag(node, true, &mut n);
            }
        }
        if n == 0 {
            return Err(EngineError::Other("Make Guides: select paths".into()));
        }
        sel.clear();
        Ok(n)
    })?;
    Ok(json!({ "count": n }))
}

fn guide_paths(d: &vectorcraft_doc::Document) -> Vec<NodeId> {
    let mut v = vec![];
    d.walk(|n| {
        if matches!(n.kind, NodeKind::Path { guide: true, .. }) {
            v.push(n.id);
        }
    });
    v
}

/// Turn guides into paths: ruler guide `index` and the guide paths in `ids`, else the selected
/// guide paths and ruler guides, else every guide path, in one undo step.
fn release_guides(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "view.guides.release";
    let (index, ids) = (p.get("index"), p.get("ids"));
    if (index.is_some() || ids.is_some()) && s.menu.guides_locked {
        return Err(EngineError::Other("guides are locked".into()));
    }
    let (rulers, paths): (Vec<NodeId>, Vec<NodeId>) = if index.is_some() || ids.is_some() {
        let rulers = match index {
            Some(v) => vec![editable_guide_index(s, Some(v), C)?],
            None => vec![],
        };
        let roots = if ids.is_some() { checked_ids_param(s, p, "ids", C)? } else { vec![] };
        let d = &s.doc()?.doc;
        let mut paths = vec![];
        let editable = d.editable();
        for r in roots {
            if let Some(n) = d.node(r) {
                n.walk(&mut |c| {
                    if matches!(c.kind, NodeKind::Path { guide: true, .. }) && editable(c.id) && !paths.contains(&c.id) {
                        paths.push(c.id);
                    }
                });
            }
        }
        (rulers, paths)
    } else {
        let st = s.doc()?;
        let all = guide_paths(&st.doc);
        let sel: Vec<NodeId> = all.iter().copied().filter(|id| st.selection.contains(*id)).collect();
        let rulers: Vec<NodeId> = if s.menu.guides_locked {
            vec![]
        } else {
            st.selection.guides.iter().copied().filter(|id| st.doc.ruler_guide(*id).is_some_and(|g| st.doc.guide_editable(&g))).collect()
        };
        let editable = st.doc.editable();
        if sel.is_empty() && rulers.is_empty() { (vec![], all.into_iter().filter(|id| editable(*id)).collect()) } else { (rulers, sel) }
    };
    // Find each ruler guide's line and its parent: the guide's layer, else where new art goes.
    let st = s.doc()?;
    let artboards = st.doc.artboards.iter().map(|a| a.rect).reduce(|a, b| a.union(b));
    let mut lines = Vec::with_capacity(rulers.len());
    for &i in &rulers {
        let Some(g) = st.doc.ruler_guide(i) else { continue };
        let span = st.doc.guide_span(&g).or_else(|| artboards.map(|r| if g.vertical { (r.y0, r.y1) } else { (r.x0, r.x1) }));
        let Some((a, b)) = span else { continue };
        let (a, b) = if g.vertical { (Point::new(g.pos, a), Point::new(g.pos, b)) } else { (Point::new(a, g.pos), Point::new(b, g.pos)) };
        let parent = match g.layer.filter(|l| st.doc.node(*l).is_some_and(Node::is_layer)) {
            Some(l) => Some(l),
            None => st.target_parent()?,
        };
        lines.push((i, parent, a, b));
    }
    let n = paths.len() + lines.len();
    if n == 0 {
        return Err(EngineError::Other("Release Guides: no guides".into()));
    }
    s.edit("Release Guides", |d, sel| {
        for id in &paths {
            if let Some(NodeKind::Path { guide, .. }) = d.node_mut(*id).map(|n| &mut n.kind) {
                *guide = false;
            }
        }
        let mut made = Vec::with_capacity(lines.len());
        for &(_, parent, a, b) in &lines {
            let id = d.alloc_id();
            let look = vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::None, vectorcraft_color::Paint::None, 0.0);
            d.insert(parent, usize::MAX, Node::path(id, vectorcraft_geom::shapes::line(a, b), look))?;
            made.push(id);
        }
        d.retain_guides(sel, |g| !lines.iter().any(|l| Some(l.0) == g.id));
        sel.set(paths.iter().copied().chain(made));
        Ok(())
    })?;
    Ok(json!({ "count": n }))
}

fn lock_guides(s: &mut Session, p: &Value) -> Result<Value> {
    let v = switch_or(p, "locked", !s.menu.guides_locked, "view.guides.lock")?;
    s.set_guides_locked(v);
    Ok(json!({ "locked": v }))
}

fn clear_guides(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let paths = guide_paths(&st.doc);
    let n = paths.len() + st.doc.ruler_guides().len();
    if n == 0 {
        return Ok(json!({ "count": 0 }));
    }
    s.edit("Clear Guides", |d, sel| {
        d.retain_guides(sel, |_| false);
        let mut gone = Beside::replacing(d, &paths);
        for id in &paths {
            if d.node(*id).is_some() {
                gone.put(d, *id, vec![])?;
            }
        }
        gone.finish(d)
    })?;
    Ok(json!({ "count": n }))
}

fn guide_add(s: &mut Session, p: &Value) -> Result<Value> {
    let pos = f64_req(p, "pos", "guide.add")?;
    let vertical = switch_or(p, "vertical", false, "guide.add")?;
    let artboard = match p.get("artboard").filter(|v| !v.is_null()) {
        Some(v) => {
            let boards = &s.doc()?.doc.artboards;
            let ab = v.as_u64().and_then(|i| boards.get(usize::try_from(i).ok()?)).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
            Some(ab.id)
        }
        None => None,
    };
    let st = s.doc()?;
    let layer = match id_param(p, "layer") {
        Some(l) if st.doc.node(l).is_some_and(Node::is_layer) => Some(l),
        Some(_) => return Err(bad("guide.add", "layer must name a layer or sublayer")),
        // Where new art goes (refused when no layer is shown and unlocked), or that group's layer.
        None => st.target_parent()?.and_then(|p| if st.doc.node(p).is_some_and(Node::is_layer) { Some(p) } else { st.doc.layer_containing(p) }),
    };
    let id = s.edit("New Guide", |d, _| Ok(d.add_ruler_guide(&Guide { artboard, layer, ..Guide::new(vertical, pos) })))?;
    let i = guide_indexes(&s.doc()?.doc, &[id]).first().copied();
    Ok(json!({ "index": i }))
}

/// The ids of the active document's ruler guides by index: the order of `guide.list` (ascending
/// id).
pub(crate) fn guide_ids(d: &vectorcraft_doc::Document) -> Vec<NodeId> {
    d.ruler_guides().into_iter().filter_map(|g| g.id).collect()
}

/// The `guide.list` indexes of ruler guides `ids`, in the order of `ids` (an id that is not a ruler
/// guide is left out).
pub fn guide_indexes(d: &vectorcraft_doc::Document, ids: &[NodeId]) -> Vec<usize> {
    let all = guide_ids(d);
    ids.iter().filter_map(|id| all.binary_search(id).ok()).collect()
}

fn guide_list(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let row = |(i, g): (usize, &Guide)| {
        let selected = g.id.is_some_and(|id| st.selection.guides.contains(&id));
        let mut r = json!({ "index": i, "vertical": g.vertical, "pos": g.pos, "selected": selected });
        if let Some(id) = g.id {
            r["id"] = json!(id.0);
        }
        if let Some(ab) = st.doc.artboards.iter().position(|a| Some(a.id) == g.artboard) {
            r["artboard"] = json!(ab);
        }
        if let Some(l) = g.layer {
            r["layer"] = json!(l.0);
        }
        r["hidden"] = json!(g.hidden);
        r["locked"] = json!(g.locked);
        r["shown"] = json!(st.doc.guide_shown(g));
        r["editable"] = json!(st.doc.guide_editable(g));
        r
    };
    Ok(Value::Array(st.doc.ruler_guides().iter().enumerate().map(row).collect()))
}

/// The id of ruler guide `index` in `ids` ([`guide_ids`]; `index` validated).
fn guide_index_in(ids: &[NodeId], v: Option<&Value>, cmd: &str) -> Result<NodeId> {
    let i = v.and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, "an index must be a whole number"))?;
    ids.get(i).copied().ok_or_else(|| bad(cmd, format!("no guide {i}")))
}

/// The most guide indexes one `indexes` list may hold.
const MAX_GUIDE_INDEXES: usize = 10_000;

/// `indexes` as a list of at most [`MAX_GUIDE_INDEXES`] entries.
fn guide_index_list<'a>(v: &'a Value, cmd: &str) -> Result<&'a Vec<Value>> {
    let list = v.as_array().ok_or_else(|| bad(cmd, "indexes must be an array of guide indexes"))?;
    if list.len() > MAX_GUIDE_INDEXES {
        return Err(bad(cmd, format!("indexes may hold at most {MAX_GUIDE_INDEXES} entries")));
    }
    Ok(list)
}

/// The id of ruler guide `index` of the active document (`index` validated).
fn guide_index(s: &Session, v: Option<&Value>, cmd: &str) -> Result<NodeId> {
    guide_index_in(&guide_ids(&s.doc()?.doc), v, cmd)
}

/// Is ruler guide `id` one that can be moved or deleted: not hidden or locked itself, nor on a
/// hidden or locked layer?
fn check_editable_guide(s: &Session, id: NodeId) -> Result<NodeId> {
    let d = &s.doc()?.doc;
    let guide = d.ruler_guide(id);
    if guide.as_ref().is_some_and(|g| g.hidden || g.locked) {
        return Err(EngineError::Other("the guide is hidden or locked".into()));
    }
    if !guide.is_some_and(|g| d.guide_editable(&g)) {
        return Err(EngineError::Other("the guide is on a hidden or locked layer".into()));
    }
    Ok(id)
}

/// [`guide_index`] of a guide that can be moved or deleted ([`check_editable_guide`]).
fn editable_guide_index(s: &Session, v: Option<&Value>, cmd: &str) -> Result<NodeId> {
    check_editable_guide(s, guide_index(s, v, cmd)?)
}

fn guide_select(s: &mut Session, p: &Value) -> Result<Value> {
    let list = guide_index_list(p.get("indexes").ok_or_else(|| bad("guide.select", "missing indexes"))?, "guide.select")?;
    let ids = guide_ids(&s.doc()?.doc);
    let picked = list.iter().map(|v| guide_index_in(&ids, Some(v), "guide.select")).collect::<Result<Vec<_>>>()?;
    if switch_or(p, "toggle", false, "guide.select")? {
        // Added to (or taken from) what is selected, the art included.
        s.select(|_, sel| {
            for i in picked {
                sel.toggle_guide(i);
            }
        })?;
    } else {
        s.select(|_, sel| sel.set_guides(picked))?;
    }
    let st = s.doc()?;
    Ok(json!({ "selected": guide_indexes(&st.doc, &st.selection.guides) }))
}

/// Delete ruler guide `index` (or guides `indexes`), else the selected guides, in one undo step
/// → {count}.
pub(crate) fn guide_remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "guide.remove";
    let mut gone: Vec<NodeId> = match (p.get("index"), p.get("indexes")) {
        (Some(_), Some(_)) => return Err(bad(C, "give index or indexes, not both")),
        (Some(v), None) => vec![editable_guide_index(s, Some(v), C)?],
        (None, Some(list)) => {
            let list = guide_index_list(list, C)?;
            let ids = guide_ids(&s.doc()?.doc);
            // A set, so an index given twice deletes its guide once and the ids held never outnumber the guides.
            let set = list.iter().map(|v| check_editable_guide(s, guide_index_in(&ids, Some(v), C)?)).collect::<Result<BTreeSet<_>>>()?;
            set.into_iter().collect()
        }
        (None, None) => s.doc()?.selection.guides.clone(),
    };
    gone.sort_unstable();
    gone.dedup();
    if gone.is_empty() {
        return Err(bad(C, "no guide selected"));
    }
    let label = if gone.len() == 1 { "Delete Guide" } else { "Delete Guides" };
    s.edit(label, |d, sel| {
        d.retain_guides(sel, |g| !g.id.is_some_and(|id| gone.binary_search(&id).is_ok()));
        Ok(())
    })?;
    Ok(json!({ "count": gone.len() }))
}

/// Put ruler guide `index`, else the selected guides, on layer `layer`, in one undo step.
fn guide_set_layer(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "guide.setLayer";
    let d = &s.doc()?.doc;
    let layer =
        id_param(p, "layer").filter(|l| d.node(*l).is_some_and(Node::is_layer)).ok_or_else(|| bad(C, "layer must name a layer or sublayer"))?;
    // As objects dragged in the Layers panel: never onto a locked layer.
    if super::layer::locked_within(d, layer) {
        return Err(EngineError::Other("the layer or group is locked".into()));
    }
    let mut moving: Vec<NodeId> = match (p.get("index"), p.get("indexes")) {
        (Some(_), Some(_)) => return Err(bad(C, "give index or indexes, not both")),
        (Some(v), None) => vec![editable_guide_index(s, Some(v), C)?],
        (None, Some(list)) => {
            let list = guide_index_list(list, C)?;
            let ids = guide_ids(&s.doc()?.doc);
            list.iter().map(|v| check_editable_guide(s, guide_index_in(&ids, Some(v), C)?)).collect::<Result<Vec<_>>>()?
        }
        (None, None) => s.doc()?.selection.guides.clone(),
    };
    // An index given twice moves (and counts) its guide once; the others keep their order.
    let mut seen = HashSet::new();
    moving.retain(|id| seen.insert(*id));
    if moving.is_empty() {
        return Err(bad(C, "no guide selected (or give an index)"));
    }
    s.edit("Move Guide to Layer", |d, _| {
        // Each keeps its id (and so its index), at the top of the layer.
        for id in &moving {
            if d.parent_of(*id) != Some(layer) && d.ruler_guide(*id).is_some() {
                d.move_node(*id, Some(layer), usize::MAX)?;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "count": moving.len() }))
}

/// Put ruler guide `index` at `pos`, else move (or copy) the selected guides, in one undo step.
pub(crate) fn guide_move(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(v) = p.get("index") {
        let i = editable_guide_index(s, Some(v), "guide.move")?;
        let pos = f64_req(p, "pos", "guide.move")?;
        s.edit("Move Guide", |d, _| {
            d.edit_ruler_guide(i, |g| g.pos = pos);
            Ok(())
        })?;
        return ok();
    }
    let moving = s.doc()?.selection.guides.clone();
    if moving.is_empty() {
        return Err(bad("guide.move", "no guide selected (or give an index)"));
    }
    let dx = number_or(p, "dx", 0.0, "guide.move")?;
    let dy = number_or(p, "dy", 0.0, "guide.move")?;
    let copy = switch_or(p, "copy", false, "guide.move")?;
    if !s.doc()?.selection.is_empty() {
        let ids = super::edit::selected_roots(s)?;
        return super::object::apply_transform(
            s,
            if copy { "Copy Guide" } else { "Move Guide" },
            ids,
            vectorcraft_geom::Affine::translate((dx, dy)),
            p,
            true,
        );
    }
    s.edit(if copy { "Copy Guide" } else { "Move Guide" }, |d, sel| {
        let mut copies = vec![];
        for &i in &moving {
            if copy {
                copies.extend(d.ruler_guide(i).map(|g| d.add_ruler_guide(&g.moved(Vec2::new(dx, dy)))));
            } else {
                d.edit_ruler_guide(i, |g| *g = g.moved(Vec2::new(dx, dy)));
            }
        }
        if copy {
            sel.set_guides(copies);
        }
        Ok(())
    })?;
    ok()
}

// ---------- File ----------

fn close_all(s: &mut Session, _: &Value) -> Result<Value> {
    let n = s.documents().len();
    while !s.documents().is_empty() {
        s.close_document(s.documents().len() - 1);
    }
    Ok(json!({ "closed": n }))
}
