//! The command palette's search: a small index of the words in each result's label, menu path and
//! id, the menu bar's home of each result, and the ranking of the results a query matches.

use std::collections::HashMap;

use super::{Entry, WRAPPED};
use crate::menus;

/// The most characters of a query the index reads (VectorCraft's own limit). The substring rule
/// and the exact tiers read the whole query.
const INDEX_CHARS: usize = 200;

/// The field of a result a word comes from, best first, and the fields of the English stem forms
/// ([`stems`]) of the words of the label, the menu path and the description ([`description`]).
const LABEL: u8 = 0;
const PATH: u8 = 1;
const ID: u8 = 2;
const LABEL_STEMMED: u8 = 3;
const PATH_STEMMED: u8 = 4;
const DESCRIPTION: u8 = 5;
const DESCRIPTION_STEMMED: u8 = 6;

/// A word's grade in a result, best first: a whole word of the label, a word of the label it
/// shares an English stem form with ("outlines" and Outline), a word of the label it starts, a
/// word of the label with the same meaning ([`super::words`]), a word of the menu path it is,
/// shares a stem form with or starts, a word of the menu path with the same meaning, a word of the
/// id it is or starts, text inside a word of any field ([`super::matching`]'s substring rule), a
/// word of the description it is, shares a stem form with, starts or means the same as, a word of
/// the label a typo away from it ([`Index::typos`]), or a word of the menu path a typo away. A word
/// of the description comes after the substring rule, so every result the substring rule finds
/// ranks above a result found only by its description. The grades without a stem or a word with
/// the same meaning are twice the values they had before those were added, so results matched
/// without them keep their order by the sum of the grades.
const LABEL_WORD: u8 = 2;
const LABEL_STEM: u8 = 3;
const LABEL_PREFIX: u8 = 4;
const LABEL_SYNONYM: u8 = 5;
const PATH_WORD: u8 = 6;
const PATH_SYNONYM: u8 = 7;
const ID_WORD: u8 = 8;
const SUBSTRING: u8 = 10;
const DESCRIPTION_WORD: u8 = 11;
const LABEL_TYPO: u8 = 12;
const PATH_TYPO: u8 = 14;
const NONE: u8 = u8::MAX;

/// The most characters of a command's description the index reads (VectorCraft's own limit).
const DESCRIPTION_CHARS: usize = 300;

/// The words of a description the index leaves out, besides words of 2 characters or fewer and
/// words with no letter.
const STOP_WORDS: &[&str] = &["the", "and", "ids", "true", "false", "null"];

/// The most words a query word with a typo meets ([`Index::typos`]), nearest first.
const TYPO_TERMS: usize = 32;

/// How a whole query matches a result, best first: it is the result's id, it is a label, a label
/// starts with it, or it is the initials of a label.
const EXACT_ID: u8 = 0;
const EXACT_LABEL: u8 = 1;
const PREFIX: u8 = 2;
const INITIALS: u8 = 3;
const NO_PHRASE: u8 = 4;

/// One result in the list of a term: the result's place in the catalog and the field the term is
/// a word or a word's stem form of.
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
    /// Each word of a group of words with the same meaning ([`super::words::GROUPS`], the words the
    /// UI language's catalog adds, and the English words' stem forms), with the group's place in the
    /// table.
    synonyms: HashMap<Box<str>, u16>,
}

/// Is `w` one word as the index splits text ([`words`])?
pub(crate) fn is_one_word(w: &str) -> bool {
    let mut it = words(w);
    it.next() == Some(w) && it.next().is_none()
}

/// The words of each group of [`super::words::GROUPS`], with the words `search_row` gives for the
/// group's catalog row ([`super::words::row_key`]) in the UI language, and the English stem forms
/// ([`stems`]) of the group's English words (`setting` of settings). A word the catalog gives that
/// is not a single word, or that is already a word of a group, is left out, and so is a stem form
/// that is already a word of a group.
fn synonym_groups(search_row: impl Fn(&str) -> Option<String>) -> HashMap<Box<str>, u16> {
    let mut out: HashMap<Box<str>, u16> = HashMap::new();
    let groups = super::words::GROUPS.iter().enumerate().filter_map(|(g, words)| u16::try_from(g).ok().map(|g| (g, words)));
    for (g, words) in groups.clone() {
        for &w in *words {
            out.insert(w.into(), g);
        }
    }
    for (g, words) in groups.clone() {
        let Some(text) = search_row(&super::words::row_key(words)) else { continue };
        for (w, single) in super::words::row_words(&text) {
            if single {
                out.entry(w.into()).or_insert(g);
            }
        }
    }
    for (g, words) in groups {
        for form in words.iter().flat_map(|w| stems(w)) {
            out.entry(form.into()).or_insert(g);
        }
    }
    out.shrink_to_fit();
    out
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

/// The English stem forms of the lowercased `word`, at most two, and none for a word that is not
/// plain English letters. `'s` is dropped; `sses` becomes `ss`; `es` after `s`, `x`, `z`, `ch` or
/// `sh` is dropped, and with only its `s` dropped the word is a second form (`boxes`, `box` and
/// `boxe`; `rasterizes`, `rasteriz` and `rasterize`); in a word of more than 5 letters, `ies`
/// becomes `y` or `ie` (`copies`, `copy` and `copie`; `movies`, `movy` and `movie`); `ing` (more
/// than 5 letters) and `ed` (more than 4) are dropped, and when a doubled `b`, `d`, `g`, `m`, `n`,
/// `p`, `r` or `t` is left before them, the rest with that letter made single and the rest as it
/// is are the forms (`cutting`, `cut` and `cutt`), the single one only when it keeps at least 3
/// letters (`added`, `add` alone); otherwise the rest and the rest followed by `e` are the forms
/// (`scaling`, `scal` and `scale`); a final `s` is dropped from a word of more than 3 letters that
/// does not end in `ss`, `us` or `is`. `series` and `species` have none.
fn stems(word: &str) -> Vec<String> {
    let one = |rest: &str| if rest.is_empty() { vec![] } else { vec![rest.to_string()] };
    if !word.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'') || matches!(word, "series" | "species") {
        return vec![];
    }
    let n = word.len();
    if let Some(rest) = word.strip_suffix("'s") {
        return one(rest);
    }
    if let Some(rest) = word.strip_suffix("sses") {
        return vec![format!("{rest}ss")];
    }
    if let Some(rest) = word.strip_suffix("es")
        && ["s", "x", "z", "ch", "sh"].iter().any(|end| rest.ends_with(end))
    {
        return vec![rest.to_string(), format!("{rest}e")];
    }
    if n > 5
        && let Some(rest) = word.strip_suffix("ies")
    {
        return vec![format!("{rest}y"), format!("{rest}ie")];
    }
    let verb = if n > 5 { word.strip_suffix("ing") } else { None }.or_else(|| if n > 4 { word.strip_suffix("ed") } else { None });
    if let Some(rest) = verb {
        if let [.., a, b] = rest.as_bytes()
            && a == b
            && b"bdgmnprt".contains(a)
        {
            let single = rest.get(..rest.len() - 1).unwrap_or_default();
            if single.len() < 3 {
                return vec![rest.to_string()];
            }
            return vec![single.to_string(), rest.to_string()];
        }
        return vec![rest.to_string(), format!("{rest}e")];
    }
    if n > 3 && word.ends_with('s') && !["ss", "us", "is"].iter().any(|end| word.ends_with(end)) {
        return one(word.get(..n - 1).unwrap_or_default());
    }
    vec![]
}

/// Is `c` a letter of a script written without spaces between words (Han, kana or Hangul)?
fn spaceless(c: char) -> bool {
    matches!(
        u32::from(c),
        0x1100..=0x11FF
            | 0x3040..=0x30FF
            | 0x3130..=0x318F
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7FF
            | 0xF900..=0xFAFF
            | 0xFF66..=0xFFDC
            | 0x20000..=0x323AF
    )
}

/// The most edits a query word may be away from a word it meets as a typo: 1 for a word of 4 to 7
/// characters, 2 for a longer one, and none for a shorter word or one with letters of a script
/// written without spaces ([`spaceless`]).
fn typo_bound(word: &str) -> Option<usize> {
    if word.chars().any(spaceless) {
        return None;
    }
    match word.chars().count() {
        0..=3 => None,
        4..=7 => Some(1),
        _ => Some(2),
    }
}

