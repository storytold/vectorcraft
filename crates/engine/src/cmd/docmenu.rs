//! Select menu long tail (Same → text/symbol attributes, Object → Direction Handles / text kinds /
//! brush strokes, Save/recall selections), View → Guides, and File → Close All / Document Color
//! Mode / File Info.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, Guide, Node, NodeId, NodeKind, SavedSelection, TextKind};
use vectorcraft_geom::Vec2;

use super::edit::selected_roots;
use super::pathops::Beside;
use super::select::{selectable_ids, walk_selectable};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("select.same.symbolInstance", "Symbol Instance", ["Select", "Same"], None, "{} → {count}", has_selection, |s, _| same(
            s,
            "select.same.symbolInstance",
            |a, b| match (&a.kind, &b.kind) {
                (NodeKind::SymbolInstance { symbol: x, .. }, NodeKind::SymbolInstance { symbol: y, .. }) => x == y,
                _ => false,
            }
        )),
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
            "{locked?: bool} toggle (or set) the session's guide lock; locking deselects the ruler guides and guide paths in every open document → {locked}",
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
            "{} → [{index, vertical, pos, selected, artboard?: index (an artboard guide's), layer?: id (the layer it is on), shown, editable (its layer shown and unlocked)}…] the ruler guides",
            has_doc,
            guide_list
        ),
        cmd!(
            "guide.select",
            "Select Guides",
            [],
            None,
            "{indexes: [index…], toggle?: bool} select ruler guides on their own (deselecting the art); toggle adds or removes them instead, keeping the selected art (a Shift-click on a guide) → {selected: [index…]}",
            guides_unlocked,
            guide_select
        ),
        cmd!(
            "guide.remove",
            "Remove Guide",
            [],
            None,
            "{index?} delete ruler guide `index`, else the selected ones (as Delete / edit.clear does) → {count}",
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
            "{index?, indexes?: [index…], layer: id} put ruler guide `index` (or guides `indexes`), else the selected guides, on layer `layer` (a layer or sublayer), as dragging their rows onto that layer's row in the Layers panel does → {count}",
            guides_unlocked,
            guide_set_layer
        ),
        cmd!("file.closeAll", "Close All", ["File"], Some("Cmd+Alt+W"), "{} → {closed}", has_doc, close_all),
        cmd!(
            "file.documentColorMode",
            "Document Color Mode",
            ["File", "Document Color Mode"],
            None,
            "{mode: \"cmyk\"|\"rgb\", convert?: true (convert every colour of the art, symbols, pattern tiles and swatches through the colour settings; swatch links kept; Gray colours stay Gray, on the black plate), intent?, grays?: \"profile\" (default: RGB greys separate through the CMYK profile like any colour, into four-colour greys and a rich black) | \"black\" (to CMYK, RGB greys with R = G = B go on the black plate only, K = their grey value)} → {changed}",
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

fn same(s: &mut Session, cmd: &str, eq: fn(&Node, &Node) -> bool) -> Result<Value> {
    let st = s.doc()?;
    let r = st.selection.objects.first().and_then(|id| st.doc.node(*id)).cloned().ok_or_else(|| bad(cmd, "nothing selected"))?;
    let ids = candidates(s, |n| eq(n, &r))?;
    finish_same(s, cmd, ids)
}

fn first_text_style(s: &Session) -> Option<CharStyle> {
    let st = s.active()?;
    let mut out = None;
    for id in &st.selection.objects {
        st.doc.node(*id)?.walk(&mut |n| {
            if out.is_none()
                && let NodeKind::Text(t) = &n.kind
            {
                out = Some(t.first_style());
            }
        });
        if out.is_some() {
            break;
        }
    }
    out
}

fn same_text(s: &mut Session, cmd: &str, eq: fn(&CharStyle, &CharStyle) -> bool) -> Result<Value> {
    let r = first_text_style(s).ok_or_else(|| bad(cmd, "select a text object"))?;
    let ids = candidates(s, |n| match &n.kind {
        NodeKind::Text(t) => t.runs.iter().any(|run| eq(&run.style, &r)),
        _ => false,
    })?;
    finish_same(s, cmd, ids)
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
    let name = str_param(p, "name").and_then(SavedSelection::clean_name).unwrap_or_else(|| SavedSelection::default_name(saved));
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
        let Some(slot) = after.get_mut(i) else { continue };
        if bool_or(e, "delete", false) {
            *slot = None;
        } else if let Some(n) = str_param(e, "newName").and_then(SavedSelection::clean_name)
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
    let (rulers, paths): (Vec<usize>, Vec<NodeId>) = if index.is_some() || ids.is_some() {
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
        let rulers: Vec<usize> = if s.menu.guides_locked {
            vec![]
        } else {
            st.selection.guides.iter().copied().filter(|i| st.doc.guides.get(*i).is_some_and(|g| st.doc.guide_editable(g))).collect()
        };
        let editable = st.doc.editable();
        if sel.is_empty() && rulers.is_empty() { (vec![], all.into_iter().filter(|id| editable(*id)).collect()) } else { (rulers, sel) }
    };
    // Find each ruler guide's line and its parent: the guide's layer, else where new art goes.
    let st = s.doc()?;
    let artboards = st.doc.artboards.iter().map(|a| a.rect).reduce(|a, b| a.union(b));
    let mut lines = Vec::with_capacity(rulers.len());
    for &i in &rulers {
        let Some(g) = st.doc.guides.get(i) else { continue };
        let span = st.doc.guide_span(g).or_else(|| artboards.map(|r| if g.vertical { (r.y0, r.y1) } else { (r.x0, r.x1) }));
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
        d.retain_guides(sel, |i, _| !lines.iter().any(|l| l.0 == i));
        sel.set(paths.iter().copied().chain(made));
        Ok(())
    })?;
    Ok(json!({ "count": n }))
}

