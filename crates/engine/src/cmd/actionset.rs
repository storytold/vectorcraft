//! Action sets another app saves as text (`.aia` files that start `/version`), read into their
//! actions and steps without changing anything (`actionSet.read`).
//!
//! The text is a set of fields and blocks. A slash name followed by a value sets a field, and a
//! slash name followed by `{` opens a block that `}` closes. A value is a decimal number, text in
//! parentheses, a byte count and hexadecimal bytes in square brackets (`[ 5 48656c6c6f ]`, UTF-8
//! text, which can run over several words), or a byte count and hexadecimal bytes in angle brackets
//! (`< 3 0a0b0c >`, raw bytes). The set's `/version` comes first, and its `/name` is hexadecimal
//! text. Each action is an `/action-N` block with a hexadecimal `/name`; each step is an `/event-N`
//! block inside one, with its `/internalName` in parentheses, the `/localizedName` it is shown with
//! as hexadecimal text, and `/isOn` 0 when it is left out of playback; each value a step was
//! recorded with is a `/parameter-N` block inside it, which is counted, and whose hexadecimal
//! `/value` is checked against its byte count. The set's `/actionCount`, an action's `/eventCount`
//! and a step's `/parameterCount` give how many actions, steps and recorded values follow.
//! The layout follows the README of Inventsable's Redesigning-Illustrator-Actions-in-Scripting
//! (https://github.com/Inventsable/Redesigning-Illustrator-Actions-in-Scripting), the forum post
//! "Made a two-way aia to json converter"
//! (https://community.adobe.com/t5/illustrator-discussions/made-a-two-way-aia-to-json-converter/m-p/13751909),
//! sttk3's article on names as UTF-8 hexadecimal
//! (https://sttk3.com/blog/tips/illustrator/dynamic-generate-action.html), and the MIT-licensed
//! reader in zachshallbetter/adobe-mcp (https://github.com/zachshallbetter/adobe-mcp). The raw form
//! follows the forum thread "Creating a dynamic action"
//! (https://community.adobe.com/questions-652/creating-a-dynamic-action-to-use-with-app-doscript-method-814406).

use serde_json::{Value, json};

use super::{CommandSpec, EngineError, Result, Session, always, bad, cmd, text_param};

/// The most bytes of a file read.
const MAX_BYTES: usize = 8 << 20;
/// The most bytes taken in: [`MAX_BYTES`] after a byte order mark, and one more that shows the text
/// goes on.
const READ_LIMIT: usize = MAX_BYTES + 4;
/// The most blocks open at once.
const MAX_DEPTH: usize = 16;
/// The most actions read.
const MAX_ACTIONS: usize = 1000;
/// The most steps read in one action.
const MAX_STEPS: usize = 10_000;
/// The most steps read in the set: a small text can hold hundreds of thousands of empty steps, and
/// the answer holds an object for each.
const MAX_SET_STEPS: usize = 100_000;
/// The longest name kept (bytes).
const MAX_NAME: usize = 1024;
/// The most tokens listed as unread.
const MAX_UNREAD: usize = 1000;
/// The longest unread token shown (characters).
const UNREAD_SHOWN: usize = 80;

/// The extensions of action sets other apps save.
pub const EXTS: &[&str] = &["aia"];

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "actionSet.read",
        "Read Action Set File",
        [],
        None,
        "{path? | data?: the file's text | dataBase64?, name?: the file's name (default: the path's)} read an action set another app saved as text (.aia, starting \"/version\") without changing anything → {set: the set's /name (else the file's name without its extension, else empty), version, actions: [{name, steps: [{name: the step's /internalName, label: its /localizedName, or null when it has none or it is blank, on: false when /isOn is 0, params: how many /parameter blocks it has, line}]}], unread: [{line, text}] (tokens that aren't read, at most 1000), truncated: true when a cap stopped the reading (8 MiB, 16 open blocks, 1,000 actions, 10,000 steps in an action, 100,000 steps in the set; names past 1,024 bytes are cut), when the text ends inside a block or a value or after a field's name, when /actionCount, /eventCount or a step's /parameterCount differs from the actions, steps or recorded values read (a negative count, one too large for a number and one that isn't a number always differ), or when a hexadecimal name, label or recorded /value isn't as many bytes as its count says}. Give exactly one of path, data and dataBase64. Text that doesn't start with /version and a number is an error",
        always,
        read
    )]
}

