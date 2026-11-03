//! Parser and lookup tables for translation catalogs (`*.tsv`, format documented in `zh-hant.tsv`).

use std::collections::HashMap;

/// A parsed catalog. Values are owned for the life of the process (catalogs are built once).
#[derive(Debug, Default)]
pub struct Catalog {
    /// context-free strings: English source → translation (the hot path, looked up every frame)
    plain: HashMap<String, String>,
    /// strings with a disambiguating context, keyed `context \u{1} source`
    contextual: HashMap<String, String>,
    /// command-id keyed strings
    ids: HashMap<String, String>,
    /// plural messages: `one|other` → forms
    plurals: HashMap<String, Vec<String>>,
    /// status and error messages without placeholders (`@msg`): English → translation
    messages: HashMap<String, String>,
    /// status and error messages with `{name}` placeholders (`@msg`, and each English form of a
    /// `@msgplural` row), most specific first
    templates: Vec<Template>,
}

/// A piece of a message template: literal text or a `{name}` placeholder.
#[derive(Debug)]
enum Piece {
    Text(String),
    Hole(String),
}

/// A message template (`Couldn't open {name}: {e}`) and its translation: one form for `@msg`,
/// the plural forms for `@msgplural`.
#[derive(Debug)]
struct Template {
    pieces: Vec<Piece>,
    /// Bytes of literal text: a template with more of it is the more specific match.
    literal: usize,
    forms: Vec<String>,
}

/// The placeholder values of a matched message template: placeholder name and value.
pub type Captures<'p, 'a> = Vec<(&'p str, &'a str)>;

/// Messages longer than this are shown as they are (template matching backtracks).
const MAX_MESSAGE_LEN: usize = 2000;

impl Template {
    /// `None` for a template that can't be matched reliably: no literal text, or two placeholders
    /// in a row (where one ends and the next begins is unknowable).
    fn parse(source: &str, forms: Vec<String>) -> Option<Template> {
        let mut pieces = Vec::new();
        let mut rest = source;
        while let Some(a) = rest.find('{') {
            let after = rest.get(a + 1..)?;
            let b = after.find('}')?;
            if a > 0 {
                pieces.push(Piece::Text(rest.get(..a)?.to_string()));
            } else if matches!(pieces.last(), Some(Piece::Hole(_))) {
                return None;
            }
            pieces.push(Piece::Hole(after.get(..b)?.to_string()));
            rest = after.get(b + 1..)?;
        }
        if !rest.is_empty() {
            pieces.push(Piece::Text(rest.to_string()));
        }
        let literal = pieces.iter().map(|p| if let Piece::Text(t) = p { t.len() } else { 0 }).sum();
        (literal > 0).then_some(Template { pieces, literal, forms })
    }

    /// The placeholder values if `s` is an instance of this template.
    fn captures<'a>(&self, s: &'a str) -> Option<Vec<(&str, &'a str)>> {
        let mut out = Vec::new();
        match_pieces(&self.pieces, s, &mut out).then_some(out)
    }
}

/// Match `s` against `pieces`, a placeholder taking the shortest non-empty text that lets the
/// rest match (the last one takes what remains).
fn match_pieces<'p, 'a>(pieces: &'p [Piece], s: &'a str, out: &mut Vec<(&'p str, &'a str)>) -> bool {
    let Some((first, rest)) = pieces.split_first() else { return s.is_empty() };
    match first {
        Piece::Text(t) => s.strip_prefix(t.as_str()).is_some_and(|tail| match_pieces(rest, tail, out)),
        Piece::Hole(name) => {
            let Some((next, after)) = rest.split_first() else {
                if s.is_empty() {
                    return false;
                }
                out.push((name.as_str(), s));
                return true;
            };
            let Piece::Text(next) = next else { return false };
            // Every place the next literal occurs, nearest first (the hole is never empty).
            let mut from = s.chars().next().map_or(1, char::len_utf8);
            while let Some(at) = s.get(from..).and_then(|tail| tail.find(next.as_str())).map(|i| i + from) {
                let (Some(value), Some(tail)) = (s.get(..at), s.get(at + next.len()..)) else { return false };
                let mark = out.len();
                out.push((name.as_str(), value));
                if match_pieces(after, tail, out) {
                    return true;
                }
                out.truncate(mark);
                from = at + next.chars().next().map_or(1, char::len_utf8);
            }
            false
        }
    }
}

