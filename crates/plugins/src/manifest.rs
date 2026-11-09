//! The plug-in manifest (`vc_manifest`) and parameter schema: what each parameter holds
//! ([`ParamSpec`]) and how dialogs show it ([`ParamUi`], in [`crate::dialog`]).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::dialog::ParamUi;
use crate::permissions::{self, API_LEVEL, PROGRESS_API, Permission};
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
/// Most menu commands a plug-in may add (`commands`).
pub const MAX_COMMANDS: usize = 32;
/// Longest command id, in characters.
pub const MAX_COMMAND_ID: usize = 32;

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

/// A menu command a plug-in adds (`commands`): the host lists it under Object › Plug-ins › the
/// plug-in's name (Effect › Plug-ins for a live effect's), runs it as the command
/// `plugin.<plug-in id>.<id>`, and calls the plug-in's `vc_run` with `_context.command` set to
/// `id` and the parameters it declares.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Command {
    /// `[a-z][a-z0-9_]*`, at most [`MAX_COMMAND_ID`] characters, unique within the manifest.
    pub id: String,
    /// The menu label: 1 to 64 printable characters, shown with bidirectional characters escaped
    /// ([`crate::permissions::display_text`]).
    pub label: String,
    /// Parameters in declaration (dialog) order, declared as the manifest's own are.
    #[serde(default, deserialize_with = "ordered_params", serialize_with = "params_as_object")]
    pub params: Vec<(String, Param)>,
}

