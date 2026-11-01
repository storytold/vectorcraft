//! Shortcut sets another app saves as text (`.kys` files that start `/Menus {`), read into their
//! entries without changing anything (`shortcutSet.read`). Names and codes the user matched on an
//! earlier import ([`super::importmatches`]) come back with what they stand for.
//!
//! The text is a tree of blocks. A slash name opens a block with `{` (a backslash in a name takes the
//! next character as it is, so `/Acme\ Lasso\ Tool` names "Acme Lasso Tool"), and `}` closes it.
//! Inside a block, a slash name followed by an integer sets a field: `/Context`, `/Modifiers`,
//! `/Represent` and `/Key`. A block that sets `/Key` is an entry, and the blocks around it are its
//! section. The layout follows the description at https://filext.com/file-extension/KYS and the
//! example in the MIT-licensed AiCommandPalette README
//! (https://github.com/joshbduncan/AiCommandPalette), whose entry for the 0 key holds `/Key 48` and
//! `/Modifiers 0`. No public source says what the other codes mean, so they are read as numbers.

use serde_json::{Value, json};

use super::{CommandSpec, EngineError, Result, Session, always, bad, cmd, importmatches, str_param};

/// The most bytes of a file read.
const MAX_BYTES: usize = 4 << 20;
/// The most bytes taken in: [`MAX_BYTES`] after a byte order mark, and one more that shows the text
/// goes on.
pub const READ_LIMIT: usize = MAX_BYTES + 4;
/// The most blocks open at once.
const MAX_DEPTH: usize = 16;
/// The most entries read.
const MAX_ENTRIES: usize = 20_000;
/// The longest name kept (bytes).
const MAX_NAME: usize = 1024;
/// The most bytes of names kept over all entries, sections included: each entry repeats the names
/// of the blocks around it, so a small file could otherwise fill hundreds of megabytes.
const MAX_KEPT: usize = 2 * MAX_BYTES;
/// The most tokens listed as unread.
const MAX_UNREAD: usize = 1000;
/// The longest unread token shown (characters).
const UNREAD_SHOWN: usize = 80;

/// What `shortcutSet.read` says about a file that isn't a text shortcut set.
pub const NOT_READABLE: &str = "This shortcut file is in a form VectorCraft can't read.";

/// The extensions of shortcut sets other apps save (File › Open lists them).
pub const EXTS: &[&str] = &["kys"];

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "shortcutSet.read",
        "Read Shortcut Set File",
        [],
        None,
        "{path? | data?: the file's text | dataBase64?, name?: the file's name (default: the path's)} read a shortcut set another app saved as text (.kys, starting \"/Menus {\") without changing anything: each entry is a block that sets /Key, with the names of the blocks around it as its section → {set: the file's name without its extension, entries: [{section: [names], name, key, modifiers, context, represent, line, target?, chord?}] (codes as the file writes them; null where the entry doesn't set one; target: what importMatches says the name stands for; chord: the shortcut its codes stand for, as Edit › Keyboard Shortcuts writes it, when importMatches decodes them, an entry without /Modifiers taking none), codes: {keys: [{value, count, key?}], modifiers: [{value, count, mods?}]} (key and mods from importMatches), unread: [{line, text}] (tokens that aren't read, at most 1000), truncated: true when a cap stopped the reading (4 MiB, 16 open blocks, 20,000 entries, 8 MiB of names over the entries and their sections; names past 1,024 bytes are cut)}. A file in another form (such as XML) is an error",
        always,
        read
    )]
}

/// An entry of a shortcut set: a block that sets `/Key`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KysEntry {
    /// The names of the blocks around it, outermost first (`["Menus"]`).
    pub section: Vec<String>,
    pub name: String,
    pub key: Option<i64>,
    pub modifiers: Option<i64>,
    pub context: Option<i64>,
    pub represent: Option<i64>,
    /// The line its name is on (from 1).
    pub line: usize,
}

/// A shortcut set read from text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KysSet {
    pub entries: Vec<KysEntry>,
    /// The tokens that weren't read, with their lines.
    pub unread: Vec<(usize, String)>,
    /// A cap stopped the reading, or cut a name.
    pub truncated: bool,
}

