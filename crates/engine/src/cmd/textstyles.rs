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
use super::{CommandSpec, Result, Run, Session, bad, bool_or, has_doc, id_param, index, ok, str_param};

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
                "{name} | {names: [name…]} delete styles in one undo step (text keeps its look; never the Normal style)",
                |s, p| delete(s, p, $kind),
                true,
            ),
            spec(
                concat!($p, ".unused"),
                "Select All Unused",
                &["Window", "Type", $menu],
                "{} → {names: [the styles no text uses, in panel order; never the Normal style]} (the panel highlights them)",
                |s, _| unused(s, $kind),
                false,
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
        ("charDirection", json!(v.char_direction)),
        ("proportionalMetrics", json!(v.proportional_metrics)),
        ("tateChuYoko", json!(v.tate_chu_yoko)),
        ("noBreak", json!(v.no_break)),
        ("digits", json!(v.digits)),
    ] {
        m.insert(k.into(), x);
    }
    m
}

/// Every paragraph attribute of `v` as a JSON map, without the style name: those the paragraph's
/// JSON leaves out at their defaults (the Hyphenation and Justification options, the Every-line Composer, no tab
/// stops, no list, Auto direction…) are written out, as [`char_attrs`] does for characters.
pub fn para_attrs(v: &ParaStyle) -> Map<String, Value> {
    let mut m = to_attrs(v);
    for (k, x) in [
        ("hyphenation", json!(v.hyphenation)),
        ("justification", json!(v.justification)),
        ("composer", json!(v.composer)),
        ("tabs", json!(v.tabs)),
        ("list", json!(v.list)),
        ("mojikumi", json!(v.mojikumi)),
        ("direction", json!(v.direction)),
        ("leading_model", json!(v.leading_model)),
        ("burasagari", json!(v.burasagari)),
        ("kinsoku", json!(v.kinsoku)),
        ("roman_hanging", json!(v.roman_hanging)),
        ("kashida", json!(v.kashida)),
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

/// How many runs (char) or paragraphs (para) use each style, by style name (`None`: the Normal
/// style). Type in the art (opacity masks included), symbols, graph designs, pattern tiles and the
/// art of brushes counts.
fn uses(d: &Document, kind: Kind) -> std::collections::HashMap<Option<String>, usize> {
    let mut n = std::collections::HashMap::new();
    let mut count = |name: &Option<String>| *n.entry(name.clone()).or_insert(0) += 1;
    let brushes = vectorcraft_brush::library(d);
    let trees = d.layers.iter().map(|l| &**l).chain(d.symbols.iter().map(|s| &*s.art)).chain(d.graph_designs.iter().map(|g| &*g.art));
    let trees = trees.chain(d.patterns.iter().flat_map(|p| p.art.iter().map(|a| &**a))).chain(brushes.iter().flat_map(|b| b.art()));
    for tree in trees {
        vectorcraft_doc::swatches::walk_with_masks(tree, &mut |node| {
            if let NodeKind::Text(t) = &node.kind {
                match kind {
                    Kind::Char => t.runs.iter().for_each(|r| count(&r.style.style_name)),
                    Kind::Para => (0..t.paragraph_count()).for_each(|i| count(&t.para_at(i).style_name)),
                }
            }
        });
    }
    n
}

/// How many runs (char) or paragraphs (para) use `name` (see [`uses`]).
fn uses_of(uses: &std::collections::HashMap<Option<String>, usize>, kind: Kind, name: &str) -> usize {
    uses.get(&(name != kind.normal()).then(|| name.to_string())).copied().unwrap_or(0)
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
    let at = p.get("start").and_then(Value::as_u64).map_or(0, index);
    Ok(Some(match kind {
        Kind::Char => {
            let style = match p.get("end").and_then(Value::as_u64).map(index) {
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

/// The attributes `charStyle.new` (`para`: `paraStyle.new`) would give a style made now from
/// `target` (its `id`, `start` and `end`; empty for the selected objects). `None` without text
/// there.
pub fn target_style_attrs(s: &Session, target: &Value, para: bool) -> Option<Map<String, Value>> {
    selection_attrs(s, target, if para { Kind::Para } else { Kind::Char }).ok().flatten()
}

/// Whether the document has a character (`para`: paragraph) style `name`, and its attributes.
pub(crate) fn style_attrs(d: &Document, para: bool, name: &str) -> Option<Map<String, Value>> {
    if para { Kind::Para } else { Kind::Char }.attrs(d, name)
}

/// The name `charStyle.new` (`para`: `paraStyle.new`) gives a style when it is given none: the
/// first "Character Style N" ("Paragraph Style N") that no character style (paragraph style) of the
/// document has.
pub fn new_style_name(d: &Document, para: bool) -> String {
    let kind = if para { Kind::Para } else { Kind::Char };
    let base = if para { "Paragraph Style" } else { "Character Style" };
    (1..).map(|i| format!("{base} {i}")).find(|n| kind.attrs(d, n).is_none()).unwrap_or_else(|| base.to_string())
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
    let uses = uses(d, kind);
    let styles: Vec<Value> = names.iter().map(|n| json!({ "name": n, "attrs": kind.attrs(d, n), "uses": uses_of(&uses, kind, n) })).collect();
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
    let name = match str_param(p, "name") {
        Some(n) => n.trim().to_string(),
        None => new_style_name(d, kind == Kind::Para),
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
                    let (a, b) = match (range, text_range) {
                        (Some(_), Some(r)) => r.bytes(t),
                        _ => (0, len),
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
    let names: Vec<String> = match p.get("names") {
        Some(v) => v
            .as_array()
            .and_then(|a| a.iter().map(|n| n.as_str().map(str::to_string)).collect::<Option<Vec<_>>>())
            .ok_or_else(|| bad(&c, "`names` must be an array of style names"))?,
        None => vec![name_param(p, kind, "delete")?.to_string()],
    };
    if names.is_empty() {
        return Err(bad(&c, "missing `name`"));
    }
    let defs = kind.defs(&s.doc()?.doc);
    if let Some(name) = names.iter().find(|n| *n == kind.normal() || !defs.iter().any(|s| s.name == **n)) {
        return Err(bad(&c, format!("`{name}` can't be deleted")));
    }
    s.edit(if names.len() > 1 { "Delete Styles" } else { "Delete Style" }, |d, _| {
        kind.defs_mut(d).retain(|s| !names.contains(&s.name));
        for id in all_text(d) {
            let Some(t) = text_mut(d, id) else { continue };
            let fix = |n: &mut Option<String>| {
                if n.as_ref().is_some_and(|n| names.contains(n)) {
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

/// The styles no text uses (see [`uses`]), in the order the panel lists them, without the Normal
/// style.
fn unused(s: &mut Session, kind: Kind) -> Result<Value> {
    let d = &s.doc()?.doc;
    let uses = uses(d, kind);
    let names: Vec<&str> = kind.defs(d).iter().map(|s| s.name.as_str()).filter(|n| *n != kind.normal() && uses_of(&uses, kind, n) == 0).collect();
    Ok(json!({ "names": names }))
}

#[cfg(test)]
mod tests;