fn lock_guides(s: &mut Session, p: &Value) -> Result<Value> {
    let v = p.get("locked").and_then(Value::as_bool).unwrap_or(!s.menu.guides_locked);
    s.menu.guides_locked = v;
    // Locked guides can't stay selected, ruler guides or guide paths, in any open document
    // ([`Session::select`] lets those go in the active one).
    let st = s.doc()?;
    if v && (!st.selection.guides.is_empty() || st.selection.objects.iter().any(|id| crate::is_guide_path(&st.doc, *id))) {
        s.select(|_, sel| sel.guides.clear())?;
    }
    if v {
        let active = s.active;
        for (i, st) in s.docs.iter_mut().enumerate() {
            if Some(i) != active && crate::deselect_hidden(&s.menu, &st.doc, &mut st.selection) {
                st.revision += 1;
            }
        }
    }
    Ok(json!({ "locked": v }))
}

fn clear_guides(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let paths = guide_paths(&st.doc);
    let n = paths.len() + st.doc.guides.len();
    if n == 0 {
        return Ok(json!({ "count": 0 }));
    }
    s.edit("Clear Guides", |d, _| {
        d.guides.clear();
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
    let vertical = bool_or(p, "vertical", false);
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
    let i = s.edit("New Guide", |d, _| {
        d.guides.push(Guide { artboard, layer, ..Guide::new(vertical, pos) });
        Ok(d.guides.len() - 1)
    })?;
    Ok(json!({ "index": i }))
}

fn guide_list(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let row = |(i, g): (usize, &Guide)| {
        let mut r = json!({ "index": i, "vertical": g.vertical, "pos": g.pos, "selected": st.selection.guides.contains(&i) });
        if let Some(ab) = st.doc.artboards.iter().position(|a| Some(a.id) == g.artboard) {
            r["artboard"] = json!(ab);
        }
        if let Some(l) = g.layer {
            r["layer"] = json!(l.0);
        }
        r["shown"] = json!(st.doc.guide_shown(g));
        r["editable"] = json!(st.doc.guide_editable(g));
        r
    };
    Ok(Value::Array(st.doc.guides.iter().enumerate().map(row).collect()))
}

/// Ruler guide `index` of the active document (`index` validated).
fn guide_index(s: &Session, v: Option<&Value>, cmd: &str) -> Result<usize> {
    let i = v.and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, "an index must be a whole number"))?;
    if i >= s.doc()?.doc.guides.len() {
        return Err(bad(cmd, format!("no guide {i}")));
    }
    Ok(i)
}