/// A token of the text.
enum Token {
    /// A slash name, its escapes undone.
    Name(Vec<u8>),
    /// An integer; `None` when it doesn't fit an `i64`.
    Int(Option<i64>),
    Open,
    Close,
    /// Anything else, as written.
    Other(Vec<u8>),
}

/// The tokens of `text`, each with its line.
struct Tokens<'a> {
    text: &'a [u8],
    at: usize,
    line: usize,
}

impl Tokens<'_> {
    fn next_token(&mut self) -> Option<(usize, Token)> {
        // Whitespace: LF, CR and CRLF each end a line.
        while let Some(&b) = self.text.get(self.at) {
            match b {
                b'\n' => self.line += 1,
                b'\r' if self.text.get(self.at + 1) != Some(&b'\n') => self.line += 1,
                b' ' | b'\t' | b'\r' | 0x0C | 0 => {}
                _ => break,
            }
            self.at += 1;
        }
        let line = self.line;
        let b = *self.text.get(self.at)?;
        let ends = |b: u8| b.is_ascii_whitespace() || matches!(b, b'{' | b'}' | b'/' | 0);
        let token = match b {
            b'{' => {
                self.at += 1;
                Token::Open
            }
            b'}' => {
                self.at += 1;
                Token::Close
            }
            b'/' => {
                self.at += 1;
                let mut name = vec![];
                while let Some(&c) = self.text.get(self.at) {
                    if c == b'\\' {
                        // The next character as it is (a backslash at the end of the text is dropped).
                        if let Some(&n) = self.text.get(self.at + 1) {
                            name.push(n);
                            if n == b'\n' || n == b'\r' && self.text.get(self.at + 2) != Some(&b'\n') {
                                self.line += 1;
                            }
                        }
                        self.at += 2;
                    } else if ends(c) {
                        break;
                    } else {
                        name.push(c);
                        self.at += 1;
                    }
                }
                Token::Name(name)
            }
            _ => {
                let start = self.at;
                while self.text.get(self.at).is_some_and(|&c| !ends(c)) {
                    self.at += 1;
                }
                // Always past at least one byte.
                self.at = self.at.max(start + 1);
                let word = self.text.get(start..self.at).unwrap_or_default();
                let number = word.strip_prefix(b"-").or_else(|| word.strip_prefix(b"+")).unwrap_or(word);
                if !number.is_empty() && number.iter().all(u8::is_ascii_digit) {
                    Token::Int(std::str::from_utf8(word).ok().and_then(|s| s.parse().ok()))
                } else {
                    Token::Other(word.to_vec())
                }
            }
        };
        Some((line, token))
    }
}

/// A block being read.
struct Block {
    name: String,
    line: usize,
    fields: [Option<i64>; 4],
}

/// The fields an entry's block sets, in [`Block::fields`] order.
const FIELDS: [&[u8]; 4] = [b"Key", b"Modifiers", b"Context", b"Represent"];

/// A name as text: at most [`MAX_NAME`] bytes, read as UTF-8 with replacement characters.
fn name_text(name: &[u8], set: &mut KysSet) -> String {
    if name.len() > MAX_NAME {
        set.truncated = true;
    }
    let mut s = String::from_utf8_lossy(name.get(..MAX_NAME).unwrap_or(name)).into_owned();
    // A character cut at the limit reads as one replacement character.
    if name.len() > MAX_NAME && s.ends_with('\u{FFFD}') {
        s.pop();
    }
    s
}

/// List a token that isn't read.
fn unread(set: &mut KysSet, line: usize, text: impl AsRef<[u8]>) {
    if set.unread.len() < MAX_UNREAD {
        let text: String = String::from_utf8_lossy(text.as_ref()).chars().take(UNREAD_SHOWN).collect();
        set.unread.push((line, text));
    }
}