impl Command {
    /// The label as menus and errors show it: control and bidirectional characters escaped, at
    /// most [`crate::permissions::MAX_SHOWN_NAME`] characters.
    pub fn shown_label(&self) -> String {
        crate::permissions::display_text(self.label.trim(), crate::permissions::MAX_SHOWN_NAME)
    }
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
    /// The menu commands it adds, in menu order (a filter or a live effect only).
    #[serde(default, deserialize_with = "ordered_commands", skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<Command>,
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
            commands: Vec::new(),
        }
    }

    /// The manifest command `c` runs with: this one with `c`'s label as its name and `c`'s
    /// parameters as its own, so dialogs show them and [`Manifest::resolve_params`] checks them.
    pub fn for_command(&self, c: &Command) -> Manifest {
        Manifest { name: c.label.clone(), params: c.params.clone(), commands: Vec::new(), ..self.clone() }
    }

    /// The command with id `id`.
    pub fn command(&self, id: &str) -> Option<&Command> {
        self.commands.iter().find(|c| c.id == id)
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
        if let Some(names) = &self.permissions
            && let Some(host) =
                permissions::validate(self.kind, !self.commands.is_empty(), names)?.into_iter().find(|p| permissions::HOST.contains(p))
            && self.api < PROGRESS_API
        {
            let permission = host.name();
            return Err(Error::Manifest(format!("permission `{permission}` needs plug-in API level {PROGRESS_API}: give `api` {PROGRESS_API}")));
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
        self.validate_params()?;
        self.validate_commands()
    }

    /// Checks each parameter and its dialog layout.
    fn validate_params(&self) -> Result<()> {
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

    /// A filter or a live effect lists at most [`MAX_COMMANDS`] commands, each with a distinct
    /// id, a printable label and parameters declared as the manifest's own are. A heavy filter
    /// lists none. The error names the command: its id, or its place in the list.
    fn validate_commands(&self) -> Result<()> {
        if self.commands.is_empty() {
            return Ok(());
        }
        if self.kind == Kind::Exporter {
            return Err(Error::Manifest("only a filter or a live effect declares `commands`".into()));
        }
        if self.budget == Budget::Heavy {
            return Err(Error::Manifest("a heavy filter runs in the background, so it can't declare `commands`".into()));
        }
        if self.commands.len() > MAX_COMMANDS {
            return Err(Error::Manifest(format!("at most {MAX_COMMANDS} `commands`")));
        }
        for (i, c) in self.commands.iter().enumerate() {
            let name = command_name(i, &c.id);
            if !valid_command_id(&c.id) {
                return Err(Error::Manifest(format!(
                    "command {name}: use an id of 1-{MAX_COMMAND_ID} characters from a-z 0-9 _ that starts with a letter"
                )));
            }
            let label = c.label.trim();
            if label.is_empty() || label.chars().count() > 64 || label.chars().any(char::is_control) {
                return Err(Error::Manifest(format!("command {name}: `label` must be 1-64 printable characters")));
            }
            if self.commands.get(..i).is_some_and(|before| before.iter().any(|b| b.id == c.id)) {
                return Err(Error::Manifest(format!("command {name} is declared twice")));
            }
            self.for_command(c).validate_params().map_err(|e| {
                let e = match e {
                    Error::Manifest(why) => why,
                    other => other.to_string(),
                };
                Error::Manifest(format!("command {name}: {e}"))
            })?;
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

    /// What the plug-in may do: the permissions it lists, else every one its kind may list
    /// ([`Permission::allowed`]: a live effect with commands also changes the document and keeps
    /// data with it).
    pub fn permissions(&self) -> Vec<Permission> {
        let commands = !self.commands.is_empty();
        match &self.permissions {
            Some(names) => permissions::validate(self.kind, commands, names).unwrap_or_default(),
            None => Permission::allowed(self.kind, commands).to_vec(),
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

/// Reads the `commands` list one entry at a time from the manifest itself, so each command's
/// `params` keep their declaration order and a repeated parameter is refused. A malformed entry
/// is an error that names it (its id when the id comes before the fault, otherwise its place in
/// the list), and a list is refused at its first entry past [`MAX_COMMANDS`].
fn ordered_commands<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<Command>, D::Error> {
    use serde::de::{Error as _, IgnoredAny, MapAccess, SeqAccess, Visitor};

    /// A command's `params`, read in declaration order.
    struct Params(Vec<(String, Param)>);
    impl<'de> Deserialize<'de> for Params {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            ordered_params(d).map(Params)
        }
    }

    /// The entry at place `.0` in the list.
    struct Entry(usize);
    impl<'de> serde::de::DeserializeSeed<'de> for Entry {
        type Value = Command;
        fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> std::result::Result<Command, D::Error> {
            d.deserialize_any(self)
        }
    }
    impl<'de> Visitor<'de> for Entry {
        type Value = Command;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "command [{}] as {{id, label, params?}}", self.0)
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Command, A::Error> {
            let (mut id, mut label, mut params) = (None::<String>, None::<String>, None::<Params>);
            let named = |id: &Option<String>, e: &dyn std::fmt::Display| {
                A::Error::custom(format!("command {}: {e}", command_name(self.0, id.as_deref().unwrap_or_default())))
            };
            while let Some(key) = map.next_key::<String>().map_err(|e| named(&id, &e))? {
                let repeated = match key.as_str() {
                    "id" if id.is_none() => {
                        id = Some(map.next_value().map_err(|e| named(&id, &e))?);
                        false
                    }
                    "label" if label.is_none() => {
                        label = Some(map.next_value().map_err(|e| named(&id, &e))?);
                        false
                    }
                    "params" if params.is_none() => {
                        params = Some(map.next_value().map_err(|e| named(&id, &e))?);
                        false
                    }
                    "id" | "label" | "params" => true,
                    _ => {
                        map.next_value::<IgnoredAny>().map_err(|e| named(&id, &e))?;
                        false
                    }
                };
                if repeated {
                    return Err(named(&id, &format_args!("duplicate field `{key}`")));
                }
            }
            let Some(id) = id else {
                return Err(named(&None, &"missing field `id`"));
            };
            let Some(label) = label else {
                return Err(named(&Some(id), &"missing field `label`"));
            };
            Ok(Command { id, label, params: params.map(|p| p.0).unwrap_or_default() })
        }
    }

    struct List;
    impl<'de> Visitor<'de> for List {
        type Value = Vec<Command>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("`commands` as a list of {id, label, params?}")
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_map<A: MapAccess<'de>>(self, _: A) -> std::result::Result<Self::Value, A::Error> {
            Err(A::Error::custom("`commands` must be a list of {id, label, params?}"))
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while out.len() < MAX_COMMANDS {
                match seq.next_element_seed(Entry(out.len()))? {
                    Some(c) => out.push(c),
                    None => return Ok(out),
                }
            }
            match seq.next_element::<IgnoredAny>()? {
                Some(_) => Err(A::Error::custom(format!("at most {MAX_COMMANDS} `commands`"))),
                None => Ok(out),
            }
        }
    }

    d.deserialize_any(List)
}

/// How errors name the command at place `i` with id `id`: the id in backticks, escaped and capped
/// as prompts show text, or `[i]` when it has none.
fn command_name(i: usize, id: &str) -> String {
    match id {
        "" => format!("[{i}]"),
        id => format!("`{}`", crate::permissions::display_text(id, crate::permissions::MAX_SHOWN_FIELD)),
    }
}

/// `[a-z][a-z0-9_]*`, 1 to [`MAX_COMMAND_ID`] characters.
fn valid_command_id(id: &str) -> bool {
    id.len() <= MAX_COMMAND_ID
        && id.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
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
        // A live effect's commands may change the document: one with commands gets `doc.write`
        // and `state` too, and may list them.
        let commands = br#"{"id":"a","name":"x","kind":"effect","commands":[{"id":"c","label":"C"}]}"#;
        let with_commands = Manifest::parse(commands).unwrap();
        assert_eq!(with_commands.permissions(), [Permission::DocRead, Permission::DocWrite, Permission::State, Permission::EffectRender]);
        let listing =
            br#"{"id":"a","name":"x","kind":"effect","permissions":["doc.read","effect.render","doc.write"],"commands":[{"id":"c","label":"C"}]}"#;
        assert_eq!(Manifest::parse(listing).unwrap().permissions(), [Permission::DocRead, Permission::DocWrite, Permission::EffectRender]);
        let without = br#"{"id":"a","name":"x","kind":"effect","permissions":["doc.read","effect.render","doc.write"]}"#;
        assert!(Manifest::parse(without).unwrap_err().to_string().contains("`doc.write` doesn't apply"));
        let exporter = Manifest::parse(br#"{"id":"a","name":"x","kind":"exporter","extensions":["plt"]}"#).unwrap();
        assert_eq!(exporter.permissions(), [Permission::DocRead, Permission::ExportWrite]);
        assert!(!Manifest::parse(br#"{"id":"a","name":"x","kind":"filter","permissions":[]}"#).unwrap().asks_permissions());
        // Level 2 adds `progress`, which only a level-2 manifest may list.
        let level2 = Manifest::parse(br#"{"id":"a","name":"x","kind":"filter","api":2,"permissions":["progress","doc.read"]}"#).unwrap();
        assert_eq!((level2.api, level2.permissions()), (2, vec![Permission::DocRead, Permission::Progress]));
        assert_eq!(serde_json::to_value(&level2).unwrap()["api"], json!(2));
        let plain2 = Manifest::parse(br#"{"id":"a","name":"x","kind":"filter","api":2}"#).unwrap();
        assert!(!plain2.permissions().contains(&Permission::Progress), "never implied");
        let storing = Manifest::parse(br#"{"id":"a","name":"x","kind":"filter","api":2,"permissions":["cache","settings"]}"#).unwrap();
        assert_eq!(storing.permissions(), [Permission::Settings, Permission::Cache]);
        assert!(!plain2.permissions().iter().any(|p| permissions::HOST.contains(p)), "never implied");
        for (bad, says) in [
            (r#"{"id":"a","name":"x","kind":"filter","permissions":["doc.read","net.fetch"]}"#, "unknown permission `net.fetch`"),
            (r#"{"id":"a","name":"x","kind":"filter","permissions":["ui.panel"]}"#, "reserved"),
            (r#"{"id":"a","name":"x","kind":"effect","permissions":["doc.read"]}"#, "effect.render"),
            (r#"{"id":"a","name":"x","kind":"filter","permissions":"doc.read"}"#, "manifest"),
            (r#"{"id":"a","name":"x","kind":"filter","permissions":["progress"]}"#, "`progress` needs plug-in API level 2"),
            (r#"{"id":"a","name":"x","kind":"filter","api":1,"permissions":["progress"]}"#, "`progress` needs plug-in API level 2"),
            (r#"{"id":"a","name":"x","kind":"filter","permissions":["settings"]}"#, "`settings` needs plug-in API level 2"),
            (r#"{"id":"a","name":"x","kind":"filter","api":1,"permissions":["doc.read","cache"]}"#, "`cache` needs plug-in API level 2"),
            (r#"{"id":"a","name":"x","kind":"filter","api":3}"#, "`api` is 3"),
            (r#"{"id":"a","name":"x","kind":"filter","api":0}"#, "`api` is 0"),
            (r#"{"id":"a","name":"x","kind":"filter","api":-1}"#, "manifest"),
        ] {
            let e = Manifest::parse(bad.as_bytes()).unwrap_err().to_string();
            assert!(e.contains(says), "{bad}: {e}");
        }
    }

    /// A filter or a live effect lists menu commands with their own parameters; a `menu` key is
    /// ignored. A malformed entry, a bad or repeated id, a label that isn't printable and too many
    /// commands are refused, and the error names the command.
    #[test]
    fn commands_are_checked_and_errors_name_them() {
        let m = Manifest::parse(
            br#"{"id":"org.example.tidy","name":"Tidy","kind":"filter","commands":[
                {"id":"tidy_all","label":"Tidy All","menu":"File"},
                {"id":"grow","label":"Grow","params":{"mode":{"type":"bool"},"by":{"type":"number","min":0,"max":10,"default":2}}}]}"#,
        )
        .unwrap();
        assert_eq!(m.commands.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["tidy_all", "grow"]);
        let grow = m.command("grow").unwrap();
        assert_eq!(grow.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["mode", "by"], "declaration order");
        let run = m.for_command(grow);
        assert_eq!((run.name.as_str(), run.id.as_str()), ("Grow", "org.example.tidy"));
        assert_eq!(Value::Object(run.resolve_params(&json!({"by": 50})).unwrap()), json!({"by": 10.0, "mode": false}));
        let written = serde_json::to_value(&m).unwrap();
        assert_eq!(written["commands"][1]["params"]["by"]["max"], json!(10.0));
        assert!(written["commands"][0].get("menu").is_none(), "a menu key is not kept");
        assert!(serde_json::to_value(Manifest::parse(br#"{"id":"a","name":"x","kind":"filter"}"#).unwrap()).unwrap().get("commands").is_none());
        let effect = Manifest::parse(br#"{"id":"a","name":"x","kind":"effect","commands":[{"id":"c","label":"C"}]}"#).unwrap();
        assert_eq!(effect.commands.len(), 1);
        let label = |l: &str| Command { id: "c".into(), label: l.into(), params: Vec::new() }.shown_label();
        assert_eq!(label("Flip\u{202E}txt.exe"), "Flip\\u{202E}txt.exe");

        let many: Vec<String> = (0..=MAX_COMMANDS).map(|i| format!(r#"{{"id":"c{i}","label":"C"}}"#)).collect();
        let too_many = format!(r#"{{"id":"a","name":"x","kind":"filter","commands":[{}]}}"#, many.join(","));
        for (bad, says) in [
            (r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"ok","label":"A"},{"label":"B"}]}"#, "command [1]: missing field `id`"),
            (r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"nolabel"}]}"#, "command `nolabel`: missing field `label`"),
            (r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"Upper","label":"A"}]}"#, "command `Upper`: use an id"),
            (r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"9lives","label":"A"}]}"#, "command `9lives`: use an id"),
            (r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"a.b","label":"A"}]}"#, "command `a.b`: use an id"),
            (r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"","label":"A"}]}"#, "command [0]: use an id"),
            (r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"x","label":" "}]}"#, "command `x`: `label`"),
            (r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"x","label":"a\nb"}]}"#, "command `x`: `label`"),
            (r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"x","label":"A"},{"id":"x","label":"B"}]}"#, "command `x` is declared twice"),
            (
                r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"x","label":"A","params":{"r":{"type":"number","min":5,"max":1,"default":1}}}]}"#,
                "command `x`: parameter `r`: bad range",
            ),
            (
                r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"x","label":"A","params":{"id":{"type":"bool"}}}]}"#,
                "command `x`: invalid parameter",
            ),
            (
                r#"{"id":"a","name":"x","kind":"filter","commands":[{"id":"x","label":"A","params":{"r":{"type":"bool"},"r":{"type":"int"}}}]}"#,
                "command `x`: parameter `r` is declared twice",
            ),
            (r#"{"id":"a","name":"x","kind":"filter","commands":[1]}"#, "command [0]"),
            (r#"{"id":"a","name":"x","kind":"filter","commands":{"id":"x"}}"#, "must be a list"),
            (r#"{"id":"a","name":"x","kind":"exporter","extensions":["plt"],"commands":[{"id":"x","label":"A"}]}"#, "only a filter or a live effect"),
            (r#"{"id":"a","name":"x","kind":"filter","budget":"heavy","commands":[{"id":"x","label":"A"}]}"#, "heavy filter"),
            (too_many.as_str(), "at most 32 `commands`"),
        ] {
            let e = Manifest::parse(bad.as_bytes()).unwrap_err().to_string();
            assert!(e.contains(says), "{bad}: {e}");
        }
        let long_id = format!(r#"{{"id":"a","name":"x","kind":"filter","commands":[{{"id":"{}","label":"A"}}]}}"#, "c".repeat(MAX_COMMAND_ID + 1));
        assert!(Manifest::parse(long_id.as_bytes()).is_err());
        let hidden =
            Manifest::parse("{\"id\":\"a\",\"name\":\"x\",\"kind\":\"filter\",\"commands\":[{\"id\":\"x\u{202E}\",\"label\":\"A\"}]}".as_bytes());
        assert!(hidden.unwrap_err().to_string().contains("command `x\\u{202E}`"), "an id in an error is escaped");
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
