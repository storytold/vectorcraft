//! Machine-readable parameter schemas for VectorCraft commands. A [`ParamSchema`] lists what each
//! parameter of a command is ([`Param`], [`Type`]): its type, whether it is required, what an
//! omitted value means, whether null counts as omitted, and the API levels it was added and
//! deprecated at. From one schema come:
//!
//! - [`ParamSchema::validate`], which checks a params object and names the parameter of the first
//!   problem it finds ([`ParamError`]), and fills in the defaults of omitted values;
//! - [`ParamSchema::definition_problem`], which checks the schema itself;
//! - [`ParamSchema::params_text`], the params text a command lists;
//! - [`ParamSchema::to_json`], the schema as a JSON object in the shape of a JSON Schema 2020-12
//!   object schema (<https://json-schema.org/draft/2020-12>), with VectorCraft's own fields as
//!   `x-` keywords;
//! - [`hash`], a hash of a set of command schemas, which changes whenever any of them changes.
//!
//! Checking a value walks the schema, so its depth is the schema's, at most [`MAX_DEPTH`]. An error
//! shows a bounded part of a refused value and of a refused key, and the value is not written out
//! first, so a huge or deeply nested value costs no more to refuse than a small one.
//!
//! The crate knows no command and depends on no other VectorCraft crate.
#![forbid(unsafe_code)]

use std::fmt;

use serde_json::{Map, Value, json};

/// The JSON Schema dialect [`ParamSchema::to_json`] names in `$schema`.
pub const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// How many objects and lists may nest inside a schema's params object.
pub const MAX_DEPTH: usize = 16;

/// The largest object id [`Type::Id`] takes: 2^53 - 1, the largest integer every JSON reader holds
/// exactly.
pub const MAX_ID: u64 = (1 << 53) - 1;

/// How many characters of a refused value an error shows.
const SHOWN_CHARS: usize = 40;

/// How many characters of a key an error shows.
const KEY_CHARS: usize = 64;

/// One end of a number's range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bound {
    /// The value may equal the bound.
    Inclusive(f64),
    /// The value must be past the bound.
    Exclusive(f64),
}

/// The type of a parameter's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Type {
    /// A JSON number in the range the bounds give, measured in `unit` (`"pt"`, `"deg"`) when set.
    Number { min: Option<Bound>, max: Option<Bound>, unit: Option<&'static str> },
    /// A JSON number with no fractional part, in the range given. [`ParamSchema::validate`] gives
    /// the command `2.0` as `2`.
    Integer { min: Option<i64>, max: Option<i64> },
    /// `true` or `false`.
    Boolean,
    /// Any string.
    String,
    /// One of the strings listed.
    Enum(&'static [&'static str]),
    /// An object id: an integer from 0 to [`MAX_ID`], read as [`Type::Integer`] reads one.
    Id,
    /// A list of object ids.
    Ids,
    /// A color: a string (`"#rrggbb"`, a color key), `[r, g, b]` or an object. The command reads
    /// which color it is.
    Color,
    /// A point: `[x, y]`, two numbers.
    Point,
    /// An object with the parameters listed and no other key.
    Object(&'static [Param]),
    /// A list of values of one type.
    Array(&'static Type),
}

impl Type {
    /// A number with no range and no unit.
    pub const NUMBER: Type = Type::Number { min: None, max: None, unit: None };
}

/// What an omitted value means.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Omitted {
    /// The parameter is required, or the command describes the omitted case in its summary.
    Nothing,
    /// The value, as JSON text, that [`ParamSchema::validate`] fills in.
    Value(&'static str),
    /// A default the command works out itself (`"toggle"`, `"the selected text"`).
    Note(&'static str),
}

/// What a command changes when it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writes {
    /// Nothing: the command only reads.
    Nothing,
    /// State of the running app outside the document and the preferences (view state, the Caps
    /// Lock state).
    Session,
    /// The preferences.
    Preferences,
    /// The document, with an undo step.
    Document,
}

impl Writes {
    fn name(self) -> &'static str {
        match self {
            Writes::Nothing => "nothing",
            Writes::Session => "session",
            Writes::Preferences => "preferences",
            Writes::Document => "document",
        }
    }
}