/// Read a shortcut set saved as text. An error when the text isn't one: it starts with `<` (XML) or
/// doesn't start with a slash name and `{`.
pub fn read_kys(bytes: &[u8]) -> std::result::Result<KysSet, String> {
    let mut set = KysSet::default();
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = match text.get(..MAX_BYTES) {
        Some(head) if text.len() > MAX_BYTES => {
            set.truncated = true;
            head
        }
        _ => text,
    };
    let mut tokens = Tokens { text, at: 0, line: 1 };
    // The first two tokens show what the text is.
    let mut first = Tokens { text, at: 0, line: 1 };
    match (first.next_token(), first.next_token()) {
        (Some((_, Token::Name(_))), Some((_, Token::Open))) => {}
        (Some((_, Token::Other(w))), _) if w.starts_with(b"<") => return Err(NOT_READABLE.into()),
        _ => return Err("not a shortcut set: the text doesn't start with a slash name and an opening brace".into()),
    }
    let mut stack: Vec<Block> = vec![];
    // A slash name waiting for its `{` or its integer: (line, name).
    let mut pending: Option<(usize, Vec<u8>)> = None;
    // Blocks past [`MAX_DEPTH`] being skipped.
    let mut skipped = 0usize;
    // The bytes of names the entries keep so far ([`MAX_KEPT`]).
    let mut kept = 0usize;
    while let Some((line, token)) = tokens.next_token() {
        if skipped > 0 {
            match token {
                Token::Open => skipped += 1,
                Token::Close => skipped -= 1,
                _ => {}
            }
            continue;
        }
        match token {
            Token::Name(name) => {
                if let Some((l, n)) = pending.replace((line, name)) {
                    unread(&mut set, l, [b"/".as_slice(), &n].concat());
                }
            }
            Token::Int(value) => match pending.take() {
                Some((_, name)) if !stack.is_empty() && FIELDS.contains(&name.as_slice()) && value.is_some() => {
                    let i = FIELDS.iter().position(|f| *f == name.as_slice()).unwrap_or(0);
                    if let Some(slot) = stack.last_mut().and_then(|b| b.fields.get_mut(i)) {
                        *slot = value;
                    }
                }
                Some((l, name)) => {
                    let shown = match value {
                        Some(v) => format!("/{} {v}", String::from_utf8_lossy(&name)),
                        None => format!("/{} (a number too large)", String::from_utf8_lossy(&name)),
                    };
                    unread(&mut set, l, shown);
                }
                None => unread(&mut set, line, value.map_or_else(|| "(a number too large)".to_string(), |v| v.to_string())),
            },
            Token::Open => {
                let (l, name) = pending.take().unwrap_or((line, vec![]));
                if stack.len() >= MAX_DEPTH {
                    set.truncated = true;
                    unread(&mut set, l, format!("blocks open more than {MAX_DEPTH} deep"));
                    skipped = 1;
                    continue;
                }
                let name = name_text(&name, &mut set);
                stack.push(Block { name, line: l, fields: [None; 4] });
            }
            Token::Close => {
                if let Some((l, n)) = pending.take() {
                    unread(&mut set, l, [b"/".as_slice(), &n].concat());
                }
                let Some(block) = stack.pop() else {
                    unread(&mut set, line, "}");
                    continue;
                };
                let [key, modifiers, context, represent] = block.fields;
                if key.is_some() {
                    kept += block.name.len() + stack.iter().map(|b| b.name.len()).sum::<usize>();
                    if set.entries.len() >= MAX_ENTRIES || kept > MAX_KEPT {
                        set.truncated = true;
                        break;
                    }
                    let section = stack.iter().map(|b| b.name.clone()).collect();
                    set.entries.push(KysEntry { section, name: block.name, key, modifiers, context, represent, line: block.line });
                }
            }
            Token::Other(word) => {
                if let Some((l, n)) = pending.take() {
                    unread(&mut set, l, [b"/".as_slice(), &n].concat());
                }
                unread(&mut set, line, word);
            }
        }
    }
    if let Some((l, n)) = pending {
        unread(&mut set, l, [b"/".as_slice(), &n].concat());
    }
    for block in stack.iter().rev() {
        unread(&mut set, block.line, format!("/{} {{ (not closed)", block.name));
    }
    Ok(set)
}

