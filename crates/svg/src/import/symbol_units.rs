//! Percentage lengths inside a `<symbol>` with a `viewBox`, as numbers, for usvg.
//!
//! A percentage is of the nearest viewport's size in user units, and a symbol's `viewBox` sets
//! that size for its content as a nested `<svg>`'s does (SVG 2, 8.9 Units; 5.6 The `use` element).
//! usvg takes the percentages of the `<use>`'s width and height instead and then scales the art by
//! the `viewBox`, so it comes out too large and out of place (#1561). Before usvg runs, the
//! percentages of the shapes' geometry are rewritten in the `viewBox`'s units.

use std::ops::Range;
use std::str::FromStr;

use usvg::roxmltree;

/// The elements whose geometry attributes are rewritten.
const SHAPES: &[&str] = &["rect", "circle", "ellipse", "line", "image", "use", "text"];
/// The elements whose content has lengths of its own kind (or a viewport of its own).
const SCOPES: &[&str] = &["symbol", "svg", "pattern", "mask", "clipPath", "marker"];

/// What a percentage of attribute `name` is of, in a viewport of `w` × `h`: its width, its height,
/// or (a radius) its diagonal over √2.
fn basis(name: &str, w: f64, h: f64) -> Option<f64> {
    match name {
        "x" | "x1" | "x2" | "cx" | "width" | "rx" => Some(w),
        "y" | "y1" | "y2" | "cy" | "height" | "ry" => Some(h),
        "r" => Some(((w * w + h * h) / 2.0).sqrt()),
        _ => None,
    }
}

/// The percentage geometry of shapes in symbols with a `viewBox`, each as (value range, number).
pub(super) fn rewrites(xml: &roxmltree::Document) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    for n in xml.descendants().filter(|n| n.is_element() && SHAPES.contains(&n.tag_name().name())) {
        let Some(scope) = n.ancestors().skip(1).find(|a| SCOPES.contains(&a.tag_name().name())) else { continue };
        let Some(vb) = Some(scope)
            .filter(|s| s.tag_name().name() == "symbol")
            .and_then(|s| s.attribute("viewBox"))
            .and_then(|v| svgtypes::ViewBox::from_str(v).ok())
        else {
            continue;
        };
        if !(vb.w > 0.0 && vb.h > 0.0 && vb.w.is_finite() && vb.h.is_finite()) {
            continue;
        }
        for a in n.attributes().filter(|a| a.namespace().is_none()) {
            let percent = a.value().trim().strip_suffix('%').and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
            if let (Some(p), Some(of)) = (percent, basis(a.name(), vb.w, vb.h)) {
                out.push((a.range_value(), (p / 100.0 * of).to_string()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_in_a_symbol_are_of_its_view_box() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg"><rect width="50%"/>
            <symbol id="a" viewBox="0 0 200 100"><rect x="10%" y="10%" width="100%" height="50%" rx="5"/><circle cx="50%" r="100%"/>
              <linearGradient x2="100%"/><pattern><rect width="100%"/></pattern><g><line x2="25%" y2=" 25% "/></g></symbol>
            <symbol id="b"><rect width="100%"/></symbol><use href="#a" width="30%"/></svg>"##;
        let xml = roxmltree::Document::parse(svg).unwrap();
        let found: Vec<(&str, String)> = rewrites(&xml).into_iter().map(|(r, v)| (&svg[r], v)).collect();
        let r = (200.0f64 * 200.0 / 2.0 + 100.0 * 100.0 / 2.0).sqrt().to_string();
        let expected =
            [("10%", "20"), ("10%", "10"), ("100%", "200"), ("50%", "50"), ("50%", "100"), ("100%", r.as_str()), ("25%", "50"), (" 25% ", "25")];
        assert_eq!(found.iter().map(|(a, b)| (*a, b.as_str())).collect::<Vec<_>>(), expected);
    }
}