/// A step of an action: an `/event-N` block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiaStep {
    /// Its `/internalName`.
    pub name: String,
    /// Its `/localizedName`, if it has one.
    pub label: Option<String>,
    /// Its `/isOn`, if it has one.
    pub on: Option<i64>,
    /// How many `/parameter-N` blocks it has.
    pub params: usize,
    /// Its `/parameterCount`, if it has one.
    pub declared: Option<i64>,
    /// The line its block opens on (from 1).
    pub line: usize,
}

/// An action: an `/action-N` block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiaAction {
    pub name: String,
    pub steps: Vec<AiaStep>,
    /// Its `/eventCount`, if it has one.
    pub declared: Option<i64>,
}

/// An action set read from text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiaSet {
    pub version: i64,
    /// The set's `/name`, if it has one.
    pub name: Option<String>,
    pub actions: Vec<AiaAction>,
    /// Its `/actionCount`, if it has one.
    pub declared: Option<i64>,
    /// The tokens that weren't read, with their lines.
    pub unread: Vec<(usize, String)>,
    /// A cap stopped the reading or cut a name, the text ends inside a block or a value or after a
    /// field's name, its counts differ from the actions, steps and recorded values read (a negative
    /// count, one too large for an `i64` and one that isn't a number always differ), or a hexadecimal
    /// name, label or recorded `/value` isn't as many bytes as its count says.
    pub truncated: bool,
    /// How many steps were read in all the actions.
    steps: usize,
}

/// A token of the text.
enum Token {
    /// A slash name.
    Name(Vec<u8>),
    /// An integer; `None` when it doesn't fit an `i64`.
    Int(Option<i64>),
    /// Text in parentheses, a backslash taking the next byte as it is.
    Text(Vec<u8>),
    /// The bytes of `[ count hex… ]`; `None` when the hexadecimal digits don't make whole bytes or
    /// as many bytes as the count says.
    Hex(Option<Vec<u8>>),
    /// `< count hex… >`.
    Raw,
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
    /// The text ended inside a value in parentheses, square brackets or angle brackets.
    unclosed: bool,
}

impl<'a> Tokens<'a> {
    /// Past one byte, counting a line at LF, at CR alone, and once for CRLF.
    fn step(&mut self) -> Option<u8> {
        let b = *self.text.get(self.at)?;
        if b == b'\n' || b == b'\r' && self.text.get(self.at + 1) != Some(&b'\n') {
            self.line += 1;
        }
        self.at += 1;
        Some(b)
    }

