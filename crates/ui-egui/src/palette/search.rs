//! The command palette's search: a small index of the words in each result's label, menu path and
//! id, the menu bar's home of each result, and the ranking of the results a query matches.

use std::collections::HashMap;

use super::{Entry, WRAPPED};
use crate::menus;

/// The most characters of a query the index reads (VectorCraft's own limit). The substring rule
/// and the exact tiers read the whole query.
const INDEX_CHARS: usize = 200;

/// The field of a result a word comes from, best first.
const LABEL: u8 = 0;
const PATH: u8 = 1;
const ID: u8 = 2;

/// A word's grade in a result, best first: a whole word of the label, a word of the label it
/// starts, a word of the menu path it is or starts, a word of the id it is or starts, or text
/// inside a word of any field ([`super::matching`]'s substring rule).
const LABEL_WORD: u8 = 1;
const LABEL_PREFIX: u8 = 2;
const PATH_WORD: u8 = 3;
const ID_WORD: u8 = 4;
const SUBSTRING: u8 = 5;
const NONE: u8 = u8::MAX;

/// How a whole query matches a result, best first: it is the result's id, it is a label, a label
/// starts with it, or it is the initials of a label.
const EXACT_ID: u8 = 0;
const EXACT_LABEL: u8 = 1;
const PREFIX: u8 = 2;
const INITIALS: u8 = 3;
const NO_PHRASE: u8 = 4;

/// One result in the list of a term: the result's place in the catalog and the field the term is
/// a word of.
#[derive(Clone, Copy)]
struct Posting {
    entry: u16,
    field: u8,
}

/// The words of every result, each distinct word stored once.
#[derive(Default)]
pub(crate) struct Index {
    /// Every distinct word, one after another.
    term_text: String,
    /// Each word's offset and length in `term_text`, sorted by the word's text.
    terms: Vec<(u32, u32)>,
    /// Each word's first posting and number of postings in `postings`, in the order of `terms`.
    spans: Vec<(u32, u32)>,
    postings: Vec<Posting>,
    /// The initials of each result's labels, one line per label.
    initials_text: String,
    /// Each result's offset and length in `initials_text`.
    initials: Vec<(u32, u16)>,
}

/// Splits `text` into lowercase words at whitespace and at `.`, `-`, `/`, `(`, `)`, `…`, `›`, `,`
/// and `:`.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '.' | '-' | '/' | '(' | ')' | '…' | '›' | ',' | ':')).filter(|w| !w.is_empty())
}

/// The first letter of each word of `label`, words split at whitespace and at `-`, `/`, `(` and
/// `›`, lowercased.
fn initials_of(label: &str) -> String {
    label
        .split(|c: char| c.is_whitespace() || matches!(c, '-' | '/' | '(' | '›'))
        .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
        .flat_map(char::to_lowercase)
        .collect()
}

/// A name's last `›`-separated part (the label) and the parts before it (the menu path).
fn split_name(name: &str) -> (&str, &str) {
    match name.rsplit_once(" › ") {
        Some((path, label)) => (label, path),
        None => (name, ""),
    }
}

/// `name` without the id that follows it when the result shows its id.
fn without_id<'a>(name: &'a str, id: &str) -> &'a str {
    name.strip_suffix(')').and_then(|n| n.strip_suffix(id)).and_then(|n| n.strip_suffix(" (")).unwrap_or(name)
}

/// The lowercased names a result is found by: the plain English name, the English name and the
/// shown name ([`super::build`]'s haystack lines).
fn names(e: &Entry) -> impl Iterator<Item = &str> {
    e.haystack.lines().take(3)
}

/// The lowercased ids a result is found by: its own, and the engine command's a UI command wraps.
fn ids(e: &Entry) -> impl Iterator<Item = &str> {
    e.haystack.lines().skip(3)
}

