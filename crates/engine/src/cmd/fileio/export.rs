//! `document.export`, `document.serialize`, `document.exportSelection`, `document.exportForOffice`.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, SymbolBoxes};

use crate::DocState;

use super::super::*;
use super::encode::single_artboard;
use super::{
    ARTBOARD_PARAMS, Format, default_name, encode, encode_all, encode_with_warnings, merge, with_black_output, writable, write_encoded,
    write_or_return,
};

pub(super) fn serialize(s: &mut Session, p: &Value) -> Result<Value> {
    let f = writable("document.serialize", Some(str_param(p, "format").unwrap_or("vectorcraft")), None)?;
    let expanded = expand_presets(s, "document.serialize", p)?;
    let p = with_black_output(s, &expanded);
    let doc = &*source(s, f, &p, "document.serialize")?;
    let enc = encode_all(doc, f.id, &p)?;
    let files = enc.named(doc, &default_name(doc, f.extensions[0]));
    let (main, linked) = files.split_at(enc.files.len());
    // Text formats come back as text; an SVG in UTF-16 or ISO 8859-1 also as its bytes.
    let data = |bytes: &[u8]| match f.id {
        "svg" => {
            let text = vectorcraft_svg::text_of(bytes).unwrap_or_else(|_| String::from_utf8_lossy(bytes));
            let mut v = json!({ "text": text });
            if text.as_bytes() != bytes {
                v["dataBase64"] = json!(vectorcraft_format::base64_encode(bytes));
            }
            v
        }
        _ => json!({ "dataBase64": vectorcraft_format::base64_encode(bytes) }),
    };
    let mut out = merge(data(main.first().map_or(&[][..], |m| &m.1)), json!({ "warnings": enc.warnings }));
    if main.len() > 1 {
        out["files"] = main.iter().map(|(name, bytes)| merge(json!({ "name": name }), data(bytes))).collect();
    }
    if !linked.is_empty() {
        out["linked"] = linked.iter().map(|(name, bytes)| json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) })).collect();
    }
    Ok(out)
}

pub(super) fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path");
    if let Some(path) = path {
        super::check_not_lossy_overwrite(s.doc()?, path, p, "document.export")?;
    }
    let f = writable("document.export", str_param(p, "format"), path)?;
    let expanded = expand_presets(s, "document.export", p)?;
    let p = with_black_output(s, &expanded);
    let doc = &*source(s, f, &p, "document.export")?;
    let enc = encode_all(doc, f.id, &p)?;
    write_encoded(path, &default_name(doc, f.extensions[0]), doc, &enc, json!({ "format": f.id, "warnings": enc.warnings }))
}

/// The document an export of `st` writes: the whole document, or with `selectedOnly` just the
/// selected objects, each in its layers and groups (a clipping group keeps its clipping path).
pub fn export_source<'a>(st: &'a DocState, p: &Value) -> Result<Cow<'a, Document>> {
    if !bool_or(p, "selectedOnly", false) {
        return Ok(Cow::Borrowed(&*st.doc));
    }
    let keep: HashSet<NodeId> = edit::roots_of(&st.doc, st.selection.in_paint_order(&st.doc)).into_iter().collect();
    if keep.is_empty() {
        return Err(bad("document.export", "select something to export (or turn off selectedOnly)"));
    }
    let mut d = Document::clone(&st.doc);
    d.layers = st.doc.layers.iter().filter_map(|l| selected_part(l, &keep)).map(Arc::new).collect();
    Ok(Cow::Owned(d))
}

/// `n` with only the parts in `keep` (whole, with everything inside them), `None` when none is.
fn selected_part(n: &Node, keep: &HashSet<NodeId>) -> Option<Node> {
    if keep.contains(&n.id) {
        return Some(n.clone());
    }
    let children = n.children()?;
    let mut kept: Vec<Arc<Node>> = children.iter().filter_map(|c| selected_part(c, keep).map(Arc::new)).collect();
    if kept.is_empty() {
        return None;
    }
    let clip = matches!(n.kind, NodeKind::Group { clip: true, .. } | NodeKind::Layer { clip: true, .. });
    if let Some(first) = children.first().filter(|f| clip && kept.first().is_none_or(|k| k.id != f.id)) {
        kept.insert(0, first.clone());
    }
    let mut out = n.clone();
    *out.children_mut()? = kept;
    Some(out)
}

