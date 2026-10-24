//! The style cascade for the `<text>` pre-pass: `<style>` rules, the `style` attribute and
//! presentation attributes.
//!
//! Selectors: type, `*`, `#id`, any number of `.class`es, `[attr]` / `[attr=value]` and `:not()` of a
//! list of those, combined by descendant (space) and child (`>`) combinators. Rules with other
//! selectors (other pseudo-classes, sibling combinators, namespaces) never match, and at-rule blocks (`@media`, `@font-face`…) are
//! skipped. Precedence: `!important`, then the `style` attribute, then specificity, then source
//! order; a presentation attribute applies only when no rule sets the property.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use usvg::roxmltree;

pub(super) type XNode<'a, 'i> = roxmltree::Node<'a, 'i>;

/// `name: value` declarations of a block or `style` attribute, with their `!important` flag. The
/// `font` shorthand becomes its longhands, as usvg reads it for the rest of the drawing (#1557).
pub(super) fn parse_decls(s: &str) -> Vec<(String, String, bool)> {
    s.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let v = v.trim();
            let stem = v.len().checked_sub("important".len()).and_then(|i| v.get(..i).zip(v.get(i..)));
            let flagged = stem.filter(|(_, kw)| kw.eq_ignore_ascii_case("important"));
            let (v, important) = match flagged.map(|(h, _)| h.trim_end()).and_then(|v| v.strip_suffix('!')) {
                Some(v) => (v.trim_end(), true),
                None => (v, false),
            };
            Some((k.trim().to_ascii_lowercase(), v.to_string(), important))
        })
        .flat_map(|(k, v, important)| match k.as_str() {
            "font" => font_longhands(&v).into_iter().map(|(k, v)| (k.to_string(), v, important)).collect(),
            _ => vec![(k, v, important)],
        })
        .collect()
}

/// The longhands of a `font` shorthand: those it leaves out back to their initial values. None
/// when it doesn't parse (it is then ignored, as in CSS).
fn font_longhands(v: &str) -> Vec<(&'static str, String)> {
    let Ok(f) = svgtypes::FontShorthand::from_str(v) else { return Vec::new() };
    let or = |o: Option<&str>| o.unwrap_or("normal").to_string();
    vec![
        ("font-style", or(f.font_style)),
        ("font-variant", or(f.font_variant)),
        ("font-weight", or(f.font_weight)),
        ("font-stretch", or(f.font_stretch)),
        ("font-size", f.font_size.to_string()),
        ("font-family", f.font_family.to_string()),
    ]
}

/// One compound selector (`text.a.b#c[x=y]`).
#[derive(Debug, Default)]
struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, Option<String>)>,
    /// The compounds of `:not()`s: the element must match none of them.
    nots: Vec<Compound>,
}

impl Compound {
    fn matches(&self, n: XNode) -> bool {
        self.tag.as_deref().is_none_or(|t| n.tag_name().name() == t)
            && self.id.as_deref().is_none_or(|id| n.attribute("id") == Some(id))
            && (self.classes.is_empty() || n.attribute("class").is_some_and(|c| self.classes.iter().all(|k| c.split_whitespace().any(|x| x == k))))
            && self.attrs.iter().all(|(a, v)| n.attribute(a.as_str()).is_some_and(|x| v.as_deref().is_none_or(|v| x == v)))
            && !self.nots.iter().any(|c| c.matches(n))
    }
}

/// A complex selector, rightmost compound first; each compound's flag says whether it must be the
/// child (true) or a descendant (false) of the next one.
#[derive(Debug)]
struct Selector {
    parts: Vec<(Compound, bool)>,
    /// (ids, classes and attributes, types).
    specificity: (u32, u32, u32),
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

/// How deep `:not()`s may nest (real style sheets nest one or two).
const MAX_NOT_NESTING: usize = 8;

impl Selector {
    /// `None` for selectors this cascade doesn't support (they never match).
    fn parse(s: &str) -> Option<Self> {
        Self::parse_nested(s, 0)
    }