impl Index {
    /// The index of `entries`, whose menu homes and menu items read `homes` in the UI language. The
    /// words of a result's menu home and of its menu item's label count as menu path words. A
    /// catalog with more results than a posting can name gets an empty index, and queries match by
    /// the substring rule alone.
    pub(crate) fn build(entries: &[Entry], homes: &[Option<String>]) -> Index {
        if entries.len() > usize::from(u16::MAX) {
            return Index::default();
        }
        let mut found: Vec<(String, u16, u8)> = vec![];
        let mut initials_text = String::new();
        let mut initials = Vec::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            let Ok(entry) = u16::try_from(i) else { break };
            let id = ids(e).next().unwrap_or_default();
            let start = initials_text.len();
            let mut seen_initials: Vec<String> = vec![];
            for name in names(e) {
                let (label, path) = split_name(without_id(name, id));
                found.extend(words(label).map(|w| (w.to_string(), entry, LABEL)));
                found.extend(words(path).map(|w| (w.to_string(), entry, PATH)));
                let ini = initials_of(label);
                if ini.chars().count() >= 2 && !seen_initials.contains(&ini) {
                    if !seen_initials.is_empty() {
                        initials_text.push('\n');
                    }
                    initials_text.push_str(&ini);
                    seen_initials.push(ini);
                }
            }
            for home in [e.home.as_deref(), Some(e.menu_item), homes.get(i).and_then(Option::as_deref)].into_iter().flatten() {
                found.extend(words(&home.to_lowercase()).map(|w| (w.to_string(), entry, PATH)));
            }
            for id in ids(e) {
                found.extend(words(id).map(|w| (w.to_string(), entry, ID)));
            }
            let len = u16::try_from(initials_text.len() - start).unwrap_or(0);
            initials.push((u32::try_from(start).unwrap_or(0), len));
        }
        found.sort_unstable();
        found.dedup();
        let mut index = Index { initials_text, initials, ..Index::default() };
        let mut last: Option<&str> = None;
        for (word, entry, field) in &found {
            if last != Some(word.as_str()) {
                let (Ok(offset), Ok(len), Ok(start)) =
                    (u32::try_from(index.term_text.len()), u32::try_from(word.len()), u32::try_from(index.postings.len()))
                else {
                    return Index::default();
                };
                index.term_text.push_str(word);
                index.terms.push((offset, len));
                index.spans.push((start, 0));
                last = Some(word.as_str());
            }
            index.postings.push(Posting { entry: *entry, field: *field });
            if let Some(span) = index.spans.last_mut() {
                span.1 += 1;
            }
        }
        index.term_text.shrink_to_fit();
        index.terms.shrink_to_fit();
        index.spans.shrink_to_fit();
        index.postings.shrink_to_fit();
        index.initials_text.shrink_to_fit();
        index.initials.shrink_to_fit();
        index
    }

    /// The bytes the index holds.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.term_text.capacity()
            + self.terms.capacity() * std::mem::size_of::<(u32, u32)>()
            + self.spans.capacity() * std::mem::size_of::<(u32, u32)>()
            + self.postings.capacity() * std::mem::size_of::<Posting>()
            + self.initials_text.capacity()
            + self.initials.capacity() * std::mem::size_of::<(u32, u16)>()
    }

    /// The text of the word at `(offset, len)` in `term_text`.
    fn text(&self, &(offset, len): &(u32, u32)) -> &str {
        let offset = offset as usize;
        self.term_text.get(offset..offset + len as usize).unwrap_or_default()
    }

    /// The best grade (1 to 4) `word` gets in each result, [`NONE`] where it is no word and starts
    /// no word of the result. The words it starts are found by a binary search of the sorted words.
    fn grades(&self, word: &str, n: usize) -> Vec<u8> {
        let mut out = vec![NONE; n];
        let start = self.terms.partition_point(|t| self.text(t) < word);
        for (t, at) in self.terms.iter().enumerate().skip(start) {
            let term = self.text(at);
            if !term.starts_with(word) {
                break;
            }
            let whole = term == word;
            let Some(&(begin, len)) = self.spans.get(t) else { continue };
            let (begin, len) = (begin as usize, len as usize);
            for p in self.postings.get(begin..begin + len).unwrap_or_default() {
                let grade = match p.field {
                    LABEL if whole => LABEL_WORD,
                    LABEL => LABEL_PREFIX,
                    PATH => PATH_WORD,
                    _ => ID_WORD,
                };
                if let Some(g) = out.get_mut(usize::from(p.entry)) {
                    *g = (*g).min(grade);
                }
            }
        }
        out
    }

    /// Is `query` (no spaces) the initials of one of result `i`'s labels?
    fn initials_are(&self, i: usize, query: &str) -> bool {
        let Some(&(offset, len)) = self.initials.get(i) else { return false };
        let (offset, len) = (offset as usize, usize::from(len));
        self.initials_text.get(offset..offset + len).is_some_and(|text| text.lines().any(|l| l == query))
    }
}

