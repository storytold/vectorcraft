//! Character Styles and Paragraph Styles panels.
//!
//! A style is a named, partial set of attributes ([`TextStyleDef`]); text using it records the
//! name in `CharStyle::style_name` / `ParaStyle::style_name`. Applying a style sets its attributes
//! (Clear Overrides first resets to the Normal style). Redefining a style updates its users but keeps
//! their local overrides: an attribute changes only where it still had the old style's value.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use vectorcraft_doc::{CharStyle, Document, NodeId, NodeKind, ParaStyle, TextStyleDef};

use super::typecmd::{refresh_bounds, text_targets};
use super::*;

pub const NORMAL_CHAR: &str = "[Normal Character Style]";
pub const NORMAL_PARA: &str = "[Normal Paragraph Style]";

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Char,
    Para,
}

impl Kind {
    fn cmd(self, verb: &str) -> String {
        format!("{}.{verb}", if self == Kind::Char { "charStyle" } else { "paraStyle" })
    }
    fn normal(self) -> &'static str {
        if self == Kind::Char { NORMAL_CHAR } else { NORMAL_PARA }
    }
    fn defs(self, d: &Document) -> &Vec<TextStyleDef> {
        if self == Kind::Char { &d.char_styles } else { &d.para_styles }
    }
    fn defs_mut(self, d: &mut Document) -> &mut Vec<TextStyleDef> {
        if self == Kind::Char { &mut d.char_styles } else { &mut d.para_styles }
    }
    /// Attributes of style `name` (the Normal style is built in: empty unless redefined).
    fn attrs(self, d: &Document, name: &str) -> Option<Map<String, Value>> {
        match self.defs(d).iter().find(|s| s.name == name) {
            Some(s) => Some(s.attrs.clone()),
            None if name == self.normal() => Some(Map::new()),
            None => None,
        }
    }
}

/// One command spec (the `cmd!` macro wants literals; these ids are built with `concat!`).
const fn spec(id: &'static str, label: &'static str, menu: &'static [&'static str], params: &'static str, run: Run, journal: bool) -> CommandSpec {
    CommandSpec { id, label, menu, shortcut: None, params, enabled: has_doc, run, journal }
}

macro_rules! style_cmds {
    ($kind:expr, $p:literal, $menu:literal, $what:literal) => {
        vec![
            spec(concat!($p, ".list"), concat!($what, " Styles"), &[], "{} → {styles: [{name, attrs, uses}]}", |s, _| list(s, $kind), false),
            spec(
                concat!($p, ".new"),
                concat!("New ", $what, " Style"),
                &["Window", "Type", $menu],
                "{name? (trimmed, as rename trims `to`), attrs?: {…} (default: selected text), id?, start?: byte, end?: byte} → {name}; a nonempty character range samples its first selected character; start alone keeps insertion affinity; paragraph styles sample start",
                |s, p| new(s, p, $kind),
                true,
            ),
            spec(
                concat!($p, ".apply"),
                concat!("Apply ", $what, " Style"),
                &[],
                "{name, clearOverrides?, ids?|id?, start?, end? (id with start and end: a Type tool range; character styles style it, paragraph styles apply to the paragraphs it touches)} → {count}",
                |s, p| apply(s, p, $kind),
                true,
            ),
            spec(
                concat!($p, ".redefine"),
                concat!("Redefine ", $what, " Style"),
                &["Window", "Type", $menu],
                "{name, id?, start?: byte, end?: byte} from selected text; nonempty character ranges sample their first selected character, start alone keeps insertion affinity, paragraph styles sample start; users keep their overrides",
                |s, p| redefine(s, p, $kind),
                true,
            ),
            spec(
                concat!($p, ".setAttrs"),
                concat!($what, " Style Options"),
                &[],
                "{name, attrs: {…}} replace the style's attributes; users keep their overrides",
                |s, p| set_attrs(s, p, $kind),
                true,
            ),
            spec(
                concat!($p, ".duplicate"),
                concat!("Duplicate ", $what, " Style"),
                &["Window", "Type", $menu],
                "{name} → {name}",
                |s, p| duplicate(s, p, $kind),
                true,
            ),
            spec(concat!($p, ".rename"), concat!("Rename ", $what, " Style"), &[], "{name, to}", |s, p| rename(s, p, $kind), true),
            spec(
                concat!($p, ".delete"),
                concat!("Delete ", $what, " Style"),
                &["Window", "Type", $menu],
                "{name} (text keeps its look)",
                |s, p| delete(s, p, $kind),
                true,
            ),
        ]
    };
}

pub fn specs() -> Vec<CommandSpec> {
    let mut v = style_cmds!(Kind::Char, "charStyle", "Character Styles", "Character");
    v.extend(style_cmds!(Kind::Para, "paraStyle", "Paragraph Styles", "Paragraph"));
    v
}

// ---------- attribute plumbing ----------

/// `v` as a JSON map with one key per attribute: Auto alignment, which files save as its
/// direction's alignment plus `justify_auto` (see `vectorcraft_doc::ParaStyle`), is `"justify":
/// "Auto"`, so a style's `justify` replaces it.
fn to_map<T: Serialize>(v: &T) -> Option<Map<String, Value>> {
    let Ok(Value::Object(mut m)) = serde_json::to_value(v) else { return None };
    if m.remove("justify_auto") == Some(Value::Bool(true)) {
        m.insert("justify".into(), Value::from("Auto"));
    }
    Some(m)
}

/// A style's attributes as a JSON map, without the style name.
fn to_attrs<T: Serialize>(v: &T) -> Map<String, Value> {
    let mut m = to_map(v).unwrap_or_default();
    m.remove("style_name");
    m
}

/// Every character attribute of `v` as a JSON map, without the style name: those the run's JSON
/// leaves out at their defaults (Auto leading and kerning, no OpenType features, Roman baseline
/// alignment…) are written out. A style captured from text sets them all; an attribute missing
/// from a style instead leaves the text's own value alone.
pub fn char_attrs(v: &CharStyle) -> Map<String, Value> {
    let mut m = to_attrs(v);
    for (k, x) in [
        ("font_version", json!(v.font_version)),
        ("fontAxes", json!(v.font_axes)),
        ("leading", json!(v.leading)),
        ("kerning", json!(v.kerning)),
        ("features", json!(v.features)),
        ("overprint_fill", json!(v.overprint_fill)),
        ("overprint_stroke", json!(v.overprint_stroke)),
        ("stroke_cap", json!(v.stroke_cap)),
        ("stroke_join", json!(v.stroke_join)),
        ("stroke_miter_limit", json!(v.stroke_miter_limit)),
        ("stroke_dash", json!(v.stroke_dash)),
        ("stroke_align", json!(v.stroke_align)),
        ("position", json!(v.position)),
        ("small_caps", json!(v.small_caps)),
        ("charAlign", json!(v.char_align)),
        ("proportionalMetrics", json!(v.proportional_metrics)),
        ("digits", json!(v.digits)),
    ] {
        m.insert(k.into(), x);
    }
    m
}