/// The number of edits between `a` and `b` when it is at most `bound`: a character inserted,
/// removed or changed, or two adjacent characters swapped (`gruop` and `group`), each count as one,
/// and each character inserted between two swapped characters or removed from between them counts
/// as one more (`rectgale` and `rectangle` are two edits apart). The count stops as soon as every
/// way of lining the two up needs more than `bound`.
fn edits_within(a: &[char], b: &[char], bound: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > bound {
        return None;
    }
    let width = b.len() + 1;
    // The edits between the first i characters of `a` and the first j of `b`, at
    // (i % kept) * width + j, for the last `kept` rows: a swap within the bound reaches back at most
    // `bound` + 1 rows.
    let kept = bound.saturating_add(2);
    let mut table: Vec<usize> = (0..width).collect();
    table.resize(kept.min(a.len() + 1) * width, 0);
    let place = |i: usize, j: usize| (i % kept) * width + j;
    let at = |table: &[usize], i: usize, j: usize| table.get(place(i, j)).copied().unwrap_or(usize::MAX);
    for (i, &ca) in (1usize..).zip(a) {
        if let Some(first) = table.get_mut(place(i, 0)) {
            *first = i;
        }
        // The last column so far in this row whose character of `b` is `ca`.
        let mut column = 0;
        for (j, &cb) in (1usize..).zip(b) {
            let changed = at(&table, i - 1, j - 1).saturating_add(usize::from(ca != cb));
            let removed = at(&table, i - 1, j).saturating_add(1);
            let inserted = at(&table, i, j - 1).saturating_add(1);
            let mut edits = changed.min(removed).min(inserted);
            // `cb` is in row k of `a` and `ca` in column `column` of `b`: swap the two, remove the
            // characters of `a` between them and insert the characters of `b` between them. Removing
            // more than `bound` characters goes past the bound, so only the `bound` rows before this
            // one are searched, newest first.
            let k = (i.saturating_sub(bound).max(1)..i).rev().find(|&k| a.get(k - 1) == Some(&cb)).unwrap_or(0);
            if k > 0 && column > 0 {
                edits = edits.min(at(&table, k - 1, column - 1).saturating_add(i - k + (j - column - 1)));
            }
            if ca == cb {
                column = j;
            }
            if let Some(cell) = table.get_mut(place(i, j)) {
                *cell = edits;
            }
        }
        if (0..width).all(|j| at(&table, i, j) > bound) {
            return None;
        }
    }
    Some(at(&table, a.len(), b.len())).filter(|&e| e <= bound)
}

/// Adds each word of `text` to `found` for result `entry` in `field`, and with `stemmed`, each
/// word's English stem forms in that field.
fn add_words(found: &mut Vec<(String, u16, u8)>, text: &str, entry: u16, field: u8, stemmed: Option<u8>) {
    for w in words(text) {
        found.push((w.to_string(), entry, field));
        if let Some(stemmed) = stemmed {
            found.extend(stems(w).into_iter().map(|s| (s, entry, stemmed)));
        }
    }
}

/// A name's last `›`-separated part (the label) and the parts before it (the menu path).
fn split_name(name: &str) -> (&str, &str) {
    match name.rsplit_once(" › ") {
        Some((path, label)) => (label, path),
        None => (name, ""),
    }
}

/// A command's description, read from its params text: the text without each `{…}` group that is
/// not inside a `{`, `[` or `(` group (brackets inside double quotes or backticks are text, as in
/// the engine's params object), without each such
/// `[…]` or `(…)` group just after a `→`, and without the `→` just before a removed group, with
/// each run of whitespace made one space and cut to its first [`DESCRIPTION_CHARS`] characters at
/// the end of a word. A group that is never closed stays in the text, and so does a result written
/// as words after a `→`. Params text that is only a group has an empty description.
pub(crate) fn description(params: &str) -> String {
    let mut out = String::new();
    // The end of the text copied to `out`, and whether a removed group is open.
    let (mut copied, mut open) = (0, false);
    for (i, ch, depth) in crate::shortcut_editor::outside_quotes_and_backticks(params) {
        let before = params.get(copied..i).unwrap_or_default();
        match ch {
            '{' | '[' | '(' if depth == 0 && (ch == '{' || before.trim_end().ends_with('→')) => {
                out.push_str(before);
                (copied, open) = (i, true);
            }
            '}' | ']' | ')' if depth == 1 && open => {
                let kept = out.trim_end().strip_suffix('→').unwrap_or(&out).len();
                out.truncate(kept);
                out.push(' ');
                (copied, open) = (i + ch.len_utf8(), false);
            }
            _ => {}
        }
    }
    out.push_str(params.get(copied..).unwrap_or_default());
    let text = out.split_whitespace().collect::<Vec<_>>().join(" ");
    // A cut inside a word drops the rest of that word, so no part of a word is read as a word.
    let Some((head, rest)) = text.char_indices().nth(DESCRIPTION_CHARS).and_then(|(end, _)| text.split_at_checked(end)) else { return text };
    let head = if rest.starts_with(' ') { head } else { head.rsplit_once(' ').map_or("", |(kept, _)| kept) };
    head.trim_end().to_string()
}

