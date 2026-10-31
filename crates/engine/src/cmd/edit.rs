//! File (document-level) and Edit commands, plus document queries.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, RulerMode, Unit};
use vectorcraft_geom::{Affine, Point, Vec2};

use super::clipboard::SwatchChoices;
use super::{
    CommandSpec, Result, Session, always, bad, bool_or, can_clear_history, can_redo, can_undo, checked_ids_param, cmd, f64_or, f64_req,
    finite_numbers, has_clipboard, has_doc, has_selection, has_selection_or_artboard_tool, has_selection_or_guides, id_param, index, ok, point_param,
    str_param,
};
use crate::{Clipboard, EngineError, inspect};

/// Register editing commands, including document-to-document artwork drops and their parameter contracts.
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
        cmd!(query "document.inspect", "Inspect Document", [], None, "{depth?, childLimit?, ids?: [id…] (list just these nodes, of any level, in this order, under `nodes` in place of `layers`; an unknown id is an error; at most 10000), kind?: panel label such as Layer; Group; Path; Type or Image (exact, ignoring case; list every such node under `nodes`, each with `path`, its ancestor ids with the layer first, searching the subtrees of `ids` when given), limit?: at most this many entries of the list, offset?: entries skipped first (default 0; with `ids`, `kind`, `limit` or `offset` the reply adds `total`, the entries before paging)} → layer tree (each node: id, name, kind, visible, locked, bounds, opacity (0–1) and opacityPct (0–100 %, the scale object.setProps takes) when below 1, blend when not Normal, fill, stroke and strokeWidth of objects and compound paths; a compound path's members have compoundMember: true instead of paint, which is the compound's), artboards, selection, history (a sliced tree reports childCount); without the scope options the reply is the whole layer tree", has_doc, doc_inspect),
        cmd!(query "document.node", "Inspect Object", [], None, "{id, summary?: compact summary, depth?: child levels in the summary (default all), childLimit?: children shown per node (default all; a level that shows fewer reports childCount)} → one object", has_doc, doc_node),
        cmd!(query "document.find", "Find Objects", [], None, "{name?: substring of the Layers panel name, kind?: panel label such as Group; Path; Type or Image (exact), text?: substring of type content, within?: [x, y, width, height] (an object's geometric bounds inside this rectangle, edges included; document points as document.inspect's bounds; layers never match this or the size filters), intersects?: [x, y, width, height] (bounds touching or overlapping it), minWidth?, maxWidth?, minHeight?, maxHeight?: pt (the bounds' size), minThickness?, maxThickness?: pt (the smaller of the bounds' width and height), minArea?, maxArea?: pt² (the bounds' width × height), minAnchors?, maxAnchors?: anchor points of a path (a compound path's members together; other objects never match), select?: bool (also select the matches, leaving out hidden and locked ones, those inside another match, and a compound path's members unless the compound path is isolated; Select > Reselect repeats it), limit?: max matches listed (default 100, 0 counts only)} → {matches: [{id, name, kind, path}], total, selected (with select: the count selected)}; at least one filter, all of them matching", has_doc, doc_find),
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
        cmd!(
            "document.switchUnits",
            "Switch Units",
            [],
            Some("Cmd+Alt+Shift+U"),
            "{} the document's units become the next of Points, Picas, Inches, Millimeters, Centimeters, Pixels, Feet & Inches, Meters, Yards and Feet (the order a right-click on a ruler lists them), Feet going back to Points; one undo step → {units}",
            has_doc,
            switch_units
        ),
        cmd!(
            query "document.rulerOrigin",
            "Ruler Origin",
            [],
            None,
            "{artboard?: index=0 (the active artboard, which artboard rulers measure from)} → {x, y} the rulers' zero point, in absolute document points",
            has_doc,
            ruler_origin
        ),
        cmd!(
            "document.setRulerOrigin",
            "Ruler Origin",
            [],
            None,
            "{x, y, artboard?: index=0} set the rulers' zero point in absolute document points: the document's with global rulers, that artboard's with artboard rulers (it moves with the artboard); art, guides and command coordinates stay in place",
            has_doc,
            set_ruler_origin
        ),
        cmd!(
            "document.resetRulerOrigin",
            "Reset Ruler Origin",
            [],
            None,
            "{artboard?: index=0} reset the rulers' zero point to that artboard's top-left corner (with artboard rulers, that artboard's own zero point)",
            has_doc,
            reset_ruler_origin
        ),
        cmd!(
            "view.globalRulers",
            "Change to Global Rulers",
            ["View", "Rulers"],
            Some("Cmd+Alt+R"),
            "{global?: bool (default: toggle)} global rulers measure the whole document from one zero point; artboard rulers (a new document's) from the active artboard's top-left corner, or the zero point set for it. Saved with the document → {global}",
            has_doc,
            global_rulers
        ),
        cmd!(
            "edit.dropSelection",
            "Drop Artwork",
            [],
            None,
            "{sourceDocument: uid, ids: [id…], grab: [x,y], at: [x,y]} copy 1–10000 editable source objects from an open document to the active document, preserving the grab point and clipboard, as one undo step",
            has_doc,
            drop_selection
        ),
        cmd!("edit.undo", "Undo", ["Edit"], Some("Cmd+Z"), "{}", can_undo, undo),
        cmd!("edit.redo", "Redo", ["Edit"], Some("Cmd+Shift+Z"), "{}", can_redo, redo),
        cmd!(
            "edit.clearHistory",
            "Clear History",
            [],
            None,
            "{} remove every undo and redo step of the active document (the History panel menu's Clear History): the document stays as it is, and clearing is not an undo step → {cleared: the number of steps removed}",
            can_clear_history,
            clear_history
        ),
        cmd!(
            "edit.undoGroup.begin",
            "Begin Undo Group",
            [],
            None,
            "{label?: string} until edit.undoGroup.end, the edits made in the document active at the begin, and in each document opened or created before then (also when no document is open; the edits of a command that opens a document and edits it stay that command's undo step), are one undo step in that document, named `label` (else after its first edit); in a document that was open before the begin and is switched to (document.activate), each edit stays its own undo step: for automation, such as MCP command_batch with `atomic` (#1163)",
            always,
            undo_group_begin
        ),
        cmd!(
            "edit.undoGroup.end",
            "End Undo Group",
            [],
            None,
            "{cancel?: bool} close the undo group: its edits stay one undo step or, with `cancel`, are undone",
            always,
            undo_group_end
        ),
        cmd!("edit.cut", "Cut", ["Edit"], Some("Cmd+X"), "{} (the Artboard tool chosen: artboard.cut)", has_selection_or_artboard_tool, cut),
        cmd!("edit.copy", "Copy", ["Edit"], Some("Cmd+C"), "{} (the Artboard tool chosen: artboard.copy)", has_selection_or_artboard_tool, copy),
        cmd!(
            "edit.paste",
            "Paste",
            ["Edit"],
            Some("Cmd+V"),
            "{center?: [x, y], dx?, dy?, swatchConflict?} paste centred on `center` (the app passes the view centre), else offset by dx/dy (default: the Paste Offset preference). Pasting brings the image blobs, symbols, patterns, global and spot swatches (with the tint swatches of the tints used), gradient swatches, graphic styles, character and paragraph styles and brushes the objects use; one of the same name that differs comes in renamed. swatchConflict, for a swatch whose name the document gives another colour (clipboard.conflicts): \"merge\" (default: the objects take the document's swatch) | \"add\" (the pasted swatch comes in renamed) | {name: \"merge\"|\"add\"}. With Paste Remembers Layers on (layer.pasteRemembersLayers), objects go back into the layers they came from (by name; made when missing). Artboards copied with artboard.copy come with their art, together right of the last artboard as they lay (the other paste commands but Paste on All Artboards: where they were), their art back in the layers it came from, and the Artboard tool, when chosen, selects them. In Draw Inside (view.drawMode) the objects go inside the object drawn into, as new art does, and Paste Remembers Layers doesn't apply; edit.pasteInPlace and edit.pasteOnAllArtboards do the same, and in Draw Behind they go behind the selection as edit.pasteInBack puts them; edit.pasteInFront and edit.pasteInBack ignore the drawing mode → {ids, artboard: the first new artboard's index or null, artboards: the new artboards' indices, added: resources added, merged: conflicts merged, renamed: [{kind, from, to}]}",
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
            "{swatchConflict?, artboard?: index (as edit.pasteInFront)} paste where the objects were copied (resources, layers and drawing modes as edit.paste) → {ids, added, merged, renamed}",
            has_clipboard,
            |s, p| paste(s, p, PasteMode::InPlace)
        ),
        cmd!(
            "edit.pasteOnAllArtboards",
            "Paste on All Artboards",
            ["Edit"],
            Some("Cmd+Alt+Shift+V"),
            "{swatchConflict?} paste a copy on every artboard at the offset the objects had to the artboard they were copied from (resources, layers and drawing modes as edit.paste) → {ids, added, merged, renamed}",
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
    let i = p.get("index").and_then(Value::as_u64).map(index).or(s.active_index()).ok_or(EngineError::NoDocument)?;
    if !s.close_document(i) {
        return Err(bad("file.close", "no such document"));
    }
    ok()
}

