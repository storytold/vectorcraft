//! The plug-in manifest (`vc_manifest`) and parameter schema: what each parameter holds
//! ([`ParamSpec`]) and how dialogs show it ([`ParamUi`], in [`crate::dialog`]).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::dialog::ParamUi;
use crate::{Error, Result};

/// Longest manifest the host reads.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Most parameters a plug-in may declare.
pub const MAX_PARAMS: usize = 256;
/// Most parameters outside groups (`group`): a dialog of more is sorted into sections.
pub const MAX_UNGROUPED: usize = 64;
/// Longest text a `string` parameter may hold, in characters.
pub const MAX_TEXT: usize = 4_096;
/// Most numbers a `numberList` parameter may hold.
pub const MAX_LIST: usize = 256;
/// Most file extensions an exporter may declare.
pub const MAX_EXTENSIONS: usize = 8;

/// What a plug-in is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// An object filter: reads the selected paths and returns the objects to keep, change or add
    /// (Object › Plug-ins, `plugin.run`).
    Filter,
    /// A live effect: rewrites one object's geometry each time it is drawn, from its appearance
    /// stack (Effect › Plug-ins, `effect.apply`).
    Effect,
    /// An exporter: writes the document in a file format of its own, whose extensions the manifest
    /// lists (File › Export › Export As, `document.export`; see [`crate::export`]).
    Exporter,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Filter => "filter",
            Kind::Effect => "effect",
            Kind::Exporter => "exporter",
        }
    }
}

/// How much one run of a plug-in may compute.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Budget {
    /// The usual budget ([`crate::Limits::default`]).
    #[default]
    Light,
    /// A filter's larger budget for heavy but legitimate work (routing, packing, simulation):
    /// [`crate::HEAVY_FUEL`] instructions and [`crate::HEAVY_WALL_TIME_MS`]. The
    /// user allows it once, when the plug-in is installed, and the app runs it in the background
    /// with Cancel.
    Heavy,
}

impl Budget {
    /// The usual budget, which a manifest written back leaves out (`skip_serializing_if`).
    fn is_light(&self) -> bool {
        *self == Budget::Light
    }
}

/// One declared parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ParamSpec {
    /// A real number in `min..=max`.
    Number { min: f64, max: f64, default: f64 },
    /// A whole number in `min..=max`.
    Int {
        #[serde(default = "int_min")]
        min: i64,
        #[serde(default = "int_max")]
        max: i64,
        #[serde(default)]
        default: i64,
    },
    /// A checkbox.
    Bool {
        #[serde(default)]
        default: bool,
    },
    /// One of `options` (a dropdown).
    Choice {
        options: Vec<String>,
        #[serde(default)]
        default: Option<String>,
    },
    /// Text of at most `max_length` characters (a text field).
    #[serde(rename = "string")]
    Text {
        #[serde(default)]
        default: String,
        #[serde(default = "text_max", rename = "maxLength")]
        max_length: usize,
    },
    /// `min_items` to `max_items` numbers, each in `min..=max` (a field of numbers separated by
    /// commas or spaces).
    #[serde(rename_all = "camelCase")]
    NumberList {
        min: f64,
        max: f64,
        #[serde(default)]
        default: Vec<f64>,
        #[serde(default)]
        min_items: usize,
        #[serde(default = "list_max")]
        max_items: usize,
    },
}

fn int_min() -> i64 {
    -1_000_000
}
fn int_max() -> i64 {
    1_000_000
}
fn text_max() -> usize {
    256
}
fn list_max() -> usize {
    16
}

/// One declared parameter: what it holds, and how dialogs show it. In the manifest both are keys of
/// the same object: `{"type": "number", "min": 0, "max": 10, "default": 1, "unit": "mm"}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Param {
    #[serde(flatten)]
    pub spec: ParamSpec,
    #[serde(flatten)]
    pub ui: ParamUi,
}

impl From<ParamSpec> for Param {
    fn from(spec: ParamSpec) -> Self {
        Param { spec, ui: ParamUi::default() }
    }
}