/// One parameter of a command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Param {
    pub name: &'static str,
    pub ty: Type,
    pub required: bool,
    pub omitted: Omitted,
    /// A null value is read as if the parameter were omitted. Otherwise null is refused as a value
    /// of the wrong type.
    pub null_omitted: bool,
    /// What the params text says about the value, in parentheses after its type.
    pub doc: &'static str,
    /// The API level the parameter was added at.
    pub since: u32,
    /// The API level the parameter was deprecated at.
    pub deprecated: Option<u32>,
    /// A value of the wrong type or out of range is not refused here: the command reads it as it
    /// did before it had a schema. [`ParamSchema::to_json`] still writes the documented type, so a
    /// JSON Schema validator refuses a value this passes.
    pub lenient: bool,
}

impl Param {
    /// A required parameter, added at API level 1.
    pub const fn new(name: &'static str, ty: Type) -> Param {
        Param { name, ty, required: true, omitted: Omitted::Nothing, null_omitted: false, doc: "", since: 1, deprecated: None, lenient: false }
    }
    /// The parameter may be omitted.
    pub const fn optional(self) -> Param {
        Param { required: false, ..self }
    }
    /// The parameter may be omitted, and then takes the value `json` (JSON text).
    pub const fn defaults_to(self, json: &'static str) -> Param {
        Param { required: false, omitted: Omitted::Value(json), ..self }
    }
    /// The parameter may be omitted, and the command then works out the value `note` describes.
    pub const fn default_note(self, note: &'static str) -> Param {
        Param { required: false, omitted: Omitted::Note(note), ..self }
    }
    /// A null value reads as an omitted one.
    pub const fn null_omitted(self) -> Param {
        Param { null_omitted: true, ..self }
    }
    pub const fn doc(self, doc: &'static str) -> Param {
        Param { doc, ..self }
    }
    pub const fn since(self, level: u32) -> Param {
        Param { since: level, ..self }
    }
    pub const fn deprecated(self, level: u32) -> Param {
        Param { deprecated: Some(level), ..self }
    }
    pub const fn lenient(self) -> Param {
        Param { lenient: true, ..self }
    }
}

/// The parameters of one command, and what the command does with them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSchema {
    pub params: &'static [Param],
    /// What the command does, after the params object in the params text.
    pub summary: &'static str,
    /// What the command returns, after `→` in the params text; empty when the text leaves it out.
    pub returns: &'static str,
    /// The API level the command was added at.
    pub since: u32,
    /// The API level the command was deprecated at.
    pub deprecated: Option<u32>,
    /// What one run costs, in units of a light command (1), for callers that limit a script's work.
    pub cost: u32,
    pub writes: Writes,
}

impl ParamSchema {
    /// A command added at API level 1, costing 1, that changes the document.
    pub const fn new(params: &'static [Param], summary: &'static str) -> ParamSchema {
        ParamSchema { params, summary, returns: "", since: 1, deprecated: None, cost: 1, writes: Writes::Document }
    }
    pub const fn returns(self, returns: &'static str) -> ParamSchema {
        ParamSchema { returns, ..self }
    }
    pub const fn writes(self, writes: Writes) -> ParamSchema {
        ParamSchema { writes, ..self }
    }
    pub const fn cost(self, cost: u32) -> ParamSchema {
        ParamSchema { cost, ..self }
    }
    pub const fn since(self, level: u32) -> ParamSchema {
        ParamSchema { since: level, ..self }
    }
    pub const fn deprecated(self, level: u32) -> ParamSchema {
        ParamSchema { deprecated: Some(level), ..self }
    }

