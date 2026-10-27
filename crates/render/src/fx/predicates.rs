use super::bounds::drawn_bounds;
use super::*;

fn visible(effects: &[Effect]) -> bool {
    effects.iter().any(|e| e.visible)
}

/// Does this node need the effects path (a path or compound path with visible effects)?
pub(crate) fn has_fx(n: &Node) -> bool {
    if !matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. }) {
        return false;
    }
    visible(&n.appearance.effects)
        || n.appearance.items.iter().any(|i| match i {
            AppearanceItem::Fill(f) => visible(&f.effects),
            AppearanceItem::Stroke(s) => visible(&s.effects),
        })
}

/// Does `n` carry visible object-level effects that only apply through its art (type, images,
/// symbol instances, live objects; Crop Marks and colour adjustments on anything), is it type with
/// a geometry effect on a fill or stroke of its own or an image with a fill or stroke of its own
/// that paints a shape ([`effects::has_item_geometry`]), is it a path whose Transform effect copies
/// it with a gradient or changes its fills and strokes ([`effects::has_transform_art`]), or is it a
/// group or layer with an appearance of its own (see [`Renderer::draw_object_fx`])?
pub(crate) fn has_object_fx(n: &Node) -> bool {
    (effects::needs_outline(n) && (visible(&n.appearance.effects) || n.projection().is_some()))
        || effects::has_item_geometry(n)
        || effects::has_transform_art(n)
        || effects::has_container_appearance(n)
        || effects::has_crop_marks(n)
        || effects::has_adjustment(n)
        || effects::has_revolve(n)
        || effects::has_solid(n)
}

/// Is `n` a group or layer (whose evaluated art keeps its knockout setting)?
pub(super) fn is_container(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. })
}

/// Can the art [`Renderer::fx_art`] makes for `n` be made from live objects it evaluates? It can
/// when `n` is a live object, and when `n` is a group or layer without a Pathfinder effect that
/// holds live objects and whose geometry effects reshape them or whose own fills and strokes paint
/// their evaluated outlines ([`effects::evaluate_container`]).
pub(super) fn evaluates_live(n: &Node) -> bool {
    use vectorcraft_doc::live::{is_live, subtree_has_live};
    let to_members = effects::has_geometry(&n.appearance.effects) || n.appearance.items.iter().any(effects::paints);
    is_live(n) || (is_container(n) && to_members && !effects::has_pathfinder(n) && subtree_has_live(n))
}

/// Can the art [`Renderer::fx_art`] makes for `n` change while `n` stays the same, other than
/// through live objects it evaluates? It can when `n` holds type (laid out in the fonts installed),
/// a symbol instance (made from its symbol's art), or an object with a 3D effect or a color
/// adjustment (shaded or recolored through the color settings), in it or in the opacity masks of
/// the objects in it.
pub(super) fn reads_outside(n: &Node) -> bool {
    let mut reads = false;
    n.walk(&mut |c| {
        reads |= matches!(c.kind, NodeKind::Text(_) | NodeKind::SymbolInstance { .. })
            || effects::has_revolve(c)
            || effects::has_solid(c)
            || effects::has_adjustment(c)
            || c.extras().mask.as_ref().is_some_and(|m| reads_outside(&m.art))
    });
    reads
}

/// The [`Content::bounds`] of `n` under raster effects `rfx`: its [`drawn_bounds`] when `rfx`
/// holds a Rasterize.
pub(super) fn rasterize_bounds(symbols: &SymbolBoxes, n: &Node, rfx: &[RasterFx]) -> Option<Rect> {
    rfx.iter().any(|fx| matches!(fx, RasterFx::Rasterize(_))).then(|| drawn_bounds(symbols, n)).flatten()
}

/// `f` with the content moved by (`dx`, `dy`) document units (a shadow's offset).
pub(super) fn shifted<'a>(f: &Frame<'a>, dx: f64, dy: f64) -> Frame<'a> {
    Frame { view: f.view * Affine::translate((dx, dy)), visible: f.visible - Vec2::new(dx, dy), ..*f }
}