fn doc_activate(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).map(index).ok_or_else(|| bad("document.activate", "missing index"))?;
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

/// Most node ids `document.inspect {ids}` takes.
const MAX_INSPECT_IDS: usize = 10_000;

fn doc_inspect(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "document.inspect";
    let opts = slice_opts(p, C)?;
    let ids = match p.get("ids") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) if a.len() > MAX_INSPECT_IDS => return Err(bad(C, format!("`ids` takes at most {MAX_INSPECT_IDS} ids"))),
        Some(_) => Some(checked_ids_param(s, p, "ids", C)?),
    };
    let kind = match p.get("kind") {
        None | Some(Value::Null) => None,
        Some(Value::String(k)) => Some(k.clone()),
        Some(_) => return Err(bad(C, "`kind` must be a string")),
    };
    let scope = inspect::Scope { ids, kind, limit: opt_u64(p, C, "limit")?, offset: opt_u64(p, C, "offset")?.unwrap_or(0) };
    Ok(inspect::document_scoped(s, opts, &scope))
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
    const C: &str = "document.find";
    // A filter of the wrong type is an error, not dropped: dropping it would widen the search.
    let needle = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.to_lowercase()).filter(|v| !v.is_empty())),
        Some(_) => Err(bad(C, format!("`{key}` must be a string"))),
    };
    let rect = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match finite_numbers::<4>(v) {
            Some([x, y, w, h]) if w >= 0.0 && h >= 0.0 => Ok(Some(vectorcraft_geom::Rect::new(x, y, x + w, y + h))),
            _ => Err(bad(C, format!("`{key}` must be [x, y, width, height]: four finite numbers, width and height not negative"))),
        },
    };
    let bound = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(Some).ok_or_else(|| bad(C, format!("`{key}` must be a number not below 0"))),
    };
    let range = |name: &str| -> Result<inspect::FindRange> {
        let (min_key, max_key) = (format!("min{name}"), format!("max{name}"));
        let r = inspect::FindRange { min: bound(&min_key)?, max: bound(&max_key)? };
        if let (Some(min), Some(max)) = (r.min, r.max)
            && min > max
        {
            return Err(bad(C, format!("`{min_key}` is greater than `{max_key}`")));
        }
        Ok(r)
    };
    let anchors = range("Anchors")?;
    if [anchors.min, anchors.max].iter().flatten().any(|n| n.fract() != 0.0) {
        return Err(bad(C, "`minAnchors` and `maxAnchors` must be whole numbers"));
    }
    let filter = inspect::FindFilter {
        name: needle("name")?,
        kind: needle("kind")?,
        text: needle("text")?,
        within: rect("within")?,
        intersects: rect("intersects")?,
        width: range("Width")?,
        height: range("Height")?,
        thickness: range("Thickness")?,
        area: range("Area")?,
        anchors,
    };
    if filter.is_empty() {
        return Err(bad(
            C,
            "give at least one of `name`, `kind`, `text`, `within`, `intersects`, `minWidth`, `maxWidth`, `minHeight`, `maxHeight`, `minThickness`, `maxThickness`, `minArea`, `maxArea`, `minAnchors`, `maxAnchors`",
        ));
    }
    let select = match p.get("select") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(C, "`select` must be true or false"))?,
    };
    let limit = opt_u64(p, C, "limit")?.unwrap_or(100);
    let (matches, ids) = inspect::find_nodes(&s.doc()?.doc, &filter, limit);
    let mut reply = json!({"matches": matches, "total": ids.len()});
    if select {
        reply["selected"] = select_found(s, p, &ids)?["count"].clone();
        s.journal_query();
    }
    Ok(reply)
}

