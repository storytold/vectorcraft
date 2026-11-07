//! Machine-readable parameter schemas for VectorCraft commands. A [`ParamSchema`] lists what each
//! parameter of a command is ([`Param`], [`Type`]): its type, whether it is required, what an
//! omitted value means, whether null counts as omitted, and the API levels it was added and
//! deprecated at. From one schema come:
//!
//! - [`ParamSchema::validate`], which checks a params object and names the parameter of the first
//!   problem it finds ([`ParamError`]), and fills in the defaults of omitted values;
//! - [`ParamSchema::params_text`], the params text a command lists;
//! - [`ParamSchema::to_json`], the schema as a JSON object in the shape of a JSON Schema 2020-12
//!   object schema (<https://json-schema.org/draft/2020-12>), with VectorCraft's own fields as
//!   `x-` keywords;
//! - [`hash`], a hash of a set of command schemas, which changes whenever any of them changes.
//!
//! The crate knows no command and depends on no other VectorCraft crate.
#![forbid(unsafe_code)]

use std::fmt;

use serde_json::{Map, Value, json};

/// The JSON Schema dialect [`ParamSchema::to_json`] names in `$schema`.
pub const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

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
    /// A JSON number without a fraction (written without a decimal point), in the range given.
    Integer { min: Option<i64>, max: Option<i64> },
    /// `true` or `false`.
    Boolean,
    /// Any string.
    String,
    /// One of the strings listed.
    Enum(&'static [&'static str]),
    /// An object id: a non-negative integer.
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
    /// did before it had a schema.
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
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        let given = match params {
            Value::Object(o) => o,
            Value::Null => EMPTY.get_or_init(Map::new),
            v => return Err(ParamError { path: String::new(), problem: Problem::NotAnObject { got: shown(v) } }),
        };
        Ok(check_object("", self.params, given)?.map(Value::Object))
    }

    /// The params text: the params object (`{name?: string, zoom: number > 0}`), then the summary,
    /// then `→` and what the command returns.
    pub fn params_text(&self) -> String {
        let mut s = object_text(self.params);
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
    /// `x-kind`, `x-cost`, `x-writes` and `x-returns`.
    pub fn to_json(&self) -> Value {
        let mut o = object_schema(self.params);
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

/// A value as an error message shows it: its JSON, cut to 40 characters.
fn shown(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 40 { format!("{}…", s.chars().take(40).collect::<String>()) } else { s }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() { key.to_string() } else { format!("{path}.{key}") }
}

/// The object `given` checked against `params` (unknown keys first, then each parameter in
/// order): with the defaults of omitted values filled in when there are any.
fn check_object(path: &str, params: &'static [Param], given: &Map<String, Value>) -> Result<Option<Map<String, Value>>, ParamError> {
    if let Some(key) = given.keys().find(|k| !params.iter().any(|p| p.name == k.as_str())) {
        return Err(ParamError { path: join(path, key), problem: Problem::Unknown { keys: params.iter().map(|p| p.name).collect() } });
    }
    let mut filled: Option<Map<String, Value>> = None;
    for p in params {
        let at = || join(path, p.name);
        match given.get(p.name) {
            Some(Value::Null) if p.null_omitted => {}
            Some(v) => {
                if let Some(changed) = check_value(&at(), &p.ty, v, p.lenient)? {
                    filled.get_or_insert_with(|| given.clone()).insert(p.name.to_string(), changed);
                }
                continue;
            }
            None if p.required => return Err(ParamError { path: at(), problem: Problem::Missing }),
            None => {}
        }
        // Omitted, or null read as omitted.
        if let Omitted::Value(json) = p.omitted
            && let Ok(v) = serde_json::from_str::<Value>(json)
        {
            filled.get_or_insert_with(|| given.clone()).insert(p.name.to_string(), v);
        }
    }
    Ok(filled)
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

fn is_id(v: &Value) -> bool {
    v.as_u64().is_some()
}

fn is_number(v: &Value) -> bool {
    v.as_f64().is_some_and(f64::is_finite)
}

/// `v` checked against `ty` at `path`: with the defaults of omitted values inside it filled in
/// when there are any. A lenient value of the wrong type passes as it is.
fn check_value(path: &str, ty: &Type, v: &Value, lenient: bool) -> Result<Option<Value>, ParamError> {
    let wrong = || ParamError { path: path.to_string(), problem: Problem::WrongType { expected: expected(ty), got: shown(v) } };
    let ok = match ty {
        Type::Number { min, max, .. } => v.as_f64().is_some_and(|x| x.is_finite() && in_range(x, min.as_ref(), max.as_ref())),
        Type::Integer { min, max } => match (v.as_i64(), v.as_u64()) {
            (Some(x), _) => min.is_none_or(|m| x >= m) && max.is_none_or(|m| x <= m),
            // Past i64::MAX: above any i64 bound.
            (None, Some(_)) => max.is_none(),
            (None, None) => false,
        },
        Type::Boolean => v.is_boolean(),
        Type::String => v.is_string(),
        Type::Enum(names) => v.as_str().is_some_and(|s| names.contains(&s)),
        Type::Id => is_id(v),
        Type::Ids => v.as_array().is_some_and(|a| a.iter().all(is_id)),
        Type::Color => match v {
            Value::String(_) | Value::Object(_) => true,
            Value::Array(a) => a.len() >= 3 && a.iter().all(is_number),
            _ => false,
        },
        Type::Point => v.as_array().is_some_and(|a| a.len() == 2 && a.iter().all(is_number)),
        Type::Object(params) => {
            return match v {
                Value::Object(o) => Ok(check_object(path, params, o)?.map(Value::Object)),
                _ if lenient => Ok(None),
                _ => Err(wrong()),
            };
        }
        Type::Array(item) => {
            let Some(items) = v.as_array() else { return if lenient { Ok(None) } else { Err(wrong()) } };
            let mut filled: Option<Vec<Value>> = None;
            for (i, x) in items.iter().enumerate() {
                if let Some(changed) = check_value(&format!("{path}[{i}]"), item, x, lenient)? {
                    let all = filled.get_or_insert_with(|| items.clone());
                    if let Some(slot) = all.get_mut(i) {
                        *slot = changed;
                    }
                }
            }
            return Ok(filled.map(Value::Array));
        }
    };
    if ok || lenient { Ok(None) } else { Err(wrong()) }
}

/// `{a: number, b?: bool}`: the params object of a params text.
fn object_text(params: &[Param]) -> String {
    format!("{{{}}}", params.iter().map(param_text).collect::<Vec<_>>().join(", "))
}

fn param_text(p: &Param) -> String {
    let mut s = format!("{}{}: {}", p.name, if p.required { "" } else { "?" }, type_text(&p.ty));
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

fn type_text(ty: &Type) -> String {
    match ty {
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
        Type::Object(params) => object_text(params),
        Type::Array(item) => format!("[{}…]", type_text(item)),
    }
}

/// `{"type": "object", "properties": …, "required": […], "additionalProperties": false}`.
fn object_schema(params: &[Param]) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("type".into(), "object".into());
    o.insert("properties".into(), Value::Object(params.iter().map(|p| (p.name.to_string(), param_schema(p))).collect()));
    let required: Vec<Value> = params.iter().filter(|p| p.required).map(|p| p.name.into()).collect();
    if !required.is_empty() {
        o.insert("required".into(), required.into());
    }
    o.insert("additionalProperties".into(), false.into());
    o
}

fn param_schema(p: &Param) -> Value {
    let inner = type_schema(&p.ty);
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

fn type_schema(ty: &Type) -> Value {
    match ty {
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
        Type::Id => json!({"type": "integer", "minimum": 0, "x-kind": "id"}),
        Type::Ids => json!({"type": "array", "items": {"type": "integer", "minimum": 0, "x-kind": "id"}}),
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
        Type::Object(params) => Value::Object(object_schema(params)),
        Type::Array(item) => json!({"type": "array", "items": type_schema(item)}),
    }
}

#[cfg(test)]
mod tests;