/// Each code of `codes` with how many entries use it, the most used first.
fn counts(codes: impl Iterator<Item = i64>) -> Vec<(i64, usize)> {
    let mut by: std::collections::BTreeMap<i64, usize> = std::collections::BTreeMap::new();
    for c in codes {
        *by.entry(c).or_default() += 1;
    }
    let mut v: Vec<(i64, usize)> = by.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// The shortcut text for `mods` (`Cmd+Shift`, or "" for none) and `key`, as Edit › Keyboard
/// Shortcuts writes it; none when the key has no equivalent.
fn chord(mods: &str, key: &str) -> Option<String> {
    match (mods, key) {
        (_, "") => None,
        ("", k) => Some(k.to_string()),
        (m, k) => Some(format!("{m}+{k}")),
    }
}

/// The bytes base64 text `b64` holds, up to [`READ_LIMIT`] and a few more: only the text they take is
/// decoded.
fn decode_head(b64: &str) -> Option<Vec<u8>> {
    let chars = READ_LIMIT.div_ceil(3) * 4;
    let mut kept = 0usize;
    let end = b64
        .bytes()
        .position(|b| {
            kept += usize::from(!b.is_ascii_whitespace());
            kept > chars
        })
        .unwrap_or(b64.len());
    vectorcraft_format::base64_decode(b64.get(..end)?)
}

fn read(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "shortcutSet.read";
    let bytes: std::borrow::Cow<[u8]> = match (str_param(p, "path"), str_param(p, "data"), str_param(p, "dataBase64")) {
        (Some(path), ..) => super::fileio::read_file_head(path, READ_LIMIT)?.into(),
        (None, Some(text), _) => text.as_bytes().get(..READ_LIMIT).unwrap_or(text.as_bytes()).into(),
        (None, None, Some(b64)) => decode_head(b64).ok_or_else(|| bad(C, "bad dataBase64"))?.into(),
        _ => return Err(bad(C, "give `path`, `data` or `dataBase64`")),
    };
    let set = read_kys(&bytes).map_err(EngineError::Other)?;
    let file = str_param(p, "name").or_else(|| str_param(p, "path")).unwrap_or_default();
    let name = std::path::Path::new(file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let targets = importmatches::targets(&s.prefs);
    let saved = |kind: &str, name: &str| targets.get(&(kind, name)).copied();
    let code = |kind: &str, c: i64| saved(kind, &c.to_string());
    let entries: Vec<Value> = set
        .entries
        .iter()
        .map(|e| {
            let mut v = json!({
                "section": e.section, "name": e.name, "key": e.key, "modifiers": e.modifiers,
                "context": e.context, "represent": e.represent, "line": e.line,
            });
            if let Some(t) = saved("name", &e.name) {
                v["target"] = json!(t);
            }
            let mods = match e.modifiers {
                Some(m) => code("modifiers", m),
                None => Some(""),
            };
            if let Some(c) = e.key.and_then(|k| code("key", k)).zip(mods).and_then(|(k, m)| chord(m, k)) {
                v["chord"] = json!(c);
            }
            v
        })
        .collect();
    let rows = |codes: Vec<(i64, usize)>, kind: &str, field: &str| -> Vec<Value> {
        codes
            .into_iter()
            .map(|(value, count)| {
                let mut v = json!({ "value": value, "count": count });
                if let Some(t) = code(kind, value) {
                    v[field] = json!(t);
                }
                v
            })
            .collect()
    };
    Ok(json!({
        "set": name,
        "entries": entries,
        "codes": {
            "keys": rows(counts(set.entries.iter().filter_map(|e| e.key)), "key", "key"),
            "modifiers": rows(counts(set.entries.iter().filter_map(|e| e.modifiers)), "modifiers", "mods"),
        },
        "unread": set.unread.iter().map(|(line, text)| json!({ "line": line, "text": text })).collect::<Vec<_>>(),
        "truncated": set.truncated,
    }))
}