    /// [`Self::parse`] for a selector `nesting` `:not()`s deep: past [`MAX_NOT_NESTING`] it is not
    /// supported, so a style sheet of `:not(:not(:not(…` can't exhaust the stack.
    fn parse_nested(s: &str, nesting: usize) -> Option<Self> {
        fn ident(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<String> {
            let mut out = String::new();
            while let Some(&c) = chars.peek().filter(|c| is_ident(**c)) {
                out.push(c);
                chars.next();
            }
            (!out.is_empty()).then_some(out)
        }
        // Compounds left to right; `links[i]` joins compounds i and i + 1 (true = child).
        let mut comps: Vec<Compound> = vec![];
        let mut links: Vec<bool> = vec![];
        let mut cur: Option<Compound> = None;
        let mut link: Option<bool> = None;
        // What the `:not()`s add: the specificity of each one's most specific argument.
        let mut not_specificity = (0, 0, 0);
        let mut chars = s.trim().chars().peekable();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == '>' {
                chars.next();
                comps.extend(cur.take());
                if comps.is_empty() {
                    return None;
                }
                link = Some(c == '>' || link == Some(true));
                continue;
            }
            let comp = match &mut cur {
                Some(comp) => comp,
                None => {
                    if !comps.is_empty() {
                        links.push(link.take()?);
                    }
                    cur.insert(Compound::default())
                }
            };
            match c {
                '*' => {
                    chars.next();
                }
                '#' => {
                    chars.next();
                    comp.id = Some(ident(&mut chars)?);
                }
                '.' => {
                    chars.next();
                    comp.classes.push(ident(&mut chars)?);
                }
                '[' => {
                    chars.next();
                    let body: String = chars.by_ref().take_while(|c| *c != ']').collect();
                    let (name, value) = match body.split_once('=') {
                        Some((n, v)) => (n.trim(), Some(v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())),
                        None => (body.trim(), None),
                    };
                    if name.is_empty() || !name.chars().all(is_ident) {
                        return None;
                    }
                    comp.attrs.push((name.to_string(), value));
                }
                ':' => {
                    chars.next();
                    if nesting >= MAX_NOT_NESTING || !ident(&mut chars)?.eq_ignore_ascii_case("not") || chars.next() != Some('(') {
                        return None;
                    }
                    let mut depth = 1;
                    let mut arg = String::new();
                    for c in chars.by_ref() {
                        match c {
                            '(' => depth += 1,
                            ')' => depth -= 1,
                            _ => {}
                        }
                        if depth == 0 {
                            break;
                        }
                        arg.push(c);
                    }
                    if depth != 0 {
                        return None;
                    }
                    // A list of compound selectors (each may hold a `:not()` of its own).
                    let mut most = (0, 0, 0);
                    for a in top_level_commas(&arg) {
                        let s = Self::parse_nested(a, nesting + 1)?;
                        let [(c, _)]: [(Compound, bool); 1] = s.parts.try_into().ok()?;
                        most = most.max(s.specificity);
                        comp.nots.push(c);
                    }
                    not_specificity = (not_specificity.0 + most.0, not_specificity.1 + most.1, not_specificity.2 + most.2);
                }
                c if is_ident(c) => comp.tag = Some(ident(&mut chars)?),
                _ => return None,
            }
        }
        // A trailing `>` has nothing to apply to.
        if cur.is_none() && link == Some(true) {
            return None;
        }
        comps.extend(cur);
        if comps.is_empty() {
            return None;
        }
        let mut specificity = not_specificity;
        for c in &comps {
            specificity.0 += c.id.is_some() as u32;
            specificity.1 += (c.classes.len() + c.attrs.len()) as u32;
            specificity.2 += c.tag.is_some() as u32;
        }
        // Rightmost first, each with its link to the compound on its left.
        let parts = comps.into_iter().enumerate().rev().map(|(i, c)| (c, i.checked_sub(1).and_then(|k| links.get(k)).is_some_and(|l| *l))).collect();
        Some(Self { parts, specificity })
    }

    fn has_not(&self) -> bool {
        self.parts.iter().any(|(c, _)| !c.nots.is_empty())
    }

