//! The plug-in manifest (`vc_manifest`) and parameter schema: what each parameter holds
//! ([`ParamSpec`]) and how dialogs show it ([`ParamUi`], in [`crate::dialog`]).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::dialog::ParamUi;
use crate::permissions::{self, API_LEVEL, Permission};
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
    /// A color (a color button that opens a color picker). Its default, and a value passed to
    /// `plugin.run`, is a color object as paints hold one (`{"model": "rgb", "r", "g", "b"}`, or
    /// `cmyk`, `gray`, `lab`) or a hex string (`"#3a7bd5"`); the plug-in always gets a color object.
    Color {
        /// As the manifest gives it (`null`: black), read with [`color_value`] when used, so a hex
        /// default follows the working RGB space in use then, not the one at install.
        #[serde(default)]
        default: Value,
    },
}

/// A color parameter's value: a color object, or a hex string (display sRGB, converted into the
/// active working RGB space as the color picker's choices are), its components clamped into range.
pub fn color_value(v: &Value) -> Option<vectorcraft_color::Color> {
    use vectorcraft_color::{Color, cms};
    let c = match v {
        Value::String(s) => match Color::from_hex(s)? {
            Color::Rgb { r, g, b } => {
                let [r, g, b] = cms::active().srgb_to_rgb([r, g, b]);
                Color::rgb(r, g, b)
            }
            other => other,
        },
        Value::Object(_) => serde_json::from_value(v.clone()).ok()?,
        _ => return None,
    };
    Some(crate::objects::clamp_color(c))
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
fn first_api() -> u32 {
    1
}
fn is_first_api(api: &u32) -> bool {
    *api == 1
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
            ParamSpec::Color { default } => serde_json::to_value(color_value(default).unwrap_or_default()).unwrap_or_default(),
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
            ParamSpec::Color { default } if !default.is_null() && color_value(default).is_none() => {
                bad("a color default must be a color object or a hex string such as \"#3a7bd5\"")
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
    /// The plug-in API level it was built for, 1 to [`API_LEVEL`] (1 when left out).
    #[serde(default = "first_api", skip_serializing_if = "is_first_api")]
    pub api: u32,
    /// The permissions it asks for ([`crate::permissions`]), which the user approves when it is
    /// installed. Left out, it gets the ones its kind implies ([`Permission::implied`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,
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
    /// A live effect at `api` 2 that returns painted pieces ([`crate::painted`]): `true` when the
    /// manifest has `"paintOutput": true`, the only value the key may hold.
    #[serde(default, rename = "paintOutput", deserialize_with = "true_only", skip_serializing_if = "std::ops::Not::not")]
    pub paint_output: bool,
}

/// Reads a key that may only be `true` when present (`paintOutput`).
fn true_only<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<bool, D::Error> {
    match Value::deserialize(d)? {
        Value::Bool(true) => Ok(true),
        _ => Err(serde::de::Error::custom("`paintOutput` may only be true")),
    }
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
            api: 1,
            permissions: None,
            params: Vec::new(),
            extensions: Vec::new(),
            budget: Budget::Light,
            paint_output: false,
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
        if !(1..=API_LEVEL).contains(&self.api) {
            return Err(Error::Manifest(format!("`api` is {}; this host implements plug-in API levels 1 to {API_LEVEL}", self.api)));
        }
        if let Some(names) = &self.permissions {
            permissions::validate(self.kind, names)?;
        }
        if self.paint_output && (self.kind != Kind::Effect || self.api < 2) {
            return Err(Error::Manifest("`paintOutput` is only for a live effect (`kind` \"effect\") at `api` 2".into()));
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
                ParamSpec::Color { default } => format!("{k}: color object or hex ({})", color_value(default).unwrap_or_default().to_hex()),
            })
            .collect();
        format!("{{{}}}", parts.join(", "))
    }

    /// Every parameter at its default value.
    pub fn defaults(&self) -> Map<String, Value> {
        self.params.iter().map(|(k, p)| (k.clone(), p.spec.default_value())).collect()
    }

    /// What the plug-in may do: the permissions it lists, else the ones its kind implies.
    pub fn permissions(&self) -> Vec<Permission> {
        match &self.permissions {
            Some(names) => permissions::validate(self.kind, names).unwrap_or_default(),
            None => Permission::implied(self.kind).to_vec(),
        }
    }

    /// Does installing it need the user's approval: does it list permissions?
    pub fn asks_permissions(&self) -> bool {
        self.permissions.as_ref().is_some_and(|p| !p.is_empty())
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
                ParamSpec::Color { .. } => {
                    let c = color_value(v).ok_or_else(|| wrong("a color object or a hex string such as \"#3a7bd5\""))?;
                    serde_json::to_value(c).map_err(|e| Error::Params(format!("`{k}`: {e}")))?
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

    /// A manifest lists the permissions it asks for, checked against its kind; without the key it
    /// gets the ones its kind implies, as plug-ins did before manifests listed them, and needs no
    /// approval. `api` is 1 when left out, and a later level is refused.
    #[test]
    fn permissions_and_api_level() {
        let listed = Manifest::parse(br#"{"id":"a","name":"x","kind":"filter","api":1,"permissions":["doc.write","doc.read"]}"#).unwrap();
        assert_eq!(listed.permissions(), [Permission::DocRead, Permission::DocWrite]);
        assert!(listed.asks_permissions());
        assert_eq!(serde_json::to_value(&listed).unwrap()["permissions"], json!(["doc.write", "doc.read"]));
        assert!(serde_json::to_value(&listed).unwrap().get("api").is_none(), "level 1 isn't written back");
        let plain = Manifest::parse(br#"{"id":"a","name":"x","kind":"filter"}"#).unwrap();
        assert_eq!((plain.api, plain.asks_permissions()), (1, false));
        assert_eq!(plain.permissions(), [Permission::DocRead, Permission::DocWrite, Permission::State]);
        assert!(serde_json::to_value(&plain).unwrap().get("permissions").is_none());
        let effect = Manifest::parse(br#"{"id":"a","name":"x","kind":"effect"}"#).unwrap();
        assert_eq!(effect.permissions(), [Permission::DocRead, Permission::EffectRender]);
        let exporter = Manifest::parse(br#"{"id":"a","name":"x","kind":"exporter","extensions":["plt"]}"#).unwrap();
        assert_eq!(exporter.permissions(), [Permission::DocRead, Permission::ExportWrite]);
        assert!(!Manifest::parse(br#"{"id":"a","name":"x","kind":"filter","permissions":[]}"#).unwrap().asks_permissions());
        for (bad, says) in [
            (r#"{"id":"a","name":"x","kind":"filter","permissions":["doc.read","net.fetch"]}"#, "unknown permission `net.fetch`"),
            (r#"{"id":"a","name":"x","kind":"filter","permissions":["ui.panel"]}"#, "reserved"),
            (r#"{"id":"a","name":"x","kind":"effect","permissions":["doc.read"]}"#, "effect.render"),
            (r#"{"id":"a","name":"x","kind":"filter","permissions":"doc.read"}"#, "manifest"),
            (r#"{"id":"a","name":"x","kind":"filter","api":3}"#, "`api` is 3"),
            (r#"{"id":"a","name":"x","kind":"filter","api":0}"#, "`api` is 0"),
            (r#"{"id":"a","name":"x","kind":"filter","api":-1}"#, "manifest"),
        ] {
            let e = Manifest::parse(bad.as_bytes()).unwrap_err().to_string();
            assert!(e.contains(says), "{bad}: {e}");
        }
    }

    /// A live effect at `api` 2 may declare `"paintOutput": true`, and only that value; any other
    /// kind or level, and `false`, `null` or another type, is refused naming the key. Without the
    /// key, at any level, the effect returns geometry only.
    #[test]
    fn only_a_level_2_live_effect_declares_paint_output() {
        let painted = Manifest::parse(br#"{"id":"org.example.tone","name":"Tone","kind":"effect","api":2,"paintOutput":true}"#).unwrap();
        assert!(painted.paint_output);
        assert_eq!(serde_json::to_value(&painted).unwrap()["paintOutput"], json!(true));
        let listed =
            Manifest::parse(br#"{"id":"a","name":"x","kind":"effect","api":2,"paintOutput":true,"permissions":["doc.read","effect.render"]}"#)
                .unwrap();
        assert!(listed.paint_output);
        for plain in [r#"{"id":"a","name":"x","kind":"effect","api":2}"#, r#"{"id":"a","name":"x","kind":"effect"}"#] {
            let m = Manifest::parse(plain.as_bytes()).unwrap();
            assert!(!m.paint_output, "{plain}");
            assert!(serde_json::to_value(&m).unwrap().get("paintOutput").is_none(), "{plain}");
        }
        for bad in [
            r#"{"id":"a","name":"x","kind":"effect","api":2,"paintOutput":false}"#,
            r#"{"id":"a","name":"x","kind":"effect","api":2,"paintOutput":null}"#,
            r#"{"id":"a","name":"x","kind":"effect","api":2,"paintOutput":1}"#,
            r#"{"id":"a","name":"x","kind":"effect","api":2,"paintOutput":"true"}"#,
            r#"{"id":"a","name":"x","kind":"effect","paintOutput":true}"#,
            r#"{"id":"a","name":"x","kind":"effect","api":1,"paintOutput":true}"#,
            r#"{"id":"a","name":"x","kind":"filter","api":2,"paintOutput":true}"#,
            r#"{"id":"a","name":"x","kind":"exporter","api":2,"extensions":["plt"],"paintOutput":true}"#,
            r#"{"id":"a","name":"x","kind":"effect","api":2,"paintOutput":true,"permissions":["doc.read"]}"#,
        ] {
            let e = Manifest::parse(bad.as_bytes()).unwrap_err().to_string();
            assert!(e.contains("paintOutput") || e.contains("effect.render"), "{bad}: {e}");
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

    /// A color's default and value may be a color object or a hex string; the plug-in always gets
    /// a color object, its components clamped. Without a default it is black.
    #[test]
    fn colors_resolve_to_color_objects() {
        let m = Manifest::parse(
            br##"{"id":"org.example.grid","name":"Grid","kind":"filter","params":{
                "line":{"type":"color","default":"#ff0000"},
                "ink":{"type":"color","default":{"model":"cmyk","c":1,"m":0,"y":0,"k":2}},
                "plain":{"type":"color"}}}"##,
        )
        .unwrap();
        assert_eq!(
            Value::Object(m.defaults()),
            json!({"line": {"model": "rgb", "r": 1.0, "g": 0.0, "b": 0.0},
                   "ink": {"model": "cmyk", "c": 1.0, "m": 0.0, "y": 0.0, "k": 1.0},
                   "plain": {"model": "rgb", "r": 0.0, "g": 0.0, "b": 0.0}})
        );
        let p = m.resolve_params(&json!({"line": "#0000ff", "ink": {"model": "gray", "k": 0.5}})).unwrap();
        assert_eq!(p["line"], json!({"model": "rgb", "r": 0.0, "g": 0.0, "b": 1.0}));
        assert_eq!(p["ink"], json!({"model": "gray", "k": 0.5}));
        for bad in [json!({"line": "not a color"}), json!({"line": 3}), json!({"line": {"model": "hsv", "h": 1}})] {
            assert!(m.resolve_params(&bad).is_err(), "{bad}");
        }
        assert!(m.params_doc().starts_with("{line: color object or hex (#ff0000)"));
        assert!(Manifest::parse(br#"{"id":"a","name":"x","kind":"filter","params":{"c":{"type":"color","default":"nope"}}}"#).is_err());
    }

    /// A hex color is display sRGB: in a wide-gamut working RGB space it becomes the working RGB
    /// color that shows as that hex, read when used, while the manifest keeps the default as written.
    #[test]
    fn hex_colors_follow_the_working_rgb_space() {
        const CHILD: &str = "VECTORCRAFT_TEST_PLUGIN_COLOR_PROFILE";
        if std::env::var_os(CHILD).is_none() {
            // Color settings are process-wide: exercise this profile in a separate test process.
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "manifest::tests::hex_colors_follow_the_working_rgb_space", "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
                "isolated color profile test failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }
        use vectorcraft_color::{Color, cms};
        let m =
            Manifest::parse(br##"{"id":"org.example.grid","name":"Grid","kind":"filter","params":{"line":{"type":"color","default":"#3a7bd5"}}}"##)
                .unwrap();
        cms::set_active(&cms::ColorSettings { rgb: cms::WIDE_GAMUT_RGB.into(), ..Default::default() }).unwrap();
        for c in [m.defaults()["line"].clone(), m.resolve_params(&json!({"line": "#3a7bd5"})).unwrap()["line"].clone()] {
            let c: Color = serde_json::from_value(c).unwrap();
            assert_ne!(c, Color::rgb8(0x3a, 0x7b, 0xd5), "converted into the working space");
            assert_eq!(c.to_hex(), "#3a7bd5", "shows as the hex given");
        }
        assert_eq!(serde_json::to_value(&m).unwrap()["params"]["line"]["default"], json!("#3a7bd5"), "kept as written");
        cms::set_active(&cms::ColorSettings::default()).unwrap();
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
