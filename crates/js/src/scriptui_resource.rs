//! The reader of ScriptUI resource specifications, as the help page
//! <https://extendscript.docsforadobe.dev/user-interface-tools/resource-specifications/> describes
//! them: a type, then braces holding property definitions, where a property whose value is a
//! control class followed by braces defines a control the container holds.
//!
//! `[name:] Type { name: value, ... }` is read into a [`Spec`]. A value is `null`, `true`, `false`,
//! a string in single or double quotes, a number, an inline array `[value, ...]`, an inline object
//! `{name: value, ...}`, or, directly in a window's or control's braces, a control `Class { ... }`.
//! A name is an identifier or a quoted string. The commas between properties may be left out, and
//! a comma may end a list. A string longer than [`MAX_BYTES`] or values nested more than
//! [`MAX_NESTING`] deep are refused; these are VectorCraft's own limits.

/// The most bytes (UTF-8) of a resource specification.
pub const MAX_BYTES: usize = 1 << 20;
/// How deeply braces and brackets nest in a resource specification.
pub const MAX_NESTING: usize = 64;

/// A value in a resource specification.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Res {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Res>),
    Object(Vec<(String, Res)>),
    Control(Spec),
}

/// A window or control: its class as written and its properties in order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Spec {
    pub(crate) class: String,
    pub(crate) props: Vec<(String, Res)>,
}

/// Why a resource specification could not be read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fault {
    /// The text is not a resource specification; the message says where.
    Syntax(String),
    /// The text passes [`MAX_BYTES`] or [`MAX_NESTING`].
    Limit(String),
}

/// Reads `text`, which is `[name:] Type { ... }` with nothing after it but white space.
pub(crate) fn parse(text: &str) -> Result<Spec, Fault> {
    if text.len() > MAX_BYTES {
        return Err(Fault::Limit(format!("a resource specification is longer than {} MiB", MAX_BYTES >> 20)));
    }
    let mut r = Reader { chars: text.chars().collect(), at: 0 };
    r.space();
    let quoted = matches!(r.peek(), Some('\'' | '"'));
    let first = r.name()?;
    r.space();
    let class = if r.eat(':') {
        r.space();
        r.ident()?
    } else if quoted {
        return Err(r.expected(": after the name"));
    } else {
        first
    };
    r.space();
    if !r.eat('{') {
        return Err(r.expected("{ after the type"));
    }
    let props = r.props(true, 1)?;
    r.space();
    if r.at < r.chars.len() {
        return Err(r.expected("the end of the resource specification"));
    }
    Ok(Spec { class, props })
}

struct Reader {
    chars: Vec<char>,
    at: usize,
}

impl Reader {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    fn expected(&self, what: &str) -> Fault {
        match self.peek() {
            Some(c) => Fault::Syntax(format!("expected {what} at character {}, found {c:?}", self.at + 1)),
            None => Fault::Syntax(format!("expected {what} at character {}, the end of the resource specification", self.at + 1)),
        }
    }