    fn matches(&self, n: XNode) -> bool {
        // The result of matching a selector suffix from a node depends only on the suffix and
        // the node, so memoize it: without this, every descendant step retries every matching
        // ancestor and a selector whose leftmost compound never matches makes the search
        // enumerate ancestor combinations (#1703). A suffix of `parts` is identified by its
        // length, so (suffix length, node id) is a complete key and the visited states are
        // bounded by parts × ancestors.
        fn go(parts: &[(Compound, bool)], n: XNode, memo: &mut HashMap<(usize, roxmltree::NodeId), bool>) -> bool {
            let Some(((c, child), rest)) = parts.split_first() else { return true };
            if !c.matches(n) {
                return false;
            }
            if rest.is_empty() {
                return true;
            }
            let key = (rest.len(), n.id());
            if let Some(&hit) = memo.get(&key) {
                return hit;
            }
            let hit = if *child {
                n.parent_element().is_some_and(|p| go(rest, p, memo))
            } else {
                n.ancestors().skip(1).filter(|a| a.is_element()).any(|a| go(rest, a, memo))
            };
            memo.insert(key, hit);
            hit
        }
        go(&self.parts, n, &mut HashMap::new())
    }
}

struct Rule {
    selector: Selector,
    decls: Rc<Vec<(String, String, bool)>>,
}

/// The rules filed under the id of their selector's rightmost compound, else under its first
/// class, else under its type. Rules whose rightmost compound has none of these (`*`, `[x]`) are
/// in `any`. Each list holds positions in the rule list, in source order.
#[derive(Default)]
struct Index {
    ids: HashMap<String, Vec<usize>>,
    classes: HashMap<String, Vec<usize>>,
    tags: HashMap<String, Vec<usize>>,
    any: Vec<usize>,
}

impl Index {
    fn new(rules: &[Rule]) -> Self {
        let mut index = Self::default();
        for (i, r) in rules.iter().enumerate() {
            let list = match r.selector.parts.first() {
                Some((c, _)) => match (&c.id, c.classes.first(), &c.tag) {
                    (Some(id), _, _) => index.ids.entry(id.clone()).or_default(),
                    (None, Some(class), _) => index.classes.entry(class.clone()).or_default(),
                    (None, None, Some(tag)) => index.tags.entry(tag.clone()).or_default(),
                    (None, None, None) => &mut index.any,
                },
                None => &mut index.any,
            };
            list.push(i);
        }
        index
    }

    /// The positions of the rules to test against element `n`, each once, in source order: those
    /// filed under its id, its classes and its type, and those in `any`.
    fn candidates(&self, n: XNode) -> Vec<usize> {
        let mut out = self.any.clone();
        let mut add = |list: Option<&Vec<usize>>| out.extend(list.into_iter().flatten());
        add(n.attribute("id").and_then(|id| self.ids.get(id)));
        let mut seen = HashSet::new();
        for class in n.attribute("class").into_iter().flat_map(str::split_whitespace).filter(|c| seen.insert(*c)) {
            add(self.classes.get(class));
        }
        add(self.tags.get(n.tag_name().name()));
        out.sort();
        out
    }
}

/// `list` split at the commas outside parentheses (those in a `:not()` stay).
fn top_level_commas(list: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0usize;
    list.split(move |c| {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        c == ',' && depth == 0
    })
}

/// Rules from every `<style>` element, in source order.
fn parse_sheet(css: &str, out: &mut Vec<Rule>) {
    // Drop comments.
    let mut text = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        text.push_str(&rest[..i]);
        rest = rest[i + 2..].find("*/").map_or("", |j| &rest[i + 2 + j + 2..]);
    }
    text.push_str(rest);
    let mut s = text.as_str();
    loop {
        s = s.trim_start();
        if s.is_empty() {
            break;
        }
        let Some(open) = s.find('{') else { break };
        let head = s[..open].trim();
        if head.starts_with('@') {
            // Statement at-rules end at `;` before any block.
            if let Some(semi) = s.find(';').filter(|&semi| semi < open) {
                s = &s[semi + 1..];
                continue;
            }
            // Skip the (possibly nested) block.
            let mut depth = 0usize;
            let mut end = s.len();
            for (i, c) in s[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            s = &s[end..];
            continue;
        }
        let close = s[open..].find('}').map_or(s.len(), |i| open + i);
        let decls = Rc::new(parse_decls(&s[open + 1..close]));
        for sel in top_level_commas(head) {
            if let Some(selector) = Selector::parse(sel) {
                out.push(Rule { selector, decls: decls.clone() });
            }
        }
        s = s.get(close + 1..).unwrap_or("");
    }
}

/// The cascade over one parsed document.
pub(super) struct Styles {
    rules: Vec<Rule>,
    index: Index,
    /// Declared properties of each element queried so far.
    cache: RefCell<HashMap<roxmltree::NodeId, Rc<HashMap<String, String>>>>,
}

impl Styles {
    pub(super) fn new(doc: &roxmltree::Document) -> Self {
        let mut rules = vec![];
        for st in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "style") {
            if st.attribute("type").is_some_and(|t| !t.trim().is_empty() && t.trim() != "text/css") {
                continue;
            }
            let css: String = st.children().filter_map(|c| c.text()).collect();
            parse_sheet(&css, &mut rules);
        }
        let index = Index::new(&rules);
        Self { rules, index, cache: RefCell::default() }
    }

