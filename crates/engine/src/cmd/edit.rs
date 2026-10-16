//! File (document-level) and Edit commands, plus document queries.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, Unit};
use vectorcraft_geom::{Affine, Point, Vec2};

use super::clipboard::SwatchChoices;
use super::*;
use crate::{Clipboard, EngineError, inspect};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.new",
            "New…",
            ["File"],
            None,
            "{preset?: a name from file.newPresets (start from it; the other params change it), name?|title?: document name (default Untitled-N), width?: pt=612|\"210 mm\", height?: pt=792, units?: \"Pixels\"|\"Points\"|\"Picas\"|\"Inches\"|\"Millimeters\"|\"Centimeters\"|\"Feet\"|\"Yards\"|\"Meters\"|\"Feet & Inches\" (default: prefs unitsGeneral, as print presets; screen presets: Pixels), orientation?: \"portrait\"|\"landscape\" (swaps width and height to match), artboards?: n (1–1000), artboardLayout?: {layout?: \"gridByRow\"|\"gridByColumn\"|\"row\"|\"column\", columns?: n (default: all in one row), spacing?: pt=20, rightToLeft?: bool}, bleed?: pt|[top, bottom, left, right]|{top?, …} (0–72 pt), backgroundContents?: \"transparent\"|\"white\" (a white artboard background, not an object), colorMode?: \"rgb\"|\"cmyk\" (a CMYK document starts with CMYK swatches and stores the colours applied to it as CMYK), rasterEffectsPpi?: 72|150|300 (1–2400), previewMode?: \"default\"|\"pixel\"|\"overprint\" (overprint turns Overprint Preview on, pixel Pixel Preview in the app; default leaves both), created?: Unix seconds|null (File Info's created date; default now, recorded in the journal so a replay matches), size?: a preset's size label (a preset file.newPresets lists can be passed back; ignored)} → {index, previewMode}; the size is remembered in file.newPresets' Recent",
            always,
            super::newdoc::file_new
        ),
        cmd!("file.close", "Close", ["File"], Some("Cmd+W"), "{index?}", has_doc, file_close),
        cmd!("document.activate", "Activate Document", [], None, "{index}", always, doc_activate),
        cmd!(query "document.inspect", "Inspect Document", [], None, "{depth?, childLimit?} → layer tree (each node: id, name, kind, visible, locked, bounds, opacity when below 1, blend when not Normal, fill, stroke and strokeWidth of objects and compound paths; a compound path's members have compoundMember: true instead of paint, which is the compound's), artboards, selection, history (a sliced tree reports childCount)", has_doc, |s, p| Ok(inspect::document_opts(s, slice_opts(p, "document.inspect")?))),
        cmd!(query "document.node", "Inspect Object", [], None, "{id, summary?: compact summary, depth?: child levels in the summary (default all), childLimit?: children shown per node (default all; a level that shows fewer reports childCount)} → one object", has_doc, doc_node),
        cmd!(query "document.find", "Find Objects", [], None, "{name?: substring of the Layers panel name, kind?: panel label such as Group; Path; Type or Image (exact), text?: substring of type content, limit?: max matches (default 100, 0 counts only)} → {matches: [{id, name, kind, path}], total}", has_doc, doc_find),
        cmd!(query "document.json", "Document JSON", [], None, "{} → complete document model", has_doc, |s, _| Ok(serde_json::to_value(&*s.doc()?.doc).unwrap_or(Value::Null))),
        cmd!(
            "document.setUnits",
            "Units",
            ["File", "Document Setup"],
            None,
            "{units: \"Points\"|\"Picas\"|\"Inches\"|\"Millimeters\"|\"Centimeters\"|\"Pixels\"|\"Feet & Inches\"|\"Meters\"|\"Yards\"|\"Feet\"} the document's units: every length the UI shows and reads (General)",
            has_doc,
            set_units
        ),
        cmd!(query "document.rulerOrigin", "Ruler Origin", [], None, "{} → {x, y} the rulers' zero point, in absolute document points", has_doc, |s, _| Ok(json!(s.doc()?.doc.ruler_origin))),
        cmd!(
            "document.setRulerOrigin",
            "Ruler Origin",
            [],
            None,
            "{x, y} set the rulers' zero point in absolute document points; art, guides and command coordinates stay in place",
            has_doc,
            set_ruler_origin
        ),
        cmd!(
            "document.resetRulerOrigin",
            "Reset Ruler Origin",
            [],
            None,
            "{artboard?: index=0} reset the rulers' zero point to that artboard's top-left corner",
            has_doc,
            reset_ruler_origin
        ),
        cmd!("edit.undo", "Undo", ["Edit"], Some("Cmd+Z"), "{}", can_undo, undo),
        cmd!("edit.redo", "Redo", ["Edit"], Some("Cmd+Shift+Z"), "{}", can_redo, redo),
        cmd!(
            "edit.undoGroup.begin",
            "Begin Undo Group",
            [],
            None,
            "{label?: string} until edit.undoGroup.end, the edits made in the active document are one undo step, named `label` (else after its first edit): for automation, such as MCP command_batch with `atomic` (#1163)",
            has_doc,
            undo_group_begin
        ),
        cmd!(
            "edit.undoGroup.end",
            "End Undo Group",
            [],
            None,
            "{cancel?: bool} close the undo group: its edits stay one undo step or, with `cancel`, are undone",
            has_doc,
            undo_group_end
        ),
        cmd!("edit.cut", "Cut", ["Edit"], Some("Cmd+X"), "{} (the Artboard tool chosen: artboard.cut)", has_selection_or_artboard_tool, cut),
        cmd!("edit.copy", "Copy", ["Edit"], Some("Cmd+C"), "{} (the Artboard tool chosen: artboard.copy)", has_selection_or_artboard_tool, copy),
        cmd!(
            "edit.paste",
            "Paste",
            ["Edit"],
            Some("Cmd+V"),
            "{center?: [x, y], dx?, dy?, swatchConflict?} paste centred on `center` (the app passes the view centre), else offset by dx/dy (default: the Paste Offset preference). Pasting brings the image blobs, symbols, patterns, global and spot swatches (with the tint swatches of the tints used), gradient swatches, graphic styles, character and paragraph styles and brushes the objects use; one of the same name that differs comes in renamed. swatchConflict, for a swatch whose name the document gives another colour (clipboard.conflicts): \"merge\" (default: the objects take the document's swatch) | \"add\" (the pasted swatch comes in renamed) | {name: \"merge\"|\"add\"}. With Paste Remembers Layers on (layer.pasteRemembersLayers), objects go back into the layers they came from (by name; made when missing). An artboard copied with artboard.copy comes with its art, right of the last artboard (the other paste commands but Paste on All Artboards: where it was), its art back in the layers it came from → {ids, artboard: the new artboard's index or null, added: resources added, merged: conflicts merged, renamed: [{kind, from, to}]}",
            has_clipboard,
            |s, p| paste(s, p, PasteMode::Offset)
        ),
        cmd!(
            "edit.pasteInFront",
            "Paste in Front",
            ["Edit"],
            Some("Cmd+F"),
            "{swatchConflict?, artboard?: index (the active artboard: the objects go where they were on the artboard they were copied from, on this one)} paste in place just above the top selected object, or on top of the current layer when nothing is selected (resources and layers as edit.paste) → {ids, added, merged, renamed}",
            has_clipboard,
            |s, p| paste(s, p, PasteMode::Front)
        ),
        cmd!(
            "edit.pasteInBack",
            "Paste in Back",
            ["Edit"],
            Some("Cmd+B"),
            "{swatchConflict?, artboard?: index (as edit.pasteInFront)} paste in place just below the bottom selected object, or at the bottom of the current layer when nothing is selected (resources and layers as edit.paste) → {ids, added, merged, renamed}",
            has_clipboard,
            |s, p| paste(s, p, PasteMode::Back)
        ),
        cmd!(
            "edit.pasteInPlace",
            "Paste in Place",
            ["Edit"],
            Some("Cmd+Shift+V"),
            "{swatchConflict?, artboard?: index (as edit.pasteInFront)} paste where the objects were copied (resources and layers as edit.paste) → {ids, added, merged, renamed}",
            has_clipboard,
            |s, p| paste(s, p, PasteMode::InPlace)
        ),
        cmd!(
            "edit.pasteOnAllArtboards",
            "Paste on All Artboards",
            ["Edit"],
            Some("Cmd+Alt+Shift+V"),
            "{swatchConflict?} paste a copy on every artboard at the offset the objects had to the artboard they were copied from (resources and layers as edit.paste) → {ids, added, merged, renamed}",
            has_clipboard,
            |s, p| paste(s, p, PasteMode::AllArtboards)
        ),
        cmd!(
            "edit.clear",
            "Clear",
            ["Edit"],
            Some("Delete"),
            "{ids?} delete the objects `ids`, else the selection (fully selected objects and direct-selected anchors together, else ruler guides)",
            has_selection_or_guides,
            clear
        ),
        cmd!(
            "edit.duplicate",
            "Duplicate",
            ["Edit"],
            None,
            "{ids?: [id…] (selected first; default: the selection), dx?, dy?} duplicate the selection in place (offset optional)",
            has_selection,
            duplicate
        ),
    ]
}

