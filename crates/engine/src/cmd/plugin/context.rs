//! A plug-in run's document context (`_context.document`): the parts of the active document its
//! manifest's `context` asks for ([`ContextSection`]), read when the run is prepared, with or
//! without a selection. It holds plain values only: names, flags, rectangles and colors, never an
//! object, a file path or a link target.

use serde_json::{Value, json};
use vectorcraft_doc::{ColorMode, LayerColor, Node, NodeKind};
use vectorcraft_plugins::ContextSection;

use super::super::{Result, bad};
use crate::DocState;

/// Most rows (artboards, layers and sublayers, and swatches together) a document context holds.
pub const MAX_CONTEXT_ITEMS: usize = 10_000;

/// Most bytes the rows of a document context take as JSON.
pub const MAX_CONTEXT_BYTES: usize = 4 * 1024 * 1024;

/// Instructions each row costs: they come off the run's instruction budget.
pub const CONTEXT_FUEL_PER_ITEM: u64 = 1_000;

/// A document context, and the rows it holds.
pub(super) struct DocumentContext {
    pub value: Value,
    pub items: usize,
}

/// The document context of `sections` for a run of command `c` on `st`: `{revision}` and a key
/// for each section asked for. A document with more than [`MAX_CONTEXT_ITEMS`] rows in them, or
/// rows of more than [`MAX_CONTEXT_BYTES`], is an error, before any row is made in the first case.
pub(super) fn document_context(st: &DocState, sections: &[ContextSection], c: &str) -> Result<DocumentContext> {
    let d = &st.doc;
    let layers = if sections.contains(&ContextSection::Layers) { layer_rows(d) } else { Vec::new() };
    let swatches = || super::super::swatch::listed_swatches(d, true, d.swatch_groups.iter().collect());
    let mut items = 0;
    for s in sections {
        items += match s {
            ContextSection::Settings => 0,
            ContextSection::Artboards => d.artboards.len(),
            ContextSection::Layers => layers.len(),
            ContextSection::Swatches => swatches().count(),
        };
    }
    let too_large = || {
        bad(
            c,
            format!(
                "the document information the plug-in asks for is too large to hand over: at most {MAX_CONTEXT_ITEMS} artboards, layers and swatches in {MAX_CONTEXT_BYTES} bytes"
            ),
        )
    };
    if items > MAX_CONTEXT_ITEMS {
        return Err(too_large());
    }
    let mut bytes = 0usize;
    let mut row = |v: Value| -> Result<Value> {
        bytes = bytes.saturating_add(serde_json::to_vec(&v).map_or(usize::MAX, |b| b.len() + 1));
        if bytes > MAX_CONTEXT_BYTES { Err(too_large()) } else { Ok(v) }
    };
    let active = active_artboard(st);
    let mut out = json!({"revision": st.revision});
    for s in sections {
        let value = match s {
            ContextSection::Settings => {
                let board = active.and_then(|i| d.artboards.get(i));
                row(json!({
                    "name": st.title(),
                    "colorMode": match d.color_mode {
                        ColorMode::Rgb => "rgb",
                        ColorMode::Cmyk => "cmyk",
                    },
                    "units": d.units.label(),
                    "width": board.map(|a| a.rect.width()),
                    "height": board.map(|a| a.rect.height()),
                    "bleed": d.setup.bleed,
                    "rasterEffectsPpi": d.raster_ppi(),
                    "activeArtboard": board.map(|a| a.id),
                }))?
            }
            ContextSection::Artboards => {
                let rows = d.artboards.iter().enumerate().map(|(i, a)| {
                    let r = a.rect;
                    row(json!({"id": a.id, "index": i, "name": a.name, "rect": [r.x0, r.y0, r.x1, r.y1], "active": active == Some(i)}))
                });
                Value::Array(rows.collect::<Result<_>>()?)
            }
            ContextSection::Layers => {
                let rows = layers.iter().map(|l| {
                    let n = l.node;
                    row(json!({
                        "id": n.id.0, "parentId": l.parent, "depth": l.depth, "name": n.display_name(),
                        "visible": n.visible, "locked": n.locked, "template": l.template, "color": l.color.label_text(),
                        "sublayers": l.sublayers, "current": st.active_layer == Some(n.id),
                    }))
                });
                Value::Array(rows.collect::<Result<_>>()?)
            }
            ContextSection::Swatches => {
                let rows = swatches().map(|(w, g)| row(super::super::swatch::swatch_json(w, g)));
                Value::Array(rows.collect::<Result<_>>()?)
            }
        };
        out[s.name()] = value;
    }
    Ok(DocumentContext { value: out, items })
}

/// Is `n` a layer or sublayer?
fn is_layer(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Layer { .. })
}

/// A layer or sublayer, where it is in the layer tree, and what its row reports besides the
/// node's own name and flags.
struct LayerRow<'a> {
    node: &'a Node,
    /// Its parent layer's id; `None` for a top-level layer.
    parent: Option<u64>,
    /// 0 for a top-level layer, 1 for its sublayers, and so on.
    depth: usize,
    color: LayerColor,
    template: bool,
    /// The sublayers it holds directly.
    sublayers: usize,
}

/// Every layer and sublayer of `d`, depth first in document order (bottom first within each
/// parent). The walk keeps its own stack, so deep nesting can't overflow the thread's.
fn layer_rows(d: &vectorcraft_doc::Document) -> Vec<LayerRow<'_>> {
    let mut out = Vec::new();
    let mut stack: Vec<(&Node, Option<u64>, usize)> = d.layers.iter().rev().map(|l| (l.as_ref(), None, 0)).collect();
    while let Some((node, parent, depth)) = stack.pop() {
        let NodeKind::Layer { color, template, children, .. } = &node.kind else { continue };
        let sublayers = children.iter().filter(|c| is_layer(c)).count();
        stack.extend(children.iter().rev().filter(|c| is_layer(c)).map(|c| (c.as_ref(), Some(node.id.0), depth + 1)));
        out.push(LayerRow { node, parent, depth, color: *color, template: *template, sublayers });
    }
    out
}

/// The index of the view's active artboard ([`DocState::active_artboard`]): when the document no
/// longer has it, the artboard the document opened on, else the first; `None` with no artboards.
fn active_artboard(st: &DocState) -> Option<usize> {
    let d = &st.doc;
    let at = |id: Option<u32>| id.and_then(|id| d.artboards.iter().position(|a| a.id == id));
    at(st.active_artboard).or_else(|| at(d.opened_artboard)).or((!d.artboards.is_empty()).then_some(0))
}