impl ParamSpec {
    /// The value a parameter takes when it isn't given (numeric defaults clamped into range).
    pub fn default_value(&self) -> Value {
        match self {
            ParamSpec::Number { min, max, default } => json!(default.clamp(*min, *max)),
            ParamSpec::Int { min, max, default } => json!((*default).clamp(*min, *max)),
            ParamSpec::Bool { default } => json!(default),
            ParamSpec::Choice { options, default } => json!(default.clone().or_else(|| options.first().cloned()).unwrap_or_default()),
            ParamSpec::Text { default, max_length } => json!(default.chars().take(*max_length).collect::<String>()),
            ParamSpec::NumberList { min, max, default, .. } => json!(default.iter().map(|x| x.clamp(*min, *max)).collect::<Vec<f64>>()),
        }
    }

    /// Checks the spec of parameter `k`.
    fn validate(&self, k: &str) -> Result<()> {
        let bad = |what: &str| Err(Error::Manifest(format!("parameter `{k}`: {what}")));
        match self {
            ParamSpec::Number { min, max, default } if !(min.is_finite() && max.is_finite() && default.is_finite()) || min > max => bad("bad range"),
            ParamSpec::Int { min, max, .. } if min > max => bad("bad range"),
            ParamSpec::Choice { options, default } => {
                if options.is_empty() || options.len() > 64 || options.iter().any(|o| o.is_empty() || o.len() > 64 || o.chars().any(char::is_control))
                {
                    return bad("choices must be 1-64 non-empty, printable strings");
                }
                if default.as_ref().is_some_and(|d| !options.contains(d)) {
                    return bad("default is not one of the options");
                }
                Ok(())
            }
            ParamSpec::Text { default, max_length } if !(1..=MAX_TEXT).contains(max_length) || default.chars().count() > *max_length => {
                bad(&format!("`maxLength` must be 1-{MAX_TEXT}, and the default no longer"))
            }
            ParamSpec::NumberList { min, max, default, min_items, max_items } => {
                if !(min.is_finite() && max.is_finite()) || min > max || default.iter().any(|x| !x.is_finite()) {
                    return bad("bad range");
                }
                if !(1..=MAX_LIST).contains(max_items) || min_items > max_items || !(*min_items..=*max_items).contains(&default.len()) {
                    return bad(&format!("`minItems` and `maxItems` must be 0-{MAX_LIST}, `maxItems` at least 1, and the default that many"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// The manifest a plug-in returns from `vc_manifest` (UTF-8 JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Stable id, `[A-Za-z0-9._-]{1,64}` (reverse-DNS style is recommended: `org.example.tidy`).
    pub id: String,
    /// Menu label.
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub kind: Kind,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// Parameters in declaration (dialog) order; a JSON object `{"name": spec, …}` in the manifest.
    #[serde(default, deserialize_with = "ordered_params", serialize_with = "params_as_object")]
    pub params: Vec<(String, Param)>,
    /// An exporter's file extensions, lower case and without the dot; the first is the one its
    /// files get. Only exporters declare them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// How much a run may compute: only a filter may be [`Budget::Heavy`].
    #[serde(default, skip_serializing_if = "Budget::is_light")]
    pub budget: Budget,
}

impl Manifest {
    /// Parses and validates manifest JSON.
    pub fn parse(bytes: &[u8]) -> Result<Manifest> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(Error::Manifest(format!("manifest is {} bytes (limit {MAX_MANIFEST_BYTES})", bytes.len())));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::Manifest("manifest is not UTF-8".into()))?;
        let man: Manifest = serde_json::from_str(text).map_err(|e| Error::Manifest(format!("manifest: {e}")))?;
        man.validate()?;
        Ok(man)
    }

    /// The manifest of a module still being loaded.
    pub(crate) fn placeholder() -> Manifest {
        Manifest {
            id: String::new(),
            name: String::new(),
            version: String::new(),
            kind: Kind::Filter,
            description: String::new(),
            author: String::new(),
            params: Vec::new(),
            extensions: Vec::new(),
            budget: Budget::Light,
        }
    }

    /// Checks what the module declared: its id, name and text lengths, its extensions, a budget its
    /// kind may have (only a filter may be heavy), and each parameter and its dialog layout.
    fn validate(&self) -> Result<()> {
        if !valid_id(&self.id) {
            return Err(Error::Manifest(format!("invalid id {:?}: use 1-64 characters from A-Z a-z 0-9 . _ -", self.id)));
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err(Error::Manifest("`name` must be 1-64 printable characters".into()));
        }
        if self.version.len() > 32 || self.description.len() > 1024 || self.author.len() > 128 {
            return Err(Error::Manifest("`version`, `description` or `author` is too long".into()));
        }
        self.validate_extensions()?;
        match (self.budget, self.kind) {
            (Budget::Light, _) | (Budget::Heavy, Kind::Filter) => {}
            (Budget::Heavy, Kind::Effect) => {
                return Err(Error::Manifest("a live effect runs while the document is drawn, so it can't be `heavy`".into()));
            }
            (Budget::Heavy, Kind::Exporter) => {
                return Err(Error::Manifest("an exporter always gets an export's budget: leave `budget` out".into()));
            }
        }
        for (i, (k, p)) in self.params.iter().enumerate() {
            if !valid_key(k) {
                return Err(Error::Manifest(format!("invalid parameter name {k:?}")));
            }
            p.spec.validate(k)?;
            p.ui.validate(self, i, k)?;
        }
        if self.params.iter().filter(|(_, p)| p.ui.group.is_none()).count() > MAX_UNGROUPED {
            return Err(Error::Manifest(format!("at most {MAX_UNGROUPED} parameters outside groups: give the others a `group`")));
        }
        Ok(())
    }

    /// An exporter lists 1 to [`MAX_EXTENSIONS`] distinct extensions of 1 to 16 lower-case
    /// letters and digits; other kinds list none.
    fn validate_extensions(&self) -> Result<()> {
        let exts = &self.extensions;
        match self.kind {
            Kind::Exporter => {}
            _ if exts.is_empty() => return Ok(()),
            _ => return Err(Error::Manifest("only an exporter declares `extensions`".into())),
        }
        if exts.is_empty() || exts.len() > MAX_EXTENSIONS {
            return Err(Error::Manifest(format!("an exporter declares 1 to {MAX_EXTENSIONS} `extensions`")));
        }
        for (i, e) in exts.iter().enumerate() {
            if !(1..=16).contains(&e.len()) || !e.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) {
                return Err(Error::Manifest(format!("extension {e:?}: use 1-16 lower-case letters and digits, without the dot")));
            }
            if exts.get(..i).is_some_and(|before| before.contains(e)) {
                return Err(Error::Manifest(format!("extension {e:?} is listed twice")));
            }
        }
        Ok(())
    }

    /// The parameters documented the way command and effect parameters are
    /// (`{amount: 0..100 (50), mode: "soft"|"hard" (soft), mono: bool (false)}`).
    pub fn params_doc(&self) -> String {
        let parts: Vec<String> = self
            .params
            .iter()
            .map(|(k, p)| match &p.spec {
                ParamSpec::Number { min, max, default } => format!("{k}: {min}..{max} ({})", default.clamp(*min, *max)),
                ParamSpec::Int { min, max, default } => format!("{k}: int {min}..{max} ({})", (*default).clamp(*min, *max)),
                ParamSpec::Bool { default } => format!("{k}: bool ({default})"),
                ParamSpec::Choice { options, .. } => {
                    let opts: Vec<String> = options.iter().map(|o| format!("{o:?}")).collect();
                    format!("{k}: {} ({})", opts.join("|"), p.spec.default_value().as_str().unwrap_or_default())
                }
                ParamSpec::Text { max_length, .. } => format!("{k}: string of at most {max_length} ({})", p.spec.default_value()),
                ParamSpec::NumberList { min, max, min_items, max_items, .. } => {
                    format!("{k}: {min_items} to {max_items} numbers {min}..{max} ({})", p.spec.default_value())
                }
            })
            .collect();
        format!("{{{}}}", parts.join(", "))
    }

    /// Every parameter at its default value.
    pub fn defaults(&self) -> Map<String, Value> {
        self.params.iter().map(|(k, p)| (k.clone(), p.spec.default_value())).collect()
    }

    /// Validates user `params` against the schema: missing keys take their defaults, numbers are
    /// clamped into range (whole numbers rounded), unknown keys are dropped; a value of the wrong
    /// type is an error.
    pub fn resolve_params(&self, params: &Value) -> Result<Map<String, Value>> {
        let given = match params {
            Value::Null => &Map::new(),
            Value::Object(m) => m,
            _ => return Err(Error::Params("plug-in parameters must be a JSON object".into())),
        };
        let mut out = Map::new();
        for (k, p) in &self.params {
            let spec = &p.spec;
            let Some(v) = given.get(k).filter(|v| !v.is_null()) else {
                out.insert(k.clone(), spec.default_value());
                continue;
            };
            let wrong = |what: &str| Error::Params(format!("`{k}` must be {what}"));
            let resolved = match spec {
                ParamSpec::Number { min, max, .. } => json!(v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?.clamp(*min, *max)),
                ParamSpec::Int { min, max, .. } => {
                    // Keep JSON integers exact: f64 cannot represent every i64 or its bounds.
                    let value = if let Some(i) = v.as_i64() {
                        i.clamp(*min, *max)
                    } else if v.is_u64() {
                        // Past i64::MAX, so past any maximum.
                        *max
                    } else {
                        let f = v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?;
                        (f.round() as i64).clamp(*min, *max)
                    };
                    json!(value)
                }
                ParamSpec::Bool { .. } => json!(v.as_bool().ok_or_else(|| wrong("true or false"))?),
                ParamSpec::Choice { options, .. } => {
                    let s = v.as_str().ok_or_else(|| wrong("a string"))?;
                    if !options.iter().any(|o| o == s) {
                        return Err(Error::Params(format!("`{k}` must be one of {}", options.join(", "))));
                    }
                    json!(s)
                }
                ParamSpec::Text { max_length, .. } => {
                    json!(v.as_str().ok_or_else(|| wrong("a string"))?.chars().take(*max_length).collect::<String>())
                }
                ParamSpec::NumberList { min, max, min_items, max_items, .. } => {
                    let items = v.as_array().ok_or_else(|| wrong("a list of numbers"))?;
                    if !(*min_items..=*max_items).contains(&items.len()) {
                        return Err(wrong(&format!("a list of {min_items} to {max_items} numbers")));
                    }
                    let numbers: Option<Vec<f64>> = items.iter().map(|x| x.as_f64().filter(|f| f.is_finite()).map(|f| f.clamp(*min, *max))).collect();
                    json!(numbers.ok_or_else(|| wrong("a list of numbers"))?)
                }
            };
            out.insert(k.clone(), resolved);
        }
        Ok(out)
    }
}

/// Reads the `params` object keeping its key order (serde_json's `Map` would sort it).
fn ordered_params<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<(String, Param)>, D::Error> {
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Vec<(String, Param)>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object of parameter specs")
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
            let mut out: Vec<(String, Param)> = Vec::new();
            while let Some((k, v)) = map.next_entry::<String, Param>()? {
                if out.len() >= MAX_PARAMS {
                    return Err(serde::de::Error::custom(format!("too many parameters (limit {MAX_PARAMS})")));
                }
                if out.iter().any(|(o, _)| *o == k) {
                    return Err(serde::de::Error::custom(format!("parameter `{k}` is declared twice")));
                }
                out.push((k, v));
            }
            Ok(out)
        }
    }
    d.deserialize_any(V)
}

fn params_as_object<S: serde::Serializer>(p: &[(String, Param)], s: S) -> std::result::Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut m = s.serialize_map(Some(p.len()))?;
    for (k, v) in p {
        m.serialize_entry(k, v)?;
    }
    m.end()
}