fn file_close(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).map(|v| v as usize).or(s.active_index()).ok_or(EngineError::NoDocument)?;
    if !s.close_document(i) {
        return Err(bad("file.close", "no such document"));
    }
    ok()
}

fn doc_activate(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad("document.activate", "missing index"))? as usize;
    if s.set_active(i) { ok() } else { Err(bad("document.activate", "no such document")) }
}

fn doc_node(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("document.node", "missing id"))?;
    let mut opts = slice_opts(p, "document.node")?;
    let summary = bool_or(p, "summary", false);
    if !summary && (opts.depth.is_some() || opts.child_limit.is_some()) {
        // The full object JSON is never truncated: a slice without `summary` would
        // otherwise silently return the whole subtree.
        return Err(bad("document.node", "`depth` and `childLimit` slice the summary: pass `summary: true`"));
    }
    let doc = &s.doc()?.doc;
    let n = doc.node(id).ok_or(EngineError::NoNode(id))?;
    opts.compound_member = doc.parent_of(id).and_then(|p| doc.node(p)).is_some_and(|p| matches!(p.kind, vectorcraft_doc::NodeKind::Compound { .. }));
    if summary { Ok(inspect::node_summary_opts(n, opts, &doc.symbol_boxes())) } else { Ok(serde_json::to_value(n).unwrap_or(Value::Null)) }
}

