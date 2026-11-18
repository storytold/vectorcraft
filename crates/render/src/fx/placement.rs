//! Stored gradient fits for geometry edits on the renderer's ordinary shape/container paths.

use vectorcraft_color::Paint;
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, Node, NodeKind};
use vectorcraft_effects::{self as effects, GeomContext};
use vectorcraft_geom::{BezPath, Rect, Shape};

use super::bounds::{apply_carrying, effected_path_carrying, item_effects, node_bezpath};
use super::predicates::has_non_container_art_fx;
use super::renderer::carried;

/// Store the current renderer fit of gradients that can be represented by the existing appearance.
///
/// Alternate materialized art, visible plug-ins, Rasterize and path-coordinate stroke gradients
/// keep their existing fit behavior. An unsupported or non-roundtripping carry also leaves its row
/// unchanged; this neither flattens art nor changes persisted effects. `None` makes no change.
pub fn pinned_appearance_for_geometry_edit(doc: &Document, n: &Node) -> Option<Appearance> {
    if !n.appearance.has_unplaced_gradient() || unsupported(n) || has_non_container_art_fx(n) {
        return None;
    }
    // Enclosing geometry effects change the fit outside this node-local pass.
    // Without that context, leave the paint fitted rather than storing a false frame.
    if doc.ancestry(n.id)?.into_iter().filter(|&id| id != n.id).any(|id| {
        doc.node(id).is_some_and(|ancestor| {
            matches!(ancestor.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) && effects::has_geometry(&ancestor.appearance.effects)
        })
    }) {
        return None;
    }
    match &n.kind {
        NodeKind::Group { .. } | NodeKind::Layer { .. } => effects::pinned_container_appearance(doc, n),
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => pinned_shape(doc, n),
        _ => None,
    }
}

fn rasterizes(fx: &[Effect]) -> bool {
    fx.iter().any(|effect| effect.visible && effect.id == effects::RASTERIZE)
}

fn unsupported(n: &Node) -> bool {
    effects::has_plugin_effect(n)
        || rasterizes(&n.appearance.effects)
        || n.appearance.items.iter().any(|item| {
            item.visible() && (rasterizes(item.effects()) || matches!(item, AppearanceItem::Stroke(stroke) if stroke.path_gradient().is_some()))
        })
}

fn unplaced(item: &AppearanceItem) -> bool {
    item.visible() && matches!(item.paint(), Paint::Gradient(gradient) if !gradient.is_placed())
}

fn pinned_shape(doc: &Document, n: &Node) -> Option<Appearance> {
    let bp = node_bezpath(n)?;
    let (geometry, object_carry) = effected_path_carrying(doc, n, &bp);
    if geometry.elements().is_empty() {
        return None;
    }
    // Share the renderer's actual temporary object carry and skipped-effect flags.
    let temporary = carried(n, &object_carry);
    let paints = temporary.as_deref().unwrap_or(n);
    let context = GeomContext { doc: Some(doc), ..GeomContext::of(paints) };
    let mut appearance = (*n.appearance).clone();
    let mut changed = false;
    for (index, original) in n.appearance.items.iter().enumerate().filter(|(_, item)| unplaced(item)) {
        // `item` must belong to the same temporary appearance borrowed by `context.item`:
        // that lookup uses pointer identity to recover the actual row index/seed.
        let item = paints.appearance.items.get(index)?;
        let (item_geometry, item_carry) = apply_carrying(item_effects(item), &geometry, &context.item(item));
        let bounds = item_bounds(item, &geometry, &item_geometry, &item_carry)?;
        if let Some(pinned) = effects::pinned_gradient_item(original, bounds, &[&object_carry, &item_carry]) {
            *appearance.items.get_mut(index)? = pinned;
            changed = true;
        }
    }
    changed.then_some(appearance)
}

/// The same temporary item carry/skipped flags and `ib` choice as `Renderer::paint_items`.
fn item_bounds(item: &AppearanceItem, geometry: &BezPath, item_geometry: &BezPath, carry: &effects::Carry) -> Option<Rect> {
    let mut appearance = Appearance { items: vec![item.clone()], ..Default::default() };
    if !carry.is_empty() && matches!(item.paint(), Paint::Gradient(_)) {
        carry.apply(&mut appearance);
    }
    let mut item = appearance.items.pop()?;
    if carry.skipped() {
        carry.hide_skipped(item.effects_mut());
    }
    Some(if effects::has_geometry(item_effects(&item)) && !item_geometry.elements().is_empty() {
        item_geometry.bounding_box()
    } else {
        geometry.bounding_box()
    })
}
