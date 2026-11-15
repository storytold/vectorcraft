//! `vector-effect="non-scaling-stroke"`: the stroke keeps its width (and dashes) in screen space
//! whatever the transforms above it. usvg knows the property but drops it, so [`find`] records
//! the elements that ask for it before usvg runs. Their transforms are baked into the path, so a
//! document-space stroke of the screen width is exact; type and the art usvg drops ids from
//! (patterns, markers, `<use>` copies) can't always keep it, and a warning says so.

use std::collections::HashSet;

use craft_svg::tree::usvg::roxmltree;
use vectorcraft_doc::TextObject;
use vectorcraft_geom::Affine;

use super::css::{Styles, XNode};
use super::{Edits, href};

/// The property value this module reads.
pub(super) const NON_SCALING: &str = "non-scaling-stroke";

/// The shapes the property applies to (`<text>` is read by the text import).
const SHAPES: &[&str] = &["path", "rect", "circle", "ellipse", "line", "polyline", "polygon"];

/// Does element `n` ask for a non-scaling stroke?
pub(super) fn is_non_scaling(css: &Styles, n: XNode) -> bool {
    css.own(n, "vector-effect").is_some_and(|v| v.trim() == NON_SCALING)
}

/// Record the ids of the shapes with a non-scaling stroke (made up for those without one) in
/// `ids`, and warn about those shown where usvg drops ids: their strokes scale like others.
pub(super) fn find(svg: &str, xml: &roxmltree::Document, css: &Styles, edits: &mut Edits, ids: &mut HashSet<String>, warnings: &mut Vec<String>) {
    let reused: HashSet<&str> =
        xml.descendants().filter(|n| n.tag_name().name() == "use").filter_map(href).filter_map(|h| h.trim().strip_prefix('#')).collect();
    for n in xml.descendants().filter(|n| n.is_element() && SHAPES.contains(&n.tag_name().name()) && is_non_scaling(css, *n)) {
        // usvg drops the ids of marker and `<use>` copies; a pattern's paint scales its art.
        let placed = n.ancestors().any(|a| matches!(a.tag_name().name(), "pattern" | "marker" | "symbol"));
        if placed || n.ancestors().any(|a| a.attribute("id").is_some_and(|id| reused.contains(id))) {
            let label = n.attribute("id").map_or_else(|| format!("a {}", n.tag_name().name()), |id| format!("'{id}'"));
            let w = format!("non-scaling stroke on {label} in a pattern, marker or reused element: it scales with the transform there");
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        if !placed {
            ids.insert(edits.id(svg, n));
        }
    }
}

/// Is `m` free of non-uniform scaling and skew (a uniform scale, rotation and reflection)?
pub(super) fn conformal(m: Affine) -> bool {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let tol = 1e-9 * (a.abs() + b.abs() + c.abs() + d.abs()).max(1e-12);
    ((a - d).abs() <= tol && (b + c).abs() <= tol) || ((a + d).abs() <= tol && (b - c).abs() <= tol)
}

/// Give the character strokes of `t` (whose `xf` maps text space to the document) the widths and
/// dashes they have in screen space, `screen` points per screen pixel. `false` when the text is
/// stretched or skewed, so its strokes can only be approximated.
pub(super) fn unscale_text(t: &mut TextObject, screen: f64) -> bool {
    let k = screen / t.xf.determinant().abs().sqrt();
    if !k.is_finite() || k <= 0.0 {
        return false;
    }
    t.scale_char_strokes(k);
    conformal(t.xf) || !t.runs.iter().any(|r| r.style.has_stroke())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conformal_transforms_keep_circles_round() {
        assert!(conformal(Affine::scale(3.0)));
        assert!(conformal(Affine::rotate(0.7) * Affine::scale(2.0)));
        assert!(conformal(Affine::scale_non_uniform(-2.0, 2.0)));
        assert!(!conformal(Affine::scale_non_uniform(2.0, 1.0)));
        assert!(!conformal(Affine::skew(0.5, 0.0)));
    }
}