/// Select the objects among `ids` that Select All and the objects inside them take (none hidden or
/// locked, nor inside a hidden or locked one), leaving out those inside another of them and the
/// members of compound paths, unless a compound path is isolated. Select > Reselect repeats
/// `document.find` with `p`.
fn select_found(s: &mut Session, p: &Value, ids: &[NodeId]) -> Result<Value> {
    let st = s.doc()?;
    let compound = |n: &Node| matches!(n.kind, vectorcraft_doc::NodeKind::Compound { .. });
    let members = st.isolation.and_then(|i| st.doc.ancestry(i)).is_some_and(|a| a.iter().any(|i| st.doc.node(*i).is_some_and(compound)));
    let found: BTreeSet<NodeId> = ids.iter().copied().collect();
    super::select::select_where(s, "document.find", p, |d, n| {
        found.contains(&n.id) && (members || !d.parent_of(n.id).and_then(|p| d.node(p)).is_some_and(compound))
    })
}

/// Document Setup's units (also `document.setup {units}`).
fn set_units(s: &mut Session, p: &Value) -> Result<Value> {
    let u = str_param(p, "units").and_then(Unit::named).ok_or_else(|| bad("document.setUnits", "unknown units"))?;
    s.set_document_units(u)?;
    ok()
}

/// Switch Units: the unit after the document's in [`Unit::ALL`], wrapping around.
fn switch_units(s: &mut Session, _: &Value) -> Result<Value> {
    let now = s.doc()?.doc.units;
    let next = Unit::ALL.iter().position(|u| *u == now).and_then(|i| Unit::ALL.get((i + 1) % Unit::ALL.len())).copied().unwrap_or_default();
    s.set_document_units(next)?;
    Ok(json!({ "units": next.label() }))
}