/// `depth` / `childLimit` for a summary read: absent (or null) means no limit.
fn slice_opts(p: &Value, cmd: &str) -> Result<inspect::SummaryOpts> {
    Ok(inspect::SummaryOpts { depth: opt_u64(p, cmd, "depth")?, child_limit: opt_u64(p, cmd, "childLimit")?, compound_member: false })
}

/// An optional count: absent (or null) is `None`; a present value that is not a
/// non-negative integer is rejected rather than silently reinterpreted.
fn opt_u64(p: &Value, cmd: &str, key: &str) -> Result<Option<u64>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be a non-negative integer"))),
    }
}

fn doc_find(s: &mut Session, p: &Value) -> Result<Value> {
    // A filter of the wrong type is an error, not dropped: dropping it would widen the search.
    let needle = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.to_lowercase()).filter(|v| !v.is_empty())),
        Some(_) => Err(bad("document.find", format!("`{key}` must be a string"))),
    };
    let filter = inspect::FindFilter { name: needle("name")?, kind: needle("kind")?, text: needle("text")? };
    if filter.is_empty() {
        return Err(bad("document.find", "give at least one of `name`, `kind`, `text`"));
    }
    let limit = opt_u64(p, "document.find", "limit")?.unwrap_or(100);
    let (matches, total) = inspect::find_nodes(&s.doc()?.doc, &filter, limit);
    Ok(json!({"matches": matches, "total": total}))
}

/// Document Setup's units (also `document.setup {units}`).
fn set_units(s: &mut Session, p: &Value) -> Result<Value> {
    let u = str_param(p, "units").and_then(Unit::named).ok_or_else(|| bad("document.setUnits", "unknown units"))?;
    s.set_document_units(u)?;
    ok()
}

