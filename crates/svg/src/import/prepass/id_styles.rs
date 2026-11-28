use super::{Edits, XNode, css_transform, roxmltree};
use crate::SvgError;

struct Element<'a, 'input>(XNode<'a, 'input>);

impl simplecss::Element for Element<'_, '_> {
    fn parent_element(&self) -> Option<Self> {
        self.0.parent_element().map(Self)
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.0.prev_sibling_element().map(Self)
    }

    fn has_local_name(&self, name: &str) -> bool {
        self.0.tag_name().name() == name
    }

    fn attribute_matches(&self, name: &str, operator: simplecss::AttributeOperator<'_>) -> bool {
        self.0.attribute(name).is_some_and(|value| operator.matches(value))
    }

    fn pseudo_class_matches(&self, class: simplecss::PseudoClass<'_>) -> bool {
        matches!(class, simplecss::PseudoClass::FirstChild) && self.0.prev_sibling_element().is_none()
    }
}

fn append(out: &mut String, declaration: &simplecss::Declaration<'_>, remaining: &mut usize) -> Result<(), SvgError> {
    let transform = if declaration.name == "transform" { css_transform::to_svg(declaration.value) } else { None };
    let value = transform.as_deref().unwrap_or(declaration.value);
    let important = if declaration.important { "!important" } else { "" };
    let text = crate::xml_escape(&format!("{}:{value}{important};", declaration.name));
    *remaining = remaining.checked_sub(text.len()).ok_or_else(|| SvgError::TooBig("expanded SVG styles exceed the import size limit".into()))?;
    out.push_str(&text);
    Ok(())
}

pub(super) fn preserve_matches(svg: &str, xml: &roxmltree::Document, edits: &mut Edits, warnings: &mut Vec<String>) -> Result<(), SvgError> {
    preserve_with_limit(svg, xml, edits, warnings, usize::try_from(crate::MAX_SVGZ).unwrap_or(usize::MAX))
}

fn preserve_with_limit(
    svg: &str,
    xml: &roxmltree::Document,
    edits: &mut Edits,
    warnings: &mut Vec<String>,
    mut remaining: usize,
) -> Result<(), SvgError> {
    let styles: Vec<_> = xml.descendants().filter(|n| n.has_tag_name("style") && n.attribute("type").is_none_or(|t| t == "text/css")).collect();
    let texts: Vec<String> = styles.iter().map(|n| n.children().filter(XNode::is_text).filter_map(|n| n.text()).collect()).collect();
    let mut sheet = simplecss::StyleSheet::new();
    for text in &texts {
        sheet.parse_more(text);
    }
    let element_count = xml.descendants().filter(XNode::is_element).count();
    // Too much matching: the file opens without this pass (nothing is edited yet), as it did before it.
    if element_count.checked_mul(sheet.rules.len()).is_none_or(|work| work > 1_000_000) {
        warnings.push("the style sheet is too large to match against every element: attribute selectors may not apply to linked objects".into());
        return Ok(());
    }
    for node in xml.descendants().filter(XNode::is_element) {
        let mut declarations = String::new();
        for rule in sheet.rules.iter().filter(|r| r.selector.matches(&Element(node))) {
            for declaration in &rule.declarations {
                append(&mut declarations, declaration, &mut remaining)?;
            }
        }
        if declarations.is_empty() {
            continue;
        }
        for declaration in simplecss::DeclarationTokenizer::from(node.attribute("style").unwrap_or_default()) {
            append(&mut declarations, &declaration, &mut remaining)?;
        }
        if let Some((_, _, previous)) = edits.styles.remove(&node.id()) {
            declarations.push_str(&previous);
        }
        if let Some(attribute) = node.attributes().find(|a| a.name() == "style" && a.namespace().is_none()) {
            edits.list.push((attribute.range_value(), declarations));
        } else {
            edits.style(svg, node, &declarations);
        }
    }
    for style in styles {
        edits.list.extend(style.children().map(|child| (child.range(), String::new())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_styles_are_bounded() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><style>circle {fill:red}</style><circle/><circle/></svg>"#;
        let xml = roxmltree::Document::parse(svg).unwrap();
        assert!(matches!(preserve_with_limit(svg, &xml, &mut Edits::default(), &mut vec![], 1), Err(SvgError::TooBig(_))));
        assert!(preserve_with_limit(svg, &xml, &mut Edits::default(), &mut vec![], 64).is_ok());
    }

    #[test]
    fn selector_matching_is_bounded_even_without_matches() {
        let rules = "circle[missing]{fill:red}".repeat(1001);
        let elements = "<circle/>".repeat(1000);
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><style>{rules}</style>{elements}</svg>"#);
        let xml = roxmltree::Document::parse(&svg).unwrap();
        let (mut edits, mut warnings) = (Edits::default(), vec![]);
        assert!(preserve_matches(&svg, &xml, &mut edits, &mut warnings).is_ok(), "the file still opens");
        assert!(edits.list.is_empty() && edits.styles.is_empty(), "nothing is edited");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn stylesheet_text_after_comments_is_preserved() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><style>circle[id]{fill:red}<!-- -->text{font-size:30px}</style><text>x</text></svg>"#;
        let xml = roxmltree::Document::parse(svg).unwrap();
        let mut edits = Edits::default();
        preserve_matches(svg, &xml, &mut edits, &mut vec![]).unwrap();
        let output = edits.apply(svg);
        let xml = roxmltree::Document::parse(&output).unwrap();
        let text = xml.descendants().find(|n| n.has_tag_name("text")).unwrap();
        assert_eq!(text.attribute("style"), Some("font-size:30px;"));
    }
}