/// The lowercase words of the lowercased `description` the index holds: split at every character
/// that is not a letter, a digit or an apostrophe, without apostrophes at either end, and leaving
/// out words of 2 characters or fewer, words with no letter and [`STOP_WORDS`].
fn description_words(description: &str) -> impl Iterator<Item = &str> {
    description
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .map(|w| w.trim_matches('\''))
        .filter(|w| w.chars().count() > 2 && w.chars().any(char::is_alphabetic) && !STOP_WORDS.contains(w))
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
    /// words of a result's menu home and of its menu item's label count as menu path words. The
    /// words of the English names, menu home and menu item label also add their English stem forms
    /// ([`stems`]); words in the UI language do not. `params` gives each result's params text, whose
    /// [`description`] adds its words ([`description_words`]) and their stem forms; only the words
    /// are kept. `search_row` gives the UI language's words for a group of words with the same
    /// meaning ([`synonym_groups`]). A catalog with more results than a posting can name gets an
    /// empty index, and queries match by the substring rule alone.
    pub(crate) fn build(entries: &[Entry], homes: &[Option<String>], params: &[&str], search_row: impl Fn(&str) -> Option<String>) -> Index {
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
            // The first two names are English, so their words also give stem forms; the shown name
            // may be in another language, and its words are indexed as they are.
            for (n, name) in names(e).enumerate() {
                let english = n < 2;
                let (label, path) = split_name(without_id(name, id));
                add_words(&mut found, label, entry, LABEL, english.then_some(LABEL_STEMMED));
                add_words(&mut found, path, entry, PATH, english.then_some(PATH_STEMMED));
                let ini = initials_of(label);
                if ini.chars().count() >= 2 && !seen_initials.contains(&ini) {
                    if !seen_initials.is_empty() {
                        initials_text.push('\n');
                    }
                    initials_text.push_str(&ini);
                    seen_initials.push(ini);
                }
            }
            for home in [e.home.as_deref(), Some(e.menu_item)].into_iter().flatten() {
                add_words(&mut found, &home.to_lowercase(), entry, PATH, Some(PATH_STEMMED));
            }
            if let Some(home) = homes.get(i).and_then(Option::as_deref) {
                add_words(&mut found, &home.to_lowercase(), entry, PATH, None);
            }
            for id in ids(e) {
                found.extend(words(id).map(|w| (w.to_string(), entry, ID)));
            }
            if let Some(text) = params.get(i).filter(|p| !p.is_empty()) {
                for w in description_words(&description(text).to_lowercase()) {
                    found.push((w.to_string(), entry, DESCRIPTION));
                    found.extend(stems(w).into_iter().map(|s| (s, entry, DESCRIPTION_STEMMED)));
                }
            }
            let len = u16::try_from(initials_text.len() - start).unwrap_or(0);
            initials.push((u32::try_from(start).unwrap_or(0), len));
        }
        found.sort_unstable();
        found.dedup();
        let mut index = Index { initials_text, initials, synonyms: synonym_groups(search_row), ..Index::default() };
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
            + self.synonyms.capacity() * (std::mem::size_of::<(Box<str>, u16)>() + 1)
            + self.synonyms.keys().map(|w| w.len()).sum::<usize>()
    }

    /// The text of the word at `(offset, len)` in `term_text`.
    fn text(&self, &(offset, len): &(u32, u32)) -> &str {
        let offset = offset as usize;
        self.term_text.get(offset..offset + len as usize).unwrap_or_default()
    }

    /// The postings of the word at place `t` in `terms`.
    fn postings_of(&self, t: usize) -> &[Posting] {
        let Some(&(begin, len)) = self.spans.get(t) else { return &[] };
        let (begin, len) = (begin as usize, len as usize);
        self.postings.get(begin..begin + len).unwrap_or_default()
    }

    /// The place of the word `text` in `terms`, found by a binary search.
    fn find(&self, text: &str) -> Option<usize> {
        self.terms.binary_search_by(|t| self.text(t).cmp(text)).ok()
    }

    /// The best grade ([`LABEL_WORD`] to [`DESCRIPTION_WORD`], without [`SUBSTRING`]) `word` gets in
    /// each result, [`NONE`] where it is no word, shares no English stem form with a word of the
    /// label, menu path or description, means the same as none of them, and starts no word of the
    /// result. The words it starts are found by a binary search of the sorted words; a stem form
    /// matches only a word or stem form equal to it.
    fn grades(&self, word: &str, n: usize) -> Vec<u8> {
        let mut out = vec![NONE; n];
        let mut mark = |postings: &[Posting], grade: &dyn Fn(u8) -> Option<u8>| {
            for p in postings {
                if let Some(grade) = grade(p.field)
                    && let Some(g) = out.get_mut(usize::from(p.entry))
                {
                    *g = (*g).min(grade);
                }
            }
        };
        let start = self.terms.partition_point(|t| self.text(t) < word);
        for (t, at) in self.terms.iter().enumerate().skip(start) {
            let term = self.text(at);
            if !term.starts_with(word) {
                break;
            }
            let whole = term == word;
            // A stem form is not a word of the result, so a query word that only starts it does
            // not match.
            mark(self.postings_of(t), &|field| match field {
                LABEL if whole => Some(LABEL_WORD),
                LABEL => Some(LABEL_PREFIX),
                PATH => Some(PATH_WORD),
                DESCRIPTION => Some(DESCRIPTION_WORD),
                ID => Some(ID_WORD),
                _ => None,
            });
        }
        // The word meets a stem form of a word of the result ("scale" and Scaling) when it is itself
        // a word of some result's label, menu path, id or description, so a partly typed word
        // ("scal") that only equals a stem form does not lift Scaling above Scale. A description is
        // cut at the end of a word ([`description`]), so it holds no partly typed word. A word that
        // only descriptions hold meets the stem forms of descriptions only, so it does not change
        // how labels and menu paths rank.
        if let Some(t) = self.find(word) {
            let postings = self.postings_of(t);
            let named = postings.iter().any(|p| matches!(p.field, LABEL | PATH | ID));
            let described = named || postings.iter().any(|p| p.field == DESCRIPTION);
            mark(postings, &|field| match field {
                LABEL_STEMMED if named => Some(LABEL_STEM),
                PATH_STEMMED if named => Some(PATH_WORD),
                DESCRIPTION_STEMMED if described => Some(DESCRIPTION_WORD),
                _ => None,
            });
        }
        // A stem form of the word meets a word of the result or a stem form of one ("outlines"
        // and Outline, "rotated" and Rotating). Ids are not stemmed.
        for form in stems(word) {
            let Some(t) = self.find(&form) else { continue };
            mark(self.postings_of(t), &|field| match field {
                LABEL | LABEL_STEMMED => Some(LABEL_STEM),
                PATH | PATH_STEMMED => Some(PATH_WORD),
                DESCRIPTION | DESCRIPTION_STEMMED => Some(DESCRIPTION_WORD),
                _ => None,
            });
        }
        // Each other word of the word's group of words with the same meaning ([`Index::synonyms_of`])
        // meets a word of the label, menu path or description, or a stem form of one, equal to it.
        // Ids are not matched this way.
        for t in self.synonyms_of(word) {
            mark(self.postings_of(t), &|field| match field {
                LABEL | LABEL_STEMMED => Some(LABEL_SYNONYM),
                PATH | PATH_STEMMED => Some(PATH_SYNONYM),
                DESCRIPTION | DESCRIPTION_STEMMED => Some(DESCRIPTION_WORD),
                _ => None,
            });
        }
        out
    }

    /// The other words of the group of words with the same meaning that `word`, or a stem form of
    /// it, is in ("bg" and background, "flipped" and reflect, "setting" and preferences), with the
    /// stem forms of the group's words ("prefs" and setting), as places in `terms`; the words of the
    /// group no result holds are left out.
    fn synonyms_of(&self, word: &str) -> Vec<usize> {
        let group = std::iter::once(word.to_string()).chain(stems(word)).find_map(|w| self.synonyms.get(w.as_str()).copied());
        let Some(group) = group else { return vec![] };
        let mut found: Vec<usize> = self.synonyms.iter().filter(|&(w, &g)| g == group && &**w != word).filter_map(|(w, _)| self.find(w)).collect();
        // The map's order changes from one build to the next; the words' order in `terms` does not.
        found.sort_unstable();
        found
    }

    /// The words of labels and menu paths that `word` is a typo of: within [`typo_bound`] edits of
    /// it ([`edits_within`]), at most [`TYPO_TERMS`], nearest first, as places in `terms`. Ids and
    /// words with letters of a script written without spaces ([`spaceless`]) are not compared.
    fn typos(&self, word: &str) -> Vec<usize> {
        let Some(bound) = typo_bound(word) else { return vec![] };
        let word: Vec<char> = word.chars().collect();
        let mut term: Vec<char> = vec![];
        let mut near: Vec<(usize, usize)> = vec![];
        for (t, at) in self.terms.iter().enumerate() {
            let text = self.text(at);
            if text.chars().count().abs_diff(word.len()) > bound || text.chars().any(spaceless) {
                continue;
            }
            term.clear();
            term.extend(text.chars());
            if let Some(edits) = edits_within(&word, &term, bound)
                && self.postings_of(t).iter().any(|p| matches!(p.field, LABEL | PATH))
            {
                near.push((edits, t));
            }
        }
        near.sort_unstable();
        near.truncate(TYPO_TERMS);
        near.into_iter().map(|(_, t)| t).collect()
    }

    /// Gives each result a word of `near` ([`Index::typos`]) is a word of its label or menu path
    /// [`LABEL_TYPO`] or [`PATH_TYPO`] in `grades`, where its grade is not better. A typo replaces
    /// [`DESCRIPTION_WORD`], since a result found only by its description ranks below one found
    /// through a typo ([`rank`]).
    fn mark_typos(&self, near: &[usize], grades: &mut [u8]) {
        for &t in near {
            for p in self.postings_of(t) {
                let grade = match p.field {
                    LABEL => LABEL_TYPO,
                    PATH => PATH_TYPO,
                    _ => continue,
                };
                if let Some(g) = grades.get_mut(usize::from(p.entry)) {
                    *g = if *g == DESCRIPTION_WORD { grade } else { (*g).min(grade) };
                }
            }
        }
    }

    /// The text of the first word of `near` that is a word of result `i` in one of `fields` (its
    /// label or its menu path, and their stem forms).
    fn nearest_of(&self, near: &[usize], i: usize, fields: &[u8]) -> Option<&str> {
        let t = near.iter().find(|&&t| self.postings_of(t).iter().any(|p| usize::from(p.entry) == i && fields.contains(&p.field)))?;
        self.terms.get(*t).map(|at| self.text(at))
    }

    /// The texts of the words of `near` that are words of result `i` in one of `fields`, in the
    /// order of `near`.
    fn words_of(&self, near: &[usize], i: usize, fields: &[u8]) -> Vec<&str> {
        near.iter()
            .filter(|&&t| self.postings_of(t).iter().any(|p| usize::from(p.entry) == i && fields.contains(&p.field)))
            .filter_map(|&t| self.terms.get(t).map(|at| self.text(at)))
            .collect()
    }

    /// Is `query` (no spaces) the initials of one of result `i`'s labels?
    fn initials_are(&self, i: usize, query: &str) -> bool {
        let Some(&(offset, len)) = self.initials.get(i) else { return false };
        let (offset, len) = (offset as usize, usize::from(len));
        self.initials_text.get(offset..offset + len).is_some_and(|text| text.lines().any(|l| l == query))
    }

    /// Does the index hold the `n` results it was built from? A catalog too large to index gets an
    /// empty index ([`Index::build`]), which holds none.
    fn holds(&self, n: usize) -> bool {
        self.initials.len() == n
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

/// What results are ordered by, compared field by field: whether a word matched only through a
/// description, whether a word matched only through a typo, how the whole query matches, the worst
/// grade a word gets, the sum of the grades, how the whole query matches when read with the words
/// of the same meaning it matched through, the result's place among the recent picks, the kind of
/// result and the label's length.
type RankKey = (bool, bool, u8, u8, u32, u8, usize, u8, usize);

/// The results of `entries` that the lowercased `query` matches, best first, as places in
/// `entries`. A result matches when every word of the query is text in one of its names or ids
/// (the rule the palette always had, so nothing it found is lost), when every word of the query is
/// or starts a word of its label, menu path, description ([`description`]) or id, shares an English
/// stem form with a word of its label, menu path or description ("rounded corners" for Round
/// Corners…) or means the same as a word of its label, menu path or description ("remove bg" for
/// Remove Background…), where a word may also be text in one of its names or ids ("bg mov" for
/// Remove Background…), or when the query is the initials of a label ("ea" for Export As…). A query
/// word that is no word, stem form or word start of any result's label, menu path or id also
/// matches the words of labels and menu paths it is a typo of ([`Index::typos`]); with such a word,
/// a result also matches when each other word of the query is text in one of its names or ids
/// ("rop shdaow" for Drop Shadow).
/// Results where a word matched only through a description come after every other result. Before
/// them, results where a word matched only through a typo come after every other result, and their
/// whole-query match is read with each such word replaced by the word it is a typo of ("gruop" as
/// "group"). Results are ordered by that, by how the whole query matches ([`phrase`]), then by the
/// worst grade a word gets, the sum of the grades, how the whole query matches when read with each
/// word matched through a word of the same meaning replaced by that word ("colour" as "color"; of
/// several such words the result holds, the ones that match the whole query best),
/// then by the result's place in `recent`, the places in `entries` of the recent picks, most
/// recent first (a result not in it comes after every result in it), then the kind of result
/// ([`kind`]), the label's length and the catalog order. So a recent pick ranks above the results
/// that match the query as well as it does and below every result that matches it better. An
/// empty query lists every result in catalog order.
pub(crate) fn rank(entries: &[Entry], index: &Index, query: &str, recent: &[u32]) -> Vec<u32> {
    let query = normalize(query);
    if query.is_empty() {
        return (0..entries.len()).filter_map(|i| u32::try_from(i).ok()).collect();
    }
    let spaced: Vec<&str> = query.split(' ').collect();
    let head: String = query.chars().take(INDEX_CHARS).collect();
    let tokens: Vec<&str> = words(&head).collect();
    // Where each word starts in the query (`head` is its start), for reading the query with a typo
    // corrected.
    let starts: Vec<usize> = tokens.iter().map(|t| (t.as_ptr() as usize).saturating_sub(head.as_ptr() as usize)).collect();
    let n = entries.len();
    let mut indexed: Vec<Vec<u8>> = Vec::with_capacity(tokens.len());
    let mut typos: Vec<Vec<usize>> = Vec::with_capacity(tokens.len());
    let mut meanings: Vec<Vec<usize>> = Vec::with_capacity(tokens.len());
    // After a word that is nothing in any result, text in no result's names or ids and a typo of
    // nothing, a result is found only by its initials, where a typo grade changes nothing, so the
    // later words are not compared.
    let mut compare = true;
    for (k, t) in tokens.iter().enumerate() {
        // A repeated word is graded once.
        if let Some(j) = tokens.iter().take(k).position(|u| u == t)
            && let (Some(grades), Some(near), Some(same)) = (indexed.get(j).cloned(), typos.get(j).cloned(), meanings.get(j).cloned())
        {
            indexed.push(grades);
            typos.push(near);
            meanings.push(same);
            continue;
        }
        meanings.push(index.synonyms_of(t));
        let mut grades = index.grades(t, n);
        // A word that matches only words of descriptions, or words with the same meaning, is still
        // compared with the words of labels and menu paths it may be a typo of.
        let near = if compare && grades.iter().all(|&g| matches!(g, NONE | DESCRIPTION_WORD | LABEL_SYNONYM | PATH_SYNONYM)) {
            index.typos(t)
        } else {
            vec![]
        };
        if near.is_empty() && grades.iter().all(|&g| g == NONE) && !entries.iter().any(|e| e.haystack.contains(t)) {
            compare = false;
        }
        index.mark_typos(&near, &mut grades);
        indexed.push(grades);
        typos.push(near);
    }
    // The index reads only the start of a longer query, so such a query matches by the substring
    // rule, which reads all of it, as every query does in a catalog too large to index.
    let fits = query.chars().count() <= INDEX_CHARS && index.holds(n);
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
        // Whether each word is in this result's index or is text in its names or ids, for a result
        // a word matches only through a typo.
        let mut held = all_indexed;
        // The query as read for the whole-query match, with each word matched only through a typo
        // replaced by the word of this result it is a typo of; and as read with each word matched
        // through a word with the same meaning replaced by that word too.
        let (mut typo, mut corrected, mut copied) = (false, String::new(), 0);
        let mut described = false;
        // The words each such word may be read as, by its place in the query.
        let (mut synonym, mut meant): (bool, Vec<(usize, Vec<&str>)>) = (false, vec![]);
        let splice = |out: &mut String, copied: &mut usize, k: usize, token: &str, word: &str| {
            if let Some(&start) = starts.get(k)
                && let Some(between) = query.get(*copied..start)
            {
                out.push_str(between);
                out.push_str(word);
                *copied = start + token.len();
            }
        };
        for (k, (grades, token)) in indexed.iter().zip(&tokens).enumerate() {
            let mut g = grades.get(i).copied().unwrap_or(NONE);
            if g == NONE {
                held &= e.haystack.contains(token);
            }
            if g > SUBSTRING && e.haystack.contains(token) {
                g = SUBSTRING;
            }
            // A word that is text inside a word of the result counts as matched, so it combines
            // with words matched through a stem form, a word of the same meaning, a typo or the
            // description.
            if g == NONE {
                all_indexed = false;
            }
            let g = if initials {
                g.min(LABEL_PREFIX)
            } else if g == NONE {
                SUBSTRING
            } else {
                g
            };
            described |= g == DESCRIPTION_WORD;
            let field = match g {
                LABEL_TYPO => Some(LABEL),
                PATH_TYPO => Some(PATH),
                _ => None,
            };
            if let Some(field) = field {
                typo = true;
                if let Some(near) = typos.get(k).and_then(|near| index.nearest_of(near, i, &[field])) {
                    splice(&mut corrected, &mut copied, k, token, near);
                    meant.push((k, vec![near]));
                }
            }
            // The word may be read as each word of its group the label or menu path holds, and then
            // as each stem form of one, so "prefs" reads Preferences as preferences before its stem
            // form preference; the reading that matches the whole query best counts.
            let fields: &[u8] = match g {
                LABEL_SYNONYM => &[LABEL, LABEL_STEMMED],
                PATH_SYNONYM => &[PATH, PATH_STEMMED],
                _ => &[],
            };
            if !fields.is_empty() {
                synonym = true;
                let mut same: Vec<&str> = vec![];
                for &f in fields {
                    for w in meanings.get(k).map(|near| index.words_of(near, i, &[f])).unwrap_or_default() {
                        if !same.contains(&w) {
                            same.push(w);
                        }
                    }
                }
                if !same.is_empty() {
                    meant.push((k, same));
                }
            }
            worst = worst.max(g);
            sum += u32::from(g);
        }
        if !(substring || all_indexed || initials || (typo && held)) {
            continue;
        }
        let tier = if typo {
            corrected.push_str(query.get(copied..).unwrap_or_default());
            phrase(e, &corrected, false)
        } else {
            phrase(e, &query, initials)
        };
        let meant_tier = if synonym { best_reading(&query, &starts, &tokens, &meant, |read| phrase(e, read, false)) } else { tier };
        let length = split_name(&e.shown).0.chars().count();
        let picked = recent.iter().position(|&r| r == place).unwrap_or(recent.len());
        ranked.push(((described, typo, tier, worst, sum, meant_tier, picked, e.kind, length), place));
    }
    ranked.sort_by_key(|&(key, place)| (key, place));
    ranked.into_iter().map(|(_, place)| place).collect()
}

