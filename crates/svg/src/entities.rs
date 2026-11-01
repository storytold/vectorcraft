//! A cap on what the entities of an SVG's document type declaration expand to. The XML reader
//! replaces every `&name;` with the entity's text as it reads, with no total limit: a short file
//! that repeats a large entity in an attribute allocates far more than it holds.

use std::collections::HashMap;

/// The most text the references to entities may expand to in all (bytes).
const MAX_EXPANDED: usize = 16 << 20;

/// How deep entities that refer to entities are followed (the reader's own limit is lower).
const MAX_NESTING: u32 = 16;

/// The internal general entities declared in `svg`: name → value.
fn declared(svg: &str) -> HashMap<&str, &str> {
    let mut map = HashMap::new();
    for (at, _) in svg.match_indices("<!ENTITY") {
        let rest = svg.get(at + "<!ENTITY".len()..).unwrap_or("").trim_start();
        // A parameter entity (`%`) isn't referenced as `&name;`.
        if rest.starts_with('%') {
            continue;
        }
        let Some(end) = rest.find(char::is_whitespace) else { continue };
        let (name, rest) = rest.split_at(end);
        let rest = rest.trim_start();
        let Some(quote) = rest.chars().next().filter(|q| matches!(q, '"' | '\'')) else { continue };
        let Some(value) = rest.get(1..).and_then(|r| r.split_once(quote)).map(|(v, _)| v) else { continue };
        map.entry(name).or_insert(value);
    }
    map
}

/// The names of the `&name;` references in `text`.
fn references(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices('&').filter_map(|(at, _)| {
        let rest = text.get(at + 1..)?;
        // Look no further than the longest name: a file of `&`s without `;` would be read quadratically.
        let end = rest.bytes().take(257).position(|b| b == b';')?;
        let name = rest.get(..end)?;
        (!name.is_empty() && name.len() <= 256 && !name.starts_with('#') && !name.contains(char::is_whitespace)).then_some(name)
    })
}

/// What entity `name` expands to (bytes), references in its value followed.
fn expanded<'a>(name: &'a str, entities: &HashMap<&'a str, &'a str>, memo: &mut HashMap<&'a str, usize>, depth: u32) -> usize {
    if let Some(n) = memo.get(name) {
        return *n;
    }
    let Some(value) = entities.get(name) else { return 0 };
    if depth >= MAX_NESTING {
        return usize::MAX;
    }
    let mut total = value.len();
    for inner in references(value) {
        total = total.saturating_add(expanded(inner, entities, memo, depth + 1));
    }
    memo.insert(name, total);
    total
}

/// Whether the references to the entities `svg` declares expand to more than [`MAX_EXPANDED`]
/// bytes in all. False for a file that declares none.
pub(crate) fn expansion_too_large(svg: &str) -> bool {
    let entities = declared(svg);
    if entities.is_empty() {
        return false;
    }
    let mut memo = HashMap::new();
    let mut total = 0usize;
    for name in references(svg) {
        total = total.saturating_add(expanded(name, &entities, &mut memo, 0));
        if total > MAX_EXPANDED {
            return true;
        }
    }
    false
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
}