/// Every paragraph attribute of `v` as a JSON map, without the style name: those the paragraph's
/// JSON leaves out at their defaults (the Hyphenation and Justification options, the Every-line Composer, no tab
/// stops, Auto direction…) are written out, as [`char_attrs`] does for characters.
pub fn para_attrs(v: &ParaStyle) -> Map<String, Value> {
    let mut m = to_attrs(v);
    for (k, x) in [
        ("hyphenation", json!(v.hyphenation)),
        ("justification", json!(v.justification)),
        ("composer", json!(v.composer)),
        ("tabs", json!(v.tabs)),
        ("mojikumi", json!(v.mojikumi)),
        ("direction", json!(v.direction)),
        ("leading_model", json!(v.leading_model)),
        ("burasagari", json!(v.burasagari)),
        ("kinsoku", json!(v.kinsoku)),
        ("roman_hanging", json!(v.roman_hanging)),
    ] {
        m.insert(k.into(), x);
    }
    m
}

/// Every attribute name `T` reads, including those its JSON leaves out at their defaults (Auto
/// leading and kerning, no mojikumi…): the field names serde's derive hands `deserialize_struct`.
/// Not `justify_auto`, which [`to_map`] turns into `"justify": "Auto"`.
fn attr_names<T: DeserializeOwned>() -> Vec<&'static str> {
    use serde::de::{Error as _, Visitor, value::Error};

    struct Names<'a>(&'a mut &'static [&'static str]);
    impl<'de> serde::Deserializer<'de> for Names<'_> {
        type Error = Error;
        fn deserialize_any<V: Visitor<'de>>(self, _: V) -> std::result::Result<V::Value, Error> {
            Err(Error::custom("not a struct"))
        }
        fn deserialize_struct<V: Visitor<'de>>(self, _: &'static str, fields: &'static [&'static str], _: V) -> std::result::Result<V::Value, Error> {
            *self.0 = fields;
            Err(Error::custom("field names only"))
        }
        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option unit
            unit_struct newtype_struct seq tuple tuple_struct map enum identifier ignored_any
        }
    }

    let mut fields: &'static [&'static str] = &[];
    // Always an error: the probe only records the names.
    let _ = T::deserialize(Names(&mut fields));
    fields.iter().copied().filter(|&f| f != "justify_auto").collect()
}

/// `base` with `attrs` written over it (unknown or ill-typed attributes are an error).
fn with_attrs<T: Serialize + DeserializeOwned>(base: &T, attrs: &Map<String, Value>, cmd: &str) -> Result<T> {
    let mut m = to_attrs(base);
    if let Some(full) = to_map(base) {
        m.insert("style_name".into(), full.get("style_name").cloned().unwrap_or(Value::Null));
    }
    let names = attr_names::<T>();
    for (k, v) in attrs {
        // Not only the keys `base` writes: those at their defaults (Auto leading…) are left out.
        if !m.contains_key(k) && !names.contains(&k.as_str()) {
            return Err(bad(cmd, format!("unknown attribute `{k}`")));
        }
        m.insert(k.clone(), v.clone());
    }
    serde_json::from_value(Value::Object(m)).map_err(|e| bad(cmd, e.to_string()))
}