/// A catalog entry as read from the file: (context, source, translation).
pub type Entry = (String, String, String);

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Read the entries of a catalog file. Malformed lines are returned as errors (and skipped), so
/// a bad translation never breaks the UI; the tests insist the bundled catalogs have none.
pub fn parse_entries(text: &str) -> (Vec<Entry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        match (cols.next(), cols.next(), cols.next(), cols.next()) {
            (Some(ctx), Some(src), Some(tr), None) if !src.is_empty() && !tr.is_empty() => {
                entries.push((unescape(ctx), unescape(src), unescape(tr)));
            }
            _ => errors.push(format!("line {}: expected `context<TAB>source<TAB>translation`", n + 1)),
        }
    }
    (entries, errors)
}

impl Catalog {
    pub fn parse(text: &str) -> Catalog {
        let mut c = Catalog::default();
        for (ctx, src, tr) in parse_entries(text).0 {
            match ctx.as_str() {
                "" => {
                    c.plain.insert(src, tr);
                }
                "@id" => {
                    c.ids.insert(src, tr);
                }
                "@plural" => {
                    c.plurals.insert(src, tr.split('|').map(str::to_string).collect());
                }
                "@msg" => {
                    if src.contains('{') {
                        c.templates.extend(Template::parse(&src, vec![tr]));
                    } else {
                        c.messages.insert(src, tr);
                    }
                }
                "@msgplural" => {
                    let forms: Vec<String> = tr.split('|').map(str::to_string).collect();
                    c.templates.extend(src.split('|').filter_map(|english| Template::parse(english, forms.clone())));
                }
                _ => {
                    c.contextual.insert(format!("{ctx}\u{1}{src}"), tr);
                }
            }
        }
        // Most literal text first, so `Couldn't open {name}: {e}` wins over `{path}: {e}`.
        c.templates.sort_by_key(|t| std::cmp::Reverse(t.literal));
        c
    }

    pub fn plain(&self, s: &str) -> Option<&str> {
        self.plain.get(s).map(String::as_str)
    }

    pub fn contextual(&self, ctx: &str, s: &str) -> Option<&str> {
        self.contextual.get(&format!("{ctx}\u{1}{s}")).map(String::as_str)
    }

    pub fn id(&self, id: &str) -> Option<&str> {
        self.ids.get(id).map(String::as_str)
    }

    /// [`Catalog::message_forms`] with the first form of the translation.
    #[cfg(test)]
    pub fn message<'a>(&self, s: &'a str) -> Option<(&str, Captures<'_, 'a>)> {
        self.message_forms(s).and_then(|(forms, caps)| Some((forms.first()?.as_str(), caps)))
    }

    /// A status or error message: an exact `@msg` entry, else the first template it is an instance
    /// of, with the placeholder values filled in. The values are returned for the caller to
    /// translate (a reason after `…: {e}` is often a message of its own). The translation has one
    /// form for an `@msg` row and the plural forms of a `@msgplural` row, of which the caller picks
    /// one by the `{n}` value.
    pub fn message_forms<'a>(&self, s: &'a str) -> Option<(&[String], Captures<'_, 'a>)> {
        if let Some(tr) = self.messages.get(s) {
            return Some((std::slice::from_ref(tr), Vec::new()));
        }
        if s.len() > MAX_MESSAGE_LEN {
            return None;
        }
        self.templates.iter().find_map(|t| t.captures(s).map(|caps| (t.forms.as_slice(), caps)))
    }

    /// The plural form `index` of the message whose English forms are `one|other`.
    pub fn plural(&self, one: &str, other: &str, index: usize) -> Option<&str> {
        let forms = self.plurals.get(&format!("{one}|{other}"))?;
        forms.get(index.min(forms.len().saturating_sub(1))).map(String::as_str)
    }
}

/// `{name}` placeholders of a template, in order of appearance.
#[cfg(test)]
pub fn placeholders(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(a) = rest.find('{') {
        let after = &rest[a + 1..];
        match after.find('}') {
            Some(b) => {
                out.push(&after[..b]);
                rest = &after[b + 1..];
            }
            None => break,
        }
    }
    out
}
