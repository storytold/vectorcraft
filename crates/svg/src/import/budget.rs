//! What usvg makes of an SVG, measured in the XML before usvg runs: how many elements it makes,
//! and how many bytes of style values it copies into them.
//!
//! usvg makes an element of each SVG element, and of each element a `<use>` shows a copy of, by
//! copying what the `<use>` refers to. Each copy gets its own copy of every declaration the style
//! sheets and the `style` attribute give the element, so a long style on an element that `<use>`s
//! copy, again and again in a chain, or that a style sheet rule gives to many elements, takes more
//! memory than the file holds many times over. usvg stops past 1,000,000 elements, but only after
//! it has made them and copied their styles. An SVG past [`MAX_SVG_ELEMENTS`] elements or
//! [`MAX_COPIED_STYLES`] bytes of copied styles is refused here instead.
//!
//! The copies are counted, never made: each element's count is worked out once and added up, so a
//! `<use>` of an element costs one addition, however many elements its copy holds. References
//! follow usvg 0.48.1 (`parser/svgtree/parse.rs`): an id names the first element that has it, an
//! unprefixed `href` wins over `xlink:href`, and a `<use>` of itself, of the `<use>` its own copy
//! starts from, or of an element holding a `<use>` of the element or of the `<use>`, is left out.
//! Any other loop of references makes usvg copy without end, so an SVG with one is refused. usvg
//! looks for those `<use>`s inside an element only in the SVG namespace: in an SVG without one, a
//! `<use>` that leads back to the `<use>` its copy starts from is refused here too, though usvg
//! would leave it out.
//!
//! The style sheets' rules are matched against each element once, as usvg matches them against
//! each copy. usvg tests a selector against an element without keeping what it found, so a
//! selector whose leftmost part matches nothing can test every set of ancestors in turn: the tests
//! usvg makes for each copy are counted, and an SVG past [`MAX_SELECTOR_TESTS`] of them is refused.
//! A look at an attribute counts one test for every 16 attributes of the element it scans, and one
//! for every 16 bytes of the value it compares. Matching stops once the tests pass what is left of
//! the limit.
//!
//! The import's own style cascade (`css::Styles`) keeps the declarations of the rules that match
//! each element, once per element, and reads selectors usvg drops (`:not()`). What it keeps and the
//! tests it makes count toward the same limits, measured before it runs.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use usvg::roxmltree;

use super::{MAX_NODES, SVG_NS, XNode, css};
use crate::{MAX_ELEMENT_DEPTH, SvgError};

/// Most elements usvg may make of an SVG: [`MAX_NODES`], and below the 1,000,000 elements past
/// which usvg stops (its count includes a root of its own).
pub(crate) const MAX_SVG_ELEMENTS: usize = if MAX_NODES < 999_999 { MAX_NODES } else { 999_999 };

/// Most bytes of style values usvg may copy into the elements of an SVG and the SVG images it
/// shows, all together: VectorCraft's own limit.
pub(crate) const MAX_COPIED_STYLES: u64 = 64 << 20;

/// Most tests of a style sheet selector's parts against elements an SVG and the SVG images it
/// shows may take, all together: VectorCraft's own limit.
pub(crate) const MAX_SELECTOR_TESTS: u64 = 50_000_000;

/// What usvg keeps for each value it copies besides its bytes (the count and length of a shared
/// string).
const COPY: u64 = 16;

/// The longhands a `font` shorthand resets before setting the ones it gives, and the most bytes of
/// each reset value (`normal`).
const FONT_RESETS: u64 = 12;
const RESET_VALUE: u64 = 6;

/// The longhands a `font` shorthand sets from its own text.
const FONT_PARTS: u64 = 6;

const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// The elements usvg reads (`EId` in usvg's `parser/svgtree/names.rs`); it skips the others with
/// everything inside them.
const ELEMENTS: [&str; 53] = [
    "a",
    "circle",
    "clipPath",
    "defs",
    "ellipse",
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite",
    "feConvolveMatrix",
    "feDiffuseLighting",
    "feDisplacementMap",
    "feDistantLight",
    "feDropShadow",
    "feFlood",
    "feFuncA",
    "feFuncB",
    "feFuncG",
    "feFuncR",
    "feGaussianBlur",
    "feImage",
    "feMerge",
    "feMergeNode",
    "feMorphology",
    "feOffset",
    "fePointLight",
    "feSpecularLighting",
    "feSpotLight",
    "feTile",
    "feTurbulence",
    "filter",
    "g",
    "image",
    "line",
    "linearGradient",
    "marker",
    "mask",
    "path",
    "pattern",
    "polygon",
    "polyline",
    "radialGradient",
    "rect",
    "stop",
    "style",
    "svg",
    "switch",
    "symbol",
    "text",
    "textPath",
    "tref",
    "tspan",
    "use",
];