/// [`guide_index`] of a guide that can be moved or deleted: not on a hidden or locked layer.
fn editable_guide_index(s: &Session, v: Option<&Value>, cmd: &str) -> Result<usize> {
    let i = guide_index(s, v, cmd)?;
    let d = &s.doc()?.doc;
    if !d.guides.get(i).is_some_and(|g| d.guide_editable(g)) {
        return Err(EngineError::Other("the guide is on a hidden or locked layer".into()));
    }
    Ok(i)
}

fn guide_select(s: &mut Session, p: &Value) -> Result<Value> {
    let list = p.get("indexes").and_then(Value::as_array).ok_or_else(|| bad("guide.select", "missing indexes"))?;
    let picked = list.iter().map(|v| guide_index(s, Some(v), "guide.select")).collect::<Result<Vec<_>>>()?;
    if bool_or(p, "toggle", false) {
        // Added to (or taken from) what is selected, the art included.
        s.select(|_, sel| {
            for i in picked {
                sel.toggle_guide(i);
            }
        })?;
    } else {
        s.select(|_, sel| sel.set_guides(picked))?;
    }
    Ok(json!({ "selected": s.doc()?.selection.guides }))
}

/// Delete ruler guide `index`, else the selected guides, in one undo step → {count}.
pub(crate) fn guide_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let gone: Vec<usize> = match p.get("index") {
        Some(v) => vec![editable_guide_index(s, Some(v), "guide.remove")?],
        None => s.doc()?.selection.guides.clone(),
    };
    if gone.is_empty() {
        return Err(bad("guide.remove", "no guide selected"));
    }
    let label = if gone.len() == 1 { "Delete Guide" } else { "Delete Guides" };
    s.edit(label, |d, sel| {
        d.retain_guides(sel, |i, _| !gone.contains(&i));
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
    let moving: Vec<usize> = match (p.get("index"), p.get("indexes").and_then(Value::as_array)) {
        (Some(v), _) => vec![editable_guide_index(s, Some(v), C)?],
        (None, Some(list)) => list.iter().take(10_000).map(|v| editable_guide_index(s, Some(v), C)).collect::<Result<Vec<_>>>()?,
        (None, None) => s.doc()?.selection.guides.clone(),
    };
    if moving.is_empty() {
        return Err(bad(C, "no guide selected (or give an index)"));
    }
    s.edit("Move Guide to Layer", |d, _| {
        for i in &moving {
            if let Some(g) = d.guides.get_mut(*i) {
                g.layer = Some(layer);
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
            if let Some(g) = d.guides.get_mut(i) {
                g.pos = pos;
            }
            Ok(())
        })?;
        return ok();
    }
    let moving = s.doc()?.selection.guides.clone();
    if moving.is_empty() {
        return Err(bad("guide.move", "no guide selected (or give an index)"));
    }
    let (dx, dy, copy) = (f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0), bool_or(p, "copy", false));
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
            let Some(g) = d.guides.get_mut(i) else { continue };
            let moved = g.moved(Vec2::new(dx, dy));
            if copy {
                copies.push(moved);
            } else {
                *g = moved;
            }
        }
        if copy {
            let first = d.guides.len();
            d.guides.extend(copies);
            sel.set_guides(first..d.guides.len());
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