    /// The parameter named `name`.
    pub fn param(&self, name: &str) -> Option<&'static Param> {
        self.params.iter().find(|p| p.name == name)
    }

    /// The names of the top-level parameters, in order.
    pub fn names(&self) -> Vec<&'static str> {
        self.params.iter().map(|p| p.name).collect()
    }

    /// `params` checked against the schema: the first problem as an error naming the parameter,
    /// or, when it has none, `params` with the defaults of its omitted values filled in (`None`
    /// when no default was filled in, so `params` is used as it is). Null params read as `{}`.
    pub fn validate(&self, params: &Value) -> Result<Option<Value>, ParamError> {
        self.validate_as(params, false)
    }

    /// [`ParamSchema::validate`] for saved steps recorded by this version or an older one: a key
    /// the schema doesn't name passes, at any depth, and the command does not read it. Values of
    /// the parameters the schema names are checked as `validate` checks them.
    pub fn validate_saved(&self, params: &Value) -> Result<Option<Value>, ParamError> {
        self.validate_as(params, true)
    }

    fn validate_as(&self, params: &Value, saved: bool) -> Result<Option<Value>, ParamError> {
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        let given = match params {
            Value::Object(o) => o,
            Value::Null => EMPTY.get_or_init(Map::new),
            v => return Err(ParamError { path: String::new(), problem: Problem::NotAnObject { got: shown(v) } }),
        };
        Ok(check_object(&At::Root, 0, self.params, given, saved)?.map(Value::Object))
    }

    /// What is wrong with the schema itself, or `None`: a parameter name that is empty, repeated
    /// or holds `.`, `[` or `]`; a required parameter with a default; a default that isn't JSON or
    /// doesn't pass its parameter's type; an enum with no names or a repeated name; a range with
    /// an end that isn't finite or that no value can be in; nesting past [`MAX_DEPTH`], or a
    /// [`Type::Ids`] or [`Type::Point`], whose values are all lists, at the nesting limit.
    pub fn definition_problem(&self) -> Option<String> {
        object_problem("", self.params, 0)
    }

    /// The params text: the params object (`{name?: string, zoom: number > 0}`), then the summary,
    /// then `→` and what the command returns.
    pub fn params_text(&self) -> String {
        let mut s = object_text(self.params, 0);
        if !self.summary.is_empty() {
            s.push(' ');
            s.push_str(self.summary);
        }
        if !self.returns.is_empty() {
            s.push_str(" → ");
            s.push_str(self.returns);
        }
        s
    }

    /// The schema as a JSON Schema 2020-12 object schema. Defaults written as JSON go in
    /// `default`, a parameter that accepts null as omitted is an `anyOf` with `{"type": "null"}`,
    /// and VectorCraft's own fields are `x-since`, `x-deprecatedSince`, `x-lenient`, `x-unit`,
    /// `x-kind`, `x-cost`, `x-writes` and `x-returns`. The root `type` is `["object", "null"]` when
    /// no parameter is required, because null params read as `{}`, and `"object"` otherwise. A JSON
    /// Schema validator accepts the values [`ParamSchema::validate`] accepts, except that an
    /// `x-lenient` parameter's schema states only the documented type.
    pub fn to_json(&self) -> Value {
        let mut o = object_schema(self.params, 0);
        if !self.params.iter().any(|p| p.required) {
            o.insert("type".into(), json!(["object", "null"]));
        }
        o.insert("$schema".into(), DIALECT.into());
        if !self.summary.is_empty() {
            o.insert("description".into(), self.summary.into());
        }
        if !self.returns.is_empty() {
            o.insert("x-returns".into(), self.returns.into());
        }
        o.insert("x-since".into(), self.since.into());
        if let Some(level) = self.deprecated {
            o.insert("deprecated".into(), true.into());
            o.insert("x-deprecatedSince".into(), level.into());
        }
        o.insert("x-cost".into(), self.cost.into());
        o.insert("x-writes".into(), self.writes.name().into());
        Value::Object(o)
    }
}

/// A problem [`ParamSchema::validate`] found, at `path`: the parameter's name, with the keys and
/// indices leading to a value inside one (`edits[1].name`). The path is empty for the params
/// object itself.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamError {
    pub path: String,
    pub problem: Problem,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// The params are not an object.
    NotAnObject { got: String },
    /// The key at `path` is not a parameter of the object it is in, which takes `keys`.
    Unknown { keys: Vec<&'static str> },
    /// A required parameter is missing.
    Missing,
    /// The value is not of the parameter's type: `expected` says what it must be.
    WrongType { expected: String, got: String },
    /// The value nests deeper than [`MAX_DEPTH`] here: a list given for a [`Type::Color`] at the
    /// nesting limit, or any value under a schema that nests too deeply itself
    /// ([`ParamSchema::definition_problem`]).
    TooDeep,
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = &self.path;
        match &self.problem {
            Problem::NotAnObject { got } => write!(f, "the params must be an object, not {got}"),
            Problem::Unknown { keys } if keys.is_empty() => write!(f, "unknown parameter `{path}`; the object takes no keys"),
            Problem::Unknown { keys } => write!(f, "unknown parameter `{path}`; it takes: {}", keys.join(", ")),
            Problem::Missing => write!(f, "missing `{path}`"),
            Problem::WrongType { expected, got } => write!(f, "`{path}` must be {expected}, not {got}"),
            Problem::TooDeep => write!(f, "`{path}` is nested too deeply"),
        }
    }
}

impl std::error::Error for ParamError {}

