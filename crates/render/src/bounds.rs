use super::*;

/// Bounds of everything `n` paints, as the renderer culls it: its members, strokes, brush art and
/// geometry effects, and the shadows and glows of it and its members. Symbol instances are
/// measured by their art in `doc` ([`Node::visual_bounds_with`]).
pub fn painted_bounds(doc: &Document, n: &Node) -> Option<Rect> {
    brush_fx::cull_bounds(&doc.symbol_boxes(), n)
}

/// [`painted_bounds`] of `n` without the room it leaves for brush art past the brushed strokes of
/// `n` and of a symbol instance's art (the brush art of the objects in a blend, envelope or repeat
/// still counts). A group or layer without effects of its own is measured by its members, leaving
/// out the transparent source art of a Shaper group as [`Node::visual_bounds_with`] does. Symbol
/// instances are measured by their art in `symbols`.
pub fn effect_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    match &n.kind {
        NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } if !fx::has_object_fx(n) => children
            .iter()
            .skip(usize::from(n.extras().shaper.is_some()))
            .fold(None, |acc, c| vectorcraft_geom::union_opt(acc, effect_bounds(symbols, c))),
        _ => fx::cull_bounds(symbols, n),
    }
}

/// Bounds of what `n` draws with its own and its members' geometry effects applied and with the
/// reach of its own and its members' raster effects, measured as [`Node::visual_bounds_with`]
/// measures art without effects: what Object › Rasterize makes its image over.
pub fn drawn_bounds(doc: &Document, n: &Node) -> Option<Rect> {
    fx::painted_reach(&doc.symbol_boxes(), n)
}

/// The outline, with `n`'s geometry effects applied, that the canvas clips `n`'s Rasterize with
/// Create Clipping Mask to. Object › Rasterize's clipping mask follows it.
pub fn drawn_outline(doc: &Document, n: &Node) -> Option<(BezPath, FillRule)> {
    fx::drawn_outline(doc, n)
}