    fn ident(&mut self) -> Result<String, Fault> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$') {
            self.at += 1;
        }
        if self.at == start || self.chars[start].is_ascii_digit() {
            self.at = start;
            return Err(self.expected("a name"));
        }
        Ok(self.chars[start..self.at].iter().collect())
    }

    /// A property name: an identifier or a quoted string.
    fn name(&mut self) -> Result<String, Fault> {
        match self.peek() {
            Some('\'' | '"') => self.string(),
            _ => self.ident(),
        }
    }

    /// The properties after an opening brace, up to and including its closing brace. Controls are
    /// allowed among them when `controls`.
    fn props(&mut self, controls: bool, depth: usize) -> Result<Vec<(String, Res)>, Fault> {
        if depth > MAX_NESTING {
            return Err(Fault::Limit(format!("a resource specification nests more than {MAX_NESTING} deep")));
        }
        let mut props = Vec::new();
        loop {
            self.space();
            if self.eat('}') {
                return Ok(props);
            }
            let name = self.name()?;
            self.space();
            if !self.eat(':') {
                return Err(self.expected(": after a property name"));
            }
            self.space();
            let value = self.value(controls, depth)?;
            props.push((name, value));
            self.space();
            self.eat(',');
        }
    }

    fn value(&mut self, controls: bool, depth: usize) -> Result<Res, Fault> {
        match self.peek() {
            Some('\'' | '"') => Ok(Res::String(self.string()?)),
            Some('[') => {
                self.at += 1;
                if depth + 1 > MAX_NESTING {
                    return Err(Fault::Limit(format!("a resource specification nests more than {MAX_NESTING} deep")));
                }
                let mut items = Vec::new();
                loop {
                    self.space();
                    if self.eat(']') {
                        return Ok(Res::Array(items));
                    }
                    items.push(self.value(false, depth + 1)?);
                    self.space();
                    if !self.eat(',') && self.peek() != Some(']') {
                        return Err(self.expected(", or ] in an array"));
                    }
                }
            }
            Some('{') => {
                self.at += 1;
                Ok(Res::Object(self.props(false, depth + 1)?))
            }
            Some(c) if c.is_ascii_digit() || matches!(c, '-' | '+' | '.') => self.number(),
            Some(_) => {
                let start = self.at;
                let word = self.ident()?;
                match word.as_str() {
                    "null" => return Ok(Res::Null),
                    "true" => return Ok(Res::Bool(true)),
                    "false" => return Ok(Res::Bool(false)),
                    _ => {}
                }
                self.space();
                if self.peek() != Some('{') {
                    self.at = start;
                    return Err(self.expected("a value"));
                }
                if !controls {
                    self.at = start;
                    return Err(self.expected("a value (a control can only be a property of a window or a container)"));
                }
                self.at += 1;
                Ok(Res::Control(Spec { class: word, props: self.props(true, depth + 1)? }))
            }
            None => Err(self.expected("a value")),
        }
    }

    fn number(&mut self) -> Result<Res, Fault> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '.')) {
            self.at += 1;
        }
        let text: String = self.chars[start..self.at].iter().collect();
        let plain = text.strip_prefix(['-', '+']).unwrap_or(&text);
        // Rust also reads words such as inf and nan, which are not numbers here.
        let n = if plain.starts_with(|c: char| c.is_ascii_digit() || c == '.') { text.parse::<f64>().ok() } else { None };
        match n {
            Some(n) if n.is_finite() => Ok(Res::Number(n)),
            _ => {
                self.at = start;
                Err(self.expected("a number"))
            }
        }
    }

    fn string(&mut self) -> Result<String, Fault> {
        let Some(quote) = self.peek() else { return Err(self.expected("a string")) };
        let start = self.at;
        self.at += 1;
        let mut s = String::new();
        loop {
            let Some(c) = self.peek() else {
                self.at = start;
                return Err(Fault::Syntax(format!("the string at character {} has no closing quote", start + 1)));
            };
            self.at += 1;
            if c == quote {
                return Ok(s);
            }
            if c != '\\' {
                s.push(c);
                continue;
            }
            let Some(e) = self.peek() else { continue };
            self.at += 1;
            match e {
                'n' => s.push('\n'),
                't' => s.push('\t'),
                'r' => s.push('\r'),
                'b' => s.push('\u{8}'),
                'f' => s.push('\u{c}'),
                'v' => s.push('\u{b}'),
                '0' => s.push('\0'),
                '\n' => {}
                'u' | 'x' => {
                    let digits = if e == 'u' { 4 } else { 2 };
                    let hex: String = self.chars.iter().skip(self.at).take(digits).collect();
                    // `from_str_radix` also takes a sign, which an escape does not have.
                    let valid = hex.len() == digits && hex.chars().all(|c| c.is_ascii_hexdigit());
                    match u32::from_str_radix(&hex, 16).ok().filter(|_| valid).and_then(char::from_u32) {
                        Some(c) => {
                            s.push(c);
                            self.at += digits;
                        }
                        None => return Err(Fault::Syntax(format!("the escape at character {} is not valid", self.at - 1))),
                    }
                }
                e => s.push(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(class: &str, props: Vec<(&str, Res)>) -> Spec {
        Spec { class: class.into(), props: props.into_iter().map(|(k, v)| (k.into(), v)).collect() }
    }

    #[test]
    fn a_resource_specification_reads_its_controls_and_values() {
        let got = parse(
            "p: Panel { text: 'Messages', alignChildren: ['right', \"top\"], \
             g: Group { st: StaticText { text: 'a\\'b' } } \
             e: EditText { properties: {multiline: true, name: null}, characters: -3.5e1, }, }",
        );
        let want = spec(
            "Panel",
            vec![
                ("text", Res::String("Messages".into())),
                ("alignChildren", Res::Array(vec![Res::String("right".into()), Res::String("top".into())])),
                ("g", Res::Control(spec("Group", vec![("st", Res::Control(spec("StaticText", vec![("text", Res::String("a'b".into()))])))]))),
                (
                    "e",
                    Res::Control(spec(
                        "EditText",
                        vec![
                            ("properties", Res::Object(vec![("multiline".into(), Res::Bool(true)), ("name".into(), Res::Null)])),
                            ("characters", Res::Number(-35.0)),
                        ],
                    )),
                ),
            ],
        );
        assert_eq!(got, Ok(want));
        assert_eq!(parse("  dialog{}  "), Ok(spec("dialog", vec![])));
    }

    #[test]
    fn a_malformed_resource_specification_is_a_syntax_fault() {
        for text in [
            "dialog",
            "dialog {",
            "dialog { b: }",
            "dialog { b: Button { } } x",
            "dialog { b Button {} }",
            "dialog { t: 'open }",
            "dialog { n: inf }",
            "dialog { n: 1.2.3 }",
            "dialog { a: [1 2] }",
            "dialog { o: { b: Button {} } }",
            "dialog { a: [Button {}] }",
            "dialog { v: orange }",
            "dialog { t: '\\u12' }",
            "dialog { t: '\\x+A' }",
            "dialog { t: '\\u+00A' }",
            "{ }",
        ] {
            assert!(matches!(parse(text), Err(Fault::Syntax(_))), "{text}: {:?}", parse(text));
        }
        assert_eq!(parse("dialog { t: '\\x0A\\u0041' }"), Ok(spec("dialog", vec![("t", Res::String("\nA".into()))])));
    }

    #[test]
    fn a_resource_specification_that_ends_early_says_at_which_character() {
        assert_eq!(parse("dialog {"), Err(Fault::Syntax("expected a name at character 9, the end of the resource specification".into())));
        assert_eq!(parse("dialog"), Err(Fault::Syntax("expected { after the type at character 7, the end of the resource specification".into())));
    }

    #[test]
    fn a_resource_specification_past_the_limits_is_refused() {
        let deep = format!("dialog {{ a: {}1{} }}", "[".repeat(MAX_NESTING), "]".repeat(MAX_NESTING));
        assert!(matches!(parse(&deep), Err(Fault::Limit(_))), "{:?}", parse(&deep));
        let fits = format!("dialog {{ a: {}1{} }}", "[".repeat(MAX_NESTING - 1), "]".repeat(MAX_NESTING - 1));
        assert!(parse(&fits).is_ok());
        let long = format!("dialog {{ t: '{}' }}", "x".repeat(MAX_BYTES));
        assert!(matches!(parse(&long), Err(Fault::Limit(_))));
    }
}