/// The most readings of a query [`best_reading`] compares for one result.
const MAX_READINGS: usize = 16;

/// The best whole-query match `tier` gives `query` read with each word `k` of `swaps` (the word at
/// `starts[k]`, as long as `tokens[k]`) replaced by one of its words. At most [`MAX_READINGS`]
/// readings are compared, the first one with each word's first choice.
fn best_reading(query: &str, starts: &[usize], tokens: &[&str], swaps: &[(usize, Vec<&str>)], tier: impl Fn(&str) -> u8) -> u8 {
    let readings = swaps.iter().fold(1usize, |n, (_, words)| n.saturating_mul(words.len().max(1))).min(MAX_READINGS);
    let (mut best, mut read) = (NO_PHRASE, String::new());
    for r in 0..readings {
        read.clear();
        let (mut copied, mut rest) = (0, r);
        for (k, words) in swaps {
            let len = words.len().max(1);
            let pick = words.get(rest % len);
            rest /= len;
            if let (Some(word), Some(&start), Some(token)) = (pick, starts.get(*k), tokens.get(*k))
                && let Some(between) = query.get(copied..start)
            {
                read.push_str(between);
                read.push_str(word);
                copied = start + token.len();
            }
        }
        read.push_str(query.get(copied..).unwrap_or_default());
        best = best.min(tier(&read));
        if best == EXACT_ID {
            break;
        }
    }
    best
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
                    // Window › Learn runs `window.learn`, which is a result of its own too.
                    if *id == "window.learn" {
                        out.entry((*id).to_string()).or_insert_with(|| (path.join(" › "), *label));
                    }
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
        recently(c, query, &[])
    }

    /// The ids of the results `query` lists, best first, with the results of ids `picked` as the
    /// recent picks, most recent first.
    fn recently(c: &Catalog, query: &str, picked: &[&str]) -> Vec<String> {
        let recent: Vec<u32> = picked.iter().filter_map(|id| c.list.iter().position(|e| e.id == *id)).filter_map(|i| u32::try_from(i).ok()).collect();
        rank(&c.list, &c.index, query, &recent).iter().filter_map(|&i| c.list.get(i as usize)).map(|e| e.id.clone()).collect()
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
            label_at: 0,
            names_item: false,
        }
    }

    /// `entries` with their index.
    fn synthetic(entries: Vec<Entry>) -> Catalog {
        described(entries, &[])
    }

    /// `entries` with their index, the params text of each in `params`.
    fn described(entries: Vec<Entry>, params: &[&str]) -> Catalog {
        let index = Index::build(&entries, &[], params, |_| None);
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
            // A whole word of the label, then a word of the label it shares a stem form with.
            ("scale zz", entry("Zz Scale", "a.one"), entry("Zz Scaling", "b.two")),
            // A word of the label it shares a stem form with, then a word of the label it starts.
            // The id makes "scale" a word of the catalog.
            ("scale zz", entry("Zz Scaling", "a.one"), entry("Zz Scalers", "scale.two")),
            // A word of the label it starts, then a word of the menu path.
            ("fill zz", entry("Zz Fillet", "a.one"), entry("Fill › Zz Other", "b.two")),
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

    /// A partly typed word that equals only a stem form ("scal" of Scaling, "spac" of Spacing) is a
    /// word the query starts in both results, so the shorter label stays first.
    #[test]
    fn a_partly_typed_word_does_not_lift_a_verb_form() {
        for (query, base, form) in [("scal zz", "Zz Scale", "Zz Scaling"), ("spac zz", "Zz Space", "Zz Spacing"), ("ad zz", "Zz Add", "Zz Added")] {
            let c = synthetic(vec![entry(form, "a.one"), entry(base, "b.two")]);
            assert_eq!(ranked(&c, query), ["b.two", "a.one"], "{query}");
        }
    }

    /// How the whole query matches comes before the word grades, so a stem form ranks above a word
    /// the query starts only among results in the same whole-query tier: "scale" lists Scale, then
    /// Scalers, which starts with the query, then Scaling, and "scale zz" lists Zz Scaling above Zz
    /// Scalers.
    #[test]
    fn a_whole_query_tier_comes_before_a_stem_form() {
        let c = synthetic(vec![entry("Scaling", "c.three"), entry("Scalers", "b.two"), entry("Scale", "a.one")]);
        assert_eq!(ranked(&c, "scale"), ["a.one", "b.two", "c.three"]);
        let c = synthetic(vec![entry("Zz Scalers", "b.two"), entry("Zz Scaling", "a.one"), entry("Zz Scale", "c.three")]);
        assert_eq!(ranked(&c, "scale zz"), ["c.three", "a.one", "b.two"]);
    }

    /// Results that no stem form matches keep the order they had before stems were added: two
    /// results whose grades summed the same still tie on the sum, and the shorter label comes
    /// first.
    #[test]
    fn results_matched_without_a_stem_keep_their_order() {
        // Two label prefixes and a menu path word, against a label word and two menu path words.
        let c = synthetic(vec![entry("Bet › Gam › Alp Zzzzzzzzzz", "b.two"), entry("Gam › Alpx Betx", "a.one")]);
        assert_eq!(ranked(&c, "alp bet gam"), ["a.one", "b.two"]);
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

    /// For each pair of adjacent grades and whole-query tiers, a recent pick ranks above a result
    /// that matches the query as well as it does and comes earlier in the catalog, and below the
    /// result that matches better; of two such recent picks, the more recent ranks first.
    #[test]
    fn a_recent_pick_ranks_first_within_its_grades() {
        let description = "{} keeps the art central on the page";
        for (query, better, worse, said) in [
            ("cut zz", entry("Zz Cut", "a.one"), entry("Zz Cutter", "b.two"), ""),
            ("scale zz", entry("Zz Scale", "a.one"), entry("Zz Scaling", "b.two"), ""),
            ("scale zz", entry("Zz Scaling", "a.one"), entry("Zz Scalers", "scale.two"), ""),
            ("fill zz", entry("Zz Fillet", "a.one"), entry("Fill › Zz Other", "b.two"), ""),
            ("zap zz", entry("Zap › Zz One", "a.one"), entry("Zz Two", "zap.two"), ""),
            ("zip zz", entry("Zz Three", "zip.three"), entry("Zz Unzipped", "b.two"), ""),
            ("zz.one", entry("Alpha", "zz.one"), entry("zz.one", "b.two"), ""),
            ("snap", entry("Snap", "a.one"), entry("Snapshot", "b.two"), ""),
            ("ab", entry("Abacus", "a.one"), entry("Alpha Beta", "b.two"), ""),
            ("gb", entry("Green Blue", "a.one"), entry("Zz Gb", "b.two"), ""),
            // Text inside a word, then a typo of a word of the label.
            ("colr", entry("Zz Bcolrb", "b.two"), entry("Zz Color", "a.one"), ""),
            // A typo of a word of the label, then a word of the description.
            ("centr", entry("Zz Center", "a.one"), entry("Zz Beta", "b.two"), description),
            // The word itself in the label, then a word of the same meaning in the label.
            ("remove", entry("Zz Remove", "a.one"), entry("Zz Delete", "b.two"), ""),
            // Matches through a word of the same meaning: the whole label, then a word of it.
            ("colour", entry("Color", "a.one"), entry("Zz Color", "b.two"), ""),
        ] {
            let (b, w) = (better.id.clone(), worse.id.clone());
            // A second result that matches as the worse one does and comes before it.
            let twin = entry(&worse.shown, &format!("{w}x"));
            let t = twin.id.clone();
            let c = described(vec![twin, worse, better], &[said, said, ""]);
            let (b, w, t) = (b.as_str(), w.as_str(), t.as_str());
            assert_eq!(recently(&c, query, &[]), [b, t, w], "{query}: with no recent picks");
            assert_eq!(recently(&c, query, &[w]), [b, w, t], "{query}: the recent pick first within its grades");
            assert_eq!(recently(&c, query, &[w, t]), [b, w, t], "{query}: the more recent first");
            assert_eq!(recently(&c, query, &[t, w]), [b, t, w], "{query}: the more recent first");
        }
        // An empty query lists every result in catalog order.
        let c = synthetic(vec![entry("Zz One", "a.one"), entry("Zz Two", "b.two")]);
        assert_eq!(recently(&c, "", &["b.two"]), ["a.one", "b.two"]);
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
            let first = rank(&en.list, &en.index, q, &[]).first().and_then(|&i| en.list.get(i as usize)).map(|e| e.shown.to_lowercase());
            let label = first.as_deref().map(|n| split_name(n).0.to_string()).unwrap_or_default();
            assert!(words(&label).any(|w| w.starts_with(q)), "{q}: {first:?}");
        }
    }

    /// Do `a` and `b` share a form, each taken as it is or as one of its stem forms?
    fn meet(a: &str, b: &str) -> bool {
        let forms = |w: &str| std::iter::once(w.to_string()).chain(stems(w)).collect::<Vec<_>>();
        let b = forms(b);
        forms(a).iter().any(|f| b.contains(f))
    }

    /// English plurals and verb forms meet their base word, and words that only look alike do not.
    #[test]
    fn plurals_and_verb_forms_meet_their_base_word() {
        for (a, b) in [
            ("outlines", "outline"),
            ("rounded", "round"),
            ("paths", "path"),
            ("cutting", "cut"),
            ("scaling", "scale"),
            ("rotated", "rotate"),
            ("spacing", "space"),
            ("tracing", "trace"),
            ("curved", "curve"),
            ("filled", "fill"),
            ("missing", "missing"),
            ("boxes", "box"),
            ("brushes", "brush"),
            ("glasses", "glass"),
            ("copies", "copy"),
            ("movies", "movie"),
            ("artist's", "artist"),
            ("rotated", "rotating"),
            ("added", "add"),
            ("rasterizes", "rasterize"),
            ("sizes", "size"),
        ] {
            assert!(meet(a, b), "{a} meets {b}");
        }
        for (a, b) in [("series", "sery"), ("missing", "mis"), ("press", "pres"), ("bus", "bu"), ("axis", "axi"), ("cutting", "cu"), ("added", "ad")]
        {
            assert!(!meet(a, b), "{a} does not meet {b}");
        }
        for w in ["outlines", "scaling", "cutting", "copies", "s", "'s", "ed", "ing", "übers", "群組", "series"] {
            assert!(stems(w).len() <= 2, "{w}");
            assert!(stems(w).iter().all(|s| !s.is_empty() && s != w), "{w}: {:?}", stems(w));
        }
        assert!(stems("übers").is_empty() && stems("series").is_empty() && stems("species").is_empty());
    }

    /// A query word in a plural or verb form finds a result whose label or menu path holds the base
    /// word, below one that holds the word itself, and the words of a name shown in another language
    /// are not stemmed.
    #[test]
    fn a_plural_or_verb_form_finds_the_result() {
        let c = synthetic(vec![entry("Zz Rotate", "a.one"), entry("Spacing › Zz Other", "b.two"), entry("Zz Rotated", "c.three")]);
        assert_eq!(ranked(&c, "rotating"), ["a.one", "c.three"]);
        assert_eq!(ranked(&c, "rotated"), ["c.three", "a.one"]);
        assert_eq!(ranked(&c, "spaces other"), ["b.two"], "through the menu path");
        let shown = Entry { shown: "Zz Trees".into(), haystack: "zz one\nzz one\nzz trees\na.one".into(), ..entry("Zz One", "a.one") };
        let c = synthetic(vec![shown, entry("Zz Trees", "b.two")]);
        assert_eq!(ranked(&c, "treed zz"), ["b.two"], "only the English name is stemmed");
        let en = catalog(Lang::EN, false);
        assert!(place(&en, "outlines", "type.createOutlines").is_some_and(|p| p < 3));
        assert_eq!(ranked(&en, "rounded corners").first().map(String::as_str), Some("effect:stylize.roundCorners"));
        for (q, id) in [
            ("scaling", "object.scale"),
            ("rotated", "object.rotate"),
            ("rasterizes", "object.rasterize"),
            ("added anchor points", "object.path.addAnchorPoints"),
        ] {
            assert!(place(&en, q, id).is_some(), "{q} finds {id}");
        }
    }

    /// Edits are counted up to a bound: a changed, inserted or removed character, or two adjacent
    /// characters swapped, is one edit, a character inserted between two swapped characters is one
    /// more, and words further apart than the bound give none.
    #[test]
    fn edits_are_counted_up_to_a_bound() {
        let edits = |a: &str, b: &str, bound: usize| edits_within(&a.chars().collect::<Vec<_>>(), &b.chars().collect::<Vec<_>>(), bound);
        for (a, b, bound, expected) in [
            ("group", "group", 1, Some(0)),
            ("gruop", "group", 1, Some(1)),
            ("baab", "abab", 1, Some(1)),
            ("elipse", "ellipse", 1, Some(1)),
            ("rectange", "rectangle", 2, Some(1)),
            ("eyedroper", "eyedropper", 2, Some(1)),
            ("recktanglee", "rectangle", 2, Some(2)),
            ("grop", "group", 1, Some(1)),
            ("grap", "group", 1, None),
            ("ab", "abcd", 1, None),
            ("ca", "abc", 2, Some(2)),
            ("ca", "abc", 1, None),
            ("rectgale", "rectangle", 2, Some(2)),
            ("rectgalle", "rectangle", 2, None),
            ("bacdefghijkl", "abcdefghijkl", 2, Some(1)),
            ("abcdefghijlk", "abcdefghijkxl", 2, Some(2)),
            ("abcdefghijlk", "abcdefghijkxyl", 2, None),
            ("abcd", "wxyz", 2, None),
            ("über", "uber", 1, Some(1)),
            ("", "ab", 2, Some(2)),
        ] {
            assert_eq!(edits(a, b, bound), expected, "{a} {b}");
            assert_eq!(edits(b, a, bound), expected, "{b} {a}");
        }
        assert_eq!(typo_bound("pen"), None);
        assert_eq!(typo_bound("elip"), Some(1));
        assert_eq!(typo_bound("rectang"), Some(1));
        assert_eq!(typo_bound("rectange"), Some(2));
        for w in ["群組群組", "かなかなか", "한국어단어", "zz群組"] {
            assert_eq!(typo_bound(w), None, "{w}");
        }
    }

    /// A query word with a typo finds a result whose label or menu path holds the word, below every
    /// result matched without a typo, and a word that is a word or a word start of some result, a
    /// word of fewer than 4 letters, an id and a word of a script written without spaces get no
    /// typo matches.
    #[test]
    fn a_word_with_a_typo_finds_the_result() {
        let c = synthetic(vec![entry("Zz Group", "a.one"), entry("Zz Cut", "b.two"), entry("Zz Rectangle", "c.three")]);
        assert_eq!(ranked(&c, "gruop"), ["a.one"], "two letters swapped");
        assert_eq!(ranked(&c, "gruop zz"), ["a.one"]);
        assert_eq!(ranked(&c, "cutx"), ["b.two"]);
        assert!(ranked(&c, "cux").is_empty(), "3 letters");
        assert_eq!(ranked(&c, "recktanglee"), ["c.three"], "two edits in a longer word");
        assert_eq!(ranked(&c, "rectgale"), ["c.three"], "two letters swapped and one inserted between them");
        assert!(ranked(&c, "rectanglexxx").is_empty());
        // A word found only as text inside a word of the result goes with a word with a typo, in
        // either order, and a word found nowhere in the result still leaves it out.
        let c = synthetic(vec![entry("Zz Drop Shadow", "a.one"), entry("Zz Drop", "b.two")]);
        assert_eq!(ranked(&c, "rop shdaow"), ["a.one"]);
        assert_eq!(ranked(&c, "shdaow rop"), ["a.one"]);
        assert!(ranked(&c, "rop shdaow qqq").is_empty());
        assert!(ranked(&c, "qqq rop shdaow").is_empty());
        // Below a result whose name holds the word as text, and below one that holds it whole.
        let c = synthetic(vec![entry("Zz Color", "a.one"), entry("Zz Bcolrb", "b.two")]);
        assert_eq!(ranked(&c, "colr"), ["b.two", "a.one"]);
        let c = synthetic(vec![entry("Color", "a.one"), entry("Zz Bcolrb", "b.two")]);
        assert_eq!(ranked(&c, "colr"), ["b.two", "a.one"], "also when the word spelled right is the label");
        let c = synthetic(vec![entry("Zz Color", "a.one"), entry("Zz Colr", "b.two")]);
        assert_eq!(ranked(&c, "colr"), ["b.two"], "a word some result holds gets no typo matches");
        // A word of the label before a word of the menu path.
        let c = synthetic(vec![entry("Color › Zz Other", "a.one"), entry("Zz Color", "b.two")]);
        assert_eq!(ranked(&c, "colr"), ["b.two", "a.one"]);
        // Ids and words of scripts written without spaces.
        let c = synthetic(vec![entry("Zz One", "group.one"), entry("物件 › 群組群組", "b.two")]);
        assert!(ranked(&c, "gruop").is_empty());
        assert!(ranked(&c, "群組群物").is_empty());
        // A word of Latin letters followed by Han letters, one edit from "abcd", is not compared.
        let c = synthetic(vec![entry("Zz Abc群", "a.one")]);
        assert!(ranked(&c, "abcd").is_empty());
        // The whole query is read with the word it is a typo of: "gruop" names Group as "group" does.
        let c = synthetic(vec![entry("Group Selection Tool", "tool:groupSelection"), entry("Object › Group", "object.group")]);
        assert_eq!(ranked(&c, "gruop"), ranked(&c, "group"));
        assert_eq!(ranked(&c, "gruop").first().map(String::as_str), Some("object.group"));
        // The rest of the query is read as typed, `›` included.
        let c = synthetic(vec![entry("Object › Group Zz Tool", "tool:zz"), entry("Object › Group", "object.group")]);
        assert_eq!(ranked(&c, "object › group"), ["object.group", "tool:zz"]);
        assert_eq!(ranked(&c, "object › gruop"), ranked(&c, "object › group"));
        // A label typo is read with the word of the label, even when a word of the menu path is
        // nearer: "abcdefgh" reads as "abcdefyy" in the first, which is then its label.
        let c = synthetic(vec![entry("Abcdefyy Zz", "tool:b"), entry("Abcdefgx › Abcdefyy", "a.one")]);
        assert_eq!(ranked(&c, "abcdefgh"), ["a.one", "tool:b"]);
    }

    /// A query word that another result matches only through a word with the same meaning is still
    /// compared with the words it may be a typo of: "spin" means rotate, and is also one letter from
    /// Span.
    #[test]
    fn a_word_with_the_same_meaning_elsewhere_still_finds_a_typo() {
        let c = synthetic(vec![entry("Zz Rotate Canvas", "a.one"), entry("Zz Span Anchors", "b.two")]);
        assert_eq!(ranked(&c, "spin anchors"), ["b.two"]);
        assert_eq!(ranked(&c, "spin"), ["a.one", "b.two"], "the word with the same meaning ranks above the typo");
        assert_eq!(ranked(&c, "spin canvas"), ["a.one"]);
    }

    /// A query word with a typo meets at most 32 words, the nearest first, and a long word with no
    /// match finds nothing.
    #[test]
    fn a_typo_meets_at_most_32_words() {
        // 35 words one edit from the query and one word two edits from it, which sorts before them.
        let near = ('a'..='z').chain('0'..='9').filter(|&ch| ch != 'q');
        let mut entries: Vec<Entry> = near.enumerate().map(|(i, ch)| entry(&format!("Abcdefg{ch}"), &format!("a.n{i}"))).collect();
        entries.insert(0, entry("Abcdeaxq", "far.one"));
        let c = synthetic(entries);
        let found = ranked(&c, "abcdefgq");
        assert_eq!(found.len(), 32, "{found:?}");
        assert!(!found.contains(&"far.one".to_string()));
        let c = synthetic(vec![entry("Abcdeaxq", "far.one")]);
        assert_eq!(ranked(&c, "abcdefgq"), ["far.one"], "two edits in a word of 8 letters");
        assert!(ranked(&c, &"q".repeat(30)).is_empty());
    }

    /// On the English results, misspelled words list first the commands the word spelled right
    /// lists first by labels, menu paths and ids (a typo does not match descriptions), the tool among
    /// the first four, and a word of 3 letters is not read as a typo.
    #[test]
    fn misspelled_words_find_the_command() {
        let c = catalog(Lang::EN, false);
        let first = |q: &str| ranked(&c, q).first().cloned().unwrap_or_default();
        assert_eq!(first("gruop"), "object.group");
        let plain = Index::build(&c.list, &[], &[], |_| None);
        let id = |&i: &u32| c.list.get(i as usize).map(|e| e.id.clone()).unwrap_or_default();
        for (typo, word, tool) in [
            ("eyedroper", "eyedropper", "tool:eyedropper"),
            ("rectange", "rectangle", "tool:rectangle"),
            ("elipse", "ellipse", "tool:ellipse"),
            ("gruop", "group", "tool:groupSelection"),
        ] {
            assert!(substring_rule(&c, typo).is_empty(), "{typo} matched nothing before");
            let top = |q: &str| ranked(&c, q).into_iter().take(4).collect::<Vec<_>>();
            let spelled_right: Vec<String> = rank(&c.list, &plain, word, &[]).iter().take(4).map(id).collect();
            assert_eq!(top(typo), spelled_right, "{typo}");
            assert!(place(&c, typo, tool).is_some_and(|p| p < 4), "{typo}: {:?}", ranked(&c, typo));
        }
        assert_eq!(first("pen"), "tool:pen");
        assert!(place(&c, "pen", "file.open").is_some(), "Open… holds the text");
        assert!(place(&c, "rop shdaow", "effect:stylize.dropShadow").is_some(), "{:?}", ranked(&c, "rop shdaow"));
    }

    /// A query word finds a result whose label or menu path holds another word of its group of words
    /// with the same meaning: below a result whose label holds the word itself, above one whose menu
    /// path holds the word, and with a menu path word of the group last. A stem form of the query word,
    /// of a result's word and of a group's word meet the group too; ids, word starts and words of no
    /// group do not.
    #[test]
    fn a_word_with_the_same_meaning_finds_the_result() {
        let c = synthetic(vec![
            entry("Delete › Zz Path", "d.four"),
            entry("Remove › Zz Other", "c.three"),
            entry("Zz Delete", "a.one"),
            entry("Zz Remove", "b.two"),
        ]);
        assert_eq!(ranked(&c, "remove"), ["b.two", "a.one", "c.three", "d.four"]);
        assert!(ranked(&c, "remo zz").iter().all(|id| id != "a.one" && id != "d.four"), "a word start is not looked up");
        let c = synthetic(vec![entry("Zz Reflect…", "a.one"), entry("Zz Reflecting", "b.two"), entry("Zz One", "reflect.one")]);
        assert_eq!(ranked(&c, "flipped zz"), ["a.one", "b.two"], "stem forms on both sides, and no id");
        let c = synthetic(vec![entry("Zz Unite", "a.one"), entry("Zz Merge", "b.two")]);
        assert_eq!(ranked(&c, "merge"), ["b.two"], "merge is in no group");
        assert_eq!(ranked(&c, "combine"), ["a.one"]);
        // The stem form of a plural word of a group is in the group too: "setting" meets settings.
        let c = synthetic(vec![entry("Zz Preferences", "a.one"), entry("Zz Other", "b.two")]);
        assert_eq!(ranked(&c, "setting zz"), ["a.one"], "a stem form of a group's word");
        assert_eq!(ranked(&c, "settings zz"), ["a.one"]);
        // Among results that match equally well, the whole query read with the word of the same
        // meaning ("colour" as "color") puts the label Color first, but never above a label that
        // holds the query word itself.
        let c = synthetic(vec![entry("Zz Color", "tool:zz"), entry("Color", "panel:color"), entry("Zz Colour", "c.three")]);
        assert_eq!(ranked(&c, "colour"), ["c.three", "panel:color", "tool:zz"]);
    }

    /// A word that is text inside a word of the result combines with a word matched through a word
    /// of the same meaning, a stem form, a typo or the description, and a word that is text inside
    /// a word of no result finds nothing.
    #[test]
    fn a_word_inside_a_word_combines_with_the_other_matches() {
        let c = described(vec![entry("Remove Background…", "a.one"), entry("Zz Group", "b.two")], &["{} make the images transparent", ""]);
        assert_eq!(ranked(&c, "bg remove"), ["a.one"]);
        assert_eq!(ranked(&c, "background mov"), ["a.one"]);
        assert_eq!(ranked(&c, "bg mov"), ["a.one"], "a word of the same meaning");
        assert_eq!(ranked(&c, "removing kgrou"), ["a.one"], "a stem form");
        assert_eq!(ranked(&c, "transparent kgrou"), ["a.one"], "the description");
        assert_eq!(ranked(&c, "emov backgruond"), ["a.one"], "a typo after a word inside a word");
        assert!(ranked(&c, "bg qxq").is_empty());
    }

    /// A catalog without an index (one with more results than a posting can name) matches by the
    /// substring rule alone: a query word whose parts are each text in a name, where the word
    /// itself is not, finds nothing.
    #[test]
    fn without_an_index_only_the_substring_rule_matches() {
        let entries = vec![entry("Remove Background…", "a.one"), entry("Zz Group", "b.two")];
        let c = Catalog { list: entries, index: Index::default(), ranked: std::sync::Mutex::new(None) };
        assert!(ranked(&c, "mov.nd").is_empty(), "{:?}", ranked(&c, "mov.nd"));
        assert_eq!(ranked(&c, "background mov"), ["a.one"]);
    }

    /// A query word that is an English stem form of a word of a group ("setting" of settings,
    /// "preference" of preferences) finds the group, and a stem form of a word of the group meets a
    /// result's word ("prefs" and Zz Setting). A result whose label holds the query word itself
    /// still ranks first. The whole query is read with the group word a label holds before a stem
    /// form of it, so "prefs" reads Preferences as its exact label.
    #[test]
    fn a_stem_form_of_a_group_word_meets_the_group() {
        let c = synthetic(vec![entry("Preferences Tool", "tool:zz"), entry("Preferences", "a.one")]);
        assert_eq!(ranked(&c, "prefs"), ["a.one", "tool:zz"]);
        let c = synthetic(vec![entry("Zz Preferences", "a.one"), entry("Zz Setting", "b.two"), entry("Zz Settings", "c.three")]);
        let setting = ranked(&c, "setting");
        assert_eq!(setting.first().map(String::as_str), Some("b.two"), "{setting:?}");
        assert!(setting.iter().any(|id| id == "a.one"), "{setting:?}");
        assert!(ranked(&c, "preference").iter().any(|id| id == "c.three"), "a singular query word finds the group");
        let prefs = ranked(&c, "prefs");
        for id in ["a.one", "b.two", "c.three"] {
            assert!(prefs.iter().any(|r| r == id), "{id} in {prefs:?}");
        }
        assert!(ranked(&c, "set").iter().all(|id| id != "a.one"), "a stem form of a query word that is no group word finds no group");
    }

    /// A query word that only a description holds meets the stem forms of descriptions, not those
    /// of labels: "rat" in another command's description does not lift Zz Rating above Zz Ratio.
    #[test]
    fn a_description_word_does_not_lift_a_label_stem() {
        let entries = || vec![entry("Zz Ratio", "a.one"), entry("Zz Rating", "b.two"), entry("Animal Outline", "c.three")];
        assert_eq!(ranked(&synthetic(entries()), "rat zz"), ["a.one", "b.two"]);
        let c = described(entries(), &["", "", "Trace a rat silhouette"]);
        assert_eq!(ranked(&c, "rat zz"), ["a.one", "b.two"]);
        let c = described(vec![entry("Zz Outline", "a.one"), entry("Zz Trace", "b.two")], &["Scaling the art", "Scale it"]);
        assert!(ranked(&c, "scale").iter().any(|id| id == "a.one"), "a description word still meets a description's stem form");
    }

    /// When a label holds two words of the group a query word means, the whole query is read with
    /// the one that matches it best: "prefs preferences" reads Settings Preferences as its exact
    /// label, which ranks it before a tool whose label holds only one of the words.
    #[test]
    fn a_synonym_is_read_as_the_group_word_that_matches_the_whole_query() {
        let c = synthetic(vec![entry("Preferences Tool", "tool:zz"), entry("Settings Preferences", "a.one")]);
        assert_eq!(ranked(&c, "prefs preferences").first().map(String::as_str), Some("a.one"));
        let c = synthetic(vec![entry("Preferences Tool", "tool:zz"), entry("Preferences", "a.one")]);
        assert_eq!(ranked(&c, "prefs"), ["a.one", "tool:zz"]);
    }

    /// A catalog row adds words in the UI language to a group, and a word that is not a single word
    /// or that is a word of another group is left out.
    #[test]
    fn a_catalog_row_adds_words_to_a_group() {
        let entries = || vec![entry("Zz Delete", "a.one")];
        let row = |key: &str| (key == "remove, delete, erase, clear").then(|| "Entfernen, ganz weg, text".to_string());
        let with = Catalog { list: entries(), index: Index::build(&entries(), &[], &[], row), ranked: std::sync::Mutex::new(None) };
        assert_eq!(ranked(&with, "entfernen"), ["a.one"]);
        assert!(ranked(&with, "ganz weg").is_empty(), "two words are left out");
        assert!(ranked(&with, "text").is_empty(), "text stays in its own group");
        let without = synthetic(entries());
        assert!(ranked(&without, "entfernen").is_empty(), "only while the row's language is the UI language");
    }

    /// A description is the params text without the groups of its schema and result: one fixture
    /// of each shape the params texts take.
    #[test]
    fn a_description_leaves_out_the_schema_and_the_result() {
        for (params, expected) in [
            // A schema, prose and a result.
            ("{a: 1, b?: bool (true)} make it so → {changed, ids}", "make it so"),
            // A quoted brace inside a schema.
            ("{sep: \"}\" or \"{\" (\"}\")} split the text", "split the text"),
            // A schema only.
            ("{radius: pt (10)}", ""),
            // An empty schema, then prose with no arrow.
            ("{} choose the Corner Tool", "choose the Corner Tool"),
            // Prose after the result.
            ("{from?: index} → {index}: moves the document", ": moves the document"),
            // Nested braces and brackets.
            ("{a: {b: 1}, c: [x, {y}]} trims the path", "trims the path"),
            // Alternative schemas.
            ("{id, dx} or {items: [{id}…], dx} move anchors", "or move anchors"),
            // A result in brackets or parentheses, and a result written as words.
            ("{} → [name…] for the active document", "for the active document"),
            ("{id} → ({x, y}) its place", "its place"),
            ("{id} → its id.", "→ its id."),
            // A group inside parentheses is text, and so is a group that is never closed.
            ("fits it (as {x} does) here", "fits it (as {x} does) here"),
            ("{a: 1 never closed", "{a: 1 never closed"),
            // A bracket or a double quote inside a backtick span is text, as in the engine's params
            // object, and so is a group inside a backtick span outside a group.
            ("{pattern?: `(`} transparent background", "transparent background"),
            ("{sep: `}`, quote: `\"`} split the text", "split the text"),
            ("uses `{x}` as text", "uses `{x}` as text"),
            // No group at all.
            ("plain   words\n here", "plain words here"),
            ("", ""),
        ] {
            assert_eq!(description(params), expected, "{params}");
        }
        // A long description ends at the end of a word.
        let cut = description(&"word ".repeat(100));
        assert!(cut.chars().count() <= DESCRIPTION_CHARS && cut.ends_with(" word"), "{cut}");
        let cut = description(&format!("{}handles", "abcd ".repeat(59)));
        assert!(cut.chars().count() <= DESCRIPTION_CHARS && cut.ends_with(" abcd"), "{cut}");
        let whole = format!("{}handles", "abcd ".repeat(58));
        assert_eq!(description(&whole), whole);
        assert!(description_words("the and ids true false null of a 16 10pt x").eq(["10pt"]));
        assert!(description_words("image's 'quoted' png").eq(["image's", "quoted", "png"]));
    }

    /// A word of a description finds the result as a whole word, a stem form, a word start or a word
    /// with the same meaning, below a result that holds the word in its label or menu path or inside
    /// a word. A schema's words, words left out of descriptions and typos do not find it.
    #[test]
    fn a_word_of_the_description_finds_the_result() {
        let c = described(
            vec![
                entry("Zz Alpha", "a.one"),
                entry("Zz Background", "b.two"),
                entry("Zz Underbackground", "c.three"),
                entry("Background › Zz Delta", "d.four"),
            ],
            &["{corners?: bool (true)} make the background of the images transparent → {changed}", "", "", ""],
        );
        assert_eq!(ranked(&c, "transparent"), ["a.one"]);
        assert_eq!(ranked(&c, "making images"), ["a.one"], "stem forms");
        assert_eq!(ranked(&c, "transp"), ["a.one"], "a word start");
        assert_eq!(ranked(&c, "background"), ["b.two", "d.four", "c.three", "a.one"]);
        assert_eq!(ranked(&c, "bg"), ["b.two", "d.four", "a.one"], "a word with the same meaning");
        assert_eq!(ranked(&c, "transparent background"), ["a.one"]);
        assert!(ranked(&c, "corners").is_empty(), "a word of the schema");
        assert!(ranked(&c, "changed").is_empty(), "a word of the result");
        assert!(ranked(&c, "the").is_empty(), "a word left out");
        assert!(ranked(&c, "trasnparent").is_empty(), "a typo");
        assert!(synthetic(vec![entry("Zz Alpha", "a.one")]).index.find("transparent").is_none());
    }

    /// A result found only by its description ranks below a result found through a typo of a word
    /// of its label: "centr" starts "centred" in a description and is a typo of "center". A result
    /// matched both ways ranks as matched through the typo.
    #[test]
    fn a_description_match_ranks_below_a_typo() {
        let c = described(vec![entry("Zz Beta", "b.two"), entry("Zz Center", "a.one")], &["{} keeps the art centred on the page", ""]);
        assert_eq!(ranked(&c, "centr"), ["a.one", "b.two"]);
        assert_eq!(ranked(&c, "zz centr"), ["a.one", "b.two"]);
        let both = described(vec![entry("Zz Center", "a.one"), entry("Center", "c.three")], &["", "{} keeps the art centred on the page"]);
        assert_eq!(ranked(&both, "centr"), ["c.three", "a.one"]);
    }

    /// On the English results, words of what a command does find it: "transparent background"
    /// lists Remove Background… near the top, and "png" lists it below every result with png in its
    /// name or menu path, while a word of its schema alone ("corners") does not list it.
    #[test]
    fn words_of_a_description_find_the_command() {
        let c = catalog(Lang::EN, false);
        let remove = "addon.image.removeBackground";
        let text = super::super::params_texts(&c.list);
        let at = c.list.iter().position(|e| e.id == remove).unwrap();
        assert!(description(text[at]).starts_with("make the background of each image transparent"), "{}", description(text[at]));
        assert!(substring_rule(&c, "transparent background").len() < ranked(&c, "transparent background").len());
        assert!(place(&c, "transparent background", remove).is_some_and(|p| p < 3), "{:?}", ranked(&c, "transparent background"));
        let png = ranked(&c, "png");
        let below = png.iter().position(|id| id == remove).unwrap();
        let named = |e: &Entry| names(e).chain(e.home.as_deref()).any(|n| n.to_lowercase().contains("png"));
        let with_png: Vec<usize> = c.list.iter().filter(|e| named(e)).filter_map(|e| png.iter().position(|id| *id == e.id)).collect();
        assert!(!with_png.is_empty() && with_png.iter().all(|&p| p < below), "{png:?}");
        assert!(place(&c, "corners", remove).is_none());
        assert!(place(&c, "corners", "effect:stylize.roundCorners").is_some_and(|p| p < 3), "{:?}", ranked(&c, "corners"));
    }

    /// On the English results, everyday words find commands whose names use another word, and a
    /// result whose label holds a word with the same meaning as the query ranks above one that holds
    /// the query only in its menu path.
    #[test]
    fn words_with_the_same_meaning_find_the_command() {
        let c = catalog(Lang::EN, false);
        let first = |q: &str| ranked(&c, q).first().cloned().unwrap_or_default();
        let within = |q: &str, id: &str, n: usize| place(&c, q, id).is_some_and(|p| p < n);
        for q in ["remove bg", "outline text", "flip", "combine", "vectorize", "colour"] {
            assert!(substring_rule(&c, q).len() < ranked(&c, q).len(), "{q} finds more than before");
        }
        assert_eq!(first("remove bg"), "addon.image.removeBackground");
        assert_eq!(first("outline text"), "type.createOutlines");
        assert!(within("flip", "object.reflect", 3), "{:?}", ranked(&c, "flip"));
        assert!(within("combine", "object.pathfinder.unite", 3), "{:?}", ranked(&c, "combine"));
        assert!(ranked(&c, "vectorize").iter().take(3).any(|id| id.starts_with("imageTrace.")), "{:?}", ranked(&c, "vectorize"));
        assert_eq!(first("colour"), "panel:color", "{:?}", ranked(&c, "colour"));
        // A word that only starts other labels is in a group, and its labels follow the direct match.
        assert_eq!(first("unite"), "object.pathfinder.unite");
        assert!(place(&c, "unite", "object.slice.combine").is_some(), "{:?}", ranked(&c, "unite"));
        assert!(place(&c, "delete", "view.guides.clear").is_some(), "{:?}", ranked(&c, "delete"));
        assert!(place(&c, "settings", "edit.preferences").is_some(), "{:?}", ranked(&c, "settings"));
        assert!(place(&c, "setting", "edit.preferences").is_some(), "{:?}", ranked(&c, "setting"));
        let merge = ranked(&c, "merge");
        let unite = merge.iter().position(|id| id == "object.pathfinder.unite");
        let named = |id: &String| c.list.iter().any(|e| &e.id == id && split_name(&e.shown).0 == "Merge");
        assert!(merge.first().is_some_and(named), "{merge:?}");
        assert!(unite.is_none() || merge.iter().rposition(named) < unite, "{merge:?}");
        let (scale, vertical) = (place(&c, "type", "type.scaleTextAttributes"), place(&c, "type", "type.orientation.vertical"));
        assert!(scale.is_some() && scale < vertical, "{scale:?} {vertical:?}");
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
        assert!(place(&native_catalog, "vectorcraft settings pref", "edit.preferences").is_some());
        assert!(place(&c, "vectorcraft settings pref", "edit.preferences").is_none());
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
        let params = super::super::params_texts(&c.list);
        let started = std::time::Instant::now();
        let index = Index::build(&c.list, &[], &params, |_| None);
        println!("catalog {built:?}, index alone {:?}, {} results, {} bytes", started.elapsed(), c.list.len(), index.bytes());
        let started = std::time::Instant::now();
        let without = Index::build(&c.list, &[], &[], |_| None);
        println!("index without descriptions {:?}, {} bytes", started.elapsed(), without.bytes());
        let long = "q".repeat(200);
        let typos = "gruop elipse rectange eyedroper trasnform patern gradeint symbl";
        let repeated = format!("{typos} ").repeat(3);
        let queries = [
            "e",
            "ea",
            "select",
            "bring front",
            "zzzz",
            "eyedroper",
            "gruop zz",
            "remove bg",
            "colour",
            "transparent background",
            "png",
            typos,
            repeated.as_str(),
            long.as_str(),
        ];
        for q in queries {
            let started = std::time::Instant::now();
            let runs = 100;
            for _ in 0..runs {
                std::hint::black_box(rank(&c.list, &c.index, q, &[]));
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