    /// The bytes up to `end` (not taken in); `None` when the text ends first.
    fn until(&mut self, end: u8) -> Option<&'a [u8]> {
        let (text, start) = (self.text, self.at);
        while text.get(self.at).is_some_and(|&b| b != end) {
            self.step();
        }
        let inside = text.get(start..self.at)?;
        text.get(self.at)?;
        self.at += 1;
        Some(inside)
    }

    fn next_token(&mut self) -> Option<(usize, Token)> {
        while self.text.get(self.at).is_some_and(|&b| b.is_ascii_whitespace() || b == 0) {
            self.step();
        }
        let line = self.line;
        let b = *self.text.get(self.at)?;
        let ends = |b: u8| b.is_ascii_whitespace() || b == 0 || b"{}[]()<>/".contains(&b);
        let token = match b {
            b'{' | b'}' => {
                self.at += 1;
                if b == b'{' { Token::Open } else { Token::Close }
            }
            b'(' => {
                self.at += 1;
                let (mut text, mut depth) = (vec![], 0usize);
                self.unclosed = true;
                while let Some(c) = self.step() {
                    match c {
                        b'\\' => text.extend(self.step()),
                        b'(' => {
                            depth += 1;
                            text.push(c);
                        }
                        b')' if depth == 0 => {
                            self.unclosed = false;
                            break;
                        }
                        b')' => {
                            depth -= 1;
                            text.push(c);
                        }
                        _ => text.push(c),
                    }
                }
                Token::Text(text)
            }
            b'[' | b'<' => {
                self.at += 1;
                let close = if b == b'[' { b']' } else { b'>' };
                let inside = self.until(close);
                self.unclosed = inside.is_none();
                let inside = inside.unwrap_or_default().to_vec();
                if b == b'<' {
                    Token::Raw
                } else {
                    // The first word is the byte count; the bytes follow in one or more words.
                    let mut words = inside.split(|c| c.is_ascii_whitespace()).filter(|w| !w.is_empty());
                    let count = words.next().and_then(|w| std::str::from_utf8(w).ok()).and_then(|w| w.parse::<usize>().ok());
                    let digits: Vec<u8> = words.flatten().copied().collect();
                    Token::Hex(hex_bytes(&digits).filter(|b| Some(b.len()) == count))
                }
            }
            b'/' => {
                self.at += 1;
                let start = self.at;
                while self.text.get(self.at).is_some_and(|&c| !ends(c)) {
                    self.at += 1;
                }
                Token::Name(self.text.get(start..self.at).unwrap_or_default().to_vec())
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

/// The bytes hexadecimal `digits` stand for, two digits a byte.
fn hex_bytes(digits: &[u8]) -> Option<Vec<u8>> {
    let nibble = |c: u8| (c as char).to_digit(16).and_then(|d| u8::try_from(d).ok());
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    digits.chunks(2).map(|pair| Some((nibble(*pair.first()?)? << 4) | nibble(*pair.get(1)?)?)).collect()
}

/// What a block is.
enum Kind {
    Action,
    Step,
    Param,
    /// A block that isn't read, or one past a cap.
    Other,
}

/// `name` ends with `-` and a number after `prefix`, as `/action-1` does.
fn numbered(name: &[u8], prefix: &[u8]) -> bool {
    name.strip_prefix(prefix).is_some_and(|n| !n.is_empty() && n.iter().all(u8::is_ascii_digit))
}

/// A name as text: at most [`MAX_NAME`] bytes, read as UTF-8 with replacement characters.
fn name_text(name: &[u8], set: &mut AiaSet) -> String {
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

/// How the unread list shows a value in square brackets that isn't as many bytes as its count says.
const BAD_HEX: &str = "[ … ] (not the hexadecimal bytes its count says)";

/// List a token that isn't read.
fn unread(set: &mut AiaSet, line: usize, text: impl AsRef<[u8]>) {
    if set.unread.len() < MAX_UNREAD {
        let text: String = String::from_utf8_lossy(text.as_ref()).chars().take(UNREAD_SHOWN).collect();
        set.unread.push((line, text));
    }
}

/// What the block `name` that opens on `line` inside the blocks `stack` is: a new action, step or
/// recorded value of `set`, or a block that isn't read (listed when it is out of place).
fn open_block(set: &mut AiaSet, stack: &[Kind], name: &[u8], line: usize) -> Kind {
    match stack.last() {
        _ if stack.len() >= MAX_DEPTH => {
            set.truncated = true;
            Kind::Other
        }
        None if numbered(name, b"action-") => {
            if set.actions.len() < MAX_ACTIONS {
                set.actions.push(AiaAction::default());
                Kind::Action
            } else {
                set.truncated = true;
                Kind::Other
            }
        }
        Some(Kind::Action) if numbered(name, b"event-") => match set.actions.last_mut() {
            Some(a) if a.steps.len() < MAX_STEPS && set.steps < MAX_SET_STEPS => {
                a.steps.push(AiaStep { line, ..AiaStep::default() });
                set.steps += 1;
                Kind::Step
            }
            _ => {
                set.truncated = true;
                Kind::Other
            }
        },
        Some(Kind::Step) if numbered(name, b"parameter-") => {
            if let Some(s) = set.actions.last_mut().and_then(|a| a.steps.last_mut()) {
                s.params += 1;
            }
            Kind::Param
        }
        Some(Kind::Param | Kind::Other) => Kind::Other,
        _ => {
            unread(set, line, format!("/{} {{", String::from_utf8_lossy(name)));
            Kind::Other
        }
    }
}

/// Set the field `name` of the block `kind` (the set when `None`) to the hexadecimal text `bytes`
/// on `line`: the set's or an action's `/name`, or a step's `/localizedName`.
fn set_hex(set: &mut AiaSet, kind: Option<&Kind>, name: Option<&[u8]>, bytes: Option<Vec<u8>>, line: usize) {
    let Some(bytes) = bytes else {
        set.truncated = true;
        unread(set, line, BAD_HEX);
        return;
    };
    match (kind, name) {
        (None, Some(b"name")) => set.name = Some(name_text(&bytes, set)),
        (Some(Kind::Action), Some(b"name")) => {
            let text = name_text(&bytes, set);
            if let Some(a) = set.actions.last_mut() {
                a.name = text;
            }
        }
        (Some(Kind::Step), Some(b"localizedName")) => {
            let text = name_text(&bytes, set);
            if let Some(s) = set.actions.last_mut().and_then(|a| a.steps.last_mut()) {
                s.label = Some(text);
            }
        }
        (_, Some(_)) => {}
        (_, None) => unread(set, line, "[ … ]"),
    }
}

/// Mark `set` truncated when the hexadecimal `/value` on `line` isn't as many bytes as its count
/// says.
fn check_value(set: &mut AiaSet, bytes: Option<Vec<u8>>, line: usize) {
    if bytes.is_none() {
        set.truncated = true;
        unread(set, line, BAD_HEX);
    }
}

/// Mark `set` truncated when the field `name` of the block `kind` (the set when `None`) is a count
/// (the set's `/actionCount`, an action's `/eventCount` or a step's `/parameterCount`) whose value is
/// not a number that fits an `i64`.
fn bad_count(set: &mut AiaSet, kind: Option<&Kind>, name: Option<&[u8]>) {
    if matches!((kind, name), (None, Some(b"actionCount")) | (Some(Kind::Action), Some(b"eventCount")) | (Some(Kind::Step), Some(b"parameterCount")))
    {
        set.truncated = true;
    }
}

/// Set the field `name` of the block `kind` (the set when `None`) to the integer `value` on `line`:
/// the set's `/actionCount`, an action's `/eventCount`, a step's `/parameterCount`, or a step's
/// `/isOn`. A count too large for an `i64` marks the set truncated.
fn set_int(set: &mut AiaSet, kind: Option<&Kind>, name: Option<&[u8]>, value: Option<i64>, line: usize) {
    if value.is_none() {
        bad_count(set, kind, name);
    }
    match (kind, name) {
        (None, Some(b"actionCount")) => set.declared = value,
        (Some(Kind::Action), Some(b"eventCount")) => {
            if let Some(a) = set.actions.last_mut() {
                a.declared = value;
            }
        }
        (Some(Kind::Step), Some(b"parameterCount")) => {
            if let Some(s) = set.actions.last_mut().and_then(|a| a.steps.last_mut()) {
                s.declared = value;
            }
        }
        (Some(Kind::Step), Some(b"isOn")) => {
            if let Some(s) = set.actions.last_mut().and_then(|a| a.steps.last_mut()) {
                s.on = value;
            }
        }
        (_, Some(_)) => {}
        (_, None) => unread(set, line, value.map_or_else(|| "(a number too large)".to_string(), |v| v.to_string())),
    }
}

/// Read an action set saved as text. An error when the text isn't one: it doesn't start with
/// `/version` and an integer.
pub fn read_aia(bytes: &[u8]) -> std::result::Result<AiaSet, String> {
    let mut set = AiaSet::default();
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = match text.get(..MAX_BYTES) {
        Some(head) if text.len() > MAX_BYTES => {
            set.truncated = true;
            head
        }
        _ => text,
    };
    let mut tokens = Tokens { text, at: 0, line: 1, unclosed: false };
    match (tokens.next_token(), tokens.next_token()) {
        (Some((_, Token::Name(n))), Some((_, Token::Int(Some(v))))) if n == b"version" => set.version = v,
        _ => return Err("not an action set: the text doesn't start with /version and a number".into()),
    }
    let mut stack: Vec<Kind> = vec![];
    // A slash name waiting for its value or its `{`: (line, name).
    let mut pending: Option<(usize, Vec<u8>)> = None;
    while let Some((line, token)) = tokens.next_token() {
        let in_step = matches!(stack.last(), Some(Kind::Step));
        let field = |pending: &mut Option<(usize, Vec<u8>)>| pending.take().map(|(_, n)| n);
        match token {
            Token::Name(name) => {
                if let Some((l, n)) = pending.replace((line, name)) {
                    bad_count(&mut set, stack.last(), Some(&n));
                    unread(&mut set, l, [b"/".as_slice(), &n].concat());
                }
            }
            Token::Open => {
                let (l, name) = pending.take().unwrap_or((line, vec![]));
                bad_count(&mut set, stack.last(), Some(&name));
                let kind = open_block(&mut set, &stack, &name, l);
                stack.push(kind);
            }
            Token::Close => {
                if let Some((l, n)) = pending.take() {
                    bad_count(&mut set, stack.last(), Some(&n));
                    unread(&mut set, l, [b"/".as_slice(), &n].concat());
                }
                if stack.pop().is_none() {
                    unread(&mut set, line, "}");
                }
            }
            Token::Hex(bytes) if matches!(stack.last(), Some(Kind::Param)) => {
                if field(&mut pending).as_deref() == Some(b"value".as_slice()) {
                    check_value(&mut set, bytes, line);
                }
            }
            // The other values inside a recorded value, or a block that isn't read.
            _ if matches!(stack.last(), Some(Kind::Param | Kind::Other)) => {
                pending = None;
            }
            Token::Hex(bytes) => {
                let name = field(&mut pending);
                bad_count(&mut set, stack.last(), name.as_deref());
                set_hex(&mut set, stack.last(), name.as_deref(), bytes, line);
            }
            Token::Text(t) => {
                let name = field(&mut pending);
                bad_count(&mut set, stack.last(), name.as_deref());
                match name {
                    Some(n) if in_step && n == b"internalName" => {
                        let name = name_text(&t, &mut set);
                        if let Some(s) = set.actions.last_mut().and_then(|a| a.steps.last_mut()) {
                            s.name = name;
                        }
                    }
                    Some(_) => {}
                    None => unread(&mut set, line, [b"(".as_slice(), &t, b")"].concat()),
                }
            }
            Token::Int(v) => {
                let name = field(&mut pending);
                set_int(&mut set, stack.last(), name.as_deref(), v, line);
            }
            Token::Raw => {
                let name = field(&mut pending);
                bad_count(&mut set, stack.last(), name.as_deref());
                if name.is_none() {
                    unread(&mut set, line, "< … >");
                }
            }
            Token::Other(word) => {
                // A real number, or another word as a field's value.
                let name = field(&mut pending);
                bad_count(&mut set, stack.last(), name.as_deref());
                if name.is_none() {
                    unread(&mut set, line, word);
                }
            }
        }
    }
    // The text ends after a field's name, before its value.
    if let Some((l, n)) = pending {
        set.truncated = true;
        unread(&mut set, l, [b"/".as_slice(), &n].concat());
    }
    // An unclosed value is the last token: it takes the rest of the text.
    if tokens.unclosed {
        set.truncated = true;
        unread(&mut set, tokens.line, "a value not closed at the end");
    }
    if !stack.is_empty() {
        set.truncated = true;
        unread(&mut set, tokens.line, format!("blocks left open at the end: {}", stack.len()));
    }
    // Not as many actions, steps or recorded values as the counts give; a negative count never matches.
    let differs = |declared: Option<i64>, read: usize| declared.is_some_and(|d| !usize::try_from(d).is_ok_and(|d| d == read));
    let step_differs = |a: &AiaAction| a.steps.iter().any(|s| differs(s.declared, s.params));
    if differs(set.declared, set.actions.len()) || set.actions.iter().any(|a| differs(a.declared, a.steps.len()) || step_differs(a)) {
        set.truncated = true;
        unread(&mut set, tokens.line, "not as many actions, steps or recorded values as the counts give");
    }
    Ok(set)
}

fn read(_: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "actionSet.read";
    let (path, data, b64) = (text_param(p, "path", C)?, text_param(p, "data", C)?, text_param(p, "dataBase64", C)?);
    let name = text_param(p, "name", C)?;
    let bytes: std::borrow::Cow<[u8]> = match (path, data, b64) {
        (Some(path), None, None) => super::fileio::read_file_head(path, READ_LIMIT)?.into(),
        (None, Some(text), None) => text.as_bytes().get(..READ_LIMIT).unwrap_or(text.as_bytes()).into(),
        (None, None, Some(b64)) => super::scripts::decode_head(b64, READ_LIMIT).ok_or_else(|| bad(C, "bad dataBase64"))?.into(),
        _ => return Err(bad(C, "give exactly one of `path`, `data` and `dataBase64`")),
    };
    let set = read_aia(&bytes).map_err(EngineError::Other)?;
    let file = name.or(path).unwrap_or_default();
    let stem = std::path::Path::new(file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = set.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or(stem);
    let actions: Vec<Value> = set
        .actions
        .iter()
        .map(|a| {
            let steps: Vec<Value> = a
                .steps
                .iter()
                .map(|s| {
                    let label = s.label.clone().filter(|l| !l.trim().is_empty());
                    json!({ "name": s.name, "label": label, "on": s.on != Some(0), "params": s.params, "line": s.line })
                })
                .collect();
            json!({ "name": a.name, "steps": steps })
        })
        .collect();
    Ok(json!({
        "set": name,
        "version": set.version,
        "actions": actions,
        "unread": set.unread.iter().map(|(line, text)| json!({ "line": line, "text": text })).collect::<Vec<_>>(),
        "truncated": set.truncated,
    }))
}
