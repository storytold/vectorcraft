//! CSS `transform` declarations in SVG syntax, for usvg.
//!
//! usvg takes the `transform` property (from a `style` attribute or a `<style>` rule) as a
//! presentation attribute that overrides the element's `transform` attribute, but reads its value
//! as SVG, without CSS units: `translate(100px, 0)` or `rotate(45deg)` fails, and the element is
//! left with no transform at all (#1557). Before usvg runs, such values are rewritten in SVG
//! syntax: lengths in `px` (or unitless), angles in `deg`, `rad`, `grad` or `turn`, and the CSS
//! functions SVG lacks (`translateX`, `scaleY`, `skew`…) spelled with SVG's. A value that can't be
//! (percentages, `em`, 3D functions) is left as it is.

use std::ops::Range;

use craft_svg::tree::usvg::roxmltree;

use super::css::XNode;

/// The CSS transform `value` in SVG syntax; `None` when it has nothing to rewrite or can't be.
pub(super) use craft_svg::transform_compat::css_transform_to_svg as to_svg;

/// Where CSS can be in `svg`: `style` attribute values and `<style>` contents.
fn css_ranges(xml: &roxmltree::Document) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for n in xml.descendants().filter(XNode::is_element) {
        if let Some(a) = n.attributes().find(|a| a.name() == "style" && a.namespace().is_none()) {
            out.push(a.range_value());
        }
        if n.tag_name().name() == "style" {
            out.extend(n.children().filter(XNode::is_text).map(|t| t.range()));
        }
    }
    out
}

/// The `transform` declarations in CSS in `svg`, each as (value range, value in SVG syntax).
pub(super) fn rewrites(svg: &str, xml: &roxmltree::Document) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    for r in css_ranges(xml) {
        let Some(css) = svg.get(r.clone()) else { continue };
        let mut from = 0;
        while let Some(i) = css.get(from..).and_then(|s| s.find("transform")).map(|i| i + from) {
            from = i + "transform".len();
            let before = css.get(..i).and_then(|s| s.chars().next_back());
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                continue;
            }
            let after = css.get(from..).unwrap_or("");
            let Some(colon) = after.find(|c: char| !c.is_whitespace()).filter(|&k| after.get(k..).is_some_and(|s| s.starts_with(':'))) else {
                continue;
            };
            let start = from + colon + 1;
            let len = css.get(start..).and_then(|s| s.find([';', '}', '!', '"', '\'', '<'])).unwrap_or(css.len() - start);
            let Some(value) = css.get(start..start + len) else { continue };
            if let Some(svg_value) = to_svg(value) {
                out.push((r.start + start..r.start + start + len, format!(" {svg_value}")));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_transforms_become_svg() {
        assert_eq!(to_svg("translate(100px, 100px)").as_deref(), Some("translate(100 100)"));
        assert_eq!(to_svg(" translateX(5px) rotate(0.25turn) ").as_deref(), Some("translate(5 0) rotate(90)"));
        assert_eq!(to_svg("rotate(1.5707963267948966rad) scaleY(2)").as_deref(), Some("rotate(90) scale(1 2)"));
        assert_eq!(to_svg("skew(45deg)").as_deref(), Some("skewX(45)"));
        assert_eq!(to_svg("matrix(1, 0, 0, 1, 10px, 20px)").as_deref(), Some("matrix(1 0 0 1 10 20)"));
        assert_eq!(to_svg("none").as_deref(), Some("matrix(1 0 0 1 0 0)"));
        // Already SVG, or not expressible: left alone.
        assert_eq!(to_svg("translate(10 20)"), None);
        assert_eq!(to_svg("translate(50%, 0)"), None);
        assert_eq!(to_svg("rotate3d(1, 0, 0, 45deg)"), None);
        assert_eq!(to_svg("translate(1px"), None);
    }

    #[test]
    fn only_css_is_rewritten() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><style>.a { fill: red; transform: rotate(45deg) } .b{-webkit-transform:rotate(1deg)}</style><rect class="a" transform="translate(1 2)" style="transform:translate(3px,4px);fill:blue"/><text>transform: rotate(9deg)</text></svg>"#;
        let xml = roxmltree::Document::parse(svg).unwrap();
        let found = rewrites(svg, &xml);
        let r: Vec<(&str, &str)> = found.iter().map(|(r, v)| (&svg[r.clone()], v.as_str())).collect();
        assert_eq!(r, vec![(" rotate(45deg) ", " rotate(45)"), ("translate(3px,4px)", " translate(3 4)")]);
    }
}