/// The name of element `n` when usvg reads it: in the SVG namespace or none, and one of
/// [`ELEMENTS`].
fn svg_name<'a>(n: XNode<'a, '_>) -> Option<&'a str> {
    let name = n.tag_name();
    (n.is_element() && matches!(name.namespace(), None | Some(SVG_NS)) && ELEMENTS.contains(&name.name())).then(|| name.name())
}

#[cfg(test)]
thread_local! {
    /// The selector tests the last measurement on this thread made as usvg matches the style
    /// sheets, as far as it got.
    static USVG_TESTS: Cell<u64> = const { Cell::new(0) };
}

/// An element as the style sheets' selectors see it, as usvg matches them. Each look at it counts
/// as tests in `tests`; past `most` it has no parent or sibling and matches nothing, so the
/// matching ends.
struct Element<'a, 'i, 't> {
    node: XNode<'a, 'i>,
    tests: &'t Cell<u64>,
    most: u64,
}

impl<'a, 'i, 't> Element<'a, 'i, 't> {
    /// Counts `n` tests: false past `most`.
    fn test(&self, n: u64) -> bool {
        let tests = self.tests.get().saturating_add(n);
        self.tests.set(tests);
        #[cfg(test)]
        USVG_TESTS.set(tests);
        tests <= self.most
    }

    fn of(&self, node: Option<XNode<'a, 'i>>) -> Option<Self> {
        node.filter(|_| self.test(1)).map(|node| Element { node, tests: self.tests, most: self.most })
    }
}

impl simplecss::Element for Element<'_, '_, '_> {
    fn parent_element(&self) -> Option<Self> {
        self.of(self.node.parent_element())
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.of(self.node.prev_sibling_element())
    }

    fn has_local_name(&self, name: &str) -> bool {
        self.test(1) && self.node.tag_name().name() == name
    }

    fn attribute_matches(&self, name: &str, operator: simplecss::AttributeOperator<'_>) -> bool {
        // The lookup scans the element's attributes, and the match the value.
        let value = self.node.attribute(name);
        let scanned = css::scan_tests(self.node.attributes().len()).saturating_add(value.map_or(0, |v| css::scan_tests(v.len())));
        self.test(scanned) && value.is_some_and(|value| operator.matches(value))
    }

    fn pseudo_class_matches(&self, class: simplecss::PseudoClass<'_>) -> bool {
        self.test(1) && matches!(class, simplecss::PseudoClass::FirstChild) && self.node.prev_sibling_element().is_none()
    }
}

/// The bytes usvg copies for one declaration: three markers for `marker`, the resets and longhands
/// for `font`, its value for any other. Counted for every declaration, the ones usvg then drops (a
/// property it doesn't read, a value an earlier one overrides) too.
fn copied(declaration: &simplecss::Declaration) -> u64 {
    let len = declaration.value.len() as u64;
    match declaration.name {
        "marker" => 3 * (COPY + len),
        "font" => (FONT_RESETS + FONT_PARTS) * COPY + FONT_RESETS * RESET_VALUE + len,
        _ => COPY + len,
    }
}

/// What usvg makes of an element or an SVG: elements (text nodes included), bytes of copied
/// styles and selector tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Cost {
    pub(crate) elements: u64,
    pub(crate) bytes: u64,
    pub(crate) tests: u64,
}

impl Cost {
    /// The most an SVG and the SVG images it shows may cost together.
    pub(crate) const LIMITS: Cost = Cost { elements: MAX_SVG_ELEMENTS as u64, bytes: MAX_COPIED_STYLES, tests: MAX_SELECTOR_TESTS };

    pub(crate) fn add(self, other: Cost) -> Cost {
        Cost {
            elements: self.elements.saturating_add(other.elements),
            bytes: self.bytes.saturating_add(other.bytes),
            tests: self.tests.saturating_add(other.tests),
        }
    }

    /// What is left of `self` after `spent`.
    pub(crate) fn less(self, spent: Cost) -> Cost {
        Cost {
            elements: self.elements.saturating_sub(spent.elements),
            bytes: self.bytes.saturating_sub(spent.bytes),
            tests: self.tests.saturating_sub(spent.tests),
        }
    }
}

/// What a `<use>` shows: nothing, the element it refers to with all it holds, or only that
/// element (a `<use>` whose own reference leads back to the `<use>` showing it).
enum Shows<'a, 'i> {
    Nothing,
    All(XNode<'a, 'i>),
    Bare(XNode<'a, 'i>),
}

