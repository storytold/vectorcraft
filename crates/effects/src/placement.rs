//! Conservative stored placements for gradients fitted by the existing effects passes.

use vectorcraft_doc::color::{GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem};
use vectorcraft_geom::Rect;

use crate::Carry;

/// Freeze one fitted gradient at the evaluated row's geometric bounds, before its live effects.
/// `carries` are the actual object and item passes, in rendering order. Only missing placement
/// fields are returned; explicit coordinates and the row's other data stay unchanged. A map or
/// model that cannot reproduce the original resolved row returns `None`, leaving it fitted.
pub fn pinned_gradient_item(item: &AppearanceItem, bounds: Rect, carries: &[&Carry]) -> Option<AppearanceItem> {
    if gradient(item)?.is_placed() {
        return Some(item.clone());
    }
    if ![bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let resolved = resolve(item, bounds, carries);
    if !finite_placement(resolved.items.first()?) {
        return None;
    }
    let mut stored = resolved.clone();
    for carry in carries.iter().rev() {
        carry.unapply_placed(&mut stored)?;
    }
    if !finite_placement(stored.items.first()?) {
        return None;
    }
    let candidate = missing_placement(item, stored.items.first()?)?;
    let replayed = resolve(&candidate, bounds, carries);
    (finite_placement(replayed.items.first()?) && replayed.approx_eq(&resolved)).then_some(candidate)
}

fn gradient(item: &AppearanceItem) -> Option<&GradientPaint> {
    match item.paint() {
        Paint::Gradient(g) => Some(g),
        _ => None,
    }
}

fn resolve(item: &AppearanceItem, bounds: Rect, carries: &[&Carry]) -> Appearance {
    let mut ap = Appearance { items: vec![item.clone()], ..Default::default() };
    for carry in carries {
        carry.apply(&mut ap);
    }
    ap.pin_gradients(bounds);
    ap
}

fn missing_placement(item: &AppearanceItem, placed: &AppearanceItem) -> Option<AppearanceItem> {
    let original = gradient(item)?;
    let placed = gradient(placed)?;
    let mut out = item.clone();
    let Paint::Gradient(g) = out.paint_mut() else { return None };
    if original.geom.is_none() {
        g.geom = placed.geom;
    }
    if original.gradient.kind == GradientKind::Freeform && !original.has_freeform_points() {
        let freeform = placed.freeform.as_ref()?;
        match &mut g.freeform {
            Some(f) => f.points.clone_from(&freeform.points),
            None => g.freeform = Some(freeform.clone()),
        }
    }
    Some(out)
}

fn finite_placement(item: &AppearanceItem) -> bool {
    let Some(g) = gradient(item) else { return false };
    g.geom.is_none_or(|v| v.start.is_finite() && v.end.is_finite() && v.aspect.is_finite() && v.focal.is_none_or(|p| p.is_finite()))
        && g.freeform.as_ref().is_none_or(|f| f.points.iter().all(|p| p.at.is_finite()))
}