/// The `artboard` param of the ruler origin commands: an artboard index, 0 when not given.
fn artboard_param(s: &Session, p: &Value, c: &str) -> Result<usize> {
    let index = match p.get("artboard") {
        None => 0,
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(c, "index out of range"))?,
    };
    if index >= s.doc()?.doc.artboards.len() {
        return Err(bad(c, "index out of range"));
    }
    Ok(index)
}

fn ruler_origin(s: &mut Session, p: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    // A document without artboards measures from its global zero point.
    let index = if d.artboards.is_empty() && p.get("artboard").is_none() { 0 } else { artboard_param(s, p, "document.rulerOrigin")? };
    Ok(json!(s.doc()?.doc.ruler_origin_for(index)))
}

fn set_ruler_origin(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "document.setRulerOrigin";
    let origin = Point::new(f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    let index = if s.doc()?.doc.rulers == RulerMode::Global && p.get("artboard").is_none() { 0 } else { artboard_param(s, p, C)? };
    change_ruler_origin(s, index, Some(origin), "Ruler Origin")
}

fn reset_ruler_origin(s: &mut Session, p: &Value) -> Result<Value> {
    let index = artboard_param(s, p, "document.resetRulerOrigin")?;
    change_ruler_origin(s, index, None, "Reset Ruler Origin")
}

/// Move the rulers' zero point to `origin`, or back to artboard `index`'s top-left corner: the
/// document's (global rulers) or that artboard's (artboard rulers, kept as an offset from its
/// corner so it follows the artboard).
fn change_ruler_origin(s: &mut Session, index: usize, origin: Option<Point>, label: &str) -> Result<Value> {
    let d = &s.doc()?.doc;
    let global = d.rulers == RulerMode::Global;
    let corner = d.artboards.get(index).map(|ab| Point::new(ab.rect.x0, ab.rect.y0));
    let target = origin.or(corner).unwrap_or(Point::ZERO);
    if !target.is_finite() || target.x.abs() > crate::MAX_COORD || target.y.abs() > crate::MAX_COORD {
        return Err(EngineError::Other("result would exceed the canvas (coordinates out of range)".into()));
    }
    if global {
        if d.ruler_origin != target {
            s.edit(label, |d, _| {
                d.ruler_origin = target;
                Ok(())
            })?;
        }
        return Ok(json!(target));
    }
    let Some((id, corner)) = d.artboards.get(index).map(|ab| ab.id).zip(corner) else {
        return Err(bad("document.setRulerOrigin", "index out of range"));
    };
    let offset = Some(target - corner).filter(|o| *o != Vec2::ZERO);
    if d.artboard_ruler_origins.get(&id).copied() != offset {
        s.edit(label, |d, _| {
            // Origins of artboards that no longer exist go too.
            let ids: BTreeSet<u32> = d.artboards.iter().map(|ab| ab.id).collect();
            d.artboard_ruler_origins.retain(|id, _| ids.contains(id));
            match offset {
                Some(o) => d.artboard_ruler_origins.insert(id, o),
                None => d.artboard_ruler_origins.remove(&id),
            };
            Ok(())
        })?;
    }
    Ok(json!(target))
}

/// View › Rulers › Change to Global / Artboard Rulers.
fn global_rulers(s: &mut Session, p: &Value) -> Result<Value> {
    let current = s.doc()?.doc.rulers == RulerMode::Global;
    let global = match p.get("global") {
        None => !current,
        Some(v) => v.as_bool().ok_or_else(|| bad("view.globalRulers", "global must be true or false"))?,
    };
    if global != current {
        let label = if global { "Change to Global Rulers" } else { "Change to Artboard Rulers" };
        s.edit(label, |d, _| {
            d.rulers = if global { RulerMode::Global } else { RulerMode::Artboard };
            Ok(())
        })?;
    }
    Ok(json!({ "global": global }))
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
    s.explicit_undo_group = Some(label.clone());
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
    let menu = s.menu.clone();
    let st = s.doc_mut()?;
    let e = st.history.undo.pop().ok_or_else(|| EngineError::Other("nothing to undo".into()))?;
    let label = e.label.clone();
    let costs = e.costs();
    let editing = st.doc.mask_edit.map(|m| m.layer);
    let doc = std::mem::replace(&mut st.doc, e.doc);
    let selection = std::mem::replace(&mut st.selection, Arc::unwrap_or_clone(e.selection));
    let selection = shared_selection(selection, st.history.redo.last());
    let mut turned = crate::HistoryEntry::turned(e.label, doc, selection, &st.doc, costs);
    turned.named = e.named;
    st.history.redo.push(turned);
    st.history.trim();
    st.revision += 1;
    st.selection.prune(&st.doc);
    // Guides locked and slices locked or hidden since the step was made stay deselected.
    crate::deselect_hidden(&menu, &st.doc, &mut st.selection);
    st.sync_new_art();
    super::maskedit::follow_history(st, editing);
    leave_isolation_if_gone(st);
    super::globaledit::follow_history(st);
    Ok(json!({ "undone": label }))
}

fn redo(s: &mut Session, _: &Value) -> Result<Value> {
    settle_interaction(s)?;
    let menu = s.menu.clone();
    let st = s.doc_mut()?;
    let e = st.history.redo.pop().ok_or_else(|| EngineError::Other("nothing to redo".into()))?;
    let label = e.label.clone();
    let costs = e.costs();
    let editing = st.doc.mask_edit.map(|m| m.layer);
    let doc = std::mem::replace(&mut st.doc, e.doc);
    let selection = std::mem::replace(&mut st.selection, Arc::unwrap_or_clone(e.selection));
    let selection = shared_selection(selection, st.history.undo.last());
    let mut turned = crate::HistoryEntry::turned(e.label, doc, selection, &st.doc, costs);
    turned.named = e.named;
    st.history.undo.push(turned);
    st.history.trim();
    st.revision += 1;
    st.selection.prune(&st.doc);
    crate::deselect_hidden(&menu, &st.doc, &mut st.selection);
    st.sync_new_art();
    super::maskedit::follow_history(st, editing);
    leave_isolation_if_gone(st);
    super::globaledit::follow_history(st);
    Ok(json!({ "redone": label }))
}

/// After undo or redo, isolation mode ends when the isolated object is no longer in the document,
/// such as the clip group Draw Inside made around the isolated object (#3705), rather than naming
/// an id a later object may take.
fn leave_isolation_if_gone(st: &mut crate::DocState) {
    if st.isolation.is_some_and(|i| st.doc.node(i).is_none()) {
        st.isolation = None;
    }
}

/// History panel › Clear History: removes the steps after the current state and the steps before
/// it, and leaves the document as it is. Typing or a drag still in progress is first kept as a
/// step, as Undo keeps it, so it is removed with the others and its change stays in the document.
fn clear_history(s: &mut Session, _: &Value) -> Result<Value> {
    settle_interaction(s)?;
    let st = s.doc_mut()?;
    let cleared = st.history.undo.len() + st.history.redo.len();
    st.history.undo.clear();
    st.history.redo.clear();
    st.revision += 1;
    Ok(json!({ "cleared": cleared }))
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

/// [`selected_roots`] for the commands that move or reshape the art (Move, Transform, Rotate…): a
/// path of a compound path selected on its own (Group Selection, or in the isolated compound path)
/// is an object of its own, so the hole of a ring moves without the ring (#2441). Paint and the
/// rest still go to the compound path, which holds the paint.
pub(crate) fn selected_roots_to_transform(s: &Session) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    Ok(roots_within(&st.doc, st.selection.in_paint_order(&st.doc), true))
}

/// The top-level objects among `ids` (in paint order): compound-path members stand for their
/// compound, and ids whose ancestor is also listed, and layers, are dropped.
pub(crate) fn roots_of(doc: &Document, ids: Vec<NodeId>) -> Vec<NodeId> {
    roots_within(doc, ids, false)
}

/// [`roots_of`]; with `members`, a compound path's member listed stays itself.
fn roots_within(doc: &Document, ids: Vec<NodeId>, members: bool) -> Vec<NodeId> {
    // Every id's place in the tree, found in one walk (a lookup per id is a walk per id).
    let paths = doc.index_paths(ids.iter().copied());
    // A compound-path member acts as its compound (compounds are one object).
    let mut sel: Vec<(NodeId, Option<Vec<usize>>)> = ids
        .into_iter()
        .map(|id| {
            let path = paths.get(&id);
            let parent = path.and_then(|p| p.split_last()).map(|(_, up)| up).filter(|up| !up.is_empty());
            match parent.and_then(|up| doc.node_at(up).map(|n| (up, n))) {
                Some((up, n)) if matches!(n.kind, vectorcraft_doc::NodeKind::Compound { .. }) && !members => (n.id, Some(up.to_vec())),
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

/// Document ids scope node ids: two tabs may contain objects with the same node id.
fn drop_selection(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "edit.dropSelection";
    let key = "sourceDocument";
    let uid = p[key].as_u64().ok_or_else(|| bad(CMD, format!("missing integer `{key}`")))?;
    let point = |key: &str| finite_numbers::<2>(&p[key]).ok_or_else(|| bad(CMD, format!("{key} must be [x, y] with finite coordinates")));
    let grab = point("grab")?;
    let at = point("at")?;
    let source = s.documents().iter().find(|d| d.uid == uid).ok_or_else(|| bad(CMD, "that document is no longer open"))?;
    let key = "ids";
    let values = p[key].as_array().ok_or_else(|| bad(CMD, format!("{key} must be an array of non-negative integer object ids")))?;
    if values.is_empty() || values.len() > 10000 {
        return Err(bad(CMD, "ids must contain 1–10000 object ids"));
    }
    let ids = values
        .iter()
        .map(|value| value.as_u64().map(NodeId).ok_or_else(|| bad(CMD, format!("invalid object id {value}: expected a non-negative integer"))))
        .collect::<Result<Vec<_>>>()?;
    // Validate the entire batch in one walk; node ids are local to the source document.
    let paths = source.doc.index_paths(ids.iter().copied());
    if let Some(id) = ids.iter().find(|id| !paths.contains_key(id)) {
        return Err(EngineError::NoNode(*id));
    }
    let clip = Clipboard::copy(source, &roots_of(&source.doc, ids));
    let bounds = clip.bounds().ok_or_else(|| bad(CMD, "the art has no bounds"))?;
    let center = bounds.center() + Vec2::new(at[0] - grab[0], at[1] - grab[1]);
    if !center.x.is_finite() || !center.y.is_finite() {
        return Err(bad(CMD, "result would exceed the canvas (coordinates out of range)"));
    }
    // A drop preserves the source colours even when the flyer has same-named swatches.
    let choices = SwatchChoices::parse(PasteMode::Drag.label(), &json!({"swatchConflict": "add"}))?;
    paste_clip(s, &json!({"center": [center.x, center.y]}), PasteMode::Drag, &clip, &choices)
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
    Drag,
}

impl PasteMode {
    /// Return the user-visible action label for this insertion mode.
    fn label(self) -> &'static str {
        match self {
            PasteMode::Front => "Paste in Front",
            PasteMode::Back => "Paste in Back",
            PasteMode::InPlace => "Paste in Place",
            PasteMode::AllArtboards => "Paste on All Artboards",
            PasteMode::Offset => "Paste",
            PasteMode::Library => "Place from Library",
            PasteMode::Drag => "Drop Artwork",
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

/// Insert clipboard resources atomically in the active editable context using the requested placement mode.
fn paste_clip(s: &mut Session, p: &Value, mode: PasteMode, clip: &Clipboard, choices: &SwatchChoices) -> Result<Value> {
    let off = s.prefs.paste_offset;
    let st = s.doc()?;
    let same_doc = clip.source_doc == Some(st.uid);
    let parent = st.target_parent()?;
    // Copied artboards come back with their art: together right of the last artboard, as they lay,
    // or where they were (Paste in Place, in Front, in Back). Paste on All Artboards pastes only the
    // art.
    let boards = clip
        .artboards
        .iter()
        .map(|a| a.rect)
        .reduce(|a, b| a.union(b))
        .filter(|_| mode != PasteMode::AllArtboards)
        .map(|all| (&clip.artboards, if mode == PasteMode::Offset { super::panelcmds::beside_artboards(&st.doc, all) } else { Vec2::ZERO }));
    // Drawing modes (#3705): Draw Inside puts what Paste, Paste in Place and Paste on All
    // Artboards bring inside the object it draws into; Draw Behind puts what Paste in Place and
    // Paste on All Artboards bring behind the selection, as Paste in Back does. Paste in Front and
    // Paste in Back keep their own places, and copied artboards' art goes back into its layers.
    let modal = boards.is_none() && matches!(mode, PasteMode::Offset | PasteMode::InPlace | PasteMode::AllArtboards);
    // (Only with objects to paste: an artboard copied without art makes no clip group.)
    let inside = s.draw_inside.filter(|t| modal && !clip.nodes.is_empty() && s.draw_mode == crate::DrawMode::Inside && st.doc.node(*t).is_some());
    let behind = modal && mode != PasteMode::Offset && s.draw_mode == crate::DrawMode::Behind;
    let (front, back) = (mode == PasteMode::Front, mode == PasteMode::Back || behind);
    // Paste Remembers Layers (not while isolating a group: pastes stay in it, nor in Draw
    // Inside); an artboard's art always goes back into its layers.
    // A drop goes into the current editable insertion context, not a same-named source layer.
    let remember = mode != PasteMode::Drag
        && inside.is_none()
        && (st.doc.paste_remembers_layers || boards.is_some())
        && parent.is_some_and(|p| st.doc.node(p).is_some_and(Node::is_layer));
    // Front/back: relative to the selection (top-most / bottom-most selected object); with
    // nothing selected, the top / bottom of the current layer.
    let anchor = if front || back {
        let order = st.selection.in_paint_order(&st.doc);
        let a = if front { order.last() } else { order.first() };
        a.and_then(|id| st.doc.position(*id))
    } else {
        None
    };
    let placements: Vec<Affine> = match (mode, boards) {
        (_, Some((_, dv))) => vec![Affine::translate(dv)],
        (PasteMode::Offset | PasteMode::Library | PasteMode::Drag, None) => vec![match point_param(p, "center") {
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
    let (ids, imported, artboards, into) = s.edit(mode.label(), |d, sel| {
        let imported = clip.import_into(d, choices, same_doc);
        // Draw Inside: the clip group of the object drawn into, made on the first paste.
        let into = inside.map(|t| super::create::draw_inside_group(d, t)).transpose()?;
        let artboards: Vec<usize> = boards
            .into_iter()
            .flat_map(|(all, dv)| all.iter().map(move |a| (a, dv)))
            .map(|(a, dv)| super::panelcmds::push_artboard_copy(d, a, a.rect + dv))
            .collect();
        let mut layers: BTreeMap<&str, NodeId> = BTreeMap::new();
        // Objects pasted into each parent so far (keeps their order in front and back pastes).
        let mut placed: BTreeMap<Option<NodeId>, usize> = BTreeMap::new();
        let mut new_ids = vec![];
        for xf in placements {
            for (k, n) in clip.nodes.iter().enumerate() {
                let mut c = d.reid(n);
                imported.apply(&mut c);
                if xf != Affine::IDENTITY {
                    // A cross-document drag carries the graphic's appearance with it,
                    // including the pattern phase, independently of transform-tool settings.
                    c.transform(xf, vectorcraft_doc::Scaling { patterns: mode == PasteMode::Drag, ..Default::default() });
                }
                let layer = match clip.source_layers.get(k) {
                    Some(Some(name)) if remember => Some(*layers.entry(name).or_insert_with(|| layer_named(d, name))),
                    _ => None,
                };
                let (par, idx) = match (into, anchor, layer) {
                    // Inside the object Draw Inside draws into, on top of what is in it already.
                    (Some(group), ..) => (Some(group), usize::MAX),
                    // Next to the anchor, unless the object goes back to another layer.
                    (None, Some((par, i, _)), l) if l.is_none() || l == par => {
                        let k = placed.get(&par).copied().unwrap_or(0);
                        (par, if front { i + 1 + k } else { i + k })
                    }
                    (None, _, l) => {
                        let par = l.or(parent);
                        (par, if back { placed.get(&par).copied().unwrap_or(0) } else { usize::MAX })
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
        Ok((new_ids, imported, artboards, into))
    })?;
    if let (Some(target), Some(group)) = (inside, into) {
        super::create::isolate_draw_inside_group(s, target, group)?;
    }
    // The Artboard tool selects the pasted artboards, the first one active.
    if !artboards.is_empty() && s.tool_id() == "artboard" {
        s.set_tool_option("selected", &json!(artboards));
    }
    Ok(json!({
        "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(),
        "artboard": artboards.first(),
        "artboards": artboards,
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
/// before it left it. Copied type whose layout caches [`Document::reid`] dropped has them again.
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
    super::typecmd::refresh_bounds_of(d, &out);
    Ok(out)
}

#[allow(dead_code)]
pub(crate) fn arc_nodes(v: Vec<Node>) -> Vec<Arc<Node>> {
    v.into_iter().map(Arc::new).collect()
}