struct Measure<'a, 'i> {
    /// The first element of each id.
    ids: HashMap<&'a str, XNode<'a, 'i>>,
    sheet: simplecss::StyleSheet<'a>,
    /// The selector tests made of `sheet` so far, each element once.
    tests: Cell<u64>,
    /// The most selector tests of `sheet` that are left: matching stops past them.
    most_tests: u64,
    /// The selector tests the import's own cascade made ([`Measure::cascade`]).
    cascade_tests: Cell<u64>,
    /// The import's own style cascade of the same style sheets.
    cascade: css::Styles,
    /// The elements that hold a `<use>` of themselves.
    holds_use_of_itself: HashSet<roxmltree::NodeId>,
    /// The `<use>`s of each element, by the element.
    used_by: HashMap<roxmltree::NodeId, Vec<XNode<'a, 'i>>>,
    /// The bytes of text in each element a `<tref>` shows.
    tref_text: HashMap<roxmltree::NodeId, u64>,
}

impl<'a, 'i> Measure<'a, 'i> {
    fn new(xml: &'a roxmltree::Document<'i>) -> Self {
        let mut ids = HashMap::new();
        for n in xml.descendants() {
            if let Some(id) = n.attribute("id") {
                ids.entry(id).or_insert(n);
            }
        }
        // The style sheets usvg reads: `<style>` elements of no type or of `text/css`, the first
        // text of each.
        let mut sheet = simplecss::StyleSheet::new();
        for n in xml.descendants().filter(|n| n.has_tag_name("style")) {
            if n.attribute("type").is_none_or(|t| t == "text/css")
                && let Some(text) = n.text()
            {
                sheet.parse_more(text);
            }
        }
        let mut m = Measure {
            ids,
            sheet,
            tests: Cell::new(0),
            most_tests: MAX_SELECTOR_TESTS,
            cascade_tests: Cell::new(0),
            cascade: css::Styles::new(xml),
            holds_use_of_itself: HashSet::new(),
            used_by: HashMap::new(),
            tref_text: HashMap::new(),
        };
        for u in xml.descendants().filter(|n| n.has_tag_name((SVG_NS, "use"))) {
            let Some(target) = m.target(u) else { continue };
            if u.ancestors().skip(1).any(|a| a == target) {
                m.holds_use_of_itself.insert(target.id());
            }
            m.used_by.entry(target.id()).or_default().push(u);
        }
        m
    }

    /// The element `<use>` `u` refers to, as usvg resolves it.
    fn target(&self, u: XNode<'a, 'i>) -> Option<XNode<'a, 'i>> {
        let link = u
            .attributes()
            .find(|a| a.name() == "href" && a.namespace().is_none())
            .or_else(|| u.attributes().find(|a| a.name() == "href" && a.namespace() == Some(XLINK_NS)))?;
        let id = svgtypes::IRI::from_str(link.value()).ok()?.0;
        self.ids.get(id).copied()
    }

    /// What `<use>` `u` shows: see the module docs.
    fn shows(&self, u: XNode<'a, 'i>) -> Shows<'a, 'i> {
        let Some(link) = self.target(u) else { return Shows::Nothing };
        if link == u || svg_name(link).is_none() {
            return Shows::Nothing;
        }
        let inside = |v: &XNode| v.ancestors().skip(1).any(|a| a == link);
        if self.holds_use_of_itself.contains(&link.id()) || self.used_by.get(&u.id()).is_some_and(|uses| uses.iter().any(inside)) {
            return Shows::Nothing;
        }
        if svg_name(link) == Some("use") && self.target(link) == Some(u) {
            return Shows::Bare(link);
        }
        Shows::All(link)
    }

    /// Element `n` alone, as usvg makes one copy of it: the bytes of style values it copies into
    /// it, and the selector tests usvg makes to find them (each rule tested counts as one too).
    fn own(&self, n: XNode) -> Cost {
        let before = self.tests.get();
        let mut bytes = 0u64;
        for rule in &self.sheet.rules {
            self.tests.set(self.tests.get().saturating_add(1));
            if rule.selector.matches(&Element { node: n, tests: &self.tests, most: self.most_tests }) {
                bytes = rule.declarations.iter().map(copied).fold(bytes, u64::saturating_add);
            }
        }
        if let Some(style) = n.attribute("style") {
            bytes = simplecss::DeclarationTokenizer::from(style).map(|d| copied(&d)).fold(bytes, u64::saturating_add);
        }
        Cost { elements: 1, bytes, tests: self.tests.get().saturating_sub(before) }
    }