/// `doc` with only the art over `rect` (an artboard): objects whose visual bounds miss it are left
/// out, at any depth of layers, sublayers and groups that don't clip, unless what the renderer
/// draws for them, their effects included ([`vectorcraft_render::effect_bounds`]), reaches it.
/// Objects that reach it stay whole (the file's bounds clip them), as DXF's crop leaves them.
/// Symbol instances are measured by their art, and the box of one whose art holds brushed strokes
/// grows by as far as their brush art can paint ([`SymbolBoxes::with_brush_art`]). An object that
/// holds a symbol instance also stays when what the canvas paints for it reaches `rect`
/// ([`vectorcraft_render::painted_bounds`]: the effects in the symbols' art and its own effects). A
/// clipping layer keeps every object, its clipping path among them. An artboard's file then holds
/// its own art, not a whole document of artboards.
pub(super) fn art_over(doc: &Document, rect: vectorcraft_geom::Rect) -> Document {
    fn kept(n: &Arc<Node>, doc: &Document, symbols: &SymbolBoxes, rect: vectorcraft_geom::Rect) -> Option<Arc<Node>> {
        if matches!(n.kind, NodeKind::Layer { clip: false, .. }) {
            let mut layer = Node::clone(n);
            let children = layer.children_mut()?;
            *children = children.iter().filter_map(|c| kept(c, doc, symbols, rect)).collect();
            return Some(Arc::new(layer));
        }
        // A group that doesn't clip keeps only its members over the artboard, so art gathered in one
        // group across several artboards splits by artboard. Not with effects of its own: a shadow
        // can reach in from a member outside.
        if matches!(n.kind, NodeKind::Group { clip: false, .. }) && n.appearance.effects.is_empty() {
            let mut group = Node::clone(n);
            let children = group.children_mut()?;
            *children = children.iter().filter_map(|c| kept(c, doc, symbols, rect)).collect();
            return (!children.is_empty()).then(|| Arc::new(group));
        }
        let over = |b: vectorcraft_geom::Rect| b.x0 <= rect.x1 && rect.x0 <= b.x1 && b.y0 <= rect.y1 && rect.y0 <= b.y1;
        let mut holds_instance = false;
        n.walk(&mut |c| holds_instance |= matches!(c.kind, NodeKind::SymbolInstance { .. }));
        let drawn = || vectorcraft_render::effect_bounds(symbols, n).is_some_and(over);
        let painted = || holds_instance && vectorcraft_render::painted_bounds(doc, n).is_some_and(over);
        (n.visual_bounds_with(symbols).is_none_or(over) || drawn() || painted()).then(|| n.clone())
    }
    let symbols = doc.symbol_boxes().with_brush_art();
    let mut d = doc.clone();
    d.layers = doc.layers.iter().filter_map(|l| kept(l, doc, &symbols, rect)).collect();
    d
}

/// `p` without its artboard choice, also inside its SVG options (for documents made of one
/// synthetic artboard, and for callers that pick the artboard themselves).
pub(super) fn without_artboards(p: &Value) -> Value {
    let strip = |o: &mut serde_json::Map<String, Value>| ARTBOARD_PARAMS.iter().for_each(|k| _ = o.remove(*k));
    let mut q = p.clone();
    if let Some(o) = q.as_object_mut() {
        strip(o);
        if let Some(svg) = o.get_mut("svg").and_then(Value::as_object_mut) {
            strip(svg);
        }
    }
    q
}

/// `p` with the saved presets it names (PDF, flattener) written out as options, for the encoders,
/// which know only the built-in ones.
fn expand_presets<'a>(s: &Session, cmd: &str, p: &'a Value) -> Result<Cow<'a, Value>> {
    super::pdf::expand_preset(s, cmd, p)
}

/// The document an export of `f` writes: the active one (see [`export_source`]), or for text
/// with `selectionOnly` the selected objects alone.
fn source<'a>(s: &'a mut Session, f: &Format, p: &Value, cmd: &str) -> Result<Cow<'a, Document>> {
    if f.id == "txt" && bool_or(p, "selectionOnly", false) {
        return Ok(Cow::Owned(selection(s, cmd)?.0));
    }
    let st = s.doc()?;
    let doc = export_source(st, p)?;
    Ok(super::titled_for_export(&doc, st.path.as_deref(), f.id).map_or(doc, Cow::Owned))
}

/// The selected objects (not on template layers) alone on one layer, with one artboard: their
/// visual bounds.
fn selection(s: &mut Session, cmd: &str) -> Result<(Document, vectorcraft_geom::Rect)> {
    let ids = selected_artwork(s)?;
    isolated(&s.doc()?.doc, &ids, "Selection").ok_or_else(|| bad(cmd, "select something to export"))
}