    /// Do the document's style sheets have any rules?
    pub(super) fn has_rules(&self) -> bool {
        !self.rules.is_empty()
    }

    /// Properties set on `n` by rules and its `style` attribute, after the cascade.
    pub(super) fn declared(&self, n: XNode) -> Rc<HashMap<String, String>> {
        if let Some(m) = self.cache.borrow().get(&n.id()) {
            return m.clone();
        }
        // (important, inline, specificity, order) → highest wins.
        type Key = (bool, bool, (u32, u32, u32), usize);
        let mut best: HashMap<String, (Key, String)> = HashMap::new();
        let mut put = |name: &str, value: &str, key: Key| {
            if best.get(name).is_none_or(|(k, _)| key >= *k) {
                best.insert(name.to_string(), (key, value.to_string()));
            }
        };
        let mut order = 0;
        for r in self.index.candidates(n).into_iter().filter_map(|i| self.rules.get(i)).filter(|r| r.selector.matches(n)) {
            for (k, v, imp) in r.decls.iter() {
                order += 1;
                put(k, v, (*imp, false, r.selector.specificity, order));
            }
        }
        if let Some(st) = n.attribute("style") {
            for (k, v, imp) in parse_decls(st) {
                order += 1;
                put(&k, &v, (imp, true, (0, 0, 0), order));
            }
        }
        let m: Rc<HashMap<String, String>> = Rc::new(best.into_iter().map(|(k, (_, v))| (k, v)).collect());
        self.cache.borrow_mut().insert(n.id(), m.clone());
        m
    }

    /// Are there rules with a `:not()` (which usvg's style sheets drop)?
    pub(super) fn has_not_rules(&self) -> bool {
        self.rules.iter().any(|r| r.selector.has_not())
    }

    /// The properties rules with a `:not()` set on `n`, each with its value after the cascade.
    pub(super) fn set_by_not_rules(&self, n: XNode) -> Vec<(String, String)> {
        let declared = self.declared(n);
        let mut out: Vec<(String, String)> = Vec::new();
        for r in self.rules.iter().filter(|r| r.selector.has_not() && r.selector.matches(n)) {
            for (k, ..) in r.decls.iter() {
                if !out.iter().any(|(o, _)| o == k)
                    && let Some(v) = declared.get(k)
                {
                    out.push((k.clone(), v.clone()));
                }
            }
        }
        out
    }

    /// A property specified on this element (rules and `style` beat the presentation attribute).
    pub(super) fn own(&self, n: XNode, name: &str) -> Option<String> {
        self.declared(n).get(name).cloned().or_else(|| n.attribute(name).map(|v| v.trim().to_string()))
    }