    /// Have the selector tests made so far passed what is left of them?
    fn tests_past(&self) -> bool {
        self.tests.get() > self.most_tests
    }

    /// What the import's own style cascade keeps for the elements of `xml`, each once, up to
    /// `left`: the bytes of the declarations of the rules that match it, and the tests it makes to
    /// find them.
    /// The tests it makes are kept in `cascade_tests`, as far as it gets.
    fn cascade(&self, xml: &roxmltree::Document, left: Cost) -> Result<Cost, Past> {
        let mut cost = Cost::default();
        if !self.cascade.has_rules() {
            return Ok(cost);
        }
        let mut result = Ok(());
        for n in xml.descendants().filter(XNode::is_element) {
            let Some(bytes) = self.cascade.matched_bytes(n, COPY, &mut cost.tests, left.tests) else {
                result = Err(Past::Tests);
                break;
            };
            cost.bytes = cost.bytes.saturating_add(bytes);
            if cost.bytes > left.bytes {
                result = Err(Past::Bytes);
                break;
            }
        }
        self.cascade_tests.set(cost.tests);
        result.map(|()| cost)
    }

    /// `<text>` `t` and what usvg makes of what it holds: at most an element of each element, and
    /// a copy of each run of text and of the text each `<tref>` shows.
    fn text(&mut self, t: XNode<'a, 'i>) -> Cost {
        let mut cost = self.own(t);
        for d in t.descendants().skip(1) {
            if d.is_text() {
                let len = d.text().map_or(0, str::len) as u64;
                cost = cost.add(Cost { elements: 1, bytes: COPY + len, tests: 0 });
            } else if svg_name(d).is_some() {
                cost = cost.add(self.own(d));
                if svg_name(d) == Some("tref") {
                    cost = cost.add(Cost { elements: 1, bytes: COPY + self.tref(d), tests: 0 });
                }
            }
        }
        cost
    }

    /// The bytes of text `<tref>` `r` shows.
    fn tref(&mut self, r: XNode<'a, 'i>) -> u64 {
        let link = r.attribute((XLINK_NS, "href")).or_else(|| r.attribute("href"));
        let Some(target) = link.and_then(|l| svgtypes::IRI::from_str(l).ok()).and_then(|iri| self.ids.get(iri.0).copied()) else { return 0 };
        *self.tref_text.entry(target.id()).or_insert_with(|| {
            target.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).map(|t| t.len() as u64).fold(0, u64::saturating_add)
        })
    }
}

/// What usvg would make of `xml`, with what the import's own style cascade keeps of it and the
/// selector tests it makes, when that is no more than `left` and nested no deeper than `depth`
/// (counted as `depth::height` counts, which finds at least as much).
pub(super) fn check(xml: &roxmltree::Document, left: Cost, depth: usize) -> Result<Cost, SvgError> {
    check_spending(xml, left, depth).0
}

/// [`check`], with the selector tests it made, as far as it got.
pub(super) fn check_spending(xml: &roxmltree::Document, left: Cost, depth: usize) -> (Result<Cost, SvgError>, u64) {
    let (result, tests) = measure_spending(xml, left, depth);
    let result = result.map_err(|past| match past {
        Past::Elements => SvgError::TooLarge(MAX_SVG_ELEMENTS),
        Past::Bytes => SvgError::StylesTooLarge((MAX_COPIED_STYLES >> 20) as usize),
        Past::Tests => SvgError::StylesTooComplex,
        Past::Depth => SvgError::TooDeep(MAX_ELEMENT_DEPTH),
    });
    (result, tests)
}

/// The limit [`measure`] stopped at.
#[derive(Debug, PartialEq)]
enum Past {
    Elements,
    Bytes,
    Tests,
    Depth,
}

/// What usvg makes of `xml` with what the import's own style cascade keeps of it and its tests,
/// up to `left`, its elements and the copies `<use>`s show nested up to `depth` deep. Without
/// recursion; each element is measured once.
#[cfg(test)]
fn measure(xml: &roxmltree::Document, left: Cost, depth: usize) -> Result<Cost, Past> {
    measure_spending(xml, left, depth).0
}

/// [`measure`], with the selector tests it made, as far as it got.
fn measure_spending(xml: &roxmltree::Document, left: Cost, depth: usize) -> (Result<Cost, Past>, u64) {
    let mut m = Measure::new(xml);
    let result = walk(&mut m, xml, left, depth);
    (result, m.cascade_tests.get().saturating_add(m.tests.get()))
}