/// `cur` after its style changed from `old` to `new`: attributes still equal to the old style's
/// value (or not set by it) take the new value; overrides stay. `attrs` lists every attribute of
/// `cur` ([`char_attrs`], [`para_attrs`]), so one at its default matches a style that sets the
/// default. Unchanged if the style doesn't round-trip through JSON.
fn restyle<T: Serialize + DeserializeOwned + Clone>(
    cur: &T,
    attrs: fn(&T) -> Map<String, Value>,
    old: &Map<String, Value>,
    new: &Map<String, Value>,
) -> T {
    let Some(full) = to_map(cur) else { return cur.clone() };
    let mut m = attrs(cur);
    if let Some(name) = full.get("style_name") {
        m.insert("style_name".into(), name.clone());
    }
    for (k, v) in new {
        if old.get(k).is_none_or(|o| m.get(k) == Some(o)) {
            m.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(Value::Object(m)).unwrap_or_else(|_| cur.clone())
}

/// `st`, written over `before` by a character style's `attrs`: when its font or style changed,
/// it keeps none of the old one's versions or Variable Font settings unless the style sets them.
fn font_change(before: &CharStyle, mut st: CharStyle, attrs: &Map<String, Value>) -> CharStyle {
    if st.font_family == before.font_family && st.font_style == before.font_style {
        return st;
    }
    if st.font_version.as_ref().is_some_and(|v| attrs.get("font_version") != Some(&json!(v))) {
        st.font_version = None;
    }
    if !st.font_axes.is_empty() && attrs.get("fontAxes") != Some(&json!(st.font_axes)) {
        st.font_axes.clear();
    }
    st
}

/// Every text object id in the document.
fn all_text(d: &Document) -> Vec<NodeId> {
    let mut v = vec![];
    d.walk(|n| {
        if matches!(n.kind, NodeKind::Text(_)) {
            v.push(n.id)
        }
    });
    v
}

fn text_mut(d: &mut Document, id: NodeId) -> Option<&mut vectorcraft_doc::TextObject> {
    match d.node_mut(id).map(|n| &mut n.kind) {
        Some(NodeKind::Text(t)) => Some(t),
        _ => None,
    }
}

/// How many runs (char) or paragraphs (para) use `name`.
fn uses(d: &Document, kind: Kind, name: &str) -> usize {
    let target = (name != kind.normal()).then_some(name);
    let mut n = 0;
    d.walk(|node| {
        if let NodeKind::Text(t) = &node.kind {
            n += match kind {
                Kind::Char => t.runs.iter().filter(|r| r.style.style_name.as_deref() == target).count(),
                Kind::Para => (0..t.paragraph_count()).filter(|&i| t.para_at(i).style_name.as_deref() == target).count(),
            };
        }
    });
    n
}

/// Update every user of `name` from `old` to `new` attributes (overrides kept).
fn update_users(d: &mut Document, kind: Kind, name: &str, old: &Map<String, Value>, new: &Map<String, Value>) {
    let target = (name != kind.normal()).then_some(name.to_string());
    for id in all_text(d) {
        let Some(t) = text_mut(d, id) else { continue };
        let mut changed = false;
        match kind {
            Kind::Char => {
                for r in &mut t.runs {
                    if r.style.style_name == target {
                        r.style = font_change(&r.style, restyle(&r.style, char_attrs, old, new), new);
                        changed = true;
                    }
                }
            }
            Kind::Para => {
                for pa in t.para_styles_mut() {
                    if pa.style_name == target {
                        *pa = restyle(pa, para_attrs, old, new);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            refresh_bounds(t);
        }
    }
}

/// The selected text's attributes: the first selected character for a nonempty range,
/// the insertion style at a lone `start`, or the first selected text object's first run.
/// Paragraph capture always samples the paragraph at `start`.
fn selection_attrs(s: &Session, p: &Value, kind: Kind) -> Result<Option<Map<String, Value>>> {
    let d = &s.doc()?.doc;
    let id = match id_param(p, "id") {
        Some(id) => Some(id),
        None => text_targets(s, &json!({}), "").ok().and_then(|v| v.first().copied()),
    };
    let Some(NodeKind::Text(t)) = id.and_then(|i| d.node(i)).map(|n| &n.kind) else { return Ok(None) };
    let at = p.get("start").and_then(Value::as_u64).map_or(0, |v| usize::try_from(v).unwrap_or(usize::MAX));
    Ok(Some(match kind {
        Kind::Char => {
            let style = match p.get("end").and_then(Value::as_u64).map(|v| usize::try_from(v).unwrap_or(usize::MAX)) {
                Some(end) => vectorcraft_text::edit::insertion_style(&t.runs, at.min(end), at.max(end)),
                None => vectorcraft_text::edit::style_at(&t.runs, at),
            };
            char_attrs(&style)
        }
        // The paragraph at `start`.
        Kind::Para => para_attrs(t.para_at(t.paragraphs_in(at, at).start)),
    }))
}

/// The selected text's character (`para`: paragraph) attributes, every one of them: what a
/// library keeps of a text style. `None` without text selected.
pub(crate) fn selected_style_attrs(s: &Session, para: bool) -> Result<Option<Map<String, Value>>> {
    selection_attrs(s, &json!({}), if para { Kind::Para } else { Kind::Char })
}

/// Whether the document has a character (`para`: paragraph) style `name`, and its attributes.
pub(crate) fn style_attrs(d: &Document, para: bool, name: &str) -> Option<Map<String, Value>> {
    if para { Kind::Para } else { Kind::Char }.attrs(d, name)
}

fn name_param<'a>(p: &'a Value, kind: Kind, verb: &str) -> Result<&'a str> {
    str_param(p, "name").ok_or_else(|| bad(&kind.cmd(verb), "missing `name`"))
}

fn attrs_param(p: &Value, kind: Kind, verb: &str) -> Result<Option<Map<String, Value>>> {
    match p.get("attrs") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(m)) => {
            // Validate against the attribute set, and store values in their canonical serialized
            // form (so comparing a style with its text's attributes is exact).
            let c = kind.cmd(verb);
            let full = match kind {
                Kind::Char => char_attrs(&with_attrs(&CharStyle::default(), m, &c)?),
                Kind::Para => para_attrs(&with_attrs(&ParaStyle::default(), m, &c)?),
            };
            Ok(Some(m.keys().filter_map(|k| full.get(k).map(|v| (k.clone(), v.clone()))).collect()))
        }
        Some(_) => Err(bad(&kind.cmd(verb), "attrs must be an object")),
    }
}

/// The attributes of a new style made from nothing (no `attrs`, no text selected): while the
/// interface is in Japanese, new type's em box top-to-top leading (paragraph styles) and em box
/// centre alignment (character styles); else none (the defaults).
fn new_style_attrs(s: &Session, kind: Kind) -> Map<String, Value> {
    if !s.japanese_interface() {
        return Map::new();
    }
    let (set, default) = match kind {
        Kind::Char => {
            let st = CharStyle { char_align: vectorcraft_doc::CharAlign::EmBoxCenter, ..CharStyle::default() };
            (to_attrs(&st), to_attrs(&CharStyle::default()))
        }
        Kind::Para => {
            let st = ParaStyle { leading_model: vectorcraft_doc::LeadingModel::EmBoxTop, ..ParaStyle::default() };
            (to_attrs(&st), to_attrs(&ParaStyle::default()))
        }
    };
    set.into_iter().filter(|(k, v)| default.get(k) != Some(v)).collect()
}

// ---------- commands ----------

fn list(s: &mut Session, kind: Kind) -> Result<Value> {
    let d = &s.doc()?.doc;
    let mut names = vec![kind.normal().to_string()];
    names.extend(kind.defs(d).iter().map(|s| s.name.clone()).filter(|n| n != kind.normal()));
    let styles: Vec<Value> = names.iter().map(|n| json!({ "name": n, "attrs": kind.attrs(d, n), "uses": uses(d, kind, n) })).collect();
    Ok(json!({ "styles": styles }))
}

fn new(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let c = kind.cmd("new");
    let attrs = match attrs_param(p, kind, "new")? {
        Some(a) => a,
        None => match selection_attrs(s, p, kind)? {
            Some(a) => a,
            None => {
                let a = new_style_attrs(s, kind);
                s.note_journal("attrs", Value::Object(a.clone()));
                a
            }
        },
    };
    let d = &s.doc()?.doc;
    let base = if kind == Kind::Char { "Character Style" } else { "Paragraph Style" };
    let name = match str_param(p, "name") {
        Some(n) => n.trim().to_string(),
        None => (1..).map(|i| format!("{base} {i}")).find(|n| kind.attrs(d, n).is_none()).unwrap_or_else(|| base.to_string()),
    };
    if name.is_empty() {
        return Err(bad(&c, "give the style a name"));
    }
    if kind.attrs(d, &name).is_some() {
        return Err(bad(&c, format!("a style named `{name}` already exists")));
    }
    let n2 = name.clone();
    s.edit(if kind == Kind::Char { "New Character Style" } else { "New Paragraph Style" }, |d, _| {
        kind.defs_mut(d).push(TextStyleDef { name: n2, attrs });
        Ok(())
    })?;
    Ok(json!({ "name": name }))
}

fn apply(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let c = kind.cmd("apply");
    let name = name_param(p, kind, "apply")?.to_string();
    let d = &s.doc()?.doc;
    let attrs = kind.attrs(d, &name).ok_or_else(|| bad(&c, format!("no style `{name}`")))?;
    let normal = kind.attrs(d, kind.normal()).unwrap_or_default();
    let clear = bool_or(p, "clearOverrides", false);
    let style_name = (name != kind.normal()).then(|| name.clone());
    // A Type tool range or whole text objects.
    let text_range = super::typecmd::TextRange::parse(p, &c)?;
    let range = id_param(p, "id").filter(|_| text_range.is_some());
    let ids = match range.or_else(|| id_param(p, "id")) {
        Some(id) => vec![id],
        None => text_targets(s, p, &c)?,
    };
    // Characters: Normal then the style over the defaults (clear) or over the current attributes.
    let char_fn = |cur: &CharStyle| -> Result<CharStyle> {
        let base = if clear { with_attrs(&CharStyle::default(), &normal, &c)? } else { cur.clone() };
        let mut st = font_change(&base, with_attrs(&base, &attrs, &c)?, &attrs);
        st.style_name = style_name.clone();
        Ok(st)
    };
    let para_fn = |cur: &ParaStyle| -> Result<ParaStyle> {
        let base = if clear { with_attrs(&ParaStyle::default(), &normal, &c)? } else { cur.clone() };
        let mut st = with_attrs(&base, &attrs, &c)?;
        st.style_name = style_name.clone();
        Ok(st)
    };
    let count = ids.len();
    s.edit_named("Apply {name}", &name, name == kind.normal(), |d, _| {
        for id in &ids {
            let Some(t) = text_mut(d, *id) else { continue };
            match kind {
                Kind::Char => {
                    let len = vectorcraft_text::edit::runs_len(&t.runs);
                    let (a, b) = match range {
                        Some(_) => {
                            let g = |k: &str, def: usize| p.get(k).and_then(Value::as_u64).map_or(def, |v| (v as usize).min(len));
                            (g("start", 0), g("end", len))
                        }
                        None => (0, len),
                    };
                    let mut err = None;
                    vectorcraft_text::edit::style_range(&mut t.runs, a.min(b), a.max(b), |st| match char_fn(st) {
                        Ok(n) => *st = n,
                        Err(e) => err = Some(e),
                    });
                    if len == 0 {
                        for r in &mut t.runs {
                            r.style = char_fn(&r.style)?;
                        }
                    }
                    if let Some(e) = err {
                        return Err(e);
                    }
                }
                Kind::Para => {
                    let mut v = t.paragraph_styles();
                    let span = match (range, text_range) {
                        (Some(_), Some(r)) => r.paras(t),
                        _ => 0..v.len(),
                    };
                    for st in v.iter_mut().take(span.end).skip(span.start) {
                        *st = para_fn(st)?;
                    }
                    t.set_paragraph_styles(v);
                }
            }
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(json!({ "count": count }))
}

fn redefine_to(s: &mut Session, kind: Kind, name: &str, new_attrs: Map<String, Value>, verb: &str) -> Result<Value> {
    let d = &s.doc()?.doc;
    let old = kind.attrs(d, name).ok_or_else(|| bad(&kind.cmd(verb), format!("no style `{name}`")))?;
    let name = name.to_string();
    s.edit_named("Redefine {name}", &name, name == kind.normal(), |d, _| {
        match kind.defs_mut(d).iter_mut().find(|s| s.name == name) {
            Some(def) => def.attrs = new_attrs.clone(),
            None => kind.defs_mut(d).push(TextStyleDef { name: name.clone(), attrs: new_attrs.clone() }),
        }
        update_users(d, kind, &name, &old, &new_attrs);
        Ok(())
    })?;
    ok()
}

fn redefine(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let name = name_param(p, kind, "redefine")?.to_string();
    let attrs = selection_attrs(s, p, kind)?.ok_or_else(|| bad(&kind.cmd("redefine"), "select text to redefine the style from"))?;
    redefine_to(s, kind, &name, attrs, "redefine")
}

fn set_attrs(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let name = name_param(p, kind, "setAttrs")?.to_string();
    let attrs = attrs_param(p, kind, "setAttrs")?.ok_or_else(|| bad(&kind.cmd("setAttrs"), "missing `attrs`"))?;
    redefine_to(s, kind, &name, attrs, "setAttrs")
}

fn duplicate(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let name = name_param(p, kind, "duplicate")?.to_string();
    let d = &s.doc()?.doc;
    let attrs = kind.attrs(d, &name).ok_or_else(|| bad(&kind.cmd("duplicate"), format!("no style `{name}`")))?;
    // `new` trims the name, so the copy is named from the trimmed one.
    let stem = name.trim();
    let copy = (1..)
        .map(|i| if i == 1 { format!("{stem} copy") } else { format!("{stem} copy {i}") })
        .find(|n| kind.attrs(d, n).is_none())
        .unwrap_or_else(|| format!("{stem} copy"));
    new(s, &json!({ "name": copy, "attrs": attrs }), kind)
}

fn rename(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let c = kind.cmd("rename");
    let name = name_param(p, kind, "rename")?.to_string();
    let to = str_param(p, "to").ok_or_else(|| bad(&c, "missing `to`"))?.trim().to_string();
    let d = &s.doc()?.doc;
    if name == kind.normal() || !kind.defs(d).iter().any(|s| s.name == name) {
        return Err(bad(&c, format!("`{name}` can't be renamed")));
    }
    if to.is_empty() || kind.attrs(d, &to).is_some() {
        return Err(bad(&c, format!("`{to}` is empty or already used")));
    }
    s.edit("Rename Style", |d, _| {
        if let Some(def) = kind.defs_mut(d).iter_mut().find(|s| s.name == name) {
            def.name = to.clone();
        }
        for id in all_text(d) {
            let Some(t) = text_mut(d, id) else { continue };
            let fix = |n: &mut Option<String>| {
                if n.as_deref() == Some(name.as_str()) {
                    *n = Some(to.clone());
                }
            };
            match kind {
                Kind::Char => t.runs.iter_mut().for_each(|r| fix(&mut r.style.style_name)),
                Kind::Para => t.para_styles_mut().for_each(|pa| fix(&mut pa.style_name)),
            }
        }
        Ok(())
    })?;
    ok()
}

fn delete(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let c = kind.cmd("delete");
    let name = name_param(p, kind, "delete")?.to_string();
    if name == kind.normal() || !kind.defs(&s.doc()?.doc).iter().any(|s| s.name == name) {
        return Err(bad(&c, format!("`{name}` can't be deleted")));
    }
    s.edit("Delete Style", |d, _| {
        kind.defs_mut(d).retain(|s| s.name != name);
        for id in all_text(d) {
            let Some(t) = text_mut(d, id) else { continue };
            let fix = |n: &mut Option<String>| {
                if n.as_deref() == Some(name.as_str()) {
                    *n = None;
                }
            };
            match kind {
                Kind::Char => t.runs.iter_mut().for_each(|r| fix(&mut r.style.style_name)),
                Kind::Para => t.para_styles_mut().for_each(|pa| fix(&mut pa.style_name)),
            }
        }
        Ok(())
    })?;
    ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with_text(text: &str) -> (Session, u64) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": text})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        (s, id)
    }

    fn runs(s: &Session, id: u64) -> Vec<vectorcraft_doc::TextRun> {
        match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.runs.clone(),
            _ => panic!(),
        }
    }

    #[test]
    fn char_style_new_apply_range_redefine_keeps_overrides() {
        let (mut s, id) = session_with_text("Hello world");
        s.execute("charStyle.new", &json!({"name": "Emphasis", "attrs": {"size": 24.0, "underline": true}})).unwrap();
        // Style "world" only (a Type tool range).
        s.execute("charStyle.apply", &json!({"name": "Emphasis", "id": id, "start": 6, "end": 11})).unwrap();
        let r = runs(&s, id);
        assert_eq!(r.len(), 2);
        assert_eq!((r[1].text.as_str(), r[1].style.size, r[1].style.underline), ("world", 24.0, true));
        assert_eq!(r[1].style.style_name.as_deref(), Some("Emphasis"));
        assert_eq!(r[0].style.style_name, None);
        // A local override on the styled run survives a redefinition; other attributes update.
        s.execute("text.setRangeStyle", &json!({"id": id, "start": 6, "end": 11, "size": 30.0})).unwrap();
        s.execute("charStyle.setAttrs", &json!({"name": "Emphasis", "attrs": {"size": 18.0, "underline": false, "tracking": 50.0}})).unwrap();
        let r = runs(&s, id);
        assert_eq!((r[1].style.size, r[1].style.underline, r[1].style.tracking), (30.0, false, 50.0));
        // Clear Overrides: back to exactly the style's attributes.
        s.execute("charStyle.apply", &json!({"name": "Emphasis", "id": id, "start": 6, "end": 11, "clearOverrides": true})).unwrap();
        assert_eq!(runs(&s, id)[1].style.size, 18.0);
        let list = s.execute("charStyle.list", &json!({})).unwrap();
        assert_eq!(list["styles"][0]["name"], NORMAL_CHAR);
        assert_eq!(list["styles"][1]["uses"], 1);
    }

    /// Applying and redefining a style record a step named from a template with the style's name
    /// in it (`HistoryEntry::named`), and undo and redo keep it. The Normal styles' names are
    /// marked as names VectorCraft gives. A new style's step has a name of its own.
    #[test]
    fn style_steps_are_named_from_a_template() {
        let (mut s, id) = session_with_text("Hello");
        let last = |s: &Session| {
            let e = s.doc().unwrap().history.undo.last().unwrap();
            (e.label.clone(), e.named().map(|n| (n.template, n.name.clone(), n.built_in)))
        };
        let named = |template, name: &str, built_in| Some((template, name.to_string(), built_in));
        s.execute("charStyle.new", &json!({"name": "Body"})).unwrap();
        assert_eq!(last(&s), ("New Character Style".into(), None));
        s.execute("charStyle.apply", &json!({"name": "Body", "id": id})).unwrap();
        assert_eq!(last(&s), ("Apply Body".into(), named("Apply {name}", "Body", false)));
        s.execute("edit.undo", &json!({})).unwrap();
        let redo = s.doc().unwrap().history.redo.last().unwrap().named().cloned();
        assert_eq!(redo.map(|n| n.label()).as_deref(), Some("Apply Body"));
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(last(&s), ("Apply Body".into(), named("Apply {name}", "Body", false)));
        s.execute("charStyle.apply", &json!({"name": NORMAL_CHAR, "id": id})).unwrap();
        assert_eq!(last(&s), (format!("Apply {NORMAL_CHAR}"), named("Apply {name}", NORMAL_CHAR, true)));
        s.execute("paraStyle.new", &json!({"name": "Lead"})).unwrap();
        assert_eq!(last(&s), ("New Paragraph Style".into(), None));
        s.execute("paraStyle.setAttrs", &json!({"name": "Lead", "attrs": {"direction": "rightToLeft"}})).unwrap();
        assert_eq!(last(&s), ("Redefine Lead".into(), named("Redefine {name}", "Lead", false)));
        // A step renamed since it was recorded is no longer named from the template.
        s.doc_mut().unwrap().history.undo.last_mut().unwrap().label = "Typing".into();
        assert_eq!(last(&s), ("Typing".into(), None));
    }

    #[test]
    fn new_from_selection_rename_delete_and_undo() {
        let (mut s, id) = session_with_text("Title");
        s.execute("text.setStyle", &json!({"size": 40})).unwrap();
        let name = s.execute("charStyle.new", &json!({})).unwrap()["name"].as_str().unwrap().to_string();
        assert_eq!(name, "Character Style 1");
        assert_eq!(s.doc().unwrap().doc.char_styles[0].attrs["size"], 40.0);
        s.execute("charStyle.apply", &json!({"name": name})).unwrap();
        s.execute("charStyle.rename", &json!({"name": name, "to": "Heading"})).unwrap();
        assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Heading"));
        assert!(s.execute("charStyle.rename", &json!({"name": NORMAL_CHAR, "to": "x"})).is_err());
        s.execute("charStyle.duplicate", &json!({"name": "Heading"})).unwrap();
        s.execute("charStyle.delete", &json!({"name": "Heading"})).unwrap();
        assert_eq!(runs(&s, id)[0].style.style_name, None);
        assert_eq!(runs(&s, id)[0].style.size, 40.0, "deleting keeps the look");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Heading"));
        assert!(s.execute("charStyle.new", &json!({"name": "Bad", "attrs": {"nope": 1}})).is_err());
        // Attributes are stored canonically (a hand-written colour matches the text's afterwards).
        s.execute(
            "charStyle.new",
            &json!({"name": "Pink", "attrs": {"fill": {"type": "solid", "color": {"model": "rgb", "r": 0.9, "g": 0.2, "b": 0.4}}}}),
        )
        .unwrap();
        s.execute("charStyle.apply", &json!({"name": "Pink"})).unwrap();
        let fill = serde_json::to_value(&runs(&s, id)[0].style.fill).unwrap();
        assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == "Pink").unwrap().attrs["fill"], fill);
    }

    /// New Character Style and New Paragraph Style trim the name they are given, as a rename
    /// trims `to`; the trimmed name is the one compared with the styles there are.
    #[test]
    fn new_styles_trim_their_names_as_a_rename_does() {
        let (mut s, _) = session_with_text("Body text");
        for p in ["charStyle", "paraStyle"] {
            assert_eq!(s.execute(&format!("{p}.new"), &json!({"name": "  Body \t"})).unwrap()["name"], "Body", "{p}");
            let list = s.execute(&format!("{p}.list"), &json!({})).unwrap();
            let names: Vec<&str> = list["styles"].as_array().unwrap().iter().filter_map(|x| x["name"].as_str()).collect();
            assert_eq!(names[1..], ["Body"], "{p}");
            let e = s.execute(&format!("{p}.new"), &json!({"name": " Body"})).unwrap_err().to_string();
            assert!(e.contains("a style named `Body` already exists"), "{p}: {e}");
            let e = s.execute(&format!("{p}.new"), &json!({"name": " \t "})).unwrap_err().to_string();
            assert!(e.contains("give the style a name"), "{p}: {e}");
            assert_eq!(s.execute(&format!("{p}.duplicate"), &json!({"name": "Body"})).unwrap()["name"], "Body copy", "{p}");
        }
        // A style whose name has spaces around it, as a saved file can hold, is copied under the
        // trimmed name, numbered past the copies there are.
        std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).char_styles.push(TextStyleDef { name: " Old ".into(), attrs: Map::new() });
        s.execute("charStyle.new", &json!({"name": "Old copy"})).unwrap();
        assert_eq!(s.execute("charStyle.duplicate", &json!({"name": " Old "})).unwrap()["name"], "Old copy 2");
    }

    /// #1001: attributes their JSON leaves out at the defaults (Auto leading and kerning, no
    /// mojikumi) are still attributes a style can set, and a style captured with one applies to
    /// text at the default, is duplicated and comes back out of a library.
    #[test]
    fn styles_take_attributes_left_out_at_their_defaults() {
        let (mut s, id) = session_with_text("A");
        s.execute("charStyle.new", &json!({"name": "Kerned", "attrs": {"kerning": 40, "leading": 30}})).unwrap();
        s.execute("charStyle.setAttrs", &json!({"name": "Kerned", "attrs": {"kerning": 20, "small_caps": 70}})).unwrap();
        s.execute("paraStyle.new", &json!({"name": "Punctuation", "attrs": {"mojikumi": "lineEndHalf"}})).unwrap();
        s.execute("paraStyle.setAttrs", &json!({"name": "Punctuation", "attrs": {"direction": "rightToLeft"}})).unwrap();
        for bad in [json!({"nope": 1}), json!({"justify_auto": true})] {
            assert!(s.execute("paraStyle.new", &json!({"name": "Bad", "attrs": bad})).is_err(), "{bad}");
        }
        // Captured from text with explicit leading, applied to fresh Auto-leading text.
        s.execute("text.setStyle", &json!({"leading": 30})).unwrap();
        s.execute("charStyle.new", &json!({"name": "Spaced"})).unwrap();
        let fresh = s.execute("text.create", &json!({"x": 10, "y": 90, "text": "B"})).unwrap()["id"].as_u64().unwrap();
        assert_eq!(runs(&s, fresh)[0].style.leading, None);
        for clear in [false, true] {
            s.execute("charStyle.apply", &json!({"name": "Spaced", "id": fresh, "clearOverrides": clear})).unwrap();
            assert_eq!(runs(&s, fresh)[0].style.leading, Some(30.0));
        }
        let copy = s.execute("charStyle.duplicate", &json!({"name": "Spaced"})).unwrap()["name"].as_str().unwrap().to_string();
        assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == copy).unwrap().attrs["leading"], 30.0);
        s.execute("charStyle.apply", &json!({"name": "Kerned", "id": fresh})).unwrap();
        let st = runs(&s, fresh)[0].style.clone();
        assert_eq!((st.kerning, st.small_caps), (Some(20.0), Some(70.0)));
        s.execute("paraStyle.apply", &json!({"name": "Punctuation", "id": id})).unwrap();
        let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!() };
        assert_eq!((t.para.mojikumi, t.para.direction), (vectorcraft_doc::Mojikumi::LineEndHalf, Some(vectorcraft_doc::ParaDirection::RightToLeft)));
        // A library keeps the captured attributes and makes the style again where it's used.
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let lib = s.execute("library.add", &json!({"kind": "charStyle", "name": "Lib Spaced"})).unwrap();
        let other = s.execute("text.create", &json!({"x": 10, "y": 150, "text": "C"})).unwrap()["id"].as_u64().unwrap();
        s.execute("library.use", &json!({"library": lib["library"], "kind": "charStyle", "item": lib["name"]})).unwrap();
        assert_eq!(runs(&s, other)[0].style.leading, Some(30.0));
    }

    #[test]
    fn auto_leading_redefine_keeps_overrides_and_history() {
        for redefine in [true, false] {
            let (mut s, source) = session_with_text("Source");
            let [linked, local] =
                ["Linked", "Local"].map(|text| s.execute("text.create", &json!({"x": 10, "y": 90, "text": text})).unwrap()["id"].as_u64().unwrap());
            let ids = [source, linked, local];
            s.execute("charStyle.new", &json!({"name": "Body", "attrs": {"leading": 30}})).unwrap();
            s.execute("charStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
            s.execute("text.setStyle", &json!({"ids": [local], "leading": 42})).unwrap();
            s.execute("text.setStyle", &json!({"ids": [source], "leading": "auto"})).unwrap();
            let before = s.doc().unwrap().doc.clone();
            let undo = s.doc().unwrap().history.undo.len();
            if redefine {
                s.execute("charStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
            } else {
                s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"leading": null}})).unwrap();
            }
            assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [None, None, Some(42.0)]);
            assert!(ids.iter().all(|&id| runs(&s, id)[0].style.style_name.as_deref() == Some("Body")));
            let after = s.doc().unwrap().doc.clone();
            assert_eq!(after.char_styles[0].attrs.get("leading"), Some(&Value::Null), "Auto is explicit, not an unset attribute");
            assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, before);
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, after);
            let back = vectorcraft_format::load(&vectorcraft_format::save(&after, false)).unwrap();
            assert_eq!(back.char_styles, after.char_styles);
            for id in ids {
                let NodeKind::Text(t) = &back.node(NodeId(id)).unwrap().kind else { panic!("not text") };
                assert_eq!(t.runs, runs(&s, id));
            }
            // Auto users follow the next numeric definition; real numeric and Auto overrides stay.
            s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"leading": 26}})).unwrap();
            assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [Some(26.0), Some(26.0), Some(42.0)]);
            s.execute("text.setStyle", &json!({"ids": [linked], "leading": "auto"})).unwrap();
            s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"leading": 28}})).unwrap();
            assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [Some(28.0), None, Some(42.0)]);
            // Omitting leading replaces the definition without resetting the text's leading.
            s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"tracking": 20}})).unwrap();
            assert!(!s.doc().unwrap().doc.char_styles[0].attrs.contains_key("leading"));
            assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [Some(28.0), None, Some(42.0)]);
        }
    }

    /// The attribute names a style of type `T` can set, without the style name.
    fn settable<T: DeserializeOwned>() -> std::collections::BTreeSet<String> {
        attr_names::<T>().into_iter().filter(|&n| n != "style_name").map(String::from).collect()
    }

    /// New and Redefine capture every paragraph attribute. A paragraph style redefined from a
    /// paragraph at the defaults gives its paragraphs the defaults back where they had the old
    /// definition's values, and `setAttrs` with a default value does too; local overrides, and
    /// attributes `setAttrs` leaves out, stay.
    #[test]
    fn para_style_redefined_to_the_defaults_updates_its_paragraphs() {
        use vectorcraft_doc::{Composer, HyphenationOptions};
        let (mut s, source) = session_with_text("Source");
        let [linked, local] =
            ["Linked", "Local"].map(|text| s.execute("text.create", &json!({"x": 10, "y": 90, "text": text})).unwrap()["id"].as_u64().unwrap());
        let ids = [source, linked, local];
        let para = |s: &Session, id: u64| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.para_at(0).clone(),
            _ => panic!("not text"),
        };
        let limits = |s: &Session| ids.map(|id| para(s, id).hyphenation.hyphen_limit);
        let composers = |s: &Session| ids.map(|id| para(s, id).composer);
        let defined = |s: &Session, i: usize| s.doc().unwrap().doc.para_styles[i].attrs.keys().cloned().collect::<std::collections::BTreeSet<_>>();
        s.execute("text.setFormat", &json!({"ids": ids, "hyphenLimit": 2, "composer": "singleLine"})).unwrap();
        s.execute("paraStyle.new", &json!({"name": "Body", "id": source})).unwrap();
        s.execute("paraStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [local], "hyphenLimit": 4})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [source], "hyphenLimit": 0, "composer": "everyLine"})).unwrap();
        s.execute("paraStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
        assert_eq!((limits(&s), composers(&s)), ([0, 0, 4], [Composer::EveryLine; 3]));
        assert_eq!(para(&s, linked).hyphenation, HyphenationOptions::default());
        assert!(ids.iter().all(|&id| para(&s, id).style_name.as_deref() == Some("Body")));
        assert_eq!(defined(&s, 0), settable::<ParaStyle>(), "Redefine captures every attribute");
        s.execute("paraStyle.new", &json!({"name": "Copy", "id": local})).unwrap();
        assert_eq!(defined(&s, 1), settable::<ParaStyle>(), "New captures every attribute");
        // Style options: a default value reaches the users too; an attribute left out stays as it is.
        s.execute("paraStyle.setAttrs", &json!({"name": "Body", "attrs": {"hyphenation": {"hyphen_limit": 3}, "composer": "singleLine"}})).unwrap();
        assert_eq!((limits(&s), composers(&s)), ([3, 3, 4], [Composer::SingleLine; 3]));
        s.execute("paraStyle.setAttrs", &json!({"name": "Body", "attrs": {"composer": "everyLine"}})).unwrap();
        assert_eq!((limits(&s), composers(&s)), ([3, 3, 4], [Composer::EveryLine; 3]));
        assert_eq!(Value::Object(s.doc().unwrap().doc.para_styles[0].attrs.clone()), json!({"composer": "everyLine"}));
    }

    /// The same for character styles: Auto kerning and the Roman baseline alignment, which the
    /// run's JSON leaves out, are captured and reach the style's users.
    #[test]
    fn char_style_redefined_to_the_defaults_updates_its_text() {
        use vectorcraft_doc::CharAlign;
        let (mut s, source) = session_with_text("Source");
        let [linked, local] =
            ["Linked", "Local"].map(|text| s.execute("text.create", &json!({"x": 10, "y": 90, "text": text})).unwrap()["id"].as_u64().unwrap());
        let ids = [source, linked, local];
        let kerning = |s: &Session| ids.map(|id| runs(s, id)[0].style.kerning);
        let aligns = |s: &Session| ids.map(|id| runs(s, id)[0].style.char_align);
        let defined = |s: &Session, i: usize| s.doc().unwrap().doc.char_styles[i].attrs.keys().cloned().collect::<std::collections::BTreeSet<_>>();
        s.execute("text.setFormat", &json!({"ids": ids, "kerning": 40, "charAlign": "emBoxCenter"})).unwrap();
        s.execute("charStyle.new", &json!({"name": "Body", "id": source})).unwrap();
        s.execute("charStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [local], "kerning": 60})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [source], "kerning": "auto", "charAlign": "romanBaseline"})).unwrap();
        s.execute("charStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
        assert_eq!((kerning(&s), aligns(&s)), ([None, None, Some(60.0)], [CharAlign::RomanBaseline; 3]));
        assert_eq!(defined(&s, 0), settable::<CharStyle>(), "Redefine captures every attribute");
        s.execute("charStyle.new", &json!({"name": "Copy", "id": local})).unwrap();
        assert_eq!(defined(&s, 1), settable::<CharStyle>(), "New captures every attribute");
        s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"kerning": 10, "charAlign": "emBoxTop"}})).unwrap();
        assert_eq!((kerning(&s), aligns(&s)), ([Some(10.0), Some(10.0), Some(60.0)], [CharAlign::EmBoxTop; 3]));
        s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"charAlign": "romanBaseline"}})).unwrap();
        assert_eq!((kerning(&s), aligns(&s)), ([Some(10.0), Some(10.0), Some(60.0)], [CharAlign::RomanBaseline; 3]));
        assert_eq!(Value::Object(s.doc().unwrap().doc.char_styles[0].attrs.clone()), json!({"charAlign": "romanBaseline"}));
    }

    /// A style captured from type without Variable Font settings sets none explicitly (`{}`):
    /// applying it clears type's settings, and redefining it from type whose settings were reset
    /// resets the type that uses it.
    #[test]
    fn variable_font_settings_are_captured_explicitly() {
        use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
        vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
        let (mut s, source) = session_with_text("Source");
        let linked = s.execute("text.create", &json!({"x": 10, "y": 90, "text": "Linked"})).unwrap()["id"].as_u64().unwrap();
        let ids = [source, linked];
        s.execute("text.setStyle", &json!({"ids": ids, "font": TWO_AXIS_FAMILY})).unwrap();
        s.execute("charStyle.new", &json!({"name": "Body", "id": source})).unwrap();
        assert_eq!(s.doc().unwrap().doc.char_styles[0].attrs.get("fontAxes"), Some(&json!({})));
        s.execute("text.setFormat", &json!({"ids": [linked], "fontAxes": {"wght": 650}})).unwrap();
        s.execute("charStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
        let wght = |s: &Session| ids.map(|id| runs(s, id)[0].style.font_axes.get("wght").copied());
        assert_eq!(wght(&s), [None, None], "applying the style clears the settings");
        for (set, want) in [(json!(650), Some(650.0)), (json!(null), None)] {
            s.execute("text.setFormat", &json!({"ids": [source], "fontAxes": {"wght": set}})).unwrap();
            s.execute("charStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
            assert_eq!(wght(&s), [want, want], "redefined from {set}");
        }
    }

    /// A style that changes the font, applied or redefined, leaves none of the old font's
    /// version and Variable Font settings, unless the style sets them.
    #[test]
    fn another_font_from_a_style_clears_the_old_fonts_version_and_settings() {
        use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
        vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
        let (mut s, id) = session_with_text("Source");
        let set_variable = |s: &mut Session| {
            s.execute("text.setStyle", &json!({"ids": [id], "font": TWO_AXIS_FAMILY})).unwrap();
            s.execute("text.setFormat", &json!({"ids": [id], "fontAxes": {"wght": 650}})).unwrap();
            s.edit("version", |d, _| {
                if let Some(t) = text_mut(d, NodeId(id)) {
                    t.runs.iter_mut().for_each(|r| r.style.font_version = Some("Version 2.000".into()));
                }
                Ok(())
            })
            .unwrap();
        };
        let font = |s: &Session| {
            let st = runs(s, id)[0].style.clone();
            (st.font_family, st.font_version, st.font_axes.get("wght").copied())
        };
        set_variable(&mut s);
        s.execute("charStyle.new", &json!({"name": "Plain", "attrs": {"font_family": "Inter"}})).unwrap();
        s.execute("charStyle.new", &json!({"name": "Same", "attrs": {"size": 20}})).unwrap();
        s.execute("charStyle.apply", &json!({"name": "Same", "id": id})).unwrap();
        assert_eq!(font(&s), (TWO_AXIS_FAMILY.to_string(), Some("Version 2.000".to_string()), Some(650.0)), "the same font keeps them");
        s.execute("charStyle.apply", &json!({"name": "Plain", "id": id})).unwrap();
        assert_eq!(font(&s), ("Inter".to_string(), None, None));
        // A style that sets the settings keeps its own.
        s.execute("charStyle.new", &json!({"name": "Heavy", "attrs": {"font_family": TWO_AXIS_FAMILY, "fontAxes": {"wght": 600}}})).unwrap();
        s.execute("charStyle.apply", &json!({"name": "Heavy", "id": id})).unwrap();
        assert_eq!(font(&s), (TWO_AXIS_FAMILY.to_string(), None, Some(600.0)));
        // Redefined to another font: its users' settings and versions go too.
        s.execute("charStyle.new", &json!({"name": "Sans", "attrs": {"font_family": TWO_AXIS_FAMILY}})).unwrap();
        s.execute("charStyle.apply", &json!({"name": "Sans", "id": id})).unwrap();
        set_variable(&mut s);
        assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Sans"));
        s.execute("charStyle.setAttrs", &json!({"name": "Sans", "attrs": {"font_family": "Inter"}})).unwrap();
        assert_eq!(font(&s), ("Inter".to_string(), None, None));
    }

    #[test]
    fn auto_leading_new_distinguishes_unset_attributes() {
        let (mut s, id) = session_with_text("Title");
        s.execute("text.setStyle", &json!({"leading": 30})).unwrap();
        s.execute("charStyle.new", &json!({"name": "Sparse", "attrs": {"tracking": 20}})).unwrap();
        assert!(!s.doc().unwrap().doc.char_styles[0].attrs.contains_key("leading"));
        s.execute("charStyle.apply", &json!({"name": "Sparse", "id": id})).unwrap();
        assert_eq!(runs(&s, id)[0].style.leading, Some(30.0));
        s.execute("charStyle.new", &json!({"name": "Automatic", "attrs": {"leading": null}})).unwrap();
        s.execute("charStyle.apply", &json!({"name": "Automatic", "id": id})).unwrap();
        assert_eq!(runs(&s, id)[0].style.leading, None);
        assert_eq!(s.doc().unwrap().doc.char_styles[1].attrs.get("leading"), Some(&Value::Null));
        // New from Auto text captures that value too, rather than leaving leading unspecified.
        s.execute("charStyle.new", &json!({"name": "Captured", "id": id})).unwrap();
        s.execute("text.setStyle", &json!({"ids": [id], "leading": 36})).unwrap();
        s.execute("charStyle.apply", &json!({"name": "Captured", "id": id})).unwrap();
        assert_eq!(runs(&s, id)[0].style.leading, None);
        assert_eq!(s.doc().unwrap().doc.char_styles[2].attrs.get("leading"), Some(&Value::Null));
    }

    #[test]
    fn paragraph_styles_and_survive_save() {
        let (mut s, id) = session_with_text("Body");
        s.execute("paraStyle.new", &json!({"name": "Centered", "attrs": {"justify": "Center", "space_after": 6.0}})).unwrap();
        s.execute("paraStyle.apply", &json!({"name": "Centered"})).unwrap();
        let para = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.para.clone(),
            _ => panic!(),
        };
        assert_eq!((para(&s).justify, para(&s).space_after), (vectorcraft_doc::Justify::Center, 6.0));
        // The Normal style can be redefined too (stored once redefined).
        s.execute("paraStyle.redefine", &json!({"name": NORMAL_PARA})).unwrap();
        assert_eq!(s.execute("paraStyle.list", &json!({})).unwrap()["styles"][0]["attrs"]["justify"], "Center");
        let d = s.doc().unwrap().doc.clone();
        let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
        assert_eq!(back.para_styles, d.para_styles);
    }

    fn capture_boundary_command(redefine: bool, text: &str, start: usize, end: Option<usize>, expected_size: f64) {
        let (mut s, id) = session_with_text(text);
        s.execute("text.setStyle", &json!({"id": id, "size": 18})).unwrap();
        s.execute("text.setRangeStyle", &json!({"id": id, "start": 2, "end": text.len(), "size": 34})).unwrap();
        if redefine {
            s.execute("charStyle.new", &json!({"name": "Captured", "attrs": {"size": 24}})).unwrap();
        }
        s.doc_mut().unwrap().mark_saved();
        let before = s.doc().unwrap().doc.clone();
        let selection = s.doc().unwrap().selection.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let mut params = json!({"name": "Captured", "id": id, "start": start});
        if let Some(end) = end {
            params["end"] = json!(end);
        }
        let command = if redefine { "charStyle.redefine" } else { "charStyle.new" };
        s.execute(command, &params).unwrap();
        let st = s.doc().unwrap();
        let def = st.doc.char_styles.iter().find(|d| d.name == "Captured").unwrap();
        assert_eq!(def.attrs.get("size").and_then(Value::as_f64), Some(expected_size), "{command} {text:?} {start}..{end:?}");
        assert!(!def.attrs.contains_key("style_name"));
        assert_eq!(st.doc.node(NodeId(id)), before.node(NodeId(id)), "capture does not apply the definition");
        assert_eq!(st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
        assert!(st.is_dirty());
        let after = st.doc.clone();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, before);
        assert_eq!(s.doc().unwrap().selection, selection);
        assert!(!s.doc().unwrap().is_dirty());
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, after);
        assert_eq!(s.doc().unwrap().selection, selection);
    }

    fn capture_boundary_ranges(redefine: bool) {
        for (text, start, end, size) in [
            ("ABcdef", 2, 5, 34.0),
            ("é漢字", 2, 5, 34.0),
            ("ABcdef", 5, 2, 34.0), // command ranges are ordered before selected-character sampling
            ("ABcdef", 1, 5, 18.0), // a mixed range samples its first selected character
            ("ABcdef", 2, 2, 18.0), // collapsed range retains insertion affinity
            ("ABcdef", 3, 3, 34.0),
            ("ABcdef", 99, 100, 34.0), // beyond the text retains the final run
        ] {
            capture_boundary_command(redefine, text, start, Some(end), size);
        }
    }

    #[test]
    fn char_style_new_range_capture_boundary() {
        capture_boundary_ranges(false);
    }

    #[test]
    fn char_style_redefine_range_capture_boundary() {
        capture_boundary_ranges(true);
    }

    #[test]
    fn char_style_start_only_capture_affinity_controls() {
        for redefine in [false, true] {
            for text in ["ABcdef", "é漢字"] {
                for (start, size) in [(0, 18.0), (2, 18.0), (3, 34.0), (99, 34.0)] {
                    capture_boundary_command(redefine, text, start, None, size);
                }
            }
        }
    }

    #[test]
    fn para_style_range_capture_samples_start_paragraph() {
        for redefine in [false, true] {
            for (start, end, justify) in [(0, 7, "Left"), (4, 7, "Right"), (4, 0, "Right")] {
                let (mut s, id) = session_with_text("One\nTwo");
                s.execute("text.setStyle", &json!({"id": id, "justify": "left"})).unwrap();
                s.execute("text.setStyle", &json!({"id": id, "start": 4, "end": 7, "justify": "right"})).unwrap();
                if redefine {
                    s.execute("paraStyle.new", &json!({"name": "Captured", "attrs": {"justify": "Center"}})).unwrap();
                }
                let before = s.doc().unwrap().doc.clone();
                let selection = s.doc().unwrap().selection.clone();
                let undo = s.doc().unwrap().history.undo.len();
                let command = if redefine { "paraStyle.redefine" } else { "paraStyle.new" };
                s.execute(command, &json!({"name": "Captured", "id": id, "start": start, "end": end})).unwrap();
                let st = s.doc().unwrap();
                let def = st.doc.para_styles.iter().find(|d| d.name == "Captured").unwrap();
                assert_eq!(def.attrs["justify"], json!(justify), "paragraph capture still samples start, not the end or a normalized range");
                assert_eq!(st.doc.node(NodeId(id)), before.node(NodeId(id)));
                assert_eq!(st.selection, selection);
                assert_eq!(st.history.undo.len(), undo + 1);
                let after = st.doc.clone();
                s.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(s.doc().unwrap().doc, before);
                s.execute("edit.redo", &json!({})).unwrap();
                assert_eq!(s.doc().unwrap().doc, after);
            }
        }
    }
}