/// The selected objects (their top-most selected ancestors) that are artwork: those on template
/// layers are guides and are left out.
pub(crate) fn selected_artwork(s: &mut Session) -> Result<Vec<NodeId>> {
    let ids = edit::selected_roots(s)?;
    let st = s.doc()?;
    let is_template = |id| st.doc.node(id).is_some_and(|l| matches!(l.kind, NodeKind::Layer { template: true, .. }));
    // On a template layer or sublayer, at any depth.
    let on_template = |id| st.doc.ancestry(id).is_some_and(|a| a.into_iter().any(is_template));
    Ok(ids.into_iter().filter(|id| !on_template(*id)).collect())
}

/// The crop of an export of objects `ids` of `doc` alone: the bounds the renderer measures for what
/// it draws for them, their effects included ([`vectorcraft_render::effect_bounds`]; symbol
/// instances are measured by their art), `None` when they have none.
pub(crate) fn export_crop(doc: &Document, ids: &[NodeId]) -> Option<vectorcraft_geom::Rect> {
    let symbols = doc.symbol_boxes();
    ids.iter().filter_map(|id| vectorcraft_render::effect_bounds(&symbols, doc.node(*id)?)).reduce(|a, b| a.union(b))
}

/// Objects `ids` of `doc` alone on one layer (in that order, back to front), with one artboard
/// named `name`: their [`export_crop`] (`None` when they have none).
pub(crate) fn isolated(doc: &Document, ids: &[NodeId], name: &str) -> Option<(Document, vectorcraft_geom::Rect)> {
    let bounds = export_crop(doc, ids)?;
    let nodes: Vec<Arc<Node>> = ids.iter().filter_map(|id| doc.node(*id).cloned().map(Arc::new)).collect();
    let mut d = single_artboard(doc, bounds, name);
    let mut layer = Node::layer(d.alloc_id(), name, vectorcraft_doc::LayerColor::Preset(0));
    if let Some(ch) = layer.children_mut() {
        *ch = nodes;
    }
    d.layers = vec![Arc::new(layer)];
    Some((d, bounds))
}

/// File → Export Selection: the selected objects alone, cropped to the bounds the renderer
/// measures for what they draw, their effects included ([`isolated`]). Objects on template layers
/// are guides, not artwork, and are left out.
pub(super) fn export_selection(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "document.exportSelection";
    if let Some(path) = str_param(p, "path") {
        super::check_not_lossy_overwrite(s.doc()?, path, p, C)?;
    }
    let path = str_param(p, "path");
    let f = writable(C, str_param(p, "format"), path)?;
    let expanded = expand_presets(s, C, p)?;
    let p = with_black_output(s, &expanded);
    let (d, bounds) = selection(s, C)?;
    // The encoder's warnings (options accepted but not applied, features approximated or left
    // out), as document.export reports them.
    let (bytes, warnings) = encode_with_warnings(&d, f.id, &without_artboards(&p))?;
    write_or_return(path, &bytes, json!({ "format": f.id, "warnings": warnings, "bounds": [bounds.x0, bounds.y0, bounds.width(), bounds.height()] }))
}

/// File → Save for Office Documents: one artboard as a PNG at `ppi`, on white unless
/// `transparent`.
pub(super) fn export_for_office(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "document.exportForOffice";
    if let Some(path) = str_param(p, "path") {
        super::check_not_lossy_overwrite(s.doc()?, path, p, C)?;
    }
    let path = str_param(p, "path");
    let ppi = f64_or(p, "ppi", 150.0);
    let doc = &s.doc()?.doc;
    let artboard = match p.get("artboard") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(C, "artboard is a 0-based index"))?,
    };
    if artboard >= doc.artboards.len() {
        return Err(bad(C, format!("no artboard {artboard} (0-based; the document has {})", doc.artboards.len())));
    }
    let background = if bool_or(p, "transparent", false) { "transparent" } else { "white" };
    let opts = with_black_output(s, &json!({ "ppi": ppi, "background": background, "artboard": artboard }));
    let bytes = encode(doc, "png", &opts)?;
    // The pixel size, from the PNG header.
    let side = |at: usize| bytes.get(at..at + 4).and_then(|b| b.try_into().ok()).map_or(0, u32::from_be_bytes);
    write_or_return(path, &bytes, json!({ "width": side(16), "height": side(20) }))
}