fn set_ruler_origin(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "document.setRulerOrigin";
    let origin = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    change_ruler_origin(s, origin, "Ruler Origin")
}

fn reset_ruler_origin(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "document.resetRulerOrigin";
    let index = match p.get("artboard") {
        None => 0,
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(C, "index out of range"))?,
    };
    let ab = s.doc()?.doc.artboards.get(index).ok_or_else(|| bad(C, "index out of range"))?;
    change_ruler_origin(s, Point::new(ab.rect.x0, ab.rect.y0), "Reset Ruler Origin")
}

fn change_ruler_origin(s: &mut Session, origin: Point, label: &str) -> Result<Value> {
    if !origin.is_finite() || origin.x.abs() > crate::MAX_COORD || origin.y.abs() > crate::MAX_COORD {
        return Err(EngineError::Other("result would exceed the canvas (coordinates out of range)".into()));
    }
    if s.doc()?.doc.ruler_origin != origin {
        s.edit(label, |d, _| {
            d.ruler_origin = origin;
            Ok(())
        })?;
    }
    Ok(json!(origin))
}

/// A typing session in progress (the Type tool previews the whole session as one interaction).
pub(crate) fn typing_in_progress(s: &Session) -> bool {
    s.active().and_then(|d| d.interaction.as_ref()).is_some_and(|it| it.label == "Typing" && it.preview.is_some())
}

fn undo_group_begin(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "edit.undoGroup.begin";
    let label = match p.get("label") {
        None | Some(Value::Null) => None,
        Some(Value::String(l)) if !l.trim().is_empty() && l.chars().count() <= 200 => Some(l.trim().to_string()),
        Some(_) => return Err(bad(C, "`label` must be a non-empty name of at most 200 characters")),
    };
    s.begin_named_undo_group(label);
    ok()
}

fn undo_group_end(s: &mut Session, p: &Value) -> Result<Value> {
    s.end_undo_group(bool_or(p, "cancel", false));
    ok()
}

/// What is still in progress (typing, a drag) is a step of its own: keep it before the history
/// moves, so Undo takes back only that and Redo brings it back, instead of dropping it and undoing
/// the step before. As any new edit it ends the steps to redo, so Redo never swaps the document
/// under an interaction that would then preview on, or cancel to, the document from before (#1719).
/// A typing session also ends in the Type tool, whose caret and marked text (IME) must follow the
/// document.
pub(crate) fn settle_interaction(s: &mut Session) -> Result<()> {
    let typing = typing_in_progress(s);
    s.commit_interaction()?;
    if typing {
        s.set_tool_option("endTyping", &Value::Bool(true));
    }
    Ok(())
}

fn undo(s: &mut Session, _: &Value) -> Result<Value> {
    settle_interaction(s)?;
    let st = s.doc_mut()?;
    let e = st.history.undo.pop().ok_or_else(|| EngineError::Other("nothing to undo".into()))?;
    let label = e.label.clone();
    let costs = e.costs();
    let editing = st.doc.mask_edit.map(|m| m.layer);
    let doc = std::mem::replace(&mut st.doc, e.doc);
    let selection = std::mem::replace(&mut st.selection, Arc::unwrap_or_clone(e.selection));
    let selection = shared_selection(selection, st.history.redo.last());
    st.history.redo.push(crate::HistoryEntry::turned(e.label, doc, selection, &st.doc, costs));
    st.history.trim();
    st.revision += 1;
    st.selection.prune(&st.doc);
    super::maskedit::follow_history(st, editing);
    Ok(json!({ "undone": label }))
}

fn redo(s: &mut Session, _: &Value) -> Result<Value> {
    settle_interaction(s)?;
    let st = s.doc_mut()?;
    let e = st.history.redo.pop().ok_or_else(|| EngineError::Other("nothing to redo".into()))?;
    let label = e.label.clone();
    let costs = e.costs();
    let editing = st.doc.mask_edit.map(|m| m.layer);
    let doc = std::mem::replace(&mut st.doc, e.doc);
    let selection = std::mem::replace(&mut st.selection, Arc::unwrap_or_clone(e.selection));
    let selection = shared_selection(selection, st.history.undo.last());
    st.history.undo.push(crate::HistoryEntry::turned(e.label, doc, selection, &st.doc, costs));
    st.history.trim();
    st.revision += 1;
    st.selection.prune(&st.doc);
    super::maskedit::follow_history(st, editing);
    Ok(json!({ "redone": label }))
}