/// FNV-1a over the bytes of `text`, from `h`.
fn fnv1a(mut h: u64, text: &str) -> u64 {
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// `v` as JSON text with the keys of each object in sorted order, whichever order the map keeps
/// them in.
fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::from(k.as_str()).to_string());
                out.push(':');
                if let Some(x) = o.get(k) {
                    canonical(x, out);
                }
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// A 64-bit FNV-1a hash of the commands' schemas, taken in the order of their ids: each id and
/// its [`ParamSchema::to_json`] text, with the keys of each object sorted. Any change to a schema,
/// or a schema added or removed, changes it.
pub fn hash<'a>(schemas: impl IntoIterator<Item = (&'a str, &'a ParamSchema)>) -> u64 {
    let mut all: Vec<(&str, &ParamSchema)> = schemas.into_iter().collect();
    all.sort_by_key(|(id, _)| *id);
    all.iter().fold(0xcbf2_9ce4_8422_2325, |h, (id, schema)| {
        let mut text = String::new();
        canonical(&schema.to_json(), &mut text);
        let h = fnv1a(h, id);
        let h = fnv1a(h, "\n");
        let h = fnv1a(h, &text);
        fnv1a(h, "\n")
    })
}

/// `key` as an error message shows it: cut to [`KEY_CHARS`] characters.
pub fn shown_key(key: &str) -> String {
    let mut chars = key.chars();
    let head: String = chars.by_ref().take(KEY_CHARS).collect();
    if chars.next().is_some() { format!("{head}…") } else { head }
}

/// The JSON text of a value, stopping when its characters run out.
struct Shown {
    text: String,
    left: usize,
}

impl Shown {
    /// Adds `s`; false when it didn't fit.
    fn push(&mut self, s: &str) -> bool {
        for c in s.chars() {
            if self.left == 0 {
                return false;
            }
            self.left -= 1;
            self.text.push(c);
        }
        true
    }

    /// Adds the JSON text of string `s`, reading no more of it than can be shown.
    fn push_string(&mut self, s: &str) -> bool {
        let head: String = s.chars().take(self.left + 1).collect();
        self.push(&Value::from(head).to_string())
    }

    /// Adds the JSON text of `v`; false when it didn't fit. Each object or list writes a bracket,
    /// so the characters left bound the recursion.
    fn push_value(&mut self, v: &Value) -> bool {
        match v {
            Value::String(s) => self.push_string(s),
            Value::Array(items) => {
                self.push("[") && items.iter().enumerate().all(|(i, x)| (i == 0 || self.push(",")) && self.push_value(x)) && self.push("]")
            }
            Value::Object(o) => {
                self.push("{")
                    && o.iter()
                        .enumerate()
                        .all(|(i, (k, x))| (i == 0 || self.push(",")) && self.push_string(k) && self.push(":") && self.push_value(x))
                    && self.push("}")
            }
            other => self.push(&other.to_string()),
        }
    }
}

/// A value as an error message shows it: its JSON, cut to [`SHOWN_CHARS`] characters. Only what is
/// shown is read, however large the value is.
fn shown(v: &Value) -> String {
    let mut out = Shown { text: String::new(), left: SHOWN_CHARS };
    if !out.push_value(v) {
        out.text.push('…');
    }
    out.text
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() { key.to_string() } else { format!("{path}.{key}") }
}

/// Where a value is, from the value up to the params object. The path's text is written only for
/// an error.
#[derive(Clone, Copy)]
enum At<'a> {
    Root,
    Key(&'a At<'a>, &'a str),
    Index(&'a At<'a>, usize),
}

impl At<'_> {
    /// The path, `edits[1].name`, with each key as [`shown_key`] shows it.
    fn text(&self) -> String {
        let mut steps = Vec::new();
        let mut at = self;
        while let At::Key(up, _) | At::Index(up, _) = at {
            steps.push(at);
            at = up;
        }
        let mut out = String::new();
        for step in steps.into_iter().rev() {
            match step {
                At::Key(_, key) => {
                    if !out.is_empty() {
                        out.push('.');
                    }
                    out.push_str(&shown_key(key));
                }
                At::Index(_, i) => out.push_str(&format!("[{i}]")),
                At::Root => {}
            }
        }
        out
    }
}

/// Whether `v` nests no deeper than [`MAX_DEPTH`] counted from `depth`. The walk stops at the
/// limit, so it recurses at most [`MAX_DEPTH`] levels and allocates nothing.
fn nests_within(v: &Value, depth: usize) -> bool {
    match v {
        Value::Array(a) => depth < MAX_DEPTH && a.iter().all(|x| nests_within(x, depth + 1)),
        Value::Object(o) => depth < MAX_DEPTH && o.values().all(|x| nests_within(x, depth + 1)),
        _ => true,
    }
}

