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
    split_decls(s)
        .into_iter()
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

/// Splits on the `;`s that are outside quoted strings and parentheses (`url(data:…;base64,…)`).
fn split_decls(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut depth, mut quote, mut escaped) = (0, 0usize, None::<char>, false);
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if let Some(q) = quote {
            if c == q {
                quote = None;
            }
        } else {
            match c {
                '\'' | '"' => quote = Some(c),
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ';' if depth == 0 => {
                    parts.push(s.get(start..i).unwrap_or_default());
                    start = i + 1;
                }
                _ => {}
            }
        }
    }
    parts.push(s.get(start..).unwrap_or_default());
    parts
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
    // Drop comments outside strings.
    let mut text = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    let mut quote = None;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            text.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            text.push(c);
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev_star = false;
            for c in chars.by_ref() {
                if prev_star && c == '/' {
                    break;
                }
                prev_star = c == '*';
            }
        } else {
            text.push(c);
        }
    }
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
mod tests;