/// `selection` for an undo (redo) step, sharing that of `like`, the step beside it, when the same.
fn shared_selection(selection: vectorcraft_doc::Selection, like: Option<&crate::HistoryEntry>) -> Arc<vectorcraft_doc::Selection> {
    match like {
        Some(l) if *l.selection == selection => l.selection.clone(),
        _ => Arc::new(selection),
    }
}

/// Selected top-level objects in paint order, dropping any whose ancestor is also selected.
pub(crate) fn selected_roots(s: &Session) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    Ok(roots_of(&st.doc, st.selection.in_paint_order(&st.doc)))
}

/// [`selected_roots`] for the commands that move or reshape the art (Move, Transform, Rotate…): in
/// an isolated compound path its paths are objects of their own, so the hole of a ring moves
/// without the ring. Paint and the rest still go to the compound path, which holds the paint.
pub(crate) fn selected_roots_to_transform(s: &Session) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    Ok(roots_within(&st.doc, st.selection.in_paint_order(&st.doc), st.isolation))
}

/// The top-level objects among `ids` (in paint order): compound-path members stand for their
/// compound, and ids whose ancestor is also listed, and layers, are dropped.
pub(crate) fn roots_of(doc: &Document, ids: Vec<NodeId>) -> Vec<NodeId> {
    roots_within(doc, ids, None)
}

/// [`roots_of`], less the swap for the members of `isolation` when it is a compound path.
fn roots_within(doc: &Document, ids: Vec<NodeId>, isolation: Option<NodeId>) -> Vec<NodeId> {
    // Every id's place in the tree, found in one walk (a lookup per id is a walk per id).
    let paths = doc.index_paths(ids.iter().copied());
    // A compound-path member acts as its compound (compounds are one object).
    let mut sel: Vec<(NodeId, Option<Vec<usize>>)> = ids
        .into_iter()
        .map(|id| {
            let path = paths.get(&id);
            let parent = path.and_then(|p| p.split_last()).map(|(_, up)| up).filter(|up| !up.is_empty());
            match parent.and_then(|up| doc.node_at(up).map(|n| (up, n))) {
                Some((up, n)) if matches!(n.kind, vectorcraft_doc::NodeKind::Compound { .. }) && Some(n.id) != isolation => (n.id, Some(up.to_vec())),
                _ => (id, path.cloned()),
            }
        })
        .collect();
    sel.dedup_by_key(|(id, _)| *id);
    without_inner(doc, sel)
        .filter(|(_, path)| path.as_deref().and_then(|p| doc.node_at(p)).is_some_and(|n| !n.is_layer()))
        .map(|(id, _)| id)
        .collect()
}

/// `ids` in the order given, less those whose ancestor is also listed: an object listed with a
/// group that holds it is dropped, as [`roots_of`] drops it from a selection. Nothing else is
/// dropped or replaced: unlike `roots_of`, it keeps layers, ids that aren't in the document and
/// compound-path members.
pub(crate) fn outermost(doc: &Document, ids: Vec<NodeId>) -> Vec<NodeId> {
    let paths = doc.index_paths(ids.iter().copied());
    let placed = ids.into_iter().map(|id| (id, paths.get(&id).cloned())).collect();
    without_inner(doc, placed).map(|(id, _)| id).collect()
}

/// The entries of `placed` (ids with their index paths) whose id has no ancestor among its ids.
fn without_inner(doc: &Document, placed: Vec<(NodeId, Option<Vec<usize>>)>) -> impl Iterator<Item = (NodeId, Option<Vec<usize>>)> + '_ {
    let listed: std::collections::HashSet<NodeId> = placed.iter().map(|(id, _)| *id).collect();
    placed.into_iter().filter(move |(_, path)| {
        let anc = path.as_deref().and_then(|p| doc.ancestry_at(p)).unwrap_or_default();
        !anc[..anc.len().saturating_sub(1)].iter().any(|a| listed.contains(a))
    })
}