    /// An inherited property: the nearest specified value other than `inherit`.
    pub(super) fn prop(&self, n: XNode, name: &str) -> Option<String> {
        n.ancestors().filter(|a| a.is_element()).find_map(|a| self.own(a, name).filter(|v| v != "inherit"))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn important_flag_is_case_insensitive() {
        for s in ["fill: red !IMPORTANT", "fill: red !Important", "fill: red ! important"] {
            assert_eq!(parse_decls(s), [("fill".to_string(), "red".to_string(), true)], "{s}");
        }
        assert_eq!(parse_decls("fill: red !importantx"), [("fill".to_string(), "red !importantx".to_string(), false)]);
    }

    fn with_doc(svg: &str, f: impl FnOnce(&roxmltree::Document, &Styles)) {
        let doc = roxmltree::Document::parse(svg).unwrap();
        let st = Styles::new(&doc);
        f(&doc, &st);
    }

    fn by_id<'a, 'i>(doc: &'a roxmltree::Document<'i>, id: &str) -> XNode<'a, 'i> {
        doc.descendants().find(|n| n.attribute("id") == Some(id)).unwrap()
    }

    #[test]
    fn selectors_and_precedence() {
        with_doc(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><style>
            /* comment { fill: red } */
            @media print { text { fill: red } }
            @import url(x.css);
            text { fill: #111; font-size: 9px }
            g text { fill: #222 }
            g > text.a { fill: #333 }
            .a.b { stroke: blue }
            #t2 { fill: #444 }
            [data-k=v] { font-weight: bold }
            text:first-child, text ~ text { fill: red }
            .imp { fill: #555 !important }
            </style>
            <text id="t0">x</text>
            <g><text id="t1" class="a b" fill="green">x</text>
               <g><text id="t2" class="a" style="fill:#666">x</text></g>
               <text id="t3" class="a imp" style="fill:#777" data-k="v">x</text></g>
            </svg>"#,
            |doc, st| {
                assert_eq!(st.own(by_id(doc, "t0"), "fill").as_deref(), Some("#111"));
                assert_eq!(st.own(by_id(doc, "t1"), "fill").as_deref(), Some("#333"), "child + class beats descendant");
                assert_eq!(st.own(by_id(doc, "t1"), "stroke").as_deref(), Some("blue"), "multiple classes");
                assert_eq!(st.own(by_id(doc, "t2"), "fill").as_deref(), Some("#666"), "style attribute beats #id");
                assert_eq!(st.own(by_id(doc, "t3"), "fill").as_deref(), Some("#555"), "!important beats style");
                assert_eq!(st.own(by_id(doc, "t3"), "font-weight").as_deref(), Some("bold"));
                assert_eq!(st.own(by_id(doc, "t2"), "stroke"), None, "`.a.b` needs both classes");
                assert_eq!(st.prop(by_id(doc, "t2"), "font-size").as_deref(), Some("9px"));
            },
        );
    }

    #[test]
    fn descendant_matching_does_not_backtrack_combinatorially() {
        // Many nested groups share class `a` and the first rule stacks eight descendant
        // `.a` compounds under a leftmost class that matches nothing: an unmemoized
        // matcher enumerates ancestor combinations for it and effectively never
        // finishes (#1703). With suffix states memoized this returns at once, and the
        // matching descendant and mixed child/descendant rules still apply.
        let depth = 48;
        let mut svg = String::from(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><style>
            .missing .a .a .a .a .a .a .a .a text { fill: #111 }
            .a .a .a text { fill: #222 }
            .a > .a text { stroke: blue }
            </style>"#,
        );
        for _ in 0..depth {
            svg.push_str(r#"<g class="a">"#);
        }
        svg.push_str(r#"<text id="t">x</text>"#);
        for _ in 0..depth {
            svg.push_str("</g>");
        }
        svg.push_str("</svg>");
        with_doc(&svg, |doc, st| {
            let t = by_id(doc, "t");
            assert_eq!(st.own(t, "fill").as_deref(), Some("#222"), "descendant rule matches, the .missing one does not");
            assert_eq!(st.own(t, "stroke").as_deref(), Some("blue"), "mixed child/descendant rule matches");
        });
    }

    #[test]
    fn presentation_attributes_lose_to_rules() {
        with_doc(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><style>#a { fill: blue }</style><text id="a" fill="red" stroke="red">x</text></svg>"#,
            |doc, st| {
                assert_eq!(st.own(by_id(doc, "a"), "fill").as_deref(), Some("blue"));
                assert_eq!(st.own(by_id(doc, "a"), "stroke").as_deref(), Some("red"));
            },
        );
    }

    /// What [`Styles::declared`] gave before the rules were indexed: every rule tested against `n`.
    fn declared_by_every_rule(st: &Styles, n: XNode) -> HashMap<String, String> {
        type Key = (bool, bool, (u32, u32, u32), usize);
        let mut best: HashMap<String, (Key, String)> = HashMap::new();
        let mut put = |name: &str, value: &str, key: Key| {
            if best.get(name).is_none_or(|(k, _)| key >= *k) {
                best.insert(name.to_string(), (key, value.to_string()));
            }
        };
        let mut order = 0;
        for r in st.rules.iter().filter(|r| r.selector.matches(n)) {
            for (k, v, imp) in r.decls.iter() {
                order += 1;
                put(k, v, (*imp, false, r.selector.specificity, order));
            }
        }
        if let Some(s) = n.attribute("style") {
            for (k, v, imp) in parse_decls(s) {
                order += 1;
                put(&k, &v, (imp, true, (0, 0, 0), order));
            }
        }
        best.into_iter().map(|(k, (_, v))| (k, v)).collect()
    }

    /// Checks that each element of `svg` gets the properties [`declared_by_every_rule`] gives it.
    fn assert_same_as_every_rule(svg: &str) {
        let doc = roxmltree::Document::parse(svg).unwrap();
        let st = Styles::new(&doc);
        for n in doc.descendants().filter(|n| n.is_element()) {
            assert_eq!(*st.declared(n), declared_by_every_rule(&st, n), "{n:?} in {svg}");
        }
    }

    /// Id, class, type, universal and attribute selectors, descendant and child combinators, and
    /// rules that override each other give each element what testing every rule gives it.
    #[test]
    fn indexed_rules_give_what_testing_every_rule_gives() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><style>
            * { stroke: s0 }
            g rect { fill: f1 }
            #a { fill: f2 }
            .a { fill: f3 }
            rect.a { fill: f4 }
            .b.a { stroke: s5 }
            g > .c { fill: f6 }
            svg > g g rect { font-size: 9px }
            [data-k=v] { display: none }
            #a > tspan, text .x { font-size: 8px }
            .c { fill: f8 !important }
            rect { fill: f9 }
            .a { fill: f10 }
            [data-k] { fill: f11 }
            </style>
            <g class="c">
              <rect id="a" class="a"/>
              <rect id="r2" class="b&#9;a&#10;a"/>
              <g class="c x"><rect id="r3" class="c" data-k="v"/><rect id="r4"/></g>
              <circle id="c5" class="a x" data-k="w" style="stroke: s !important"/>
            </g>
            <text id="a" class="a"><tspan id="t6" class="x">t</tspan><tspan id="t7">u</tspan></text>
            <rect id="r8" class="z"/>
            </svg>"#;
        assert_same_as_every_rule(svg);
        with_doc(svg, |doc, st| {
            let own = |id: &str, name: &str| st.own(by_id(doc, id), name);
            assert_eq!(own("a", "fill").as_deref(), Some("f2"), "#a beats rect.a");
            assert_eq!(own("r2", "fill").as_deref(), Some("f4"), "rect.a beats g rect");
            assert_eq!(own("r2", "stroke").as_deref(), Some("s5"), "classes apart by a tab and a line feed, one repeated");
            assert_eq!(own("r3", "fill").as_deref(), Some("f8"), "!important");
            assert_eq!(own("r3", "display").as_deref(), Some("none"));
            assert_eq!(own("r4", "fill").as_deref(), Some("f1"), "g rect beats rect");
            assert_eq!(own("r4", "font-size").as_deref(), Some("9px"));
            assert_eq!(own("c5", "fill").as_deref(), Some("f11"), "of rules alike, the last one");
            assert_eq!(own("c5", "stroke").as_deref(), Some("s"));
            assert_eq!(own("t7", "font-size").as_deref(), Some("8px"));
            assert_eq!(own("r8", "fill").as_deref(), Some("f9"));
        });
    }

    /// An element's candidates hold each rule once, however often the element repeats a class.
    #[test]
    fn repeated_classes_give_each_rule_once() {
        with_doc(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><style>.a { fill: x } .b { stroke: y } .a.b { opacity: 0.5 } rect { fill: z }</style><rect id="r" class="a a b a b"/></svg>"#,
            |doc, st| assert_eq!(st.index.candidates(by_id(doc, "r")), [0, 1, 2, 3]),
        );
    }

    /// A compound selector over a few tags, ids, classes and attributes.
    fn arb_compound() -> impl Strategy<Value = String> {
        let tag = prop::sample::select(vec!["", "*", "g", "rect", "text", "tspan"]);
        let id = prop::sample::select(vec!["", "#a", "#b"]);
        let classes = prop::collection::vec(prop::sample::select(vec![".a", ".b", ".c"]), 0..3);
        let attr = prop::sample::select(vec!["", "[data-k]", "[data-k=v]"]);
        (tag, id, classes, attr).prop_map(|(t, i, c, a)| match format!("{t}{i}{}{a}", c.concat()) {
            s if s.is_empty() => "*".to_string(),
            s => s,
        })
    }

    /// One to three compounds joined by descendant and child combinators.
    fn arb_selector() -> impl Strategy<Value = String> {
        (arb_compound(), prop::collection::vec((prop::sample::select(vec![" ", " > ", ">"]), arb_compound()), 0..3))
            .prop_map(|(first, rest)| rest.into_iter().fold(first, |s, (comb, c)| format!("{s}{comb}{c}")))
    }

    /// A rule of one or two selectors that sets one to three properties, some of them `!important`.
    fn arb_rule() -> impl Strategy<Value = String> {
        let decl = (prop::sample::select(vec!["fill", "stroke", "display"]), 0..1000u16, any::<bool>())
            .prop_map(|(name, v, important)| format!("{name}: v{v}{}", if important { " !important" } else { "" }));
        (prop::collection::vec(arb_selector(), 1..3), prop::collection::vec(decl, 1..4))
            .prop_map(|(selectors, decls)| format!("{} {{ {} }}", selectors.join(", "), decls.join("; ")))
    }

    /// An element that may have an id, classes (some repeated, apart by spaces, tabs and line feeds)
    /// and a `data-k` or `style` attribute, with up to four children on each of the `depth` levels
    /// below it.
    fn arb_element(depth: u32) -> BoxedStrategy<String> {
        let tag = prop::sample::select(vec!["g", "rect", "text", "tspan"]);
        let id = prop::sample::select(vec!["", r#" id="a""#, r#" id="b""#, r#" id="c""#]);
        let classes = prop::collection::vec((prop::sample::select(vec![" ", "&#9;", "&#10;"]), prop::sample::select(vec!["a", "b", "c", "x"])), 0..4)
            .prop_map(|v| match v.iter().map(|(gap, c)| format!("{gap}{c}")).collect::<String>() {
                s if s.is_empty() => s,
                s => format!(r#" class="{s}""#),
            });
        let more = prop::sample::select(vec!["", r#" data-k="v""#, r#" data-k="w""#, r#" style="fill: s""#, r#" style="stroke: s !important""#]);
        let children = if depth == 0 { Just(vec![]).boxed() } else { prop::collection::vec(arb_element(depth - 1), 0..5).boxed() };
        (tag, id, classes, more, children).prop_map(|(t, i, c, m, ch)| format!("<{t}{i}{c}{m}>{}</{t}>", ch.concat())).boxed()
    }

    proptest! {
        #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

        /// Random style sheets give each element of a random document what testing every rule
        /// gives it.
        #[test]
        fn random_sheets_give_what_testing_every_rule_gives(rules in prop::collection::vec(arb_rule(), 0..24), body in prop::collection::vec(arb_element(3), 1..6)) {
            assert_same_as_every_rule(&format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><style>{}</style>{}</svg>"#, rules.join("\n"), body.concat()));
        }
    }

    /// 50,000 class rules give 50,000 elements, each with one of the classes, their fill within
    /// 20 s.
    #[test]
    fn styling_50000_elements_with_50000_rules_finishes_within_20_s() {
        const N: usize = 50_000;
        let rules: String = (0..N).map(|i| format!(".c{i} {{ fill: #{i:06x} }}\n")).collect();
        let rects: String = (0..N).map(|i| format!(r#"<rect class="c{i}"/>"#)).collect();
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><style>{rules}</style>{rects}</svg>"#);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let doc = roxmltree::Document::parse(&svg).unwrap();
            let st = Styles::new(&doc);
            let fills: Vec<Option<String>> = doc.descendants().filter(|n| n.tag_name().name() == "rect").map(|n| st.own(n, "fill")).collect();
            tx.send(fills).ok();
        });
        let fills = rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap_or_else(|_| panic!("styling {N} elements did not finish within 20 s"));
        assert_eq!(fills.len(), N);
        for (i, fill) in fills.iter().enumerate() {
            assert_eq!(fill.as_deref(), Some(format!("#{i:06x}").as_str()));
        }
    }

    #[test]
    fn the_font_shorthand_sets_its_longhands() {
        let d = parse_decls("fill: red; font: italic bold 30px 'DejaVu Sans' ; font-weight: 300 !important; font: nonsense");
        let get = |k: &str| d.iter().filter(|(n, ..)| n == k).map(|(_, v, i)| (v.as_str(), *i)).collect::<Vec<_>>();
        assert_eq!(get("font-style"), [("italic", false)]);
        assert_eq!(get("font-weight"), [("bold", false), ("300", true)]);
        assert_eq!(get("font-size"), [("30px", false)]);
        assert_eq!(get("font-family"), [("'DejaVu Sans'", false)]);
        assert_eq!(get("font-variant"), [("normal", false)]);
        assert_eq!(get("fill"), [("red", false)]);
        assert!(get("font").is_empty(), "the shorthand itself is not kept");
    }

    #[test]
    fn unsupported_selectors_are_dropped() {
        for s in ["a + b", "a ~ b", "a:hover", "svg|text", "> a", "a >", "", "a:not(b c)", "a:not(.x", "a:not(b > c)", "a:not(:hover)"] {
            assert!(Selector::parse(s).is_none(), "{s}");
        }
        let s = Selector::parse("g  >  text.x#y").unwrap();
        assert_eq!(s.specificity, (1, 1, 2));
        assert_eq!(s.parts.len(), 2);
        assert!(s.parts[0].1, "text must be a child of g");
    }

    /// #1557: `:not()` of a list of compound selectors; it counts as its most specific argument.
    #[test]
    fn not_selectors() {
        assert_eq!(Selector::parse("rect:not(.keep)").unwrap().specificity, (0, 1, 1));
        assert_eq!(Selector::parse("g :not(#a, .b.c, text)").unwrap().specificity, (1, 0, 1));
        assert_eq!(Selector::parse(":not(rect):not([x])").unwrap().specificity, (0, 1, 1));
        // Nesting is bounded: a hostile sheet can't recurse without end.
        let nested = |n: usize| format!("{}.x{}", ":not(".repeat(n), ")".repeat(n));
        assert!(Selector::parse(&nested(MAX_NOT_NESTING)).is_some());
        assert!(Selector::parse(&nested(MAX_NOT_NESTING + 1)).is_none());
        assert!(Selector::parse(&nested(200_000)).is_none());
        with_doc(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><style>
            rect:not(.keep) { fill: green }
            g > :not(rect, #t1) { stroke: blue }
            :not(:not(.both)) { opacity: 0.5 }
            .keep { fill: red }
            </style>
            <rect id="r0" fill="red"/><rect id="r1" class="keep"/>
            <g><text id="t0">x</text><text id="t1">x</text><rect id="r2" class="both"/></g>
            </svg>"#,
            |doc, st| {
                assert_eq!(st.own(by_id(doc, "r0"), "fill").as_deref(), Some("green"));
                assert_eq!(st.own(by_id(doc, "r1"), "fill").as_deref(), Some("red"));
                assert_eq!(st.own(by_id(doc, "t0"), "stroke").as_deref(), Some("blue"));
                assert_eq!(st.own(by_id(doc, "t1"), "stroke"), None);
                assert_eq!(st.own(by_id(doc, "r2"), "stroke"), None);
                assert_eq!(st.own(by_id(doc, "r2"), "opacity").as_deref(), Some("0.5"));
                assert_eq!(st.own(by_id(doc, "r0"), "opacity"), None);
                assert!(st.has_not_rules());
                assert_eq!(st.set_by_not_rules(by_id(doc, "r0")), [("fill".to_string(), "green".to_string())]);
                assert!(st.set_by_not_rules(by_id(doc, "r1")).is_empty());
            },
        );
    }
}
