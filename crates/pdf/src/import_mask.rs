//! Soft masks → opacity masks. A luminosity mask's art becomes the mask art; an alpha mask's art
//! is turned white (its opacity kept), so its luminance over the black backdrop is its alpha.
//! The backdrop colour gives Clip (black: outside the art is hidden), an inverting transfer
//! function gives Invert, and the rectangles the PDF export draws for those (a backdrop cover, a
//! white Difference cover) read back as the options rather than as art. An alpha mask that is one
//! rectangle of constant opacity over everything is plain opacity.

use std::sync::Arc;

use kurbo::{Rect, Shape};
use vectorcraft_color::blend::MASK_LUM;
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::node::whiten;
use vectorcraft_doc::{AppearanceItem, Node, NodeKind, OpacityMask};

/// What a soft mask does to what it masks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MaskSpec {
    /// A constant opacity (a group drawn through it is not isolated).
    Opacity(f32),
    Mask(OpacityMask),
}

/// The luminance of `c` (0 black … 1 white), as opacity masks weigh it.
pub(crate) fn luminance(c: &Color) -> f32 {
    let rgb = c.to_rgb();
    rgb.iter().zip(MASK_LUM).map(|(v, w)| v * w).sum()
}

/// A rectangle (at most 6 path elements, its area that of its bounds).
pub(crate) fn is_rectangle(p: &kurbo::BezPath) -> bool {
    let b = p.bounding_box();
    p.elements().len() <= 6 && b.area() > 0.0 && (p.area().abs() - b.area()).abs() <= b.area() * 1e-3
}

/// A path covering `page` filled with one solid colour and no stroke → its colour and opacity.
fn cover(n: &Node, page: Rect) -> Option<(Color, f32)> {
    let NodeKind::Path { path, .. } = &n.kind else { return None };
    let [AppearanceItem::Fill(f)] = n.appearance.items.as_slice() else { return None };
    let Paint::Solid { color, .. } = &f.paint else { return None };
    let b = path.to_bezpath();
    let covers = page.area() > 1.0 && contains(b.bounding_box(), page.inset(-0.5)) && is_rectangle(&b);
    (covers && n.extras().mask.is_none()).then_some((*color, f.opacity * n.opacity))
}

/// An opaque white rectangle covering `page`, filled only.
pub(crate) fn white_cover(n: &Node, page: Rect) -> bool {
    cover(n, page).is_some_and(|(c, a)| a >= 0.999 && luminance(&c) > 0.999)
}

/// Does `outer` contain `inner`?
pub(crate) fn contains(outer: Rect, inner: Rect) -> bool {
    outer.x0 <= inner.x0 && outer.y0 <= inner.y0 && outer.x1 >= inner.x1 && outer.y1 >= inner.y1
}

/// How a soft mask drawing `art` acts: `alpha` for an alpha mask, `backdrop` the luminance outside
/// the art, `inverted` when its transfer function inverts it. `page`: the area masks cover.
pub(crate) fn mask_spec(
    mut art: Vec<Arc<Node>>,
    alpha: bool,
    backdrop: f32,
    inverted: bool,
    page: Rect,
    group: impl FnOnce(Vec<Arc<Node>>) -> Node,
) -> MaskSpec {
    if alpha {
        if let [only] = art.as_slice()
            && let Some((_, a)) = cover(only, page)
        {
            return MaskSpec::Opacity(a);
        }
        art.iter_mut().for_each(|n| whiten(Arc::make_mut(n)));
    }
    let mut clip = alpha || backdrop < 0.5;
    let mut invert = inverted;
    // The PDF export's covers: a white Difference rectangle on top inverts, an opaque one at the
    // bottom is the backdrop (white: no clip).
    if !alpha {
        if art.len() > 1
            && let Some(last) = art.last()
            && last.blend == BlendMode::Difference
            && white_cover(last, page)
        {
            art.pop();
            invert = !invert;
        }
        if art.len() > 1
            && let Some(first) = art.first()
            && first.blend == BlendMode::Normal
            && let Some((c, a)) = cover(first, page)
            && a >= 0.999
        {
            clip = luminance(&c) < 0.5;
            art.remove(0);
        }
    }
    let art = match <[Arc<Node>; 1]>::try_from(art) {
        Ok([one]) => Arc::unwrap_or_clone(one),
        Err(all) => group(all),
    };
    MaskSpec::Mask(OpacityMask { invert, ..OpacityMask::new(art, clip) })
}