fn copy(s: &mut Session, _: &Value) -> Result<Value> {
    // The Artboard tool copies its artboard (and the art on it).
    if s.tool_id() == "artboard" {
        return super::panelcmds::artboard_copy(s, &json!({}), false);
    }
    let roots = selected_roots(s)?;
    s.clipboard = Clipboard::copy(s.doc()?, &roots);
    Ok(json!({ "copied": s.clipboard.nodes.len() }))
}

fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    if s.tool_id() == "artboard" {
        return super::panelcmds::artboard_copy(s, &json!({}), true);
    }
    // Copy promotes a compound member to its compound. Cut must remove the same objects it
    // copied, while Clear on its own deletes exactly the selected members.
    let ids = selected_roots(s)?;
    copy(s, p)?;
    if s.doc()?.selection.anchors.is_empty() { clear(s, &json!({ "ids": ids })) } else { clear(s, &json!({})) }
}

fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    let explicit = if p.get("ids").is_some() { Some(checked_ids_param(s, p, "ids", "edit.clear")?) } else { None };
    // Direct-selected anchors or segments: remove those, and any objects selected whole with them.
    if explicit.is_none() && !s.doc()?.selection.anchors.is_empty() {
        return super::path::delete_anchors(s, p);
    }
    // Selected ruler guides (selected on their own).
    if explicit.is_none() && s.doc()?.selection.is_empty() {
        return super::docmenu::guide_remove(s, &json!({}));
    }
    let st = s.doc()?;
    // Guides selected with the art go with it (not when ids are given).
    let with_guides = explicit.is_none();
    let targets = explicit.unwrap_or_else(|| st.selection.in_paint_order(&st.doc));
    // Keep the selected ids themselves, without promoting compound members. Delete a selected
    // ancestor just once; its selected descendants leave with it. Layers themselves are kept.
    let targets: BTreeSet<_> = targets.into_iter().filter(|id| st.doc.node(*id).is_some_and(|n| !n.is_layer())).collect();
    let ids: Vec<_> = targets
        .iter()
        .copied()
        .filter(|id| !st.doc.ancestry(*id).unwrap_or_default().iter().rev().skip(1).any(|ancestor| targets.contains(ancestor)))
        .collect();
    if ids.is_empty() {
        return ok();
    }
    s.edit("Clear", |d, sel| {
        d.remove_many(&ids)?;
        if with_guides {
            let gone = sel.guides.clone();
            d.retain_guides(sel, |i, _| !gone.contains(&i));
        }
        sel.clear();
        Ok(())
    })?;
    ok()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PasteMode {
    Offset,
    Front,
    Back,
    InPlace,
    AllArtboards,
    /// A graphic from the Libraries panel: centred where it is placed, as Paste centres.
    Library,
}

impl PasteMode {
    fn label(self) -> &'static str {
        match self {
            PasteMode::Front => "Paste in Front",
            PasteMode::Back => "Paste in Back",
            PasteMode::InPlace => "Paste in Place",
            PasteMode::AllArtboards => "Paste on All Artboards",
            PasteMode::Offset => "Paste",
            PasteMode::Library => "Place from Library",
        }
    }
}

fn paste(s: &mut Session, p: &Value, mode: PasteMode) -> Result<Value> {
    let choices = SwatchChoices::parse(mode.label(), p)?;
    // Taken out of the session for the edit (and put back whatever happens).
    let clip = std::mem::take(&mut s.clipboard);
    let r = paste_clip(s, p, mode, &clip, &choices);
    s.clipboard = clip;
    r
}

/// Place the objects of `clip` (a library graphic) into the active document centred on `center`,
/// as one undo step: their resources come along as a paste's do, a swatch whose name the document
/// gives another colour merged into the document's.
pub(crate) fn place_clip(s: &mut Session, clip: &Clipboard, center: vectorcraft_geom::Point) -> Result<Value> {
    let choices = SwatchChoices::parse(PasteMode::Library.label(), &json!({}))?;
    paste_clip(s, &json!({ "center": [center.x, center.y] }), PasteMode::Library, clip, &choices)
}