/// [`measure`] with `m`, made of `xml`.
fn walk<'a, 'i>(m: &mut Measure<'a, 'i>, xml: &'a roxmltree::Document<'i>, left: Cost, depth: usize) -> Result<Cost, Past> {
    // First: its tests are bounded by what it finds, usvg's are not.
    let cascade = m.cascade(xml, left)?;
    let left = left.less(cascade);
    m.most_tests = left.tests;
    let (elements, bytes) = (left.elements, left.bytes);
    let mut costs: HashMap<roxmltree::NodeId, Cost> = HashMap::new();
    // The elements entered and not yet left: one entered again is a loop.
    let mut open: HashSet<roxmltree::NodeId> = HashSet::new();
    enum Step<'a, 'i> {
        Enter(XNode<'a, 'i>),
        /// An element, what its cost adds up (its children, or what a `<use>` shows), and the one
        /// element a `<use>` shows alone.
        Leave(XNode<'a, 'i>, Vec<XNode<'a, 'i>>, Option<XNode<'a, 'i>>),
    }
    let root = xml.root_element();
    let mut stack = vec![Step::Enter(root)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Enter(n) => {
                if costs.contains_key(&n.id()) || svg_name(n).is_none_or(|name| name == "style") {
                    continue;
                }
                if !open.insert(n.id()) {
                    // usvg copies a loop until it nests past what it reads.
                    return Err(Past::Depth);
                }
                // The elements entered and not yet left are the ones it is nested in.
                if open.len() > depth {
                    return Err(Past::Depth);
                }
                let (parts, bare) = match svg_name(n) {
                    Some("text") => (vec![], None),
                    Some("use") => match m.shows(n) {
                        Shows::All(link) => (vec![link], None),
                        Shows::Bare(link) => (vec![], Some(link)),
                        Shows::Nothing => (vec![], None),
                    },
                    _ => (n.children().filter(|c| svg_name(*c).is_some_and(|name| name != "style")).collect(), None),
                };
                stack.push(Step::Leave(n, parts.clone(), bare));
                stack.extend(parts.into_iter().map(Step::Enter));
            }
            Step::Leave(n, parts, bare) => {
                open.remove(&n.id());
                let mut cost = match svg_name(n) {
                    Some("text") => m.text(n),
                    _ => m.own(n),
                };
                if let Some(link) = bare {
                    cost = cost.add(m.own(link));
                }
                for part in parts {
                    cost = cost.add(costs.get(&part.id()).copied().unwrap_or_default());
                }
                if cost.elements > elements {
                    return Err(Past::Elements);
                }
                if cost.bytes > bytes {
                    return Err(Past::Bytes);
                }
                if m.tests_past() || cost.tests > left.tests {
                    return Err(Past::Tests);
                }
                costs.insert(n.id(), cost);
            }
        }
    }
    let usvg = costs.get(&root.id()).copied().unwrap_or_default();
    Ok(usvg.add(cascade))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svg(body: &str) -> String {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">{body}</svg>"#)
    }

    const NO_LIMITS: Cost = Cost { elements: u64::MAX, bytes: u64::MAX, tests: u64::MAX };

    /// What [`measure`] makes of `text` with no limits, as (elements, bytes).
    fn cost(text: &str) -> (u64, u64) {
        let xml = roxmltree::Document::parse(text).unwrap();
        let c = measure(&xml, NO_LIMITS, usize::MAX).unwrap_or_else(|p| panic!("{p:?}"));
        (c.elements, c.bytes)
    }

    fn past(text: &str, elements: u64, bytes: u64) -> Option<Past> {
        measure(&roxmltree::Document::parse(text).unwrap(), Cost { elements, bytes, tests: u64::MAX }, usize::MAX).err()
    }

    /// Groups `g0` to `g{levels}`: `g0` a rectangle styled `style`, each other two `<use>`s of the
    /// one before; one `<use>` of the last shown.
    fn chain(levels: usize, style: &str) -> String {
        let links: String = (1..=levels).map(|i| format!("<g id='g{i}'><use href='#g{0}'/><use xlink:href='#g{0}'/></g>", i - 1)).collect();
        svg(&format!("<defs><g id='g0'><rect width='1' height='1' style='{style}'/></g>{links}</defs><use href='#g{levels}'/>"))
    }

    #[test]
    fn copies_are_counted_with_their_styles_up_to_each_limit() {
        // svg, defs, g0 and its rect (2), g1 with two uses of g0 (1 + 2 × 3), the shown use (1 + 7).
        // Five rects, each copying `red` (3 bytes and a copy's 16).
        let one = chain(1, "fill:red");
        assert_eq!(cost(&one), (19, 5 * 19));
        assert_eq!(past(&one, 19, 95), None);
        assert_eq!(past(&one, 18, 95), Some(Past::Elements));
        assert_eq!(past(&one, 19, 94), Some(Past::Bytes));
        // Each level doubles what the shown use holds, without making any of it.
        let (elements, _) = cost(&chain(60, ""));
        assert!(elements > 1 << 60, "{elements}");
        assert_eq!(past(&chain(60, ""), MAX_SVG_ELEMENTS as u64, u64::MAX), Some(Past::Elements));
        assert_eq!(past(&chain(40, "font-family:a"), u64::MAX, MAX_COPIED_STYLES), Some(Past::Bytes));
    }

    #[test]
    fn the_element_limit_stays_below_where_usvg_stops() {
        assert_eq!(MAX_SVG_ELEMENTS, if cfg!(target_arch = "wasm32") { 500_000 } else { 999_999 });
    }

    #[test]
    fn style_sheet_rules_count_where_they_match() {
        let groups = "<g class='a'/>".repeat(4);
        // `g` matches the four groups (16 + 3 each), `.b` none, `svg` the root (16 + 4); the
        // import's own cascade keeps the same, names included (16 + 4 + 3 and 16 + 4 + 4).
        let (_, bytes) = cost(&svg(&format!("<style>g{{fill:red}} .b{{fill:blue}} svg{{fill:none}}</style>{groups}")));
        assert_eq!(bytes, 4 * 19 + 20 + 4 * 23 + 24);
        // usvg drops a `:not()` rule; the cascade keeps it for each group it matches.
        assert_eq!(cost(&svg(&format!("<style>g:not(.x){{fill:red}}</style>{groups}"))).1, 4 * 23);
        // A style sheet of another type is not read; the style element itself makes nothing.
        assert_eq!(cost(&svg(&format!("<style type='text/x'>g{{fill:red}}</style>{groups}"))), (5, 0));
        // `marker` sets three properties; `font` resets twelve and sets six from its text.
        let (_, marker) = cost(&svg("<path style='marker:url(#m)'/>"));
        let (_, font) = cost(&svg("<path style='font:12px a'/>"));
        assert_eq!((marker, font), (3 * (16 + 7), 18 * 16 + 12 * 6 + 6));
    }

    #[test]
    fn references_usvg_leaves_out_copy_nothing() {
        // A use of itself, of a group holding it, of what isn't there or isn't SVG.
        let (n, _) = cost(&svg("<use id='u' href='#u'/><g id='a'><g><use href='#a'/></g></g><use href='#none'/><x id='x'/><use href='#x'/>"));
        assert_eq!(n, 1 + 1 + 3 + 1 + 1);
        // Two uses of each other: each shows the other alone.
        assert_eq!(cost(&svg("<use id='u1' href='#u2'/><use id='u2' href='#u1'/>")).0, 1 + 2 + 2);
        // A group holding a use of the use that shows it: that use shows nothing, and the one the
        // group holds shows it alone.
        let back = svg("<g id='g'><use id='inner' href='#outer'/></g><use id='outer' href='#g'/>");
        assert_eq!(cost(&back).0, 1 + 3 + 1);
        // The unprefixed href wins, and an id names its first element.
        let first = svg("<g id='a'><rect/><rect/></g><g id='a'/><rect id='b'/><use xlink:href='#a' href='#b'/><use href='#a'/>");
        assert_eq!(cost(&first).0, 1 + 3 + 1 + 1 + 2 + 4);
    }

    #[test]
    fn a_loop_usvg_copies_without_end_is_refused_as_usvg_refuses_it() {
        let looped = svg("<g id='a'><use href='#b'/></g><g id='b'><use href='#a'/></g>");
        assert_eq!(past(&looped, u64::MAX, u64::MAX), Some(Past::Depth));
        assert!(usvg::Tree::from_str(&looped, &usvg::Options::default()).is_err());
        // Without the loop, usvg reads it.
        let open = svg("<g id='a'><use href='#b'/></g><g id='b'><rect width='1' height='1'/></g>");
        assert!(past(&open, u64::MAX, u64::MAX).is_none() && usvg::Tree::from_str(&open, &usvg::Options::default()).is_ok());
    }

    /// A chain of `<use>`s nested past the depth is too deep, as `depth::height` finds it, before
    /// what the chain makes is added up; one within the depth `depth::height` gives is not.
    #[test]
    fn copies_nested_past_the_depth_are_too_deep() {
        let links: String = (0..3000).map(|i| format!("<g id='u{i}'><use href='#u{}'/></g>", i + 1)).collect();
        let deep = svg(&format!("<defs>{links}<rect id='u3000' width='1' height='1'/></defs><use href='#u0'/>"));
        let xml = roxmltree::Document::parse(&deep).unwrap();
        assert_eq!(measure(&xml, NO_LIMITS, MAX_ELEMENT_DEPTH).err(), Some(Past::Depth));
        assert!(super::super::depth::height(&xml, MAX_ELEMENT_DEPTH).is_none());
        assert!(matches!(check(&xml, Cost::LIMITS, MAX_ELEMENT_DEPTH), Err(SvgError::TooDeep(_))));
        for text in [chain(5, ""), svg("<g><g><use href='#r'/></g></g><rect id='r'/>")] {
            let xml = roxmltree::Document::parse(&text).unwrap();
            let height = super::super::depth::height(&xml, usize::MAX).unwrap_or(0);
            assert!(measure(&xml, NO_LIMITS, height).is_ok(), "{height} {text}");
        }
    }

    #[test]
    fn text_counts_its_runs_and_what_trefs_show() {
        let (n, bytes) = cost(&svg("<g id='t'>abcd</g><text>xy<tspan>z</tspan><tref href='#t'/></text>"));
        // svg, g (its text is not counted outside a text), text, its runs "xy" and "z", tspan, tref and its text.
        assert_eq!(n, 1 + 1 + 1 + 2 + 1 + 1 + 1);
        assert_eq!(bytes, (16 + 2) + (16 + 1) + (16 + 4));
    }

    /// A long style sheet rule that matches many elements is refused before the import's own style
    /// cascade (which keeps the declarations of each element) reads them, in an SVG and in an SVG
    /// image it shows.
    #[test]
    fn a_long_rule_on_many_elements_is_refused_before_the_cascade_reads_it() {
        let groups = svg(&format!("<style>g{{font-family:{}}}</style>{}", "a".repeat(100_000), "<g/>".repeat(1000)));
        let image = svg(&format!(r#"<image href="data:image/svg+xml;base64,{}" width="10" height="10"/>"#, crate::base64_encode(groups.as_bytes())));
        // usvg drops a `:not()` rule, which the import's own cascade reads.
        let not = svg(&format!("<style>g:not(.x){{font-family:{}}}</style>{}", "a".repeat(100_000), "<g/>".repeat(1000)));
        for text in [groups, image, not] {
            let before = super::super::css::selector_tests();
            let err = super::super::import(&text, &crate::ImportOptions::default(), MAX_NODES).err();
            assert!(matches!(err, Some(SvgError::StylesTooLarge(64))), "{err:?}");
            assert_eq!(super::super::css::selector_tests(), before);
        }
    }

    /// A linked SVG file with such a rule, and an `<image>` of its own whose link the import reads,
    /// is refused before the cascade reads it too.
    #[test]
    fn a_linked_svg_with_a_long_rule_is_refused_before_the_cascade_reads_it() {
        let linked =
            svg(&format!("<style>g{{font-family:{}}}</style>{}<image href='b.png' width='1' height='1'/>", "a".repeat(100_000), "<g/>".repeat(1000)));
        let read = |path: &str| path.ends_with("a.svg").then(|| (linked.clone().into_bytes(), vectorcraft_doc::LinkInfo::new(path)));
        let folder = std::env::temp_dir().join("vectorcraft-svg-budget");
        let folder = folder.to_string_lossy();
        let opts = crate::ImportOptions { folder: Some(&folder), read: Some(&read) };
        let before = super::super::css::selector_tests();
        let err = super::super::import(&svg("<image href='a.svg' width='10' height='10'/>"), &opts, MAX_NODES).err();
        assert!(matches!(err, Some(SvgError::StylesTooLarge(64))), "{err:?}");
        assert_eq!(super::super::css::selector_tests(), before);
    }

    /// `depth` groups in one another, each of class `a`, around a rectangle, with `sheet`.
    fn nested(depth: usize, sheet: &str) -> String {
        svg(&format!("<style>{sheet}</style>{}<rect width='1' height='1'/>{}", "<g class='a'>".repeat(depth), "</g>".repeat(depth)))
    }

    /// A selector whose leftmost part matches nothing makes usvg test every set of ancestors in
    /// turn: the tests stop at the limit and the SVG is refused, before usvg or the cascade runs.
    #[test]
    fn a_selector_that_never_matches_stops_at_the_test_limit() {
        let never = nested(60, &format!(".missing{} {{fill:red}}", " g".repeat(20)));
        let xml = roxmltree::Document::parse(&never).unwrap();
        assert_eq!(measure(&xml, Cost::LIMITS, usize::MAX).err(), Some(Past::Tests));
        let before = super::super::css::selector_tests();
        let err = super::super::import(&never, &crate::ImportOptions::default(), MAX_NODES).err();
        assert!(matches!(err, Some(SvgError::StylesTooComplex)), "{err:?}");
        assert_eq!(super::super::css::selector_tests(), before);
        // The same selector without its leftmost part matches at once, and imports.
        let matching = nested(60, &format!("g{} {{fill:red}}", " g".repeat(19)));
        assert!(super::super::import(&matching, &crate::ImportOptions::default(), MAX_NODES).is_ok());
    }

    /// usvg tests the rules against each copy a `<use>` shows: the tests add up as the copies do,
    /// and the cascade's tests count once for each element.
    #[test]
    fn selector_tests_count_for_each_copy() {
        let tests = |levels: usize| {
            let text = chain(levels, "").replacen("<defs>", "<style>.b rect{fill:red}</style><defs>", 1);
            let xml = roxmltree::Document::parse(&text).unwrap();
            measure(&xml, NO_LIMITS, usize::MAX).unwrap_or_else(|p| panic!("{p:?}")).tests
        };
        let (ten, eleven) = (tests(10), tests(11));
        assert!(eleven > 2 * ten - 100, "{ten} {eleven}");
        let text = chain(11, "").replacen("<defs>", "<style>.b rect{fill:red}</style><defs>", 1);
        let xml = roxmltree::Document::parse(&text).unwrap();
        assert_eq!(measure(&xml, Cost { tests: eleven - 1, ..NO_LIMITS }, usize::MAX).err(), Some(Past::Tests));
        assert!(measure(&xml, Cost { tests: eleven, ..NO_LIMITS }, usize::MAX).is_ok());
    }

    /// Each alternative of a `:not()` counts as a test of the cascade, for each element.
    #[test]
    fn not_alternatives_count_as_tests() {
        let alternatives: Vec<String> = (0..1000).map(|i| format!(".m{i}")).collect();
        let text = svg(&format!("<style>g:not({}){{fill:red}}</style>{}", alternatives.join(","), "<g/>".repeat(1000)));
        let xml = roxmltree::Document::parse(&text).unwrap();
        let tests = measure(&xml, NO_LIMITS, usize::MAX).unwrap_or_else(|p| panic!("{p:?}")).tests;
        assert!(tests > 1000 * 1000, "{tests}");
        assert_eq!(measure(&xml, Cost { tests: 1000 * 1000, ..NO_LIMITS }, usize::MAX).err(), Some(Past::Tests));
    }

    /// Matching as usvg matches stops once its tests pass what the limits have left, as when an
    /// SVG image gets what the SVG showing it left.
    #[test]
    fn usvg_matching_stops_at_what_is_left() {
        let never = nested(60, &format!(".missing{} {{fill:red}}", " g".repeat(20)));
        let xml = roxmltree::Document::parse(&never).unwrap();
        let cascade = Measure::new(&xml).cascade(&xml, NO_LIMITS).unwrap_or_else(|p| panic!("{p:?}")).tests;
        assert_eq!(measure(&xml, Cost { tests: cascade + 1000, ..NO_LIMITS }, usize::MAX).err(), Some(Past::Tests));
        let made = USVG_TESTS.get();
        assert!((1000..1100).contains(&made), "{made}");
    }

    /// Each class of a compound is looked for in the element's class list, so the tests grow with
    /// the list's length: in the import's own cascade (a `:not()` rule, which usvg drops) and as
    /// usvg matches.
    #[test]
    fn class_tests_grow_with_the_class_list() {
        let list = (0..1000).map(|i| format!("c{i}")).collect::<Vec<_>>().join(" ");
        let classes: String = (0..100).map(|i| format!(".c{i}")).collect();
        let scans = 100 * css::scan_tests(list.len());
        let tests = |rule: &str| {
            let text = svg(&format!("<style>{rule}{{fill:red}}</style><g class='{list}'/>"));
            let xml = roxmltree::Document::parse(&text).unwrap();
            measure(&xml, NO_LIMITS, usize::MAX).unwrap_or_else(|p| panic!("{p:?}")).tests
        };
        let cascade = tests(&format!("g:not(.x{classes})"));
        assert!(cascade >= scans, "{cascade} {scans}");
        tests(&format!("g{classes}"));
        let usvg = USVG_TESTS.get();
        assert!(usvg >= scans, "{usvg} {scans}");
    }
}
