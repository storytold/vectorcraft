//! A cap on what the entities of an SVG's document type declaration expand to. The XML reader
//! replaces every `&name;` with the entity's text as it reads, with no total limit: a short file
//! that repeats a large entity in an attribute allocates far more than it holds.

/// Whether the references to the entities `svg` declares expand to more than the limit
/// bytes in all. False for a file that declares none.
pub(crate) fn expansion_too_large(svg: &str) -> bool {
    craft_svg::guard::entities_expand_past(svg, craft_svg::guard::MAX_ENTITY_EXPANSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_large_entity_is_too_large_and_a_namespace_entity_is_not() {
        let big = "x".repeat(1 << 20);
        let refs = "&a;".repeat(20);
        let bomb = format!("<!DOCTYPE svg [<!ENTITY a \"{big}\">]><svg xmlns=\"http://www.w3.org/2000/svg\" id=\"{refs}\"/>");
        assert!(expansion_too_large(&bomb));
        let nested = format!("<!DOCTYPE svg [<!ENTITY a \"{big}\"><!ENTITY b \"{}\">]><svg id=\"{}\"/>", "&a;".repeat(8), "&b;".repeat(8));
        assert!(expansion_too_large(&nested));
        let ns = "<!DOCTYPE svg [<!ENTITY ns_svg \"http://www.w3.org/2000/svg\">]><svg xmlns=\"&ns_svg;\"/>";
        assert!(!expansion_too_large(ns));
        assert!(!expansion_too_large("<svg a=\"&amp;&#10;\"/>"));
        // Many `&`s with no `;` after them are read in linear time.
        let ampersands = format!("<!DOCTYPE svg [<!ENTITY a \"x\">]><svg id=\"{}\"/>", "&".repeat(1 << 20));
        let start = std::time::Instant::now();
        assert!(!expansion_too_large(&ampersands));
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
    }

    #[test]
    fn declarations_in_comments_and_long_names_cannot_hide_a_large_entity() {
        let big = "x".repeat(1 << 20);
        let refs = "&a;".repeat(20);
        let commented = format!("<!DOCTYPE svg [<!-- <!ENTITY a \"x\"> --><!ENTITY a \"{big}\">]><svg id=\"{refs}\"/>");
        assert!(expansion_too_large(&commented));
        let in_value = format!("<!DOCTYPE svg [<!ENTITY c \"<!ENTITY a 'x'>\"><!ENTITY a \"{big}\">]><svg id=\"{refs}\"/>");
        assert!(expansion_too_large(&in_value));
        let long = "n".repeat(257);
        let long_name = format!("<!DOCTYPE svg [<!ENTITY {long} \"x\">]><svg id=\"&{long};\"/>");
        assert!(expansion_too_large(&long_name));
    }
}