/// Paste in Place, in Front or in Back onto artboard `artboard` (the active one, #693): where the
/// objects were on the artboard they were copied from, on that one; without it (or a copy from no
/// artboard), where they were.
fn onto_artboard(doc: &Document, p: &Value, clip: &Clipboard) -> Affine {
    let target = p.get("artboard").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).and_then(|i| doc.artboards.get(i));
    match (target, clip.source_artboard) {
        (Some(t), Some(src)) => Affine::translate(t.rect.origin() - src.origin()),
        _ => Affine::IDENTITY,
    }
}

fn paste_clip(s: &mut Session, p: &Value, mode: PasteMode, clip: &Clipboard, choices: &SwatchChoices) -> Result<Value> {
    let off = s.prefs.paste_offset;
    let st = s.doc()?;
    let same_doc = clip.source_doc == Some(st.uid);
    let parent = st.target_parent()?;
    // A copied artboard comes back with its art: right of the last artboard, or where it was
    // (Paste in Place, in Front, in Back). Paste on All Artboards pastes only the art.
    let board = clip
        .artboard
        .as_ref()
        .filter(|_| mode != PasteMode::AllArtboards)
        .map(|a| (a, if mode == PasteMode::Offset { super::panelcmds::beside_artboards(&st.doc, a.rect) } else { Vec2::ZERO }));
    // Paste Remembers Layers (not while isolating a group: pastes stay in it); an artboard's art
    // always goes back into its layers.
    let remember = (st.doc.paste_remembers_layers || board.is_some()) && parent.is_some_and(|p| st.doc.node(p).is_some_and(Node::is_layer));
    // Front/back: relative to the selection (top-most / bottom-most selected object); with
    // nothing selected, the top / bottom of the current layer.
    let anchor = match mode {
        PasteMode::Front | PasteMode::Back => {
            let order = st.selection.in_paint_order(&st.doc);
            let a = if mode == PasteMode::Front { order.last() } else { order.first() };
            a.and_then(|id| st.doc.position(*id))
        }
        _ => None,
    };
    let placements: Vec<Affine> = match (mode, board) {
        (_, Some((_, dv))) => vec![Affine::translate(dv)],
        (PasteMode::Offset | PasteMode::Library, None) => vec![match point_param(p, "center") {
            Some(c) => clip.bounds().map_or(Affine::IDENTITY, |b| Affine::translate(c - b.center())),
            None => Affine::translate((f64_or(p, "dx", off), f64_or(p, "dy", off))),
        }],
        (PasteMode::AllArtboards, None) => {
            let src = clip.source_artboard.or_else(|| st.doc.artboards.first().map(|a| a.rect)).map(|r| r.origin()).unwrap_or_default();
            st.doc.artboards.iter().map(|a| Affine::translate(a.rect.origin() - src)).collect()
        }
        // In place, in front, in back: onto the active artboard when the UI names it.
        _ => vec![onto_artboard(&st.doc, p, clip)],
    };
    let (ids, imported, artboard) = s.edit(mode.label(), |d, sel| {
        let imported = clip.import_into(d, choices, same_doc);
        let artboard = board.map(|(a, dv)| super::panelcmds::push_artboard_copy(d, a, a.rect + dv));
        let mut layers: BTreeMap<&str, NodeId> = BTreeMap::new();
        // Objects pasted into each parent so far (keeps their order in front and back pastes).
        let mut placed: BTreeMap<Option<NodeId>, usize> = BTreeMap::new();
        let mut new_ids = vec![];
        for xf in placements {
            for (k, n) in clip.nodes.iter().enumerate() {
                let mut c = d.reid(n);
                imported.apply(&mut c);
                if xf != Affine::IDENTITY {
                    c.transform(xf, false);
                }
                let layer = match clip.source_layers.get(k) {
                    Some(Some(name)) if remember => Some(*layers.entry(name).or_insert_with(|| layer_named(d, name))),
                    _ => None,
                };
                let (par, idx) = match (anchor, layer) {
                    // Next to the anchor, unless the object goes back to another layer.
                    (Some((par, i, _)), l) if l.is_none() || l == par => {
                        let k = placed.get(&par).copied().unwrap_or(0);
                        (par, if mode == PasteMode::Front { i + 1 + k } else { i + k })
                    }
                    (_, l) => {
                        let par = l.or(parent);
                        (par, if mode == PasteMode::Back { placed.get(&par).copied().unwrap_or(0) } else { usize::MAX })
                    }
                };
                *placed.entry(par).or_default() += 1;
                new_ids.push(d.insert(par, idx, c)?);
            }
        }
        sel.set(new_ids.iter().copied());
        // Pasted type arrives without the layout bounds cache: fill it so boxes and alignment are
        // right away (opening the file later would, but paste must not depend on a reopen).
        super::typecmd::refresh_bounds_of(d, &new_ids);
        Ok((new_ids, imported, artboard))
    })?;
    // The Artboard tool takes the pasted artboard.
    if let Some(i) = artboard.filter(|_| s.tool_id() == "artboard") {
        s.set_tool_option("active", &json!(i));
    }
    Ok(json!({
        "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(),
        "artboard": artboard,
        "added": imported.added,
        "merged": imported.merged,
        "renamed": imported.renamed,
    }))
}