/// `query` trimmed, lowercased, with each run of whitespace made one space.
pub(crate) fn normalize(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The order of a result's kind when everything else ties: a tool, a command of the menu bar, a
/// panel, an effect, a filter plug-in, then a command with no item in the menu bar.
pub(crate) fn kind(id: &str, home: Option<&str>) -> u8 {
    match id.split_once(':').map(|(k, _)| k) {
        Some("tool") => 0,
        Some("panel") => 2,
        Some("effect") => 3,
        Some("plugin") => 4,
        _ if home.is_some() => 1,
        _ => 5,
    }
}

/// How the whole lowercased `query` matches result `e` ([`EXACT_ID`] to [`NO_PHRASE`]), where
/// `initials` says whether the query is the initials of one of its labels. A query starts a name
/// when it starts the name's label, or, for a query that holds `›`, the name with its menu path.
fn phrase(e: &Entry, query: &str, initials: bool) -> u8 {
    if ids(e).any(|id| id == query) {
        return EXACT_ID;
    }
    let trimmed = query.trim_end_matches('…');
    let labels = || names(e).flat_map(|n| [n, split_name(n).0]);
    if labels().any(|l| l == query || l.trim_end_matches('…') == trimmed) {
        return EXACT_LABEL;
    }
    // A menu's title alone does not put the items of that menu in this tier.
    let with_path = query.contains('›');
    if names(e).any(|n| split_name(n).0.starts_with(query) || (with_path && n.starts_with(query))) {
        return PREFIX;
    }
    if initials {
        return INITIALS;
    }
    NO_PHRASE
}

/// What results are ordered by, compared field by field: how the whole query matches, the worst
/// grade a word gets, the sum of the grades, the kind of result and the label's length.
type RankKey = (u8, u8, u32, u8, usize);

/// The results of `entries` that the lowercased `query` matches, best first, as places in
/// `entries`. A result matches when every word of the query is text in one of its names or ids
/// (the rule the palette always had, so nothing it found is lost), when every word of the query is
/// or starts a word of its label, menu path or id, or when the query is the initials of a label
/// ("ea" for Export As…). Results are ordered by how the whole query matches ([`phrase`]), then by
/// the worst grade a word gets, the sum of the grades, the kind of result ([`kind`]), the label's
/// length and the catalog order. An empty query lists every result in catalog order.
pub(crate) fn rank(entries: &[Entry], index: &Index, query: &str) -> Vec<u32> {
    let query = normalize(query);
    if query.is_empty() {
        return (0..entries.len()).filter_map(|i| u32::try_from(i).ok()).collect();
    }
    let spaced: Vec<&str> = query.split(' ').collect();
    let head: String = query.chars().take(INDEX_CHARS).collect();
    let tokens: Vec<&str> = words(&head).collect();
    let n = entries.len();
    let indexed: Vec<Vec<u8>> = tokens.iter().map(|t| index.grades(t, n)).collect();
    // The index reads only the start of a longer query, so such a query matches by the substring
    // rule, which reads all of it.
    let fits = query.chars().count() <= INDEX_CHARS;
    let packed: String = query.chars().filter(|c| !c.is_whitespace()).collect();
    let packed = Some(packed).filter(|p| (2..=6).contains(&p.chars().count()) && p.chars().all(char::is_alphabetic));
    let mut ranked: Vec<(RankKey, u32)> = vec![];
    for (i, e) in entries.iter().enumerate() {
        let Ok(place) = u32::try_from(i) else { break };
        let substring = spaced.iter().all(|w| e.haystack.contains(w));
        let initials = packed.as_deref().is_some_and(|p| index.initials_are(i, p));
        // Each word's grade, the worst and their sum; with initials, every word counts as a word
        // the label starts, as the initials stand for every word of the label.
        let (mut all_indexed, mut worst, mut sum) = (fits && !tokens.is_empty(), 0u8, 0u32);
        for (grades, token) in indexed.iter().zip(&tokens) {
            let mut g = grades.get(i).copied().unwrap_or(NONE);
            if g == NONE {
                all_indexed = false;
                if e.haystack.contains(token) {
                    g = SUBSTRING;
                }
            }
            let g = if initials { g.min(LABEL_PREFIX) } else { g.min(SUBSTRING) };
            worst = worst.max(g);
            sum += u32::from(g);
        }
        if !(substring || all_indexed || initials) {
            continue;
        }
        let tier = phrase(e, &query, initials);
        let length = split_name(&e.shown).0.chars().count();
        ranked.push(((tier, worst, sum, e.kind, length), place));
    }
    ranked.sort_by_key(|&(key, place)| (key, place));
    ranked.into_iter().map(|(_, place)| place).collect()
}

/// The first place in the menu bar of each item a result runs, as an English `›`-separated path
/// of the menus that hold it: by command id, `panel:<id>` for a Window menu panel (Learn too),
/// `effect:<id>` for an Effect menu item and `plugin:<id>` for an Object › Plug-ins filter. With
/// the native Mac menu bar (`native`), About, Preferences (under Settings), Language, Appearance
/// and Quit are in the application menu, which comes first, as `native_menu::mac_layout` puts
/// them. The in-window menu bar shows no application menu. Each home comes with the English label
/// of its item (File › Exit for Quit VectorCraft), empty for the application menu's items.
pub(crate) fn menu_homes(native: bool) -> HashMap<String, (String, &'static str)> {
    fn key(id: &str, p: &serde_json::Value) -> String {
        let field = |k: &str| p.get(k).and_then(serde_json::Value::as_str);
        match (id, field("panel"), field("effect"), field("id")) {
            ("window.panel", Some(panel), ..) => format!("panel:{panel}"),
            ("window.learn", ..) => format!("panel:{}", crate::panels::learn::ID),
            ("effect.dialog" | "effect.apply", _, Some(effect), _) => format!("effect:{effect}"),
            ("plugin.dialog", _, _, Some(filter)) => format!("plugin:{filter}"),
            _ => WRAPPED.iter().find(|(engine, _)| *engine == id).map_or(id, |(_, ui)| ui).to_string(),
        }
    }
    fn walk(items: &[menus::Item], path: &mut Vec<&'static str>, skip: &[&str], out: &mut HashMap<String, (String, &'static str)>) {
        for it in items {
            match it {
                menus::Item::Cmd(label, id, p) if !skip.contains(id) => {
                    out.entry(key(id, p)).or_insert_with(|| (path.join(" › "), *label));
                }
                menus::Item::Sub(name, children) => {
                    path.push(name);
                    walk(children, path, skip, out);
                    path.pop();
                }
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    let app_menu: &[(&str, &str)] =
        &[("help.about", ""), ("edit.preferences", "Settings"), ("app.language", "Language"), ("window.brightness", "Appearance"), ("app.quit", "")];
    let skip: Vec<&str> = if native { app_menu.iter().map(|(id, _)| *id).collect() } else { vec![] };
    if native {
        for (id, sub) in app_menu {
            let path = if sub.is_empty() { crate::native_menu::APP_NAME.to_string() } else { format!("{} › {sub}", crate::native_menu::APP_NAME) };
            out.insert((*id).to_string(), (path, ""));
        }
    }
    // The tree's first menu is the application menu, which the in-window bar never shows and
    // the native bar builds itself.
    for (name, items) in menus::menu_tree().into_iter().skip(1) {
        walk(&items, &mut vec![name], &skip, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{Catalog, catalog};
    use super::*;
    use crate::i18n::Lang;

    /// The ids of the results `query` lists, best first.
    fn ranked(c: &Catalog, query: &str) -> Vec<String> {
        rank(&c.list, &c.index, query).iter().filter_map(|&i| c.list.get(i as usize)).map(|e| e.id.clone()).collect()
    }

    /// A result named `name` (`›`-separated, its last part the label) with id `id`.
    fn entry(name: &str, id: &str) -> Entry {
        let home = None;
        Entry {
            id: id.to_string(),
            shortcut: String::new(),
            shown: name.to_string(),
            haystack: [name, name, name, id].map(str::to_lowercase).join("\n"),
            kind: kind(id, home.as_deref()),
            home,
            menu_item: "",
        }
    }

    /// `entries` with their index.
    fn synthetic(entries: Vec<Entry>) -> Catalog {
        let index = Index::build(&entries, &[]);
        Catalog { list: entries, index, ranked: std::sync::Mutex::new(None) }
    }

    /// Where `id` is among the results of `query`.
    fn place(c: &Catalog, query: &str, id: &str) -> Option<usize> {
        ranked(c, query).iter().position(|r| r == id)
    }

    /// For each pair of adjacent word grades, a result where the word gets the better grade ranks
    /// above one where it gets the worse, and both are listed. The second word of each query is a
    /// whole word of every label, so no whole-query tier applies.
    #[test]
    fn a_better_word_grade_ranks_first() {
        for (query, better, worse) in [
            // A whole word of the label, then a word of the label it starts.
            ("cut zz", entry("Zz Cut", "a.one"), entry("Zz Cutter", "b.two")),
            // A word of the label it starts, then a word of the menu path.
            ("fill zz", entry("Zz Filling", "a.one"), entry("Fill › Zz Other", "b.two")),
            // A word of the menu path, then a word of the id.
            ("zap zz", entry("Zap › Zz One", "a.one"), entry("Zz Two", "zap.two")),
            // A word of the id, then text inside a word.
            ("zip zz", entry("Zz Three", "zip.three"), entry("Zz Unzipped", "b.two")),
        ] {
            let (b, w) = (better.id.clone(), worse.id.clone());
            // The worse one comes first in the catalog, so only the grade puts the better first.
            let c = synthetic(vec![worse, better]);
            assert_eq!(ranked(&c, query), [b, w], "{query}");
        }
    }

    /// For each pair of adjacent whole-query tiers, a result in the better tier ranks above one in
    /// the worse.
    #[test]
    fn a_better_whole_query_tier_ranks_first() {
        for (query, better, worse) in [
            // The query is the result's id, then it is a label.
            ("zz.one", entry("Alpha", "zz.one"), entry("zz.one", "b.two")),
            // The query is a label, then a label starts with it.
            ("snap", entry("Snap", "a.one"), entry("Snapshot", "b.two")),
            // A label starts with the query, then the query is a label's initials.
            ("ab", entry("Abacus", "a.one"), entry("Alpha Beta", "b.two")),
            // The query is a label's initials, then a whole word of a label that it does not start.
            ("gb", entry("Green Blue", "a.one"), entry("Zz Gb", "b.two")),
        ] {
            let (b, w) = (better.id.clone(), worse.id.clone());
            let c = synthetic(vec![worse, better]);
            assert_eq!(ranked(&c, query), [b, w], "{query}");
        }
    }

    /// The kind of result and the label's length order results only when the grades tie: a tool
    /// with a short label stays below a command whose label holds the word whole. Ties keep the
    /// catalog order, the same on every ranking.
    #[test]
    fn kind_and_length_never_lift_a_worse_grade() {
        let c = synthetic(vec![entry("Zz Cutx", "tool:cutx"), entry("Zz Cut With a Much Longer Label", "a.one")]);
        assert_eq!(ranked(&c, "cut zz"), ["a.one", "tool:cutx"]);
        let c = synthetic(vec![entry("Zz Cut Long Label", "a.one"), entry("Zz Cut Long", "b.two"), entry("Zz Cut", "tool:cut")]);
        assert_eq!(ranked(&c, "cut zz"), ["tool:cut", "b.two", "a.one"], "the tool first, then the shorter label");
        let c = synthetic(vec![entry("Zz Same", "a.one"), entry("Zz Same", "b.two"), entry("Zz Same", "c.three")]);
        assert_eq!(ranked(&c, "same zz"), ["a.one", "b.two", "c.three"]);
        assert_eq!(ranked(&c, "same zz"), ranked(&c, "same zz"));
    }

    /// A query that is a menu's title ranks a result whose label holds it as a word above the items
    /// of that menu, which match only by their menu path. A query that names the menu with `›`
    /// still starts the item's name.
    #[test]
    fn a_menu_title_does_not_lift_its_items() {
        let c = synthetic(vec![entry("Object › Paint", "a.one"), entry("Outline Object", "b.two")]);
        assert_eq!(ranked(&c, "object"), ["b.two", "a.one"]);
        assert_eq!(ranked(&c, "object › pa"), ["a.one"]);
        let en = catalog(Lang::EN, false);
        for q in ["object", "effect", "select", "view", "type", "edit"] {
            let first = rank(&en.list, &en.index, q).first().and_then(|&i| en.list.get(i as usize)).map(|e| e.shown.to_lowercase());
            let label = first.as_deref().map(|n| split_name(n).0.to_string()).unwrap_or_default();
            assert!(words(&label).any(|w| w.starts_with(q)), "{q}: {first:?}");
        }
    }

    /// The application menu of the native Mac menu bar holds the items [`menu_homes`] puts there,
    /// under the same submenus.
    #[test]
    fn native_homes_follow_the_native_application_menu() {
        use crate::native_menu::{self, MenuRole, Node};
        fn walk(nodes: &[Node], path: &mut Vec<String>, out: &mut HashMap<String, String>) {
            for n in nodes {
                match n {
                    Node::Item(it) => {
                        if let Some(id) = it.command {
                            out.entry(id.to_string()).or_insert_with(|| path.join(" › "));
                        }
                    }
                    Node::Submenu { label, children } => {
                        path.push(label.clone());
                        walk(children, path, out);
                        path.pop();
                    }
                    _ => {}
                }
            }
        }
        let app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let bar = native_menu::mac_layout(&app, &native_menu::from_tree(&app, &menus::menu_tree()), Lang::EN).bar;
        let app_menu = bar.menus.iter().find(|m| m.role == MenuRole::App).unwrap();
        let mut found = HashMap::new();
        walk(&app_menu.children, &mut vec![app_menu.title.clone()], &mut found);
        // The items the native bar runs itself (Hide, Hide Others, Minimize) are not palette results.
        found.retain(|id, _| ![native_menu::HIDE, native_menu::HIDE_OTHERS, native_menu::MINIMIZE].contains(&id.as_str()));
        let homes = menu_homes(true);
        let expected: HashMap<String, String> = homes
            .iter()
            .filter(|(_, (path, _))| path.split(" › ").next() == Some(native_menu::APP_NAME))
            .map(|(id, (path, _))| (id.clone(), path.clone()))
            .collect();
        assert!(!expected.is_empty());
        assert_eq!(found, expected);
    }

    /// Queries from the list of things people type, on the English results: some list their command
    /// first, others within the first few results.
    #[test]
    fn common_queries_list_the_command_near_the_top() {
        let c = catalog(Lang::EN, false);
        let first = |q: &str| ranked(&c, q).first().cloned().unwrap_or_default();
        let within = |q: &str, id: &str, n: usize| place(&c, q, id).is_some_and(|p| p < n);
        assert_eq!(first("ea"), "file.exportAs", "the initials of Export As…");
        assert_eq!(first("save"), "file.save");
        assert!(within("export", "file.exportAs", 3));
        assert_eq!(first("rect"), "tool:rectangle");
        let (tool, effect) = (place(&c, "rect", "tool:rectangle"), place(&c, "rect", "effect:convertToShape.rectangle"));
        assert!(effect.is_some() && tool < effect, "{tool:?} {effect:?}");
        assert!(within("type", "tool:type", 2));
        assert!(within("layers", "panel:layers", 3));
        assert_eq!(first("drop shadow"), "effect:stylize.dropShadow");
        assert_eq!(first("view.goto1"), "view.goto1", "the whole id, above Saved View 10");
        assert_eq!(first("bring front"), "object.arrange.bringToFront");
        assert!(within("ruler", "view.rulers", 4));
        assert!(place(&c, "ruler", "perspective.grid.rulers").is_some());
        assert_eq!(first("deselect"), "select.none");
        let mut blurs: Vec<String> = ranked(&c, "blur").into_iter().take(3).collect();
        blurs.sort();
        assert_eq!(blurs, ["effect:blur.gaussian", "effect:blur.radial", "effect:blur.smart"]);
        assert!(ranked(&c, "zzzz").is_empty());
        // English names are searched in every language.
        let zh = catalog(Lang::from_code("zh-hant").unwrap(), false);
        for q in ["save", "rect"] {
            assert_eq!(ranked(&zh, q).first(), ranked(&c, q).first(), "{q}");
        }
    }

    /// What the palette always matched: every word of the query is text in one of the result's
    /// names or ids.
    fn substring_rule<'a>(c: &'a Catalog, query: &str) -> Vec<&'a str> {
        let q = normalize(query);
        c.list.iter().filter(|e| q.split_whitespace().all(|w| e.haystack.contains(w))).map(|e| e.id.as_str()).collect()
    }

    /// Each result's English name, shown name (in English, German and Chinese, Traditional) and id
    /// find it, first unless another result has that name; its id always lists it first.
    #[test]
    fn every_result_is_found_first_by_its_name_and_its_id() {
        for code in ["en", "de", "zh-hant"] {
            let c = catalog(Lang::from_code(code).unwrap(), false);
            let label = |n: &str| split_name(n).0.trim_end_matches('…').to_string();
            for e in &c.list {
                let english = e.haystack.lines().nth(1).unwrap_or_default();
                for query in [english, &e.shown] {
                    let results = ranked(&c, query);
                    assert!(results.contains(&e.id), "{code}: {query} finds {}", e.id);
                    let q = normalize(query);
                    let named = c.list.iter().filter(|o| names(o).any(|n| n == q || label(n) == label(&q))).count();
                    if named <= 1 {
                        assert_eq!(results.first(), Some(&e.id), "{code}: {query}");
                    }
                }
                assert_eq!(ranked(&c, &e.id).first(), Some(&e.id), "{code}: {}", e.id);
            }
        }
    }

    /// The queries the palette's tests use and a list of things people type.
    const QUERIES: &[&str] = &[
        "select",
        "zoom",
        "zoom in",
        "show panel",
        "group",
        "layers",
        "color guide",
        "drop shadow",
        "use precise cursors",
        "continue in background",
        "export continue in background",
        "show rulers",
        "delete selection",
        "cancel key object",
        "fit headline",
        "batch",
        "flattener preview",
        "panel options",
        "close path",
        "knife",
        "play action",
        "reset panel",
        "other library",
        "ellipse",
        "blur",
        "save",
        "export",
        "rect",
        "type",
        "ea",
        "bring front",
        "ruler",
        "deselect",
        "view.goto1",
        "zzzz",
        "pen",
        "outlines",
        "join paths",
        "rounded corners",
        "scaling",
        "rotated",
        "eyedroper",
        "rectange",
        "elipse",
        "gruop",
        "remove bg",
        "outline text",
        "flip",
        "combine",
        "vectorize",
        "colour",
        "transparent background",
        "png",
        "object.group",
        "file.exportinbackground",
        "window",
        "s",
        "e",
        "…",
        "› ",
    ];

    /// For each query, every result the palette found before is still found, and when it found 50
    /// or fewer, each of them is among the first 50 results, so the list still shows it.
    #[test]
    fn every_result_found_before_is_still_found_and_shown() {
        assert_eq!(QUERIES.len(), 60);
        let c = catalog(Lang::EN, false);
        for q in QUERIES {
            let before = substring_rule(&c, q);
            let now = ranked(&c, q);
            for id in &before {
                let at = now.iter().position(|r| r == id);
                assert!(at.is_some(), "{q}: {id} is still found");
                if before.len() <= super::super::MAX_RESULTS {
                    assert!(at < Some(super::super::MAX_RESULTS), "{q}: {id} is shown at {at:?}");
                }
            }
        }
    }

    /// A query longer than the index reads still has to match in full: two labels that share their
    /// first 200 characters are told apart by the rest.
    #[test]
    fn a_long_query_matches_in_full() {
        let shared = "a".repeat(200);
        let c = synthetic(vec![entry(&format!("{shared}b tail"), "a.one"), entry(&format!("{shared}c tail"), "b.two")]);
        let query = format!("{shared}b");
        assert_eq!(substring_rule(&c, &query), ["a.one"]);
        assert_eq!(ranked(&c, &query), ["a.one"]);
    }

    /// Text in scripts without spaces, right-to-left text, combining characters, a very long pasted
    /// query, punctuation alone and names that differ only in the id they show.
    #[test]
    fn unusual_text_is_searched_safely() {
        let c = synthetic(vec![entry("物件 › 群組", "a.one"), entry("مرحبا عالم", "b.two"), entry("Cafe\u{301} Tool", "c.three")]);
        assert_eq!(ranked(&c, "群組"), ["a.one"]);
        assert_eq!(ranked(&c, "عالم"), ["b.two"]);
        assert_eq!(ranked(&c, "cafe\u{301}"), ["c.three"]);
        assert!(ranked(&c, &"x".repeat(5000)).is_empty());
        assert!(ranked(&c, &"群".repeat(5000)).is_empty());
        assert_eq!(ranked(&c, "›"), ["a.one"]);
        assert!(ranked(&c, "…,.-").is_empty());
        let en = catalog(Lang::EN, false);
        let both = ranked(&en, "fit headline");
        assert!(both.contains(&"type.fitHeadline".to_string()) && both.contains(&"text.fitHeadline".to_string()), "{both:?}");
        for id in ["type.fitHeadline", "text.fitHeadline"] {
            assert_eq!(ranked(&en, &format!("Type › Fit Headline ({id})")).first().map(String::as_str), Some(id));
        }
        let zh = Lang::from_code("zh-hant").unwrap();
        let zh_catalog = catalog(zh, false);
        let group = crate::i18n::tr(zh, "Group");
        assert!(place(&zh_catalog, group, "object.group").is_some_and(|p| p < 3), "{group}");
    }

    /// Each result's home is the first item of the menu bar that runs it: an Effect menu item's
    /// submenu, the Window menu for a panel whose item shows and hides it, Learn's own item, and
    /// with the native Mac menu bar Preferences under the application menu's Settings.
    #[test]
    fn results_know_their_place_in_the_menu_bar() {
        let homes = menu_homes(false);
        let home = |id: &str| homes.get(id).map(|(path, _)| path.as_str());
        assert_eq!(home("effect:stylize.roundCorners"), Some("Effect › Stylize"));
        assert_eq!(home("panel:layers"), Some("Window"), "matched by the panel, although the item also asks to toggle it");
        assert_eq!(home("panel:learn"), Some("Window"));
        assert_eq!(home("panel:character"), Some("Window › Type"));
        assert_eq!(home("edit.preferences"), Some("Edit"));
        assert_eq!(home("help.about"), Some("Help"), "the in-window bar shows no application menu");
        assert_eq!(home("ui.layersPanelOptions"), None);
        assert_eq!(homes.get("app.quit"), Some(&("File".to_string(), "Exit")), "with its item's own label");
        let native = menu_homes(true);
        let native_home = |id: &str| native.get(id).map(|(path, _)| path.as_str());
        assert_eq!(native_home("edit.preferences"), Some("VectorCraft › Settings"));
        assert_eq!(native_home("help.about"), Some("VectorCraft"));
        assert_eq!(native_home("help.discord"), Some("Help"));
        let c = catalog(Lang::EN, false);
        let of = |id: &str| c.list.iter().find(|e| e.id == id).and_then(|e| e.home.clone());
        assert_eq!(of("effect:stylize.roundCorners").as_deref(), Some("Effect › Stylize"));
        assert_eq!(of("tool:pen"), None);
        // The home's words find the result: Preferences… under the native bar's Settings.
        let native_catalog = catalog(Lang::EN, true);
        assert!(place(&native_catalog, "settings pref", "edit.preferences").is_some());
        assert!(place(&c, "settings pref", "edit.preferences").is_none());
        // The menu item's own label finds the result too: File › Exit runs Quit VectorCraft.
        assert!(place(&c, "exit", "app.quit").is_some());
    }

    /// The index of the real results takes at most 512 KiB, homes included.
    #[test]
    fn the_index_stays_small() {
        let c = catalog(Lang::EN, false);
        let homes: usize = c.list.iter().filter_map(|e| e.home.as_ref()).map(String::capacity).sum();
        let bytes = c.index.bytes() + homes;
        println!("{} results, index {} bytes, homes {homes} bytes", c.list.len(), c.index.bytes());
        assert!(bytes <= 512 * 1024, "{bytes}");
    }

    #[test]
    #[ignore = "times the index and sample queries; run in a release build"]
    fn time_the_index_and_queries() {
        let started = std::time::Instant::now();
        let c = catalog(Lang::EN, false);
        let built = started.elapsed();
        let started = std::time::Instant::now();
        let index = Index::build(&c.list, &[]);
        println!("catalog {built:?}, index alone {:?}, {} results, {} bytes", started.elapsed(), c.list.len(), index.bytes());
        let long = "q".repeat(200);
        for q in ["e", "ea", "select", "bring front", "zzzz", long.as_str()] {
            let started = std::time::Instant::now();
            let runs = 100;
            for _ in 0..runs {
                std::hint::black_box(rank(&c.list, &c.index, q));
            }
            println!("{:>12}: {:?} per query", q.get(..12).unwrap_or(q), started.elapsed() / runs);
        }
        let started = std::time::Instant::now();
        for _ in 0..100 {
            std::hint::black_box(substring_rule(&c, "bring front"));
        }
        println!("substring rule alone: {:?} per query", started.elapsed() / 100);
    }
}