/// `[A-Za-z0-9._-]{1,64}`.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `[A-Za-z0-9_]{1,64}`, not starting with `_` and not a key the commands and dialogs use
/// themselves.
fn valid_key(k: &str) -> bool {
    (1..=64).contains(&k.len())
        && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !k.starts_with('_')
        && !matches!(k, "id" | "ids" | "params" | "preview")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{"id":"org.example.tone","name":"Tone","version":"1.2","kind":"effect",
        "params":{"amount":{"type":"number","min":0,"max":100,"default":50},
                  "mode":{"type":"choice","options":["soft","hard"],"default":"hard"},
                  "mono":{"type":"bool"},"seed":{"type":"int","min":0,"max":9}}}"#;

    #[test]
    fn parses_and_keeps_param_order() {
        let m = Manifest::parse(FULL.as_bytes()).unwrap();
        assert_eq!(m.id, "org.example.tone");
        assert_eq!(m.kind, Kind::Effect);
        assert_eq!(m.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["amount", "mode", "mono", "seed"]);
        assert_eq!(m.params_doc(), r#"{amount: 0..100 (50), mode: "soft"|"hard" (hard), mono: bool (false), seed: int 0..9 (0)}"#);
        assert_eq!(Value::Object(m.defaults()), json!({"amount": 50.0, "mode": "hard", "mono": false, "seed": 0}));
    }

    #[test]
    fn resolves_defaults_clamps_and_rejects_wrong_types() {
        let m = Manifest::parse(FULL.as_bytes()).unwrap();
        let p = m.resolve_params(&json!({"amount": 500, "seed": 3.6, "junk": 1})).unwrap();
        assert_eq!(Value::Object(p), json!({"amount": 100.0, "mode": "hard", "mono": false, "seed": 4}));
        assert!(m.resolve_params(&json!({"amount": "lots"})).is_err());
        assert!(m.resolve_params(&json!({"mode": "medium"})).is_err());
        assert!(m.resolve_params(&json!({"mono": 1})).is_err());
        assert!(m.resolve_params(&json!([1, 2])).is_err());
        assert!(m.resolve_params(&Value::Null).is_ok());
    }

    #[test]
    fn integer_parameters_preserve_precision_and_exact_bounds() {
        let m = Manifest::parse(
            br#"{"id":"org.example.generative","name":"Generative Pattern","kind":"filter",
                "params":{"seed":{"type":"int","min":-9007199254740995,"max":9007199254740995}}}"#,
        )
        .unwrap();
        for (given, expected) in [
            (json!(9007199254740992_i64), 9007199254740992_i64),
            (json!(9007199254740993_i64), 9007199254740993_i64),
            (json!(-9007199254740993_i64), -9007199254740993_i64),
            (json!(i64::MAX), 9007199254740995_i64),
            (json!(i64::MIN), -9007199254740995_i64),
            (json!(u64::MAX), 9007199254740995_i64),
            (json!(1e30), 9007199254740995_i64),
            (json!(-1e30), -9007199254740995_i64),
            (json!(3.6), 4_i64),
        ] {
            let resolved = m.resolve_params(&json!({"seed": given})).unwrap();
            assert_eq!(resolved["seed"], json!(expected), "input: {given}");
        }
    }

    #[test]
    fn numeric_defaults_stay_in_the_declared_range() {
        let m = Manifest::parse(
            br#"{"id":"org.example.defaults","name":"Defaults","kind":"filter",
                "params":{"count":{"type":"int","min":1,"max":10},
                          "seed":{"type":"int","min":-9007199254740995,"max":-9007199254740993,"default":0},
                          "amount":{"type":"number","min":0,"max":100,"default":500}}}"#,
        )
        .unwrap();
        let expected = json!({"count": 1, "seed": -9007199254740993_i64, "amount": 100.0});
        assert_eq!(Value::Object(m.defaults()), expected);
        assert_eq!(
            m.params_doc(),
            "{count: int 1..10 (1), seed: int -9007199254740995..-9007199254740993 (-9007199254740993), amount: 0..100 (100)}"
        );
        for given in [Value::Null, json!({}), json!({"count": null, "seed": null, "amount": null})] {
            assert_eq!(Value::Object(m.resolve_params(&given).unwrap()), expected);
        }
    }

    #[test]
    fn an_exporter_lists_its_extensions_and_other_kinds_list_none() {
        let m = Manifest::parse(br#"{"id":"org.example.plotter","name":"Plotter","kind":"exporter","extensions":["plt","plot"]}"#).unwrap();
        assert_eq!((m.kind, m.extensions.as_slice()), (Kind::Exporter, &["plt".to_string(), "plot".to_string()][..]));
        assert_eq!(serde_json::to_value(&m).unwrap()["extensions"], json!(["plt", "plot"]));
        let filter = Manifest::parse(br#"{"id":"a","name":"x","kind":"filter"}"#).unwrap();
        assert!(serde_json::to_value(&filter).unwrap().get("extensions").is_none());
        for bad in [
            r#"{"id":"a","name":"x","kind":"exporter"}"#,
            r#"{"id":"a","name":"x","kind":"exporter","extensions":[]}"#,
            r#"{"id":"a","name":"x","kind":"exporter","extensions":["PLT"]}"#,
            r#"{"id":"a","name":"x","kind":"exporter","extensions":[".plt"]}"#,
            r#"{"id":"a","name":"x","kind":"exporter","extensions":[""]}"#,
            r#"{"id":"a","name":"x","kind":"exporter","extensions":["abcdefghijklmnopq"]}"#,
            r#"{"id":"a","name":"x","kind":"exporter","extensions":["plt","plt"]}"#,
            r#"{"id":"a","name":"x","kind":"exporter","extensions":["a","b","c","d","e","f","g","h","i"]}"#,
            r#"{"id":"a","name":"x","kind":"filter","extensions":["plt"]}"#,
            r#"{"id":"a","name":"x","kind":"effect","extensions":["plt"]}"#,
        ] {
            assert!(Manifest::parse(bad.as_bytes()).is_err(), "{bad}");
        }
    }

    /// A filter may ask for a heavy budget; a live effect runs while the document is drawn and an
    /// exporter has an export's budget, so they can't. Light, the default, isn't written back.
    #[test]
    fn only_a_filter_asks_for_a_heavy_budget() {
        let heavy = Manifest::parse(br#"{"id":"org.example.route","name":"Route","kind":"filter","budget":"heavy"}"#).unwrap();
        assert_eq!(heavy.budget, Budget::Heavy);
        assert_eq!(serde_json::to_value(&heavy).unwrap()["budget"], json!("heavy"));
        for light in [r#"{"id":"a","name":"x","kind":"filter"}"#, r#"{"id":"a","name":"x","kind":"effect","budget":"light"}"#] {
            let m = Manifest::parse(light.as_bytes()).unwrap();
            assert_eq!(m.budget, Budget::Light);
            assert!(serde_json::to_value(&m).unwrap().get("budget").is_none(), "{light}");
        }
        for bad in [
            r#"{"id":"a","name":"x","kind":"effect","budget":"heavy"}"#,
            r#"{"id":"a","name":"x","kind":"exporter","extensions":["plt"],"budget":"heavy"}"#,
            r#"{"id":"a","name":"x","kind":"filter","budget":"huge"}"#,
            r#"{"id":"a","name":"x","kind":"filter","budget":true}"#,
        ] {
            assert!(Manifest::parse(bad.as_bytes()).is_err(), "{bad}");
        }
    }

    #[test]
    fn strings_and_number_lists_resolve_like_the_other_types() {
        let m = Manifest::parse(
            br#"{"id":"org.example.text","name":"Text","kind":"filter","params":{
                "label":{"type":"string","default":"Rose","maxLength":8},
                "angles":{"type":"numberList","min":0,"max":360,"default":[45,135],"minItems":1,"maxItems":3},
                "free":{"type":"string"}}}"#,
        )
        .unwrap();
        assert_eq!(Value::Object(m.defaults()), json!({"label": "Rose", "angles": [45.0, 135.0], "free": ""}));
        let p = m.resolve_params(&json!({"label": "Sunflowers", "angles": [-10, 90, 400]})).unwrap();
        assert_eq!((&p["label"], &p["angles"]), (&json!("Sunflowe"), &json!([0.0, 90.0, 360.0])), "cut and clamped");
        for bad in [json!({"label": 3}), json!({"angles": []}), json!({"angles": [1, 2, 3, 4]}), json!({"angles": [1, "x"]}), json!({"angles": 5})] {
            assert!(m.resolve_params(&bad).is_err(), "{bad}");
        }
        assert_eq!(
            m.params_doc(),
            r#"{label: string of at most 8 ("Rose"), angles: 1 to 3 numbers 0..360 ([45.0,135.0]), free: string of at most 256 ("")}"#
        );
    }

    #[test]
    fn up_to_256_parameters_with_at_most_64_outside_groups() {
        let manifest = |n: usize, grouped: usize| {
            let params: Vec<String> = (0..n)
                .map(|i| match i < grouped {
                    true => format!(r#""p{i}":{{"type":"bool","group":"G{}"}}"#, i % 4),
                    false => format!(r#""p{i}":{{"type":"bool"}}"#),
                })
                .collect();
            format!(r#"{{"id":"a","name":"x","kind":"filter","params":{{{}}}}}"#, params.join(","))
        };
        assert!(Manifest::parse(manifest(MAX_UNGROUPED, 0).as_bytes()).is_ok());
        assert!(Manifest::parse(manifest(MAX_UNGROUPED + 1, 0).as_bytes()).is_err(), "too many outside groups");
        let all = Manifest::parse(manifest(MAX_PARAMS, MAX_PARAMS - MAX_UNGROUPED).as_bytes()).unwrap();
        assert_eq!(all.params.len(), MAX_PARAMS);
        assert!(Manifest::parse(manifest(MAX_PARAMS + 1, MAX_PARAMS).as_bytes()).is_err(), "too many in all");
    }

    #[test]
    fn rejects_bad_manifests() {
        for bad in [
            "",
            "[]",
            "not json",
            r#"{"id":"","name":"x","kind":"filter"}"#,
            r#"{"id":"a b","name":"x","kind":"filter"}"#,
            r#"{"id":"a","name":"","kind":"filter"}"#,
            r#"{"id":"a","name":"x","kind":"panel"}"#,
            r#"{"id":"a","name":"x"}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"r":{"type":"number","min":5,"max":1,"default":1}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"c":{"type":"choice","options":[]}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"__x":{"type":"bool"}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"preview":{"type":"bool"}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"a":{"type":"bool"},"a":{"type":"bool"}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":[1]}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"t":{"type":"string","maxLength":0}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"t":{"type":"string","default":"abc","maxLength":2}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"l":{"type":"numberList","min":1,"max":0}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"l":{"type":"numberList","min":0,"max":1,"maxItems":0}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"l":{"type":"numberList","min":0,"max":1,"maxItems":257}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"l":{"type":"numberList","min":0,"max":1,"minItems":2,"maxItems":1}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"l":{"type":"numberList","min":0,"max":1,"minItems":2,"default":[1]}}}"#,
        ] {
            assert!(Manifest::parse(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(Manifest::parse(&[0xff, 0xfe]).is_err());
        assert!(Manifest::parse(&vec![b' '; MAX_MANIFEST_BYTES + 1]).is_err());
    }
}