/// The first unlocked layer or sublayer called `name` (topmost first), or a new top layer.
fn layer_named(d: &mut Document, name: &str) -> NodeId {
    let mut found = None;
    for l in d.layers.iter().rev() {
        l.walk(&mut |n| {
            if found.is_none() && n.is_layer() && !n.locked && n.name.as_deref() == Some(name) {
                found = Some(n.id);
            }
        });
    }
    found.unwrap_or_else(|| d.add_layer(Some(name)))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    let dx = f64_or(p, "dx", 0.0);
    let dy = f64_or(p, "dy", 0.0);
    let ids = s.edit("Duplicate", |d, sel| duplicate_in(d, sel, &roots, Affine::translate((dx, dy))))?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// Duplicate `roots` directly above each original, transform the copies, select them
/// ([`copy_above`]).
pub(crate) fn duplicate_in(d: &mut Document, sel: &mut vectorcraft_doc::Selection, roots: &[NodeId], xf: Affine) -> Result<Vec<NodeId>> {
    let out = copy_above(d, roots, |_, _, c| c.transform(xf, true))?;
    sel.set(out.iter().copied());
    Ok(out)
}

/// Copy `roots` directly above each original and return the copies' ids. `change` gets the
/// document, each original and its copy before the copy goes in, and must leave the document's
/// tree as it is. The copies take new ids in the order of `roots`. Roots listed twice, or inside
/// another root, are copied one at a time: each copy is made from the document as the copies
/// before it left it.
pub(crate) fn copy_above(d: &mut Document, roots: &[NodeId], mut change: impl FnMut(&mut Document, &Node, &mut Node)) -> Result<Vec<NodeId>> {
    let paths = d.index_paths(roots.iter().copied());
    let mut found: Vec<_> = roots.iter().filter_map(|id| paths.get(id)).collect();
    found.sort_unstable();
    let out = if found.windows(2).all(|w| !w[1].starts_with(w[0])) {
        let mut copies = Vec::with_capacity(roots.len());
        for id in roots {
            let Some(n) = paths.get(id).and_then(|p| d.node_at(p)).cloned() else { continue };
            let mut c = d.reid(&n);
            change(d, &n, &mut c);
            copies.push((*id, vec![c]));
        }
        let out: Vec<NodeId> = copies.iter().flat_map(|(_, c)| c.iter().map(|n| n.id)).collect();
        d.insert_beside(copies, false)?;
        out
    } else {
        let mut out = vec![];
        for id in roots {
            let Some((par, idx, _)) = d.position(*id) else { continue };
            let Some(n) = d.node(*id).cloned() else { continue };
            let mut c = d.reid(&n);
            change(d, &n, &mut c);
            out.push(d.insert(par, idx + 1, c)?);
        }
        out
    };
    Ok(out)
}

#[allow(dead_code)]
pub(crate) fn arc_nodes(v: Vec<Node>) -> Vec<Arc<Node>> {
    v.into_iter().map(Arc::new).collect()
}