/// The object `given` checked against `params` (unknown keys first, then each parameter in
/// order): with the defaults of omitted values filled in when there are any. `saved` keeps a key
/// the schema doesn't name, unless its value nests past [`MAX_DEPTH`].
fn check_object(
    at: &At<'_>,
    depth: usize,
    params: &'static [Param],
    given: &Map<String, Value>,
    saved: bool,
) -> Result<Option<Map<String, Value>>, ParamError> {
    let refused = |key: &String, v: &Value| !params.iter().any(|p| p.name == key.as_str()) && (!saved || !nests_within(v, depth));
    if let Some((key, _)) = given.iter().find(|(k, v)| refused(k, v)) {
        return Err(ParamError { path: At::Key(at, key).text(), problem: Problem::Unknown { keys: params.iter().map(|p| p.name).collect() } });
    }
    // The changes are applied once every value has passed, so no refused value is ever copied.
    let mut changes: Vec<(&'static str, Value)> = Vec::new();
    for p in params {
        let here = At::Key(at, p.name);
        match given.get(p.name) {
            Some(Value::Null) if p.null_omitted => {}
            Some(v) => {
                if let Some(changed) = check_value(&here, depth, &p.ty, v, p.lenient, saved)? {
                    changes.push((p.name, changed));
                }
                continue;
            }
            None if p.required => return Err(ParamError { path: here.text(), problem: Problem::Missing }),
            None => {}
        }
        // Omitted, or null read as omitted.
        if let Omitted::Value(json) = p.omitted
            && let Ok(v) = serde_json::from_str::<Value>(json)
        {
            changes.push((p.name, v));
        }
    }
    if changes.is_empty() {
        return Ok(None);
    }
    let mut filled = given.clone();
    for (name, v) in changes {
        filled.insert(name.to_string(), v);
    }
    Ok(Some(filled))
}

/// What a value of type `ty` must be, as an error message says it.
fn expected(ty: &Type) -> String {
    match ty {
        Type::Number { min, max, unit } => {
            format!("a number{}{}", range_text(min.as_ref(), max.as_ref()), unit.map(|u| format!(" ({u})")).unwrap_or_default())
        }
        Type::Integer { min, max } => {
            format!("an integer{}", range_text(min.map(|m| Bound::Inclusive(m as f64)).as_ref(), max.map(|m| Bound::Inclusive(m as f64)).as_ref()))
        }
        Type::Boolean => "true or false".into(),
        Type::String => "a string".into(),
        Type::Enum(names) => format!("one of {}", names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ")),
        Type::Id => "an object id (a non-negative integer)".into(),
        Type::Ids => "a list of object ids".into(),
        Type::Color => "a color (a string, [r, g, b] or an object)".into(),
        Type::Point => "[x, y] (two numbers)".into(),
        Type::Object(_) => "an object".into(),
        Type::Array(_) => "a list".into(),
    }
}

/// ` > 0`, ` 0 to 100`, ` >= 1 and < 10`: the range text after a number's type.
fn range_text(min: Option<&Bound>, max: Option<&Bound>) -> String {
    match (min, max) {
        (Some(Bound::Inclusive(a)), Some(Bound::Inclusive(b))) => format!(" {a} to {b}"),
        (Some(a), Some(b)) => format!("{} and{}", low_text(a), high_text(b)),
        (Some(a), None) => low_text(a),
        (None, Some(b)) => high_text(b),
        (None, None) => String::new(),
    }
}

fn low_text(b: &Bound) -> String {
    match b {
        Bound::Inclusive(a) => format!(" >= {a}"),
        Bound::Exclusive(a) => format!(" > {a}"),
    }
}

fn high_text(b: &Bound) -> String {
    match b {
        Bound::Inclusive(a) => format!(" <= {a}"),
        Bound::Exclusive(a) => format!(" < {a}"),
    }
}

fn in_range(x: f64, min: Option<&Bound>, max: Option<&Bound>) -> bool {
    let low = match min {
        Some(Bound::Inclusive(a)) => x >= *a,
        Some(Bound::Exclusive(a)) => x > *a,
        None => true,
    };
    let high = match max {
        Some(Bound::Inclusive(b)) => x <= *b,
        Some(Bound::Exclusive(b)) => x < *b,
        None => true,
    };
    low && high
}

/// The integer a JSON number with no fractional part stands for, as an integer value when it fits
/// in one (`2.0` is `2`); `None` for anything else.
fn whole(v: &Value) -> Option<Value> {
    if v.is_i64() || v.is_u64() {
        return Some(v.clone());
    }
    const TWO_63: f64 = 9_223_372_036_854_775_808.0;
    const TWO_64: f64 = 18_446_744_073_709_551_616.0;
    let x = v.as_f64().filter(|x| x.fract() == 0.0)?;
    Some(if (-TWO_63..TWO_63).contains(&x) {
        json!(x as i64)
    } else if (0.0..TWO_64).contains(&x) {
        json!(x as u64)
    } else {
        v.clone()
    })
}

/// Whether the [`whole`] value `w` is in the range.
fn whole_in_range(w: &Value, min: Option<i64>, max: Option<i64>) -> bool {
    match (w.as_i64(), w.as_u64(), w.as_f64()) {
        (Some(x), _, _) => min.is_none_or(|m| x >= m) && max.is_none_or(|m| x <= m),
        // Past i64::MAX: above any i64 bound.
        (None, Some(_), _) => max.is_none(),
        // Past what u64 holds: above any i64 bound.
        (None, None, Some(x)) if x > 0.0 => max.is_none(),
        // Below what i64 holds: under any i64 bound, so it meets every maximum and no minimum.
        (None, None, Some(_)) => min.is_none(),
        _ => false,
    }
}

fn is_number(v: &Value) -> bool {
    v.as_f64().is_some_and(f64::is_finite)
}

/// `v` checked against `ty` at `at`: with the defaults of omitted values inside it filled in, and
/// whole numbers written with a fraction made integers, when there are any. A lenient value of
/// the wrong type passes as it is, unless it nests past [`MAX_DEPTH`].
fn check_value(at: &At<'_>, depth: usize, ty: &Type, v: &Value, lenient: bool, saved: bool) -> Result<Option<Value>, ParamError> {
    let done = |ok: bool| {
        if ok || (lenient && nests_within(v, depth)) {
            Ok(None)
        } else {
            Err(ParamError { path: at.text(), problem: Problem::WrongType { expected: expected(ty), got: shown(v) } })
        }
    };
    let too_deep = || ParamError { path: at.text(), problem: Problem::TooDeep };
    if matches!(ty, Type::Ids | Type::Color | Type::Point) && v.is_array() && depth >= MAX_DEPTH {
        return Err(too_deep());
    }
    match ty {
        Type::Number { min, max, .. } => done(v.as_f64().is_some_and(|x| x.is_finite() && in_range(x, min.as_ref(), max.as_ref()))),
        Type::Integer { min, max } => match whole(v).filter(|w| whole_in_range(w, *min, *max)) {
            Some(w) => Ok((w != *v).then_some(w)),
            None => done(false),
        },
        Type::Boolean => done(v.is_boolean()),
        Type::String => done(v.is_string()),
        Type::Enum(names) => done(v.as_str().is_some_and(|s| names.contains(&s))),
        Type::Id => match whole(v).filter(|w| w.as_u64().is_some_and(|x| x <= MAX_ID)) {
            Some(w) => Ok((w != *v).then_some(w)),
            None => done(false),
        },
        Type::Ids => match v.as_array() {
            Some(items) => check_items(at, depth, &Type::Id, items, lenient, saved),
            None => done(false),
        },
        Type::Color => done(match v {
            Value::String(_) => true,
            Value::Object(_) => nests_within(v, depth),
            Value::Array(a) => a.len() >= 3 && a.iter().all(is_number),
            _ => false,
        }),
        Type::Point => done(v.as_array().is_some_and(|a| a.len() == 2 && a.iter().all(is_number))),
        Type::Object(params) => match v {
            Value::Object(_) if depth >= MAX_DEPTH => Err(too_deep()),
            Value::Object(o) => Ok(check_object(at, depth + 1, params, o, saved)?.map(Value::Object)),
            _ => done(false),
        },
        Type::Array(item) => match v.as_array() {
            Some(_) if depth >= MAX_DEPTH => Err(too_deep()),
            Some(items) => check_items(at, depth + 1, item, items, lenient, saved),
            None => done(false),
        },
    }
}

/// Each of `items` checked against `item`: the list with what [`check_value`] changed, when it
/// changed anything.
fn check_items(at: &At<'_>, depth: usize, item: &Type, items: &[Value], lenient: bool, saved: bool) -> Result<Option<Value>, ParamError> {
    let mut changes: Vec<(usize, Value)> = Vec::new();
    for (i, x) in items.iter().enumerate() {
        if let Some(changed) = check_value(&At::Index(at, i), depth, item, x, lenient, saved)? {
            changes.push((i, changed));
        }
    }
    if changes.is_empty() {
        return Ok(None);
    }
    let mut filled = items.to_vec();
    for (i, v) in changes {
        if let Some(slot) = filled.get_mut(i) {
            *slot = v;
        }
    }
    Ok(Some(Value::Array(filled)))
}

/// What is wrong with `params`, the object at `path` nested `depth` objects and lists deep.
fn object_problem(path: &str, params: &[Param], depth: usize) -> Option<String> {
    if depth > MAX_DEPTH {
        return Some(format!("`{path}` nests deeper than {MAX_DEPTH} levels"));
    }
    for (i, p) in params.iter().enumerate() {
        let at = join(path, p.name);
        if p.name.is_empty() || p.name.contains(['.', '[', ']']) {
            return Some(format!("the parameter name `{at}` is empty or holds a path character"));
        }
        if params.iter().take(i).any(|q| q.name == p.name) {
            return Some(format!("`{at}` is listed twice"));
        }
        if p.required && p.omitted != Omitted::Nothing {
            return Some(format!("`{at}` is required and has a default"));
        }
        if let Some(problem) = type_problem(&at, &p.ty, depth) {
            return Some(problem);
        }
        if let Omitted::Value(text) = p.omitted {
            let Ok(v) = serde_json::from_str::<Value>(text) else { return Some(format!("the default of `{at}` isn't JSON: {text}")) };
            if let Err(e) = check_value(&At::Root, 0, &p.ty, &v, false, false) {
                return Some(format!("the default of `{at}` doesn't pass its type: {e}"));
            }
        }
    }
    None
}

fn type_problem(at: &str, ty: &Type, depth: usize) -> Option<String> {
    let bound = |b: &Bound| match *b {
        Bound::Inclusive(x) => (x, false),
        Bound::Exclusive(x) => (x, true),
    };
    match ty {
        Type::Number { min, max, .. } => {
            let (low, high) = (min.as_ref().map(bound), max.as_ref().map(bound));
            if low.iter().chain(high.iter()).any(|(x, _)| !x.is_finite()) {
                return Some(format!("the range of `{at}` has an end that isn't finite"));
            }
            match (low, high) {
                (Some((a, open_a)), Some((b, open_b))) if a > b || (a == b && (open_a || open_b)) => {
                    Some(format!("no value can be in the range of `{at}`"))
                }
                _ => None,
            }
        }
        Type::Integer { min: Some(a), max: Some(b) } if a > b => Some(format!("no value can be in the range of `{at}`")),
        Type::Enum([]) => Some(format!("the enum of `{at}` has no names")),
        Type::Enum(names) if names.iter().enumerate().any(|(i, n)| names.iter().take(i).any(|m| m == n)) => {
            Some(format!("the enum of `{at}` repeats a name"))
        }
        Type::Object(params) => object_problem(at, params, depth + 1),
        Type::Array(_) if depth >= MAX_DEPTH => Some(format!("`{at}` nests deeper than {MAX_DEPTH} levels")),
        Type::Array(item) => type_problem(&format!("{at}[]"), item, depth + 1),
        // Every value of these is a list, and a list at the nesting limit is refused.
        Type::Ids | Type::Point if depth >= MAX_DEPTH => Some(format!("`{at}` nests deeper than {MAX_DEPTH} levels")),
        _ => None,
    }
}

/// `{a: number, b?: bool}`: the params object of a params text, `depth` objects and lists deep.
fn object_text(params: &[Param], depth: usize) -> String {
    format!("{{{}}}", params.iter().map(|p| param_text(p, depth)).collect::<Vec<_>>().join(", "))
}

fn param_text(p: &Param, depth: usize) -> String {
    let mut s = format!("{}{}: {}", p.name, if p.required { "" } else { "?" }, type_text(&p.ty, depth));
    match p.omitted {
        Omitted::Value(v) | Omitted::Note(v) => s.push_str(&format!(" (default: {v})")),
        Omitted::Nothing => {}
    }
    if !p.doc.is_empty() {
        s.push_str(&format!(" ({})", p.doc));
    }
    if p.deprecated.is_some() {
        s.push_str(" (deprecated)");
    }
    s
}

fn type_text(ty: &Type, depth: usize) -> String {
    match ty {
        Type::Object(_) | Type::Array(_) if depth >= MAX_DEPTH => "…".into(),
        Type::Number { min, max, unit } => {
            format!("number{}{}", range_text(min.as_ref(), max.as_ref()), unit.map(|u| format!(" {u}")).unwrap_or_default())
        }
        Type::Integer { min, max } => {
            format!("integer{}", range_text(min.map(|m| Bound::Inclusive(m as f64)).as_ref(), max.map(|m| Bound::Inclusive(m as f64)).as_ref()))
        }
        Type::Boolean => "bool".into(),
        Type::String => "string".into(),
        Type::Enum(names) => names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join("|"),
        Type::Id => "id".into(),
        Type::Ids => "[id…]".into(),
        Type::Color => "color".into(),
        Type::Point => "[x, y]".into(),
        Type::Object(params) => object_text(params, depth + 1),
        Type::Array(item) => format!("[{}…]", type_text(item, depth + 1)),
    }
}

/// `{"type": "object", "properties": …, "required": […], "additionalProperties": false}`.
fn object_schema(params: &[Param], depth: usize) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("type".into(), "object".into());
    o.insert("properties".into(), Value::Object(params.iter().map(|p| (p.name.to_string(), param_schema(p, depth))).collect()));
    let required: Vec<Value> = params.iter().filter(|p| p.required).map(|p| p.name.into()).collect();
    if !required.is_empty() {
        o.insert("required".into(), required.into());
    }
    o.insert("additionalProperties".into(), false.into());
    o
}

fn param_schema(p: &Param, depth: usize) -> Value {
    let inner = type_schema(&p.ty, depth);
    let mut o = if p.null_omitted {
        let mut o = Map::new();
        o.insert("anyOf".into(), json!([inner, {"type": "null"}]));
        o
    } else {
        match inner {
            Value::Object(o) => o,
            _ => Map::new(),
        }
    };
    if !p.doc.is_empty() {
        o.insert("description".into(), p.doc.into());
    }
    match p.omitted {
        Omitted::Value(json) => {
            if let Ok(v) = serde_json::from_str::<Value>(json) {
                o.insert("default".into(), v);
            }
        }
        Omitted::Note(note) => {
            o.insert("x-default".into(), note.into());
        }
        Omitted::Nothing => {}
    }
    o.insert("x-since".into(), p.since.into());
    if let Some(level) = p.deprecated {
        o.insert("deprecated".into(), true.into());
        o.insert("x-deprecatedSince".into(), level.into());
    }
    if p.lenient {
        o.insert("x-lenient".into(), true.into());
    }
    Value::Object(o)
}

fn bound_keywords(o: &mut Map<String, Value>, min: Option<&Bound>, max: Option<&Bound>) {
    let low = match min {
        Some(Bound::Inclusive(a)) => Some(("minimum", a)),
        Some(Bound::Exclusive(a)) => Some(("exclusiveMinimum", a)),
        None => None,
    };
    let high = match max {
        Some(Bound::Inclusive(b)) => Some(("maximum", b)),
        Some(Bound::Exclusive(b)) => Some(("exclusiveMaximum", b)),
        None => None,
    };
    for (key, x) in low.into_iter().chain(high) {
        o.insert(key.into(), json!(x));
    }
}

fn type_schema(ty: &Type, depth: usize) -> Value {
    match ty {
        Type::Object(_) | Type::Array(_) if depth >= MAX_DEPTH => Value::Bool(true),
        Type::Number { min, max, unit } => {
            let mut o = Map::new();
            o.insert("type".into(), "number".into());
            bound_keywords(&mut o, min.as_ref(), max.as_ref());
            if let Some(u) = unit {
                o.insert("x-unit".into(), (*u).into());
            }
            Value::Object(o)
        }
        Type::Integer { min, max } => {
            let mut o = Map::new();
            o.insert("type".into(), "integer".into());
            if let Some(m) = min {
                o.insert("minimum".into(), (*m).into());
            }
            if let Some(m) = max {
                o.insert("maximum".into(), (*m).into());
            }
            Value::Object(o)
        }
        Type::Boolean => json!({"type": "boolean"}),
        Type::String => json!({"type": "string"}),
        Type::Enum(names) => json!({"enum": names}),
        Type::Id => json!({"type": "integer", "minimum": 0, "maximum": MAX_ID, "x-kind": "id"}),
        Type::Ids => json!({"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": MAX_ID, "x-kind": "id"}}),
        Type::Color => json!({
            "anyOf": [{"type": "string"}, {"type": "array", "items": {"type": "number"}, "minItems": 3}, {"type": "object"}],
            "x-kind": "color",
        }),
        Type::Point => json!({
            "type": "array",
            "prefixItems": [{"type": "number"}, {"type": "number"}],
            "items": false,
            "minItems": 2,
            "x-kind": "point",
        }),
        Type::Object(params) => Value::Object(object_schema(params, depth + 1)),
        Type::Array(item) => json!({"type": "array", "items": type_schema(item, depth + 1)}),
    }
}

#[cfg(test)]
mod conformance;
#[cfg(test)]
mod tests;
